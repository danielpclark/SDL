// Rust translation of lib/jxl/render_pipeline/stage_epf.h and
// lib/jxl/render_pipeline/stage_epf.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Applies the `epf_stage`-th EPF step with the given settings and `sigma`.
//! `sigma` will be accessed with an offset of (kSigmaPadding,
//! kSigmaPadding), and should have (kSigmaBorder, kSigmaBorder) mirrored
//! sigma values available around the main image. See also
//! [`compute_sigma`](super::super::epf::compute_sigma).

use super::super::base::{StatusCode, K_BLOCK_DIM};
use super::super::dec_cache::K_SIGMA_PADDING;
use super::super::epf::K_MIN_SIGMA;
use super::super::loop_filter::LoopFilter;
use super::{RenderPipelineChannelMode, RenderPipelineStage, Settings, StageCtx, StageRows};

/// Translation of `Weight()`.
#[inline]
fn weight(sad: f32, inv_sigma: f32) -> f32 {
    let v = sad * inv_sigma + 1.0f32;
    // ZeroIfNegative
    if v < 0.0 {
        0.0
    } else {
        v
    }
}

/// `AbsDiff()`.
#[inline]
fn abs_diff(a: f32, b: f32) -> f32 {
    (a - b).abs()
}

/// The input rows of the 3 channels around the current row: `get(c, dy, x)`
/// is the pixel at offset `x` of row `dy` (in `[-border, border]`).
struct Window {
    rows: Vec<Vec<Vec<f32>>>,
    border: isize,
    x0: isize,
}

impl Window {
    fn new(rows: &StageRows<'_>, border: isize, xextra: usize, xsize: usize) -> Window {
        let x0 = -(xextra as isize) - border;
        let n = xsize + 2 * xextra + 2 * border as usize;
        let rows = (0..3)
            .map(|c| (-border..=border).map(|dy| rows.load(rows.get_input_row(c, dy), x0, n)).collect())
            .collect();
        Window { rows, border, x0 }
    }

    #[inline]
    fn get(&self, c: usize, dy: isize, x: isize) -> f32 {
        self.rows[c][(dy + self.border) as usize][(x - self.x0) as usize]
    }
}

/// The sigma row and the SAD multipliers of a row, shared by the 3 stages.
fn sigma_setup<'b>(
    sigma: &'b super::super::image::ImageF,
    ypos: usize,
    sm: f32,
    lf: &LoopFilter,
) -> (&'b [f32], [f32; K_BLOCK_DIM]) {
    let bsm = sm * lf.epf_border_sad_mul;
    let sad_mul_center: [f32; K_BLOCK_DIM] = [bsm, sm, sm, sm, sm, sm, sm, bsm];
    let sad_mul_border: [f32; K_BLOCK_DIM] = [bsm; K_BLOCK_DIM];
    let row_sigma = sigma.row(ypos / K_BLOCK_DIM + K_SIGMA_PADDING);
    let sad_mul = if ypos % K_BLOCK_DIM == 0 || ypos % K_BLOCK_DIM == K_BLOCK_DIM - 1 {
        sad_mul_border
    } else {
        sad_mul_center
    };
    (row_sigma, sad_mul)
}

/// The sigma column index and the position in the block of pixel `x`.
#[inline]
fn sigma_pos(x: isize, xpos: usize) -> (usize, usize) {
    let p = x as i64 + xpos as i64;
    let bx = ((p + (K_SIGMA_PADDING * K_BLOCK_DIM) as i64) / K_BLOCK_DIM as i64) as usize;
    let ix = p.rem_euclid(K_BLOCK_DIM as i64) as usize;
    (bx, ix)
}

/// Writes the output pixels of the 3 channels.
fn store_out(rows: &mut StageRows<'_>, x0: isize, out: &[Vec<f32>; 3]) {
    for (c, o) in out.iter().enumerate() {
        let p = rows.get_output_row(c, 0);
        rows.store(p, x0, o);
    }
}

