// Rust translation of lib/jxl/loop_filter.h and lib/jxl/loop_filter.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The loop filter parameters (gaborish and the edge-preserving filter).

use super::base::{jxl_failure, Status};
use super::fields::{bundle_init, Fields, Visitor};

pub(crate) const K_EPF_SHARP_ENTRIES: usize = 8;

/// Translation of `LoopFilter`.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoopFilter {
    pub all_default: bool,

    // --- Gaborish convolution
    pub gab: bool,

    pub gab_custom: bool,
    pub gab_x_weight1: f32,
    pub gab_x_weight2: f32,
    pub gab_y_weight1: f32,
    pub gab_y_weight2: f32,
    pub gab_b_weight1: f32,
    pub gab_b_weight2: f32,

    // --- Edge-preserving filter

    // Number of EPF stages to apply. 0 means EPF disabled. 1 applies only the
    // first stage, 2 applies both stages and 3 applies the first stage twice and
    // the second stage once.
    pub epf_iters: u32,

    pub epf_sharp_custom: bool,
    pub epf_sharp_lut: [f32; K_EPF_SHARP_ENTRIES],

    pub epf_weight_custom: bool,     // Custom weight params
    pub epf_channel_scale: [f32; 3], // Relative weight of each channel
    pub epf_pass1_zeroflush: f32,    // Minimum weight for first pass
    pub epf_pass2_zeroflush: f32,    // Minimum weight for second pass

    pub epf_sigma_custom: bool,     // Custom sigma parameters
    pub epf_quant_mul: f32,         // Sigma is ~ this * quant
    pub epf_pass0_sigma_scale: f32, // Multiplier for sigma in pass 0
    pub epf_pass2_sigma_scale: f32, // Multiplier for sigma in the second pass
    pub epf_border_sad_mul: f32,    // (inverse) multiplier for sigma on borders

    pub epf_sigma_for_modular: f32,

    pub extensions: u64,

    pub nonserialized_is_modular: bool,
}

impl LoopFilter {
    pub(crate) fn new() -> Self {
        let mut s = LoopFilter::default();
        bundle_init(&mut s);
        s
    }

    /// Translation of `Padding()`.
    pub(crate) fn padding(&self) -> usize {
        const PADDING_PER_EPF_ITER: [usize; 4] = [0, 2, 3, 6];
        PADDING_PER_EPF_ITER[self.epf_iters as usize] + if self.gab { 1 } else { 0 }
    }
}

impl Fields for LoopFilter {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        // Must come before AllDefault.

        if visitor.all_default(&mut self.all_default) {
            // Overwrite all serialized fields, but not any nonserialized_*.
            visitor.set_default(self);
            return Ok(());
        }

        let near_zero = |w1: f32, w2: f32| ((1.0f32 + (w1 + w2) * 4.0).abs() as f64) < 1e-8;

        visitor.bool_(true, &mut self.gab)?;
        if visitor.conditional(self.gab) {
            visitor.bool_(false, &mut self.gab_custom)?;
            if visitor.conditional(self.gab_custom) {
                visitor.f16(
                    (1.1 * 0.104699568f32 as f64) as f32,
                    &mut self.gab_x_weight1,
                )?;
                visitor.f16(
                    (1.1 * 0.055680538f32 as f64) as f32,
                    &mut self.gab_x_weight2,
                )?;
                if near_zero(self.gab_x_weight1, self.gab_x_weight2) {
                    return jxl_failure!("Gaborish x weights lead to near 0 unnormalized kernel");
                }
                visitor.f16(
                    (1.1 * 0.104699568f32 as f64) as f32,
                    &mut self.gab_y_weight1,
                )?;
                visitor.f16(
                    (1.1 * 0.055680538f32 as f64) as f32,
                    &mut self.gab_y_weight2,
                )?;
                if near_zero(self.gab_y_weight1, self.gab_y_weight2) {
                    return jxl_failure!("Gaborish y weights lead to near 0 unnormalized kernel");
                }
                visitor.f16(
                    (1.1 * 0.104699568f32 as f64) as f32,
                    &mut self.gab_b_weight1,
                )?;
                visitor.f16(
                    (1.1 * 0.055680538f32 as f64) as f32,
                    &mut self.gab_b_weight2,
                )?;
                if near_zero(self.gab_b_weight1, self.gab_b_weight2) {
                    return jxl_failure!("Gaborish b weights lead to near 0 unnormalized kernel");
                }
            }
        }

        visitor.bits(2, 2, &mut self.epf_iters)?;
        if visitor.conditional(self.epf_iters > 0) {
            if visitor.conditional(!self.nonserialized_is_modular) {
                visitor.bool_(false, &mut self.epf_sharp_custom)?;
                if visitor.conditional(self.epf_sharp_custom) {
                    for i in 0..K_EPF_SHARP_ENTRIES {
                        visitor.f16(
                            i as f32 / (K_EPF_SHARP_ENTRIES - 1) as f32,
                            &mut self.epf_sharp_lut[i],
                        )?;
                    }
                }
            }

            visitor.bool_(false, &mut self.epf_weight_custom)?;
            if visitor.conditional(self.epf_weight_custom) {
                visitor.f16(40.0, &mut self.epf_channel_scale[0])?;
                visitor.f16(5.0, &mut self.epf_channel_scale[1])?;
                visitor.f16(3.5, &mut self.epf_channel_scale[2])?;
                visitor.f16(0.45, &mut self.epf_pass1_zeroflush)?;
                visitor.f16(0.6, &mut self.epf_pass2_zeroflush)?;
            }

            visitor.bool_(false, &mut self.epf_sigma_custom)?;
            if visitor.conditional(self.epf_sigma_custom) {
                if visitor.conditional(!self.nonserialized_is_modular) {
                    visitor.f16(0.46, &mut self.epf_quant_mul)?;
                }
                visitor.f16(0.9, &mut self.epf_pass0_sigma_scale)?;
                visitor.f16(6.5, &mut self.epf_pass2_sigma_scale)?;
                visitor.f16(0.6666666666666666, &mut self.epf_border_sad_mul)?;
            }
            if visitor.conditional(self.nonserialized_is_modular) {
                visitor.f16(1.0, &mut self.epf_sigma_for_modular)?;
                if (self.epf_sigma_for_modular as f64) < 1e-8 {
                    return jxl_failure!("EPF: sigma for modular is too small");
                }
            }
        }

        visitor.begin_extensions(&mut self.extensions)?;
        // Extensions: in chronological order of being added to the format.
        visitor.end_extensions()
    }
}
