// Rust translation of src/enc/predictor_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2016 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Image transform methods for lossless encoder.
//!
//! The near-lossless quantization (`WEBP_NEAR_LOSSLESS`, for a near-lossless
//! setting below 100) and the low-effort mode are not translated:
//! SDL_image encodes losslessly with method 4, where `max_quantization` is
//! always 1. The scratch rows are offsets in `argb_scratch`.

use crate::webp::decode::ARGB_BLACK;
use crate::webp::dsp::lossless_enc::{
    vp8l_collect_color_blue_transforms, vp8l_collect_color_red_transforms,
    vp8l_combined_shannon_entropy, vp8l_predictors, vp8l_predictors_sub, vp8l_sub_pixels,
    vp8l_sub_sample_size, vp8l_transform_color, VP8LMultipliers,
};
use crate::webp::enc::vp8l_enc::MAX_TRANSFORM_BITS;

const MAX_DIFF_COST: f32 = 1e30;

const K_SPATIAL_PREDICTOR_BIAS: f32 = 15.0;
const K_MASK_ALPHA: u32 = 0xff000000;

/// Mostly used to reduce code size + readability. Translation of
/// `GetMin()`.
fn get_min(a: i32, b: i32) -> i32 {
    if a > b {
        b
    } else {
        a
    }
}

//------------------------------------------------------------------------------
// Methods to calculate Entropy (Shannon).

/// Translation of `PredictionCostSpatial()`.
fn prediction_cost_spatial(counts: &[i32; 256], weight_0: i32, mut exp_val: f32) -> f32 {
    let significant_symbols = 256 >> 4;
    let exp_decay_factor = 0.6f32;
    let mut bits = weight_0 as f32 * counts[0] as f32;
    for i in 1..significant_symbols {
        bits += exp_val * (counts[i] + counts[256 - i]) as f32;
        exp_val *= exp_decay_factor;
    }
    (-0.1f64 * bits as f64) as f32
}

/// Translation of `PredictionCostSpatialHistogram()`.
fn prediction_cost_spatial_histogram(accumulated: &[[i32; 256]; 4], tile: &[[i32; 256]; 4]) -> f32 {
    let mut retval = 0.0f32;
    for i in 0..4 {
        let k_exp_value = 0.94f32;
        retval += prediction_cost_spatial(&tile[i], 1, k_exp_value);
        retval += vp8l_combined_shannon_entropy(&tile[i], &accumulated[i]);
    }
    retval
}

/// Translation of `UpdateHisto()`.
fn update_histo(histo_argb: &mut [[i32; 256]; 4], argb: u32) {
    histo_argb[0][(argb >> 24) as usize] += 1;
    histo_argb[1][((argb >> 16) & 0xff) as usize] += 1;
    histo_argb[2][((argb >> 8) & 0xff) as usize] += 1;
    histo_argb[3][(argb & 0xff) as usize] += 1;
}

//------------------------------------------------------------------------------
// Spatial transform functions.

/// Translation of `PredictBatch()`: the rows `current` and `upper` are in
/// `rows`.
#[allow(clippy::too_many_arguments)]
fn predict_batch(
    mode: u32,
    mut x_start: usize,
    y: i32,
    mut num_pixels: usize,
    rows: &[u32],
    current: usize,
    upper: usize,
    out: &mut [u32],
) {
    let mut out_pos = 0;
    if x_start == 0 {
        if y == 0 {
            // ARGB_BLACK.
            vp8l_predictors_sub(0, rows, current, upper, 1, &mut out[out_pos..]);
        } else {
            // Top one.
            vp8l_predictors_sub(2, rows, current, upper, 1, &mut out[out_pos..]);
        }
        x_start += 1;
        out_pos += 1;
        num_pixels -= 1;
    }
    if y == 0 {
        // Left one.
        vp8l_predictors_sub(
            1,
            rows,
            current + x_start,
            0,
            num_pixels,
            &mut out[out_pos..],
        );
    } else {
        vp8l_predictors_sub(
            mode,
            rows,
            current + x_start,
            upper + x_start,
            num_pixels,
            &mut out[out_pos..],
        );
    }
}

