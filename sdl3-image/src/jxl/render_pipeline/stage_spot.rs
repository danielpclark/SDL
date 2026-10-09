// Rust translation of lib/jxl/render_pipeline/stage_spot.h and
// lib/jxl/render_pipeline/stage_spot.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Renders a spot color channel onto the color channels.

use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `SpotColorStage`.
struct SpotColorStage {
    spot_c: usize,
    spot_color: [f32; 4],
}

impl RenderPipelineStage for SpotColorStage {
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
        // TODO(veluca): add SIMD.
        let scale = self.spot_color[3];
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let s = rows.load(rows.get_input_row(self.spot_c, 0), x0, n);
        for c in 0..3 {
            let pp = rows.get_input_row(c, 0);
            let p = rows.row_mut(pp, x0, n);
            for x in 0..n {
                let mix = scale * s[x];
                p[x] = mix * self.spot_color[c] + (1.0f32 - mix) * p[x];
            }
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InPlace
        } else if c == self.spot_c {
            RenderPipelineChannelMode::Input
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "Spot"
    }
}

/// Translation of `GetSpotColorStage()`.
pub(crate) fn get_spot_color_stage(spot_c: usize, spot_color: [f32; 4]) -> Box<dyn RenderPipelineStage> {
    debug_assert!(spot_c >= 3);
    Box::new(SpotColorStage { spot_c, spot_color })
}
