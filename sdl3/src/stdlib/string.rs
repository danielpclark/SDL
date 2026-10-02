// Rust translation of parts of src/stdlib/SDL_string.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! String helpers whose exact behaviour SDL relies on.
//!
//! Translated: UTF-8 decoding with SDL's replacement rules (`SDL_StepUTF8`,
//! `SDL_StepBackUTF8`), `SDL_UCS4ToUTF8`, Unicode case folding and the
//! case-insensitive comparisons built on it (`SDL_strcasecmp`,
//! `SDL_strncasecmp`, `SDL_strcasestr`), `SDL_utf8strlcpy`/`SDL_utf8strlen`,
//! the integer/float scanners behind `SDL_strtol` & co., and the
//! `SDL_ltoa` family.
//!
//! Not translated, because Rust's `str`/`String`/`format!` replace them with
//! no behavioural difference SDL depends on: `SDL_strlen`, `SDL_strlcpy`,
//! `SDL_strdup`, `SDL_strchr`, `SDL_strstr`, `SDL_strrev`, `SDL_strupr`,
//! `SDL_strcmp`, `SDL_memcmp`, the `wcs*` functions (Rust has no `wchar_t`;
//! use `encode_utf16`/[`iconv`](super::iconv)), and `SDL_snprintf`/`SDL_sscanf`.
//!
//! Functions take `&[u8]` (`impl AsRef<[u8]>`) where SDL accepts arbitrary
//! bytes, so `&str`, `String` and byte slices all work. As in C, a NUL byte
//! ends the string.

use std::cmp::Ordering;

use super::casefolding::{CASE_FOLD1_16, CASE_FOLD1_32, CASE_FOLD2_16, CASE_FOLD3_16};

/// The Unicode REPLACEMENT CHARACTER, used for invalid input.
/// Translation of `SDL_INVALID_UNICODE_CODEPOINT`.
pub const INVALID_UNICODE_CODEPOINT: u32 = 0xFFFD;

/// The `char` for a code point, with values outside Unicode and UTF-16
/// surrogates replaced by U+FFFD (the replacement `SDL_UCS4ToUTF8()` makes).
pub fn ucs4_to_char(mut codepoint: u32) -> char {
    if codepoint > 0x10FFFF {
        // Outside the range of Unicode codepoints (also, larger than can be encoded in 4 bytes of UTF-8!).
        codepoint = INVALID_UNICODE_CODEPOINT;
    } else if (0xD800..=0xDFFF).contains(&codepoint) {
        // UTF-16 surrogate values are illegal in UTF-8.
        codepoint = INVALID_UNICODE_CODEPOINT;
    }
    char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// Encode a code point as UTF-8 into `dst`, returning the encoded part.
/// Translation of `SDL_UCS4ToUTF8()` (which returns the advanced pointer).
pub fn ucs4_to_utf8(codepoint: u32, dst: &mut [u8; 4]) -> &str {
    ucs4_to_char(codepoint).encode_utf8(dst)
}

/// The result of [`case_fold_unicode`]: one to three code points.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CaseFold {
    cps: [u32; 3],
    len: u8,
}

impl CaseFold {
    /// The folded code points.
    pub fn as_slice(&self) -> &[u32] {
        &self.cps[..self.len as usize]
    }
}

impl std::ops::Deref for CaseFold {
    type Target = [u32];
    fn deref(&self) -> &[u32] {
        self.as_slice()
    }
}

