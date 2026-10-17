// Rust translation of src/unpack_dsd.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 2013 - 2024 David Bryant.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Unpacking DSD blocks (`ENABLE_DSD`). SDL_mixer refuses files that start
//! with DSD audio, but the metadata of any block is parsed and DSD blocks
//! later in a file are decoded (into 8-bit "samples"). The decimation to
//! PCM (`decimate_dsd_init()` and the rest) only runs for
//! `OPEN_DSD_AS_PCM`, which SDL_mixer never asks for, so it isn't
//! translated.

// This module actually handles the uncompression of the DSD audio data.

use super::common_utils::free_dsd_tables;
use super::*;

///////////////////////////// executable code ////////////////////////////////

// This function initializes the main range-encoded data for DSD audio samples

/// Translation of `init_dsd_block()`; `dsd_multiplier` is
/// `wps->wpc->dsd_multiplier`.
pub(crate) fn init_dsd_block(
    dsd_multiplier: &mut u32,
    wps: &mut WavpackStream,
    wpmd: &WavpackMetadata<'_>,
) -> i32 {
    if wpmd.byte_length < 2 {
        return FALSE;
    }

    wps.dsd.data = wpmd.bytes().to_vec();
    wps.dsd.byteptr = 0;
    wps.dsd.endptr = wpmd.byte_length as usize;

    if wps.dsd.data[wps.dsd.byteptr] > 31 {
        return FALSE;
    }

    // safe to cast away const on stream 0 only
    if wps.stream_index == 0 {
        *dsd_multiplier = 1u32 << wps.dsd.data[wps.dsd.byteptr];
    }
    wps.dsd.byteptr += 1;

    wps.dsd.mode = wps.dsd.data[wps.dsd.byteptr];
    wps.dsd.byteptr += 1;

    if wps.dsd.mode == 0 {
        if (wps.dsd.endptr - wps.dsd.byteptr) as i64
            != wps.wphdr.block_samples as i64
                * if (wps.wphdr.flags & MONO_DATA) != 0 {
                    1
                } else {
                    2
                }
        {
            return FALSE;
        }

        wps.dsd.ready = 1;
        return TRUE;
    }

    if wps.dsd.mode == 1 {
        init_dsd_block_fast(wps)
    } else if wps.dsd.mode == 3 {
        init_dsd_block_high(wps)
    } else {
        FALSE
    }
}

/// Translation of `unpack_dsd_samples()`; `wpc` is `wps->wpc`.
pub(crate) fn unpack_dsd_samples(
    wpc: &WavpackContext<'_>,
    wps: &mut WavpackStream,
    buffer: &mut [i32],
    mut sample_count: u32,
) -> i32 {
    let flags = wps.wphdr.flags;

    // don't attempt to decode past the end of the block, but watch out for overflow!

    let block_end = get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64;
    if wps.sample_index + sample_count as i64 > block_end
        && ((block_end - wps.sample_index) as u32) < sample_count
    {
        sample_count = (block_end - wps.sample_index) as u32;
    }

    if get_block_index(&wps.wphdr) > wps.sample_index || wps.wphdr.block_samples < sample_count {
        wps.mute_error = TRUE;
    }

    if wps.mute_error == 0 {
        if wps.dsd.mode == 0 {
            let mut total_samples =
                sample_count as i64 * if (flags & MONO_DATA) != 0 { 1 } else { 2 };

            if ((wps.dsd.endptr - wps.dsd.byteptr) as i64) < total_samples {
                total_samples = (wps.dsd.endptr - wps.dsd.byteptr) as i64;
            }

            for bptr in buffer[..total_samples as usize].iter_mut() {
                *bptr = byte_at(&wps.dsd.data, wps.dsd.byteptr) as i32;
                wps.dsd.byteptr += 1;
                wps.crc = wps
                    .crc
                    .wrapping_add(wps.crc << 1)
                    .wrapping_add(*bptr as u32);
            }
        } else if wps.dsd.mode == 1 {
            if decode_fast(wps, buffer, sample_count as i32) == 0 {
                wps.mute_error = TRUE;
            }
        } else if decode_high(wps, buffer, sample_count as i32) == 0 {
            wps.mute_error = TRUE;
        }

        // If we just finished this block, then it's time to check if the applicable checksum matches. Like
        // other decoding errors, this is indicated by setting the mute_error flag.

        if wps.sample_index + sample_count as i64 == block_end
            && wps.mute_error == 0
            && wps.crc != wps.wphdr.crc
        {
            wps.mute_error = TRUE;
        }
    }

    if wps.mute_error != 0 {
        let samples_to_null = if wpc.reduced_channels == 1
            || wpc.config.num_channels == 1
            || (flags & MONO_FLAG) != 0
        {
            sample_count as usize
        } else {
            sample_count as usize * 2
        };

        buffer[..samples_to_null].fill(0x55);

        wps.sample_index += sample_count as i64;
        return sample_count as i32;
    }

    if (flags & FALSE_STEREO) != 0 {
        let mut dptr = sample_count as usize * 2;
        let mut sptr = sample_count as usize;

        for _ in 0..sample_count {
            sptr -= 1;
            dptr -= 1;
            buffer[dptr] = buffer[sptr];
            dptr -= 1;
            buffer[dptr] = buffer[sptr];
        }
    }

    wps.sample_index += sample_count as i64;

    sample_count as i32
}

