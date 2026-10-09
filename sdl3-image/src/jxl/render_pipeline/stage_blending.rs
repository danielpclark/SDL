// Rust translation of lib/jxl/render_pipeline/stage_blending.h and
// lib/jxl/render_pipeline/stage_blending.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Applies blending if applicable. (The background frames are read from
//! the reference frames of the [`StageCtx`] when rendering.)

use super::super::base::{Status, StatusCode};
use super::super::blending::perform_blending;
use super::super::dec_patch_dictionary::{PatchBlendMode, PatchBlending};
use super::super::frame_header::{BlendMode, BlendingInfo, FrameHeader, FrameOrigin};
use super::super::image_metadata::ExtraChannelInfo;
use super::super::passes_state::ReferenceFrame;
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `BlendingStage`.
struct BlendingStage {
    frame_header: FrameHeader,
    info: BlendingInfo,
    initialized: Status,
    image_xsize: usize,
    image_ysize: usize,
    blending_info: Vec<PatchBlending>,
    extra_channel_info: Vec<ExtraChannelInfo>,
    zeroes: Vec<f32>,
}

impl BlendingStage {
    fn new(
        frame_header: &FrameHeader,
        reference_frames: &[ReferenceFrame; 4],
        xyb_encoded: bool,
        color_encoding_is_original: bool,
    ) -> BlendingStage {
        let md = frame_header.nonserialized_metadata.as_ref();
        let image_xsize = md.map_or(0, |m| m.xsize());
        let image_ysize = md.map_or(0, |m| m.ysize());
        let mut s = BlendingStage {
            frame_header: frame_header.clone(),
            info: frame_header.blending_info.clone(),
            initialized: Ok(()),
            image_xsize,
            image_ysize,
            blending_info: Vec::new(),
            extra_channel_info: md.map_or(Vec::new(), |m| m.m.extra_channel_info.clone()),
            zeroes: Vec::new(),
        };
        let ec_info = &frame_header.extra_channel_blending_info;
        let bg_ref = &reference_frames[s.info.source as usize];
        let bg = &bg_ref.frame;
        if bg.xsize() == 0 || bg.ysize() == 0 {
            s.zeroes.resize(image_xsize, 0.0);
        } else if bg_ref.ib_is_in_xyb {
            // "Trying to blend XYB reference frame %i and non-XYB frame"
            s.initialized = Err(StatusCode::GenericError);
            return s;
        } else if ec_info.iter().any(|info| {
            let bg = &reference_frames[info.source as usize].frame;
            bg.xsize() == 0 || bg.ysize() == 0
        }) {
            s.zeroes.resize(image_xsize, 0.0);
        }

        if bg.xsize() != 0
            && bg.ysize() != 0
            && (bg.xsize() < image_xsize
                || bg.ysize() < image_ysize
                || bg.origin.x0 != 0
                || bg.origin.y0 != 0)
        {
            // "Trying to use a %" PRIuS "x%" PRIuS " crop as a background"
            s.initialized = Err(StatusCode::GenericError);
            return s;
        }
        if xyb_encoded && !color_encoding_is_original {
            // "Blending in unsupported color space"
            s.initialized = Err(StatusCode::GenericError);
            return s;
        }

        let make_blending = |info: &BlendingInfo| -> Option<PatchBlending> {
            let mode = match info.mode {
                BlendMode::Replace => PatchBlendMode::Replace,
                BlendMode::Add => PatchBlendMode::Add,
                BlendMode::Mul => PatchBlendMode::Mul,
                BlendMode::Blend => PatchBlendMode::BlendAbove,
                BlendMode::AlphaWeightedAdd => PatchBlendMode::AlphaWeightedAddAbove,
                #[allow(unreachable_patterns)]
                _ => return None, // JXL_ABORT("Invalid blend mode"); should have failed to decode
            };
            Some(PatchBlending {
                mode,
                alpha_channel: info.alpha_channel,
                clamp: info.clamp,
            })
        };
        let mut v = Vec::with_capacity(ec_info.len() + 1);
        for info in std::iter::once(&s.info).chain(ec_info.iter()) {
            match make_blending(info) {
                Some(b) => v.push(b),
                None => {
                    s.initialized = Err(StatusCode::GenericError);
                    return s;
                }
            }
        }
        s.blending_info = v;
        s
    }
}

impl RenderPipelineStage for BlendingStage {
    fn settings(&self) -> Settings {
        Settings::default()
    }

