// Rust translation of lib/jxl/dec_transforms-inl.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches), for Highway's
// scalar target.
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The inverse transforms of the VarDCT blocks, and the lowest frequencies
//! of the larger ones from the DC image.

use super::ac_strategy::acs_type as Type;
use super::base::{K_BLOCK_DIM, K_DCT_BLOCK_SIZE};
use super::dct::{compute_scaled_dct, compute_scaled_idct, DctFrom, DctTo};
use super::dct_scales::dct_total_resample_scale;

/// Computes the lowest-frequency LF_ROWSxLF_COLS-sized square in output, which
/// is a DCT_ROWS*DCT_COLS-sized DCT block, by doing a ROWS*COLS DCT on the
/// input block. Translation of `ReinterpretingDCT<DCT_ROWS, DCT_COLS,
/// LF_ROWS, LF_COLS, ROWS, COLS>()` (with LF_ROWS == ROWS and LF_COLS ==
/// COLS, as its static assertions require).
#[inline]
fn reinterpreting_dct(
    dct_rows: usize,
    dct_cols: usize,
    rows: usize,
    cols: usize,
    input: &[f32],
    input_stride: usize,
    output: &mut [f32],
    output_stride: usize,
) {
    let (lf_rows, lf_cols) = (rows, cols);
    let mut block = [0f32; 32 * 32];

    // ROWS, COLS <= 8, so we can put scratch space on the stack.
    let mut scratch_space = [0f32; 32 * 32];
    compute_scaled_dct(
        rows,
        cols,
        &DctFrom::new(input, input_stride),
        &mut block,
        &mut scratch_space,
    );
    if rows < cols {
        for y in 0..lf_rows {
            for x in 0..lf_cols {
                output[y * output_stride + x] = block[y * cols + x]
                    * dct_total_resample_scale(rows, dct_rows, y)
                    * dct_total_resample_scale(cols, dct_cols, x);
            }
        }
    } else {
        for y in 0..lf_cols {
            for x in 0..lf_rows {
                output[y * output_stride + x] = block[y * rows + x]
                    * dct_total_resample_scale(cols, dct_cols, y)
                    * dct_total_resample_scale(rows, dct_rows, x);
            }
        }
    }
}

/// Translation of `IDCT2TopBlock<S>()`, in place (as it is always called).
fn idct2_top_block(s: usize, block: &mut [f32]) {
    debug_assert!(K_BLOCK_DIM % s == 0, "S should be a divisor of kBlockDim");
    debug_assert!(s % 2 == 0, "S should be even");
    let mut temp = [0f32; K_DCT_BLOCK_SIZE];
    let num_2x2 = s / 2;
    for y in 0..num_2x2 {
        for x in 0..num_2x2 {
            let c00 = block[y * K_BLOCK_DIM + x];
            let c01 = block[y * K_BLOCK_DIM + num_2x2 + x];
            let c10 = block[(y + num_2x2) * K_BLOCK_DIM + x];
            let c11 = block[(y + num_2x2) * K_BLOCK_DIM + num_2x2 + x];
            let r00 = c00 + c01 + c10 + c11;
            let r01 = c00 + c01 - c10 - c11;
            let r10 = c00 - c01 + c10 - c11;
            let r11 = c00 - c01 - c10 + c11;
            temp[y * 2 * K_BLOCK_DIM + x * 2] = r00;
            temp[y * 2 * K_BLOCK_DIM + x * 2 + 1] = r01;
            temp[(y * 2 + 1) * K_BLOCK_DIM + x * 2] = r10;
            temp[(y * 2 + 1) * K_BLOCK_DIM + x * 2 + 1] = r11;
        }
    }
    for y in 0..s {
        for x in 0..s {
            block[y * K_BLOCK_DIM + x] = temp[y * K_BLOCK_DIM + x];
        }
    }
}

