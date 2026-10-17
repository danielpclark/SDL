// Rust translation of src/loaders/lzw.c and src/loaders/lzw.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 2021-2023 Alice Rowan
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Simple LZW decoder for Digital Symphony.
 * This does not handle the hacks required for ARC or UnShrink. */

//! LZW and sigma-delta decompression, for Digital Symphony.

use super::super::hio::*;

/// `LZW_FLAG_MAXBITS(x)`
pub(crate) const fn lzw_flag_maxbits(x: i32) -> i32 {
    x & 15
}
pub(crate) const LZW_FLAG_SYMQUIRKS: i32 = 0x100;

pub(crate) const LZW_FLAGS_SYM: i32 = lzw_flag_maxbits(13) | LZW_FLAG_SYMQUIRKS;

const LZW_NO_CODE: u16 = u16::MAX;
const LZW_CODE_CLEAR: i32 = 256;
const LZW_CODE_SYM_EOF: i32 = 257;

/// Translation of `struct bitstream`.
struct Bitstream {
    buf: u32,
    num_read: usize,
    max_read: usize,
    bits: i32,
}

/// Translation of `struct LZW_code`.
#[derive(Clone, Copy, Default)]
struct LzwCode {
    prev: u16,
    length: u16,
    value: u8,
}

/// Translation of `struct LZW_tree`.
struct LzwTree {
    codes: Vec<LzwCode>,
    bits: u32,
    length: u32,
    maxlength: u32,
    defaultlength: u32,
    alloclength: u32,
    previous_code: u32,
    new_inc: i32,
    flags: i32,
    previous_first_char: u8,
}

/// Translation of `bs_init()`.
fn bs_init(max_read: usize) -> Bitstream {
    Bitstream {
        buf: 0,
        num_read: 0,
        max_read,
        bits: 0,
    }
}

/// Translation of `bs_read()`.
fn bs_read(bs: &mut Bitstream, f: &mut HioHandle<'_>, bits: i32) -> i32 {
    if bs.bits < bits {
        while bs.bits < bits {
            if bs.num_read >= bs.max_read {
                return -1;
            }

            let byte = hio_read8(f);
            bs.buf |= (byte as u32) << bs.bits;
            bs.bits += 8;
            bs.num_read += 1;
        }
        if hio_error(f) != 0 {
            return -1;
        }
    }

    let ret = (bs.buf & ((1u32 << bits) - 1)) as i32;
    bs.buf >>= bits;
    bs.bits -= bits;
    ret
}

/// Translation of `LZW_init_tree()`.
fn lzw_init_tree(flags: i32) -> Option<LzwTree> {
    let maxbits = lzw_flag_maxbits(flags) as u32;

    let bits = 9;
    if maxbits < bits || maxbits > 16 {
        return None;
    }

    let alloclength = 1u32 << maxbits;
    let mut codes = Vec::new();
    codes.try_reserve_exact(alloclength as usize).ok()?;
    codes.resize(alloclength as usize, LzwCode::default());

    for (i, c) in codes.iter_mut().enumerate().take(256) {
        c.length = 1;
        c.value = i as u8;
        c.prev = LZW_NO_CODE;
    }
    Some(LzwTree {
        codes,
        bits,
        length: 258, /* 256 chars + clear + EOF. */
        maxlength: 1 << bits,
        defaultlength: 258,
        alloclength,
        previous_code: LZW_NO_CODE as u32,
        new_inc: 0,
        flags,
        previous_first_char: 0,
    })
}

