// Rust translation of lib/jxl/quant_weights.h and lib/jxl/quant_weights.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Quantization weights (dequantization matrices) of the VarDCT mode.

use super::ac_strategy::{coefficient_layout, AcStrategy};
use super::base::{jxl_failure, Status, StatusCode, K_BLOCK_DIM, K_DCT_BLOCK_SIZE};
use super::dec_bit_reader::BitReader;
use super::dec_modular::ModularFrameDecoder;
use super::fields::f16_coder_read;
use super::math::{fast_powf, hwy_convert_to_i32};

const K_NUM_PREDEFINED_TABLES: usize = 1;
const K_LOG2_NUM_QUANT_MODES: usize = 3;

const K_LOG2_MAX_DISTANCE_BANDS: usize = 4;
const K_MAX_DISTANCE_BANDS: usize = 1 + (1 << K_LOG2_MAX_DISTANCE_BANDS);

/// Translation of `DctQuantWeightParams`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DctQuantWeightParams {
    pub num_distance_bands: usize,
    pub distance_bands: [[f32; K_MAX_DISTANCE_BANDS]; 3],
}

impl DctQuantWeightParams {
    fn new<const N: usize>(dist_bands: [[f64; N]; 3]) -> Self {
        let mut p = DctQuantWeightParams {
            num_distance_bands: N,
            distance_bands: [[0.0; K_MAX_DISTANCE_BANDS]; 3],
        };
        for c in 0..3 {
            for i in 0..N {
                p.distance_bands[c][i] = dist_bands[c][i] as f32;
            }
        }
        p
    }
}

/// Translation of `QuantEncodingInternal::Mode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum QuantMode {
    #[default]
    Library = 0,
    Id = 1,
    Dct2 = 2,
    Dct4 = 3,
    Dct4x8 = 4,
    Afv = 5,
    Dct = 6,
    Raw = 7,
}

/// Translation of `QuantEncoding` (the union members are separate fields).
#[derive(Clone, Debug, Default)]
pub(crate) struct QuantEncoding {
    pub mode: QuantMode,

    // Weights for DCT4+ tables.
    pub dct_params: DctQuantWeightParams,

    // Weights for identity.
    pub idweights: [[f32; 3]; 3],
    // Weights for DCT2.
    pub dct2weights: [[f32; 6]; 3],
    // Extra multipliers for coefficients 01/10 and 11 for DCT4 and AFV.
    pub dct4multipliers: [[f32; 2]; 3],
    // Weights for AFV. {0, 1} are used directly for coefficients (0, 1) and (1,
    // 0);  {2, 3, 4} are used directly corner DC, (1,0) - (0,1) and (0, 1) +
    // (1, 0) - (0, 0) inside the AFV block. Values from 5 to 8 are interpolated
    // as in GetQuantWeights for DC and are used for other coefficients.
    pub afv_weights: [[f32; 9]; 3],
    // Extra multipliers for coefficients 01 or 10 for DCT4X8 and DCT8X4.
    pub dct4x8multipliers: [f32; 3],

    // Only used in kQuantModeRAW mode: explicit quantization table (like in
    // JPEG).
    pub qraw_qtable: Option<Vec<i32>>,
    pub qraw_qtable_den: f32,

    // Weights for 4x4 sub-block in AFV.
    pub dct_params_afv_4x4: DctQuantWeightParams,

    // Which predefined table to use. Only used if mode is kQuantModeLibrary.
    pub predefined: u8,
}

impl QuantEncoding {
    fn library(predefined: u8) -> Self {
        QuantEncoding {
            mode: QuantMode::Library,
            predefined,
            qraw_qtable_den: 1.0f32 / (8 * 255) as f32,
            ..Default::default()
        }
    }
    fn base(mode: QuantMode) -> Self {
        QuantEncoding {
            mode,
            qraw_qtable_den: 1.0f32 / (8 * 255) as f32,
            ..Default::default()
        }
    }
}

// Let's try to keep these 2**N for possible future simplicity.
pub(crate) const K_INV_DC_QUANT: [f32; 3] = [4096.0, 512.0, 256.0];

pub(crate) const K_DC_QUANT: [f32; 3] = [
    1.0f32 / K_INV_DC_QUANT[0],
    1.0f32 / K_INV_DC_QUANT[1],
    1.0f32 / K_INV_DC_QUANT[2],
];

/// `DequantMatrices::QuantTable`.
pub(crate) mod quant_table {
    pub(crate) const DCT: usize = 0;
    pub(crate) const IDENTITY: usize = 1;
    pub(crate) const DCT2X2: usize = 2;
    pub(crate) const DCT4X4: usize = 3;
    pub(crate) const DCT16X16: usize = 4;
    pub(crate) const DCT32X32: usize = 5;
    // DCT16X8
    pub(crate) const DCT8X16: usize = 6;
    // DCT32X8
    pub(crate) const DCT8X32: usize = 7;
    // DCT32X16
    pub(crate) const DCT16X32: usize = 8;
    pub(crate) const DCT4X8: usize = 9;
    // DCT8X4
    pub(crate) const AFV0: usize = 10;
    // AFV1
    // AFV2
    // AFV3
    pub(crate) const DCT64X64: usize = 11;
    // DCT64X32,
    pub(crate) const DCT32X64: usize = 12;
    pub(crate) const DCT128X128: usize = 13;
    // DCT128X64,
    pub(crate) const DCT64X128: usize = 14;
    pub(crate) const DCT256X256: usize = 15;
    // DCT256X128,
    pub(crate) const DCT128X256: usize = 16;
    pub(crate) const K_NUM: usize = 17;
}