/// Stores the difference between the pixel and its prediction in "out".
/// In case of a lossy encoding, updates the source image to avoid
/// propagating the deviation further to pixels which depend on the current
/// pixel for their predictions. Translation of `GetResidual()` (without
/// the near-lossless quantization): the rows `upper_row` and `current_row`
/// are in `rows`.
#[allow(clippy::too_many_arguments)]
fn get_residual(
    width: usize,
    rows: &mut [u32],
    upper_row: usize,
    current_row: usize,
    mode: u32,
    x_start: usize,
    x_end: usize,
    y: i32,
    exact: bool,
    out: &mut [u32],
) {
    if exact {
        predict_batch(
            mode,
            x_start,
            y,
            x_end - x_start,
            rows,
            current_row,
            upper_row,
            out,
        );
    } else {
        for x in x_start..x_end {
            let predict = if y == 0 {
                if x == 0 {
                    ARGB_BLACK
                } else {
                    rows[current_row + x - 1] // Left.
                }
            } else if x == 0 {
                rows[upper_row + x] // Top.
            } else {
                vp8l_predictors(mode, rows[current_row + x - 1], rows, upper_row + x)
            };
            let mut residual = vp8l_sub_pixels(rows[current_row + x], predict);
            if (rows[current_row + x] & K_MASK_ALPHA) == 0 {
                // If alpha is 0, cleanup RGB. We can choose the RGB values of the
                // residual for best compression. The prediction of alpha itself can be
                // non-zero and must be kept though. We choose RGB of the residual to be
                // 0.
                residual &= K_MASK_ALPHA;
                // Update the source image.
                rows[current_row + x] = predict & !K_MASK_ALPHA;
                // The prediction for the rightmost pixel in a row uses the leftmost
                // pixel
                // in that row as its top-right context pixel. Hence if we change the
                // leftmost pixel of current_row, the corresponding change must be
                // applied
                // to upper_row as well where top-right context is being read from.
                if x == 0 && y != 0 {
                    rows[upper_row + width] = rows[current_row];
                }
            }
            out[x - x_start] = residual;
        }
    }
}

