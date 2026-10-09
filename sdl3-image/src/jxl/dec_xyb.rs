// Rust translation of lib/jxl/dec_xyb.h, lib/jxl/dec_xyb.cc and
// lib/jxl/dec_xyb-inl.h from libjxl (https://github.com/libjxl/libjxl, at
// the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! XYB -> linear sRGB. (Only the parts the decoder reaches are translated,
//! for Highway's scalar target: `HasFastXYBTosRGB8()` is false there.)

use super::base::{inv3x3_matrix, mat_mul, Status};
use super::color_encoding_internal::{
    adapt_to_xyz_d50, primaries_to_xyz, primaries_to_xyz_d50, ColorEncoding, Primaries, WhitePoint,
};
use super::image_metadata::CodecMetadata;
use super::math::cbrtf;
use super::opsin_params::{
    get_opsin_absorbance_inverse_matrix, init_simd_inverse_matrix, K_NEG_OPSIN_ABSORBANCE_BIAS_RGB,
};
use super::quantizer::K_DEFAULT_QUANT_BIAS;

/// Parameters for XYB->sRGB conversion. Translation of `OpsinParams`.
#[derive(Clone, Debug)]
pub(crate) struct OpsinParams {
    pub inverse_opsin_matrix: [f32; 9 * 4],
    pub opsin_biases: [f32; 4],
    pub opsin_biases_cbrt: [f32; 4],
    pub quant_biases: [f32; 4],
}

impl Default for OpsinParams {
    fn default() -> Self {
        OpsinParams {
            inverse_opsin_matrix: [0.0; 36],
            opsin_biases: [0.0; 4],
            opsin_biases_cbrt: [0.0; 4],
            quant_biases: [0.0; 4],
        }
    }
}

impl OpsinParams {
    /// Translation of `OpsinParams::Init()`.
    #[allow(dead_code)]
    pub(crate) fn init(&mut self, intensity_target: f32) {
        init_simd_inverse_matrix(
            get_opsin_absorbance_inverse_matrix(),
            &mut self.inverse_opsin_matrix,
            intensity_target,
        );
        self.opsin_biases = K_NEG_OPSIN_ABSORBANCE_BIAS_RGB;
        self.quant_biases = K_DEFAULT_QUANT_BIAS;
        for c in 0..4 {
            self.opsin_biases_cbrt[c] = cbrtf(self.opsin_biases[c]);
        }
    }
}

/// Translation of `OutputEncodingInfo`.
#[derive(Clone, Debug)]
pub(crate) struct OutputEncodingInfo {
    //
    // Fields depending only on image metadata
    //
    pub orig_color_encoding: ColorEncoding,
    // Used for the HLG OOTF and PQ tone mapping.
    pub orig_intensity_target: f32,
    // Opsin inverse matrix taken from the metadata.
    pub orig_inverse_matrix: [f32; 9],
    pub default_transform: bool,
    pub xyb_encoded: bool,
    //
    // Fields depending on output color encoding
    //
    pub color_encoding: ColorEncoding,
    pub color_encoding_is_original: bool,
    // Contains an opsin matrix that converts to the primaries of the output
    // encoding.
    pub opsin_params: OpsinParams,
    pub all_default_opsin: bool,
    // Used for Gamma and DCI transfer functions.
    pub inverse_gamma: f32,
    // Luminances of color_encoding's primaries, used for the HLG inverse OOTF and
    // for PQ tone mapping.
    // Default to sRGB's.
    pub luminances: [f32; 3],
    // Used for the HLG inverse OOTF and PQ tone mapping.
    pub desired_intensity_target: f32,
}

impl Default for OutputEncodingInfo {
    fn default() -> Self {
        OutputEncodingInfo {
            orig_color_encoding: ColorEncoding::new(),
            orig_intensity_target: 0.0,
            orig_inverse_matrix: [0.0; 9],
            default_transform: false,
            xyb_encoded: false,
            color_encoding: ColorEncoding::new(),
            color_encoding_is_original: false,
            opsin_params: OpsinParams::default(),
            all_default_opsin: false,
            inverse_gamma: 0.0,
            luminances: [0.0; 3],
            desired_intensity_target: 0.0,
        }
    }
}

/// Translation of `CanOutputToColorEncoding()`.
fn can_output_to_color_encoding(c_desired: &ColorEncoding) -> bool {
    if !c_desired.have_fields() {
        return false;
    }
    // TODO(veluca): keep in sync with dec_reconstruct.cc
    let tf = &c_desired.tf;
    if !tf.is_pq() && !tf.is_srgb() && !tf.is_gamma() && !tf.is_linear() && !tf.is_hlg() && !tf.is_dci() && !tf.is_709()
    {
        return false;
    }
    if c_desired.is_gray() && c_desired.white_point != WhitePoint::D65 {
        // TODO(veluca): figure out what should happen here.
        return false;
    }
    true
}