use quant_table as qt;

const K_QUANT_TABLE: [usize; 27] = [
    qt::DCT,
    qt::IDENTITY,
    qt::DCT2X2,
    qt::DCT4X4,
    qt::DCT16X16,
    qt::DCT32X32,
    qt::DCT8X16,
    qt::DCT8X16,
    qt::DCT8X32,
    qt::DCT8X32,
    qt::DCT16X32,
    qt::DCT16X32,
    qt::DCT4X8,
    qt::DCT4X8,
    qt::AFV0,
    qt::AFV0,
    qt::AFV0,
    qt::AFV0,
    qt::DCT64X64,
    qt::DCT32X64,
    qt::DCT32X64,
    qt::DCT128X128,
    qt::DCT64X128,
    qt::DCT64X128,
    qt::DCT256X256,
    qt::DCT128X256,
    qt::DCT128X256,
];

pub(crate) const REQUIRED_SIZE_X: [usize; 17] = [1, 1, 1, 1, 2, 4, 1, 1, 2, 1, 1, 8, 4, 16, 8, 32, 16];
pub(crate) const REQUIRED_SIZE_Y: [usize; 17] = [1, 1, 1, 1, 2, 4, 2, 4, 4, 1, 1, 8, 8, 16, 16, 32, 32];
const REQUIRED_SIZE: [usize; 17] = [1, 1, 1, 1, 4, 16, 2, 4, 8, 1, 1, 64, 32, 256, 128, 1024, 512];
const fn array_sum(a: &[usize; 17]) -> usize {
    let mut s = 0;
    let mut i = 0;
    while i < 17 {
        s += a[i];
        i += 1;
    }
    s
}
const K_TOTAL_TABLE_SIZE: usize = array_sum(&REQUIRED_SIZE) * K_DCT_BLOCK_SIZE * 3;

static K_ALMOST_ZERO: f32 = 1e-8f32;

// (dct_scales.h)
const K_SQRT2: f32 = 1.41421356237f32;

// kQuantWeights[N * N * c + N * y + x] is the relative weight of the (x, y)
// coefficient in component c. Higher weights correspond to finer quantization
// intervals and more bits spent in encoding.

/// Translation of `GetQuantWeightsDCT2()`.
fn get_quant_weights_dct2(dct2weights: &[[f32; 6]; 3], weights: &mut [f32]) {
    for c in 0..3 {
        let start = c * 64;
        weights[start] = 0xBAD as f32;
        weights[start + 1] = dct2weights[c][0];
        weights[start + 8] = dct2weights[c][0];
        weights[start + 9] = dct2weights[c][1];
        for y in 0..2 {
            for x in 0..2 {
                weights[start + y * 8 + x + 2] = dct2weights[c][2];
                weights[start + (y + 2) * 8 + x] = dct2weights[c][2];
            }
        }
        for y in 0..2 {
            for x in 0..2 {
                weights[start + (y + 2) * 8 + x + 2] = dct2weights[c][3];
            }
        }
        for y in 0..4 {
            for x in 0..4 {
                weights[start + y * 8 + x + 4] = dct2weights[c][4];
                weights[start + (y + 4) * 8 + x] = dct2weights[c][4];
            }
        }
        for y in 0..4 {
            for x in 0..4 {
                weights[start + (y + 4) * 8 + x + 4] = dct2weights[c][5];
            }
        }
    }
}

/// Translation of `GetQuantWeightsIdentity()`.
fn get_quant_weights_identity(idweights: &[[f32; 3]; 3], weights: &mut [f32]) {
    for c in 0..3 {
        for i in 0..64 {
            weights[64 * c + i] = idweights[c][0];
        }
        weights[64 * c + 1] = idweights[c][1];
        weights[64 * c + 8] = idweights[c][1];
        weights[64 * c + 9] = idweights[c][2];
    }
}

/// Translation of `Interpolate()`.
fn interpolate(pos: f32, max: f32, array: &[f32], len: usize) -> f32 {
    let scaled_pos = pos * (len - 1) as f32 / max;
    let idx = scaled_pos as usize;
    debug_assert!(idx + 1 < len);
    let a = array[idx];
    let b = array[idx + 1];
    a * fast_powf(b / a, scaled_pos - idx as f32)
}

/// Translation of `Mult()`.
fn mult(v: f32) -> f32 {
    if v > 0.0f32 {
        return 1.0f32 + v;
    }
    1.0f32 / (1.0f32 - v)
}

