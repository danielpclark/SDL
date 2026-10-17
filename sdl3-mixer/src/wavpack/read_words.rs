// Rust translation of src/read_words.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The entropy decoder, with `USE_NEXT8_OPTIMIZATION` and
//! `USE_BITMASK_TABLES` (what a GCC build on Linux picks: CMake doesn't
//! define `HAVE___BUILTIN_CTZ`, so `USE_CTZ_OPTIMIZATION` is only on for
//! `_WIN64`).

// This module provides entropy word decoding functions using
// a variation on the Rice method.  This was introduced in version 3.93
// because it allows splitting the data into a "lossy" stream and a
// "correction" stream in a very efficient manner and is therefore ideal
// for the "hybrid" mode.  For 4.0, the efficiency of this method was
// significantly improved by moving away from the normal Rice restriction of
// using powers of two for the modulus divisions and now the method can be
// used for both hybrid and pure lossless encoding.

// Samples are divided by median probabilities at 5/7 (71.43%), 10/49 (20.41%),
// and 20/343 (5.83%). Each zone has 3.5 times fewer samples than the
// previous. Using standard Rice coding on this data would result in 1.4
// bits per sample average (not counting sign bit). However, there is a
// very simple encoding that is over 99% efficient with this data and
// results in about 1.22 bits per sample.

use super::entropy_utils::{update_error_limit, wp_log2, BITMASK, BITSET};
use super::*;

///////////////////////////// local table storage ////////////////////////////

static ONES_COUNT_TABLE: [u8; 256] = [
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 5,
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 6,
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 5,
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 7,
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 5,
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 6,
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 5,
    0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 4, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0, 8,
];

///////////////////////////// executable code ////////////////////////////////

/// The Elias gamma-like count both decoders read after a run of ones:
/// `cbits` ones, a zero and `cbits - 1` bits, or `None` for 33 ones.
#[inline]
fn read_count(bs: &mut Bitstream) -> Option<u32> {
    let mut cbits = 0;
    while cbits < 33 && getbit(bs) != 0 {
        cbits += 1;
    }

    if cbits == 33 {
        return None;
    }

    if cbits < 2 {
        Some(cbits)
    } else {
        let mut mask = 1u32;
        let mut value = 0u32;
        loop {
            cbits -= 1;
            if cbits == 0 {
                break;
            }
            if getbit(bs) != 0 {
                value |= mask;
            }
            mask <<= 1;
        }

        Some(value | mask)
    }
}

/// The ones count the two decoders read with `USE_NEXT8_OPTIMIZATION`, or
/// `None` at the end of the bitstream (all 1s).
#[inline]
fn read_ones_count(bs: &mut Bitstream) -> Option<u32> {
    let next8: u32;
    let mut ones_count: u32;

    if bs.bc < 8 {
        bs.advance();
        bs.sr |= bs.word() << bs.bc;
        next8 = bs.sr & 0xff;
        bs.bc += 16;
    } else {
        next8 = bs.sr & 0xff;
    }

    if next8 == 0xff {
        bs.bc -= 8;
        bs.sr >>= 8;

        ones_count = 8;
        while ones_count < (LIMIT_ONES + 1) && getbit(bs) != 0 {
            ones_count += 1;
        }

        if ones_count == (LIMIT_ONES + 1) {
            return None;
        }

        if ones_count == LIMIT_ONES {
            ones_count = read_count(bs)?;
            ones_count += LIMIT_ONES;
        }
    } else {
        ones_count = ONES_COUNT_TABLE[next8 as usize] as u32;
        bs.bc -= ones_count as i32 + 1;
        bs.sr >>= ones_count + 1;
    }

    Some(ones_count)
}

/// The `low` and `high` of the ones count's zone, with the medians
/// updated.
#[inline]
fn zone(c: &mut EntropyData, ones_count: u32) -> (u32, u32) {
    let mut low: u32;
    let high: u32;

    if ones_count == 0 {
        low = 0;
        high = get_med(c, 0).wrapping_sub(1);
        dec_med0(c);
    } else {
        low = get_med(c, 0);
        inc_med0(c);

        if ones_count == 1 {
            high = low.wrapping_add(get_med(c, 1)).wrapping_sub(1);
            dec_med1(c);
        } else {
            low = low.wrapping_add(get_med(c, 1));
            inc_med1(c);

            if ones_count == 2 {
                high = low.wrapping_add(get_med(c, 2)).wrapping_sub(1);
                dec_med2(c);
            } else {
                low = low.wrapping_add((ones_count - 2).wrapping_mul(get_med(c, 2)));
                high = low.wrapping_add(get_med(c, 2)).wrapping_sub(1);
                inc_med2(c);
            }
        }
    }

    (low, high)
}