const K_4X4_AFV_BASIS: [[f32; 16]; 16] = [
    [
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
        0.25_f64 as f32,
    ],
    [
        0.876902929799142_f32,
        0.2206518106944235_f32,
        -0.10140050393753763_f32,
        -0.1014005039375375_f32,
        0.2206518106944236_f32,
        -0.10140050393753777_f32,
        -0.10140050393753772_f32,
        -0.10140050393753763_f32,
        -0.10140050393753758_f32,
        -0.10140050393753769_f32,
        -0.1014005039375375_f32,
        -0.10140050393753768_f32,
        -0.10140050393753768_f32,
        -0.10140050393753759_f32,
        -0.10140050393753763_f32,
        -0.10140050393753741_f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.40670075830260755_f32,
        0.44444816619734445_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.19574399372042936_f32,
        0.2929100136981264_f32,
        -0.40670075830260716_f32,
        -0.19574399372042872_f32,
        0.0_f64 as f32,
        0.11379074460448091_f32,
        -0.44444816619734384_f32,
        -0.29291001369812636_f32,
        -0.1137907446044814_f32,
        0.0_f64 as f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.21255748058288748_f32,
        0.3085497062849767_f32,
        0.0_f64 as f32,
        0.4706702258572536_f32,
        -0.1621205195722993_f32,
        0.0_f64 as f32,
        -0.21255748058287047_f32,
        -0.16212051957228327_f32,
        -0.47067022585725277_f32,
        -0.1464291867126764_f32,
        0.3085497062849487_f32,
        0.0_f64 as f32,
        -0.14642918671266536_f32,
        0.4251149611657548_f32,
    ],
    [
        0.0_f64 as f32,
        -0.7071067811865474_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.7071067811865476_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
    ],
    [
        -0.4105377591765233_f32,
        0.6235485373547691_f32,
        -0.06435071657946274_f32,
        -0.06435071657946266_f32,
        0.6235485373547694_f32,
        -0.06435071657946284_f32,
        -0.0643507165794628_f32,
        -0.06435071657946274_f32,
        -0.06435071657946272_f32,
        -0.06435071657946279_f32,
        -0.06435071657946266_f32,
        -0.06435071657946277_f32,
        -0.06435071657946277_f32,
        -0.06435071657946273_f32,
        -0.06435071657946274_f32,
        -0.0643507165794626_f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.4517556589999482_f32,
        0.15854503551840063_f32,
        0.0_f64 as f32,
        -0.04038515160822202_f32,
        0.0074182263792423875_f32,
        0.39351034269210167_f32,
        -0.45175565899994635_f32,
        0.007418226379244351_f32,
        0.1107416575309343_f32,
        0.08298163094882051_f32,
        0.15854503551839705_f32,
        0.3935103426921022_f32,
        0.0829816309488214_f32,
        -0.45175565899994796_f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.304684750724869_f32,
        0.5112616136591823_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.290480129728998_f32,
        -0.06578701549142804_f32,
        0.304684750724884_f32,
        0.2904801297290076_f32,
        0.0_f64 as f32,
        -0.23889773523344604_f32,
        -0.5112616136592012_f32,
        0.06578701549142545_f32,
        0.23889773523345467_f32,
        0.0_f64 as f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.3017929516615495_f32,
        0.25792362796341184_f32,
        0.0_f64 as f32,
        0.16272340142866204_f32,
        0.09520022653475037_f32,
        0.0_f64 as f32,
        0.3017929516615503_f32,
        0.09520022653475055_f32,
        -0.16272340142866173_f32,
        -0.35312385449816297_f32,
        0.25792362796341295_f32,
        0.0_f64 as f32,
        -0.3531238544981624_f32,
        -0.6035859033230976_f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.40824829046386274_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.4082482904638628_f32,
        -0.4082482904638635_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.40824829046386296_f32,
        0.0_f64 as f32,
        0.4082482904638634_f32,
        0.408248290463863_f32,
        0.0_f64 as f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.1747866975480809_f32,
        0.0812611176717539_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.3675398009862027_f32,
        -0.307882213957909_f32,
        -0.17478669754808135_f32,
        0.3675398009862011_f32,
        0.0_f64 as f32,
        0.4826689115059883_f32,
        -0.08126111767175039_f32,
        0.30788221395790305_f32,
        -0.48266891150598584_f32,
        0.0_f64 as f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.21105601049335784_f32,
        0.18567180916109802_f32,
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.49215859013738733_f32,
        -0.38525013709251915_f32,
        0.21105601049335806_f32,
        -0.49215859013738905_f32,
        0.0_f64 as f32,
        0.17419412659916217_f32,
        -0.18567180916109904_f32,
        0.3852501370925211_f32,
        -0.1741941265991621_f32,
        0.0_f64 as f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.14266084808807264_f32,
        -0.3416446842253372_f32,
        0.0_f64 as f32,
        0.7367497537172237_f32,
        0.24627107722075148_f32,
        -0.08574019035519306_f32,
        -0.14266084808807344_f32,
        0.24627107722075137_f32,
        0.14883399227113567_f32,
        -0.04768680350229251_f32,
        -0.3416446842253373_f32,
        -0.08574019035519267_f32,
        -0.047686803502292804_f32,
        -0.14266084808807242_f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.13813540350758585_f32,
        0.3302282550303788_f32,
        0.0_f64 as f32,
        0.08755115000587084_f32,
        -0.07946706605909573_f32,
        -0.4613374887461511_f32,
        -0.13813540350758294_f32,
        -0.07946706605910261_f32,
        0.49724647109535086_f32,
        0.12538059448563663_f32,
        0.3302282550303805_f32,
        -0.4613374887461554_f32,
        0.12538059448564315_f32,
        -0.13813540350758452_f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        -0.17437602599651067_f32,
        0.0702790691196284_f32,
        0.0_f64 as f32,
        -0.2921026642334881_f32,
        0.3623817333531167_f32,
        0.0_f64 as f32,
        -0.1743760259965108_f32,
        0.36238173335311646_f32,
        0.29210266423348785_f32,
        -0.4326608024727445_f32,
        0.07027906911962818_f32,
        0.0_f64 as f32,
        -0.4326608024727457_f32,
        0.34875205199302267_f32,
    ],
    [
        0.0_f64 as f32,
        0.0_f64 as f32,
        0.11354987314994337_f32,
        -0.07417504595810355_f32,
        0.0_f64 as f32,
        0.19402893032594343_f32,
        -0.435190496523228_f32,
        0.21918684838857466_f32,
        0.11354987314994257_f32,
        -0.4351904965232251_f32,
        0.5550443808910661_f32,
        -0.25468277124066463_f32,
        -0.07417504595810233_f32,
        0.2191868483885728_f32,
        -0.25468277124066413_f32,
        0.1135498731499429_f32,
    ],
];

