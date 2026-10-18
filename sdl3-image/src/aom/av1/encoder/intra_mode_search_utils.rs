// Rust translation of the parts of av1/encoder/intra_mode_search_utils.h
// from aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Helpers of the RD intra mode search: the histogram of oriented
//! gradients pruning of the directional modes, the mode info rates and the
//! quick model rate-distortion estimate. The gradient caches of the RD
//! partition search and of the all intra mode are never enabled (the
//! saver's real-time frames use the non-RD pick mode).

use super::block::*;
use super::encodemb::av1_subtract_block;
use super::encoder::Av1Comp;
use super::hybrid_fwd_txfm::av1_quick_txfm;
use super::ml::{av1_nn_predict, NnConfig, NN_MAX_HIDDEN_LAYERS};
use crate::aom::aom_dsp::avg::aom_satd;
use crate::aom::av1::common::av1_common_int::Av1Common;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::reconintra::*;

const BINS: usize = 32;
static AV1_INTRA_HOG_MODEL_BIAS: [f32; DIRECTIONAL_MODES as usize] = [
    0.450578, 0.695518, -0.717944, -0.639894, -0.602019, -0.453454, 0.055857, -0.465480,
];

static AV1_INTRA_HOG_MODEL_WEIGHTS: [f32; BINS * DIRECTIONAL_MODES as usize] = [
    -3.076402, -3.757063, -3.275266, -3.180665, -3.452105, -3.216593, -2.871212, -3.134296,
    -1.822324, -2.401411, -1.541016, -1.195322, -0.434156, 0.322868, 2.260546, 3.368715, 3.989290,
    3.308487, 2.277893, 0.923793, 0.026412, -0.385174, -0.718622, -1.408867, -1.050558, -2.323941,
    -2.225827, -2.585453, -3.054283, -2.875087, -2.985709, -3.447155, 3.758139, 3.204353, 2.170998,
    0.826587, -0.269665, -0.702068, -1.085776, -2.175249, -1.623180, -2.975142, -2.779629,
    -3.190799, -3.521900, -3.375480, -3.319355, -3.897389, -3.172334, -3.594528, -2.879132,
    -2.547777, -2.921023, -2.281844, -1.818988, -2.041771, -0.618268, -1.396458, -0.567153,
    -0.285868, -0.088058, 0.753494, 2.092413, 3.215266, -3.300277, -2.748658, -2.315784, -2.423671,
    -2.257283, -2.269583, -2.196660, -2.301076, -2.646516, -2.271319, -2.254366, -2.300102,
    -2.217960, -2.473300, -2.116866, -2.528246, -3.314712, -1.701010, -0.589040, -0.088077,
    0.813112, 1.702213, 2.653045, 3.351749, 3.243554, 3.199409, 2.437856, 1.468854, 0.533039,
    -0.099065, -0.622643, -2.200732, -4.228861, -2.875263, -1.273956, -0.433280, 0.803771,
    1.975043, 3.179528, 3.939064, 3.454379, 3.689386, 3.116411, 1.970991, 0.798406, -0.628514,
    -1.252546, -2.825176, -4.090178, -3.777448, -3.227314, -3.479403, -3.320569, -3.159372,
    -2.729202, -2.722341, -3.054913, -2.742923, -2.612703, -2.662632, -2.907314, -3.117794,
    -3.102660, -3.970972, -4.891357, -3.935582, -3.347758, -2.721924, -2.219011, -1.702391,
    -0.866529, -0.153743, 0.107733, 1.416882, 2.572884, 3.607755, 3.974820, 3.997783, 2.970459,
    0.791687, -1.478921, -1.228154, -1.216955, -1.765932, -1.951003, -1.985301, -1.975881,
    -1.985593, -2.422371, -2.419978, -2.531288, -2.951853, -3.071380, -3.277027, -3.373539,
    -4.462010, -0.967888, 0.805524, 2.794130, 3.685984, 3.745195, 3.252444, 2.316108, 1.399146,
    -0.136519, -0.162811, -1.004357, -1.667911, -1.964662, -2.937579, -3.019533, -3.942766,
    -5.102767, -3.882073, -3.532027, -3.451956, -2.944015, -2.643064, -2.529872, -2.077290,
    -2.809965, -1.803734, -1.783593, -1.662585, -1.415484, -1.392673, -0.788794, -1.204819,
    -1.998864, -1.182102, -0.892110, -1.317415, -1.359112, -1.522867, -1.468552, -1.779072,
    -2.332959, -2.160346, -2.329387, -2.631259, -2.744936, -3.052494, -2.787363, -3.442548,
    -4.245075, -3.032172, -2.061609, -1.768116, -1.286072, -0.706587, -0.192413, 0.386938,
    0.716997, 1.481393, 2.216702, 2.737986, 3.109809, 3.226084, 2.490098, -0.095827, -3.864816,
    -3.507248, -3.128925, -2.908251, -2.883836, -2.881411, -2.524377, -2.624478, -2.399573,
    -2.367718, -1.918255, -1.926277, -1.694584, -1.723790, -0.966491, -1.183115, -1.430687,
    0.872896, 2.766550, 3.610080, 3.578041, 3.334928, 2.586680, 1.895721, 1.122195, 0.488519,
    -0.140689, -0.799076, -1.222860, -1.502437, -1.900969, -3.206816,
];

