// Rust translation of lib/jxl/render_pipeline/stage_noise.h and
// lib/jxl/render_pipeline/stage_noise.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Adds noise to color channels, and convolves the noise channels.

use super::super::dec_noise::NoiseParams;
use super::super::math::{hwy_convert_to_i32, hwy_floor, hwy_max, hwy_min};
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// `ZeroIfNegative()`.
#[inline]
fn zero_if_negative(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else {
        v
    }
}

// [0, max_value]
/// Translation of `Clamp0ToMax()`.
#[inline]
fn clamp0_to_max(x: f32, max_value: f32) -> f32 {
    let clamped = hwy_min(x, max_value);
    zero_if_negative(clamped)
}

/// Translation of `StrengthEvalLut` (scalar target).
struct StrengthEvalLut<'a> {
    noise_params: &'a NoiseParams,
}

impl StrengthEvalLut<'_> {
    fn eval(&self, vx: f32) -> f32 {
        const K_SCALE: usize = NoiseParams::K_NUM_NOISE_POINTS - 2;
        let scaled_vx = hwy_max(0.0, vx * K_SCALE as f32);
        let mut floor_x = hwy_floor(scaled_vx);
        let mut frac_x = scaled_vx - floor_x;
        if scaled_vx >= K_SCALE as f32 {
            floor_x = (K_SCALE - 1) as f32;
        }
        if scaled_vx >= K_SCALE as f32 {
            frac_x = 1.0;
        }
        let floor_x_int = hwy_convert_to_i32(floor_x) as usize;
        let low = self.noise_params.lut[floor_x_int];
        let hi = self.noise_params.lut[floor_x_int + 1];
        (hi - low) * frac_x + low
    }
}

// x is in [0+delta, 1+delta], delta ~= 0.06
/// Translation of `NoiseStrength()`.
#[inline]
fn noise_strength(eval: &StrengthEvalLut<'_>, x: f32) -> f32 {
    clamp0_to_max(eval.eval(x), 1.0)
}

/// Translation of `AddNoiseToRGB()`.
#[allow(clippy::too_many_arguments)]
#[inline]
fn add_noise_to_rgb(
    rnd_noise_r: f32,
    rnd_noise_g: f32,
    rnd_noise_cor: f32,
    noise_strength_g: f32,
    noise_strength_r: f32,
    ytox: f32,
    ytob: f32,
    out_x: &mut f32,
    out_y: &mut f32,
    out_b: &mut f32,
) {
    let k_rg_corr = 0.9921875f32; // 127/128
    let k_rgn_corr = 0.0078125f32; // 1/128

    let red_noise = noise_strength_r * (k_rgn_corr * rnd_noise_r + k_rg_corr * rnd_noise_cor);
    let green_noise = noise_strength_g * (k_rgn_corr * rnd_noise_g + k_rg_corr * rnd_noise_cor);

    let mut vx = *out_x;
    let mut vy = *out_y;
    let mut vb = *out_b;

    let rg_noise = red_noise + green_noise;
    vx = (ytox * rg_noise + (red_noise - green_noise)) + vx;
    vy += rg_noise;
    vb = ytob * rg_noise + vb;

    *out_x = vx;
    *out_y = vy;
    *out_b = vb;
}

/// Translation of `AddNoiseStage`.
struct AddNoiseStage {
    noise_params: NoiseParams,
    ytox: f32,
    ytob: f32,
    first_c: usize,
}