/// Translation of `LZW_add()`.
fn lzw_add(lzw: &mut LzwTree) {
    if lzw.length >= lzw.alloclength {
        return;
    }

    let current = lzw.length as usize;
    lzw.length += 1;

    /* Increase bitwidth if the NEXT code would be maxlength. */
    if lzw.length >= lzw.maxlength && lzw.length < lzw.alloclength {
        lzw.maxlength <<= 1;
        lzw.bits += 1;
        lzw.new_inc = 1;
    }

    lzw.codes[current].prev = lzw.previous_code as u16;
    lzw.codes[current].value = lzw.previous_first_char;

    /* NOTE: when the length cache deadcode below is enabled, this may
     * intentionally be set to or overflow to 0, in which case the length
     * will be computed as-needed by iterating the tree. */
    let prev_length = lzw.codes[lzw.previous_code as usize].length;
    lzw.codes[current].length = if prev_length != 0 {
        prev_length.wrapping_add(1)
    } else {
        0
    };
}

/**
 * Reset the LZW tree length.
 */
/// Translation of `LZW_clear()`.
fn lzw_clear(lzw: &mut LzwTree) {
    lzw.bits = 9;
    lzw.maxlength = 1 << lzw.bits;
    lzw.length = lzw.defaultlength;
    lzw.previous_code = LZW_NO_CODE as u32;
    /* (upstream keeps a reset of the cached code lengths here under #if 0) */
}

/**
 * Get the length of an LZW code, or compute it if it isn't currently stored.
 * This happens when one or mode codes in the sequence are marked for reuse.
 */
/// Translation of `LZW_get_length()` (its tree walk is under `#if 0`
/// upstream).
fn lzw_get_length(c: &LzwCode) -> u16 {
    c.length
}

/**
 * Output an LZW code.
 */
/// Translation of `LZW_output()`.
fn lzw_output(
    lzw: &mut LzwTree,
    mut code: u16,
    dest: &mut [u8],
    pos: &mut usize,
    left: &mut usize,
) -> i32 {
    let codes = &lzw.codes;
    let mut current = codes[code as usize];
    let length = lzw_get_length(&current) as usize;

    if length == 0 || length > *left {
        return -1;
    }

    let p = *pos;
    let mut i = length - 1;
    while i > 0 {
        dest[p + i] = current.value;
        code = current.prev;
        // (a broken chain can't happen: codes only refer to defined codes)
        let Some(&c) = codes.get(code as usize) else {
            return -1;
        };
        current = c;
        i -= 1;
    }
    dest[p] = code as u8;
    *pos += length;
    *left -= length;

    lzw.previous_first_char = code as u8;
    0
}

/**
 * Decode an LZW code and create the next code from known data.
 */
/// Translation of `LZW_decode()`.
fn lzw_decode(
    lzw: &mut LzwTree,
    code: u16,
    dest: &mut [u8],
    pos: &mut usize,
    left: &mut usize,
) -> i32 {
    let mut kwkwk = false;

    /* Digital Symphony LZW never seems to reference cleared codes,
     * which allows some assumptions to be made (like never clearing the
     * cached code lengths). If this decoder needs to support those, the
     * cached length handling deadcode above needs to be uncommented. */
    if code as u32 > lzw.length {
        return -1;
    }

    /* This is a special case--the current code is the previous code with the
     * first character of the previous code appended, and needs to be added
     * before the output occurs (instead of after). */
    if code as u32 == lzw.length {
        if lzw.previous_code == LZW_NO_CODE as u32 {
            return -1;
        }

        lzw_add(lzw);
        lzw.previous_code = code as u32;
        kwkwk = true;
    }

    /* Otherwise, output first, and then add a new code, which is the previous
     * code with the first character of the current code appended. */
    let result = lzw_output(lzw, code, dest, pos, left);
    if result == 0 && !kwkwk {
        if lzw.previous_code != LZW_NO_CODE as u32 {
            lzw_add(lzw);
        }

        lzw.previous_code = code as u32;
    }
    result
}