/// Returns best predictor and updates the accumulated histogram.
/// If max_quantization > 1, assumes that near lossless processing will be
/// applied, quantizing residuals to multiples of quantization levels up to
/// max_quantization (the actual quantization level depends on smoothness
/// near the given pixel). Translation of `GetBestPredictorForTile()`.
#[allow(clippy::too_many_arguments)]
fn get_best_predictor_for_tile(
    width: i32,
    height: i32,
    tile_x: i32,
    tile_y: i32,
    bits: i32,
    accumulated: &mut [[i32; 256]; 4],
    argb_scratch: &mut [u32],
    argb: &[u32],
    exact: bool,
    modes: &[u32],
) -> i32 {
    let k_num_pred_modes = 14;
    let start_x = tile_x << bits;
    let start_y = tile_y << bits;
    let tile_size = 1 << bits;
    let max_y = get_min(tile_size, height - start_y);
    let max_x = get_min(tile_size, width - start_x);
    // Whether there exist columns just outside the tile.
    let have_left = (start_x > 0) as i32;
    // Position and size of the strip covering the tile and adjacent columns if
    // they exist.
    let context_start_x = (start_x - have_left) as usize;
    let tiles_per_row = vp8l_sub_sample_size(width as u32, bits as u32) as i32;
    // Prediction modes of the left and above neighbor tiles.
    let left_mode = if tile_x > 0 {
        ((modes[(tile_y * tiles_per_row + tile_x - 1) as usize] >> 8) & 0xff) as i32
    } else {
        0xff
    };
    let above_mode = if tile_y > 0 {
        ((modes[((tile_y - 1) * tiles_per_row + tile_x) as usize] >> 8) & 0xff) as i32
    } else {
        0xff
    };
    let width_u = width as usize;
    // The width of upper_row and current_row is one pixel larger than image width
    // to allow the top right pixel to point to the leftmost pixel of the next row
    // when at the right edge.
    let mut upper_row = 0usize;
    let mut current_row = upper_row + width_u + 1;
    let mut best_diff = MAX_DIFF_COST;
    let mut best_mode = 0;
    let mut histo_argb = Box::new([[0i32; 256]; 4]);
    let mut best_histo = Box::new([[0i32; 256]; 4]);
    let mut residuals = [0u32; 1 << MAX_TRANSFORM_BITS];
    debug_assert!(bits <= MAX_TRANSFORM_BITS);
    debug_assert!(max_x <= (1 << MAX_TRANSFORM_BITS));

    for mode in 0..k_num_pred_modes {
        *histo_argb = [[0; 256]; 4];
        if start_y > 0 {
            // Read the row above the tile which will become the first upper_row.
            // Include a pixel to the left if it exists; include a pixel to the right
            // in all cases (wrapping to the leftmost pixel of the next row if it does
            // not exist).
            let n = (max_x + have_left + 1) as usize;
            let src = (start_y - 1) as usize * width_u + context_start_x;
            argb_scratch[current_row + context_start_x..current_row + context_start_x + n]
                .copy_from_slice(&argb[src..src + n]);
        }
        for relative_y in 0..max_y {
            let y = start_y + relative_y;
            std::mem::swap(&mut upper_row, &mut current_row);
            // Read current_row. Include a pixel to the left if it exists; include a
            // pixel to the right in all cases except at the bottom right corner of
            // the image (wrapping to the leftmost pixel of the next row if it does
            // not exist in the current row).
            let n = (max_x + have_left + (y + 1 < height) as i32) as usize;
            let src = y as usize * width_u + context_start_x;
            argb_scratch[current_row + context_start_x..current_row + context_start_x + n]
                .copy_from_slice(&argb[src..src + n]);

            get_residual(
                width_u,
                argb_scratch,
                upper_row,
                current_row,
                mode,
                start_x as usize,
                (start_x + max_x) as usize,
                y,
                exact,
                &mut residuals,
            );
            for &r in &residuals[..max_x as usize] {
                update_histo(&mut histo_argb, r);
            }
        }
        let mut cur_diff = prediction_cost_spatial_histogram(accumulated, &histo_argb);
        // Favor keeping the areas locally similar.
        if mode as i32 == left_mode {
            cur_diff -= K_SPATIAL_PREDICTOR_BIAS;
        }
        if mode as i32 == above_mode {
            cur_diff -= K_SPATIAL_PREDICTOR_BIAS;
        }

        if cur_diff < best_diff {
            std::mem::swap(&mut histo_argb, &mut best_histo);
            best_diff = cur_diff;
            best_mode = mode as i32;
        }
    }

    for i in 0..4 {
        for j in 0..256 {
            accumulated[i][j] += best_histo[i][j];
        }
    }

    best_mode
}

/// Converts pixels of the image to residuals with respect to predictions.
/// If max_quantization > 1, applies near lossless processing, quantizing
/// residuals to multiples of quantization levels up to max_quantization
/// (the actual quantization level depends on smoothness near the given
/// pixel). Translation of `CopyImageWithPrediction()` (without the
/// near-lossless and low-effort modes).
fn copy_image_with_prediction(
    width: i32,
    height: i32,
    bits: i32,
    modes: &[u32],
    argb_scratch: &mut [u32],
    argb: &mut [u32],
    exact: bool,
) {
    let tiles_per_row = vp8l_sub_sample_size(width as u32, bits as u32) as usize;
    let width_u = width as usize;
    // The width of upper_row and current_row is one pixel larger than image width
    // to allow the top right pixel to point to the leftmost pixel of the next row
    // when at the right edge.
    let mut upper_row = 0usize;
    let mut current_row = upper_row + width_u + 1;

    for y in 0..height {
        std::mem::swap(&mut upper_row, &mut current_row);
        let n = width_u + (y + 1 < height) as usize;
        let src = y as usize * width_u;
        argb_scratch[current_row..current_row + n].copy_from_slice(&argb[src..src + n]);

        let mut x = 0usize;
        while x < width_u {
            let mode = (modes[(y >> bits) as usize * tiles_per_row + (x >> bits)] >> 8) & 0xff;
            let mut x_end = x + (1 << bits);
            if x_end > width_u {
                x_end = width_u;
            }
            get_residual(
                width_u,
                argb_scratch,
                upper_row,
                current_row,
                mode,
                x,
                x_end,
                y,
                exact,
                &mut argb[src + x..],
            );
            x = x_end;
        }
    }
}

