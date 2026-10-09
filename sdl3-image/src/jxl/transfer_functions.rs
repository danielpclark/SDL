// Rust translation of lib/jxl/transfer_functions-inl.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Transfer functions for color encodings. The vector versions are written
//! for one lane, as highway's scalar target runs them.

use super::math::{eval_rational_polynomial, exp, fast_log2f, fast_powf, log, pow};

// Definitions for BT.2100-2 transfer functions (used inside/outside SIMD):
// "display" is linear light (nits) normalized to [0, 1].
// "encoded" is a nonlinear encoding (e.g. PQ) in [0, 1].
// "scene" is a linear function of photon counts, normalized to [0, 1].

// Despite the stated ranges, we need unbounded transfer functions: see
// http://www.littlecms.com/CIC18_UnboundedCMM.pdf. Inputs can be negative or
// above 1 due to chromatic adaptation. To avoid severe round-trip errors caused
// by clamping, we mirror negative inputs via copysign (f(-x) = -f(x), see
// https://developer.apple.com/documentation/coregraphics/cgcolorspace/1644735-extendedsrgb)
// and extend the function domains above 1.

const K_SIGN: u32 = 0x80000000;

/// `copysignf()` on a double (the arguments are converted to float).
#[inline]
fn copysignf(x: f64, s: f64) -> f64 {
    (x as f32).copysign(s as f32) as f64
}

/// Hybrid Log-Gamma. Translation of `TF_HLG`.
pub(crate) struct TfHlg;

impl TfHlg {
    const K_A: f64 = 0.17883277;
    const K_RA: f64 = 1.0 / Self::K_A;
    const K_B: f64 = 1.0 - 4.0 * Self::K_A;
    const K_C: f64 = 0.5599107295;
    const K_DIV12: f64 = 1.0 / 12.0;

    // EOTF. e = encoded.
    pub(crate) fn display_from_encoded(&self, e: f64) -> f64 {
        Self::ootf(Self::inv_oetf(e))
    }

    // Inverse EOTF. d = display.
    #[allow(dead_code)]
    pub(crate) fn encoded_from_display(&self, d: f64) -> f64 {
        Self::oetf(Self::inv_ootf(d))
    }

    // Maximum error 5e-7.
    pub(crate) fn encoded_from_display_v(&self, x: f32) -> f32 {
        let original_sign = x.to_bits() & K_SIGN;
        let x = f32::from_bits(x.to_bits() & !K_SIGN); // abs
        let below_div12 = (3.0f32 * x).sqrt();
        let e = (Self::K_A * 0.693147181f32 as f64) as f32
            * fast_log2f(12.0f32 * x + (-Self::K_B) as f32)
            + Self::K_C as f32;
        let magnitude = if x <= Self::K_DIV12 as f32 {
            below_div12
        } else {
            e
        };
        f32::from_bits((magnitude.to_bits() & !K_SIGN) | original_sign)
    }

    // OETF (defines the HLG approach). s = scene, returns encoded.
    #[allow(dead_code)]
    fn oetf(mut s: f64) -> f64 {
        if s == 0.0 {
            return 0.0;
        }
        let original_sign = s;
        s = s.abs();

        if s <= Self::K_DIV12 {
            return copysignf((3.0 * s).sqrt(), original_sign);
        }

        let e = Self::K_A * log(12.0 * s - Self::K_B) + Self::K_C;
        debug_assert!(e > 0.0);
        copysignf(e, original_sign)
    }

    // e = encoded, returns scene.
    fn inv_oetf(mut e: f64) -> f64 {
        if e == 0.0 {
            return 0.0;
        }
        let original_sign = e;
        e = e.abs();

        if e <= 0.5 {
            return copysignf(e * e * (1.0 / 3.0), original_sign);
        }

        let s = (exp((e - Self::K_C) * Self::K_RA) + Self::K_B) * Self::K_DIV12;
        debug_assert!(s >= 0.0);
        copysignf(s, original_sign)
    }

    // s = scene, returns display.
    fn ootf(s: f64) -> f64 {
        // The actual (red channel) OOTF is RD = alpha * YS^(gamma-1) * RS, where
        // YS = 0.2627 * RS + 0.6780 * GS + 0.0593 * BS. Let alpha = 1 so we return
        // "display" (normalized [0, 1]) instead of nits. Our transfer function
        // interface does not allow a dependency on YS. Fortunately, the system
        // gamma at 334 nits is 1.0, so this reduces to RD = RS.
        s
    }

    // d = display, returns scene.
    #[allow(dead_code)]
    fn inv_ootf(d: f64) -> f64 {
        d // see OOTF().
    }
}

/// Translation of `TF_709`.
pub(crate) struct Tf709;

impl Tf709 {
    const K_THRESH: f64 = 0.018;
    const K_MUL_LOW: f64 = 4.5;
    const K_MUL_HI: f64 = 1.099;
    const K_POW_HI: f64 = 0.45;
    const K_SUB: f64 = -0.099;