impl OutputEncodingInfo {
    /// Translation of `OutputEncodingInfo::SetFromMetadata()`.
    pub(crate) fn set_from_metadata(&mut self, metadata: &CodecMetadata) -> Status {
        self.orig_color_encoding = metadata.m.color_encoding.clone();
        self.orig_intensity_target = metadata.m.intensity_target();
        self.desired_intensity_target = self.orig_intensity_target;
        let im = &metadata.transform_data.opsin_inverse_matrix;
        self.orig_inverse_matrix = im.inverse_matrix;
        self.default_transform = im.all_default;
        self.xyb_encoded = metadata.m.xyb_encoded;
        self.opsin_params.opsin_biases[..3].copy_from_slice(&im.opsin_biases);
        for i in 0..3 {
            self.opsin_params.opsin_biases_cbrt[i] = cbrtf(self.opsin_params.opsin_biases[i]);
        }
        self.opsin_params.opsin_biases[3] = 1.0;
        self.opsin_params.opsin_biases_cbrt[3] = 1.0;
        self.opsin_params.quant_biases = im.quant_biases;
        let orig_ok = can_output_to_color_encoding(&self.orig_color_encoding);
        let orig_grey = self.orig_color_encoding.is_gray();
        let c = if !self.xyb_encoded || orig_ok {
            self.orig_color_encoding.clone()
        } else {
            ColorEncoding::linear_srgb(orig_grey)
        };
        self.set_color_encoding(&c)
    }

    /// Translation of `OutputEncodingInfo::SetColorEncoding()`.
    fn set_color_encoding(&mut self, c_desired: &ColorEncoding) -> Status {
        self.color_encoding = c_desired.clone();
        self.color_encoding_is_original = self.orig_color_encoding.same_color_encoding(c_desired);

        // Compute the opsin inverse matrix and luminances based on primaries and
        // white point.
        let mut inverse_matrix: [f32; 9] = self.orig_inverse_matrix;
        let mut inverse_matrix_is_default = self.default_transform;
        const K_SRGB_LUMINANCES: [f32; 3] = [0.2126, 0.7152, 0.0722];
        self.luminances = K_SRGB_LUMINANCES;
        if (c_desired.primaries != Primaries::Srgb || c_desired.white_point != WhitePoint::D65) && !c_desired.is_gray() {
            let mut srgb_to_xyzd50 = [0f32; 9];
            let srgb = ColorEncoding::srgb(/*is_gray=*/ false);
            let sp = srgb.get_primaries();
            let sw = srgb.get_white_point();
            if primaries_to_xyz_d50(
                sp.r.x as f32,
                sp.r.y as f32,
                sp.g.x as f32,
                sp.g.y as f32,
                sp.b.x as f32,
                sp.b.y as f32,
                sw.x as f32,
                sw.y as f32,
                &mut srgb_to_xyzd50,
            )
            .is_err()
            {
                // JXL_CHECK
                return jxl_check_failed();
            }
            let mut original_to_xyz = [0f32; 9];
            let dp = c_desired.get_primaries();
            let dw = c_desired.get_white_point();
            primaries_to_xyz(
                dp.r.x as f32,
                dp.r.y as f32,
                dp.g.x as f32,
                dp.g.y as f32,
                dp.b.x as f32,
                dp.b.y as f32,
                dw.x as f32,
                dw.y as f32,
                &mut original_to_xyz,
            )?;
            self.luminances.copy_from_slice(&original_to_xyz[3..6]);
            if self.xyb_encoded {
                let mut adapt_to_d50 = [0f32; 9];
                adapt_to_xyz_d50(dw.x as f32, dw.y as f32, &mut adapt_to_d50)?;
                let mut xyzd50_to_original = [0f32; 9];
                mat_mul(&adapt_to_d50, &original_to_xyz, 3, 3, 3, &mut xyzd50_to_original);
                inv3x3_matrix(&mut xyzd50_to_original)?;
                let mut srgb_to_original = [0f32; 9];
                mat_mul(&xyzd50_to_original, &srgb_to_xyzd50, 3, 3, 3, &mut srgb_to_original);
                mat_mul(&srgb_to_original, &self.orig_inverse_matrix, 3, 3, 3, &mut inverse_matrix);
                inverse_matrix_is_default = false;
            }
        }

        if c_desired.is_gray() {
            let tmp_inv_matrix = inverse_matrix;
            let mut srgb_to_luma = [0f32; 9];
            srgb_to_luma[0..3].copy_from_slice(&self.luminances);
            srgb_to_luma[3..6].copy_from_slice(&self.luminances);
            srgb_to_luma[6..9].copy_from_slice(&self.luminances);
            mat_mul(&srgb_to_luma, &tmp_inv_matrix, 3, 3, 3, &mut inverse_matrix);
        }

        // The internal XYB color space uses absolute luminance, so we scale back the
        // opsin inverse matrix to relative luminance where 1.0 corresponds to the
        // original intensity target, or to absolute luminance for PQ, where 1.0
        // corresponds to 10000 nits.
        if self.xyb_encoded {
            let intensity_target = if c_desired.tf.is_pq() {
                10000.0
            } else {
                self.orig_intensity_target
            };
            init_simd_inverse_matrix(&inverse_matrix, &mut self.opsin_params.inverse_opsin_matrix, intensity_target);
            self.all_default_opsin =
                (intensity_target as f64 - 255.0).abs() <= 0.1f32 as f64 && inverse_matrix_is_default;
        }

        // Set the inverse gamma based on color space transfer function.
        self.inverse_gamma = if c_desired.tf.is_gamma() {
            c_desired.tf.get_gamma() as f32
        } else if c_desired.tf.is_dci() {
            1.0f32 / 2.6f32
        } else {
            1.0
        };
        Ok(())
    }
}