/// Translation of `InterpolateVec()` (one lane).
fn interpolate_vec(scaled_pos: f32, array: &[f32]) -> f32 {
    let idx = hwy_convert_to_i32(scaled_pos);

    let frac = scaled_pos - idx as f32;

    // TODO(veluca): in theory, this could be done with 8 TableLookupBytes, but
    // it's probably slower.
    let a = array[idx as usize];
    let b = array[idx as usize + 1];

    a * fast_powf(b / a, frac)
}

// Computes quant weights for a COLS*ROWS-sized transform, using num_bands
// eccentricity bands and num_ebands eccentricity bands. If print_mode is 1,
// prints the resulting matrix; if print_mode is 2, prints the matrix in a
// format suitable for a 3d plot with gnuplot.
/// Translation of `GetQuantWeights()`.
fn get_quant_weights(
    rows: usize,
    cols: usize,
    distance_bands: &[[f32; K_MAX_DISTANCE_BANDS]; 3],
    num_bands: usize,
    out: &mut [f32],
) -> Status {
    for c in 0..3 {
        let mut bands = [0f32; K_MAX_DISTANCE_BANDS];
        bands[0] = distance_bands[c][0];
        if bands[0] < K_ALMOST_ZERO {
            return jxl_failure!("Invalid distance bands");
        }
        for i in 1..num_bands {
            bands[i] = bands[i - 1] * mult(distance_bands[c][i]);
            if bands[i] < K_ALMOST_ZERO {
                return jxl_failure!("Invalid distance bands");
            }
        }
        let scale = (num_bands - 1) as f32 / (K_SQRT2 + 1e-6f32);
        let rcpcol = scale / (cols - 1) as f32;
        let rcprow = scale / (rows - 1) as f32;
        for y in 0..rows as u32 {
            let dy = y as f32 * rcprow;
            let dy2 = dy * dy;
            for x in 0..cols as u32 {
                let dx = (x as f32 + 0.0f32) * rcpcol;
                let scaled_distance = (dx * dx + dy2).sqrt();
                let weight = if num_bands == 1 {
                    bands[0]
                } else {
                    interpolate_vec(scaled_distance, &bands)
                };
                out[c * cols * rows + y as usize * cols + x as usize] = weight;
            }
        }
    }
    Ok(())
}