/*------------------------------------------------------------------------------------------------------------------------*/

// #define DSD_BYTE_READY(low,high) (((low) >> 24) == ((high) >> 24))
// #define DSD_BYTE_READY(low,high) (!(((low) ^ (high)) >> 24))
/// Translation of `DSD_BYTE_READY()`.
#[inline]
fn dsd_byte_ready(low: u32, high: u32) -> bool {
    ((low ^ high) & 0xff000000) == 0
}

/// Translation of `init_dsd_block_fast()`.
fn init_dsd_block_fast(wps: &mut WavpackStream) -> i32 {
    let history_bits: u8;
    let max_probability: u8;
    let mut total_summed_probabilities = 0i32;

    let dsd = &mut wps.dsd;

    if dsd.byteptr == dsd.endptr {
        return FALSE;
    }

    history_bits = byte_at(&dsd.data, dsd.byteptr);
    dsd.byteptr += 1;

    if dsd.byteptr == dsd.endptr || history_bits > MAX_HISTORY_BITS {
        return FALSE;
    }

    dsd.history_bins = 1 << history_bits;
    let bins = dsd.history_bins as usize;

    free_dsd_tables(wps);
    let dsd = &mut wps.dsd;
    let mut lookup_buffer = vec![0u8; bins * MAX_BYTES_PER_BIN as usize];
    let mut lb_ptr = 0usize;
    let mut value_lookup: Vec<Option<usize>> = vec![None; bins];
    let mut summed_probabilities = vec![[0u16; 256]; bins];
    let mut probabilities = vec![[0u8; 256]; bins];

    max_probability = byte_at(&dsd.data, dsd.byteptr);
    dsd.byteptr += 1;

    let ok = 'tables: {
        if max_probability < 0xff {
            let mut outptr = 0usize;
            let outend = 256 * bins;

            while outptr < outend && dsd.byteptr < dsd.endptr {
                let code = byte_at(&dsd.data, dsd.byteptr) as i32;
                dsd.byteptr += 1;

                if code > max_probability as i32 {
                    let mut zcount = code - max_probability as i32;

                    while outptr < outend && zcount > 0 {
                        zcount -= 1;
                        probabilities[outptr / 256][outptr % 256] = 0;
                        outptr += 1;
                    }
                } else if code != 0 {
                    probabilities[outptr / 256][outptr % 256] = code as u8;
                    outptr += 1;
                } else {
                    break;
                }
            }

            if outptr < outend
                || (dsd.byteptr < dsd.endptr && {
                    let b = byte_at(&dsd.data, dsd.byteptr);
                    dsd.byteptr += 1;
                    b != 0
                })
            {
                break 'tables false;
            }
        } else if (dsd.endptr - dsd.byteptr) as i64 > 256 * bins as i64 {
            for (bi, row) in probabilities.iter_mut().enumerate() {
                let at = dsd.byteptr + bi * 256;
                row.copy_from_slice(&dsd.data[at..at + 256]);
            }
            dsd.byteptr += 256 * bins;
        } else {
            break 'tables false;
        }

        for bi in 0..bins {
            let mut sum_values = 0i32;

            for i in 0..256 {
                sum_values += probabilities[bi][i] as i32;
                summed_probabilities[bi][i] = sum_values as u16;
            }

            if sum_values != 0 {
                total_summed_probabilities += sum_values;
                if total_summed_probabilities > dsd.history_bins * MAX_BYTES_PER_BIN {
                    break 'tables false;
                }

                value_lookup[bi] = Some(lb_ptr);

                for i in 0..256 {
                    let c = probabilities[bi][i];

                    for _ in 0..c {
                        lookup_buffer[lb_ptr] = i as u8;
                        lb_ptr += 1;
                    }
                }
            }
        }

        true
    };

    dsd.lookup_buffer = Some(lookup_buffer);
    dsd.value_lookup = Some(value_lookup);
    dsd.summed_probabilities = Some(summed_probabilities);
    dsd.probabilities = Some(probabilities);

    if !ok {
        return FALSE;
    }

    if (dsd.endptr - dsd.byteptr) < 4
        || total_summed_probabilities > dsd.history_bins * MAX_BYTES_PER_BIN
    {
        return FALSE;
    }

    for _ in 0..4 {
        dsd.value = (dsd.value << 8) | byte_at(&dsd.data, dsd.byteptr) as u32;
        dsd.byteptr += 1;
    }

    dsd.p0 = 0;
    dsd.p1 = 0;
    dsd.low = 0;
    dsd.high = 0xffffffff;
    dsd.ready = 1;

    TRUE
}

