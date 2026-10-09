// Rust translation of lib/jxl/render_pipeline/stage_from_linear.h and
// lib/jxl/render_pipeline/stage_from_linear.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Converts the color channels from linear to the specified output encoding.

use super::super::dec_xyb::OutputEncodingInfo;
use super::super::math::fast_powf;
use super::super::transfer_functions::{Tf709, TfHlg, TfPq, TfSrgb};
use super::stage_tone_mapping::HlgOotf;
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// The per-pixel operation. Translation of the `Op*` structs (with
/// `PerChannelOp`).
#[derive(Clone, Copy, Debug)]
enum Op {
    Linear,
    // (JXL_HIGH_PRECISION: TF_SRGB().EncodedFromDisplay)
    Rgb,
    Pq,
    Hlg(HlgOotf),
    Bt709,
    Gamma(f32),
}

impl Op {
    #[inline]
    fn transform(&self, r: &mut f32, g: &mut f32, b: &mut f32) {
        match self {
            Op::Linear => {}
            Op::Rgb => {
                *r = TfSrgb.encoded_from_display_v(*r);
                *g = TfSrgb.encoded_from_display_v(*g);
                *b = TfSrgb.encoded_from_display_v(*b);
            }
            Op::Pq => {
                *r = TfPq.encoded_from_display_v(*r);
                *g = TfPq.encoded_from_display_v(*g);
                *b = TfPq.encoded_from_display_v(*b);
            }
            Op::Hlg(hlg_ootf) => {
                hlg_ootf.apply(r, g, b);
                *r = TfHlg.encoded_from_display_v(*r);
                *g = TfHlg.encoded_from_display_v(*g);
                *b = TfHlg.encoded_from_display_v(*b);
            }
            Op::Bt709 => {
                *r = Tf709.encoded_from_display_v(*r);
                *g = Tf709.encoded_from_display_v(*g);
                *b = Tf709.encoded_from_display_v(*b);
            }
            Op::Gamma(inverse_gamma) => {
                let f = |linear: f32| -> f32 {
                    if linear <= 1e-5f32 {
                        0.0
                    } else {
                        fast_powf(linear, *inverse_gamma)
                    }
                };
                *r = f(*r);
                *g = f(*g);
                *b = f(*b);
            }
        }
    }
}

/// Translation of `FromLinearStage`.
struct FromLinearStage {
    op: Op,
}

impl RenderPipelineStage for FromLinearStage {
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
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let p0 = rows.get_input_row(0, 0);
        let p1 = rows.get_input_row(1, 0);
        let p2 = rows.get_input_row(2, 0);
        let mut row0 = rows.load(p0, x0, n);
        let mut row1 = rows.load(p1, x0, n);
        let mut row2 = rows.load(p2, x0, n);
        for x in 0..n {
            let (mut r, mut g, mut b) = (row0[x], row1[x], row2[x]);
            self.op.transform(&mut r, &mut g, &mut b);
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
        "FromLinear"
    }
}

/// Translation of `GetFromLinearStage()`.
pub(crate) fn get_from_linear_stage(
    output_encoding_info: &OutputEncodingInfo,
) -> Result<Box<dyn RenderPipelineStage>, super::super::base::StatusCode> {
    let tf = &output_encoding_info.color_encoding.tf;
    let op = if tf.is_linear() {
        Op::Linear
    } else if tf.is_srgb() {
        Op::Rgb
    } else if tf.is_pq() {
        Op::Pq
    } else if tf.is_hlg() {
        Op::Hlg(HlgOotf::to_scene_light(
            /*display_luminance=*/ output_encoding_info.desired_intensity_target,
            &output_encoding_info.luminances,
        ))
    } else if tf.is_709() {
        Op::Bt709
    } else if tf.is_gamma() || tf.is_dci() {
        Op::Gamma(output_encoding_info.inverse_gamma)
    } else {
        // This is a programming error.
        // JXL_ABORT("Invalid target encoding");
        return Err(super::super::base::StatusCode::GenericError);
    };
    Ok(Box::new(FromLinearStage { op }))
}