/// Fold a UTF-32 code point for case-insensitive comparison (full case
/// folding: `ß` becomes `ss`). Translation of `SDL_CaseFoldUnicode()`.
pub fn case_fold_unicode(from: u32) -> CaseFold {
    let one = |c: u32| CaseFold {
        cps: [c, 0, 0],
        len: 1,
    };
    if from < 128 {
        // low-ASCII, easy!
        if (b'A' as u32..=b'Z' as u32).contains(&from) {
            return one(b'a' as u32 + (from - b'A' as u32));
        }
    } else if from <= 0xFFFF {
        // the Basic Multilingual Plane.
        let from16 = from as u16;

        // see if it maps to a single char (most common)...
        if let Ok(i) = CASE_FOLD1_16.binary_search_by_key(&from16, |m| m.0) {
            return one(CASE_FOLD1_16[i].1 as u32);
        }

        // see if it folds down to two chars...
        if let Ok(i) = CASE_FOLD2_16.binary_search_by_key(&from16, |m| m.0) {
            let m = CASE_FOLD2_16[i];
            return CaseFold {
                cps: [m.1 as u32, m.2 as u32, 0],
                len: 2,
            };
        }

        // okay, maybe it's _three_ characters!
        if let Ok(i) = CASE_FOLD3_16.binary_search_by_key(&from16, |m| m.0) {
            let m = CASE_FOLD3_16[i];
            return CaseFold {
                cps: [m.1 as u32, m.2 as u32, m.3 as u32],
                len: 3,
            };
        }
    } else {
        // codepoint that doesn't fit in 16 bits.
        if let Ok(i) = CASE_FOLD1_32.binary_search_by_key(&from, |m| m.0) {
            return one(CASE_FOLD1_32[i].1);
        }
    }

    // Not found...there's no folding needed for this codepoint.
    one(from)
}

/// Translation of the static `StepUTF8()`: decode one code point from the
/// first `slen` bytes, advancing `s` past it. Returns 0 (without advancing)
/// at the end of the input or at a NUL byte, and U+FFFD (advancing one byte)
/// for an invalid sequence.
fn step_utf8_limited(s: &mut &[u8], slen: usize) -> u32 {
    /*
     * From rfc3629, the UTF-8 spec:
     *  https://www.ietf.org/rfc/rfc3629.txt
     *
     *   Char. number range  |        UTF-8 octet sequence
     *      (hexadecimal)    |              (binary)
     *   --------------------+---------------------------------------------
     *   0000 0000-0000 007F | 0xxxxxxx
     *   0000 0080-0000 07FF | 110xxxxx 10xxxxxx
     *   0000 0800-0000 FFFF | 1110xxxx 10xxxxxx 10xxxxxx
     *   0001 0000-0010 FFFF | 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx
     */
    let str = *s;
    let slen = slen.min(str.len());
    let at = |i: usize| str.get(i).copied().unwrap_or(0);
    let octet = if slen != 0 { at(0) as u32 } else { 0 };

    if octet == 0 {
        // null terminator, end of string.
        return 0; // don't advance `*_str`.
    } else if (octet & 0x80) == 0 {
        // 0xxxxxxx: one byte codepoint.
        *s = &str[1..];
        return octet;
    } else if (octet & 0xE0) == 0xC0 && slen >= 2 {
        // 110xxxxx 10xxxxxx: two byte codepoint.
        let str1 = at(1);
        if (str1 & 0xC0) == 0x80 {
            // If trailing bytes aren't 10xxxxxx, sequence is bogus.
            let result = ((octet & 0x1F) << 6) | (str1 & 0x3F) as u32;
            if result >= 0x0080 {
                // rfc3629 says you can't use overlong sequences for smaller values.
                *s = &str[2..];
                return result;
            }
        }
    } else if (octet & 0xF0) == 0xE0 && slen >= 3 {
        // 1110xxxx 10xxxxxx 10xxxxxx: three byte codepoint.
        let str1 = at(1);
        let str2 = at(2);
        if (str1 & 0xC0) == 0x80 && (str2 & 0xC0) == 0x80 {
            // If trailing bytes aren't 10xxxxxx, sequence is bogus.
            let octet2 = ((str1 & 0x3F) as u32) << 6;
            let octet3 = (str2 & 0x3F) as u32;
            let result = ((octet & 0x0F) << 12) | octet2 | octet3;
            if result >= 0x800 {
                // rfc3629 says you can't use overlong sequences for smaller values.
                if !(0xD800..=0xDFFF).contains(&result) {
                    // UTF-16 surrogate values are illegal in UTF-8.
                    *s = &str[3..];
                    return result;
                }
            }
        }
    } else if (octet & 0xF8) == 0xF0 && slen >= 4 {
        // 11110xxxx 10xxxxxx 10xxxxxx 10xxxxxx: four byte codepoint.
        let str1 = at(1);
        let str2 = at(2);
        let str3 = at(3);
        if (str1 & 0xC0) == 0x80 && (str2 & 0xC0) == 0x80 && (str3 & 0xC0) == 0x80 {
            // If trailing bytes aren't 10xxxxxx, sequence is bogus.
            let octet2 = ((str1 & 0x1F) as u32) << 12;
            let octet3 = ((str2 & 0x3F) as u32) << 6;
            let octet4 = (str3 & 0x3F) as u32;
            let result = ((octet & 0x07) << 18) | octet2 | octet3 | octet4;
            if result >= 0x10000 {
                // rfc3629 says you can't use overlong sequences for smaller values.
                *s = &str[4..];
                return result;
            }
        }
    }

    // bogus byte, skip ahead, return a REPLACEMENT CHARACTER.
    *s = &str[1..];
    INVALID_UNICODE_CODEPOINT
}