/// Translation of `decode_fast()`.
fn decode_fast(wps: &mut WavpackStream, output: &mut [i32], sample_count: i32) -> i32 {
    let mut total_samples = sample_count;
    let mono = (wps.wphdr.flags & MONO_DATA) != 0;
    let dsd = &mut wps.dsd;
    let crc = &mut wps.crc;

    let (Some(probabilities), Some(summed_probabilities), Some(value_lookup), Some(lookup_buffer)) = (
        dsd.probabilities.as_ref(),
        dsd.summed_probabilities.as_ref(),
        dsd.value_lookup.as_ref(),
        dsd.lookup_buffer.as_ref(),
    ) else {
        // (only after a failed seek: see free_single_stream())
        return 0;
    };

    if !mono {
        total_samples *= 2;
    }

    let mut out = 0usize;
    while total_samples > 0 {
        total_samples -= 1;
        let mut mult: u32;
        let index: u32;
        let code: u32;
        let p0 = dsd.p0 as usize;
        let total = summed_probabilities[p0][255] as u32;

        if total == 0 {
            return 0;
        }

        mult = dsd.high.wrapping_sub(dsd.low) / total;

        if mult == 0 {
            if dsd.endptr - dsd.byteptr >= 4 {
                for _ in 0..4 {
                    dsd.value = (dsd.value << 8) | byte_at(&dsd.data, dsd.byteptr) as u32;
                    dsd.byteptr += 1;
                }
            }

            dsd.low = 0;
            dsd.high = 0xffffffff;
            mult = dsd.high / total;

            if mult == 0 {
                return 0;
            }
        }

        index = dsd.value.wrapping_sub(dsd.low) / mult;

        if index >= total {
            return 0;
        }

        code = value_lookup[p0]
            .and_then(|at| lookup_buffer.get(at + index as usize))
            .copied()
            .unwrap_or(0) as u32;
        output[out] = code as i32;
        out += 1;
        if code != 0 {
            dsd.low = dsd.low.wrapping_add(
                (summed_probabilities[p0][(code - 1) as usize] as u32).wrapping_mul(mult),
            );
        }

        dsd.high = dsd
            .low
            .wrapping_add((probabilities[p0][code as usize] as u32).wrapping_mul(mult))
            .wrapping_sub(1);
        *crc = crc.wrapping_add(*crc << 1).wrapping_add(code);

        if mono {
            dsd.p0 = (code & (dsd.history_bins - 1) as u32) as i32;
        } else {
            dsd.p0 = dsd.p1;
            dsd.p1 = (code & (dsd.history_bins - 1) as u32) as i32;
        }

        while dsd_byte_ready(dsd.high, dsd.low) && dsd.byteptr < dsd.endptr {
            dsd.value = (dsd.value << 8) | byte_at(&dsd.data, dsd.byteptr) as u32;
            dsd.byteptr += 1;
            dsd.high = (dsd.high << 8) | 0xff;
            dsd.low <<= 8;
        }
    }

    sample_count
}

/*------------------------------------------------------------------------------------------------------------------------*/

const PTABLE_BITS: i32 = 8;
const PTABLE_BINS: usize = 1 << PTABLE_BITS;
const PTABLE_MASK: i32 = PTABLE_BINS as i32 - 1;

