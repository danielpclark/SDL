// Rust translation of lib/jxl/opsin_params.h and lib/jxl/opsin_params.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Constants that define the XYB color space.

// Parameters for opsin absorbance.
#[allow(dead_code)]
pub(crate) const K_M02: f32 = 0.078;
#[allow(dead_code)]
pub(crate) const K_M00: f32 = 0.30;

pub(crate) const K_B0: f32 = 0.0037930732552754493;
pub(crate) const K_B1: f32 = K_B0;
pub(crate) const K_B2: f32 = K_B0;

/// Must be the inverse matrix of kOpsinAbsorbanceMatrix and match the spec.
/// Translation of `DefaultInverseOpsinAbsorbanceMatrix()`.
pub(crate) const K_DEFAULT_INVERSE_OPSIN_ABSORBANCE_MATRIX: [f32; 9] = [
    11.031566901960783,
    -9.866943921568629,
    -0.16462299647058826,
    -3.254147380392157,
    4.418770392156863,
    -0.16462299647058826,
    -3.6588512862745097,
    2.7129230470588235,
    1.9459282392156863,
];

/// Returns 3x3 row-major matrix inverse of kOpsinAbsorbanceMatrix.
/// opsin_image_test verifies this is actually the inverse. Translation of
/// `GetOpsinAbsorbanceInverseMatrix()` (`INVERSE_OPSIN_FROM_SPEC`).
#[allow(dead_code)]
pub(crate) fn get_opsin_absorbance_inverse_matrix() -> &'static [f32; 9] {
    &K_DEFAULT_INVERSE_OPSIN_ABSORBANCE_MATRIX
}

/// Translation of `InitSIMDInverseMatrix()`.
pub(crate) fn init_simd_inverse_matrix(
    inverse: &[f32; 9],
    simd_inverse: &mut [f32; 36],
    intensity_target: f32,
) {
    for i in 0..9 {
        let v = inverse[i] * (255.0f32 / intensity_target);
        simd_inverse[4 * i] = v;
        simd_inverse[4 * i + 1] = v;
        simd_inverse[4 * i + 2] = v;
        simd_inverse[4 * i + 3] = v;
    }
}

pub(crate) const K_OPSIN_ABSORBANCE_BIAS: [f32; 3] = [K_B0, K_B1, K_B2];

pub(crate) const K_NEG_OPSIN_ABSORBANCE_BIAS_RGB: [f32; 4] = [
    -K_OPSIN_ABSORBANCE_BIAS[0],
    -K_OPSIN_ABSORBANCE_BIAS[1],
    -K_OPSIN_ABSORBANCE_BIAS[2],
    1.0,
];
