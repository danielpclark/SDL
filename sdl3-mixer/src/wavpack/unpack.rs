// Rust translation of src/unpack.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Unpacking a stream's samples: the C decorrelation passes (not the
//! `DECORR_STEREO_PASS_CONT`/`DECORR_MONO_PASS_CONT` assembly that
//! `OPT_ASM_X86`, `OPT_ASM_X64` and `OPT_ASM_ARM` select, which compute the
//! same samples), with `LOSSY_MUTE` defined.

// This module actually handles the decompression of the audio data, except for
// the entropy decoding which is handled by the read_words.c module. For better
// efficiency, the conversion is isolated to tight loops that handle an entire
// buffer.

use super::common_utils::wavpack_float_normalize;
use super::read_words::{get_word, get_words_lossless};
use super::unpack_floats::float_values;
use super::*;

// This flag provides the functionality of terminating the decoding and muting
// the output when a lossy sample appears to be corrupt. This is automatic
// for lossless files because a corrupt sample is unambiguous, but for lossy
// data it might be possible for this to falsely trigger (although I have never
// seen it).

// #define LOSSY_MUTE (it is)

/// `labs (value) > limit`, with a 64-bit `long`.
#[inline]
fn labs_gt(value: i32, limit: i32) -> bool {
    (value as i64).abs() > limit as i64
}

///////////////////////////// executable code ////////////////////////////////

// This monster actually unpacks the WavPack bitstream(s) into the specified
// buffer as 32-bit integers or floats (depending on original data). Lossy
// samples will be clipped to their original limits (i.e. 8-bit samples are
// clipped to -128/+127) but are still returned in longs. It is up to the
// caller to potentially reformat this for the final output including any
// multichannel distribution, block alignment or endian compensation. The
// function unpack_init() must have been called and the entire WavPack block
// must still be visible (although wps->blockbuff will not be accessed again).
// For maximum clarity, the function is broken up into segments that handle
// various modes. This makes for a few extra infrequent flag checks, but
// makes the code easier to follow because the nesting does not become so
// deep. For maximum efficiency, the conversion is isolated to tight loops
// that handle an entire buffer. The function returns the total number of
// samples unpacked, which can be less than the number requested if an error
// occurs or the end of the block is reached.