/// A failed `JXL_CHECK`, which aborts upstream.
fn jxl_check_failed() -> Status {
    Err(super::base::StatusCode::GenericError)
}

/// Inverts the pixel-wise RGB->XYB conversion in OpsinDynamicsImage() (including
/// the gamma mixing and simple gamma). Avoids clamping to [0, 1] - out of (sRGB)
/// gamut values may be in-gamut after transforming to a wider space.
/// "inverse_matrix" points to 9 broadcasted vectors, which are the 3x3 entries
/// of the (row-major) opsin absorbance matrix inverse. Pre-multiplying its
/// entries by c is equivalent to multiplying linear_* by c afterwards.
/// Translation of `XybToRgb()` (scalar target; `MulAdd` is an unfused
/// multiply-add there).
#[inline]
pub(crate) fn xyb_to_rgb(opsin_x: f32, opsin_y: f32, opsin_b: f32, opsin_params: &OpsinParams) -> (f32, f32, f32) {
    let neg_bias_r = opsin_params.opsin_biases[0];
    let neg_bias_g = opsin_params.opsin_biases[1];
    let neg_bias_b = opsin_params.opsin_biases[2];

    // Color space: XYB -> RGB
    let mut gamma_r = opsin_y + opsin_x;
    let mut gamma_g = opsin_y - opsin_x;
    let mut gamma_b = opsin_b;

    gamma_r -= opsin_params.opsin_biases_cbrt[0];
    gamma_g -= opsin_params.opsin_biases_cbrt[1];
    gamma_b -= opsin_params.opsin_biases_cbrt[2];

    // Undo gamma compression: linear = gamma^3 for efficiency.
    let gamma_r2 = gamma_r * gamma_r;
    let gamma_g2 = gamma_g * gamma_g;
    let gamma_b2 = gamma_b * gamma_b;
    let mixed_r = gamma_r2 * gamma_r + neg_bias_r;
    let mixed_g = gamma_g2 * gamma_g + neg_bias_g;
    let mixed_b = gamma_b2 * gamma_b + neg_bias_b;

    let inverse_matrix = &opsin_params.inverse_opsin_matrix;

    // Unmix (multiply by 3x3 inverse_matrix)
    // TODO(eustas): ref would be more readable than pointer
    let mut linear_r = inverse_matrix[0] * mixed_r;
    let mut linear_g = inverse_matrix[3 * 4] * mixed_r;
    let mut linear_b = inverse_matrix[6 * 4] * mixed_r;
    linear_r = inverse_matrix[4] * mixed_g + linear_r;
    linear_g = inverse_matrix[4 * 4] * mixed_g + linear_g;
    linear_b = inverse_matrix[7 * 4] * mixed_g + linear_b;
    linear_r = inverse_matrix[2 * 4] * mixed_b + linear_r;
    linear_g = inverse_matrix[5 * 4] * mixed_b + linear_g;
    linear_b = inverse_matrix[8 * 4] * mixed_b + linear_b;
    (linear_r, linear_g, linear_b)
}