// TODO(veluca): SIMD-fy. With 256x256, this is actually slow.
/// Translation of `ComputeQuantTable()`.
fn compute_quant_table(
    encoding: &QuantEncoding,
    table_storage: &mut [f32],
    kind: usize,
    pos: &mut usize,
) -> Status {
    const N: usize = K_BLOCK_DIM;
    let wrows = 8 * REQUIRED_SIZE_X[kind];
    let wcols = 8 * REQUIRED_SIZE_Y[kind];
    let num = wrows * wcols;

    let mut weights = vec![0f32; 3 * num];

    match encoding.mode {
        QuantMode::Library => {
            // Library and copy quant encoding should get replaced by the actual
            // parameters by the caller.
            // JXL_ASSERT(false);
            return Err(StatusCode::GenericError);
        }
        QuantMode::Id => {
            if num != K_DCT_BLOCK_SIZE {
                return Err(StatusCode::GenericError);
            }
            get_quant_weights_identity(&encoding.idweights, &mut weights);
        }
        QuantMode::Dct2 => {
            if num != K_DCT_BLOCK_SIZE {
                return Err(StatusCode::GenericError);
            }
            get_quant_weights_dct2(&encoding.dct2weights, &mut weights);
        }
        QuantMode::Dct4 => {
            if num != K_DCT_BLOCK_SIZE {
                return Err(StatusCode::GenericError);
            }
            let mut weights4x4 = [0f32; 3 * 4 * 4];
            // Always use 4x4 GetQuantWeights for DCT4 quantization tables.
            get_quant_weights(
                4,
                4,
                &encoding.dct_params.distance_bands,
                encoding.dct_params.num_distance_bands,
                &mut weights4x4,
            )?;
            for c in 0..3 {
                for y in 0..K_BLOCK_DIM {
                    for x in 0..K_BLOCK_DIM {
                        weights[c * num + y * K_BLOCK_DIM + x] = weights4x4[c * 16 + (y / 2) * 4 + (x / 2)];
                    }
                }
                weights[c * num + 1] /= encoding.dct4multipliers[c][0];
                weights[c * num + N] /= encoding.dct4multipliers[c][0];
                weights[c * num + N + 1] /= encoding.dct4multipliers[c][1];
            }
        }
        QuantMode::Dct4x8 => {
            if num != K_DCT_BLOCK_SIZE {
                return Err(StatusCode::GenericError);
            }
            let mut weights4x8 = [0f32; 3 * 4 * 8];
            // Always use 4x8 GetQuantWeights for DCT4X8 quantization tables.
            get_quant_weights(
                4,
                8,
                &encoding.dct_params.distance_bands,
                encoding.dct_params.num_distance_bands,
                &mut weights4x8,
            )?;
            for c in 0..3 {
                for y in 0..K_BLOCK_DIM {
                    for x in 0..K_BLOCK_DIM {
                        weights[c * num + y * K_BLOCK_DIM + x] = weights4x8[c * 32 + (y / 2) * 8 + x];
                    }
                }
                weights[c * num + N] /= encoding.dct4x8multipliers[c];
            }
        }
        QuantMode::Dct => {
            get_quant_weights(
                wrows,
                wcols,
                &encoding.dct_params.distance_bands,
                encoding.dct_params.num_distance_bands,
                &mut weights,
            )?;
        }
        QuantMode::Raw => {
            let Some(qtable) = encoding.qraw_qtable.as_ref().filter(|q| q.len() == 3 * num) else {
                return jxl_failure!("Invalid table encoding");
            };
            for i in 0..3 * num {
                weights[i] = 1.0f32 / (encoding.qraw_qtable_den * qtable[i] as f32);
            }
        }
        QuantMode::Afv => {
            const BAD: f64 = 0xBAD as f64;
            const K_FREQS: [f64; 16] = [
                BAD,
                BAD,
                0.8517778890324296,
                5.37778436506804,
                BAD,
                BAD,
                4.734747904497923,
                5.449245381693219,
                1.6598270267479331,
                4.0,
                7.275749096817861,
                10.423227632456525,
                2.662932286148962,
                7.630657783650829,
                8.962388608184032,
                12.97166202570235,
            ];

            let mut weights4x8 = [0f32; 3 * 4 * 8];
            get_quant_weights(
                4,
                8,
                &encoding.dct_params.distance_bands,
                encoding.dct_params.num_distance_bands,
                &mut weights4x8,
            )?;
            let mut weights4x4 = [0f32; 3 * 4 * 4];
            get_quant_weights(
                4,
                4,
                &encoding.dct_params_afv_4x4.distance_bands,
                encoding.dct_params_afv_4x4.num_distance_bands,
                &mut weights4x4,
            )?;

            let lo: f32 = 0.8517778890324296f64 as f32;
            let hi: f32 = 12.97166202570235f32 - lo + 1e-6f32;
            for c in 0..3 {
                let mut bands = [0f32; 4];
                bands[0] = encoding.afv_weights[c][5];
                if bands[0] < K_ALMOST_ZERO {
                    return jxl_failure!("Invalid AFV bands");
                }
                for i in 1..4 {
                    bands[i] = bands[i - 1] * mult(encoding.afv_weights[c][i + 5]);
                    if bands[i] < K_ALMOST_ZERO {
                        return jxl_failure!("Invalid AFV bands");
                    }
                }
                let start = c * 64;
                let mut set_weight = |x: usize, y: usize, val: f32| {
                    weights[start + y * 8 + x] = val;
                };
                set_weight(0, 0, 1.0); // Not used, but causes MSAN error otherwise.
                // Weights for (0, 1) and (1, 0).
                set_weight(0, 1, encoding.afv_weights[c][0]);
                set_weight(1, 0, encoding.afv_weights[c][1]);
                // AFV special weights for 3-pixel corner.
                set_weight(0, 2, encoding.afv_weights[c][2]);
                set_weight(2, 0, encoding.afv_weights[c][3]);
                set_weight(2, 2, encoding.afv_weights[c][4]);

                // All other AFV weights.
                for y in 0..4 {
                    for x in 0..4 {
                        if x < 2 && y < 2 {
                            continue;
                        }
                        let val = interpolate(K_FREQS[y * 4 + x] as f32 - lo, hi, &bands, 4);
                        set_weight(2 * x, 2 * y, val);
                    }
                }

                // Put 4x8 weights in odd rows, except (1, 0).
                for y in 0..K_BLOCK_DIM / 2 {
                    for x in 0..K_BLOCK_DIM {
                        if x == 0 && y == 0 {
                            continue;
                        }
                        weights[c * num + (2 * y + 1) * K_BLOCK_DIM + x] = weights4x8[c * 32 + y * 8 + x];
                    }
                }
                // Put 4x4 weights in even rows / odd columns, except (0, 1).
                for y in 0..K_BLOCK_DIM / 2 {
                    for x in 0..K_BLOCK_DIM / 2 {
                        if x == 0 && y == 0 {
                            continue;
                        }
                        weights[c * num + (2 * y) * K_BLOCK_DIM + 2 * x + 1] = weights4x4[c * 16 + y * 4 + x];
                    }
                }
            }
        }
    }
    let prev_pos = *pos;
    for i in 0..num * 3 {
        let inv_val = weights[i];
        if inv_val >= 1.0f32 / K_ALMOST_ZERO || inv_val < K_ALMOST_ZERO {
            return jxl_failure!("Invalid quantization table");
        }
        let val = 1.0f32 / inv_val;
        table_storage[*pos + i] = val;
        table_storage[K_TOTAL_TABLE_SIZE + *pos + i] = inv_val;
    }
    *pos += 3 * num;

    // Ensure that the lowest frequencies have a 0 inverse table.
    // This does not affect en/decoding, but allows AC strategy selection to be
    // slightly simpler.
    let mut xs = REQUIRED_SIZE_X[kind];
    let mut ys = REQUIRED_SIZE_Y[kind];
    coefficient_layout(&mut ys, &mut xs);
    for c in 0..3 {
        for y in 0..ys {
            for x in 0..xs {
                table_storage
                    [K_TOTAL_TABLE_SIZE + prev_pos + c * ys * xs * K_DCT_BLOCK_SIZE + y * K_BLOCK_DIM * xs + x] = 0.0;
            }
        }
    }
    Ok(())
}

