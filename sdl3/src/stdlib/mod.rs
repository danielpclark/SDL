// Rust translation of src/stdlib/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Portable algorithms from SDL's standard-library shim.
//!
//! Most of upstream `src/stdlib/` exists to give SDL a C runtime it can rely
//! on everywhere; Rust's `core`/`alloc`/`std` cover that. Translated here are
//! the pieces with SDL-specific behaviour that other code depends on
//! bit-for-bit: the CRC and MurmurHash3 routines, the pseudo-random
//! generator, the bundled libm ([`math`]), UTF-8 and case-folding string
//! helpers ([`string`]), character-set conversion ([`iconv`]) and the
//! environment API ([`Environment`], [`getenv`]).
//!
//! Intentionally not translated: `SDL_malloc.c` (dlmalloc; Rust's allocator
//! replaces it), `SDL_memcpy.c`/`SDL_memmove.c`/`SDL_memset.c` (slice
//! methods), `SDL_qsort.c` (`sort_unstable_by`), `SDL_strtokr.c`
//! (`split`), `SDL_mslibc.c` (MSVC runtime shims), and `SDL_aligned_alloc`
//! (`std::alloc::Layout`).

mod casefolding;
pub mod crc16;
pub mod crc32;
mod getenv;
pub mod iconv;
pub mod math;
pub mod murmur3;
pub mod random;
pub mod string;

pub use getenv::{getenv, getenv_unsafe, setenv_unsafe, unsetenv_unsafe, Environment};
pub(crate) use getenv::{init_environment, quit_environment};

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

/// `SDL_strtol(s, NULL, 10)`.
pub(crate) fn strtol(s: &str) -> i64 {
    string::strtol(s, 10).0
}

/// `SDL_strtoll(s, NULL, 0)`: base auto-detected from a `0x`/`0` prefix.
pub(crate) fn strtoll_base0(s: &str) -> i64 {
    string::strtoll(s, 0).0
}

/// `SDL_atof()`: leading floating point number, trailing garbage ignored.
pub(crate) fn atof(s: &str) -> f64 {
    string::strtod(s).0
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
