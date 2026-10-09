// Rust translation of lib/jxl/render_pipeline/stage_patches.h and
// lib/jxl/render_pipeline/stage_patches.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Draws patches in the given channels. (The dictionary is read from the
//! image features of the [`StageCtx`].)

use super::super::image_metadata::ExtraChannelInfo;
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `PatchDictionaryStage`.
struct PatchDictionaryStage {
    num_channels: usize,
    num_extra_channels: usize,
    extra_channel_info: Vec<ExtraChannelInfo>,
}

impl RenderPipelineStage for PatchDictionaryStage {
    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn process_row(
        &self,
        rows: &mut StageRows<'_>,
        xextra: usize,
        xsize: usize,
        xpos: usize,
        ypos: usize,
        _thread_id: usize,
        ctx: &mut StageCtx<'_>,
    ) {
        debug_assert!(xpos == 0 || xpos >= xextra);
        let x0 = if xpos != 0 { xpos - xextra } else { 0 };
        let len = xsize + xextra + xpos - x0;
        let start = x0 as isize - xpos as isize;
        let ptrs: Vec<_> = (0..self.num_channels).map(|i| rows.get_input_row(i, 0)).collect();
        let mut row_data: Vec<Vec<f32>> = ptrs.iter().map(|&p| rows.load(p, start, len)).collect();
        ctx.image_features.patches.add_one_row(
            &mut row_data,
            ypos,
            x0,
            len,
            self.num_extra_channels,
            &self.extra_channel_info,
            ctx.reference_frames,
        );
        for (i, &p) in ptrs.iter().enumerate() {
            rows.store(p, start, &row_data[i]);
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < self.num_channels {
            RenderPipelineChannelMode::InPlace
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "Patches"
    }
}

/// Translation of `GetPatchesStage()`.
pub(crate) fn get_patches_stage(
    num_channels: usize,
    num_extra_channels: usize,
    extra_channel_info: &[ExtraChannelInfo],
) -> Box<dyn RenderPipelineStage> {
    Box::new(PatchDictionaryStage {
        num_channels,
        num_extra_channels,
        extra_channel_info: extra_channel_info.to_vec(),
    })
}