/// Translation of `DecodeDctParams()`.
fn decode_dct_params(br: &mut BitReader<'_>, params: &mut DctQuantWeightParams) -> Status {
    params.num_distance_bands = br.read_fixed_bits::<K_LOG2_MAX_DISTANCE_BANDS>() as usize + 1;
    for c in 0..3 {
        for i in 0..params.num_distance_bands {
            f16_coder_read(br, &mut params.distance_bands[c][i])?;
        }
        if params.distance_bands[c][0] < K_ALMOST_ZERO {
            return jxl_failure!("Distance band seed is too small");
        }
        params.distance_bands[c][0] *= 64.0f32;
    }
    Ok(())
}

/// Translation of the file-local `Decode()`.
fn decode_encoding(
    br: &mut BitReader<'_>,
    encoding: &mut QuantEncoding,
    required_size_x: usize,
    required_size_y: usize,
    idx: usize,
    modular_frame_decoder: Option<&ModularFrameDecoder>,
) -> Status {
    let required_size = required_size_x * required_size_y;
    let required_size_x = required_size_x * K_BLOCK_DIM;
    let required_size_y = required_size_y * K_BLOCK_DIM;
    let mode = br.read_fixed_bits::<K_LOG2_NUM_QUANT_MODES>() as u32;
    let new_mode = match mode {
        0 => {
            // (kCeilLog2NumPredefinedTables is 0: no bits)
            encoding.predefined = 0;
            if encoding.predefined as usize >= K_NUM_PREDEFINED_TABLES {
                return jxl_failure!("Invalid predefined table");
            }
            QuantMode::Library
        }
        1 => {
            if required_size != 1 {
                return jxl_failure!("Invalid mode");
            }
            for c in 0..3 {
                for i in 0..3 {
                    f16_coder_read(br, &mut encoding.idweights[c][i])?;
                    if encoding.idweights[c][i].abs() < K_ALMOST_ZERO {
                        return jxl_failure!("ID Quantizer is too small");
                    }
                    encoding.idweights[c][i] *= 64.0;
                }
            }
            QuantMode::Id
        }
        2 => {
            if required_size != 1 {
                return jxl_failure!("Invalid mode");
            }
            for c in 0..3 {
                for i in 0..6 {
                    f16_coder_read(br, &mut encoding.dct2weights[c][i])?;
                    if encoding.dct2weights[c][i].abs() < K_ALMOST_ZERO {
                        return jxl_failure!("Quantizer is too small");
                    }
                    encoding.dct2weights[c][i] *= 64.0;
                }
            }
            QuantMode::Dct2
        }
        4 => {
            if required_size != 1 {
                return jxl_failure!("Invalid mode");
            }
            for c in 0..3 {
                f16_coder_read(br, &mut encoding.dct4x8multipliers[c])?;
                if encoding.dct4x8multipliers[c].abs() < K_ALMOST_ZERO {
                    return jxl_failure!("DCT4X8 multiplier is too small");
                }
            }
            decode_dct_params(br, &mut encoding.dct_params)?;
            QuantMode::Dct4x8
        }
        3 => {
            if required_size != 1 {
                return jxl_failure!("Invalid mode");
            }
            for c in 0..3 {
                for i in 0..2 {
                    f16_coder_read(br, &mut encoding.dct4multipliers[c][i])?;
                    if encoding.dct4multipliers[c][i].abs() < K_ALMOST_ZERO {
                        return jxl_failure!("DCT4 multiplier is too small");
                    }
                }
            }
            decode_dct_params(br, &mut encoding.dct_params)?;
            QuantMode::Dct4
        }
        5 => {
            if required_size != 1 {
                return jxl_failure!("Invalid mode");
            }
            for c in 0..3 {
                for i in 0..9 {
                    f16_coder_read(br, &mut encoding.afv_weights[c][i])?;
                }
                for i in 0..6 {
                    encoding.afv_weights[c][i] *= 64.0;
                }
            }
            decode_dct_params(br, &mut encoding.dct_params)?;
            decode_dct_params(br, &mut encoding.dct_params_afv_4x4)?;
            QuantMode::Afv
        }
        6 => {
            decode_dct_params(br, &mut encoding.dct_params)?;
            QuantMode::Dct
        }
        _ => {
            // (7: kQuantModeRAW)
            // Set mode early, to avoid mem-leak.
            encoding.mode = QuantMode::Raw;
            ModularFrameDecoder::decode_quant_table(
                required_size_x,
                required_size_y,
                br,
                encoding,
                idx,
                modular_frame_decoder,
            )?;
            QuantMode::Raw
        }
    };
    encoding.mode = new_mode;
    Ok(())
}