/// Translation of `unpack_samples()`; `wpc` is `wps->wpc`.
pub(crate) fn unpack_samples(
    wpc: &WavpackContext<'_>,
    wps: &mut WavpackStream,
    buffer: &mut [i32],
    mut sample_count: u32,
) -> i32 {
    let flags = wps.wphdr.flags;
    let mut crc = wps.crc;
    let mut i: u32;
    let mut mute_limit = ((1i64 << ((flags & MAG_MASK) >> MAG_LSB)) + 2) as i32;
    let mut correction = [0i32; 2];
    let mut read_word: i32;
    let mut m: usize = 0;
    let num_terms = wps.num_terms as usize;

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

    if wps.mute_error != 0 {
        if wpc.reduced_channels == 1 || wpc.config.num_channels == 1 || (flags & MONO_FLAG) != 0 {
            buffer[..sample_count as usize].fill(0);
        } else {
            buffer[..sample_count as usize * 2].fill(0);
        }

        wps.sample_index += sample_count as i64;
        return sample_count as i32;
    }

    if (flags & HYBRID_FLAG) != 0 && wps.block2buff.is_none() {
        mute_limit = mute_limit.wrapping_mul(2).wrapping_add(128);
    }

    'get_word_eof: {
        //////////////// handle lossless or hybrid lossy mono data /////////////////

        if wps.block2buff.is_none() && (flags & MONO_DATA) != 0 {
            let eptr = sample_count as usize;

            if (flags & HYBRID_FLAG) != 0 {
                i = sample_count;

                for bptr in 0..eptr {
                    buffer[bptr] = get_word(wps, 0, None);
                    if buffer[bptr] == WORD_EOF {
                        i = (bptr + 1) as u32;
                        break;
                    }
                }
            } else {
                i = get_words_lossless(wps, buffer, sample_count as i32) as u32;
            }

            if i != sample_count {
                break 'get_word_eof;
            }

            for dpp in wps.decorr_passes[..num_terms].iter_mut() {
                decorr_mono_pass(dpp, &mut buffer[..eptr]);
            }

            for bptr in 0..eptr {
                if labs_gt(buffer[bptr], mute_limit) {
                    i = bptr as u32;
                    break;
                }

                crc = crc.wrapping_mul(3).wrapping_add(buffer[bptr] as u32);
            }
        }
        /////////////// handle lossless or hybrid lossy stereo data ///////////////
        else if wps.block2buff.is_none() && (flags & MONO_DATA) == 0 {
            let eptr = sample_count as usize * 2;

            if (flags & HYBRID_FLAG) != 0 {
                i = sample_count;

                let mut bptr = 0;
                while bptr < eptr {
                    buffer[bptr] = get_word(wps, 0, None);
                    let eof = buffer[bptr] == WORD_EOF || {
                        buffer[bptr + 1] = get_word(wps, 1, None);
                        buffer[bptr + 1] == WORD_EOF
                    };
                    if eof {
                        i = (bptr / 2) as u32;
                        break;
                    }
                    bptr += 2;
                }
            } else {
                i = get_words_lossless(wps, buffer, sample_count as i32) as u32;
            }

            if i != sample_count {
                break 'get_word_eof;
            }

            for dpp in wps.decorr_passes[..num_terms].iter_mut() {
                decorr_stereo_pass(dpp, &mut buffer[..eptr]);
            }

            m = (sample_count & (MAX_TERM as u32 - 1)) as usize;

            if (flags & JOINT_STEREO) != 0 {
                for pair in buffer[..eptr].chunks_exact_mut(2) {
                    pair[1] = pair[1].wrapping_sub(pair[0] >> 1);
                    pair[0] = pair[0].wrapping_add(pair[1]);
                    crc = crc
                        .wrapping_add(crc << 3)
                        .wrapping_add((pair[0] as u32) << 1)
                        .wrapping_add(pair[0] as u32)
                        .wrapping_add(pair[1] as u32);
                }
            } else {
                for pair in buffer[..eptr].chunks_exact(2) {
                    crc = crc
                        .wrapping_add(crc << 3)
                        .wrapping_add((pair[0] as u32) << 1)
                        .wrapping_add(pair[0] as u32)
                        .wrapping_add(pair[1] as u32);
                }
            }

            let mut bptr = 0;
            while bptr < eptr {
                if labs_gt(buffer[bptr], mute_limit) || labs_gt(buffer[bptr + 1], mute_limit) {
                    i = (bptr / 2) as u32;
                    break;
                }
                bptr += 16;
            }
        }
        /////////////////// handle hybrid lossless mono data ////////////////////
        else if (flags & HYBRID_FLAG) != 0 && (flags & MONO_DATA) != 0 {
            let mut bptr = 0;
            i = 0;
            while i < sample_count {
                read_word = get_word(wps, 0, Some(&mut correction[0]));
                if read_word == WORD_EOF {
                    break;
                }

                for dpp in wps.decorr_passes[..num_terms].iter_mut() {
                    let sam: i32;
                    let k: usize;

                    if dpp.term > MAX_TERM {
                        if (dpp.term & 1) != 0 {
                            sam = (2i32.wrapping_mul(dpp.samples_a[0]))
                                .wrapping_sub(dpp.samples_a[1]);
                        } else {
                            sam = (3i32.wrapping_mul(dpp.samples_a[0]))
                                .wrapping_sub(dpp.samples_a[1])
                                >> 1;
                        }

                        dpp.samples_a[1] = dpp.samples_a[0];
                        k = 0;
                    } else {
                        sam = dpp.samples_a[m];
                        k = ((m as i32 + dpp.term) & (MAX_TERM - 1)) as usize;
                    }

                    let temp = apply_weight(dpp.weight_a, sam).wrapping_add(read_word);
                    update_weight(&mut dpp.weight_a, dpp.delta, sam, read_word);
                    read_word = temp;
                    dpp.samples_a[k] = temp;
                }

                m = (m + 1) & (MAX_TERM as usize - 1);

                if (flags & HYBRID_SHAPE) != 0 {
                    wps.dc.shaping_acc[0] =
                        wps.dc.shaping_acc[0].wrapping_add(wps.dc.shaping_delta[0]);
                    let shaping_weight = wps.dc.shaping_acc[0] >> 16;
                    let mut temp = apply_weight(shaping_weight, wps.dc.error[0]).wrapping_neg();

                    if (flags & NEW_SHAPING) != 0 && shaping_weight < 0 && temp != 0 {
                        if temp == wps.dc.error[0] {
                            temp = if temp < 0 { temp + 1 } else { temp - 1 };
                        }

                        wps.dc.error[0] = temp.wrapping_sub(correction[0]);
                    } else {
                        wps.dc.error[0] = correction[0].wrapping_neg();
                    }

                    read_word = read_word.wrapping_add(correction[0].wrapping_sub(temp));
                } else {
                    read_word = read_word.wrapping_add(correction[0]);
                }

                crc = crc.wrapping_add(crc << 1).wrapping_add(read_word as u32);

                if labs_gt(read_word, mute_limit) {
                    break;
                }

                buffer[bptr] = read_word;
                bptr += 1;
                i += 1;
            }
        }
        //////////////////// handle hybrid lossless stereo data ///////////////////
        else if wps.block2buff.is_some() && (flags & MONO_DATA) == 0 {
            let mut bptr = 0;
            i = 0;
            while i < sample_count {
                let mut left: i32;
                let mut right: i32;
                let mut left2: i32;
                let mut right2: i32;
                let mut left_c: i32 = 0;
                let mut right_c: i32 = 0;

                left = get_word(wps, 0, Some(&mut correction[0]));
                if left == WORD_EOF {
                    break;
                }
                right = get_word(wps, 1, Some(&mut correction[1]));
                if right == WORD_EOF {
                    break;
                }

                if (flags & CROSS_DECORR) != 0 {
                    left_c = left.wrapping_add(correction[0]);
                    right_c = right.wrapping_add(correction[1]);

                    for dpp in wps.decorr_passes[..num_terms].iter() {
                        let sam_a: i32;
                        let sam_b: i32;

                        if dpp.term > 0 {
                            if dpp.term > MAX_TERM {
                                if (dpp.term & 1) != 0 {
                                    sam_a = (2i32.wrapping_mul(dpp.samples_a[0]))
                                        .wrapping_sub(dpp.samples_a[1]);
                                    sam_b = (2i32.wrapping_mul(dpp.samples_b[0]))
                                        .wrapping_sub(dpp.samples_b[1]);
                                } else {
                                    sam_a = (3i32.wrapping_mul(dpp.samples_a[0]))
                                        .wrapping_sub(dpp.samples_a[1])
                                        >> 1;
                                    sam_b = (3i32.wrapping_mul(dpp.samples_b[0]))
                                        .wrapping_sub(dpp.samples_b[1])
                                        >> 1;
                                }
                            } else {
                                sam_a = dpp.samples_a[m];
                                sam_b = dpp.samples_b[m];
                            }

                            left_c = left_c.wrapping_add(apply_weight(dpp.weight_a, sam_a));
                            right_c = right_c.wrapping_add(apply_weight(dpp.weight_b, sam_b));
                        } else if dpp.term == -1 {
                            left_c =
                                left_c.wrapping_add(apply_weight(dpp.weight_a, dpp.samples_a[0]));
                            right_c = right_c.wrapping_add(apply_weight(dpp.weight_b, left_c));
                        } else {
                            right_c =
                                right_c.wrapping_add(apply_weight(dpp.weight_b, dpp.samples_b[0]));

                            if dpp.term == -3 {
                                left_c = left_c
                                    .wrapping_add(apply_weight(dpp.weight_a, dpp.samples_a[0]));
                            } else {
                                left_c = left_c.wrapping_add(apply_weight(dpp.weight_a, right_c));
                            }
                        }
                    }

                    if (flags & JOINT_STEREO) != 0 {
                        right_c = right_c.wrapping_sub(left_c >> 1);
                        left_c = left_c.wrapping_add(right_c);
                    }
                }

                for dpp in wps.decorr_passes[..num_terms].iter_mut() {
                    let sam_a: i32;
                    let sam_b: i32;

                    if dpp.term > 0 {
                        let k: usize;

                        if dpp.term > MAX_TERM {
                            if (dpp.term & 1) != 0 {
                                sam_a = (2i32.wrapping_mul(dpp.samples_a[0]))
                                    .wrapping_sub(dpp.samples_a[1]);
                                sam_b = (2i32.wrapping_mul(dpp.samples_b[0]))
                                    .wrapping_sub(dpp.samples_b[1]);
                            } else {
                                sam_a = (3i32.wrapping_mul(dpp.samples_a[0]))
                                    .wrapping_sub(dpp.samples_a[1])
                                    >> 1;
                                sam_b = (3i32.wrapping_mul(dpp.samples_b[0]))
                                    .wrapping_sub(dpp.samples_b[1])
                                    >> 1;
                            }

                            dpp.samples_a[1] = dpp.samples_a[0];
                            dpp.samples_b[1] = dpp.samples_b[0];
                            k = 0;
                        } else {
                            sam_a = dpp.samples_a[m];
                            sam_b = dpp.samples_b[m];
                            k = ((m as i32 + dpp.term) & (MAX_TERM - 1)) as usize;
                        }

                        left2 = apply_weight(dpp.weight_a, sam_a).wrapping_add(left);
                        right2 = apply_weight(dpp.weight_b, sam_b).wrapping_add(right);

                        update_weight(&mut dpp.weight_a, dpp.delta, sam_a, left);
                        update_weight(&mut dpp.weight_b, dpp.delta, sam_b, right);

                        left = left2;
                        right = right2;
                        dpp.samples_a[k] = left;
                        dpp.samples_b[k] = right;
                    } else if dpp.term == -1 {
                        left2 = left.wrapping_add(apply_weight(dpp.weight_a, dpp.samples_a[0]));
                        update_weight_clip(&mut dpp.weight_a, dpp.delta, dpp.samples_a[0], left);
                        left = left2;
                        right2 = right.wrapping_add(apply_weight(dpp.weight_b, left2));
                        update_weight_clip(&mut dpp.weight_b, dpp.delta, left2, right);
                        right = right2;
                        dpp.samples_a[0] = right;
                    } else {
                        right2 = right.wrapping_add(apply_weight(dpp.weight_b, dpp.samples_b[0]));
                        update_weight_clip(&mut dpp.weight_b, dpp.delta, dpp.samples_b[0], right);
                        right = right2;

                        if dpp.term == -3 {
                            right2 = dpp.samples_a[0];
                            dpp.samples_a[0] = right;
                        }

                        left2 = left.wrapping_add(apply_weight(dpp.weight_a, right2));
                        update_weight_clip(&mut dpp.weight_a, dpp.delta, right2, left);
                        left = left2;
                        dpp.samples_b[0] = left;
                    }
                }

                m = (m + 1) & (MAX_TERM as usize - 1);

                if (flags & CROSS_DECORR) == 0 {
                    left_c = left.wrapping_add(correction[0]);
                    right_c = right.wrapping_add(correction[1]);

                    if (flags & JOINT_STEREO) != 0 {
                        right_c = right_c.wrapping_sub(left_c >> 1);
                        left_c = left_c.wrapping_add(right_c);
                    }
                }

                if (flags & JOINT_STEREO) != 0 {
                    right = right.wrapping_sub(left >> 1);
                    left = left.wrapping_add(right);
                }

                if (flags & HYBRID_SHAPE) != 0 {
                    let mut shaping_weight: i32;
                    let mut temp: i32;

                    correction[0] = left_c.wrapping_sub(left);
                    wps.dc.shaping_acc[0] =
                        wps.dc.shaping_acc[0].wrapping_add(wps.dc.shaping_delta[0]);
                    shaping_weight = wps.dc.shaping_acc[0] >> 16;
                    temp = apply_weight(shaping_weight, wps.dc.error[0]).wrapping_neg();

                    if (flags & NEW_SHAPING) != 0 && shaping_weight < 0 && temp != 0 {
                        if temp == wps.dc.error[0] {
                            temp = if temp < 0 { temp + 1 } else { temp - 1 };
                        }

                        wps.dc.error[0] = temp.wrapping_sub(correction[0]);
                    } else {
                        wps.dc.error[0] = correction[0].wrapping_neg();
                    }

                    left = left_c.wrapping_sub(temp);
                    correction[1] = right_c.wrapping_sub(right);
                    wps.dc.shaping_acc[1] =
                        wps.dc.shaping_acc[1].wrapping_add(wps.dc.shaping_delta[1]);
                    shaping_weight = wps.dc.shaping_acc[1] >> 16;
                    temp = apply_weight(shaping_weight, wps.dc.error[1]).wrapping_neg();

                    if (flags & NEW_SHAPING) != 0 && shaping_weight < 0 && temp != 0 {
                        if temp == wps.dc.error[1] {
                            temp = if temp < 0 { temp + 1 } else { temp - 1 };
                        }

                        wps.dc.error[1] = temp.wrapping_sub(correction[1]);
                    } else {
                        wps.dc.error[1] = correction[1].wrapping_neg();
                    }

                    right = right_c.wrapping_sub(temp);
                } else {
                    left = left_c;
                    right = right_c;
                }

                if labs_gt(left, mute_limit) || labs_gt(right, mute_limit) {
                    break;
                }

                crc = crc
                    .wrapping_add(crc << 3)
                    .wrapping_add((left as u32) << 1)
                    .wrapping_add(left as u32)
                    .wrapping_add(right as u32);
                buffer[bptr] = left;
                buffer[bptr + 1] = right;
                bptr += 2;
                i += 1;
            }
        } else {
            i = 0; /* this line can't execute, but suppresses compiler warning */
        }
    }

    // get_word_eof:
    if i != sample_count {
        let n = sample_count as usize * if (flags & MONO_FLAG) != 0 { 1 } else { 2 };
        buffer[..n].fill(0);
        wps.mute_error = TRUE;
        i = sample_count;

        if bs_is_open(&wps.wvxbits) {
            bs_close_read(&mut wps.wvxbits);
        }
    }

    if m != 0 {
        for dpp in wps.decorr_passes[..num_terms].iter_mut() {
            if dpp.term > 0 && dpp.term <= MAX_TERM {
                let temp_a = dpp.samples_a;
                let temp_b = dpp.samples_b;

                for k in 0..MAX_TERM as usize {
                    dpp.samples_a[k] = temp_a[m];
                    dpp.samples_b[k] = temp_b[m];
                    m = (m + 1) & (MAX_TERM as usize - 1);
                }
            }
        }
    }

    fixup_samples(wps, buffer, i);

    if (flags & FLOAT_DATA) != 0 && (wpc.open_flags & OPEN_NORMALIZE) != 0 {
        let n = if (flags & MONO_DATA) != 0 { i } else { i * 2 } as usize;
        wavpack_float_normalize(
            &mut buffer[..n],
            127 - wps.float_norm_exp as i32 + wpc.norm_offset,
        );
    }

    if (flags & FALSE_STEREO) != 0 {
        let mut dptr = i as usize * 2;
        let mut sptr = i as usize;

        for _ in 0..i {
            sptr -= 1;
            dptr -= 1;
            buffer[dptr] = buffer[sptr];
            dptr -= 1;
            buffer[dptr] = buffer[sptr];
        }
    }

    wps.sample_index += i as i64;
    wps.crc = crc;

    // If we just finished this block, then it's time to check if the applicable checksums match. Like
    // other decoding errors, these are indicated by setting the mute_error flag. We also clear the
    // buffer to reduce the chances of noise bursts getting through.

    if wps.mute_error == 0
        && wps.sample_index == get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64
        && (wps.crc != wps.wphdr.crc || (bs_is_open(&wps.wvxbits) && wps.crc_x != wps.crc_wvx))
    {
        let n = sample_count as usize * if (flags & MONO_FLAG) != 0 { 1 } else { 2 };
        buffer[..n].fill(0);
        wps.mute_error = TRUE;
    }

    i as i32
}