/// Decode the next code point and advance `s` past it.
///
/// Returns 0 (and leaves `s` alone) at the end of the input or at a NUL
/// byte. Invalid sequences — bad lead or trailing bytes, overlong forms,
/// surrogates, truncated sequences — return U+FFFD and skip one byte.
/// Translation of `SDL_StepUTF8()`; the slice length plays the role of `*pslen`.
pub fn step_utf8(s: &mut &[u8]) -> u32 {
    step_utf8_limited(s, s.len())
}

/// Step back one code point from byte offset `pos` in `s` and decode it.
/// Returns 0 if `pos` is at the start. Translation of `SDL_StepBackUTF8()`.
pub fn step_back_utf8(s: &[u8], pos: &mut usize) -> u32 {
    let mut p = (*pos).min(s.len());
    if p == 0 {
        return 0;
    }

    // Step back over the previous UTF-8 character
    let end = p;
    loop {
        if p == 0 {
            break;
        }
        p -= 1;
        if (s[p] & 0xC0) != 0x80 {
            break;
        }
    }

    let length = end - p;
    *pos = p;
    let mut rest = &s[p..];
    step_utf8_limited(&mut rest, length)
}

/// An iterator over the code points of a byte string, decoded as by
/// [`step_utf8`]; it stops at the end or at a NUL byte.
#[derive(Clone, Debug)]
pub struct Codepoints<'a> {
    rest: &'a [u8],
}

impl Iterator for Codepoints<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        match step_utf8(&mut self.rest) {
            0 => None,
            cp => Some(cp),
        }
    }
}

/// Iterate the code points of `s` the way SDL decodes UTF-8.
pub fn codepoints(s: &(impl AsRef<[u8]> + ?Sized)) -> Codepoints<'_> {
    Codepoints { rest: s.as_ref() }
}

/// The number of code points in `s` (invalid sequences count one per
/// replaced byte). Translation of `SDL_utf8strlen()`.
pub fn utf8strlen(s: &(impl AsRef<[u8]> + ?Sized)) -> usize {
    codepoints(s).count()
}

/// The number of code points in the first `bytes` bytes of `s`.
/// Translation of `SDL_utf8strnlen()`.
pub fn utf8strnlen(s: &(impl AsRef<[u8]> + ?Sized), bytes: usize) -> usize {
    let s = s.as_ref();
    codepoints(&s[..bytes.min(s.len())]).count()
}

fn utf8_is_lead_byte(c: u8) -> bool {
    (0xC0..=0xF4).contains(&c)
}

fn utf8_is_trailing_byte(c: u8) -> bool {
    (0x80..=0xBF).contains(&c)
}

/// Translation of `UTF8_GetTrailingBytes()`.
fn utf8_get_trailing_bytes(c: u8) -> usize {
    if (0xC0..=0xDF).contains(&c) {
        1
    } else if (0xE0..=0xEF).contains(&c) {
        2
    } else if (0xF0..=0xF4).contains(&c) {
        3
    } else {
        0
    }
}

