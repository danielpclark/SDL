// Rust translation of lib/jxl/fast_math-inl.h and
// lib/jxl/rational_polynomial-inl.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches), with the scalar
// operations of highway's HWY_SCALAR target (hwy/ops/scalar-inl.h,
// Copyright 2019 Google LLC, Apache-2.0 / BSD-3-Clause) that differ from
// Rust's, and the C library functions the decoder calls.
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// cbrtf() is a translation of the GNU C Library's
// sysdeps/ieee754/flt-32/s_cbrtf.c (glibc 2.39, the C library the
// reference was made with), contributed by Ulrich Drepper
// <drepper@cygnus.com>, 1997; LGPL-2.1-or-later (see LICENSE.txt).

//! The decoder's math: the fast approximations (the SIMD code at one lane,
//! as highway's scalar target runs it), highway's rounding and conversion
//! operations, and the C library functions it calls (`cbrtf()` as glibc
//! computes it, `powf()`, `log()` and `exp()` through SDL's fdlibm, so the
//! results are the same on every platform).

/// Translation of highway's scalar `Floor()` (bit manipulation; -0 and the
/// small negatives as highway gives them).
#[inline]
pub(crate) fn hwy_floor(v: f32) -> f32 {
    const K_MANTISSA_BITS: i32 = 23;
    const K_EXPONENT_MASK: u32 = (1u32 << 8) - 1;
    const K_MANTISSA_MASK: u32 = (1u32 << K_MANTISSA_BITS) - 1;
    const K_BIAS: u32 = K_EXPONENT_MASK / 2;

    let f = v;
    let negative = f < 0.0;

    let mut bits = v.to_bits();

    let exponent = (((bits >> K_MANTISSA_BITS) & K_EXPONENT_MASK) as i32) - K_BIAS as i32;
    // Already an integer.
    if exponent >= K_MANTISSA_BITS {
        return v;
    }
    // |v| <= 1 => -1 or 0.
    if exponent < 0 {
        return if negative { -1.0 } else { 0.0 };
    }

    let mantissa_mask = K_MANTISSA_MASK >> exponent;
    // Already an integer
    if (bits & mantissa_mask) == 0 {
        return v;
    }

    // Clear fractional bits and round down
    if negative {
        bits = bits.wrapping_add((K_MANTISSA_MASK + 1) >> exponent);
    }
    bits &= !mantissa_mask;

    f32::from_bits(bits)
}

/// Translation of highway's scalar `ConvertTo()` from float to int32
/// (rounding toward zero; out of range values saturate).
#[inline]
pub(crate) fn hwy_convert_to_i32(v: f32) -> i32 {
    // (CastValueForF2IConv(): the exponent field decides the range)
    let abs = v.abs();
    let exp_field = abs.to_bits() >> 23;
    // kMinOutOfRangeExpField = min(127 + 32 - 1, 255) = 158
    if exp_field < 158 {
        v as i32
    } else if v.is_sign_negative() && !v.is_nan() {
        i32::MIN
    } else if v.is_nan() {
        // (a NaN's exponent field is 255: LimitsMax or LimitsMin by sign)
        if v.is_sign_negative() {
            i32::MIN
        } else {
            i32::MAX
        }
    } else {
        i32::MAX
    }
}

/// Round-to-nearest even. Translation of highway's scalar `NearestInt()`.
#[inline]
pub(crate) fn hwy_nearest_int(v: f32) -> i32 {
    let abs = v.abs();
    let is_sign = v.is_sign_negative();

    // MantissaEnd<float>() = 8388608
    if abs.partial_cmp(&8388608.0f32) != Some(core::cmp::Ordering::Less) {
        // Huge or NaN
        // Check if too large to cast or NaN
        if abs.partial_cmp(&(i32::MAX as f32)) == Some(core::cmp::Ordering::Greater)
            || abs.is_nan()
        {
            return if is_sign { i32::MIN } else { i32::MAX };
        }
        return v as i32;
    }
    let bias = if v < 0.0 { -0.5f32 } else { 0.5f32 };
    let rounded = (v + bias) as i32;
    if rounded == 0 {
        return 0;
    }
    let mut offset = 0;
    // Round to even
    if (rounded & 1) != 0 && ((rounded as f32) - v).abs() == 0.5 {
        offset = if is_sign { -1 } else { 1 };
    }
    rounded - offset
}

