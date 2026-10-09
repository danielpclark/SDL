// Rust translation of src/utils.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Rounding and fractions. The byte order helpers (`avifNTOHS()` and the
//! like) are `from_be_bytes()` where the stream reads, and libavif's
//! growable arrays (`AVIF_ARRAY_DECLARE`, `avifArrayCreate()`,
//! `avifArrayPush()`, ...) are vectors, pushes into which are fallible as
//! upstream's (see [`try_push`]). The double to fraction conversions are
//! only used by the gain map code, which SDL_image's build doesn't have.

use super::avif::AvifFraction;

/// Translation of `avifRoundf()`.
pub(crate) fn avif_roundf(v: f32) -> f32 {
    (v + 0.5f32).floor()
}

// Thanks, Rob Pike! https://commandcenter.blogspot.nl/2012/04/byte-order-fallacy.html
// (the stream reads big-endian values with from_be_bytes())

/// `avifArrayPush()`: append an element, `false` (upstream's `NULL`) when
/// the array can't grow.
pub(crate) fn try_push<T>(v: &mut Vec<T>, item: T) -> bool {
    if v.len() == v.capacity() && v.try_reserve(1).is_err() {
        return false;
    }
    v.push(item);
    true
}

/// |a| and |b| hold int32_t values. The int64_t type is used so that we can negate INT32_MIN without
/// overflowing int32_t.
///
/// Translation of `calcGCD()`.
fn calc_gcd(mut a: i64, mut b: i64) -> i64 {
    if a < 0 {
        a *= -1;
    }
    if b < 0 {
        b *= -1;
    }
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

/// Translation of `avifFractionSimplify()`.
pub(crate) fn avif_fraction_simplify(f: &mut AvifFraction) {
    let gcd = calc_gcd(f.n as i64, f.d as i64);
    if gcd > 1 {
        f.n = (f.n as i64 / gcd) as i32;
        f.d = (f.d as i64 / gcd) as i32;
    }
}

/// Translation of `overflowsInt32()`.
fn overflows_int32(x: i64) -> bool {
    (x < i32::MIN as i64) || (x > i32::MAX as i64)
}

/// Makes the fractions have a common denominator. Translation of
/// `avifFractionCD()`.
pub(crate) fn avif_fraction_cd(a: &mut AvifFraction, b: &mut AvifFraction) -> bool {
    avif_fraction_simplify(a);
    avif_fraction_simplify(b);
    if a.d != b.d {
        let ad = a.d as i64;
        let bd = b.d as i64;
        let an_new = a.n as i64 * bd;
        let ad_new = a.d as i64 * bd;
        let bn_new = b.n as i64 * ad;
        let bd_new = b.d as i64 * ad;
        if overflows_int32(an_new)
            || overflows_int32(ad_new)
            || overflows_int32(bn_new)
            || overflows_int32(bd_new)
        {
            return false;
        }
        a.n = an_new as i32;
        a.d = ad_new as i32;
        b.n = bn_new as i32;
        b.d = bd_new as i32;
    }
    true
}

/// Translation of `avifFractionAdd()`.
pub(crate) fn avif_fraction_add(
    mut a: AvifFraction,
    mut b: AvifFraction,
    result: &mut AvifFraction,
) -> bool {
    if !avif_fraction_cd(&mut a, &mut b) {
        return false;
    }

    let result_n = a.n as i64 + b.n as i64;
    if overflows_int32(result_n) {
        return false;
    }
    result.n = result_n as i32;
    result.d = a.d;

    avif_fraction_simplify(result);
    true
}

/// Translation of `avifFractionSub()`.
pub(crate) fn avif_fraction_sub(
    mut a: AvifFraction,
    mut b: AvifFraction,
    result: &mut AvifFraction,
) -> bool {
    if !avif_fraction_cd(&mut a, &mut b) {
        return false;
    }

    let result_n = a.n as i64 - b.n as i64;
    if overflows_int32(result_n) {
        return false;
    }
    result.n = result_n as i32;
    result.d = a.d;

    avif_fraction_simplify(result);
    true
}