// Read the next word from the bitstream "wvbits" and return the value. This
// function can be used for hybrid or lossless streams, but since an
// optimized version is available for lossless this function would normally
// be used for hybrid only. If a hybrid lossless stream is being read then
// the "correction" offset is written at the specified pointer. A return value
// of WORD_EOF indicates that the end of the bitstream was reached (all 1s) or
// some other error occurred.

/// Translation of `get_word()`.
pub(crate) fn get_word(
    wps: &mut WavpackStream,
    chan: usize,
    mut correction: Option<&mut i32>,
) -> i32 {
    let mut ones_count: u32;
    let mut low: u32;
    let mut mid: u32;
    let mut high: u32;
    let value: u32;

    if !wps.wvbits.open {
        return WORD_EOF;
    }

    if let Some(correction) = correction.as_deref_mut() {
        *correction = 0;
    }

    if (wps.w.c[0].median[0] & !1) == 0
        && wps.w.holding_zero == 0
        && wps.w.holding_one == 0
        && (wps.w.c[1].median[0] & !1) == 0
    {
        if wps.w.zeros_acc != 0 {
            wps.w.zeros_acc -= 1;
            if wps.w.zeros_acc != 0 {
                let c = &mut wps.w.c[chan];
                c.slow_level = c
                    .slow_level
                    .wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
                return 0;
            }
        } else {
            match read_count(&mut wps.wvbits) {
                None => return WORD_EOF,
                Some(count) => wps.w.zeros_acc = count,
            }

            if wps.w.zeros_acc != 0 {
                let c = &mut wps.w.c[chan];
                c.slow_level = c
                    .slow_level
                    .wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
                wps.w.c[0].median = [0; 3];
                wps.w.c[1].median = [0; 3];
                return 0;
            }
        }
    }

    if wps.w.holding_zero != 0 {
        wps.w.holding_zero = 0;
        ones_count = 0;
    } else {
        match read_ones_count(&mut wps.wvbits) {
            None => return WORD_EOF,
            Some(count) => ones_count = count,
        }

        if wps.w.holding_one != 0 {
            wps.w.holding_one = ones_count & 1;
            ones_count = (ones_count >> 1) + 1;
        } else {
            wps.w.holding_one = ones_count & 1;
            ones_count >>= 1;
        }

        wps.w.holding_zero = (!wps.w.holding_one & 1) as i32;
    }

    if (wps.wphdr.flags & HYBRID_FLAG) != 0 && chan == 0 {
        update_error_limit(wps);
    }

    (low, high) = zone(&mut wps.w.c[chan], ones_count);

    low &= 0x7fffffff;
    high &= 0x7fffffff;

    if low > high {
        // make sure high and low make sense
        high = low;
    }

    mid = (high + low + 1) >> 1;

    let error_limit = wps.w.c[chan].error_limit;
    if error_limit == 0 {
        mid = read_code(&mut wps.wvbits, high - low).wrapping_add(low);
    } else {
        while high - low > error_limit {
            if getbit(&mut wps.wvbits) != 0 {
                low = mid;
                mid = (high + low + 1) >> 1;
            } else {
                high = mid.wrapping_sub(1);
                mid = (high.wrapping_add(low).wrapping_add(1)) >> 1;
            }
        }
    }

    let sign = getbit(&mut wps.wvbits);

    if bs_is_open(&wps.wvcbits) && error_limit != 0 {
        value = read_code(&mut wps.wvcbits, high.wrapping_sub(low)).wrapping_add(low);

        if let Some(correction) = correction {
            *correction = if sign != 0 {
                mid.wrapping_sub(value)
            } else {
                value.wrapping_sub(mid)
            } as i32;
        }
    }

    if (wps.wphdr.flags & HYBRID_BITRATE) != 0 {
        let c = &mut wps.w.c[chan];
        c.slow_level = c
            .slow_level
            .wrapping_sub(c.slow_level.wrapping_add(SLO) >> SLS);
        c.slow_level = c.slow_level.wrapping_add(wp_log2(mid) as u32);
    }

    if sign != 0 {
        !mid as i32
    } else {
        mid as i32
    }
}