/// Translation of `DequantMatrices`.
#[derive(Clone, Debug)]
pub(crate) struct DequantMatrices {
    computed_mask: u32,
    // kTotalTableSize entries followed by kTotalTableSize for inv_table
    table_storage: Vec<f32>,
    dc_quant: [f32; 3],
    inv_dc_quant: [f32; 3],
    table_offsets: [usize; AcStrategy::K_NUM_VALID_STRATEGIES as usize * 3],
    encodings: Vec<QuantEncoding>,
}

impl Default for DequantMatrices {
    fn default() -> Self {
        Self::new()
    }
}

impl DequantMatrices {
    /// Translation of `DequantMatrices()`.
    pub(crate) fn new() -> Self {
        let mut table_offsets = [0usize; AcStrategy::K_NUM_VALID_STRATEGIES as usize * 3];
        let mut pos = 0usize;
        let mut offsets = [0usize; qt::K_NUM * 3];
        for i in 0..qt::K_NUM {
            let num = REQUIRED_SIZE[i] * K_DCT_BLOCK_SIZE;
            for c in 0..3 {
                offsets[3 * i + c] = pos + c * num;
            }
            pos += 3 * num;
        }
        for i in 0..AcStrategy::K_NUM_VALID_STRATEGIES as usize {
            for c in 0..3 {
                table_offsets[i * 3 + c] = offsets[K_QUANT_TABLE[i] * 3 + c];
            }
        }
        DequantMatrices {
            computed_mask: 0,
            table_storage: Vec::new(),
            dc_quant: K_DC_QUANT,
            inv_dc_quant: K_INV_DC_QUANT,
            table_offsets,
            encodings: vec![QuantEncoding::library(0); qt::K_NUM],
        }
    }

    /// Returns aligned memory. Translation of `Matrix()`.
    #[inline]
    pub(crate) fn matrix(&self, quant_kind: usize, c: usize) -> &[f32] {
        debug_assert!((1 << quant_kind) & self.computed_mask != 0);
        &self.table_storage[self.table_offsets[quant_kind * 3 + c]..]
    }

    /// Translation of `InvMatrix()`.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn inv_matrix(&self, quant_kind: usize, c: usize) -> &[f32] {
        &self.table_storage[K_TOTAL_TABLE_SIZE + self.table_offsets[quant_kind * 3 + c]..]
    }

    // DC quants are used in modular mode for XYB multipliers.
    #[inline]
    pub(crate) fn dc_quant(&self, c: usize) -> f32 {
        self.dc_quant[c]
    }
    #[inline]
    pub(crate) fn dc_quants(&self) -> &[f32; 3] {
        &self.dc_quant
    }

    #[inline]
    pub(crate) fn inv_dc_quant(&self, c: usize) -> f32 {
        self.inv_dc_quant[c]
    }

    #[allow(dead_code)]
    pub(crate) fn encodings(&self) -> &Vec<QuantEncoding> {
        &self.encodings
    }

    /// Translation of `DequantMatrices::Decode()`.
    pub(crate) fn decode(
        &mut self,
        br: &mut BitReader<'_>,
        modular_frame_decoder: Option<&ModularFrameDecoder>,
    ) -> Status {
        let all_default = br.read_bits(1) != 0;
        let num_tables = if all_default { 0 } else { qt::K_NUM };
        self.encodings.clear();
        self.encodings.resize(qt::K_NUM, QuantEncoding::library(0));
        for i in 0..num_tables {
            decode_encoding(
                br,
                &mut self.encodings[i],
                REQUIRED_SIZE_X[i % qt::K_NUM],
                REQUIRED_SIZE_Y[i % qt::K_NUM],
                i,
                modular_frame_decoder,
            )?;
        }
        self.computed_mask = 0;
        Ok(())
    }

    /// Translation of `DequantMatrices::DecodeDC()`.
    pub(crate) fn decode_dc(&mut self, br: &mut BitReader<'_>) -> Status {
        let all_default = br.read_bits(1) != 0;
        if !br.all_reads_within_bounds() {
            return jxl_failure!("EOS during DecodeDC");
        }
        if !all_default {
            for c in 0..3 {
                f16_coder_read(br, &mut self.dc_quant[c])?;
                self.dc_quant[c] *= 1.0f32 / 128.0f32;
                // Negative values and nearly zero are invalid values.
                if self.dc_quant[c] < K_ALMOST_ZERO {
                    return jxl_failure!("Invalid dc_quant: coefficient is too small.");
                }
                self.inv_dc_quant[c] = 1.0f32 / self.dc_quant[c];
            }
        }
        Ok(())
    }

    /// Translation of `DequantMatrices::EnsureComputed()`.
    pub(crate) fn ensure_computed(&mut self, acs_mask: u32) -> Status {
        if self.table_storage.is_empty() {
            if self.table_storage.try_reserve_exact(2 * K_TOTAL_TABLE_SIZE).is_err() {
                return jxl_failure!("out of memory");
            }
            self.table_storage.resize(2 * K_TOTAL_TABLE_SIZE, 0.0);
        }
        let mut offsets = [0usize; qt::K_NUM * 3 + 1];
        let mut pos = 0usize;
        for i in 0..qt::K_NUM {
            let num = REQUIRED_SIZE[i] * K_DCT_BLOCK_SIZE;
            for c in 0..3 {
                offsets[3 * i + c] = pos + c * num;
            }
            pos += 3 * num;
        }
        offsets[qt::K_NUM * 3] = pos;
        debug_assert!(pos == K_TOTAL_TABLE_SIZE);
        let mut kind_mask: u32 = 0;
        for i in 0..AcStrategy::K_NUM_VALID_STRATEGIES as usize {
            if acs_mask & (1u32 << i) != 0 {
                kind_mask |= 1u32 << K_QUANT_TABLE[i];
            }
        }
        let mut computed_kind_mask: u32 = 0;
        for i in 0..AcStrategy::K_NUM_VALID_STRATEGIES as usize {
            if self.computed_mask & (1u32 << i) != 0 {
                computed_kind_mask |= 1u32 << K_QUANT_TABLE[i];
            }
        }
        for table in 0..qt::K_NUM {
            if (1 << table) & computed_kind_mask != 0 {
                continue;
            }
            if (1 << table) & !kind_mask != 0 {
                continue;
            }
            let mut pos = offsets[table * 3];
            if self.encodings[table].mode == QuantMode::Library {
                let library = library_entry(table);
                // JXL_CHECK
                compute_quant_table(&library, &mut self.table_storage, table, &mut pos)?;
            } else {
                let enc = self.encodings[table].clone();
                compute_quant_table(&enc, &mut self.table_storage, table, &mut pos)?;
            }
            debug_assert!(pos == offsets[table * 3 + 3]);
        }
        self.computed_mask |= acs_mask;
        Ok(())
    }
}