static AV1_INTRA_HOG_MODEL_NNCONFIG: NnConfig = NnConfig {
    num_inputs: BINS as i32,               // num_inputs
    num_outputs: DIRECTIONAL_MODES as i32, // num_outputs
    num_hidden_layers: 0,                  // num_hidden_layers
    num_hidden_nodes: [0; NN_MAX_HIDDEN_LAYERS],
    weights: [
        &AV1_INTRA_HOG_MODEL_WEIGHTS,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    ],
    bias: [
        &AV1_INTRA_HOG_MODEL_BIAS,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    ],
};

const FIX_PREC_BITS: i32 = 16;
#[inline]
fn get_hist_bin_idx(dx: i32, dy: i32) -> usize {
    let ratio = (dy * (1 << FIX_PREC_BITS)) / dx;

    // Find index by bisection
    static THRESHOLDS: [i32; BINS] = [
        -1334015,
        -441798,
        -261605,
        -183158,
        -138560,
        -109331,
        -88359,
        -72303,
        -59392,
        -48579,
        -39272,
        -30982,
        -23445,
        -16400,
        -9715,
        -3194,
        3227,
        9748,
        16433,
        23478,
        31015,
        39305,
        48611,
        59425,
        72336,
        88392,
        109364,
        138593,
        183191,
        261638,
        441831,
        i32::MAX,
    ];

    let (lo_idx, hi_idx);
    // Divide into segments of size 8 gives better performance than binary search
    // here.
    if ratio <= THRESHOLDS[7] {
        lo_idx = 0;
        hi_idx = 7;
    } else if ratio <= THRESHOLDS[15] {
        lo_idx = 8;
        hi_idx = 15;
    } else if ratio <= THRESHOLDS[23] {
        lo_idx = 16;
        hi_idx = 23;
    } else {
        lo_idx = 24;
        hi_idx = 31;
    }

    for idx in lo_idx..=hi_idx {
        if ratio <= THRESHOLDS[idx] {
            return idx;
        }
    }
    debug_assert!(false, "No valid histogram bin found!");
    BINS - 1
}

// Normalizes the hog data.
#[inline]
fn normalize_hog(total: f32, hist: &mut [f32; BINS]) {
    for h in hist.iter_mut() {
        *h /= total;
    }
}

/// `lowbd_generate_hog()` and `highbd_generate_hog()` (the same but for the
/// sample type).
fn generate_hog(
    src_buf: &[u16],
    src: usize,
    stride: usize,
    rows: i32,
    cols: i32,
    hist: &mut [f32; BINS],
) {
    let mut total: f32 = 0.1;
    let mut s = src + stride;
    for _r in 1..rows - 1 {
        for c in 1..(cols - 1) as usize {
            let above = s + c - stride;
            let below = s + c + stride;
            let left = s + c - 1;
            let right = s + c + 1;
            let px = |i: usize| src_buf[i] as i32;
            // Calculate gradient using Sobel filters.
            let dx = (px(right - stride) + 2 * px(right) + px(right + stride))
                - (px(left - stride) + 2 * px(left) + px(left + stride));
            let dy = (px(below - 1) + 2 * px(below) + px(below + 1))
                - (px(above - 1) + 2 * px(above) + px(above + 1));
            if dx == 0 && dy == 0 {
                continue;
            }
            let temp = dx.abs() + dy.abs();
            if temp == 0 {
                continue;
            }
            total += temp as f32;
            if dx == 0 {
                hist[0] += (temp / 2) as f32;
                hist[BINS - 1] += (temp / 2) as f32;
            } else {
                let idx = get_hist_bin_idx(dx, dy);
                debug_assert!(idx < BINS);
                hist[idx] += temp as f32;
            }
        }
        s += stride;
    }

    normalize_hog(total, hist);
}