// This is an optimized version of get_word() that is used for lossless only
// (error_limit == 0). Also, rather than obtaining a single sample, it can be
// used to obtain an entire buffer of either mono or stereo samples.

/// Translation of `get_words_lossless()`.
pub(crate) fn get_words_lossless(
    wps: &mut WavpackStream,
    buffer: &mut [i32],
    mut nsamples: i32,
) -> i32 {
    let mut c = 0usize;
    let mut ones_count: u32;
    let mut low: u32;
    let mut high: u32;
    let mut csamples: i32;
    let mono = (wps.wphdr.flags & MONO_DATA) != 0;

    if nsamples != 0 && !wps.wvbits.open {
        let n = if mono { nsamples } else { nsamples * 2 } as usize;
        buffer[..n].fill(0);
        return nsamples;
    }

    if !mono {
        nsamples *= 2;
    }

    let bs = &mut wps.wvbits;
    let w = &mut wps.w;

    csamples = 0;
    'samples: while csamples < nsamples {
        'next: {
            if !mono {
                c = (csamples & 1) as usize;
            }

            if w.holding_zero != 0 {
                w.holding_zero = 0;
                low = read_code(bs, get_med(&w.c[c], 0).wrapping_sub(1));
                dec_med0(&mut w.c[c]);
                buffer[csamples as usize] = if getbit(bs) != 0 { !low } else { low } as i32;

                csamples += 1;
                if csamples == nsamples {
                    break 'samples;
                }

                if !mono {
                    c = (csamples & 1) as usize;
                }
            }

            if w.c[0].median[0] < 2 && w.holding_one == 0 && w.c[1].median[0] < 2 {
                if w.zeros_acc != 0 {
                    w.zeros_acc -= 1;
                    if w.zeros_acc != 0 {
                        buffer[csamples as usize] = 0;
                        break 'next;
                    }
                } else {
                    match read_count(bs) {
                        None => break 'samples,
                        Some(count) => w.zeros_acc = count,
                    }

                    if w.zeros_acc != 0 {
                        w.c[0].median = [0; 3];
                        w.c[1].median = [0; 3];
                        buffer[csamples as usize] = 0;
                        break 'next;
                    }
                }
            }

            match read_ones_count(bs) {
                None => break 'samples,
                Some(count) => ones_count = count,
            }

            low = w.holding_one;
            w.holding_one = ones_count & 1;
            w.holding_zero = (!ones_count & 1) as i32;
            ones_count = (ones_count >> 1) + low;

            (low, high) = zone(&mut w.c[c], ones_count);

            low = low.wrapping_add(read_code(bs, high.wrapping_sub(low)));
            buffer[csamples as usize] = if getbit(bs) != 0 { !low } else { low } as i32;
        }
        csamples += 1;
    }

    if mono {
        csamples
    } else {
        csamples / 2
    }
}

// Read a single unsigned value from the specified bitstream with a value
// from 0 to maxcode. If there are exactly a power of two number of possible
// codes then this will read a fixed number of bits; otherwise it reads the
// minimum number of bits and then determines whether another bit is needed
// to define the code.

/// Translation of `read_code()` (with a 64-bit `unsigned long`).
#[inline]
fn read_code(bs: &mut Bitstream, maxcode: u32) -> u32 {
    let mut local_sr: u64;
    let extras: u32;
    let mut code: u32;
    let mut bitcount: i32;

    if maxcode < 2 {
        return if maxcode != 0 { getbit(bs) } else { 0 };
    }

    bitcount = count_bits(maxcode);
    extras = BITSET
        .get(bitcount as usize)
        .copied()
        .unwrap_or(0)
        .wrapping_sub(maxcode)
        .wrapping_sub(1);

    local_sr = bs.sr as u64;

    while bs.bc < bitcount {
        bs.advance();
        local_sr |= (bs.word() as u64) << bs.bc;
        bs.bc += 16;
    }

    code = (local_sr & BITMASK[(bitcount - 1) as usize] as u64) as u32;
    if code >= extras {
        code = (code << 1)
            .wrapping_sub(extras)
            .wrapping_add(((local_sr >> (bitcount - 1)) & 1) as u32);
    } else {
        bitcount -= 1;
    }

    bs.bc -= bitcount;
    bs.sr = (local_sr >> bitcount) as u32;

    code
}