/// `V()`: the library values are written as doubles and stored as floats.
fn dct_enc<const N: usize>(bands: [[f64; N]; 3]) -> QuantEncoding {
    let mut e = QuantEncoding::base(QuantMode::Dct);
    e.dct_params = DctQuantWeightParams::new(bands);
    e
}

fn dct4x8_params() -> DctQuantWeightParams {
    DctQuantWeightParams::new([
        [2198.050556016380522, -0.96269623020744692, -0.76194253026666783, -0.6551140670773547],
        [764.3655248643528689, -0.92630200888366945, -0.9675229603596517, -0.27845290869168118],
        [527.107573587542228, -1.4594385811273854, -1.450082094097871593, -1.5843722511996204],
    ])
}

fn dct4x4_params() -> DctQuantWeightParams {
    DctQuantWeightParams::new([[2200.0, 0.0, 0.0, 0.0], [392.0, 0.0, 0.0, 0.0], [112.0, -0.25, -0.25, -0.5]])
}

fn big_dct(a: f64, b: f64, c: f64) -> QuantEncoding {
    dct_enc([
        [a, -1.025, -0.78, -0.65012, -0.19041574084286472, -0.20819395464, -0.421064, -0.32733845535848671],
        [
            b,
            -0.3041958212306401,
            -0.3633036457487539,
            -0.35660379990111464,
            -0.3443074455424403,
            -0.33699592683512467,
            -0.30180866526242109,
            -0.27321683125358037,
        ],
        [c, -1.2, -1.2, -0.8, -0.7, -0.7, -0.4, -0.5],
    ])
}