// 5x5 plus-shaped kernel with 5 SADs per pixel (3x3 plus-shaped). So this makes
// this filter a 7x7 filter.
/// Translation of `EPF0Stage`.
struct Epf0Stage {
    lf: LoopFilter,
}

impl RenderPipelineStage for Epf0Stage {
    fn settings(&self) -> Settings {
        Settings::symmetric(/*shift=*/ 0, /*border=*/ 3)
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
        let lf = &self.lf;
        let sm = (lf.epf_pass0_sigma_scale as f64 * 1.65) as f32;
        let (row_sigma, sad_mul) = sigma_setup(ctx.sigma, ypos, sm, lf);
        let w = Window::new(rows, 3, xextra, xsize);
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let mut out = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];

        const SADS_OFF: [[isize; 2]; 12] = [
            [-2, 0],
            [-1, -1],
            [-1, 0],
            [-1, 1],
            [0, -2],
            [0, -1],
            [0, 1],
            [0, 2],
            [1, -1],
            [1, 0],
            [1, 1],
            [2, 0],
        ];
        const PLUS_OFF: [[isize; 2]; 5] = [[0, 0], [-1, 0], [0, -1], [1, 0], [0, 1]];

        for i in 0..n {
            let x = x0 + i as isize;
            let (bx, ix) = sigma_pos(x, xpos);

            if row_sigma[bx] < K_MIN_SIGMA {
                for c in 0..3 {
                    out[c][i] = w.get(c, 0, x);
                }
                continue;
            }

            let smv = sad_mul[ix];
            let inv_sigma = row_sigma[bx] * smv;

            let mut sads = [0f32; 12];

            // compute sads
            // TODO(veluca): consider unrolling and optimizing this.
            for c in 0..3 {
                let scale = lf.epf_channel_scale[c];
                for (k, so) in SADS_OFF.iter().enumerate() {
                    let mut sad = 0.0f32;
                    for po in &PLUS_OFF {
                        let r11 = w.get(c, po[0], x + po[1]);
                        let c11 = w.get(c, so[0] + po[0], x + so[1] + po[1]);
                        sad += abs_diff(r11, c11);
                    }
                    sads[k] = sad * scale + sads[k];
                }
            }
            let x_cc = w.get(0, 0, x);
            let y_cc = w.get(1, 0, x);
            let b_cc = w.get(2, 0, x);

            let mut wsum = 1.0f32;
            let mut xx = x_cc;
            let mut yy = y_cc;
            let mut bb = b_cc;

            for (k, so) in SADS_OFF.iter().enumerate() {
                // AddPixel
                let cx = w.get(0, so[0], x + so[1]);
                let cy = w.get(1, so[0], x + so[1]);
                let cb = w.get(2, so[0], x + so[1]);
                let weight = weight(sads[k], inv_sigma);
                wsum += weight;
                xx = weight * cx + xx;
                yy = weight * cy + yy;
                bb = weight * cb + bb;
            }
            // (JXL_HIGH_PRECISION)
            let inv_w = 1.0f32 / wsum;
            out[0][i] = xx * inv_w;
            out[1][i] = yy * inv_w;
            out[2][i] = bb * inv_w;
        }
        store_out(rows, x0, &out);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "EPF0"
    }
}

// 3x3 plus-shaped kernel with 5 SADs per pixel (also 3x3 plus-shaped). So this
// makes this filter a 5x5 filter.
/// Translation of `EPF1Stage`.
struct Epf1Stage {
    lf: LoopFilter,
}

impl RenderPipelineStage for Epf1Stage {
    fn settings(&self) -> Settings {
        Settings::symmetric(/*shift=*/ 0, /*border=*/ 2)
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
        let lf = &self.lf;
        let sm = 1.65f32;
        let (row_sigma, sad_mul) = sigma_setup(ctx.sigma, ypos, sm, lf);
        let w = Window::new(rows, 2, xextra, xsize);
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let mut out = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];