/// Translation of `AFVIDCT4x4()`.
fn afv_idct_4x4(coeffs: &[f32], pixels: &mut [f32]) {
    for i in 0..16 {
        let mut pixel = 0.0f32;
        for j in 0..16 {
            let cf = coeffs[j];
            let basis = K_4X4_AFV_BASIS[j][i];
            pixel = cf * basis + pixel;
        }
        pixels[i] = pixel;
    }
}

/// Translation of `AFVTransformToPixels<afv_kind>()`.
fn afv_transform_to_pixels(afv_kind: usize, coefficients: &[f32], pixels: &mut [f32], pixels_stride: usize) {
    let mut scratch_space = [0f32; 4 * 8];
    let afv_x = afv_kind & 1;
    let afv_y = afv_kind / 2;
    let mut dcs = [0f32; 3];
    let block00 = coefficients[0];
    let block01 = coefficients[1];
    let block10 = coefficients[8];
    dcs[0] = (block00 + block10 + block01) * 4.0f32;
    dcs[1] = block00 + block10 - block01;
    dcs[2] = block00 - block10;
    // IAFV: (even, even) positions.
    let mut coeff = [0f32; 4 * 4];
    coeff[0] = dcs[0];
    for iy in 0..4 {
        for ix in 0..4 {
            if ix == 0 && iy == 0 {
                continue;
            }
            coeff[iy * 4 + ix] = coefficients[iy * 2 * 8 + ix * 2];
        }
    }
    let mut block = [0f32; 4 * 8];
    afv_idct_4x4(&coeff, &mut block);
    for iy in 0..4 {
        for ix in 0..4 {
            pixels[(iy + afv_y * 4) * pixels_stride + afv_x * 4 + ix] =
                block[(if afv_y == 1 { 3 - iy } else { iy }) * 4 + (if afv_x == 1 { 3 - ix } else { ix })];
        }
    }
    // IDCT4x4 in (odd, even) positions.
    block[0] = dcs[1];
    for iy in 0..4 {
        for ix in 0..4 {
            if ix == 0 && iy == 0 {
                continue;
            }
            block[iy * 4 + ix] = coefficients[iy * 2 * 8 + ix * 2 + 1];
        }
    }
    let off = afv_y * 4 * pixels_stride + if afv_x == 1 { 0 } else { 4 };
    compute_scaled_idct(
        4,
        4,
        &mut block,
        &mut DctTo::new(&mut pixels[off..], pixels_stride),
        &mut scratch_space,
    );
    // IDCT4x8.
    block[0] = dcs[2];
    for iy in 0..4 {
        for ix in 0..8 {
            if ix == 0 && iy == 0 {
                continue;
            }
            block[iy * 8 + ix] = coefficients[(1 + iy * 2) * 8 + ix];
        }
    }
    let off = (if afv_y == 1 { 0 } else { 4 }) * pixels_stride;
    compute_scaled_idct(
        4,
        8,
        &mut block,
        &mut DctTo::new(&mut pixels[off..], pixels_stride),
        &mut scratch_space,
    );
}