    const K_INV_THRESH: f64 = 0.081;
    const K_INV_MUL_LOW: f64 = 1.0 / 4.5;
    const K_INV_MUL_HI: f64 = 1.0 / 1.099;
    const K_INV_POW_HI: f64 = 1.0 / 0.45;
    const K_INV_ADD: f64 = 0.099 * Self::K_INV_MUL_HI;

    #[allow(dead_code)]
    pub(crate) fn encoded_from_display(&self, d: f64) -> f64 {
        if d < Self::K_THRESH {
            return Self::K_MUL_LOW * d;
        }
        Self::K_MUL_HI * pow(d, Self::K_POW_HI) + Self::K_SUB
    }

    // Maximum error 1e-6.
    pub(crate) fn encoded_from_display_v(&self, x: f32) -> f32 {
        let low = Self::K_MUL_LOW as f32 * x;
        let hi = Self::K_MUL_HI as f32 * fast_powf(x, Self::K_POW_HI as f32) + Self::K_SUB as f32;
        if x <= Self::K_THRESH as f32 {
            low
        } else {
            hi
        }
    }

    pub(crate) fn display_from_encoded_v(&self, x: f32) -> f32 {
        let low = Self::K_INV_MUL_LOW as f32 * x;
        let hi = fast_powf(
            x * Self::K_INV_MUL_HI as f32 + Self::K_INV_ADD as f32,
            Self::K_INV_POW_HI as f32,
        );
        if x < Self::K_INV_THRESH as f32 {
            low
        } else {
            hi
        }
    }
}

/// Perceptual Quantization. Translation of `TF_PQ`.
pub(crate) struct TfPq;

impl TfPq {
    const K_M1: f64 = 2610.0 / 16384.0;
    const K_M2: f64 = (2523.0 / 4096.0) * 128.0;
    const K_C1: f64 = 3424.0 / 4096.0;
    const K_C2: f64 = (2413.0 / 4096.0) * 32.0;
    const K_C3: f64 = (2392.0 / 4096.0) * 32.0;

    // EOTF (defines the PQ approach). e = encoded.
    pub(crate) fn display_from_encoded(&self, mut e: f64) -> f64 {
        if e == 0.0 {
            return 0.0;
        }
        let original_sign = e;
        e = e.abs();

        let xp = pow(e, 1.0 / Self::K_M2);
        let num = (xp - Self::K_C1).max(0.0);
        let den = Self::K_C2 - Self::K_C3 * xp;
        let d = pow(num / den, 1.0 / Self::K_M1);
        copysignf(d, original_sign)
    }

    // Maximum error 3e-6
    pub(crate) fn display_from_encoded_v(&self, x: f32) -> f32 {
        let original_sign = x.to_bits() & K_SIGN;
        let x = f32::from_bits(x.to_bits() & !K_SIGN); // abs
                                                       // 4-over-4-degree rational polynomial approximation on x+x*x. This improves
                                                       // the maximum error by about 5x over a rational polynomial for x.
        let xpxx = x * x + x;
        const P: [f32; 5] = [
            2.62975656e-04,
            -6.23553089e-03,
            7.38602301e-01,
            2.64553172e+00,
            5.50034862e-01,
        ];
        const Q: [f32; 5] = [
            4.21350107e+02,
            -4.28736818e+02,
            1.74364667e+02,
            -3.39078883e+01,
            2.67718770e+00,
        ];
        let magnitude = eval_rational_polynomial(xpxx, &P, &Q);
        f32::from_bits((magnitude.to_bits() & !K_SIGN) | original_sign)
    }

    // Inverse EOTF. d = display.
    #[allow(dead_code)]
    pub(crate) fn encoded_from_display(&self, mut d: f64) -> f64 {
        if d == 0.0 {
            return 0.0;
        }
        let original_sign = d;
        d = d.abs();

        let xp = pow(d, Self::K_M1);
        let num = Self::K_C1 + xp * Self::K_C2;
        let den = 1.0 + xp * Self::K_C3;
        let e = pow(num / den, Self::K_M2);
        copysignf(e, original_sign)
    }

    // Maximum error 7e-7.
    pub(crate) fn encoded_from_display_v(&self, x: f32) -> f32 {
        let original_sign = x.to_bits() & K_SIGN;
        let x = f32::from_bits(x.to_bits() & !K_SIGN); // abs
                                                       // 4-over-4-degree rational polynomial approximation on x**0.25, with two
                                                       // different polynomials above and below 1e-4.
        let xto025 = x.sqrt().sqrt();
        const P: [f32; 5] = [
            1.351392e-02,
            -1.095778e+00,
            5.522776e+01,
            1.492516e+02,
            4.838434e+01,
        ];
        const Q: [f32; 5] = [
            1.012416e+00,
            2.016708e+01,
            9.263710e+01,
            1.120607e+02,
            2.590418e+01,
        ];

        const PLO: [f32; 5] = [
            9.863406e-06,
            3.881234e-01,
            1.352821e+02,
            6.889862e+04,
            -2.864824e+05,
        ];
        const QLO: [f32; 5] = [
            3.371868e+01,
            1.477719e+03,
            1.608477e+04,
            -4.389884e+04,
            -2.072546e+05,
        ];

        // (both are evaluated, as the vector code does)
        let lo = eval_rational_polynomial(xto025, &PLO, &QLO);
        let hi = eval_rational_polynomial(xto025, &P, &Q);
        let magnitude = if x < 1e-4f32 { lo } else { hi };
        f32::from_bits((magnitude.to_bits() & !K_SIGN) | original_sign)
    }
}