/// How many bytes of `src` `SDL_utf8strlcpy()` copies into a `dst_bytes`
/// buffer (which also needs room for the NUL): as many as fit without
/// splitting a multi-byte sequence. Translation of `SDL_utf8strlcpy()`.
pub fn utf8strlcpy_len(src: &(impl AsRef<[u8]> + ?Sized), dst_bytes: usize) -> usize {
    let src = src.as_ref();
    let src = &src[..src.iter().position(|&b| b == 0).unwrap_or(src.len())];
    let mut bytes = 0;
    if dst_bytes > 0 {
        let src_bytes = src.len();
        bytes = src_bytes.min(dst_bytes - 1);
        if bytes != 0 {
            let c = src[bytes - 1];
            if utf8_is_lead_byte(c) {
                bytes -= 1;
            } else if utf8_is_trailing_byte(c) {
                let mut i = bytes - 1;
                while i != 0 {
                    let c = src[i];
                    let trailing_bytes = utf8_get_trailing_bytes(c);
                    if trailing_bytes != 0 {
                        if (bytes - i) != (trailing_bytes + 1) {
                            bytes = i;
                        }
                        break;
                    }
                    i -= 1;
                }
            }
        }
    }
    bytes
}

/// The longest prefix of `s` that fits in `max_bytes` bytes without splitting
/// a character (the copy `SDL_utf8strlcpy()` makes into a `max_bytes + 1`
/// byte buffer).
pub fn utf8_truncate(s: &str, max_bytes: usize) -> &str {
    let n = utf8strlcpy_len(s, max_bytes.saturating_add(1));
    // utf8strlcpy_len never splits a sequence of valid UTF-8.
    &s[..n]
}