        for i in 0..n {
            let x = x0 + i as isize;
            let (bx, ix) = sigma_pos(x, xpos);

            if row_sigma[bx] < K_MIN_SIGMA {
                for c in 0..3 {
                    out[c][i] = w.get(c, 0, x);
                }
                continue;
            }

            let smv = sad_mul[ix];
            let inv_sigma = row_sigma[bx] * smv;
            let mut sad0 = 0.0f32;
            let mut sad1 = 0.0f32;
            let mut sad2 = 0.0f32;
            let mut sad3 = 0.0f32;

            // compute sads
            for c in 0..3 {
                // center px = 22, px above = 21
                let p20 = w.get(c, -2, x);
                let p21 = w.get(c, -1, x);
                let mut sad0c = abs_diff(p20, p21); // SAD 2, 1

                let p11 = w.get(c, -1, x - 1);
                let mut sad1c = abs_diff(p11, p21); // SAD 1, 2

                let p31 = w.get(c, -1, x + 1);
                let mut sad2c = abs_diff(p31, p21); // SAD 3, 2

                let p02 = w.get(c, 0, x - 2);
                let p12 = w.get(c, 0, x - 1);
                sad1c += abs_diff(p02, p12); // SAD 1, 2
                sad0c += abs_diff(p11, p12); // SAD 2, 1

                let p22 = w.get(c, 0, x);
                let mut t = abs_diff(p12, p22);
                sad1c += t; // SAD 1, 2
                sad2c += t; // SAD 3, 2
                t = abs_diff(p22, p21);
                let mut sad3c = t; // SAD 2, 3
                sad0c += t; // SAD 2, 1

                let p32 = w.get(c, 0, x + 1);
                sad0c += abs_diff(p31, p32); // SAD 2, 1
                t = abs_diff(p22, p32);
                sad1c += t; // SAD 1, 2
                sad2c += t; // SAD 3, 2

                let p42 = w.get(c, 0, x + 2);
                sad2c += abs_diff(p42, p32); // SAD 3, 2

                let p13 = w.get(c, 1, x - 1);
                sad3c += abs_diff(p13, p12); // SAD 2, 3

                let p23 = w.get(c, 1, x);
                t = abs_diff(p22, p23);
                sad0c += t; // SAD 2, 1
                sad3c += t; // SAD 2, 3
                sad1c += abs_diff(p13, p23); // SAD 1, 2

                let p33 = w.get(c, 1, x + 1);
                sad2c += abs_diff(p33, p23); // SAD 3, 2
                sad3c += abs_diff(p33, p32); // SAD 2, 3

                let p24 = w.get(c, 2, x);
                sad3c += abs_diff(p24, p23); // SAD 2, 3

                let scale = lf.epf_channel_scale[c];
                sad0 = sad0c * scale + sad0;
                sad1 = sad1c * scale + sad1;
                sad2 = sad2c * scale + sad2;
                sad3 = sad3c * scale + sad3;
            }
            let x_cc = w.get(0, 0, x);
            let y_cc = w.get(1, 0, x);
            let b_cc = w.get(2, 0, x);

            let mut wsum = 1.0f32;
            let mut xx = x_cc;
            let mut yy = y_cc;
            let mut bb = b_cc;

            let mut add_pixel = |row: isize, px: isize, sad: f32| {
                let cx = w.get(0, row, px);
                let cy = w.get(1, row, px);
                let cb = w.get(2, row, px);
                let weight = weight(sad, inv_sigma);
                wsum += weight;
                xx = weight * cx + xx;
                yy = weight * cy + yy;
                bb = weight * cb + bb;
            };
            // Top row
            add_pixel(-1, x, sad0);
            // Center
            add_pixel(0, x - 1, sad1);
            add_pixel(0, x + 1, sad2);
            // Bottom
            add_pixel(1, x, sad3);
            // (JXL_HIGH_PRECISION)
            let inv_w = 1.0f32 / wsum;
            out[0][i] = xx * inv_w;
            out[1][i] = yy * inv_w;
            out[2][i] = bb * inv_w;
        }
        store_out(rows, x0, &out);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "EPF1"
    }
}

