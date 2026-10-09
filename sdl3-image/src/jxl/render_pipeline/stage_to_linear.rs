// Rust translation of lib/jxl/render_pipeline/stage_to_linear.h and
// lib/jxl/render_pipeline/stage_to_linear.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Converts the color channels from the output encoding to linear.

use super::super::base::{Status, StatusCode};
use super::super::dec_xyb::OutputEncodingInfo;
use super::super::math::fast_powf;
use super::super::transfer_functions::{Tf709, TfHlg, TfPq, TfSrgb};
use super::stage_tone_mapping::HlgOotf;
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of the `Op*` structs (with `PerChannelOp`).
#[derive(Clone, Copy, Debug)]
enum Op {
    Linear,
    Rgb,
    Pq,
    Hlg(HlgOotf),
    Bt709,
    Gamma(f32),
    Invalid,
}

impl Op {
    #[inline]
    fn transform(&self, r: &mut f32, g: &mut f32, b: &mut f32) {
        match self {
            Op::Linear | Op::Invalid => {}
            Op::Rgb => {
                *r = TfSrgb.display_from_encoded_v(*r);
                *g = TfSrgb.display_from_encoded_v(*g);
                *b = TfSrgb.display_from_encoded_v(*b);
            }
            Op::Pq => {
                *r = TfPq.display_from_encoded_v(*r);
                *g = TfPq.display_from_encoded_v(*g);
                *b = TfPq.display_from_encoded_v(*b);
            }
            Op::Hlg(hlg_ootf) => {
                for val in [&mut *r, &mut *g, &mut *b] {
                    *val = TfHlg.display_from_encoded(*val as f64) as f32;
                }
                hlg_ootf.apply(r, g, b);
            }
            Op::Bt709 => {
                *r = Tf709.display_from_encoded_v(*r);
                *g = Tf709.display_from_encoded_v(*g);
                *b = Tf709.display_from_encoded_v(*b);
            }
            Op::Gamma(gamma) => {
                let f = |encoded: f32| -> f32 {
                    if encoded <= 1e-5f32 {
                        0.0
                    } else {
                        fast_powf(encoded, *gamma)
                    }
                };
                *r = f(*r);
                *g = f(*g);
                *b = f(*b);
            }
        }
    }
}

/// Translation of `ToLinearStage`.
struct ToLinearStage {
    op: Op,
    valid: bool,
}

impl RenderPipelineStage for ToLinearStage {
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

    fn is_initialized(&self) -> Status {
        if self.valid {
            Ok(())
        } else {
            Err(StatusCode::GenericError)
        }
    }

    fn get_name(&self) -> &'static str {
        "ToLinear"
    }
}

/// Translation of `GetToLinearStage()`.
pub(crate) fn get_to_linear_stage(
    output_encoding_info: &OutputEncodingInfo,
) -> Box<dyn RenderPipelineStage> {
    let tf = &output_encoding_info.color_encoding.tf;
    let (op, valid) = if tf.is_linear() {
        (Op::Linear, true)
    } else if tf.is_srgb() {
        (Op::Rgb, true)
    } else if tf.is_pq() {
        (Op::Pq, true)
    } else if tf.is_hlg() {
        (
            Op::Hlg(HlgOotf::from_scene_light(
                output_encoding_info.orig_intensity_target,
                &output_encoding_info.luminances,
            )),
            true,
        )
    } else if tf.is_709() {
        (Op::Bt709, true)
    } else if tf.is_gamma() || tf.is_dci() {
        (Op::Gamma(1.0f32 / output_encoding_info.inverse_gamma), true)
    } else {
        (Op::Invalid, false)
    };
    Box::new(ToLinearStage { op, valid })
}