/// Finds the best predictor for each tile, and converts the image to
/// residuals with respect to predictions. If near_lossless_quality < 100,
/// applies near lossless processing, shaving off more bits of residuals
/// for lower qualities. Translation of `VP8LResidualImage()` (without the
/// near-lossless and low-effort modes and the progress report).
#[allow(clippy::too_many_arguments)]
pub(crate) fn vp8l_residual_image(
    width: i32,
    height: i32,
    bits: i32,
    low_effort: bool,
    argb: &mut [u32],
    argb_scratch: &mut [u32],
    image: &mut [u32],
    near_lossless_quality: i32,
    exact: bool,
    _used_subtract_green: bool,
) -> bool {
    let tiles_per_row = vp8l_sub_sample_size(width as u32, bits as u32) as i32;
    let tiles_per_col = vp8l_sub_sample_size(height as u32, bits as u32) as i32;
    let mut histo = Box::new([[0i32; 256]; 4]);
    debug_assert!(!low_effort, "the low-effort mode is not translated");
    debug_assert!(
        near_lossless_quality == 100,
        "near-lossless is not translated"
    );
    for tile_y in 0..tiles_per_col {
        for tile_x in 0..tiles_per_row {
            let pred = get_best_predictor_for_tile(
                width,
                height,
                tile_x,
                tile_y,
                bits,
                &mut histo,
                argb_scratch,
                argb,
                exact,
                image,
            );
            image[(tile_y * tiles_per_row + tile_x) as usize] = ARGB_BLACK | ((pred as u32) << 8);
        }
    }

    copy_image_with_prediction(width, height, bits, image, argb_scratch, argb, exact);
    true
}

//------------------------------------------------------------------------------
// Color transform functions.

/// Translation of `MultipliersClear()`.
fn multipliers_clear(m: &mut VP8LMultipliers) {
    m.green_to_red = 0;
    m.green_to_blue = 0;
    m.red_to_blue = 0;
}

/// Translation of `ColorCodeToMultipliers()`.
fn color_code_to_multipliers(color_code: u32, m: &mut VP8LMultipliers) {
    m.green_to_red = (color_code & 0xff) as u8;
    m.green_to_blue = ((color_code >> 8) & 0xff) as u8;
    m.red_to_blue = ((color_code >> 16) & 0xff) as u8;
}

/// Translation of `MultipliersToColorCode()`.
fn multipliers_to_color_code(m: &VP8LMultipliers) -> u32 {
    0xff000000
        | ((m.red_to_blue as u32) << 16)
        | ((m.green_to_blue as u32) << 8)
        | m.green_to_red as u32
}

/// Translation of `PredictionCostCrossColor()`.
fn prediction_cost_cross_color(accumulated: &[i32; 256], counts: &[i32; 256]) -> f32 {
    // Favor low entropy, locally and globally.
    // Favor small absolute values for PredictionCostSpatial
    const K_EXP_VALUE: f32 = 2.4;
    vp8l_combined_shannon_entropy(counts, accumulated)
        + prediction_cost_spatial(counts, 3, K_EXP_VALUE)
}