impl RenderPipelineStage for AddNoiseStage {
    fn settings(&self) -> Settings {
        Settings::symmetric(/*shift=*/ 0, /*border=*/ 0)
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
        if !self.noise_params.has_any() {
            return;
        }
        let noise_model = StrengthEvalLut {
            noise_params: &self.noise_params,
        };
        let half = 0.5f32;

        // With the prior subtract-random Laplacian approximation, rnd_* ranges were
        // about [-1.5, 1.6]; Laplacian3 about doubles this to [-3.6, 3.6], so the
        // normalizer is half of what it was before (0.5).
        let norm_const = 0.22f32;

        let px = rows.get_input_row(0, 0);
        let py = rows.get_input_row(1, 0);
        let pb = rows.get_input_row(2, 0);
        let mut row_x = rows.load(px, 0, xsize);
        let mut row_y = rows.load(py, 0, xsize);
        let mut row_b = rows.load(pb, 0, xsize);
        let row_rnd_r = rows.load(rows.get_input_row(self.first_c, 0), 0, xsize);
        let row_rnd_g = rows.load(rows.get_input_row(self.first_c + 1, 0), 0, xsize);
        let row_rnd_c = rows.load(rows.get_input_row(self.first_c + 2, 0), 0, xsize);
        for x in 0..xsize {
            let vx = row_x[x];
            let vy = row_y[x];
            let in_g = vy - vx;
            let in_r = vy + vx;
            let noise_strength_g = noise_strength(&noise_model, in_g * half);
            let noise_strength_r = noise_strength(&noise_model, in_r * half);
            let addit_rnd_noise_red = row_rnd_r[x] * norm_const;
            let addit_rnd_noise_green = row_rnd_g[x] * norm_const;
            let addit_rnd_noise_correlated = row_rnd_c[x] * norm_const;
            add_noise_to_rgb(
                addit_rnd_noise_red,
                addit_rnd_noise_green,
                addit_rnd_noise_correlated,
                noise_strength_g,
                noise_strength_r,
                self.ytox,
                self.ytob,
                &mut row_x[x],
                &mut row_y[x],
                &mut row_b[x],
            );
        }
        rows.store(px, 0, &row_x);
        rows.store(py, 0, &row_y);
        rows.store(pb, 0, &row_b);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c >= self.first_c {
            RenderPipelineChannelMode::Input
        } else if c < 3 {
            RenderPipelineChannelMode::InPlace
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "AddNoise"
    }
}

/// Translation of `GetAddNoiseStage()` (with the color correlation map's
/// `YtoXRatio(0)` and `YtoBRatio(0)`).
pub(crate) fn get_add_noise_stage(
    noise_params: &NoiseParams,
    ytox: f32,
    ytob: f32,
    noise_c_start: usize,
) -> Box<dyn RenderPipelineStage> {
    Box::new(AddNoiseStage {
        noise_params: *noise_params,
        ytox,
        ytob,
        first_c: noise_c_start,
    })
}

/// Translation of `ConvolveNoiseStage`.
struct ConvolveNoiseStage {
    first_c: usize,
}

impl RenderPipelineStage for ConvolveNoiseStage {
    fn settings(&self) -> Settings {
        Settings::symmetric(/*shift=*/ 0, /*border=*/ 2)
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
        for c in self.first_c..self.first_c + 3 {
            let r: Vec<Vec<f32>> = (0..5)
                .map(|i| rows.load(rows.get_input_row(c, i as isize - 2), x0 - 2, n + 4))
                .collect();
            let mut out = vec![0f32; n];
            for (k, o) in out.iter_mut().enumerate() {
                let j = k + 2;
                let p00 = r[2][j];
                let mut others = 0.0f32;
                // TODO(eustas): sum loaded values to reduce the calculation chain
                for i in 0..5 {
                    let xi = j + i - 2;
                    others += r[0][xi];
                    others += r[1][xi];
                    others += r[3][xi];
                    others += r[4][xi];
                }
                others += r[2][j - 2];
                others += r[2][j - 1];
                others += r[2][j + 1];
                others += r[2][j + 2];
                // 4 * (1 - box kernel)
                *o = others * 0.16f32 + p00 * -3.84f32;
            }
            let po = rows.get_output_row(c, 0);
            rows.store(po, x0, &out);
        }
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c >= self.first_c {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "ConvNoise"
    }
}

/// Translation of `GetConvolveNoiseStage()`.
pub(crate) fn get_convolve_noise_stage(noise_c_start: usize) -> Box<dyn RenderPipelineStage> {
    Box::new(ConvolveNoiseStage { first_c: noise_c_start })
}