/// The entries of `DequantMatrices::Library()` (`LibraryInit()` with
/// `DequantMatricesLibraryDef`).
fn library_entry(table: usize) -> QuantEncoding {
    match table {
        // DCT8
        qt::DCT => dct_enc([
            [3150.0, 0.0, -0.4, -0.4, -0.4, -2.0],
            [560.0, 0.0, -0.3, -0.3, -0.3, -0.3],
            [512.0, -2.0, -1.0, 0.0, -1.0, -2.0],
        ]),
        // Identity
        qt::IDENTITY => {
            let mut e = QuantEncoding::base(QuantMode::Id);
            e.idweights = [[280.0, 3160.0, 3160.0], [60.0, 864.0, 864.0], [18.0, 200.0, 200.0]];
            e
        }
        // DCT2
        qt::DCT2X2 => {
            let mut e = QuantEncoding::base(QuantMode::Dct2);
            e.dct2weights = [
                [3840.0, 2560.0, 1280.0, 640.0, 480.0, 300.0],
                [960.0, 640.0, 320.0, 180.0, 140.0, 120.0],
                [640.0, 320.0, 128.0, 64.0, 32.0, 16.0],
            ];
            e
        }
        // DCT4 (quant_kind 3)
        qt::DCT4X4 => {
            let mut e = QuantEncoding::base(QuantMode::Dct4);
            e.dct_params = dct4x4_params();
            /* kMul */
            e.dct4multipliers = [[1.0, 1.0], [1.0, 1.0], [1.0, 1.0]];
            e
        }
        // DCT16
        qt::DCT16X16 => dct_enc([
            [
                8996.8725711814115328,
                -1.3000777393353804,
                -0.49424529824571225,
                -0.439093774457103443,
                -0.6350101832695744,
                -0.90177264050827612,
                -1.6162099239887414,
            ],
            [
                3191.48366296844234752,
                -0.67424582104194355,
                -0.80745813428471001,
                -0.44925837484843441,
                -0.35865440981033403,
                -0.31322389111877305,
                -0.37615025315725483,
            ],
            [
                1157.50408145487200256,
                -2.0531423165804414,
                -1.4,
                -0.50687130033378396,
                -0.42708730624733904,
                -1.4856834539296244,
                -4.9209142884401604,
            ],
        ]),
        // DCT32
        qt::DCT32X32 => dct_enc([
            [15718.40830982518931456, -1.025, -0.98, -0.9012, -0.4, -0.48819395464, -0.421064, -0.27],
            [
                7305.7636810695983104,
                -0.8041958212306401,
                -0.7633036457487539,
                -0.55660379990111464,
                -0.49785304658857626,
                -0.43699592683512467,
                -0.40180866526242109,
                -0.27321683125358037,
            ],
            [
                3803.53173721215041536,
                -3.060733579805728,
                -2.0413270132490346,
                -2.0235650159727417,
                -0.5495389509954993,
                -0.4,
                -0.4,
                -0.3,
            ],
        ]),
        // DCT16X8
        qt::DCT8X16 => dct_enc([
            [7240.7734393502, -0.7, -0.7, -0.2, -0.2, -0.2, -0.5],
            [1448.15468787004, -0.5, -0.5, -0.5, -0.2, -0.2, -0.2],
            [506.854140754517, -1.4, -0.2, -0.5, -0.5, -1.5, -3.6],
        ]),
        // DCT32X8
        qt::DCT8X32 => dct_enc([
            [
                16283.2494710648897,
                -1.7812845336559429,
                -1.6309059012653515,
                -1.0382179034313539,
                -0.85,
                -0.7,
                -0.9,
                -1.2360638576849587,
            ],
            [
                5089.15750884921511936,
                -0.320049391452786891,
                -0.35362849922161446,
                -0.30340000000000003,
                -0.61,
                -0.5,
                -0.5,
                -0.6,
            ],
            [
                3397.77603275308720128,
                -0.321327362693153371,
                -0.34507619223117997,
                -0.70340000000000003,
                -0.9,
                -1.0,
                -1.0,
                -1.1754605576265209,
            ],
        ]),
        // DCT32X16
        qt::DCT16X32 => dct_enc([
            [
                13844.97076442300573,
                -0.97113799999999995,
                -0.658,
                -0.42026,
                -0.22712,
                -0.2206,
                -0.226,
                -0.6,
            ],
            [
                4798.964084220744293,
                -0.61125308982767057,
                -0.83770786552491361,
                -0.79014862079498627,
                -0.2692727459704829,
                -0.38272769465388551,
                -0.22924222653091453,
                -0.20719098826199578,
            ],
            [1807.236946760964614, -1.2, -1.2, -0.7, -0.7, -0.7, -0.4, -0.5],
        ]),
        // DCT4X8 and 8x4
        qt::DCT4X8 => {
            let mut e = QuantEncoding::base(QuantMode::Dct4x8);
            e.dct_params = dct4x8_params();
            /* kMuls */
            e.dct4x8multipliers = [1.0, 1.0, 1.0];
            e
        }
        // AFV
        qt::AFV0 => {
            let mut e = QuantEncoding::base(QuantMode::Afv);
            e.dct_params = dct4x8_params();
            e.dct_params_afv_4x4 = dct4x4_params();
            e.afv_weights = [
                // 4x4/4x8 DC tendency, AFV corner, AFV high freqs.
                [3072.0, 3072.0, 256.0, 256.0, 256.0, 414.0, 0.0, 0.0, 0.0],
                [1024.0, 1024.0, 50.0, 50.0, 50.0, 58.0, 0.0, 0.0, 0.0],
                [384.0, 384.0, 12.0, 12.0, 12.0, 22.0, -0.25, -0.25, -0.25],
            ];
            e
        }
        // DCT64
        qt::DCT64X64 => big_dct(0.9 * 26629.073922049845, 0.9 * 9311.3238710010046, 0.9 * 4992.2486445538634),
        // DCT64X32
        qt::DCT32X64 => {
            let mut e = big_dct(0.65 * 23629.073922049845, 0.65 * 8611.3238710010046, 0.65 * 4492.2486445538634);
            e.mode = QuantMode::Dct;
            e
        }
        // DCT128X128
        qt::DCT128X128 => big_dct(1.8 * 26629.073922049845, 1.8 * 9311.3238710010046, 1.8 * 4992.2486445538634),
        // DCT128X64
        qt::DCT64X128 => big_dct(1.3 * 23629.073922049845, 1.3 * 8611.3238710010046, 1.3 * 4492.2486445538634),
        // DCT256X256
        qt::DCT256X256 => big_dct(3.6 * 26629.073922049845, 3.6 * 9311.3238710010046, 3.6 * 4992.2486445538634),
        // DCT256X128
        _ => big_dct(2.6 * 23629.073922049845, 2.6 * 8611.3238710010046, 2.6 * 4492.2486445538634),
    }
}
