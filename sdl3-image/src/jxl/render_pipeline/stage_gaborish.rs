// Rust translation of lib/jxl/render_pipeline/stage_gaborish.h and
// lib/jxl/render_pipeline/stage_gaborish.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Applies decoder-side Gaborish to the first 3 channels.

use super::super::loop_filter::LoopFilter;
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `GaborishStage`.
struct GaborishStage {
    weights: [f32; 9],
}

impl GaborishStage {
    fn new(lf: &LoopFilter) -> Self {
        let mut weights = [0f32; 9];
        weights[0] = 1.0;
        weights[1] = lf.gab_x_weight1;
        weights[2] = lf.gab_x_weight2;
        weights[3] = 1.0;
        weights[4] = lf.gab_y_weight1;
        weights[5] = lf.gab_y_weight2;
        weights[6] = 1.0;
        weights[7] = lf.gab_b_weight1;
        weights[8] = lf.gab_b_weight2;
        // Normalize
        for c in 0..3 {
            let div = weights[3 * c] + 4.0 * (weights[3 * c + 1] + weights[3 * c + 2]);
            let mul = 1.0f32 / div;
            weights[3 * c] *= mul;
            weights[3 * c + 1] *= mul;
            weights[3 * c + 2] *= mul;
        }
        GaborishStage { weights }
    }
}

impl RenderPipelineStage for GaborishStage {
    fn settings(&self) -> Settings {
        Settings::symmetric(/*shift=*/ 0, /*border=*/ 1)
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        xextra: usize,
        xsize: usize,
        _xpos: usize,
        _ypos: usize,
        _thread_id: usize,
        _ctx: &mut StageCtx<'_>,
    ) {
        for c in 0..3 {
            // Since GetInputRow(input_rows, c, {-1, 0, 1}) is aligned, rounding
            // xextra up to Lanes(d) doesn't access anything problematic.
            let n = xsize + 2 * xextra;
            let x0 = -(xextra as isize);
            let row_t = rows.load(rows.get_input_row(c, -1), x0 - 1, n + 2);
            let row_m = rows.load(rows.get_input_row(c, 0), x0 - 1, n + 2);
            let row_b = rows.load(rows.get_input_row(c, 1), x0 - 1, n + 2);
            let w0 = self.weights[3 * c];
            let w1 = self.weights[3 * c + 1];
            let w2 = self.weights[3 * c + 2];
            let po = rows.get_output_row(c, 0);
            let row_out = rows.row_mut(po, x0, n);
            for i in 0..n {
                let j = i + 1;
                let t = row_t[j];
                let tl = row_t[j - 1];
                let tr = row_t[j + 1];
                let m = row_m[j];
                let l = row_m[j - 1];
                let r = row_m[j + 1];
                let b = row_b[j];
                let bl = row_b[j - 1];
                let br = row_b[j + 1];
                let sum0 = m;
                let sum1 = (l + r) + (t + b);
                let sum2 = (tl + tr) + (bl + br);
                let pixels = sum2 * w2 + (sum1 * w1 + sum0 * w0);
                row_out[i] = pixels;
            }
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "Gab"
    }
}

/// Translation of `GetGaborishStage()`.
pub(crate) fn get_gaborish_stage(lf: &LoopFilter) -> Box<dyn RenderPipelineStage> {
    debug_assert!(lf.gab);
    Box::new(GaborishStage::new(lf))
}