const UP: i32 = 0x010000fe;
const DOWN: i32 = 0x00010000;
const DECAY: i32 = 8;

const PRECISION: i32 = 20;
const VALUE_ONE: i32 = 1 << PRECISION;
const PRECISION_USE: i32 = 12;

const RATE_S: i32 = 20;

/// Translation of `init_ptable()`.
fn init_ptable(table: &mut [i32], rate_i: i32, rate_s: i32) {
    let mut value: i32 = 0x808000;
    let mut rate: i32 = rate_i << 8;
    let mut c: i32;

    // for (c = (rate + 128) >> 8; c--;)
    c = rate.wrapping_add(128) >> 8;
    while c != 0 {
        c = c.wrapping_sub(1);
        value = value.wrapping_add(DOWN.wrapping_sub(value) >> DECAY);
    }

    for i in 0..PTABLE_BINS / 2 {
        table[i] = value;
        table[PTABLE_BINS - 1 - i] = 0x100ffff - value;

        if value > 0x010000 {
            rate = rate.wrapping_add(rate.wrapping_mul(rate_s).wrapping_add(128) >> 8);

            // for (c = (rate + 64) >> 7; c--;)
            c = rate.wrapping_add(64) >> 7;
            while c != 0 {
                c = c.wrapping_sub(1);
                value = value.wrapping_add(DOWN.wrapping_sub(value) >> DECAY);
            }
        }
    }
}

/// Translation of `init_dsd_block_high()`.
fn init_dsd_block_high(wps: &mut WavpackStream) -> i32 {
    let flags = wps.wphdr.flags;
    let rate_i: i32;
    let rate_s: i32;
    let dsd = &mut wps.dsd;

    if ((dsd.endptr - dsd.byteptr) as i32) < if (flags & MONO_DATA) != 0 { 13 } else { 20 } {
        return FALSE;
    }

    let mut next = || {
        let b = byte_at(&dsd.data, dsd.byteptr);
        dsd.byteptr += 1;
        b
    };

    rate_i = next() as i32;
    rate_s = next() as i32;

    if rate_s != RATE_S {
        return FALSE;
    }

    let channels = if (flags & MONO_DATA) != 0 { 1 } else { 2 };
    let mut filters = [DsdFilters::default(); 2];
    for sp in filters[..channels].iter_mut() {
        sp.filter1 = (next() as i32) << (PRECISION - 8);
        sp.filter2 = (next() as i32) << (PRECISION - 8);
        sp.filter3 = (next() as i32) << (PRECISION - 8);
        sp.filter4 = (next() as i32) << (PRECISION - 8);
        sp.filter5 = (next() as i32) << (PRECISION - 8);
        sp.factor = next() as i32 & 0xff;
        sp.factor |= ((next() as i32) << 8) & 0xff00;
    }

    let mut value = dsd.value;
    for _ in 0..4 {
        value = (value << 8) | next() as u32;
    }

    if dsd.ptable.is_none() {
        dsd.ptable = Some(vec![0; PTABLE_BINS]);
    }

    if let Some(ptable) = dsd.ptable.as_mut() {
        init_ptable(ptable, rate_i, rate_s);
    }

    for (channel, sp) in filters[..channels].iter().enumerate() {
        let dst = &mut dsd.filters[channel];
        dst.filter1 = sp.filter1;
        dst.filter2 = sp.filter2;
        dst.filter3 = sp.filter3;
        dst.filter4 = sp.filter4;
        dst.filter5 = sp.filter5;
        dst.filter6 = 0;
        dst.factor = (((sp.factor as u32) << 16) as i32) >> 16;
    }

    dsd.high = 0xffffffff;
    dsd.low = 0x0;
    dsd.value = value;

    dsd.ready = 1;

    TRUE
}

