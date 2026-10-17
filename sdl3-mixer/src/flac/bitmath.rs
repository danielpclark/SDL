// Rust translation of src/libFLAC/bitmath.c and the decoder's part of
// src/libFLAC/include/private/bitmath.h from libFLAC 1.3.4
// (https://xiph.org/flac/, at the revision SDL_mixer's external/flac pins).
// Copyright (C) 2001-2009  Josh Coalson
// Copyright (C) 2011-2016  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Bit counting. The compiler intrinsics upstream picks (`__builtin_clz()`
//! and the like) are Rust's `leading_zeros()`.

/// `FLAC__clz_uint32()`. Never used with input 0
#[inline]
pub(crate) fn FLAC__clz_uint32(v: u32) -> u32 {
    debug_assert!(v > 0);
    v.leading_zeros()
}

/// `FLAC__clz2_uint32()`. These two functions work with input 0
#[inline]
pub(crate) fn FLAC__clz2_uint32(v: u32) -> u32 {
    if v == 0 {
        return 32;
    }
    FLAC__clz_uint32(v)
}

/* An example of what FLAC__bitmath_ilog2() computes:
 *
 * ilog2( 0) = assertion failure
 * ilog2( 1) = 0
 * ilog2( 2) = 1
 * ilog2( 3) = 1
 * ilog2( 4) = 2
 * ilog2( 5) = 2
 * ilog2( 6) = 2
 * ilog2( 7) = 2
 * ilog2( 8) = 3
 * ilog2( 9) = 3
 * ilog2(10) = 3
 * ilog2(11) = 3
 * ilog2(12) = 3
 * ilog2(13) = 3
 * ilog2(14) = 3
 * ilog2(15) = 3
 * ilog2(16) = 4
 * ilog2(17) = 4
 * ilog2(18) = 4
 */

/// `FLAC__bitmath_ilog2()`.
#[inline]
pub(crate) fn FLAC__bitmath_ilog2(v: u32) -> u32 {
    debug_assert!(v > 0);
    FLAC__clz_uint32(v) ^ 31
}

/// `FLAC__bitmath_ilog2_wide()`.
#[inline]
pub(crate) fn FLAC__bitmath_ilog2_wide(v: u64) -> u32 {
    debug_assert!(v > 0);
    v.leading_zeros() ^ 63
}

/* An example of what FLAC__bitmath_silog2() computes:
 *
 * silog2(-10) = 5
 * silog2(- 9) = 5
 * silog2(- 8) = 4
 * silog2(- 7) = 4
 * silog2(- 6) = 4
 * silog2(- 5) = 4
 * silog2(- 4) = 3
 * silog2(- 3) = 3
 * silog2(- 2) = 2
 * silog2(- 1) = 2
 * silog2(  0) = 0
 * silog2(  1) = 2
 * silog2(  2) = 3
 * silog2(  3) = 3
 * silog2(  4) = 4
 * silog2(  5) = 4
 * silog2(  6) = 4
 * silog2(  7) = 4
 * silog2(  8) = 5
 * silog2(  9) = 5
 * silog2( 10) = 5
 */
/// `FLAC__bitmath_silog2()`.
pub(crate) fn FLAC__bitmath_silog2(mut v: i64) -> u32 {
    if v == 0 {
        return 0;
    }

    if v == -1 {
        return 2;
    }

    v = if v < 0 { -(v + 1) } else { v };
    FLAC__bitmath_ilog2_wide(v as u64) + 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples() {
        let ilog2 = [0, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4];
        for (i, &want) in ilog2.iter().enumerate() {
            assert_eq!(FLAC__bitmath_ilog2(i as u32 + 1), want);
        }
        let silog2 = [
            5, 5, 4, 4, 4, 4, 3, 3, 2, 2, 0, 2, 3, 3, 4, 4, 4, 4, 5, 5, 5,
        ];
        for (i, &want) in silog2.iter().enumerate() {
            assert_eq!(FLAC__bitmath_silog2(i as i64 - 10), want);
        }
        assert_eq!(FLAC__clz2_uint32(0), 32);
        assert_eq!(FLAC__clz2_uint32(1), 31);
    }
}