/// Translation of `bs_close_read()` (open_utils.c):
/// This function is called to close the bitstream. It returns the number of
/// full bytes actually read as bits.
pub(crate) fn bs_close_read(bs: &mut Bitstream) -> u32 {
    let mut bytes_read: u32;

    if bs.bc < 16 {
        bs.ptr += 1;
    }

    bytes_read = (bs.ptr as u32).wrapping_mul(2);

    if (bytes_read & 1) == 0 {
        bytes_read = bytes_read.wrapping_add(1);
    }

    *bs = Bitstream::default();
    bytes_read
}

// General function to perform mono decorrelation pass on specified buffer
// (although since this is the reverse function it might technically be called
// "correlation" instead). This version handles all sample resolutions and
// weight deltas. The dpp->samples_X[] data is returned normalized for term
// values 1-8.

/// Translation of `decorr_mono_pass()`.
fn decorr_mono_pass(dpp: &mut DecorrPass, buffer: &mut [i32]) {
    let delta = dpp.delta;
    let mut weight_a = dpp.weight_a;
    let mut sam_a: i32;

    match dpp.term {
        17 => {
            for b in buffer.iter_mut() {
                sam_a = (2i32.wrapping_mul(dpp.samples_a[0])).wrapping_sub(dpp.samples_a[1]);
                dpp.samples_a[1] = dpp.samples_a[0];
                dpp.samples_a[0] = apply_weight(weight_a, sam_a).wrapping_add(*b);
                update_weight(&mut weight_a, delta, sam_a, *b);
                *b = dpp.samples_a[0];
            }
        }

        18 => {
            for b in buffer.iter_mut() {
                sam_a = (3i32.wrapping_mul(dpp.samples_a[0])).wrapping_sub(dpp.samples_a[1]) >> 1;
                dpp.samples_a[1] = dpp.samples_a[0];
                dpp.samples_a[0] = apply_weight(weight_a, sam_a).wrapping_add(*b);
                update_weight(&mut weight_a, delta, sam_a, *b);
                *b = dpp.samples_a[0];
            }
        }

        _ => {
            let mut m = 0usize;
            let mut k = (dpp.term & (MAX_TERM - 1)) as usize;
            for b in buffer.iter_mut() {
                sam_a = dpp.samples_a[m];
                dpp.samples_a[k] = apply_weight(weight_a, sam_a).wrapping_add(*b);
                update_weight(&mut weight_a, delta, sam_a, *b);
                *b = dpp.samples_a[k];
                m = (m + 1) & (MAX_TERM as usize - 1);
                k = (k + 1) & (MAX_TERM as usize - 1);
            }

            if m != 0 {
                let temp_samples = dpp.samples_a;

                for k in 0..MAX_TERM as usize {
                    dpp.samples_a[k] = temp_samples[m & (MAX_TERM as usize - 1)];
                    m += 1;
                }
            }
        }
    }

    dpp.weight_a = weight_a;
}

