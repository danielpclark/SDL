// Rust translation of lib/jxl/render_pipeline/stage_chroma_upsampling.h and
// lib/jxl/render_pipeline/stage_chroma_upsampling.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Applies simple upsampling, either horizontal or vertical, to the given
//! channel.

use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `HorizontalChromaUpsamplingStage`.
struct HorizontalChromaUpsamplingStage {
    c: usize,
}

impl RenderPipelineStage for HorizontalChromaUpsamplingStage {
    fn settings(&self) -> Settings {
        Settings::shift_x(/*shift=*/ 1, /*border=*/ 1)
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
        // (xextra = RoundUpTo(xextra, Lanes(df)), with one lane.)
        let threefour = 0.75f32;
        let onefour = 0.25f32;
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        // row_in[x0 - 1 .. x0 + n + 1)
        let row_in = rows.load(rows.get_input_row(self.c, 0), x0 - 1, n + 2);
        let po = rows.get_output_row(self.c, 0);
        let row_out = rows.row_mut(po, 2 * x0, 2 * n);
        for i in 0..n {
            let current = row_in[i + 1] * threefour;
            let prev = row_in[i];
            let next = row_in[i + 2];
            let left = onefour * prev + current;
            let right = onefour * next + current;
            row_out[2 * i] = left;
            row_out[2 * i + 1] = right;
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c == self.c {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "HChromaUps"
    }
}

/// Translation of `VerticalChromaUpsamplingStage`.
struct VerticalChromaUpsamplingStage {
    c: usize,
}

impl RenderPipelineStage for VerticalChromaUpsamplingStage {
    fn settings(&self) -> Settings {
        Settings::shift_y(/*shift=*/ 1, /*border=*/ 1)
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
        let threefour = 0.75f32;
        let onefour = 0.25f32;
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let row_top = rows.load(rows.get_input_row(self.c, -1), x0, n);
        let row_mid = rows.load(rows.get_input_row(self.c, 0), x0, n);
        let row_bot = rows.load(rows.get_input_row(self.c, 1), x0, n);
        let mut out0 = vec![0f32; n];
        let mut out1 = vec![0f32; n];
        for i in 0..n {
            let it = row_top[i];
            let im = row_mid[i];
            let ib = row_bot[i];
            let im_scaled = im * threefour;
            out0[i] = it * onefour + im_scaled;
            out1[i] = ib * onefour + im_scaled;
        }
        let p0 = rows.get_output_row(self.c, 0);
        let p1 = rows.get_output_row(self.c, 1);
        rows.store(p0, x0, &out0);
        rows.store(p1, x0, &out1);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c == self.c {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "VChromaUps"
    }
}

/// Translation of `GetChromaUpsamplingStage()`.
pub(crate) fn get_chroma_upsampling_stage(channel: usize, horizontal: bool) -> Box<dyn RenderPipelineStage> {
    if horizontal {
        Box::new(HorizontalChromaUpsamplingStage { c: channel })
    } else {
        Box::new(VerticalChromaUpsamplingStage { c: channel })
    }
}