/// Translation of `collect_hog_data()` (the source's samples from `src`).
#[inline]
fn collect_hog_data(
    x: &Macroblock,
    src_buf: &[u16],
    bsize: BlockSize,
    plane: usize,
    hog: &mut [f32; BINS],
) {
    let xd = &x.e_mbd;
    let pd = &xd.plane[plane];
    let ss_x = pd.subsampling_x;
    let ss_y = pd.subsampling_y;
    let bh = BLOCK_SIZE_HIGH[bsize as usize] as i32;
    let bw = BLOCK_SIZE_WIDE[bsize as usize] as i32;
    let rows = (if xd.mb_to_bottom_edge >= 0 {
        bh
    } else {
        (xd.mb_to_bottom_edge >> 3) + bh
    }) >> ss_y;
    let cols = (if xd.mb_to_right_edge >= 0 {
        bw
    } else {
        (xd.mb_to_right_edge >> 3) + bw
    }) >> ss_x;

    // If gradient data is already generated at SB level, reuse the cached data.
    // Otherwise, compute the data.
    // (the gradient caches are never enabled: see the module documentation)
    let src = x.plane[plane].src.buf;
    let src_stride = x.plane[plane].src.stride as usize;
    generate_hog(src_buf, src, src_stride, rows, cols, hog);

    // Scale the hog so the luma and chroma are on the same scale
    for h in hog.iter_mut() {
        *h *= ((1 + ss_x) * (1 + ss_y)) as f32;
    }
}

/// Translation of `prune_intra_mode_with_hog()`.
pub(crate) fn prune_intra_mode_with_hog(
    x: &Macroblock,
    src_buf: &[u16],
    bsize: BlockSize,
    th: f32,
    directional_mode_skip_mask: &mut [u8; INTRA_MODES as usize],
    is_chroma: bool,
) -> Result<(), ()> {
    let plane = if is_chroma { AOM_PLANE_U } else { AOM_PLANE_Y };
    let mut hist = [0f32; BINS];
    collect_hog_data(x, src_buf, bsize, plane, &mut hist);

    // Make prediction for each of the mode
    let mut scores = [0f32; DIRECTIONAL_MODES as usize];
    av1_nn_predict(&hist, &AV1_INTRA_HOG_MODEL_NNCONFIG, 1, &mut scores)?;
    for uv_mode in UV_V_PRED..=UV_D67_PRED {
        if scores[(uv_mode - UV_V_PRED) as usize] <= th {
            directional_mode_skip_mask[uv_mode as usize] = 1;
        }
    }
    Ok(())
}

/// Translation of `intra_mode_info_cost_y()`: the rate cost for luma
/// prediction mode info of intra blocks.
pub(crate) fn intra_mode_info_cost_y(
    cpi: &Av1Comp,
    x: &Macroblock,
    mbmi: &MbModeInfo,
    bsize: BlockSize,
    mode_cost: i32,
    _discount_color_cost: i32,
) -> Result<i32, ()> {
    let mut total_rate = mode_cost;
    let mode_costs = &x.mode_costs;
    let use_palette = mbmi.palette_mode_info.palette_size[0] > 0;
    let use_filter_intra = mbmi.filter_intra_mode_info.use_filter_intra;
    let use_intrabc = mbmi.use_intrabc;
    // Can only activate one mode.
    debug_assert!(
        ((mbmi.mode != DC_PRED) as i32
            + use_palette as i32
            + use_intrabc as i32
            + use_filter_intra as i32)
            <= 1
    );
    let try_palette = av1_allow_palette(cpi.common.features.allow_screen_content_tools, mbmi.bsize);
    if try_palette && mbmi.mode == DC_PRED {
        // (palettes need screen content tools, which are off)
        return Err(());
    }
    if av1_filter_intra_allowed(&cpi.common, mbmi) {
        total_rate += mode_costs.filter_intra_cost[mbmi.bsize as usize][use_filter_intra as usize];
        if use_filter_intra != 0 {
            total_rate += mode_costs.filter_intra_mode_cost
                [mbmi.filter_intra_mode_info.filter_intra_mode as usize];
        }
    }
    if av1_is_directional_mode(mbmi.mode) && av1_use_angle_delta(bsize) {
        total_rate += mode_costs.angle_delta_cost[(mbmi.mode - V_PRED) as usize]
            [(MAX_ANGLE_DELTA + mbmi.angle_delta[PLANE_TYPE_Y as usize] as i32) as usize];
    }
    if av1_allow_intrabc(&cpi.common) {
        total_rate += mode_costs.intrabc_cost[use_intrabc as usize];
    }
    Ok(total_rate)
}

