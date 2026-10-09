// Rust translation of lib/jxl/epf.h and lib/jxl/epf.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Edge-preserving smoothing: weighted average based on L1 patch
//! similarity. (The filter itself is the EPF stages of the render pipeline.)

use super::dec_cache::{PassesDecoderState, K_SIGMA_BORDER, K_SIGMA_PADDING};
use super::image::Rect;

// 4 * (sqrt(0.5)-1), so that Weight(sigma) = 0.5.
pub(crate) const K_INV_SIGMA_NUM: f32 = -1.1715728752538099024f32;

// kInvSigmaNum / 0.3
pub(crate) const K_MIN_SIGMA: f32 = -3.90524291751269967465540850526868f32;

/// Fills the `state->filter_weights.sigma` image with the precomputed
/// sigma values in the area inside `block_rect`. Accesses the AC strategy,
/// quant field and epf_sharpness fields in the corresponding positions.
/// Translation of `ComputeSigma()`.
pub(crate) fn compute_sigma(block_rect: &Rect, state: &mut PassesDecoderState) {
    let shared = &state.shared_storage;
    let lf = &shared.frame_header.loop_filter;
    debug_assert!(lf.epf_iters > 0);
    let ac_strategy = &shared.ac_strategy;
    let quant_scale = shared.quantizer.scale();

    let sigma_stride = state.sigma.pixels_per_row();
    let sharpness_stride = shared.epf_sharpness.pixels_per_row();
    let xsize_blocks = shared.frame_dim.xsize_blocks;
    let ysize_blocks = shared.frame_dim.ysize_blocks;

    for by in 0..block_rect.ysize() {
        let sigma_base = block_rect.row_index(&state.sigma, by);
        let sharpness_base = block_rect.row_index(&shared.epf_sharpness, by);
        let acs_row = ac_strategy.const_row_rect(block_rect, by);
        let row_quant = block_rect.const_row(&shared.raw_quant_field, by);
        let sharpness = shared.epf_sharpness.data();
        let sigma = state.sigma.data_mut();

        for bx in 0..block_rect.xsize() {
            let acs = acs_row.get(bx);
            let llf_x = acs.covered_blocks_x();
            if !acs.is_first_block() {
                continue;
            }
            // quant_scale is smaller for low quality.
            // quant_scale is roughly 0.08 / butteraugli score.
            //
            // row_quant is smaller for low quality.
            // row_quant is a quantization multiplier of form 1.0 /
            // row_quant[bx]
            //
            // lf.epf_quant_mul is a parameter in the format
            // kInvSigmaNum is a constant
            let sigma_quant =
                lf.epf_quant_mul / (quant_scale * row_quant[bx] as f32 * K_INV_SIGMA_NUM);
            for iy in 0..acs.covered_blocks_y() {
                for ix in 0..acs.covered_blocks_x() {
                    let mut s = sigma_quant
                        * lf.epf_sharp_lut
                            [sharpness[sharpness_base + bx + ix + iy * sharpness_stride] as usize];
                    // Avoid infinities.
                    s = if s < -1e-4f32 { s } else { -1e-4f32 }; // TODO(veluca): remove this.
                    sigma[sigma_base
                        + bx
                        + ix
                        + K_SIGMA_PADDING
                        + (iy + K_SIGMA_PADDING) * sigma_stride] = 1.0f32 / s;
                }
            }
            // TODO(veluca): remove this padding.
            // Left padding with mirroring.
            if bx + block_rect.x0() == 0 {
                for iy in 0..acs.covered_blocks_y() {
                    // LeftMirror(p, kSigmaBorder)
                    let p = sigma_base + K_SIGMA_PADDING + (iy + K_SIGMA_PADDING) * sigma_stride;
                    for i in 0..K_SIGMA_BORDER {
                        sigma[p - 1 - i] = sigma[p + i];
                    }
                }
            }
            // Right padding with mirroring.
            if bx + block_rect.x0() + llf_x == xsize_blocks {
                for iy in 0..acs.covered_blocks_y() {
                    // RightMirror(p, kSigmaBorder)
                    let p = sigma_base
                        + K_SIGMA_PADDING
                        + bx
                        + llf_x
                        + (iy + K_SIGMA_PADDING) * sigma_stride;
                    for i in 0..K_SIGMA_BORDER {
                        sigma[p + i] = sigma[p - 1 - i];
                    }
                }
            }
            // Offsets for row copying, in blocks.
            let offset_before = if bx + block_rect.x0() == 0 {
                1
            } else {
                bx + K_SIGMA_PADDING
            };
            let offset_after = if bx + block_rect.x0() + llf_x == xsize_blocks {
                K_SIGMA_PADDING + llf_x + bx + K_SIGMA_BORDER
            } else {
                K_SIGMA_PADDING + llf_x + bx
            };
            let num = offset_after - offset_before;
            // Above
            if by + block_rect.y0() == 0 {
                for iy in 0..K_SIGMA_BORDER {
                    let dst =
                        sigma_base + offset_before + (K_SIGMA_PADDING - 1 - iy) * sigma_stride;
                    let src = sigma_base + offset_before + (K_SIGMA_PADDING + iy) * sigma_stride;
                    sigma.copy_within(src..src + num, dst);
                }
            }
            // Below
            if by + block_rect.y0() + acs.covered_blocks_y() == ysize_blocks {
                for iy in 0..K_SIGMA_BORDER {
                    let dst = sigma_base
                        + offset_before
                        + sigma_stride * (acs.covered_blocks_y() + K_SIGMA_PADDING + iy);
                    let src = sigma_base
                        + offset_before
                        + sigma_stride * (acs.covered_blocks_y() + K_SIGMA_PADDING - 1 - iy);
                    sigma.copy_within(src..src + num, dst);
                }
            }
        }
    }
}
