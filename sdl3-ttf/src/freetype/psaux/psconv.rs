// Rust translation of src/psaux/psconv.c and src/psaux/psconv.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it), with the
// PostScript character class macros of include/freetype/internal/psaux.h.
// Copyright (C) 2006-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Some convenience conversions (body).
//!
//! The `FT_Byte*` cursors and limits are offsets into the bytes `b`; the
//! characters past `b`'s end read as zeros.

use super::super::base::ftcalc::ft_div_fix;
use super::super::fttypes::*;

/* psaux.h */

/// `IS_PS_NEWLINE`
#[inline]
pub fn is_ps_newline(ch: u8) -> bool {
    ch == b'\r' || ch == b'\n'
}

/// `IS_PS_SPACE`
#[inline]
pub fn is_ps_space(ch: u8) -> bool {
    ch == b' ' || is_ps_newline(ch) || ch == b'\t' || ch == 0x0C || ch == 0
}

/// `IS_PS_SPECIAL`
#[inline]
pub fn is_ps_special(ch: u8) -> bool {
    matches!(
        ch,
        b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'%'
    )
}

/// `IS_PS_DELIM`
#[inline]
pub fn is_ps_delim(ch: u8) -> bool {
    is_ps_space(ch) || is_ps_special(ch)
}

/// `IS_PS_DIGIT`
#[inline]
pub fn is_ps_digit(ch: u8) -> bool {
    ch.is_ascii_digit()
}

/// `IS_PS_XDIGIT`
#[inline]
pub fn is_ps_xdigit(ch: u8) -> bool {
    is_ps_digit(ch) || (b'A'..=b'F').contains(&ch) || (b'a'..=b'f').contains(&ch)
}

/// `IS_PS_BASE85`
#[inline]
pub fn is_ps_base85(ch: u8) -> bool {
    (b'!'..=b'u').contains(&ch)
}

/// `IS_PS_TOKEN`: whether the bytes of `b` from `cur` (to `limit`) are
/// the token `token` (followed by a delimiter or the limit)
pub fn is_ps_token(b: &[u8], cur: usize, limit: usize, token: &[u8]) -> bool {
    /* (`sizeof ( (token) )' counts the terminating null byte) */
    let size = token.len() + 1;

    at(b, cur) == token[0]
        && (cur + size == limit || (cur + size < limit && is_ps_delim(at(b, cur + size - 1))))
        && (0..token.len()).all(|i| at(b, cur + i) == token[i])
}

/// The byte of `b` at `i` (0 past its end).
#[inline]
pub fn at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

/* The following array is used by various functions to quickly convert */
/* digits (both decimal and non-decimal) into numbers.                 */

/* ASCII */
static FT_CHAR_TABLE: [FtChar; 128] = [
    /* 0x00 */
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, -1, -1, -1, -1, -1, -1, -1, 10, 11, 12, 13, 14, 15, 16, 17, 18,
    19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, -1, -1, -1, -1, -1, -1, 10,
    11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34,
    35, -1, -1, -1, -1, -1,
];

/* no character >= 0x80 can represent a valid number */
#[inline]
fn op(c: u8) -> bool {
    c >= 0x80
}

/// `PS_Conv_Strtol`
pub fn ps_conv_strtol(b: &[u8], cursor: &mut usize, limit: usize, base: FtLong) -> FtLong {
    let mut p = *cursor;
    let mut num: FtLong = 0;
    let mut sign = false;
    let mut have_overflow = false;

    if p >= limit {
        /* Bad: */
        return 0;
    }

    if !(2..=36).contains(&base) {
        return 0;
    }

    if at(b, p) == b'-' || at(b, p) == b'+' {
        sign = at(b, p) == b'-';

        p += 1;
        if p == limit {
            /* Bad: */
            return 0;
        }

        /* only a single sign is allowed */
        if at(b, p) == b'-' || at(b, p) == b'+' {
            return 0;
        }
    }

    let num_limit: FtLong = 0x7FFFFFFF / base;
    let c_limit: FtChar = (0x7FFFFFFF % base) as FtChar;

    while p < limit {
        let ch = at(b, p);
        if is_ps_space(ch) || op(ch) {
            break;
        }

        let c = FT_CHAR_TABLE[(ch & 0x7F) as usize];

        if c < 0 || c as FtLong >= base {
            break;
        }

        if num > num_limit || (num == num_limit && c > c_limit) {
            have_overflow = true;
        } else {
            num = num * base + c as FtLong;
        }
        p += 1;
    }

    *cursor = p;

    if have_overflow {
        num = 0x7FFFFFFF;
    }

    if sign {
        num = -num;
    }

    num
}

