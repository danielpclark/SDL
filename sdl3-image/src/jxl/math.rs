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

//! The decoder's math: the fast approximations (the SIMD code at one lane,
//! as highway's scalar target runs it), highway's rounding and conversion
//! operations, and the C library functions it calls (`cbrtf()` with glibc's
//! results, `powf()`, `log()` and `exp()` through SDL's fdlibm, so the
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
        if abs.partial_cmp(&(i32::MAX as f32)) == Some(core::cmp::Ordering::Greater) || abs.is_nan()
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
    let exp =
        f32::from_bits((hwy_convert_to_i32(floorx).wrapping_add(127) as u32).wrapping_shl(23));
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
    let cosx_prescaling =
        x4 * 0.06960438f64 as f32 + (x2 * -0.84087373f64 as f32 + 1.68179268f64 as f32);
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
    // (scalar Min for floats: a NaN operand yields the other one, then
    // HWY_MIN: a < b ? a : b)
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a < b {
        a
    } else {
        b
    }
}

/// Translation of highway's scalar `Max()` on floats.
#[inline]
pub(crate) fn hwy_max(a: f32, b: f32) -> f32 {
    // (scalar Max for floats: a NaN operand yields the other one, then
    // HWY_MAX: a > b ? a : b)
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a > b {
        a
    } else {
        b
    }
}

/// Translation of Highway's `Clamp(v, lo, hi)`: `Min(Max(lo, v), hi)`.
#[inline]
pub(crate) fn hwy_clamp(v: f32, lo: f32, hi: f32) -> f32 {
    hwy_min(hwy_max(lo, v), hi)
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

/// The C library's `logf()`, computed in double through SDL's fdlibm
/// `log()` and rounded.
#[inline]
pub(crate) fn logf(x: f32) -> f32 {
    sdl3::stdlib::math::log(x as f64) as f32
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

/// The C library's `cbrtf()`: Newton's method in double from a bit-level
/// estimate (the exponent divided by three), then a Halley step, rounded
/// once to float. Written for this crate, not translated: it gives the
/// same result as glibc's `cbrtf()` (the C library the reference decoder
/// was built with) for every one of the 2^32 inputs.
pub(crate) fn cbrtf(x: f32) -> f32 {
    if x == 0.0 || !x.is_finite() {
        return x + x;
    }
    let a = (x as f64).abs();
    let mut y = f64::from_bits(a.to_bits() / 3 + 0x2a9f_7893_782d_a1ce);
    for _ in 0..4 {
        y -= (y * y * y - a) / (3.0 * y * y);
    }
    let y3 = y * y * y;
    y = y * (y3 + 2.0 * a) / (2.0 * y3 + a);
    let r = y as f32;
    if x < 0.0 {
        -r
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The C library results the reference decoder gets (glibc 2.39 on
    // x86-64), as bits.

    #[test]
    fn powf_of_the_dequantization_multipliers_is_glibcs() {
        // x_dm_multiplier and b_dm_multiplier: pow(1 / 1.25, qm_scale - 2)
        // for every 3-bit qm_scale
        let expected = [
            0x3fc80000u32,
            0x3fa00000,
            0x3f800000,
            0x3f4ccccd,
            0x3f23d70b,
            0x3f03126f,
            0x3ed1b718,
            0x3ea7c5ad,
        ];
        for (k, &e) in expected.iter().enumerate() {
            assert_eq!(
                powf(1.0 / 1.25f32, k as f32 - 2.0).to_bits(),
                e,
                "qm_scale {k}"
            );
        }
    }

    #[test]
    fn cbrtf_and_logf_are_glibcs() {
        let cbrt = [
            (0xbb789536u32, 0xbe1fb275u32),
            (0x3b789536, 0x3e1fb275),
            (0x3f800000, 0x3f800000),
            (0x41d80000, 0x40400000),
            (0x3a83126f, 0x3dcccccd),
            (0xc1080000, 0xc0029ceb),
            (0x012355e6, 0x2aaeebf1),
            (0x7149f2ca, 0x501502f9),
        ];
        for (x, e) in cbrt {
            assert_eq!(cbrtf(f32::from_bits(x)).to_bits(), e, "cbrtf({x:08x})");
        }
        let log = [
            (0x3f000000u32, 0xbf317218u32),
            (0x3fd9999a, 0x3f07d741),
            (0x3c23d70a, 0xc0935d8e),
            (0x42f6cccd, 0x409a1803),
        ];
        for (x, e) in log {
            assert_eq!(logf(f32::from_bits(x)).to_bits(), e, "logf({x:08x})");
        }
    }
}