/// Approximates smooth functions via rational polynomials (i.e. dividing two
/// polynomials). Evaluates polynomials via Horner's scheme, which is faster
/// than Clenshaw recurrence for Chebyshev polynomials. Translation of
/// `EvalRationalPolynomial()` (at one lane: the coefficients without their
/// 4x replication).
#[inline]
pub(crate) fn eval_rational_polynomial(x: f32, p: &[f32], q: &[f32]) -> f32 {
    let k_deg_p = p.len() - 1;
    let k_deg_q = q.len() - 1;
    let mut yp = p[k_deg_p];
    let mut yq = q[k_deg_q];
    let mut n = 1;
    while n <= 7 {
        if k_deg_p >= n {
            yp = yp * x + p[k_deg_p - n];
        }
        if k_deg_q >= n {
            yq = yq * x + q[k_deg_q - n];
        }
        n += 1;
    }

    yp / yq
}

/// Computes base-2 logarithm like std::log2. Undefined if negative / NaN.
/// L1 error ~3.9E-6. Translation of `FastLog2f()`.
#[inline]
pub(crate) fn fast_log2f(x: f32) -> f32 {
    // 2,2 rational polynomial approximation of std::log1p(x) / std::log(2).
    const P: [f32; 3] = [
        -1.8503833400518310E-06,
        1.4287160470083755E+00,
        7.4245873327820566E-01,
    ];
    const Q: [f32; 3] = [
        9.9032814277590719E-01,
        1.0096718572241148E+00,
        1.7409343003366853E-01,
    ];

    let x_bits = x.to_bits() as i32;

    // Range reduction to [-1/3, 1/3] - 3 integer, 2 float ops
    let exp_bits = x_bits.wrapping_sub(0x3f2aaaab); // = 2/3
    // Shifted exponent = log2; also used to clear mantissa.
    let exp_shifted = exp_bits >> 23;
    let mantissa = f32::from_bits(x_bits.wrapping_sub(exp_shifted.wrapping_shl(23)) as u32);
    let exp_val = exp_shifted as f32;
    eval_rational_polynomial(mantissa - 1.0f32, &P, &Q) + exp_val
}

/// max relative error ~3e-7. Translation of `FastPow2f()`.
#[inline]
pub(crate) fn fast_pow2f(x: f32) -> f32 {
    let floorx = hwy_floor(x);
    let exp = f32::from_bits(
        (hwy_convert_to_i32(floorx).wrapping_add(127) as u32).wrapping_shl(23),
    );
    let frac = x - floorx;
    let mut num = frac + 1.01749063e+01f32;
    num = num * frac + 4.88687798e+01f32;
    num = num * frac + 9.85506591e+01f32;
    num *= exp;
    let mut den = frac * 2.10242958e-01f32 + -2.22328856e-02f32;
    den = den * frac + -1.94414990e+01f32;
    den = den * frac + 9.85506633e+01f32;
    num / den
}

/// max relative error ~3e-5. Translation of `FastPowf()`.
#[inline]
pub(crate) fn fast_powf(base: f32, exponent: f32) -> f32 {
    fast_pow2f(fast_log2f(base) * exponent)
}