/// Translation of `libxmp_read_lzw()`.
pub(crate) fn libxmp_read_lzw(
    dest: &mut [u8],
    max_read_len: usize,
    flags: i32,
    f: &mut HioHandle<'_>,
) -> i32 {
    let mut bs = bs_init(max_read_len);
    let Some(mut lzw) = lzw_init_tree(flags) else {
        return -1;
    };

    let mut pos = 0usize;
    let mut left = dest.len();

    while left > 0 {
        let code = bs_read(&mut bs, f, lzw.bits as i32);
        if code < 0 {
            break;
        }

        if code == LZW_CODE_CLEAR {
            lzw_clear(&mut lzw);
            continue;
        } else if (flags & LZW_FLAG_SYMQUIRKS) != 0 && code == LZW_CODE_SYM_EOF {
            break;
        }

        lzw.new_inc = 0;
        let result = lzw_decode(&mut lzw, code as u16, dest, &mut pos, &mut left);
        if result != 0 {
            break;
        }
    }

    if left > 0 {
        dest[pos..pos + left].fill(0);
    } else if flags & LZW_FLAG_SYMQUIRKS != 0 {
        /* Digital Symphony - read final EOF code. */
        if lzw.new_inc != 0 {
            /* If the final code prior to EOF should have increased
             * the bitwidth, read the EOF with the old bitwidth
             * instead of the new one.
             *
             * This anomaly exists in FULLEFFECT, NARCOSIS and
             * NEWDANCE. In NEWDANCE (libxmp's test file for this),
             * it occurs specifically in the LZW-compressed sequence.
             * https://github.com/libxmp/libxmp/issues/347
             */
            lzw.bits -= 1;
        }

        let _code = bs_read(&mut bs, f, lzw.bits as i32);
        /* (upstream logs a missing EOF code, or a longer stream) */
    }

    if flags & LZW_FLAG_SYMQUIRKS != 0 {
        /* Digital Symphony LZW compressed stream size is 4 aligned. */
        let mut num_read = bs.num_read;
        while num_read & 3 != 0 {
            hio_read8(f);
            num_read += 1;
        }
    }

    let _ = lzw.flags;
    0
}

/* Decode Digital Symphony sigma-delta compressed samples.
 * This isn't really LZW but it uses the same bitstream and alignment hacks.
 *
 * Based on the sigma-delta unpacker from OpenMPT by Saga Musix.
 */
/// Translation of `libxmp_read_sigma_delta()`.
pub(crate) fn libxmp_read_sigma_delta(
    dest: &mut [u8],
    max_read_len: usize,
    f: &mut HioHandle<'_>,
) -> i32 {
    let end = dest.len();
    let mut pos = 0usize;
    let mut runlength = 0;
    let mut bits = 8;

    if end == 0 {
        return 0;
    }

    let mut bs = bs_init(max_read_len);

    /* DOESN'T count towards alignment. */
    let max_runlength = hio_read8(f) as i32;
    /* DOES count. */
    let mut accumulator = bs_read(&mut bs, f, bits) as u8;
    dest[pos] = accumulator;
    pos += 1;

    while pos < end {
        let value = bs_read(&mut bs, f, bits);
        if value < 0 {
            return -1;
        }

        /* Expand bitwidth. */
        if value == 0 {
            if bits >= 9 {
                return -1;
            }

            bits += 1;
            runlength = 0;
            continue;
        }

        if value & 1 != 0 {
            accumulator = accumulator.wrapping_sub((value >> 1) as u8);
        } else {
            accumulator = accumulator.wrapping_add((value >> 1) as u8);
        }

        dest[pos] = accumulator;
        pos += 1;

        /* High bit set resets the run length. */
        if value >> (bits - 1) != 0 {
            runlength = 0;
            continue;
        }
        /* Reduce bitwidth. */
        runlength += 1;
        if runlength >= max_runlength {
            if bits > 1 {
                bits -= 1;
            }
            runlength = 0;
        }
    }

    /* Digital Symphony aligns bitstreams to lengths of 4. */
    if bs.num_read & 3 != 0 {
        let mut total = bs.num_read;
        while total & 3 != 0 {
            hio_read8(f);
            total += 1;
        }
    }
    0
}