/// The IDCT of `coefficients` (a block of the given strategy, also used as
/// scratch) into `pixels`. Translation of `TransformToPixels()`.
pub(crate) fn transform_to_pixels(
    strategy: u8,
    coefficients: &mut [f32],
    pixels: &mut [f32],
    pixels_stride: usize,
    scratch_space: &mut [f32],
) {
    match strategy {
        Type::IDENTITY => {
            let mut dcs = [0f32; 4];
            let block00 = coefficients[0];
            let block01 = coefficients[1];
            let block10 = coefficients[8];
            let block11 = coefficients[9];
            dcs[0] = block00 + block01 + block10 + block11;
            dcs[1] = block00 + block01 - block10 - block11;
            dcs[2] = block00 - block01 + block10 - block11;
            dcs[3] = block00 - block01 - block10 + block11;
            for y in 0..2 {
                for x in 0..2 {
                    let block_dc = dcs[y * 2 + x];
                    let mut residual_sum = 0f32;
                    for iy in 0..4 {
                        for ix in 0..4 {
                            if ix == 0 && iy == 0 {
                                continue;
                            }
                            residual_sum += coefficients[(y + iy * 2) * 8 + x + ix * 2];
                        }
                    }
                    pixels[(4 * y + 1) * pixels_stride + 4 * x + 1] = block_dc - residual_sum * (1.0f32 / 16.0);
                    for iy in 0..4 {
                        for ix in 0..4 {
                            if ix == 1 && iy == 1 {
                                continue;
                            }
                            pixels[(y * 4 + iy) * pixels_stride + x * 4 + ix] = coefficients
                                [(y + iy * 2) * 8 + x + ix * 2]
                                + pixels[(4 * y + 1) * pixels_stride + 4 * x + 1];
                        }
                    }
                    pixels[y * 4 * pixels_stride + x * 4] =
                        coefficients[(y + 2) * 8 + x + 2] + pixels[(4 * y + 1) * pixels_stride + 4 * x + 1];
                }
            }
        }
        Type::DCT8X4 => {
            let mut dcs = [0f32; 2];
            let block0 = coefficients[0];
            let block1 = coefficients[8];
            dcs[0] = block0 + block1;
            dcs[1] = block0 - block1;
            for x in 0..2 {
                let mut block = [0f32; 4 * 8];
                block[0] = dcs[x];
                for iy in 0..4 {
                    for ix in 0..8 {
                        if ix == 0 && iy == 0 {
                            continue;
                        }
                        block[iy * 8 + ix] = coefficients[(x + iy * 2) * 8 + ix];
                    }
                }
                compute_scaled_idct(
                    8,
                    4,
                    &mut block,
                    &mut DctTo::new(&mut pixels[x * 4..], pixels_stride),
                    scratch_space,
                );
            }
        }
        Type::DCT4X8 => {
            let mut dcs = [0f32; 2];
            let block0 = coefficients[0];
            let block1 = coefficients[8];
            dcs[0] = block0 + block1;
            dcs[1] = block0 - block1;
            for y in 0..2 {
                let mut block = [0f32; 4 * 8];
                block[0] = dcs[y];
                for iy in 0..4 {
                    for ix in 0..8 {
                        if ix == 0 && iy == 0 {
                            continue;
                        }
                        block[iy * 8 + ix] = coefficients[(y + iy * 2) * 8 + ix];
                    }
                }
                compute_scaled_idct(
                    4,
                    8,
                    &mut block,
                    &mut DctTo::new(&mut pixels[y * 4 * pixels_stride..], pixels_stride),
                    scratch_space,
                );
            }
        }
        Type::DCT4X4 => {
            let mut dcs = [0f32; 4];
            let block00 = coefficients[0];
            let block01 = coefficients[1];
            let block10 = coefficients[8];
            let block11 = coefficients[9];
            dcs[0] = block00 + block01 + block10 + block11;
            dcs[1] = block00 + block01 - block10 - block11;
            dcs[2] = block00 - block01 + block10 - block11;
            dcs[3] = block00 - block01 - block10 + block11;
            for y in 0..2 {
                for x in 0..2 {
                    let mut block = [0f32; 4 * 4];
                    block[0] = dcs[y * 2 + x];
                    for iy in 0..4 {
                        for ix in 0..4 {
                            if ix == 0 && iy == 0 {
                                continue;
                            }
                            block[iy * 4 + ix] = coefficients[(y + iy * 2) * 8 + x + ix * 2];
                        }
                    }
                    compute_scaled_idct(
                        4,
                        4,
                        &mut block,
                        &mut DctTo::new(&mut pixels[y * 4 * pixels_stride + x * 4..], pixels_stride),
                        scratch_space,
                    );
                }
            }
        }
        Type::DCT2X2 => {
            let mut coeffs = [0f32; K_DCT_BLOCK_SIZE];
            coeffs.copy_from_slice(&coefficients[..K_DCT_BLOCK_SIZE]);
            idct2_top_block(2, &mut coeffs);
            idct2_top_block(4, &mut coeffs);
            idct2_top_block(8, &mut coeffs);
            for y in 0..K_BLOCK_DIM {
                for x in 0..K_BLOCK_DIM {
                    pixels[y * pixels_stride + x] = coeffs[y * K_BLOCK_DIM + x];
                }
            }
        }
        Type::AFV0 => afv_transform_to_pixels(0, coefficients, pixels, pixels_stride),
        Type::AFV1 => afv_transform_to_pixels(1, coefficients, pixels, pixels_stride),
        Type::AFV2 => afv_transform_to_pixels(2, coefficients, pixels, pixels_stride),
        Type::AFV3 => afv_transform_to_pixels(3, coefficients, pixels, pixels_stride),
        _ => {
            // (DCT, DCT16X16, DCT16X8, DCT8X16, ..., DCT256X256: a plain
            // ComputeScaledIDCT<ROWS, COLS>)
            let (rows, cols) = match strategy {
                Type::DCT16X16 => (16, 16),
                Type::DCT16X8 => (16, 8),
                Type::DCT8X16 => (8, 16),
                Type::DCT32X8 => (32, 8),
                Type::DCT8X32 => (8, 32),
                Type::DCT32X16 => (32, 16),
                Type::DCT16X32 => (16, 32),
                Type::DCT32X32 => (32, 32),
                Type::DCT => (8, 8),
                Type::DCT64X32 => (64, 32),
                Type::DCT32X64 => (32, 64),
                Type::DCT64X64 => (64, 64),
                Type::DCT128X64 => (128, 64),
                Type::DCT64X128 => (64, 128),
                Type::DCT128X128 => (128, 128),
                Type::DCT256X128 => (256, 128),
                Type::DCT128X256 => (128, 256),
                Type::DCT256X256 => (256, 256),
                // case Type::kNumValidStrategies: JXL_ABORT("Invalid strategy");
                _ => unreachable!("Invalid strategy"),
            };
            compute_scaled_idct(
                rows,
                cols,
                coefficients,
                &mut DctTo::new(pixels, pixels_stride),
                scratch_space,
            );
        }
    }
}