/// Computes cosine like std::cos.
/// L1 error 7e-5. Translation of `FastCosf()`.
#[inline]
pub(crate) fn fast_cosf(x: f32) -> f32 {
    const K_PI: f64 = super::base::K_PI;
    // Step 1: range reduction to [0, 2pi)
    let pi2 = (K_PI * 2.0f32 as f64) as f32;
    let pi2_inv = (0.5f32 as f64 / K_PI) as f32;
    let npi2 = hwy_floor(x * pi2_inv) * pi2;
    let xmodpi2 = x - npi2;
    // Step 2: range reduction to [0, pi]
    let x_pi = hwy_min(xmodpi2, pi2 - xmodpi2);
    // Step 3: range reduction to [0, pi/2]
    let above_pihalf = x_pi >= (K_PI / 2.0f32 as f64) as f32;
    let x_pihalf = if above_pihalf {
        K_PI as f32 - x_pi
    } else {
        x_pi
    };
    // Step 4: Taylor-like approximation, scaled by 2**0.75 to make angle
    // duplication steps faster, on x/4.
    let xs = x_pihalf * 0.25f32;
    let x2 = xs * xs;
    let x4 = x2 * x2;
    let cosx_prescaling = x4 * 0.06960438f64 as f32 + (x2 * -0.84087373f64 as f32 + 1.68179268f64 as f32);
    // Step 5: angle duplication.
    let cosx_scale1 = cosx_prescaling * cosx_prescaling + -1.414213562f64 as f32;
    let cosx_scale2 = cosx_scale1 * cosx_scale1 + -1.0f32;
    // Step 6: change sign if needed.
    let signbit: u32 = if above_pihalf { 1u32 << 31 } else { 0 };
    f32::from_bits(signbit ^ cosx_scale2.to_bits())
}

/// Computes the error function like std::erf.
/// L1 error 7e-4. Translation of `FastErff()`.
#[inline]
pub(crate) fn fast_erff(x: f32) -> f32 {
    // Formula from
    // https://en.wikipedia.org/wiki/Error_function#Numerical_approximations
    // but constants have been recomputed.
    let xle0 = x <= 0.0;
    let absx = x.abs();
    // Compute 1 - 1 / ((((x * a + b) * x + c) * x + d) * x + 1)**4
    let denom1 = absx * 7.77394369e-02f32 + 2.05260015e-04f32;
    let denom2 = denom1 * absx + 2.32120216e-01f32;
    let denom3 = denom2 * absx + 2.77820801e-01f32;
    let denom4 = denom3 * absx + 1.0f32;
    let denom5 = denom4 * denom4;
    let inv_denom5 = 1.0f32 / denom5;
    let result = 1.0f32 - inv_denom5 * inv_denom5;
    // Change sign if needed.
    let signbit: u32 = if xle0 { 1u32 << 31 } else { 0 };
    f32::from_bits(signbit ^ result.to_bits())
}

/// Returns cbrt(x) + add with 6 ulp max error.
/// Modified from vectormath_exp.h, Apache 2 license.
/// https://www.agner.org/optimize/vectorclass.zip
/// Translation of `CubeRootAndAdd()`.
#[inline]
pub(crate) fn cube_root_and_add(x: f32, add: f32) -> f32 {
    let k_exp_bias: i32 = 0x54800000; // cast(1.) + cast(1.) / 3
    let k_exp_mul: i32 = 0x002AAAAA; // shifted 1/3
    let k1_3 = 1.0f32 / 3.0;
    let k4_3 = 4.0f32 / 3.0;

    let xa = x; // assume inputs never negative
    let xa_3 = k1_3 * xa;

    // Multiply exponent by -1/3
    let m1 = xa.to_bits() as i32;
    // Special case for 0. 0 is represented with an exponent of 0, so the
    // "kExpBias - 1/3 * exp" below gives the wrong result. The IfThenZeroElse()
    // sets those values as 0, which prevents having NaNs in the computations
    // below.
    // TODO(eustas): use fused op
    let m2 = if m1 == 0 {
        0
    } else {
        k_exp_bias.wrapping_sub((m1 >> 23).wrapping_mul(k_exp_mul))
    };
    let mut r = f32::from_bits(m2 as u32);

    // Newton-Raphson iterations
    for _ in 0..3 {
        let r2 = r * r;
        r = k4_3 * r - xa_3 * (r2 * r2);
    }
    // Final iteration
    let mut r2 = r * r;
    r = k1_3 * (r - xa * (r2 * r2)) + r;
    r2 = r * r;
    r = r2 * x + add;

    r
}

/// Translation of highway's scalar `Min()` on floats.
#[inline]
pub(crate) fn hwy_min(a: f32, b: f32) -> f32 {
    // (HWY_MIN: a < b ? a : b)
    if a < b {
        a
    } else {
        b
    }
}

/// Translation of highway's scalar `Max()` on floats.
#[inline]
pub(crate) fn hwy_max(a: f32, b: f32) -> f32 {
    // (HWY_MAX: a > b ? a : b)
    if a > b {
        a
    } else {
        b
    }
}