/// `PS_Conv_ToInt`
pub fn ps_conv_to_int(b: &[u8], cursor: &mut usize, limit: usize) -> FtLong {
    let mut p = *cursor;

    let mut curp = p;
    let mut num = ps_conv_strtol(b, &mut p, limit, 10);
    if p == curp {
        return 0;
    }

    if p < limit && at(b, p) == b'#' {
        p += 1;

        curp = p;
        num = ps_conv_strtol(b, &mut p, limit, num);
        if p == curp {
            return 0;
        }
    }

    *cursor = p;

    num
}

/// `PS_Conv_ToFixed`
pub fn ps_conv_to_fixed(b: &[u8], cursor: &mut usize, limit: usize, power_ten: FtLong) -> FtFixed {
    let mut p = *cursor;
    let mut power_ten = power_ten;
    let mut integral: FtFixed = 0;
    let mut decimal: FtLong = 0;
    let mut divider: FtLong = 1;

    let mut sign = false;
    let mut have_overflow = false;
    let mut have_underflow = false;

    if p >= limit {
        /* Bad: */
        return 0;
    }

    if at(b, p) == b'-' || at(b, p) == b'+' {
        sign = at(b, p) == b'-';

        p += 1;
        if p == limit {
            /* Bad: */
            return 0;
        }

        /* only a single sign is allowed */
        if at(b, p) == b'-' || at(b, p) == b'+' {
            return 0;
        }
    }

    /* read the integer part */
    if at(b, p) != b'.' {
        let curp = p;
        integral = ps_conv_to_int(b, &mut p, limit);

        if p == curp {
            return 0;
        }

        if integral > 0x7FFF {
            have_overflow = true;
        } else {
            integral = ((integral as FtUInt32) << 16) as FtFixed;
        }
    }

    /* read the decimal part */
    if p < limit && at(b, p) == b'.' {
        p += 1;

        while p < limit {
            let ch = at(b, p);
            if is_ps_space(ch) || op(ch) {
                break;
            }

            let c = FT_CHAR_TABLE[(ch & 0x7F) as usize];

            if !(0..10).contains(&c) {
                break;
            }

            /* only add digit if we don't overflow */
            if divider < 0xCCCCCCC && decimal < 0xCCCCCCC {
                decimal = decimal * 10 + c as FtLong;

                if integral == 0 && power_ten > 0 {
                    power_ten -= 1;
                } else {
                    divider *= 10;
                }
            }
            p += 1;
        }
    }

    /* read exponent, if any */
    if p + 1 < limit && (at(b, p) == b'e' || at(b, p) == b'E') {
        p += 1;

        let curp = p;
        let exponent = ps_conv_to_int(b, &mut p, limit);

        if curp == p {
            return 0;
        }

        /* arbitrarily limit exponent */
        if exponent > 1000 {
            have_overflow = true;
        } else if exponent < -1000 {
            have_underflow = true;
        } else {
            power_ten += exponent;
        }
    }

    *cursor = p;

    if integral == 0 && decimal == 0 {
        return 0;
    }

    'exit: {
        'overflow: {
            if have_overflow {
                break 'overflow;
            }

            if have_underflow {
                /* Underflow: */
                return 0;
            }

            while power_ten > 0 {
                if integral >= 0xCCCCCCC {
                    break 'overflow;
                }
                integral *= 10;

                if decimal >= 0xCCCCCCC {
                    if divider == 1 {
                        break 'overflow;
                    }
                    divider /= 10;
                } else {
                    decimal *= 10;
                }

                power_ten -= 1;
            }

            while power_ten < 0 {
                integral /= 10;
                if divider < 0xCCCCCCC {
                    divider *= 10;
                } else {
                    decimal /= 10;
                }

                if integral == 0 && decimal == 0 {
                    /* Underflow: */
                    return 0;
                }

                power_ten += 1;
            }

            if decimal != 0 {
                decimal = ft_div_fix(decimal, divider);
                /* it's not necessary to check this addition for overflow */
                /* due to the structure of the real number representation */
                integral += decimal;
            }

            break 'exit;
        }

        /* Overflow: */
        integral = 0x7FFFFFFF;
    }

    /* Exit: */
    if sign {
        integral = -integral;
    }

    integral
}