/// sRGB. Translation of `TF_SRGB`.
pub(crate) struct TfSrgb;

impl TfSrgb {
    const K_THRESH_SRGB_TO_LINEAR: f32 = 0.04045;
    const K_THRESH_LINEAR_TO_SRGB: f32 = 0.0031308;
    const K_LOW_DIV: f32 = 12.92;
    const K_LOW_DIV_INV: f32 = 1.0 / Self::K_LOW_DIV;

    pub(crate) fn display_from_encoded_v(&self, x: f32) -> f32 {
        let original_sign = x.to_bits() & K_SIGN;
        let x = f32::from_bits(x.to_bits() & !K_SIGN); // abs

        // TODO(janwas): range reduction
        // Computed via af_cheb_rational (k=100); replicated 4x.
        const P: [f32; 5] = [
            2.200248328e-04,
            1.043637593e-02,
            1.624820318e-01,
            7.961564959e-01,
            8.210152774e-01,
        ];
        const Q: [f32; 5] = [
            2.631846970e-01,
            1.076976492e+00,
            4.987528350e-01,
            -5.512498495e-02,
            6.521209011e-03,
        ];
        let linear = x * Self::K_LOW_DIV_INV;
        let poly = eval_rational_polynomial(x, &P, &Q);
        let magnitude = if x > Self::K_THRESH_SRGB_TO_LINEAR {
            poly
        } else {
            linear
        };
        f32::from_bits((magnitude.to_bits() & !K_SIGN) | original_sign)
    }

    // Error ~5e-07
    pub(crate) fn encoded_from_display_v(&self, x: f32) -> f32 {
        let original_sign = x.to_bits() & K_SIGN;
        let x = f32::from_bits(x.to_bits() & !K_SIGN); // abs

        // Computed via af_cheb_rational (k=100); replicated 4x.
        const P: [f32; 5] = [
            -5.135152395e-04,
            5.287254571e-03,
            3.903842876e-01,
            1.474205315e+00,
            7.352629620e-01,
        ];
        const Q: [f32; 5] = [
            1.004519624e-02,
            3.036675394e-01,
            1.340816930e+00,
            9.258482155e-01,
            2.424867759e-02,
        ];
        let linear = x * Self::K_LOW_DIV;
        let poly = eval_rational_polynomial(x.sqrt(), &P, &Q);
        let magnitude = if x > Self::K_THRESH_LINEAR_TO_SRGB {
            poly
        } else {
            linear
        };
        f32::from_bits((magnitude.to_bits() & !K_SIGN) | original_sign)
    }
}

/// Linear to sRGB conversion with error of at most 1.2e-4. Translation of
/// `FastLinearToSRGB()` (its scalar fallback).
#[allow(dead_code)]
pub(crate) fn fast_linear_to_srgb(v: f32) -> f32 {
    // Convert to 0.25 - 0.5 range.
    let v025_05 = f32::from_bits((v.to_bits() | 0x3e800000) & 0x3effffff);
    // third degree polynomial approximation between 0.25 and 0.5
    // of 1.055/2^(7/2.4) * x^(1/2.4) * 0.5. A degree 4 polynomial only improves
    // accuracy by about 3x.
    let d1 = v025_05 * 0.059914046f32 + -0.108894556f32;
    let d2 = d1 * v025_05 + 0.107963754f32;
    let pow = d2 * v025_05 + 0.018092343f32;
    // Compute extra multiplier depending on exponent. Valid exponent range for
    // [0.0031308f, 1.0) is 0...8 after subtracting 118.
    // (See upstream for the representation of the powers.)
    const K2TO512POWERS_BASEBITS: u32 = 0x40000000;
    const K2TO512POWERS_25TO18BITS: [u8; 16] = [
        0x0, 0xa, 0x19, 0x26, 0x32, 0x41, 0x4d, 0x5c, 0x68, 0x75, 0x83, 0x8f, 0xa0, 0xaa, 0xb9,
        0xc6,
    ];
    const K2TO512POWERS_17TO10BITS: [u8; 16] = [
        0x0, 0xb7, 0x4, 0xd, 0xcb, 0xe7, 0x41, 0x68, 0x51, 0xd1, 0xeb, 0xf2, 0x0, 0xb7, 0x4, 0xd,
    ];
    // Fallback for scalar.
    let exp = (((v.to_bits() as i32) >> 23).wrapping_sub(118) as u32 & 0xf) as usize;
    let mul = f32::from_bits(
        ((K2TO512POWERS_25TO18BITS[exp] as u32) << 18)
            | ((K2TO512POWERS_17TO10BITS[exp] as u32) << 10)
            | K2TO512POWERS_BASEBITS,
    );
    if v < 0.0031308f32 {
        v * 12.92f32
    } else {
        pow * mul + -0.055f64 as f32
    }
}