/// The C library's `roundf()` (half away from zero).
#[inline]
pub(crate) fn roundf(x: f32) -> f32 {
    x.round()
}

/// The C library's `powf()`, computed in double through SDL's fdlibm
/// `pow()` and rounded.
#[inline]
pub(crate) fn powf(x: f32, y: f32) -> f32 {
    sdl3::stdlib::math::pow(x as f64, y as f64) as f32
}

/// The C library's `pow()` (SDL's fdlibm).
#[inline]
pub(crate) fn pow(x: f64, y: f64) -> f64 {
    sdl3::stdlib::math::pow(x, y)
}

/// The C library's `log()` (SDL's fdlibm).
#[inline]
pub(crate) fn log(x: f64) -> f64 {
    sdl3::stdlib::math::log(x)
}

/// The C library's `exp()` (SDL's fdlibm).
#[inline]
pub(crate) fn exp(x: f64) -> f64 {
    sdl3::stdlib::math::exp(x)
}

/// The C library's `log2()` (SDL's fdlibm `log()` divided by ln 2).
#[inline]
#[allow(dead_code)]
pub(crate) fn log2(x: f64) -> f64 {
    sdl3::stdlib::math::log(x) / core::f64::consts::LN_2
}

/// The C library's `hypotf()`: as glibc computes it, the square root of the
/// sum of squares in double.
#[inline]
pub(crate) fn hypotf(x: f32, y: f32) -> f32 {
    let (x, y) = (x as f64, y as f64);
    (x * x + y * y).sqrt() as f32
}

// --- glibc's s_cbrtf.c ---

const CBRT2: f64 = 1.2599210498948731648; /* 2^(1/3) */
const SQR_CBRT2: f64 = 1.5874010519681994748; /* 2^(2/3) */

const CBRT_FACTOR: [f64; 5] = [1.0 / SQR_CBRT2, 1.0 / CBRT2, 1.0, CBRT2, SQR_CBRT2];

/// glibc's `frexpf()`.
fn frexpf(x: f32) -> (f32, i32) {
    let mut hx = x.to_bits() as i32;
    let ix = 0x7fffffff & hx;
    let mut e = 0;
    if ix >= 0x7f800000 || ix == 0 {
        return (x + x, 0); /* 0,inf,nan */
    }
    let mut x = x;
    if ix < 0x00800000 {
        /* subnormal */
        x *= 3.3554432000e+07f32; /* 0x4c000000 */
        hx = x.to_bits() as i32;
        let ix = hx & 0x7fffffff;
        e = -25;
        e += (ix >> 23) - 126;
    } else {
        e += (ix >> 23) - 126;
    }
    hx = (hx & 0x807fffffu32 as i32) | 0x3f000000;
    (f32::from_bits(hx as u32), e)
}

/// glibc's `ldexpf()` (for the exponents cbrtf() gives, which stay in
/// range).
fn ldexpf(x: f32, n: i32) -> f32 {
    sdl3::stdlib::math::scalbnf(x, n)
}

/// The C library's `cbrtf()`, as glibc 2.39 computes it.
pub(crate) fn cbrtf(x: f32) -> f32 {
    /* Reduce X.  XM now is an range 1.0 to 0.5.  */
    let (xm, xe) = frexpf(x.abs());

    /* If X is not finite or is null return it (with raising exceptions
    if necessary.
    Note: *Our* version of `frexp' sets XE to zero if the argument is
    Inf or NaN.  This is not portable but faster.  */
    if xe == 0 && (x == 0.0 || !x.is_finite()) {
        return x + x;
    }

    let xm_d = xm as f64;
    let u = (0.492659620528969547 + (0.697570460207922770 - 0.191502161678719066 * xm_d) * xm_d)
        as f32;

    let t2 = u * u * u;

    let ym = (u as f64 * (t2 as f64 + 2.0 * xm_d) / (2.0 * t2 as f64 + xm_d)
        * CBRT_FACTOR[(2 + xe % 3) as usize]) as f32;

    ldexpf(if x > 0.0 { ym } else { -ym }, xe / 3)
}
