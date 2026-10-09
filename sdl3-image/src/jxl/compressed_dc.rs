// Rust translation of lib/jxl/compressed_dc.h and lib/jxl/compressed_dc.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! DC dequantization and adaptive DC smoothing.

use super::ac_context::BlockCtxMap;
use super::base::Status;
use super::frame_header::YCbCrChromaSubsampling;
use super::image::{Image3F, ImageB, Rect};
use super::math::hwy_max;
use super::modular::modular_image::Image;

// TODO(veluca): optimize constants.
const W1: f32 = 0.20345139757231578f32;
const W2: f32 = 0.0334829185968739f32;
const W0: f32 = 1.0f32 - 4.0f32 * (W1 + W2);

/// Translation of `ComputePixelChannel()`; returns (mc, sm).
#[inline]
fn compute_pixel_channel(
    dc_factor: f32,
    row_top: &[f32],
    row: &[f32],
    row_bottom: &[f32],
    gap: &mut f32,
    x: usize,
) -> (f32, f32) {
    let tl = row_top[x - 1];
    let tc = row_top[x];
    let tr = row_top[x + 1];

    let ml = row[x - 1];
    let mc = row[x];
    let mr = row[x + 1];

    let bl = row_bottom[x - 1];
    let bc = row_bottom[x];
    let br = row_bottom[x + 1];

    let w_center = W0;
    let w_side = W1;
    let w_corner = W2;

    let corner = (tl + tr) + (bl + br);
    let side = (ml + mr) + (tc + bc);
    let sm = corner * w_corner + (side * w_side + mc * w_center);

    let dc_quant = dc_factor;
    *gap = hwy_max(*gap, ((mc - sm) / dc_quant).abs());
    (mc, sm)
}

/// Translation of `AdaptiveDCSmoothing()`.
pub(crate) fn adaptive_dc_smoothing(dc_factors: &[f32; 4], dc: &mut Image3F) -> Status {
    let xsize = dc.xsize();
    let ysize = dc.ysize();
    if ysize <= 2 || xsize <= 2 {
        return Ok(());
    }

    // TODO(veluca): use tile-based processing?
    // TODO(veluca): decide if changes to the y channel should be propagated to
    // the x and b channels through color correlation.
    debug_assert!(W1 + W2 < 0.25f32);

    let mut smoothed = Image3F::new(xsize, ysize)?;
    // Fill in borders that the loop below will not. First and last are unused.
    for c in 0..3 {
        for y in [0, ysize - 1] {
            smoothed.plane_row(c, y)[..xsize].copy_from_slice(&dc.const_plane_row(c, y)[..xsize]);
        }
    }
    for y in 1..ysize - 1 {
        let mut out = [vec![0f32; xsize], vec![0f32; xsize], vec![0f32; xsize]];
        {
            let rows_top = [dc.const_plane_row(0, y - 1), dc.const_plane_row(1, y - 1), dc.const_plane_row(2, y - 1)];
            let rows = [dc.const_plane_row(0, y), dc.const_plane_row(1, y), dc.const_plane_row(2, y)];
            let rows_bottom = [dc.const_plane_row(0, y + 1), dc.const_plane_row(1, y + 1), dc.const_plane_row(2, y + 1)];
            for x in [0, xsize - 1] {
                for c in 0..3 {
                    out[c][x] = rows[c][x];
                }
            }

            // (ComputePixel, one lane)
            for x in 1..xsize - 1 {
                let mut gap = 0.5f32;
                let (mc_x, sm_x) = compute_pixel_channel(dc_factors[0], rows_top[0], rows[0], rows_bottom[0], &mut gap, x);
                let (mc_y, sm_y) = compute_pixel_channel(dc_factors[1], rows_top[1], rows[1], rows_bottom[1], &mut gap, x);
                let (mc_b, sm_b) = compute_pixel_channel(dc_factors[2], rows_top[2], rows[2], rows_bottom[2], &mut gap, x);
                let mut factor = -4.0f32 * gap + 3.0f32;
                // ZeroIfNegative
                if factor < 0.0 {
                    factor = 0.0;
                }

                out[0][x] = (sm_x - mc_x) * factor + mc_x;
                out[1][x] = (sm_y - mc_y) * factor + mc_y;
                out[2][x] = (sm_b - mc_b) * factor + mc_b;
            }
        }
        for c in 0..3 {
            smoothed.plane_row(c, y)[..xsize].copy_from_slice(&out[c]);
        }
    }
    dc.swap(&mut smoothed);
    Ok(())
}