/// Translation of `intra_mode_info_cost_uv()`: the rate cost for chroma
/// prediction mode info of intra blocks.
pub(crate) fn intra_mode_info_cost_uv(
    cpi: &Av1Comp,
    x: &Macroblock,
    mbmi: &MbModeInfo,
    bsize: BlockSize,
    mode_cost: i32,
) -> Result<i32, ()> {
    let mut total_rate = mode_cost;
    let mode_costs = &x.mode_costs;
    let use_palette = mbmi.palette_mode_info.palette_size[1] > 0;
    let mode = mbmi.uv_mode;
    // Can only activate one mode.
    debug_assert!(
        ((mode != UV_DC_PRED) as i32 + use_palette as i32 + mbmi.use_intrabc as i32) <= 1
    );

    let try_palette = av1_allow_palette(cpi.common.features.allow_screen_content_tools, mbmi.bsize);
    if try_palette && mode == UV_DC_PRED {
        // (palettes need screen content tools, which are off)
        return Err(());
    }
    if av1_is_directional_mode(get_uv_mode(mode)) && av1_use_angle_delta(bsize) {
        total_rate += mode_costs.angle_delta_cost[(mode - UV_V_PRED) as usize]
            [(mbmi.angle_delta[PLANE_TYPE_UV as usize] as i32 + MAX_ANGLE_DELTA) as usize];
    }
    Ok(total_rate)
}

// Makes a quick intra prediction and estimate the rdcost with a model without
// going through the whole txfm/quantize/itxfm process.
/// Translation of `intra_model_rd()`.
pub(crate) fn intra_model_rd(
    cm: &Av1Common,
    x: &mut Macroblock,
    src_buf: &[u16],
    plane: usize,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    use_hadamard: bool,
) -> Result<i64, ()> {
    let bd_info = get_bit_depth_info(&x.e_mbd);
    debug_assert!(!is_inter_block(x.e_mbd.mbmi()));
    let stepr = TX_SIZE_HIGH_UNIT[tx_size as usize] as i32;
    let stepc = TX_SIZE_WIDE_UNIT[tx_size as usize] as i32;
    let txbw = TX_SIZE_WIDE[tx_size as usize];
    let txbh = TX_SIZE_HIGH[tx_size as usize];
    let max_blocks_wide = max_block_wide(&x.e_mbd, plane_bsize, plane);
    let max_blocks_high = max_block_high(&x.e_mbd, plane_bsize, plane);
    let mut satd_cost: i64 = 0;
    // Prediction.
    let mut row = 0;
    while row < max_blocks_high {
        let mut col = 0;
        while col < max_blocks_wide {
            av1_predict_intra_block_facade(cm, &mut x.e_mbd, plane, col, row, tx_size)?;
            // Here we use p->src_diff and p->coeff as temporary buffers for
            // prediction residue and transform coefficients. The buffers are only
            // used in this for loop, therefore we don't need to properly add offset
            // to the buffers.
            let Macroblock {
                plane: planes,
                e_mbd: xd,
                ..
            } = x;
            let p = &mut planes[plane];
            let pd = &xd.plane[plane];
            let src = p.src.buf + (((row * p.src.stride) + col) << 2) as usize;
            let dst = pd.dst.buf + (((row * pd.dst.stride) + col) << 2) as usize;
            av1_subtract_block(
                bd_info,
                txbh,
                txbw,
                &mut p.src_diff,
                0,
                BLOCK_SIZE_WIDE[plane_bsize as usize] as usize,
                src_buf,
                src,
                p.src.stride as usize,
                &xd.recon.buffer_alloc,
                dst,
                pd.dst.stride as usize,
            );
            av1_quick_txfm(
                use_hadamard,
                tx_size,
                bd_info,
                &p.src_diff,
                BLOCK_SIZE_WIDE[plane_bsize as usize] as usize,
                &mut p.coeff,
            )?;
            satd_cost += aom_satd(&p.coeff, TX_SIZE_2D[tx_size as usize]) as i64;
            col += stepc;
        }
        row += stepr;
    }
    Ok(satd_cost)
}

/// Translation of `model_intra_yrd_and_prune()`: estimate the luma rdcost
/// of a given intra mode and try to prune it.
///
/// This function first makes a quick luma prediction and estimates the
/// rdcost with a model without going through the txfm, then try to prune
/// the current mode if the new estimate y_rd > 1.25 * best_model_rd.
///
/// Returns whether the given mode is pruned.
pub(crate) fn model_intra_yrd_and_prune(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    best_model_rd: &mut i64,
) -> Result<bool, ()> {
    let tx_size = TX_32X32.min(MAX_TXSIZE_LOOKUP[bsize as usize]);
    let plane = 0;
    let cm = &cpi.common;
    let this_model_rd = intra_model_rd(
        cm,
        x,
        &cpi.source().buffer_alloc,
        plane,
        bsize,
        tx_size,
        /*use_hadamard=*/ true,
    )?;
    if *best_model_rd != i64::MAX && this_model_rd > *best_model_rd + (*best_model_rd >> 2) {
        return Ok(true);
    } else if this_model_rd < *best_model_rd {
        *best_model_rd = this_model_rd;
    }
    Ok(false)
}
