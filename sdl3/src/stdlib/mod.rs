// Rust translation of src/stdlib/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Portable algorithms from SDL's standard-library shim.
//!
//! Most of upstream `src/stdlib/` exists to give SDL a C runtime it can rely
//! on everywhere; Rust's `core`/`alloc`/`std` cover that. Translated here are
//! the pieces with SDL-specific behaviour that other code depends on
//! bit-for-bit: the CRC and MurmurHash3 routines and the pseudo-random
//! generator.

pub mod crc16;
pub mod crc32;
pub mod math;
pub mod murmur3;
pub mod random;

pub use crc16::crc16;
pub use crc32::crc32;
pub use murmur3::murmur3_32;
pub use random::Rng;

/// Pack four ASCII bytes into a little-endian "four character code".
/// Translation of `SDL_FOURCC()`.
#[inline]
pub const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

/// `SDL_atoi()`: optional sign and leading decimal digits, trailing garbage
/// ignored, 0 when nothing parses.
pub(crate) fn atoi(s: &str) -> i32 {
    strtol(s) as i32
}

/// `SDL_strtol(s, NULL, 10)` semantics.
pub(crate) fn strtol(s: &str) -> i64 {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut negative = false;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        negative = bytes[i] == b'-';
        i += 1;
    }
    let mut value: i64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add((bytes[i] - b'0') as i64);
        i += 1;
    }
    if negative {
        -value
    } else {
        value
    }
}

/// `SDL_strtoll(s, NULL, 0)`: base auto-detected from a `0x`/`0` prefix.
pub(crate) fn strtoll_base0(s: &str) -> i64 {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut negative = false;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        negative = bytes[i] == b'-';
        i += 1;
    }
    let mut base: i64 = 10;
    if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
        base = 16;
        i += 2;
    } else if i < bytes.len() && bytes[i] == b'0' {
        base = 8;
        i += 1;
    }
    let mut value: i64 = 0;
    while i < bytes.len() {
        let Some(d) = (bytes[i] as char).to_digit(base as u32) else {
            break;
        };
        value = value.saturating_mul(base).saturating_add(d as i64);
        i += 1;
    }
    if negative {
        -value
    } else {
        value
    }
}

/// `SDL_atof()`: leading floating point number, trailing garbage ignored.
pub(crate) fn atof(s: &str) -> f64 {
    let t = s.trim_start();
    let bytes = t.as_bytes();
    let mut end = 0;
    let (mut seen_digit, mut seen_dot, mut seen_exp) = (false, false, false);
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit() {
            seen_digit = true;
            end = i + 1;
        } else if c == b'.' && !seen_dot && !seen_exp {
            seen_dot = true;
        } else if (c == b'e' || c == b'E') && seen_digit && !seen_exp {
            seen_exp = true;
            if i + 1 < bytes.len() && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-') {
                i += 1;
            }
        } else {
            break;
        }
        i += 1;
    }
    if end == 0 {
        return 0.0;
    }
    t[..end].parse::<f64>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_matches_c() {
        assert_eq!(fourcc(b'Y', b'V', b'1', b'2'), 0x32315659);
    }

    #[test]
    fn c_style_parsers() {
        assert_eq!(atoi("42abc"), 42);
        assert_eq!(atoi("  -7"), -7);
        assert_eq!(atoi("abc"), 0);
        assert_eq!(strtoll_base0("0x10"), 16);
        assert_eq!(strtoll_base0("010"), 8);
        assert_eq!(strtoll_base0("10"), 10);
        assert_eq!(atof("1.5x"), 1.5);
        assert_eq!(atof("-2e2"), -200.0);
        assert_eq!(atof("nope"), 0.0);
        assert_eq!(atof("3."), 3.0);
    }
}
