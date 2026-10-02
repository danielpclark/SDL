// Rust translation of src/libm/math_private.h from Simple DirectMedia Layer.
//
// ====================================================
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunPro, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
// ====================================================
//
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The word-access macros of `math_private.h`.
//!
//! The original fdlibm code dug the two 32 bit words out of a 64 bit IEEE
//! value through pointer casts; uClibc replaced that with a union. Rust's
//! `to_bits`/`from_bits` are the safe equivalent, and endianness never enters
//! into it.

/// `GET_HIGH_WORD`: the more significant 32 bits of a double.
#[inline(always)]
pub(super) fn hi(d: f64) -> u32 {
    (d.to_bits() >> 32) as u32
}

/// `GET_LOW_WORD`: the less significant 32 bits of a double.
#[inline(always)]
pub(super) fn lo(d: f64) -> u32 {
    d.to_bits() as u32
}

/// `EXTRACT_WORDS(ix0, ix1, d)` with the high word as the signed `int32_t`
/// fdlibm declares it as.
#[inline(always)]
pub(super) fn words(d: f64) -> (i32, u32) {
    (hi(d) as i32, lo(d))
}

/// `INSERT_WORDS(d, ix0, ix1)`.
#[inline(always)]
pub(super) fn from_words(hi: u32, lo: u32) -> f64 {
    f64::from_bits(((hi as u64) << 32) | lo as u64)
}

/// `SET_HIGH_WORD(d, v)`.
#[inline(always)]
pub(super) fn with_hi(d: f64, v: u32) -> f64 {
    from_words(v, lo(d))
}

/// `SET_LOW_WORD(d, v)`.
#[inline(always)]
pub(super) fn with_lo(d: f64, v: u32) -> f64 {
    from_words(hi(d), v)
}

/// `GET_FLOAT_WORD`.
#[inline(always)]
pub(super) fn float_word(f: f32) -> i32 {
    f.to_bits() as i32
}