// 3x3 plus-shaped kernel with 1 SAD per pixel. So this makes this filter a 3x3
// filter.
/// Translation of `EPF2Stage`.
struct Epf2Stage {
    lf: LoopFilter,
}

impl RenderPipelineStage for Epf2Stage {
    fn settings(&self) -> Settings {
        Settings::symmetric(/*shift=*/ 0, /*border=*/ 1)
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
        let lf = &self.lf;
        let sm = (lf.epf_pass2_sigma_scale as f64 * 1.65) as f32;
        let (row_sigma, sad_mul) = sigma_setup(ctx.sigma, ypos, sm, lf);
        let w = Window::new(rows, 1, xextra, xsize);
        let n = xsize + 2 * xextra;
        let x0 = -(xextra as isize);
        let mut out = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];

        for i in 0..n {
            let x = x0 + i as isize;
            let (bx, ix) = sigma_pos(x, xpos);

            if row_sigma[bx] < K_MIN_SIGMA {
                for c in 0..3 {
                    out[c][i] = w.get(c, 0, x);
                }
                continue;
            }

            let smv = sad_mul[ix];
            let inv_sigma = row_sigma[bx] * smv;

            let x_cc = w.get(0, 0, x);
            let y_cc = w.get(1, 0, x);
            let b_cc = w.get(2, 0, x);

            let mut wsum = 1.0f32;
            let mut xx = x_cc;
            let mut yy = y_cc;
            let mut bb = b_cc;

            let mut add_pixel = |row: isize, px: isize| {
                let cx = w.get(0, row, px);
                let cy = w.get(1, row, px);
                let cb = w.get(2, row, px);

                let mut sad = abs_diff(cx, x_cc) * lf.epf_channel_scale[0];
                sad = abs_diff(cy, y_cc) * lf.epf_channel_scale[1] + sad;
                sad = abs_diff(cb, b_cc) * lf.epf_channel_scale[2] + sad;

                let weight = weight(sad, inv_sigma);

                wsum += weight;
                xx = weight * cx + xx;
                yy = weight * cy + yy;
                bb = weight * cb + bb;
            };
            // Top row
            add_pixel(-1, x);
            // Center
            add_pixel(0, x - 1);
            add_pixel(0, x + 1);
            // Bottom
            add_pixel(1, x);
            // (JXL_HIGH_PRECISION)
            let inv_w = 1.0f32 / wsum;
            out[0][i] = xx * inv_w;
            out[1][i] = yy * inv_w;
            out[2][i] = bb * inv_w;
        }
        store_out(rows, x0, &out);
    }

    fn get_channel_mode(&self, c: usize) -> RenderPipelineChannelMode {
        if c < 3 {
            RenderPipelineChannelMode::InOut
        } else {
            RenderPipelineChannelMode::Ignored
        }
    }

    fn get_name(&self) -> &'static str {
        "EPF2"
    }
}

/// Translation of `GetEPFStage()` (the stages read `sigma` from the
/// [`StageCtx`]).
pub(crate) fn get_epf_stage(lf: &LoopFilter, epf_stage: usize) -> Result<Box<dyn RenderPipelineStage>, StatusCode> {
    debug_assert!(lf.epf_iters != 0);
    match epf_stage {
        0 => Ok(Box::new(Epf0Stage { lf: lf.clone() })),
        1 => Ok(Box::new(Epf1Stage { lf: lf.clone() })),
        2 => Ok(Box::new(Epf2Stage { lf: lf.clone() })),
        _ => Err(StatusCode::GenericError), // JXL_ABORT("Invalid EPF stage");
    }
}