/// Translation of `GetPredictionCostCrossColorRed()`.
#[allow(clippy::too_many_arguments)]
fn get_prediction_cost_cross_color_red(
    argb: &[u32],
    stride: usize,
    tile_width: usize,
    tile_height: usize,
    prev_x: VP8LMultipliers,
    prev_y: VP8LMultipliers,
    green_to_red: i32,
    accumulated_red_histo: &[i32; 256],
) -> f32 {
    let mut histo = [0i32; 256];

    vp8l_collect_color_red_transforms(
        argb,
        stride,
        tile_width,
        tile_height,
        green_to_red,
        &mut histo,
    );

    let mut cur_diff = prediction_cost_cross_color(accumulated_red_histo, &histo);
    if green_to_red as u8 == prev_x.green_to_red {
        cur_diff -= 3.0; // favor keeping the areas locally similar
    }
    if green_to_red as u8 == prev_y.green_to_red {
        cur_diff -= 3.0; // favor keeping the areas locally similar
    }
    if green_to_red == 0 {
        cur_diff -= 3.0;
    }
    cur_diff
}

/// Translation of `GetBestGreenToRed()`.
#[allow(clippy::too_many_arguments)]
fn get_best_green_to_red(
    argb: &[u32],
    stride: usize,
    tile_width: usize,
    tile_height: usize,
    prev_x: VP8LMultipliers,
    prev_y: VP8LMultipliers,
    quality: i32,
    accumulated_red_histo: &[i32; 256],
    best_tx: &mut VP8LMultipliers,
) {
    let k_max_iters = 4 + ((7 * quality) >> 8); // in range [4..6]
    let mut green_to_red_best = 0;
    let mut best_diff = get_prediction_cost_cross_color_red(
        argb,
        stride,
        tile_width,
        tile_height,
        prev_x,
        prev_y,
        green_to_red_best,
        accumulated_red_histo,
    );
    for iter in 0..k_max_iters {
        // ColorTransformDelta is a 3.5 bit fixed point, so 32 is equal to
        // one in color computation. Having initial delta here as 1 is sufficient
        // to explore the range of (-2, 2).
        let delta = 32 >> iter;
        // Try a negative and a positive delta from the best known value.
        let mut offset = -delta;
        while offset <= delta {
            let green_to_red_cur = offset + green_to_red_best;
            let cur_diff = get_prediction_cost_cross_color_red(
                argb,
                stride,
                tile_width,
                tile_height,
                prev_x,
                prev_y,
                green_to_red_cur,
                accumulated_red_histo,
            );
            if cur_diff < best_diff {
                best_diff = cur_diff;
                green_to_red_best = green_to_red_cur;
            }
            offset += 2 * delta;
        }
    }
    best_tx.green_to_red = (green_to_red_best & 0xff) as u8;
}

/// Translation of `GetPredictionCostCrossColorBlue()`.
#[allow(clippy::too_many_arguments)]
fn get_prediction_cost_cross_color_blue(
    argb: &[u32],
    stride: usize,
    tile_width: usize,
    tile_height: usize,
    prev_x: VP8LMultipliers,
    prev_y: VP8LMultipliers,
    green_to_blue: i32,
    red_to_blue: i32,
    accumulated_blue_histo: &[i32; 256],
) -> f32 {
    let mut histo = [0i32; 256];

    vp8l_collect_color_blue_transforms(
        argb,
        stride,
        tile_width,
        tile_height,
        green_to_blue,
        red_to_blue,
        &mut histo,
    );

    let mut cur_diff = prediction_cost_cross_color(accumulated_blue_histo, &histo);
    if green_to_blue as u8 == prev_x.green_to_blue {
        cur_diff -= 3.0; // favor keeping the areas locally similar
    }
    if green_to_blue as u8 == prev_y.green_to_blue {
        cur_diff -= 3.0; // favor keeping the areas locally similar
    }
    if red_to_blue as u8 == prev_x.red_to_blue {
        cur_diff -= 3.0; // favor keeping the areas locally similar
    }
    if red_to_blue as u8 == prev_y.red_to_blue {
        cur_diff -= 3.0; // favor keeping the areas locally similar
    }
    if green_to_blue == 0 {
        cur_diff -= 3.0;
    }
    if red_to_blue == 0 {
        cur_diff -= 3.0;
    }
    cur_diff
}

const K_GREEN_RED_TO_BLUE_NUM_AXIS: usize = 8;
const K_GREEN_RED_TO_BLUE_MAX_ITERS: usize = 7;