/// One bit of `decode_high()` for channel `sp`.
#[inline]
fn decode_high_bit(dsd: &mut DsdData, ptable: &mut [i32], ch: usize) {
    let sp = &mut dsd.filters[ch];
    let pp = &mut ptable[((sp.value >> (PRECISION - PRECISION_USE)) & PTABLE_MASK) as usize];
    let split = dsd
        .low
        .wrapping_add((dsd.high.wrapping_sub(dsd.low) >> 8).wrapping_mul((*pp >> 16) as u32));

    if dsd.value <= split {
        dsd.high = split;
        *pp = pp.wrapping_add(UP.wrapping_sub(*pp) >> DECAY);
        sp.filter0 = -1;
    } else {
        dsd.low = split.wrapping_add(1);
        *pp = pp.wrapping_add(DOWN.wrapping_sub(*pp) >> DECAY);
        sp.filter0 = 0;
    }

    while dsd_byte_ready(dsd.high, dsd.low) && dsd.byteptr < dsd.endptr {
        dsd.value = (dsd.value << 8) | byte_at(&dsd.data, dsd.byteptr) as u32;
        dsd.byteptr += 1;
        dsd.high = (dsd.high << 8) | 0xff;
        dsd.low <<= 8;
    }

    let sp = &mut dsd.filters[ch];
    sp.value = sp.value.wrapping_add(sp.filter6.wrapping_mul(8));
    sp.byte = (sp.byte << 1) | (sp.filter0 & 1) as u32;
    sp.factor = sp.factor.wrapping_add(
        (((sp.value ^ sp.filter0) >> 31) | 1)
            & ((sp.value ^ sp.value.wrapping_sub(sp.filter6.wrapping_mul(16))) >> 31),
    );
    sp.filter1 = sp
        .filter1
        .wrapping_add(((sp.filter0 & VALUE_ONE).wrapping_sub(sp.filter1)) >> 6);
    sp.filter2 = sp
        .filter2
        .wrapping_add(((sp.filter0 & VALUE_ONE).wrapping_sub(sp.filter2)) >> 4);
    sp.filter3 = sp
        .filter3
        .wrapping_add(sp.filter2.wrapping_sub(sp.filter3) >> 4);
    sp.filter4 = sp
        .filter4
        .wrapping_add(sp.filter3.wrapping_sub(sp.filter4) >> 4);
    sp.value = sp.filter4.wrapping_sub(sp.filter5) >> 4;
    sp.filter5 = sp.filter5.wrapping_add(sp.value);
    sp.filter6 = sp
        .filter6
        .wrapping_add(sp.value.wrapping_sub(sp.filter6) >> 3);
    sp.value = sp
        .filter1
        .wrapping_sub(sp.filter5)
        .wrapping_add(sp.filter6.wrapping_mul(sp.factor) >> 2);
}

/// Translation of `decode_high()`.
fn decode_high(wps: &mut WavpackStream, output: &mut [i32], sample_count: i32) -> i32 {
    let mut total_samples = sample_count;
    let stereo = (wps.wphdr.flags & MONO_DATA) == 0;
    let mut out = 0usize;

    let Some(mut ptable) = wps.dsd.ptable.take() else {
        // (only after a failed seek: see free_single_stream())
        return 0;
    };

    while total_samples > 0 {
        total_samples -= 1;
        let mut bitcount = 8;
        let dsd = &mut wps.dsd;

        let sp = &mut dsd.filters[0];
        sp.value = sp
            .filter1
            .wrapping_sub(sp.filter5)
            .wrapping_add(sp.filter6.wrapping_mul(sp.factor) >> 2);

        if stereo {
            let sp = &mut dsd.filters[1];
            sp.value = sp
                .filter1
                .wrapping_sub(sp.filter5)
                .wrapping_add(sp.filter6.wrapping_mul(sp.factor) >> 2);
        }

        while bitcount > 0 {
            bitcount -= 1;
            decode_high_bit(dsd, &mut ptable, 0);

            if !stereo {
                continue;
            }

            decode_high_bit(dsd, &mut ptable, 1);
        }

        let byte = (dsd.filters[0].byte & 0xff) as i32;
        output[out] = byte;
        out += 1;
        wps.crc = wps.crc.wrapping_add(wps.crc << 1).wrapping_add(byte as u32);
        let sp = &mut wps.dsd.filters[0];
        sp.factor = sp.factor.wrapping_sub(sp.factor.wrapping_add(512) >> 10);

        if stereo {
            let byte = (wps.dsd.filters[1].byte & 0xff) as i32;
            output[out] = byte;
            out += 1;
            wps.crc = wps.crc.wrapping_add(wps.crc << 1).wrapping_add(byte as u32);
            let sp = &mut wps.dsd.filters[1];
            sp.factor = sp.factor.wrapping_sub(sp.factor.wrapping_add(512) >> 10);
        }
    }

    wps.dsd.ptable = Some(ptable);

    sample_count
}