// General function to perform stereo decorrelation pass on specified buffer
// (although since this is the reverse function it might technically be called
// "correlation" instead). This version handles all sample resolutions and
// weight deltas. The dpp->samples_X[] data is *not* returned normalized for
// term values 1-8, so it should be normalized if it is going to be used to
// call this function again.

/// Translation of `decorr_stereo_pass()`.
fn decorr_stereo_pass(dpp: &mut DecorrPass, buffer: &mut [i32]) {
    match dpp.term {
        17 => {
            for b in buffer.chunks_exact_mut(2) {
                let mut sam: i32;
                let mut tmp: i32;

                sam = (2i32.wrapping_mul(dpp.samples_a[0])).wrapping_sub(dpp.samples_a[1]);
                dpp.samples_a[1] = dpp.samples_a[0];
                tmp = b[0];
                dpp.samples_a[0] = apply_weight(dpp.weight_a, sam).wrapping_add(tmp);
                b[0] = dpp.samples_a[0];
                update_weight(&mut dpp.weight_a, dpp.delta, sam, tmp);

                sam = (2i32.wrapping_mul(dpp.samples_b[0])).wrapping_sub(dpp.samples_b[1]);
                dpp.samples_b[1] = dpp.samples_b[0];
                tmp = b[1];
                dpp.samples_b[0] = apply_weight(dpp.weight_b, sam).wrapping_add(tmp);
                b[1] = dpp.samples_b[0];
                update_weight(&mut dpp.weight_b, dpp.delta, sam, tmp);
            }
        }

        18 => {
            for b in buffer.chunks_exact_mut(2) {
                let mut sam: i32;
                let mut tmp: i32;

                sam = dpp.samples_a[0]
                    .wrapping_add(dpp.samples_a[0].wrapping_sub(dpp.samples_a[1]) >> 1);
                dpp.samples_a[1] = dpp.samples_a[0];
                tmp = b[0];
                dpp.samples_a[0] = apply_weight(dpp.weight_a, sam).wrapping_add(tmp);
                b[0] = dpp.samples_a[0];
                update_weight(&mut dpp.weight_a, dpp.delta, sam, tmp);

                sam = dpp.samples_b[0]
                    .wrapping_add(dpp.samples_b[0].wrapping_sub(dpp.samples_b[1]) >> 1);
                dpp.samples_b[1] = dpp.samples_b[0];
                tmp = b[1];
                dpp.samples_b[0] = apply_weight(dpp.weight_b, sam).wrapping_add(tmp);
                b[1] = dpp.samples_b[0];
                update_weight(&mut dpp.weight_b, dpp.delta, sam, tmp);
            }
        }

        -1 => {
            for b in buffer.chunks_exact_mut(2) {
                let sam = b[0].wrapping_add(apply_weight(dpp.weight_a, dpp.samples_a[0]));
                update_weight_clip(&mut dpp.weight_a, dpp.delta, dpp.samples_a[0], b[0]);
                b[0] = sam;
                dpp.samples_a[0] = b[1].wrapping_add(apply_weight(dpp.weight_b, sam));
                update_weight_clip(&mut dpp.weight_b, dpp.delta, sam, b[1]);
                b[1] = dpp.samples_a[0];
            }
        }

        -2 => {
            for b in buffer.chunks_exact_mut(2) {
                let sam = b[1].wrapping_add(apply_weight(dpp.weight_b, dpp.samples_b[0]));
                update_weight_clip(&mut dpp.weight_b, dpp.delta, dpp.samples_b[0], b[1]);
                b[1] = sam;
                dpp.samples_b[0] = b[0].wrapping_add(apply_weight(dpp.weight_a, sam));
                update_weight_clip(&mut dpp.weight_a, dpp.delta, sam, b[0]);
                b[0] = dpp.samples_b[0];
            }
        }

        -3 => {
            for b in buffer.chunks_exact_mut(2) {
                let sam_a = b[0].wrapping_add(apply_weight(dpp.weight_a, dpp.samples_a[0]));
                update_weight_clip(&mut dpp.weight_a, dpp.delta, dpp.samples_a[0], b[0]);
                let sam_b = b[1].wrapping_add(apply_weight(dpp.weight_b, dpp.samples_b[0]));
                update_weight_clip(&mut dpp.weight_b, dpp.delta, dpp.samples_b[0], b[1]);
                dpp.samples_b[0] = sam_a;
                b[0] = sam_a;
                dpp.samples_a[0] = sam_b;
                b[1] = sam_b;
            }
        }

        _ => {
            let mut m = 0usize;
            let mut k = (dpp.term & (MAX_TERM - 1)) as usize;
            for b in buffer.chunks_exact_mut(2) {
                let mut sam: i32;

                sam = dpp.samples_a[m];
                dpp.samples_a[k] = apply_weight(dpp.weight_a, sam).wrapping_add(b[0]);
                update_weight(&mut dpp.weight_a, dpp.delta, sam, b[0]);
                b[0] = dpp.samples_a[k];

                sam = dpp.samples_b[m];
                dpp.samples_b[k] = apply_weight(dpp.weight_b, sam).wrapping_add(b[1]);
                update_weight(&mut dpp.weight_b, dpp.delta, sam, b[1]);
                b[1] = dpp.samples_b[k];

                m = (m + 1) & (MAX_TERM as usize - 1);
                k = (k + 1) & (MAX_TERM as usize - 1);
            }
        }
    }
}

