// Rust translation of lib/jxl/render_pipeline/stage_splines.h and
// lib/jxl/render_pipeline/stage_splines.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Draws splines if applicable. (The splines are read from the image
//! features of the [`StageCtx`].)

use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `SplineStage`.
struct SplineStage;

impl RenderPipelineStage for SplineStage {
    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        _xextra: usize,
        xsize: usize,
        xpos: usize,
        ypos: usize,
        _thread_id: usize,
        ctx: &mut StageCtx<'_>,
    ) {
        let px = rows.get_input_row(0, 0);
        let py = rows.get_input_row(1, 0);
        let pb = rows.get_input_row(2, 0);
        let mut row_x = rows.load(px, 0, xsize);
        let mut row_y = rows.load(py, 0, xsize);
        let mut row_b = rows.load(pb, 0, xsize);
        {
            let mut r: [&mut [f32]; 3] = [&mut row_x, &mut row_y, &mut row_b];
            ctx.image_features.splines.add_to_row(&mut r, xpos, ypos, xsize);
        }
        rows.store(px, 0, &row_x);
        rows.store(py, 0, &row_y);
        rows.store(pb, 0, &row_b);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InPlace
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "Splines"
    }
}

/// Translation of `GetSplineStage()`.
pub(crate) fn get_spline_stage() -> Box<dyn RenderPipelineStage> {
    Box::new(SplineStage)
}
