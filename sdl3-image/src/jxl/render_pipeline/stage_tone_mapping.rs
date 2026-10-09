// Rust translation of lib/jxl/render_pipeline/stage_tone_mapping.h,
// lib/jxl/render_pipeline/stage_tone_mapping.cc and the HLG OOTF of
// lib/jxl/dec_tone_mapping-inl.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Tone mapping. SDL_image never sets a desired intensity target, so
//! `desired_intensity_target == orig_intensity_target` always holds, no
//! tone mapping is requested and `GetToneMappingStage()` returns no stage;
//! the Rec. 2408 tone mapper and gamut mapping are therefore not
//! translated. The HLG OOTF is, as the from/to-linear stages use it.

use super::super::dec_xyb::OutputEncodingInfo;
use super::super::math::{fast_powf, hwy_min, log2, powf};
use super::RenderPipelineStage;

/// Translation of `HlgOOTF`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HlgOotf {
    exponent: f32,
    apply_ootf: bool,
    red_y: f32,
    green_y: f32,
    blue_y: f32,
}

/// `std::log2(float)`.
fn log2f(x: f32) -> f32 {
    // FIXME: upstream calls glibc's log2f/powf here (HLG images only).
    log2(x as f64) as f32
}

impl HlgOotf {
    /// Translation of `HlgOOTF(source_luminance, target_luminance, ...)`.
    #[allow(dead_code)]
    pub(crate) fn new(
        source_luminance: f32,
        target_luminance: f32,
        primaries_luminances: &[f32; 3],
    ) -> Self {
        Self::with_gamma(
            /*gamma=*/ powf(1.111f32, log2f(target_luminance / source_luminance)),
            primaries_luminances,
        )
    }

    /// Translation of `HlgOOTF::FromSceneLight()`.
    pub(crate) fn from_scene_light(
        display_luminance: f32,
        primaries_luminances: &[f32; 3],
    ) -> Self {
        Self::with_gamma(
            /*gamma=*/ 1.2f32 * powf(1.111f32, log2f(display_luminance / 1000.0f32)),
            primaries_luminances,
        )
    }

    /// Translation of `HlgOOTF::ToSceneLight()`.
    pub(crate) fn to_scene_light(display_luminance: f32, primaries_luminances: &[f32; 3]) -> Self {
        Self::with_gamma(
            /*gamma=*/
            (1.0 / 1.2f32) * powf(1.111f32, -log2f(display_luminance / 1000.0f32)),
            primaries_luminances,
        )
    }

    fn with_gamma(gamma: f32, luminances: &[f32; 3]) -> Self {
        let exponent = gamma - 1.0;
        HlgOotf {
            exponent,
            apply_ootf: exponent < -0.01f32 || 0.01f32 < exponent,
            red_y: luminances[0],
            green_y: luminances[1],
            blue_y: luminances[2],
        }
    }

    /// Translation of `HlgOOTF::Apply()`.
    pub(crate) fn apply(&self, red: &mut f32, green: &mut f32, blue: &mut f32) {
        if !self.apply_ootf {
            return;
        }
        let luminance = self.red_y * *red + (self.green_y * *green + self.blue_y * *blue);
        let ratio = hwy_min(fast_powf(luminance, self.exponent), 1e9);
        *red *= ratio;
        *green *= ratio;
        *blue *= ratio;
    }

    /// Translation of `HlgOOTF::WarrantsGamutMapping()`.
    #[allow(dead_code)]
    pub(crate) fn warrants_gamut_mapping(&self) -> bool {
        self.apply_ootf && self.exponent < 0.0
    }
}

/// Translation of `GetToneMappingStage()`: no tone mapping is requested
/// when the desired intensity target is the original one.
pub(crate) fn get_tone_mapping_stage(
    output_encoding_info: &OutputEncodingInfo,
) -> Result<Option<Box<dyn RenderPipelineStage>>, super::super::base::StatusCode> {
    if output_encoding_info.desired_intensity_target == output_encoding_info.orig_intensity_target {
        // No tone mapping requested.
        return Ok(None);
    }
    // Unreachable through SDL_image (see the module comment).
    Err(super::super::base::StatusCode::GenericError)
}