    fn is_initialized(&self) -> Status {
        self.initialized
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
        if self.initialized.is_err() {
            return;
        }
        let mut xsize = xsize;
        let frame_origin = self.frame_header.frame_origin;
        let mut bg_xpos: i64 = frame_origin.x0 as i64 + xpos as i64;
        let bg_ypos: i64 = frame_origin.y0 as i64 + ypos as i64;
        let mut offset: i64 = 0;
        if bg_xpos + xsize as i64 <= 0
            || frame_origin.x0 as i64 >= self.image_xsize as i64
            || bg_ypos < 0
            || bg_ypos >= self.image_ysize as i64
        {
            return;
        }
        if bg_xpos < 0 {
            offset -= bg_xpos;
            xsize = (xsize as i64 + bg_xpos) as usize;
            bg_xpos = 0;
        }
        if bg_xpos as usize + xsize > self.image_xsize {
            xsize = 0i64.max(self.image_xsize as i64 - bg_xpos) as usize;
        }
        let num_c = rows.input_rows.len().min(self.extra_channel_info.len() + 3);
        let bg_ypos = bg_ypos as usize;
        let bg_xpos = bg_xpos as usize;
        let mut fg_ptrs = Vec::with_capacity(num_c);
        let mut fg_rows: Vec<Vec<f32>> = Vec::with_capacity(num_c);
        for c in 0..num_c {
            let p = rows.get_input_row(c, 0).add(offset as isize);
            fg_ptrs.push(p);
            fg_rows.push(rows.load(p, 0, xsize));
        }
        let mut bg_rows: Vec<&[f32]> = Vec::with_capacity(num_c);
        for c in 0..num_c {
            if c < 3 {
                let bg = &ctx.reference_frames[self.info.source as usize].frame;
                bg_rows.push(if bg.xsize() != 0 && bg.ysize() != 0 {
                    &bg.color().const_plane_row(c, bg_ypos)[bg_xpos..]
                } else {
                    &self.zeroes
                });
            } else {
                let ec_bg = &ctx.reference_frames
                    [self.frame_header.extra_channel_blending_info[c - 3].source as usize]
                    .frame;
                bg_rows.push(if ec_bg.xsize() != 0 && ec_bg.ysize() != 0 {
                    &ec_bg.extra_channels()[c - 3].row(bg_ypos)[bg_xpos..]
                } else {
                    &self.zeroes
                });
            }
        }
        let fg: Vec<&[f32]> = fg_rows.iter().map(|r| r.as_slice()).collect();
        let out = perform_blending(
            &bg_rows,
            &fg,
            xsize,
            &self.blending_info[0],
            &self.blending_info[1..],
            &self.extra_channel_info,
        );
        for (c, o) in out.iter().enumerate().take(num_c) {
            rows.store(fg_ptrs[c], 0, o);
        }
    }

    fn get_channel_mode(&self, _c: usize) -> RenderPipelineChannelMode {
        RenderPipelineChannelMode::InPlace
    }

    fn switch_to_image_dimensions(&self) -> bool {
        true
    }

    fn get_image_dimensions(&self) -> (usize, usize, FrameOrigin) {
        (
            self.image_xsize,
            self.image_ysize,
            self.frame_header.frame_origin,
        )
    }

    fn process_padding_row(
        &self,
        rows: &mut StageRows<'_>,
        xsize: usize,
        xpos: usize,
        ypos: usize,
        ctx: &mut StageCtx<'_>,
    ) {
        let bg = &ctx.reference_frames[self.info.source as usize].frame;
        if bg.xsize() == 0 || bg.ysize() == 0 {
            for c in 0..3 {
                let p = rows.get_input_row(c, 0);
                rows.row_mut(p, 0, xsize).fill(0.0);
            }
        } else {
            for c in 0..3 {
                let p = rows.get_input_row(c, 0);
                let src = &bg.color().const_plane_row(c, ypos)[xpos..xpos + xsize];
                rows.store(p, 0, src);
            }
        }
        for ec in 0..self.extra_channel_info.len() {
            let ec_bg = &ctx.reference_frames
                [self.frame_header.extra_channel_blending_info[ec].source as usize]
                .frame;
            let p = rows.get_input_row(3 + ec, 0);
            if ec_bg.xsize() == 0 || ec_bg.ysize() == 0 {
                rows.row_mut(p, 0, xsize).fill(0.0);
            } else {
                let src = &ec_bg.extra_channels()[ec].row(ypos)[xpos..xpos + xsize];
                rows.store(p, 0, src);
            }
        }
    }

    fn get_name(&self) -> &'static str {
        "Blending"
    }
}

/// Translation of `GetBlendingStage()`.
pub(crate) fn get_blending_stage(
    frame_header: &FrameHeader,
    reference_frames: &[ReferenceFrame; 4],
    xyb_encoded: bool,
    color_encoding_is_original: bool,
) -> Box<dyn RenderPipelineStage> {
    Box::new(BlendingStage::new(
        frame_header,
        reference_frames,
        xyb_encoded,
        color_encoding_is_original,
    ))
}

