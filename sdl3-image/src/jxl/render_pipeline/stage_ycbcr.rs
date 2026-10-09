// Rust translation of lib/jxl/render_pipeline/stage_ycbcr.h and
// lib/jxl/render_pipeline/stage_ycbcr.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Converts YCbCr back to RGB.

use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `kYCbCrStage`.
struct YCbCrStage;

impl RenderPipelineStage for YCbCrStage {
    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        _xextra: usize,
        xsize: usize,
        _xpos: usize,
        _ypos: usize,
        _thread_id: usize,
        _ctx: &mut StageCtx<'_>,
    ) {
        // Full-range BT.601 as defined by JFIF Clause 7:
        // https://www.itu.int/rec/T-REC-T.871-201105-I/en
        let c128 = 128.0f32 / 255.0;
        let crcr = 1.402f32;
        let cgcb = -0.114f32 * 1.772f32 / 0.587f32;
        let cgcr = -0.299f32 * 1.402f32 / 0.587f32;
        let cbcb = 1.772f32;

        let p0 = rows.get_input_row(0, 0);
        let p1 = rows.get_input_row(1, 0);
        let p2 = rows.get_input_row(2, 0);
        let mut row0 = rows.load(p0, 0, xsize);
        let mut row1 = rows.load(p1, 0, xsize);
        let mut row2 = rows.load(p2, 0, xsize);
        // TODO(eustas): when using frame origin, addresses might be unaligned;
        //               making them aligned will void performance penalty.
        for x in 0..xsize {
            let y_vec = row1[x] + c128;
            let cb_vec = row0[x];
            let cr_vec = row2[x];
            let r_vec = crcr * cr_vec + y_vec;
            let g_vec = cgcr * cr_vec + (cgcb * cb_vec + y_vec);
            let b_vec = cbcb * cb_vec + y_vec;
            row0[x] = r_vec;
            row1[x] = g_vec;
            row2[x] = b_vec;
        }
        rows.store(p0, 0, &row0);
        rows.store(p1, 0, &row1);
        rows.store(p2, 0, &row2);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InPlace
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "YCbCr"
    }
}

/// Translation of `GetYCbCrStage()`.
pub(crate) fn get_ycbcr_stage() -> Box<dyn RenderPipelineStage> {
    Box::new(YCbCrStage)
}