/// Translation of `GetBestGreenRedToBlue()`.
#[allow(clippy::too_many_arguments)]
fn get_best_green_red_to_blue(
    argb: &[u32],
    stride: usize,
    tile_width: usize,
    tile_height: usize,
    prev_x: VP8LMultipliers,
    prev_y: VP8LMultipliers,
    quality: i32,
    accumulated_blue_histo: &[i32; 256],
    best_tx: &mut VP8LMultipliers,
) {
    const OFFSET: [[i8; 2]; K_GREEN_RED_TO_BLUE_NUM_AXIS] = [
        [0, -1],
        [0, 1],
        [-1, 0],
        [1, 0],
        [-1, -1],
        [-1, 1],
        [1, -1],
        [1, 1],
    ];
    const DELTA_LUT: [i8; K_GREEN_RED_TO_BLUE_MAX_ITERS] = [16, 16, 8, 4, 2, 2, 2];
    let iters = if quality < 25 {
        1
    } else if quality > 50 {
        K_GREEN_RED_TO_BLUE_MAX_ITERS
    } else {
        4
    };
    let mut green_to_blue_best = 0;
    let mut red_to_blue_best = 0;
    // Initial value at origin:
    let mut best_diff = get_prediction_cost_cross_color_blue(
        argb,
        stride,
        tile_width,
        tile_height,
        prev_x,
        prev_y,
        green_to_blue_best,
        red_to_blue_best,
        accumulated_blue_histo,
    );
    for iter in 0..iters {
        let delta = DELTA_LUT[iter] as i32;
        for axis in 0..K_GREEN_RED_TO_BLUE_NUM_AXIS {
            let green_to_blue_cur = OFFSET[axis][0] as i32 * delta + green_to_blue_best;
            let red_to_blue_cur = OFFSET[axis][1] as i32 * delta + red_to_blue_best;
            let cur_diff = get_prediction_cost_cross_color_blue(
                argb,
                stride,
                tile_width,
                tile_height,
                prev_x,
                prev_y,
                green_to_blue_cur,
                red_to_blue_cur,
                accumulated_blue_histo,
            );
            if cur_diff < best_diff {
                best_diff = cur_diff;
                green_to_blue_best = green_to_blue_cur;
                red_to_blue_best = red_to_blue_cur;
            }
            if quality < 25 && iter == 4 {
                // Only axis aligned diffs for lower quality.
                break; // next iter.
            }
        }
        if delta == 2 && green_to_blue_best == 0 && red_to_blue_best == 0 {
            // Further iterations would not help.
            break; // out of iter-loop.
        }
    }
    best_tx.green_to_blue = (green_to_blue_best & 0xff) as u8;
    best_tx.red_to_blue = (red_to_blue_best & 0xff) as u8;
}

/// Translation of `GetBestColorTransformForTile()`.
#[allow(clippy::too_many_arguments)]
fn get_best_color_transform_for_tile(
    tile_x: i32,
    tile_y: i32,
    bits: i32,
    prev_x: VP8LMultipliers,
    prev_y: VP8LMultipliers,
    quality: i32,
    xsize: i32,
    ysize: i32,
    accumulated_red_histo: &[i32; 256],
    accumulated_blue_histo: &[i32; 256],
    argb: &[u32],
) -> VP8LMultipliers {
    let max_tile_size = 1 << bits;
    let tile_y_offset = tile_y * max_tile_size;
    let tile_x_offset = tile_x * max_tile_size;
    let all_x_max = get_min(tile_x_offset + max_tile_size, xsize);
    let all_y_max = get_min(tile_y_offset + max_tile_size, ysize);
    let tile_width = (all_x_max - tile_x_offset) as usize;
    let tile_height = (all_y_max - tile_y_offset) as usize;
    let tile_argb = &argb[(tile_y_offset * xsize + tile_x_offset) as usize..];
    let mut best_tx = VP8LMultipliers::default();
    multipliers_clear(&mut best_tx);

    get_best_green_to_red(
        tile_argb,
        xsize as usize,
        tile_width,
        tile_height,
        prev_x,
        prev_y,
        quality,
        accumulated_red_histo,
        &mut best_tx,
    );
    get_best_green_red_to_blue(
        tile_argb,
        xsize as usize,
        tile_width,
        tile_height,
        prev_x,
        prev_y,
        quality,
        accumulated_blue_histo,
        &mut best_tx,
    );
    best_tx
}

