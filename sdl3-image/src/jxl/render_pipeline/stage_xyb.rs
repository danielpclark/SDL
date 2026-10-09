// Rust translation of lib/jxl/render_pipeline/stage_xyb.h and
// lib/jxl/render_pipeline/stage_xyb.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Converts the color channels from XYB to linear with appropriate
//! primaries. (The fast XYB-to-sRGB8 stage is not translated: it needs
//! `HasFastXYBTosRGB8()`, which is false on Highway's scalar target.)

use super::super::dec_xyb::{xyb_to_rgb, OpsinParams};
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `XYBStage`.
struct XybStage {
    opsin_params: OpsinParams,
}

impl RenderPipelineStage for XybStage {
    fn settings(&self) -> Settings {
        Settings::default()
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
        debug_assert!(xextra == 0);
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let p0 = rows.get_input_row(0, 0);
        let p1 = rows.get_input_row(1, 0);
        let p2 = rows.get_input_row(2, 0);
        let mut row0 = rows.load(p0, x0, n);
        let mut row1 = rows.load(p1, x0, n);
        let mut row2 = rows.load(p2, x0, n);
        // TODO(eustas): when using frame origin, addresses might be unaligned;
        //               making them aligned will void performance penalty.
        for x in 0..n {
            let (r, g, b) = xyb_to_rgb(row0[x], row1[x], row2[x], &self.opsin_params);
            row0[x] = r;
            row1[x] = g;
            row2[x] = b;
        }
        rows.store(p0, x0, &row0);
        rows.store(p1, x0, &row1);
        rows.store(p2, x0, &row2);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InPlace
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "XYB"
    }
}

/// Translation of `GetXYBStage()`.
pub(crate) fn get_xyb_stage(opsin_params: &OpsinParams) -> Box<dyn RenderPipelineStage> {
    Box::new(XybStage {
        opsin_params: opsin_params.clone(),
    })
}