/* (the `#if 0'ed PS_Conv_StringDecode) */

/// `PS_Conv_ASCIIHexDecode`: decodes the hexadecimal digits of `b` from
/// `cursor` into `buffer` (at most `n` bytes)
pub fn ps_conv_ascii_hex_decode(
    b: &[u8],
    cursor: &mut usize,
    limit: usize,
    buffer: &mut [u8],
    n: usize,
) -> FtUInt {
    let mut r: FtUInt = 0;
    let mut w: usize = 0;
    let mut pad: FtUInt = 0x01;

    let mut n = n * 2;

    let p = *cursor;

    if p >= limit {
        return 0;
    }

    if n > limit - p {
        n = limit - p;
    }

    /* we try to process two nibbles at a time to be as fast as possible */
    while (r as usize) < n {
        let mut c = at(b, p + r as usize) as FtUInt;

        if is_ps_space(c as u8) {
            r += 1;
            continue;
        }

        if op(c as u8) {
            break;
        }

        c = FT_CHAR_TABLE[(c & 0x7F) as usize] as FtInt as FtUInt;
        if c >= 16 {
            break;
        }

        pad = (pad << 4) | c;
        if pad & 0x100 != 0 {
            if let Some(o) = buffer.get_mut(w) {
                *o = pad as FtByte;
            }
            w += 1;
            pad = 0x01;
        }
        r += 1;
    }

    if pad != 0x01 {
        if let Some(o) = buffer.get_mut(w) {
            *o = (pad << 4) as FtByte;
        }
        w += 1;
    }

    *cursor = p + r as usize;

    w as FtUInt
}

/// `PS_Conv_EexecDecode`: decrypts the bytes of `b` from `cursor` into
/// `buffer` (at most `n` bytes)
pub fn ps_conv_eexec_decode(
    b: &[u8],
    cursor: &mut usize,
    limit: usize,
    buffer: &mut [u8],
    n: usize,
    seed: &mut FtUShort,
) -> FtUInt {
    let mut s: FtUInt = *seed as FtUInt;
    let mut n = n;

    let p = *cursor;

    if p >= limit {
        return 0;
    }

    if n > limit - p {
        n = limit - p;
    }

    for r in 0..n {
        let val: FtUInt = at(b, p + r) as FtUInt;
        let bb: FtUInt = val ^ (s >> 8);

        s = (val.wrapping_add(s).wrapping_mul(52845).wrapping_add(22719)) & 0xFFFF;
        if let Some(o) = buffer.get_mut(r) {
            *o = bb as FtByte;
        }
    }

    *cursor = p + n;
    *seed = s as FtUShort;

    n as FtUInt
}

/// `PS_Conv_EexecDecode` in place (the decrypted bytes replace those of
/// `b` from `start`, at most `n` of them).
pub fn ps_conv_eexec_decode_in_place(b: &mut [u8], start: usize, n: usize, seed: &mut FtUShort) {
    let mut s: FtUInt = *seed as FtUInt;
    let end = start.saturating_add(n).min(b.len());

    if start >= end {
        return;
    }

    for v in &mut b[start..end] {
        let val: FtUInt = *v as FtUInt;
        let bb: FtUInt = val ^ (s >> 8);

        s = (val.wrapping_add(s).wrapping_mul(52845).wrapping_add(22719)) & 0xFFFF;
        *v = bb as FtByte;
    }

    *seed = s as FtUShort;
}