/// DC dequantization. Translation of `DequantDC()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dequant_dc(
    r: &Rect,
    dc: &mut Image3F,
    quant_dc: &mut ImageB,
    input: &Image,
    dc_factors: &[f32; 4],
    mul: f32,
    cfl_factors: &[f32; 4],
    chroma_subsampling: &YCbCrChromaSubsampling,
    bctx: &BlockCtxMap,
) {
    if chroma_subsampling.is_444() {
        let fac_x = dc_factors[0] * mul;
        let fac_y = dc_factors[1] * mul;
        let fac_b = dc_factors[2] * mul;
        let cfl_fac_x = cfl_factors[0];
        let cfl_fac_b = cfl_factors[2];
        for y in 0..r.ysize() {
            let quant_row_x = input.channel[1].plane.row(y);
            let quant_row_y = input.channel[0].plane.row(y);
            let quant_row_b = input.channel[2].plane.row(y);
            let n = r.xsize();
            let mut rx = vec![0f32; n];
            let mut ry = vec![0f32; n];
            let mut rb = vec![0f32; n];
            for x in 0..n {
                let in_x = quant_row_x[x] as f32 * fac_x;
                let in_y = quant_row_y[x] as f32 * fac_y;
                let in_b = quant_row_b[x] as f32 * fac_b;
                ry[x] = in_y;
                rx[x] = in_y * cfl_fac_x + in_x;
                rb[x] = in_y * cfl_fac_b + in_b;
            }
            r.plane_row(dc, 0, y)[..n].copy_from_slice(&rx);
            r.plane_row(dc, 1, y)[..n].copy_from_slice(&ry);
            r.plane_row(dc, 2, y)[..n].copy_from_slice(&rb);
        }
    } else {
        for c in [1usize, 0, 2] {
            let rect = Rect::new(
                r.x0() >> chroma_subsampling.h_shift(c),
                r.y0() >> chroma_subsampling.v_shift(c),
                r.xsize() >> chroma_subsampling.h_shift(c),
                r.ysize() >> chroma_subsampling.v_shift(c),
            );
            let fac = dc_factors[c] * mul;
            let ch = &input.channel[if c < 2 { c ^ 1 } else { c }];
            for y in 0..rect.ysize() {
                let quant_row = ch.plane.row(y);
                let row = rect.plane_row(dc, c, y);
                for x in 0..rect.xsize() {
                    row[x] = quant_row[x] as f32 * fac;
                }
            }
        }
    }
    if bctx.num_dc_ctxs <= 1 {
        for y in 0..r.ysize() {
            let n = r.xsize();
            r.row(quant_dc, y)[..n].fill(0);
        }
    } else {
        for y in 0..r.ysize() {
            let quant_row_x = input.channel[1].plane.row(y >> chroma_subsampling.v_shift(0));
            let quant_row_y = input.channel[0].plane.row(y >> chroma_subsampling.v_shift(1));
            let quant_row_b = input.channel[2].plane.row(y >> chroma_subsampling.v_shift(2));
            let qdc_row_val = r.row(quant_dc, y);
            for x in 0..r.xsize() {
                let mut bucket_x: i32 = 0;
                let mut bucket_y: i32 = 0;
                let mut bucket_b: i32 = 0;
                for &t in &bctx.dc_thresholds[0] {
                    if quant_row_x[x >> chroma_subsampling.h_shift(0)] > t {
                        bucket_x += 1;
                    }
                }
                for &t in &bctx.dc_thresholds[1] {
                    if quant_row_y[x >> chroma_subsampling.h_shift(1)] > t {
                        bucket_y += 1;
                    }
                }
                for &t in &bctx.dc_thresholds[2] {
                    if quant_row_b[x >> chroma_subsampling.h_shift(2)] > t {
                        bucket_b += 1;
                    }
                }
                let mut bucket = bucket_x;
                bucket *= bctx.dc_thresholds[2].len() as i32 + 1;
                bucket += bucket_b;
                bucket *= bctx.dc_thresholds[1].len() as i32 + 1;
                bucket += bucket_y;
                qdc_row_val[x] = bucket as u8;
            }
        }
    }
}