/// Translation of the `UNICODE_STRCASECMP` macro for UTF-8 with optional byte limits.
fn unicode_strcasecmp(
    mut str1: &[u8],
    mut str2: &[u8],
    mut slen1: usize,
    mut slen2: usize,
) -> Ordering {
    let mut folded1 = [0u32; 3];
    let mut folded2 = [0u32; 3];
    let (mut head1, mut tail1, mut head2, mut tail2) = (0usize, 0usize, 0usize, 0usize);
    loop {
        let cp1;
        let cp2;
        if head1 != tail1 {
            cp1 = folded1[tail1];
            tail1 += 1;
        } else {
            let start = str1.len();
            let f = case_fold_unicode(step_utf8_limited(&mut str1, slen1));
            slen1 -= start - str1.len();
            folded1[..f.len()].copy_from_slice(&f);
            head1 = f.len();
            cp1 = folded1[0];
            tail1 = 1;
        }
        if head2 != tail2 {
            cp2 = folded2[tail2];
            tail2 += 1;
        } else {
            let start = str2.len();
            let f = case_fold_unicode(step_utf8_limited(&mut str2, slen2));
            slen2 -= start - str2.len();
            folded2[..f.len()].copy_from_slice(&f);
            head2 = f.len();
            cp2 = folded2[0];
            tail2 = 1;
        }
        match cp1.cmp(&cp2) {
            Ordering::Equal if cp1 == 0 => break, // complete match.
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

/// Compare two strings case-insensitively, with full Unicode case folding.
/// Translation of `SDL_strcasecmp()`.
pub fn strcasecmp(
    str1: &(impl AsRef<[u8]> + ?Sized),
    str2: &(impl AsRef<[u8]> + ?Sized),
) -> Ordering {
    let (a, b) = (str1.as_ref(), str2.as_ref());
    unicode_strcasecmp(a, b, usize::MAX, usize::MAX)
}

/// Compare up to `maxlen` bytes of two strings case-insensitively.
/// Translation of `SDL_strncasecmp()`.
pub fn strncasecmp(
    str1: &(impl AsRef<[u8]> + ?Sized),
    str2: &(impl AsRef<[u8]> + ?Sized),
    maxlen: usize,
) -> Ordering {
    let (a, b) = (str1.as_ref(), str2.as_ref());
    unicode_strcasecmp(a, b, maxlen, maxlen)
}

/// The byte offset of the first case-insensitive match of `needle` in
/// `haystack`, trying each code point position. Translation of `SDL_strcasestr()`.
pub fn strcasestr(
    haystack: &(impl AsRef<[u8]> + ?Sized),
    needle: &(impl AsRef<[u8]> + ?Sized),
) -> Option<usize> {
    let haystack = haystack.as_ref();
    let needle = needle.as_ref();
    let length = needle.iter().position(|&b| b == 0).unwrap_or(needle.len());
    let mut rest = haystack;
    loop {
        if strncasecmp(rest, needle, length) == Ordering::Equal {
            return Some(haystack.len() - rest.len());
        }
        // move ahead by a full codepoint at a time, regardless of bytes.
        if step_utf8(&mut rest) == 0 {
            return None;
        }
    }
}

/// `SDL_isspace()`: the C locale's whitespace.
fn is_space(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' || c == 0x0C || c == 0x0B
}

/// Parses an unsigned long long and returns the unsigned value and sign bit.
///
/// Positive values are clamped to ULLONG_MAX.
/// The result `value == 0 && negative` indicates negative overflow
/// and might need to be handled differently depending on whether a
/// signed or unsigned integer is being parsed.
///
/// Translation of `SDL_ScanUnsignedLongLongInternal()`; returns
/// `(value, negative, bytes consumed)`. `count == 0` means no limit.
fn scan_unsigned_long_long_internal(
    text: &[u8],
    count: usize,
    mut radix: u32,
) -> (u64, bool, usize) {
    let ullong_max = u64::MAX;
    let at = |i: usize| text.get(i).copied().unwrap_or(0);
    let mut pos = 0usize;
    let mut number_start = 0usize;
    let mut value: u64 = 0;
    let mut negative = false;
    let mut overflow = false;

    if radix == 0 || (2..=36).contains(&radix) {
        while is_space(at(pos)) {
            pos += 1;
        }
        if at(pos) == b'-' || at(pos) == b'+' {
            negative = at(pos) == b'-';
            pos += 1;
        }
        if (radix == 0 || radix == 16)
            && at(pos) == b'0'
            && (at(pos + 1) == b'x' || at(pos + 1) == b'X')
        {
            pos += 2;
            radix = 16;
        } else if radix == 0 && at(pos) == b'0' && at(pos + 1).is_ascii_digit() {
            pos += 1;
            radix = 8;
        } else if radix == 0 {
            radix = 10;
        }
        number_start = pos;
        loop {
            let c = at(pos);
            let digit: u64;
            if c.is_ascii_digit() {
                digit = (c - b'0') as u64;
            } else if radix > 10 {
                if c >= b'A' && (c as u32) < b'A' as u32 + (radix - 10) {
                    digit = 10 + (c - b'A') as u64;
                } else if c >= b'a' && (c as u32) < b'a' as u32 + (radix - 10) {
                    digit = 10 + (c - b'a') as u64;
                } else {
                    break;
                }
            } else {
                break;
            }
            // (upstream accepts digits >= radix for radix <= 10; kept as-is)
            if value != 0 && radix as u64 > ullong_max / value {
                overflow = true;
            } else {
                value *= radix as u64;
                if digit > ullong_max - value {
                    overflow = true;
                } else {
                    value += digit;
                }
            }
            pos += 1;
            if count != 0 && pos == count {
                break;
            }
        }
    }
    if pos == number_start {
        if radix == 16 && pos > 0 && (at(pos - 1) == b'x' || at(pos - 1) == b'X') {
            // the string was "0x"; consume the '0' but not the 'x'
            pos -= 1;
        } else {
            // no number was parsed, and thus no characters were consumed
            pos = 0;
        }
    }
    if overflow {
        value = if negative { 0 } else { ullong_max };
    } else if value == 0 {
        negative = false;
    }
    (value, negative, pos)
}

/// Parse a signed integer like C's `strtol` (base 0 auto-detects `0x`/`0`
/// prefixes; out-of-range values clamp). Returns `(value, bytes consumed)`;
/// zero bytes consumed means nothing parsed. `long` is 64-bit here, as on
/// the LP64 platforms; use [`strtoll`] for the same thing by its other name.
/// Translation of `SDL_strtol()` (via `SDL_ScanLong()`).
pub fn strtol(text: &(impl AsRef<[u8]> + ?Sized), base: u32) -> (i64, usize) {
    strtoll(text, base)
}

/// Parse an unsigned integer like C's `strtoul`; a leading `-` negates
/// modulo 2^64, as C does. Translation of `SDL_strtoul()` (via `SDL_ScanUnsignedLong()`).
pub fn strtoul(text: &(impl AsRef<[u8]> + ?Sized), base: u32) -> (u64, usize) {
    let ulong_max = u64::MAX;
    let (mut value, negative, len) = scan_unsigned_long_long_internal(text.as_ref(), 0, base);
    if negative {
        if value == 0 || value > ulong_max {
            value = ulong_max;
        } else if value == ulong_max {
            value = 1;
        } else {
            value = 0u64.wrapping_sub(value);
        }
    } else if value > ulong_max {
        value = ulong_max;
    }
    (value, len)
}

/// Parse a 64-bit signed integer. Translation of `SDL_strtoll()` (via `SDL_ScanLongLong()`).
pub fn strtoll(text: &(impl AsRef<[u8]> + ?Sized), base: u32) -> (i64, usize) {
    let llong_max = u64::MAX >> 1;
    let (mut value, negative, len) = scan_unsigned_long_long_internal(text.as_ref(), 0, base);
    if negative {
        let abs_llong_min = llong_max + 1;
        if value == 0 || value > abs_llong_min {
            value = 0u64.wrapping_sub(abs_llong_min);
        } else {
            value = 0u64.wrapping_sub(value);
        }
    } else if value > llong_max {
        value = llong_max;
    }
    (value as i64, len)
}

/// Parse a 64-bit unsigned integer. Translation of `SDL_strtoull()` (via `SDL_ScanUnsignedLongLong()`).
pub fn strtoull(text: &(impl AsRef<[u8]> + ?Sized), base: u32) -> (u64, usize) {
    let ullong_max = u64::MAX;
    let (mut value, negative, len) = scan_unsigned_long_long_internal(text.as_ref(), 0, base);
    if negative {
        if value == 0 {
            value = ullong_max;
        } else {
            value = 0u64.wrapping_sub(value);
        }
    }
    (value, len)
}

/// Parse a floating-point number: optional sign, digits, optional fraction,
/// optional exponent (`e`/`E`), as the platform `strtod` SDL calls does.
/// Returns `(value, bytes consumed)`. Translation of `SDL_strtod()`.
///
/// Upstream's own fallback scanner (`SDL_ScanFloat`, used only on
/// platforms without `strtod`) ignores exponents and hex floats; this
/// follows the C library behaviour every desktop build gets, minus hex
/// floats, `inf` and `nan`.
pub fn strtod(text: &(impl AsRef<[u8]> + ?Sized)) -> (f64, usize) {
    let bytes = text.as_ref();
    let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while is_space(at(i)) {
        i += 1;
    }
    let start = i;
    if at(i) == b'+' || at(i) == b'-' {
        i += 1;
    }
    let mut digits = 0;
    while at(i).is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if at(i) == b'.' {
        let mut j = i + 1;
        let mut frac = 0;
        while at(j).is_ascii_digit() {
            j += 1;
            frac += 1;
        }
        if digits + frac > 0 {
            i = j;
            digits += frac;
        }
    }
    if digits == 0 {
        return (0.0, 0);
    }
    if at(i) == b'e' || at(i) == b'E' {
        let mut j = i + 1;
        if at(j) == b'+' || at(j) == b'-' {
            j += 1;
        }
        if at(j).is_ascii_digit() {
            while at(j).is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let s = std::str::from_utf8(&bytes[start..i]).unwrap_or("0");
    // Rust doesn't accept a bare trailing '.', C does ("5." == 5.0).
    let value = s.trim_end_matches('.').parse::<f64>().unwrap_or(0.0);
    (value, i)
}

/// Translation of `ntoa_table`.
const NTOA_TABLE: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Format an unsigned integer in `radix` (2–36, uppercase digits).
/// Translation of `SDL_ulltoa()` / `SDL_ultoa()` / `SDL_uitoa()`.
///
/// # Panics
/// If `radix` is outside 2–36 (C would divide by zero or index out of bounds).
pub fn ulltoa(mut value: u64, radix: u32) -> String {
    assert!((2..=36).contains(&radix), "radix must be in 2..=36");
    let radix = radix as u64;
    let mut buf = Vec::new();
    if value != 0 {
        while value > 0 {
            buf.push(NTOA_TABLE[(value % radix) as usize]);
            value /= radix;
        }
    } else {
        buf.push(b'0');
    }
    // The numbers went into the string backwards. :)
    buf.reverse();
    String::from_utf8(buf).expect("ASCII digits")
}

/// Format a signed integer in `radix`. Translation of `SDL_lltoa()` / `SDL_ltoa()` / `SDL_itoa()`.
pub fn lltoa(value: i64, radix: u32) -> String {
    if value < 0 {
        format!("-{}", ulltoa(value.unsigned_abs(), radix))
    } else {
        ulltoa(value as u64, radix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_utf8_rules() {
        let mut s: &[u8] = "aé€😀".as_bytes();
        assert_eq!(step_utf8(&mut s), 'a' as u32);
        assert_eq!(step_utf8(&mut s), 'é' as u32);
        assert_eq!(step_utf8(&mut s), '€' as u32);
        assert_eq!(step_utf8(&mut s), 0x1F600);
        assert_eq!(step_utf8(&mut s), 0);
        assert!(s.is_empty());

        // Overlong, surrogate, bad trailing byte, truncated: one U+FFFD per byte skipped.
        let mut s: &[u8] = &[0xC0, 0x80, 0xED, 0xA0, 0x80, 0xE2, 0x41, 0xF0, 0x9F];
        let got: Vec<u32> = std::iter::from_fn(|| match step_utf8(&mut s) {
            0 => None,
            c => Some(c),
        })
        .collect();
        assert_eq!(
            got,
            vec![0xFFFD, 0xFFFD, 0xFFFD, 0xFFFD, 0xFFFD, 0xFFFD, 0x41, 0xFFFD, 0xFFFD]
        );

        // NUL ends the string without advancing.
        let mut s: &[u8] = b"a\0b";
        assert_eq!(step_utf8(&mut s), 'a' as u32);
        assert_eq!(step_utf8(&mut s), 0);
        assert_eq!(s, b"\0b");

        let text = "x€y".as_bytes();
        let mut pos = text.len();
        assert_eq!(step_back_utf8(text, &mut pos), 'y' as u32);
        assert_eq!(step_back_utf8(text, &mut pos), '€' as u32);
        assert_eq!(pos, 1);
        assert_eq!(step_back_utf8(text, &mut pos), 'x' as u32);
        assert_eq!(step_back_utf8(text, &mut pos), 0);
    }

    #[test]
    fn ucs4() {
        let mut buf = [0u8; 4];
        assert_eq!(ucs4_to_utf8(0x41, &mut buf), "A");
        assert_eq!(
            ucs4_to_utf8(0x20AC, &mut buf).as_bytes(),
            &[0xE2, 0x82, 0xAC]
        );
        assert_eq!(ucs4_to_utf8(0xD800, &mut buf), "\u{FFFD}");
        assert_eq!(ucs4_to_utf8(0x110000, &mut buf), "\u{FFFD}");
        assert_eq!(ucs4_to_char(0x1F600), '😀');
    }

    #[test]
    fn case_folding() {
        assert_eq!(&*case_fold_unicode('A' as u32), &['a' as u32]);
        assert_eq!(&*case_fold_unicode('a' as u32), &['a' as u32]);
        assert_eq!(&*case_fold_unicode('Σ' as u32), &['σ' as u32]);
        assert_eq!(&*case_fold_unicode('ß' as u32), &['s' as u32, 's' as u32]);
        assert_eq!(&*case_fold_unicode(0x0390), &[0x03B9, 0x0308, 0x0301]);
        assert_eq!(&*case_fold_unicode(0x10400), &[0x10428]); // DESERET CAPITAL LONG I
        assert_eq!(&*case_fold_unicode(0x1F600), &[0x1F600]);
        // Tables are sorted (binary search relies on it).
        assert!(CASE_FOLD1_16.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(CASE_FOLD2_16.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(CASE_FOLD3_16.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(CASE_FOLD1_32.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(
            CASE_FOLD1_16.len() + CASE_FOLD2_16.len() + CASE_FOLD3_16.len() + CASE_FOLD1_32.len(),
            1504
        );
    }

    #[test]
    fn case_folding_matches_upstream_for_every_code_point() {
        // FNV-1a over (folded code points as little-endian u32s, then the
        // count) for every code point 0..=0x10FFFF, computed with upstream's
        // SDL_CaseFoldUnicode() and SDL_casefolding.h hash tables.
        let mut h: u64 = 0xcbf29ce484222325;
        for cp in 0..0x110000u32 {
            let f = case_fold_unicode(cp);
            for &c in f.iter() {
                for k in 0..4 {
                    h ^= ((c >> (8 * k)) & 0xff) as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
            h ^= f.len() as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        assert_eq!(h, 0x101f8adaea7a58e4);
    }

    #[test]
    fn case_insensitive_compare() {
        assert_eq!(strcasecmp("Hello", "hELLO"), Ordering::Equal);
        assert_eq!(strcasecmp("straße", "STRASSE"), Ordering::Equal);
        assert_eq!(strcasecmp("ΣΊΣΥΦΟΣ", "σίσυφοσ"), Ordering::Equal);
        assert_eq!(strcasecmp("apple", "Banana"), Ordering::Less);
        assert_eq!(strcasecmp("b", "A"), Ordering::Greater);
        assert_eq!(strcasecmp("abc", "ab"), Ordering::Greater);
        assert_eq!(strncasecmp("HelloWorld", "helloworms", 7), Ordering::Equal);
        assert_eq!(strncasecmp("HelloWorld", "helloworms", 9), Ordering::Less);
        assert_eq!(strcasestr("The Quick Brown", "quick"), Some(4));
        assert_eq!(strcasestr("€uro EURO", "euro"), Some(7));
        assert_eq!(strcasestr("abc", "zz"), None);
        assert_eq!(strcasestr("abc", ""), Some(0));
    }

    #[test]
    fn utf8_lengths_and_truncation() {
        assert_eq!(utf8strlen("aé€😀"), 4);
        assert_eq!(utf8strlen(&[0xFFu8, b'a'][..]), 2);
        assert_eq!(utf8strnlen("aé€", 3), 2);
        assert_eq!(
            utf8strnlen("aé€", 4),
            3,
            "a truncated sequence counts as a replacement"
        );
        let s = "aé€";
        assert_eq!(utf8strlcpy_len(s, 0), 0);
        assert_eq!(utf8strlcpy_len(s, 3), 1, "don't split é");
        assert_eq!(utf8strlcpy_len(s, 4), 3);
        assert_eq!(utf8strlcpy_len(s, 5), 3, "don't split €");
        assert_eq!(utf8strlcpy_len(s, 7), 6);
        assert_eq!(utf8_truncate("日本語", 7), "日本");
        assert_eq!(utf8_truncate("abc", 100), "abc");
    }

    #[test]
    fn scanners() {
        assert_eq!(strtol("  -42xyz", 10), (-42, 5));
        assert_eq!(strtol("0x1F", 0), (31, 4));
        assert_eq!(strtol("017", 0), (15, 3));
        assert_eq!(strtol("0x", 0), (0, 1), "consume the '0' but not the 'x'");
        assert_eq!(strtol("zz", 36), (36 * 35 + 35, 2));
        assert_eq!(strtol("junk", 10), (0, 0));
        assert_eq!(strtol("99999999999999999999", 10), (i64::MAX, 20));
        assert_eq!(strtol("-99999999999999999999", 10), (i64::MIN, 21));
        assert_eq!(strtoul("-1", 10), (u64::MAX, 2));
        assert_eq!(strtoull("18446744073709551616", 10), (u64::MAX, 20));
        assert_eq!(strtoull("-5", 10), (u64::MAX - 4, 2));
        assert_eq!(strtol("10", 1), (0, 0), "bad radix parses nothing");
        assert_eq!(strtod(" 3.25e2!"), (325.0, 7));
        assert_eq!(strtod("-.5"), (-0.5, 3));
        assert_eq!(strtod("5."), (5.0, 2));
        assert_eq!(strtod("1e"), (1.0, 1));
        assert_eq!(strtod("."), (0.0, 0));
        assert_eq!(ulltoa(255, 16), "FF");
        assert_eq!(ulltoa(0, 2), "0");
        assert_eq!(lltoa(-255, 2), "-11111111");
        assert_eq!(lltoa(i64::MIN, 10), "-9223372036854775808");
    }
}