/// The lowest frequencies of a block of the given strategy, from its DC
/// samples. Translation of `LowestFrequenciesFromDC()`.
pub(crate) fn lowest_frequencies_from_dc(strategy: u8, dc: &[f32], dc_stride: usize, llf: &mut [f32]) {
    // (DCT_ROWS, DCT_COLS, ROWS, COLS, output stride)
    let p = match strategy {
        Type::DCT16X8 => (2 * K_BLOCK_DIM, K_BLOCK_DIM, 2, 1, 2 * K_BLOCK_DIM),
        Type::DCT8X16 => (K_BLOCK_DIM, 2 * K_BLOCK_DIM, 1, 2, 2 * K_BLOCK_DIM),
        Type::DCT16X16 => (2 * K_BLOCK_DIM, 2 * K_BLOCK_DIM, 2, 2, 2 * K_BLOCK_DIM),
        Type::DCT32X8 => (4 * K_BLOCK_DIM, K_BLOCK_DIM, 4, 1, 4 * K_BLOCK_DIM),
        Type::DCT8X32 => (K_BLOCK_DIM, 4 * K_BLOCK_DIM, 1, 4, 4 * K_BLOCK_DIM),
        Type::DCT32X16 => (4 * K_BLOCK_DIM, 2 * K_BLOCK_DIM, 4, 2, 4 * K_BLOCK_DIM),
        Type::DCT16X32 => (2 * K_BLOCK_DIM, 4 * K_BLOCK_DIM, 2, 4, 4 * K_BLOCK_DIM),
        Type::DCT32X32 => (4 * K_BLOCK_DIM, 4 * K_BLOCK_DIM, 4, 4, 4 * K_BLOCK_DIM),
        Type::DCT64X32 => (8 * K_BLOCK_DIM, 4 * K_BLOCK_DIM, 8, 4, 8 * K_BLOCK_DIM),
        Type::DCT32X64 => (4 * K_BLOCK_DIM, 8 * K_BLOCK_DIM, 4, 8, 8 * K_BLOCK_DIM),
        Type::DCT64X64 => (8 * K_BLOCK_DIM, 8 * K_BLOCK_DIM, 8, 8, 8 * K_BLOCK_DIM),
        Type::DCT128X64 => (16 * K_BLOCK_DIM, 8 * K_BLOCK_DIM, 16, 8, 16 * K_BLOCK_DIM),
        Type::DCT64X128 => (8 * K_BLOCK_DIM, 16 * K_BLOCK_DIM, 8, 16, 16 * K_BLOCK_DIM),
        Type::DCT128X128 => (16 * K_BLOCK_DIM, 16 * K_BLOCK_DIM, 16, 16, 16 * K_BLOCK_DIM),
        Type::DCT256X128 => (32 * K_BLOCK_DIM, 16 * K_BLOCK_DIM, 32, 16, 32 * K_BLOCK_DIM),
        Type::DCT128X256 => (16 * K_BLOCK_DIM, 32 * K_BLOCK_DIM, 16, 32, 32 * K_BLOCK_DIM),
        Type::DCT256X256 => (32 * K_BLOCK_DIM, 32 * K_BLOCK_DIM, 32, 32, 32 * K_BLOCK_DIM),
        Type::DCT
        | Type::DCT2X2
        | Type::DCT4X4
        | Type::DCT4X8
        | Type::DCT8X4
        | Type::AFV0
        | Type::AFV1
        | Type::AFV2
        | Type::AFV3
        | Type::IDENTITY => {
            llf[0] = dc[0];
            return;
        }
        // case Type::kNumValidStrategies: JXL_ABORT("Invalid strategy");
        _ => unreachable!("Invalid strategy"),
    };
    reinterpreting_dct(p.0, p.1, p.2, p.3, dc, dc_stride, llf, p.4);
}
