// Rust translation of celt/mathops.h and celt/mathops.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2002-2008 Jean-Marc Valin
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Written by Jean-Marc Valin, and Yunho Huh
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Various math functions, the float versions (without `FLOAT_APPROX`,
//! which SDL_mixer's CMake leaves off): they call the C library's `exp()`,
//! `log()`, `cos()` and `sqrt()` in double precision, which here are
//! sdl3's translations of SDL's (fdlibm) math functions, so the results
//! are the same on every platform.

use super::entcode::ec_ilog;

pub(crate) const PI: f32 = 3.141592653;

/*Compute floor(sqrt(_val)) with exact arithmetic.
_val must be greater than 0.
This has been tested on all possible 32-bit inputs greater than 0.*/
/// Translation of `isqrt32()`.
pub(crate) fn isqrt32(mut val: u32) -> u32 {
    /*Uses the second method from
    http://www.azillionmonkeys.com/qed/sqroot.html
    The main idea is to search for the largest binary digit b such that
    (g+b)*(g+b) <= _val, and add it to the solution g.*/
    let mut g: u32 = 0;
    let mut bshift = (ec_ilog(val) - 1) >> 1;
    let mut b: u32 = 1u32.wrapping_shl(bshift as u32);
    loop {
        let t = ((g << 1).wrapping_add(b)).wrapping_shl(bshift as u32);
        if t <= val {
            g += b;
            val -= t;
        }
        b >>= 1;
        bshift -= 1;
        if bshift < 0 {
            break;
        }
    }
    g
}

/// Translation of `celt_maxabs16()`.
#[allow(dead_code)]
pub(crate) fn celt_maxabs16(x: &[f32], len: usize) -> f32 {
    let mut maxval = 0.0f32;
    let mut minval = 0.0f32;
    for &v in &x[..len] {
        maxval = super::arch::max16(maxval, v);
        minval = super::arch::min16(minval, v);
    }
    super::arch::max32(maxval, -minval)
}

/// Translation of `celt_sqrt()`: `((float)sqrt(x))`.
#[inline(always)]
pub(crate) fn celt_sqrt(x: f64) -> f32 {
    x.sqrt() as f32
}

/// Translation of `celt_rsqrt()`: `(1.f/celt_sqrt(x))`.
#[inline(always)]
pub(crate) fn celt_rsqrt(x: f64) -> f32 {
    1.0f32 / celt_sqrt(x)
}

/// Translation of `celt_rsqrt_norm()`.
#[inline(always)]
pub(crate) fn celt_rsqrt_norm(x: f32) -> f32 {
    celt_rsqrt(x as f64)
}

/// Translation of `celt_cos_norm()`: `((float)cos((.5f*PI)*(x)))`.
#[inline(always)]
pub(crate) fn celt_cos_norm(x: f32) -> f32 {
    sdl3::stdlib::math::cos(((0.5f32 * PI) * x) as f64) as f32
}

/// Translation of `celt_rcp()`: `(1.f/(x))`.
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn celt_rcp(x: f32) -> f32 {
    1.0f32 / x
}

/// Translation of `celt_div()`: `((a)/(b))`.
#[inline(always)]
pub(crate) fn celt_div(a: f32, b: f32) -> f32 {
    a / b
}

/// Translation of `frac_div32()`: `((float)(a)/(b))`.
#[inline(always)]
pub(crate) fn frac_div32(a: f32, b: f32) -> f32 {
    a / b
}

/// Translation of `celt_log2()`: `((float)(1.442695040888963387*log(x)))`.
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn celt_log2(x: f32) -> f32 {
    (1.442695040888963387f64 * sdl3::stdlib::math::log(x as f64)) as f32
}

/// Translation of `celt_exp2()`: `((float)exp(0.6931471805599453094*(x)))`.
#[inline(always)]
pub(crate) fn celt_exp2(x: f32) -> f32 {
    sdl3::stdlib::math::exp(0.6931471805599453094f64 * x as f64) as f32
}