// This is a helper function for unpack_samples() that applies several final
// operations. First, if the data is 32-bit float data, then that conversion
// is done in the float.c module (whether lossy or lossless) and we return.
// Otherwise, if the extended integer data applies, then that operation is
// executed first. If the unpacked data is lossy (and not corrected) then
// it is clipped and shifted in a single operation. Otherwise, if it's
// lossless then the last step is to apply the final shift (if any).

/// Translation of `fixup_samples()`.
fn fixup_samples(wps: &mut WavpackStream, buffer: &mut [i32], mut sample_count: u32) {
    let flags = wps.wphdr.flags;
    let lossy_flag = (flags & HYBRID_FLAG) != 0 && wps.block2buff.is_none();
    let mut shift = ((flags & SHIFT_MASK) >> SHIFT_LSB) as i32;

    if (flags & FLOAT_DATA) != 0 {
        let n = if (flags & MONO_DATA) != 0 {
            sample_count
        } else {
            sample_count * 2
        } as usize;
        float_values(wps, &mut buffer[..n]);
        return;
    }

    if (flags & INT32_DATA) != 0 {
        let count = if (flags & MONO_DATA) != 0 {
            sample_count
        } else {
            sample_count * 2
        } as usize;
        let sent_bits = (wps.int32_sent_bits & 0x1f) as i32;
        let mut zeros = (wps.int32_zeros & 0x1f) as i32;
        let mut ones = (wps.int32_ones & 0x1f) as i32;
        let mut dups = (wps.int32_dups & 0x1f) as i32;
        let mut data: u32;
        let mask = (1u32 << sent_bits).wrapping_sub(1);

        if bs_is_open(&wps.wvxbits) {
            let max_width = wps.int32_max_width as i32;
            let mut crc = wps.crc_x;

            for dptr in buffer[..count].iter_mut() {
                if sent_bits != 0 {
                    if max_width != 0 {
                        let pvalue = if *dptr < 0 { !*dptr } else { *dptr };
                        let width = count_bits(pvalue as u32) + sent_bits;
                        let mut bits_to_read = sent_bits;

                        if width <= max_width || {
                            bits_to_read -= width - max_width;
                            bits_to_read > 0
                        } {
                            data = getbits(bits_to_read, &mut wps.wvxbits);
                            data &= (1u32 << bits_to_read).wrapping_sub(1);
                            *dptr = ((((*dptr as u32) << bits_to_read) | data)
                                << (sent_bits - bits_to_read))
                                as i32;
                        } else {
                            *dptr = ((*dptr as u32) << sent_bits) as i32;
                        }
                    } else {
                        data = getbits(sent_bits, &mut wps.wvxbits);
                        *dptr = (((*dptr as u32) << sent_bits) | (data & mask)) as i32;
                    }
                }

                if zeros != 0 {
                    *dptr = ((*dptr as u32) << zeros) as i32;
                } else if ones != 0 {
                    *dptr = (((dptr.wrapping_add(1)) as u32) << ones).wrapping_sub(1) as i32;
                } else if dups != 0 {
                    *dptr = (((dptr.wrapping_add(*dptr & 1)) as u32) << dups)
                        .wrapping_sub((*dptr & 1) as u32) as i32;
                }

                crc = crc
                    .wrapping_mul(9)
                    .wrapping_add(((*dptr & 0xffff) * 3) as u32)
                    .wrapping_add(((*dptr >> 16) & 0xffff) as u32);
            }

            wps.crc_x = crc;
        } else if sent_bits == 0 && (zeros + ones + dups) != 0 {
            while lossy_flag && (flags & BYTES_STORED) == 3 && shift < 8 {
                if zeros != 0 {
                    zeros -= 1;
                } else if ones != 0 {
                    ones -= 1;
                } else if dups != 0 {
                    dups -= 1;
                } else {
                    break;
                }

                shift += 1;
            }

            for dptr in buffer[..count].iter_mut() {
                if zeros != 0 {
                    *dptr = ((*dptr as u32) << zeros) as i32;
                } else if ones != 0 {
                    *dptr = (((dptr.wrapping_add(1)) as u32) << ones).wrapping_sub(1) as i32;
                } else if dups != 0 {
                    *dptr = (((dptr.wrapping_add(*dptr & 1)) as u32) << dups)
                        .wrapping_sub((*dptr & 1) as u32) as i32;
                }
            }
        } else {
            shift += zeros + sent_bits + ones + dups;
        }
    }

    shift &= 0x1f;

    if lossy_flag {
        let min_value: i32;
        let max_value: i32;
        let min_shifted: i32;
        let max_shifted: i32;

        match flags & BYTES_STORED {
            0 => {
                min_value = -128 >> shift;
                min_shifted = ((min_value as u32) << shift) as i32;
                max_value = 127 >> shift;
                max_shifted = max_value << shift;
            }

            1 => {
                min_value = -32768 >> shift;
                min_shifted = ((min_value as u32) << shift) as i32;
                max_value = 32767 >> shift;
                max_shifted = max_value << shift;
            }

            2 => {
                min_value = -8388608 >> shift;
                min_shifted = ((min_value as u32) << shift) as i32;
                max_value = 8388607 >> shift;
                max_shifted = max_value << shift;
            }

            _ => {
                /* "default" suppresses compiler warning */
                min_value = (0x80000000u32 as i32) >> shift;
                min_shifted = ((min_value as u32) << shift) as i32;
                max_value = 0x7fffffff >> shift;
                max_shifted = max_value << shift;
            }
        }

        if (flags & MONO_DATA) == 0 {
            sample_count *= 2;
        }

        for b in buffer[..sample_count as usize].iter_mut() {
            if *b < min_value {
                *b = min_shifted;
            } else if *b > max_value {
                *b = max_shifted;
            } else {
                *b = ((*b as u32) << shift) as i32;
            }
        }
    } else if shift != 0 {
        if (flags & MONO_DATA) == 0 {
            sample_count *= 2;
        }

        for b in buffer[..sample_count as usize].iter_mut() {
            *b = ((*b as u32) << shift) as i32;
        }
    }
}