/// Translation of `CopyTileWithColorTransform()`.
fn copy_tile_with_color_transform(
    xsize: i32,
    ysize: i32,
    tile_x: i32,
    tile_y: i32,
    max_tile_size: i32,
    color_transform: VP8LMultipliers,
    argb: &mut [u32],
) {
    let xscan = get_min(max_tile_size, xsize - tile_x) as usize;
    let yscan = get_min(max_tile_size, ysize - tile_y);
    let mut pos = (tile_y * xsize + tile_x) as usize;
    for _ in 0..yscan {
        vp8l_transform_color(&color_transform, &mut argb[pos..pos + xscan]);
        pos += xsize as usize;
    }
}

/// Translation of `VP8LColorSpaceTransform()` (without the progress
/// report).
pub(crate) fn vp8l_color_space_transform(
    width: i32,
    height: i32,
    bits: i32,
    quality: i32,
    argb: &mut [u32],
    image: &mut [u32],
) -> bool {
    let max_tile_size = 1 << bits;
    let tile_xsize = vp8l_sub_sample_size(width as u32, bits as u32) as i32;
    let tile_ysize = vp8l_sub_sample_size(height as u32, bits as u32) as i32;
    let mut accumulated_red_histo = [0i32; 256];
    let mut accumulated_blue_histo = [0i32; 256];
    let mut prev_x = VP8LMultipliers::default();
    let mut prev_y = VP8LMultipliers::default();
    multipliers_clear(&mut prev_y);
    multipliers_clear(&mut prev_x);
    for tile_y in 0..tile_ysize {
        for tile_x in 0..tile_xsize {
            let tile_x_offset = tile_x * max_tile_size;
            let tile_y_offset = tile_y * max_tile_size;
            let all_x_max = get_min(tile_x_offset + max_tile_size, width);
            let all_y_max = get_min(tile_y_offset + max_tile_size, height);
            let offset = (tile_y * tile_xsize + tile_x) as usize;
            if tile_y != 0 {
                color_code_to_multipliers(image[offset - tile_xsize as usize], &mut prev_y);
            }
            prev_x = get_best_color_transform_for_tile(
                tile_x,
                tile_y,
                bits,
                prev_x,
                prev_y,
                quality,
                width,
                height,
                &accumulated_red_histo,
                &accumulated_blue_histo,
                argb,
            );
            image[offset] = multipliers_to_color_code(&prev_x);
            copy_tile_with_color_transform(
                width,
                height,
                tile_x_offset,
                tile_y_offset,
                max_tile_size,
                prev_x,
                argb,
            );

            // Gather accumulated histogram data.
            for y in tile_y_offset..all_y_max {
                let mut ix = (y * width + tile_x_offset) as usize;
                let ix_end = ix + (all_x_max - tile_x_offset) as usize;
                let width = width as usize;
                while ix < ix_end {
                    let pix = argb[ix];
                    if ix >= 2 && pix == argb[ix - 2] && pix == argb[ix - 1] {
                        ix += 1;
                        continue; // repeated pixels are handled by backward references
                    }
                    if ix >= width + 2
                        && argb[ix - 2] == argb[ix - width - 2]
                        && argb[ix - 1] == argb[ix - width - 1]
                        && pix == argb[ix - width]
                    {
                        ix += 1;
                        continue; // repeated pixels are handled by backward references
                    }
                    accumulated_red_histo[((pix >> 16) & 0xff) as usize] += 1;
                    accumulated_blue_histo[(pix & 0xff) as usize] += 1;
                    ix += 1;
                }
            }
        }
    }
    true
}
