// Rust translation of the parts of av1/encoder/intra_mode_search.c and
// av1/encoder/intra_mode_search.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The RD intra mode search of intra frames (the real-time key frames'
//! small blocks, see `hybrid_intra_mode_search()`): the luma modes with
//! their angle deltas and the filter intra modes, then the chroma modes
//! with chroma from luma. Palettes need screen content tools (off), and
//! the multi-winner processing is off for the saver's speed features: their
//! paths fail.

use super::block::*;
use super::context_tree::PickModeContext;
use super::encodemb::av1_encode_intra_block_plane;
use super::encoder::{Av1Comp, ALLINTRA};
use super::intra_mode_search_utils::*;
use super::rd::*;
use super::rdopt_utils::*;
use super::speed_features::*;
use super::tx_search::*;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cfl::{get_cfl_pred_type, is_cfl_allowed};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::reconintra::*;

// Even though there are 7 delta angles, this macro is set to 9 to facilitate
// the rd threshold check to prune -3 and 3 delta angles.
const SIZE_OF_ANGLE_DELTA_RD_COST_ARRAY: usize = (2 * MAX_ANGLE_DELTA + 3) as usize;

// The order for evaluating delta angles while processing the luma directional
// intra modes. Currently, this order of evaluation is applicable only when
// speed feature prune_luma_odd_delta_angles_in_intra is enabled. In this case,
// even angles are evaluated first in order to facilitate the pruning of odd
// delta angles based on the rd costs of the neighboring delta angles.
static LUMA_DELTA_ANGLES_ORDER: [i8; (2 * MAX_ANGLE_DELTA) as usize] = [-2, 2, -3, -1, 1, 3];

static INTRA_RD_SEARCH_MODE_ORDER: [PredictionMode; INTRA_MODES as usize] = [
    DC_PRED,
    H_PRED,
    V_PRED,
    SMOOTH_PRED,
    PAETH_PRED,
    SMOOTH_V_PRED,
    SMOOTH_H_PRED,
    D135_PRED,
    D203_PRED,
    D157_PRED,
    D67_PRED,
    D113_PRED,
    D45_PRED,
];

static UV_RD_SEARCH_MODE_ORDER: [UvPredictionMode; UV_INTRA_MODES as usize] = [
    UV_DC_PRED,
    UV_CFL_PRED,
    UV_H_PRED,
    UV_V_PRED,
    UV_SMOOTH_PRED,
    UV_PAETH_PRED,
    UV_SMOOTH_V_PRED,
    UV_SMOOTH_H_PRED,
    UV_D135_PRED,
    UV_D203_PRED,
    UV_D157_PRED,
    UV_D67_PRED,
    UV_D113_PRED,
    UV_D45_PRED,
];

// The bitmask corresponds to the filter intra modes as defined in enums.h
// FILTER_INTRA_MODE enumeration type. Setting a bit to 0 in the mask means to
// disable the evaluation of corresponding filter intra mode. The table
// av1_derived_filter_intra_mode_used_flag is used when speed feature
// prune_filter_intra_level is 1. The evaluated filter intra modes are union
// of the following:
// 1) FILTER_DC_PRED
// 2) mode that corresponds to best mode so far of DC_PRED, V_PRED, H_PRED,
// D157_PRED and PAETH_PRED. (Eg: FILTER_V_PRED if best mode so far is V_PRED).
static AV1_DERIVED_FILTER_INTRA_MODE_USED_FLAG: [u8; INTRA_MODES as usize] = [
    0x01, // DC_PRED:           0000 0001
    0x03, // V_PRED:            0000 0011
    0x05, // H_PRED:            0000 0101
    0x01, // D45_PRED:          0000 0001
    0x01, // D135_PRED:         0000 0001
    0x01, // D113_PRED:         0000 0001
    0x09, // D157_PRED:         0000 1001
    0x01, // D203_PRED:         0000 0001
    0x01, // D67_PRED:          0000 0001
    0x01, // SMOOTH_PRED:       0000 0001
    0x01, // SMOOTH_V_PRED:     0000 0001
    0x01, // SMOOTH_H_PRED:     0000 0001
    0x11, // PAETH_PRED:        0001 0001
];

/// The total number of luma intra modes (`LUMA_MODE_COUNT`).
const LUMA_MODE_COUNT: i32 = PAETH_PRED - DC_PRED + 1 + 6 * 8;
/// `TOP_INTRA_MODEL_COUNT`
const TOP_INTRA_MODEL_COUNT: usize = 4;

/*\brief Search for the best filter_intra mode when coding intra frame.
 *
 * This function loops through all filter_intra modes to find the best one.
 *
 * Returns 1 if a new filter_intra mode is selected; 0 otherwise.
 */
fn rd_pick_filter_intra_sby(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rate: &mut i32,
    rate_tokenonly: &mut i32,
    distortion: &mut i64,
    skippable: &mut u8,
    bsize: BlockSize,
    mode_cost: i32,
    best_mode_so_far: PredictionMode,
    best_rd: &mut i64,
    best_model_rd: &mut i64,
    ctx: &mut PickModeContext,
) -> Result<bool, ()> {
    // Skip the evaluation of filter intra modes.
    if cpi.sf.intra_sf.prune_filter_intra_level == 2 {
        return Ok(false);
    }

    let mut filter_intra_selected_flag = false;
    let mut best_tx_size = TX_8X8;
    let mut filter_intra_mode_info = FilterIntraModeInfo::default();
    let mut best_tx_type_map = [0u8; (MAX_MIB_SIZE * MAX_MIB_SIZE) as usize];
    {
        let mbmi = x.e_mbd.mbmi_mut();
        mbmi.filter_intra_mode_info.use_filter_intra = 1;
        mbmi.mode = DC_PRED;
        mbmi.palette_mode_info.palette_size[0] = 0;
    }

    // Skip the evaluation of filter-intra if cached MB_MODE_INFO does not have
    // filter-intra as winner.
    // (x->use_mb_mode_cache: the mode cache is the RD partition search's)

    let n = ctx.num_4x4_blk as usize;
    for mode in 0..FILTER_INTRA_MODES {
        x.e_mbd.mbmi_mut().filter_intra_mode_info.filter_intra_mode = mode;

        if cpi.sf.intra_sf.prune_filter_intra_level == 1
            && (AV1_DERIVED_FILTER_INTRA_MODE_USED_FLAG[best_mode_so_far as usize] & (1 << mode))
                == 0
        {
            continue;
        }

        // Skip the evaluation of modes that do not match with the winner mode in
        // x->mb_mode_cache.
        // (see above)

        if model_intra_yrd_and_prune(cpi, x, bsize, best_model_rd)? {
            continue;
        }
        let mut tokenonly_rd_stats = RdStats::default();
        av1_pick_uniform_tx_size_type_yrd(cpi, x, &mut tokenonly_rd_stats, bsize, *best_rd)?;
        if tokenonly_rd_stats.rate == i32::MAX {
            continue;
        }
        let mbmi = *x.e_mbd.mbmi();
        let this_rate =
            tokenonly_rd_stats.rate + intra_mode_info_cost_y(cpi, x, &mbmi, bsize, mode_cost, 0)?;
        let this_rd = rdcost(x.rdmult, this_rate, tokenonly_rd_stats.dist);

        // Visual quality adjustment based on recon vs source variance.
        if cpi.oxcf.mode == ALLINTRA && this_rd != i64::MAX {
            // (intra_rd_variance_factor(): the all intra mode's)
            return Err(());
        }

        // Collect mode stats for multiwinner mode processing
        store_winner_mode_stats(cpi.sf.winner_mode_sf.multi_winner_mode_type)?;
        if this_rd < *best_rd {
            *best_rd = this_rd;
            best_tx_size = mbmi.tx_size;
            filter_intra_mode_info = mbmi.filter_intra_mode_info;
            for (i, t) in best_tx_type_map[..n].iter_mut().enumerate() {
                *t = x.e_mbd.tx_type_map_get(i);
            }
            ctx.blk_skip[..n].copy_from_slice(&x.txfm_search_info.blk_skip[..n]);
            *rate = this_rate;
            *rate_tokenonly = tokenonly_rd_stats.rate;
            *distortion = tokenonly_rd_stats.dist;
            *skippable = tokenonly_rd_stats.skip_txfm;
            filter_intra_selected_flag = true;
        }
    }

    if filter_intra_selected_flag {
        let mbmi = x.e_mbd.mbmi_mut();
        mbmi.mode = DC_PRED;
        mbmi.tx_size = best_tx_size;
        mbmi.filter_intra_mode_info = filter_intra_mode_info;
        ctx.tx_type_map[..n].copy_from_slice(&best_tx_type_map[..n]);
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Translation of `set_y_mode_and_delta_angle()`.
pub(crate) fn set_y_mode_and_delta_angle(
    mode_idx: i32,
    mbmi: &mut MbModeInfo,
    reorder_delta_angle_eval: i32,
) {
    if mode_idx < INTRA_MODE_END {
        mbmi.mode = INTRA_RD_SEARCH_MODE_ORDER[mode_idx as usize];
        mbmi.angle_delta[PLANE_TYPE_Y as usize] = 0;
    } else {
        mbmi.mode = (mode_idx - INTRA_MODE_END) / (MAX_ANGLE_DELTA * 2) + V_PRED;
        let delta_angle_eval_idx = (mode_idx - INTRA_MODE_END) % (MAX_ANGLE_DELTA * 2);
        if reorder_delta_angle_eval != 0 {
            mbmi.angle_delta[PLANE_TYPE_Y as usize] =
                LUMA_DELTA_ANGLES_ORDER[delta_angle_eval_idx as usize];
        } else {
            mbmi.angle_delta[PLANE_TYPE_Y as usize] = if delta_angle_eval_idx < 3 {
                delta_angle_eval_idx - 3
            } else {
                delta_angle_eval_idx - 2
            } as i8;
        }
    }
}

#[inline]
fn get_model_rd_index_for_pruning(intra_sf: &IntraModeSpeedFeatures) -> Result<usize, ()> {
    let top_intra_model_count_allowed = intra_sf.top_intra_model_count_allowed;
    if intra_sf.adapt_top_model_rd_count_using_neighbors == 0 {
        return Ok((top_intra_model_count_allowed - 1) as usize);
    }
    // (the neighbor based adaptation: off for the saver's speed features)
    Err(())
}

/// Translation of `prune_intra_y_mode()`.
pub(crate) fn prune_intra_y_mode(
    this_model_rd: i64,
    best_model_rd: &mut i64,
    top_intra_model_rd: &mut [i64],
    max_model_cnt_allowed: i32,
    model_rd_index_for_pruning: usize,
) -> bool {
    let thresh_best = 1.50f64;
    let thresh_top = 1.00f64;
    let max_model_cnt_allowed = max_model_cnt_allowed as usize;
    for i in 0..max_model_cnt_allowed {
        if this_model_rd < top_intra_model_rd[i] {
            let mut j = max_model_cnt_allowed - 1;
            while j > i {
                top_intra_model_rd[j] = top_intra_model_rd[j - 1];
                j -= 1;
            }
            top_intra_model_rd[i] = this_model_rd;
            break;
        }
    }
    if top_intra_model_rd[model_rd_index_for_pruning] != i64::MAX
        && this_model_rd as f64 > thresh_top * top_intra_model_rd[model_rd_index_for_pruning] as f64
    {
        return true;
    }

    if this_model_rd != i64::MAX && this_model_rd as f64 > thresh_best * (*best_model_rd as f64) {
        return true;
    }
    if this_model_rd < *best_model_rd {
        *best_model_rd = this_model_rd;
    }
    false
}

// Run RD calculation with given chroma intra prediction angle., and return
// the RD cost. Update the best mode info. if the RD cost is the best so far.
// (pick_intra_angle_routine_sbuv(): the chroma angle deltas are never
// searched: see av1_rd_pick_intra_sbuv_mode())

#[inline]
fn plane_sign_to_joint_sign(plane: usize, a: i32, b: i32) -> i32 {
    if plane == CFL_PRED_U as usize {
        a * CFL_SIGNS + b - 1
    } else {
        b * CFL_SIGNS + a - 1
    }
}

fn cfl_idx_to_sign_and_alpha(cfl_idx: i32, cfl_sign: &mut i32, cfl_alpha: &mut i32) {
    let cfl_linear_idx = cfl_idx - CFL_INDEX_ZERO;
    if cfl_linear_idx == 0 {
        *cfl_sign = CFL_SIGN_ZERO;
        *cfl_alpha = 0;
    } else {
        *cfl_sign = if cfl_linear_idx > 0 {
            CFL_SIGN_POS
        } else {
            CFL_SIGN_NEG
        };
        *cfl_alpha = cfl_linear_idx.abs() - 1;
    }
}

fn cfl_compute_rd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    plane: usize,
    tx_size: TxSize,
    plane_bsize: BlockSize,
    cfl_idx: i32,
    fast_mode: bool,
    rd_stats: Option<&mut RdStats>,
) -> Result<i64, ()> {
    debug_assert!(!fast_mode || rd_stats.is_none());
    let cm = &cpi.common;
    let cfl_plane = get_cfl_pred_type(plane);
    let mut cfl_sign = 0;
    let mut cfl_alpha = 0;
    cfl_idx_to_sign_and_alpha(cfl_idx, &mut cfl_sign, &mut cfl_alpha);
    // We conly build CFL for a given plane, the other plane's sign is dummy
    let dummy_sign = CFL_SIGN_NEG;
    let (orig_cfl_alpha_signs, orig_cfl_alpha_idx) = {
        let mbmi = x.e_mbd.mbmi_mut();
        let o = (mbmi.cfl_alpha_signs, mbmi.cfl_alpha_idx);
        mbmi.cfl_alpha_signs = plane_sign_to_joint_sign(cfl_plane, cfl_sign, dummy_sign) as i8;
        mbmi.cfl_alpha_idx = ((cfl_alpha << CFL_ALPHABET_SIZE_LOG2) + cfl_alpha) as u8;
        o
    };
    let res = (|| -> Result<i64, ()> {
        if fast_mode {
            intra_model_rd(
                cm,
                x,
                &cpi.source().buffer_alloc,
                plane,
                plane_bsize,
                tx_size,
                /*use_hadamard=*/ false,
            )
        } else {
            let rd_stats = rd_stats.ok_or(())?;
            av1_init_rd_stats(rd_stats);
            av1_txfm_rd_in_plane(
                x,
                cpi,
                rd_stats,
                i64::MAX,
                0,
                plane,
                plane_bsize,
                tx_size,
                FTXS_NONE,
                0,
            )?;
            av1_rd_cost_update(x.rdmult, rd_stats);
            Ok(rd_stats.rdcost)
        }
    })();
    let mbmi = x.e_mbd.mbmi_mut();
    mbmi.cfl_alpha_signs = orig_cfl_alpha_signs;
    mbmi.cfl_alpha_idx = orig_cfl_alpha_idx;
    res
}

static CFL_DIR_LS: [i32; 2] = [1, -1];

// If cfl_search_range is CFL_MAGS_SIZE, return zero. Otherwise return the index
// of the best alpha found using intra_model_rd().
fn cfl_pick_plane_parameter(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    plane: usize,
    tx_size: TxSize,
    cfl_search_range: i32,
) -> Result<i32, ()> {
    debug_assert!((1..=CFL_MAGS_SIZE).contains(&cfl_search_range));

    if cfl_search_range == CFL_MAGS_SIZE {
        return Ok(CFL_INDEX_ZERO);
    }

    let xd = &x.e_mbd;
    let mbmi = xd.mbmi();
    debug_assert!(mbmi.uv_mode == UV_CFL_PRED);
    let pd = &xd.plane[plane];
    let plane_bsize = get_plane_block_size(mbmi.bsize, pd.subsampling_x, pd.subsampling_y);

    let mut est_best_cfl_idx = CFL_INDEX_ZERO;
    let fast_mode = true;
    let start_cfl_idx = CFL_INDEX_ZERO;
    let mut best_cfl_cost = cfl_compute_rd(
        cpi,
        x,
        plane,
        tx_size,
        plane_bsize,
        start_cfl_idx,
        fast_mode,
        None,
    )?;
    for si in 0..2 {
        let dir = CFL_DIR_LS[si];
        for i in 1..CFL_MAGS_SIZE {
            let cfl_idx = start_cfl_idx + dir * i;
            if cfl_idx < 0 || cfl_idx >= CFL_MAGS_SIZE {
                break;
            }
            let cfl_cost = cfl_compute_rd(
                cpi,
                x,
                plane,
                tx_size,
                plane_bsize,
                cfl_idx,
                fast_mode,
                None,
            )?;
            if cfl_cost < best_cfl_cost {
                best_cfl_cost = cfl_cost;
                est_best_cfl_idx = cfl_idx;
            } else {
                break;
            }
        }
    }
    Ok(est_best_cfl_idx)
}

fn cfl_pick_plane_rd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    plane: usize,
    tx_size: TxSize,
    cfl_search_range: i32,
    cfl_rd_arr: &mut [RdStats; CFL_MAGS_SIZE as usize],
    est_best_cfl_idx: i32,
) -> Result<(), ()> {
    debug_assert!((1..=CFL_MAGS_SIZE).contains(&cfl_search_range));
    let xd = &x.e_mbd;
    let mbmi = xd.mbmi();
    debug_assert!(mbmi.uv_mode == UV_CFL_PRED);
    let pd = &xd.plane[plane];
    let plane_bsize = get_plane_block_size(mbmi.bsize, pd.subsampling_x, pd.subsampling_y);

    for r in cfl_rd_arr.iter_mut() {
        av1_invalid_rd_stats(r);
    }

    let fast_mode = false;
    let start_cfl_idx = est_best_cfl_idx;
    cfl_compute_rd(
        cpi,
        x,
        plane,
        tx_size,
        plane_bsize,
        start_cfl_idx,
        fast_mode,
        Some(&mut cfl_rd_arr[start_cfl_idx as usize]),
    )?;

    if cfl_search_range == 1 {
        return Ok(());
    }

    for si in 0..2 {
        let dir = CFL_DIR_LS[si];
        for i in 1..cfl_search_range {
            let cfl_idx = start_cfl_idx + dir * i;
            if cfl_idx < 0 || cfl_idx >= CFL_MAGS_SIZE {
                break;
            }
            cfl_compute_rd(
                cpi,
                x,
                plane,
                tx_size,
                plane_bsize,
                cfl_idx,
                fast_mode,
                Some(&mut cfl_rd_arr[cfl_idx as usize]),
            )?;
        }
    }
    Ok(())
}

/*\brief Pick the optimal parameters for Chroma to Luma (CFL) component
 *
 * This function will use DCT_DCT followed by computing SATD (sum of absolute
 * transformed differences) to estimate the RD score and find the best possible
 * CFL parameter.
 *
 * Then the function will apply a full RD search near the best possible CFL
 * parameter to find the best actual CFL parameter.
 *
 * Side effect:
 * We use ths buffers in x->plane[] and xd->plane[] as throw-away buffers for RD
 * search.
 */
fn cfl_rd_pick_alpha(
    x: &mut Macroblock,
    cpi: &Av1Comp,
    tx_size: TxSize,
    ref_best_rd: i64,
    cfl_search_range: i32,
    best_rd_stats: &mut RdStats,
    best_cfl_alpha_idx: &mut u8,
    best_cfl_alpha_signs: &mut i8,
) -> Result<bool, ()> {
    debug_assert!((1..=CFL_MAGS_SIZE).contains(&cfl_search_range));
    let mut cfl_rd_arr_u = [RdStats::default(); CFL_MAGS_SIZE as usize];
    let mut cfl_rd_arr_v = [RdStats::default(); CFL_MAGS_SIZE as usize];

    av1_invalid_rd_stats(best_rd_stats);

    // As the dc pred data is same for different values of alpha, enable the
    // caching of dc pred data.
    x.e_mbd.cfl.use_dc_pred_cache = 1;
    // Evaluate alpha parameter of each chroma plane.
    let est_best_cfl_idx_u = cfl_pick_plane_parameter(cpi, x, 1, tx_size, cfl_search_range)?;
    let est_best_cfl_idx_v = cfl_pick_plane_parameter(cpi, x, 2, tx_size, cfl_search_range)?;

    // For cfl_search_range=1, further refinement of alpha is not enabled. Hence
    // CfL index=0 for both the chroma planes implies invalid CfL mode.
    if cfl_search_range == 1
        && est_best_cfl_idx_u == CFL_INDEX_ZERO
        && est_best_cfl_idx_v == CFL_INDEX_ZERO
    {
        // Set invalid CfL parameters here as CfL mode is invalid.
        *best_cfl_alpha_idx = 0;
        *best_cfl_alpha_signs = 0;

        // Clear the following flags to avoid the unintentional usage of cached dc
        // pred data.
        x.e_mbd.cfl.use_dc_pred_cache = 0;
        x.e_mbd.cfl.dc_pred_is_cached[0] = 0;
        x.e_mbd.cfl.dc_pred_is_cached[1] = 0;
        return Ok(false);
    }

    // Compute the rd cost of each chroma plane using the alpha parameters which
    // were already evaluated.
    cfl_pick_plane_rd(
        cpi,
        x,
        1,
        tx_size,
        cfl_search_range,
        &mut cfl_rd_arr_u,
        est_best_cfl_idx_u,
    )?;
    cfl_pick_plane_rd(
        cpi,
        x,
        2,
        tx_size,
        cfl_search_range,
        &mut cfl_rd_arr_v,
        est_best_cfl_idx_v,
    )?;

    // Clear the following flags to avoid the unintentional usage of cached dc
    // pred data.
    x.e_mbd.cfl.use_dc_pred_cache = 0;
    x.e_mbd.cfl.dc_pred_is_cached[0] = 0;
    x.e_mbd.cfl.dc_pred_is_cached[1] = 0;

    let mode_costs = &x.mode_costs;
    for ui in 0..CFL_MAGS_SIZE {
        if cfl_rd_arr_u[ui as usize].rate == i32::MAX {
            continue;
        }
        let mut cfl_alpha_u = 0;
        let mut cfl_sign_u = 0;
        cfl_idx_to_sign_and_alpha(ui, &mut cfl_sign_u, &mut cfl_alpha_u);
        for vi in 0..CFL_MAGS_SIZE {
            if cfl_rd_arr_v[vi as usize].rate == i32::MAX {
                continue;
            }
            let mut cfl_alpha_v = 0;
            let mut cfl_sign_v = 0;
            cfl_idx_to_sign_and_alpha(vi, &mut cfl_sign_v, &mut cfl_alpha_v);
            // cfl_sign_u == CFL_SIGN_ZERO && cfl_sign_v == CFL_SIGN_ZERO is not a
            // valid parameter for CFL
            if cfl_sign_u == CFL_SIGN_ZERO && cfl_sign_v == CFL_SIGN_ZERO {
                continue;
            }
            let joint_sign = cfl_sign_u * CFL_SIGNS + cfl_sign_v - 1;
            let mut rd_stats = cfl_rd_arr_u[ui as usize];
            av1_merge_rd_stats(&mut rd_stats, &cfl_rd_arr_v[vi as usize]);
            if rd_stats.rate != i32::MAX {
                rd_stats.rate += mode_costs.cfl_cost[joint_sign as usize][CFL_PRED_U as usize]
                    [cfl_alpha_u as usize];
                rd_stats.rate += mode_costs.cfl_cost[joint_sign as usize][CFL_PRED_V as usize]
                    [cfl_alpha_v as usize];
            }
            av1_rd_cost_update(x.rdmult, &mut rd_stats);
            if rd_stats.rdcost < best_rd_stats.rdcost {
                *best_rd_stats = rd_stats;
                *best_cfl_alpha_idx = ((cfl_alpha_u << CFL_ALPHABET_SIZE_LOG2) + cfl_alpha_v) as u8;
                *best_cfl_alpha_signs = joint_sign as i8;
            }
        }
    }
    if best_rd_stats.rdcost >= ref_best_rd {
        av1_invalid_rd_stats(best_rd_stats);
        // Set invalid CFL parameters here since the rdcost is not better than
        // ref_best_rd.
        *best_cfl_alpha_idx = 0;
        *best_cfl_alpha_signs = 0;
        return Ok(false);
    }
    Ok(true)
}

/// Translation of `av1_rd_pick_intra_sbuv_mode()`.
pub(crate) fn av1_rd_pick_intra_sbuv_mode(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rate: &mut i32,
    rate_tokenonly: &mut i32,
    distortion: &mut i64,
    skippable: &mut u8,
    bsize: BlockSize,
    max_tx_size: TxSize,
) -> Result<i64, ()> {
    let cm = &cpi.common;
    debug_assert!(!is_inter_block(x.e_mbd.mbmi()));
    let mut best_mbmi = *x.e_mbd.mbmi();
    let mut best_rd = i64::MAX;
    let intra_mode_cfg = &cpi.oxcf.intra_mode_cfg;

    init_sbuv_mode(x.e_mbd.mbmi_mut());

    // Return if the current block does not correspond to a chroma block.
    if !x.e_mbd.is_chroma_ref {
        *rate = 0;
        *rate_tokenonly = 0;
        *distortion = 0;
        *skippable = 1;
        return Ok(i64::MAX);
    }

    // Only store reconstructed luma when there's chroma RDO. When there's no
    // chroma RDO, the reconstructed luma will be stored in encode_superblock().
    x.e_mbd.cfl.store_y = store_cfl_required_rdo(cm, x);
    if x.e_mbd.cfl.store_y != 0 {
        // Restore reconstructed luma values.
        // TODO(chiyotsai@google.com): right now we are re-computing the txfm in
        // this function everytime we search through uv modes. There is some
        // potential speed up here if we cache the result to avoid redundant
        // computation.
        let mbmi = *x.e_mbd.mbmi();
        av1_encode_intra_block_plane(
            cpi,
            x,
            &cpi.source().buffer_alloc,
            mbmi.bsize,
            AOM_PLANE_Y,
            DRY_RUN_NORMAL,
            cpi.optimize_seg_arr[mbmi.segment_id as usize],
        )?;
        x.e_mbd.cfl.store_y = 0;
    }
    // (init_intra_mode_search_state(): the state is only read by the chroma
    // angle delta search)

    // Search through all non-palette modes.
    for mode_idx in 0..UV_INTRA_MODES as usize {
        let mut tokenonly_rd_stats = RdStats::default();
        let mode = UV_RD_SEARCH_MODE_ORDER[mode_idx];
        let is_diagonal_mode = av1_is_diagonal_mode(get_uv_mode(mode));
        let is_directional_mode = av1_is_directional_mode(get_uv_mode(mode));

        if is_diagonal_mode && !cpi.oxcf.intra_mode_cfg.enable_diagonal_intra {
            continue;
        }
        if is_directional_mode && !cpi.oxcf.intra_mode_cfg.enable_directional_intra {
            continue;
        }

        if cpi.sf.intra_sf.intra_uv_mode_mask[TXSIZE_SQR_UP_MAP[max_tx_size as usize] as usize]
            & (1 << mode)
            == 0
        {
            continue;
        }
        if !intra_mode_cfg.enable_smooth_intra
            && (UV_SMOOTH_PRED..=UV_SMOOTH_H_PRED).contains(&mode)
        {
            continue;
        }

        if !intra_mode_cfg.enable_paeth_intra && mode == UV_PAETH_PRED {
            continue;
        }

        debug_assert!(x.e_mbd.mbmi().mode < INTRA_MODES);
        if cpi.sf.intra_sf.prune_chroma_modes_using_luma_winner != 0 {
            // (av1_derived_chroma_intra_mode_used_flag: off for the saver's
            // speed features)
            return Err(());
        }

        x.e_mbd.mbmi_mut().uv_mode = mode;

        // Init variables for cfl and angle delta
        let sf = &cpi.sf;
        x.e_mbd.mbmi_mut().angle_delta[PLANE_TYPE_UV as usize] = 0;
        if mode == UV_CFL_PRED {
            if is_cfl_allowed(&x.e_mbd) == 0 || !intra_mode_cfg.enable_cfl_intra {
                continue;
            }
            debug_assert!(!is_directional_mode);
            let uv_tx_size = av1_get_tx_size(AOM_PLANE_U, &x.e_mbd);
            let mut cfl_alpha_idx = x.e_mbd.mbmi().cfl_alpha_idx;
            let mut cfl_alpha_signs = x.e_mbd.mbmi().cfl_alpha_signs;
            let ok = cfl_rd_pick_alpha(
                x,
                cpi,
                uv_tx_size,
                best_rd,
                sf.intra_sf.cfl_search_range,
                &mut tokenonly_rd_stats,
                &mut cfl_alpha_idx,
                &mut cfl_alpha_signs,
            )?;
            {
                let mbmi = x.e_mbd.mbmi_mut();
                mbmi.cfl_alpha_idx = cfl_alpha_idx;
                mbmi.cfl_alpha_signs = cfl_alpha_signs;
            }
            if !ok {
                continue;
            }
        } else if is_directional_mode
            && av1_use_angle_delta(x.e_mbd.mbmi().bsize)
            && intra_mode_cfg.enable_angle_delta
        {
            // (the chroma angle delta search: av1_use_angle_delta() needs an
            // 8x8 or larger chroma block, and the speed features' chroma mode
            // mask leaves only the DC and CfL modes)
            return Err(());
        } else {
            if mode == UV_SMOOTH_PRED && cpi.sf.intra_sf.prune_smooth_intra_mode_for_chroma {
                // (should_prune_chroma_smooth_pred_based_on_source_variance():
                // the mode mask leaves no smooth chroma mode)
                return Err(());
            }

            // Predict directly if we don't need to search for angle delta.
            if !av1_txfm_uvrd(cpi, x, &mut tokenonly_rd_stats, bsize, best_rd)? {
                continue;
            }
        }
        let mbmi = *x.e_mbd.mbmi();
        let mode_cost = x.mode_costs.intra_uv_mode_cost[is_cfl_allowed(&x.e_mbd) as usize]
            [mbmi.mode as usize][mode as usize];
        let this_rate =
            tokenonly_rd_stats.rate + intra_mode_info_cost_uv(cpi, x, &mbmi, bsize, mode_cost)?;
        let this_rd = rdcost(x.rdmult, this_rate, tokenonly_rd_stats.dist);

        if this_rd < best_rd {
            best_mbmi = mbmi;
            best_rd = this_rd;
            *rate = this_rate;
            *rate_tokenonly = tokenonly_rd_stats.rate;
            *distortion = tokenonly_rd_stats.dist;
            *skippable = tokenonly_rd_stats.skip_txfm;
        }
    }

    // Search palette mode
    let try_palette = cpi.oxcf.tool_cfg.enable_palette
        && av1_allow_palette(
            cpi.common.features.allow_screen_content_tools,
            x.e_mbd.mbmi().bsize,
        );
    if try_palette {
        // (palettes need screen content tools, which are off)
        return Err(());
    }

    *x.e_mbd.mbmi_mut() = best_mbmi;
    // Make sure we actually chose a mode
    debug_assert!(best_rd < i64::MAX);
    Ok(best_rd)
}

/*\brief Get the intra prediction by searching through tx_type and tx_size.
 *
 * Currently this function is only used in the intra frame code path for
 * winner-mode processing.
 *
 * Returns whether the current mode is an improvement over best_rd.
 */
fn intra_block_yrd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    bmode_costs: &[i32],
    best_rd: &mut i64,
    rate: &mut i32,
    rate_tokenonly: &mut i32,
    distortion: &mut i64,
    skippable: &mut u8,
    best_mbmi: &mut MbModeInfo,
    ctx: &mut PickModeContext,
) -> Result<bool, ()> {
    let mut rd_stats = RdStats::default();
    // In order to improve txfm search avoid rd based breakouts during winner
    // mode evaluation. Hence passing ref_best_rd as a maximum value
    av1_pick_uniform_tx_size_type_yrd(cpi, x, &mut rd_stats, bsize, i64::MAX)?;
    if rd_stats.rate == i32::MAX {
        return Ok(false);
    }
    let mbmi = *x.e_mbd.mbmi();
    let mut this_rate_tokenonly = rd_stats.rate;
    if x.e_mbd.lossless[mbmi.segment_id as usize] == 0 && block_signals_txsize(mbmi.bsize) {
        // av1_pick_uniform_tx_size_type_yrd above includes the cost of the tx_size
        // in the tokenonly rate, but for intra blocks, tx_size is always coded
        // (prediction granularity), so we account for it in the full rate,
        // not the tokenonly rate.
        this_rate_tokenonly -= tx_size_cost(x, bsize, mbmi.tx_size);
    }
    let this_rate = rd_stats.rate
        + intra_mode_info_cost_y(cpi, x, &mbmi, bsize, bmode_costs[mbmi.mode as usize], 0)?;
    let this_rd = rdcost(x.rdmult, this_rate, rd_stats.dist);
    if this_rd < *best_rd {
        *best_mbmi = mbmi;
        *best_rd = this_rd;
        *rate = this_rate;
        *rate_tokenonly = this_rate_tokenonly;
        *distortion = rd_stats.dist;
        *skippable = rd_stats.skip_txfm;
        let n = ctx.num_4x4_blk as usize;
        ctx.blk_skip[..n].copy_from_slice(&x.txfm_search_info.blk_skip[..n]);
        for (i, t) in ctx.tx_type_map[..n].iter_mut().enumerate() {
            *t = x.e_mbd.tx_type_map_get(i);
        }
        return Ok(true);
    }
    Ok(false)
}

// Checks if odd delta angles can be pruned based on rdcosts of even delta
// angles of the corresponding directional mode.
#[inline]
fn prune_luma_odd_delta_angles_using_rd_cost(
    mbmi: &MbModeInfo,
    intra_modes_rd_cost: &[i64; SIZE_OF_ANGLE_DELTA_RD_COST_ARRAY],
    best_rd: i64,
    prune_luma_odd_delta_angles_in_intra: i32,
) -> bool {
    let luma_delta_angle = mbmi.angle_delta[PLANE_TYPE_Y as usize] as i32;
    if prune_luma_odd_delta_angles_in_intra == 0
        || !av1_is_directional_mode(mbmi.mode)
        || (luma_delta_angle.abs() & 1) == 0
        || best_rd == i64::MAX
    {
        return false;
    }

    let rd_thresh = best_rd + (best_rd >> 3);

    // Neighbour rdcosts are considered for pruning of odd delta angles as
    // mentioned below:
    // Delta angle      Delta angle rdcost
    // to be pruned     to be considered
    //    -3                   -2
    //    -1                -2, 0
    //     1                 0, 2
    //     3                    2
    intra_modes_rd_cost[(luma_delta_angle + MAX_ANGLE_DELTA) as usize] > rd_thresh
        && intra_modes_rd_cost[(luma_delta_angle + MAX_ANGLE_DELTA + 2) as usize] > rd_thresh
}

// Finds the best non-intrabc mode on an intra frame.
/// Translation of `av1_rd_pick_intra_sby_mode()`.
pub(crate) fn av1_rd_pick_intra_sby_mode(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rate: &mut i32,
    rate_tokenonly: &mut i32,
    distortion: &mut i64,
    skippable: &mut u8,
    bsize: BlockSize,
    mut best_rd: i64,
    ctx: &mut PickModeContext,
) -> Result<i64, ()> {
    debug_assert!(!is_inter_block(x.e_mbd.mbmi()));
    let mut best_model_rd = i64::MAX;
    let mut directional_mode_skip_mask = [0u8; INTRA_MODES as usize];
    // Flag to check rd of any intra mode is better than best_rd passed to this
    // function
    let mut beat_best_rd = false;
    let intra_mode_cfg = &cpi.oxcf.intra_mode_cfg;
    let try_palette = cpi.oxcf.tool_cfg.enable_palette
        && av1_allow_palette(
            cpi.common.features.allow_screen_content_tools,
            x.e_mbd.mbmi().bsize,
        );
    let above_mi = x.e_mbd.mi_at(x.e_mbd.above_mbmi);
    let left_mi = x.e_mbd.mi_at(x.e_mbd.left_mbmi);
    let a = av1_above_block_mode(above_mi);
    let l = av1_left_block_mode(left_mi);
    let above_ctx = INTRA_MODE_CONTEXT[a as usize] as usize;
    let left_ctx = INTRA_MODE_CONTEXT[l as usize] as usize;
    let bmode_costs = x.mode_costs.y_mode_costs[above_ctx][left_ctx];

    x.e_mbd.mbmi_mut().angle_delta[PLANE_TYPE_Y as usize] = 0;
    let intra_sf = &cpi.sf.intra_sf;
    if intra_sf.intra_pruning_with_hog != 0 {
        // Less aggressive thresholds are used here than those used in inter frame
        // encoding in av1_handle_intra_y_mode() because we want key frames/intra
        // frames to have higher quality.
        let thresh: [f32; 4] = [-1.2, -1.2, -0.6, 0.4];
        let is_chroma = false;
        prune_intra_mode_with_hog(
            x,
            &cpi.source().buffer_alloc,
            bsize,
            thresh[(intra_sf.intra_pruning_with_hog - 1) as usize],
            &mut directional_mode_skip_mask,
            is_chroma,
        )?;
    }
    {
        let mbmi = x.e_mbd.mbmi_mut();
        mbmi.filter_intra_mode_info.use_filter_intra = 0;
        mbmi.palette_mode_info.palette_size[0] = 0;
    }

    // Set params for mode evaluation
    set_mode_eval_params(cpi, x, MODE_EVAL);

    let mut best_mbmi = *x.e_mbd.mbmi();
    // (zero_winner_mode_stats(): x->winner_mode_stats is NULL, the
    // multi-winner processing being off)
    if cpi.sf.winner_mode_sf.multi_winner_mode_type != MULTI_WINNER_MODE_OFF {
        return Err(());
    }
    // (x->winner_mode_count = 0: only read by the multi-winner processing)

    // Searches the intra-modes except for intrabc, palette, and filter_intra.
    let mut top_intra_model_rd = [i64::MAX; TOP_INTRA_MODEL_COUNT];

    // Initialize the rdcost corresponding to all the directional and
    // non-directional intra modes.
    // 1. For directional modes, it stores the rdcost values for delta angles -4,
    // -3, ..., 3, 4.
    // 2. The rdcost value for luma_delta_angle is stored at index
    // luma_delta_angle + MAX_ANGLE_DELTA + 1.
    // 3. The rdcost values for fictitious/nonexistent luma_delta_angle -4 and 4
    // (array indices 0 and 8) are always set to INT64_MAX (the initial value).
    let mut intra_modes_rd_cost =
        [[i64::MAX; SIZE_OF_ANGLE_DELTA_RD_COST_ARRAY]; INTRA_MODE_END as usize];

    let n = ctx.num_4x4_blk as usize;
    for mode_idx in INTRA_MODE_START..LUMA_MODE_COUNT {
        set_y_mode_and_delta_angle(
            mode_idx,
            x.e_mbd.mbmi_mut(),
            intra_sf.prune_luma_odd_delta_angles_in_intra,
        );
        let mut this_rd_stats = RdStats::default();
        let mbmi = *x.e_mbd.mbmi();
        let luma_delta_angle = mbmi.angle_delta[PLANE_TYPE_Y as usize] as i32;

        let is_diagonal_mode = av1_is_diagonal_mode(mbmi.mode);
        if is_diagonal_mode && !intra_mode_cfg.enable_diagonal_intra {
            continue;
        }
        if av1_is_directional_mode(mbmi.mode) && !intra_mode_cfg.enable_directional_intra {
            continue;
        }

        // The smooth prediction mode appears to be more frequently picked
        // than horizontal / vertical smooth prediction modes. Hence treat
        // them differently in speed features.
        if (!intra_mode_cfg.enable_smooth_intra || intra_sf.disable_smooth_intra != 0)
            && (mbmi.mode == SMOOTH_H_PRED || mbmi.mode == SMOOTH_V_PRED)
        {
            continue;
        }
        if !intra_mode_cfg.enable_smooth_intra && mbmi.mode == SMOOTH_PRED {
            continue;
        }

        // The functionality of filter intra modes and smooth prediction
        // overlap. Hence smooth prediction is pruned only if all the
        // filter intra modes are enabled.
        if intra_sf.disable_smooth_intra != 0
            && intra_sf.prune_filter_intra_level == 0
            && mbmi.mode == SMOOTH_PRED
        {
            continue;
        }
        if !intra_mode_cfg.enable_paeth_intra && mbmi.mode == PAETH_PRED {
            continue;
        }

        // Skip the evaluation of modes that do not match with the winner mode in
        // x->mb_mode_cache.
        // (x->use_mb_mode_cache: the mode cache is the RD partition search's)

        let is_directional_mode = av1_is_directional_mode(mbmi.mode);
        if is_directional_mode && directional_mode_skip_mask[mbmi.mode as usize] != 0 {
            continue;
        }
        if is_directional_mode
            && !(av1_use_angle_delta(bsize) && intra_mode_cfg.enable_angle_delta)
            && luma_delta_angle != 0
        {
            continue;
        }

        // Use intra_y_mode_mask speed feature to skip intra mode evaluation.
        if intra_sf.intra_y_mode_mask[MAX_TXSIZE_LOOKUP[bsize as usize] as usize] & (1 << mbmi.mode)
            == 0
        {
            continue;
        }

        if prune_luma_odd_delta_angles_using_rd_cost(
            &mbmi,
            &intra_modes_rd_cost[mbmi.mode as usize],
            best_rd,
            intra_sf.prune_luma_odd_delta_angles_in_intra,
        ) {
            continue;
        }

        let tx_size = TX_32X32.min(MAX_TXSIZE_LOOKUP[bsize as usize]);
        let this_model_rd = intra_model_rd(
            &cpi.common,
            x,
            &cpi.source().buffer_alloc,
            0,
            bsize,
            tx_size,
            /*use_hadamard=*/ true,
        )?;

        let model_rd_index_for_pruning = get_model_rd_index_for_pruning(intra_sf)?;

        if prune_intra_y_mode(
            this_model_rd,
            &mut best_model_rd,
            &mut top_intra_model_rd,
            intra_sf.top_intra_model_count_allowed,
            model_rd_index_for_pruning,
        ) {
            continue;
        }

        // Builds the actual prediction. The prediction from
        // model_intra_yrd_and_prune was just an estimation that did not take into
        // account the effect of txfm pipeline, so we need to redo it for real
        // here.
        av1_pick_uniform_tx_size_type_yrd(cpi, x, &mut this_rd_stats, bsize, best_rd)?;
        let mut this_rate_tokenonly = this_rd_stats.rate;
        let this_distortion = this_rd_stats.dist;
        let s = this_rd_stats.skip_txfm;

        if this_rate_tokenonly == i32::MAX {
            continue;
        }

        let mbmi = *x.e_mbd.mbmi();
        if x.e_mbd.lossless[mbmi.segment_id as usize] == 0 && block_signals_txsize(mbmi.bsize) {
            // av1_pick_uniform_tx_size_type_yrd above includes the cost of the
            // tx_size in the tokenonly rate, but for intra blocks, tx_size is always
            // coded (prediction granularity), so we account for it in the full rate,
            // not the tokenonly rate.
            this_rate_tokenonly -= tx_size_cost(x, bsize, mbmi.tx_size);
        }
        let this_rate = this_rd_stats.rate
            + intra_mode_info_cost_y(cpi, x, &mbmi, bsize, bmode_costs[mbmi.mode as usize], 0)?;
        let this_rd = rdcost(x.rdmult, this_rate, this_distortion);

        // Visual quality adjustment based on recon vs source variance.
        if cpi.oxcf.mode == ALLINTRA && this_rd != i64::MAX {
            // (intra_rd_variance_factor(): the all intra mode's)
            return Err(());
        }

        intra_modes_rd_cost[mbmi.mode as usize]
            [(luma_delta_angle + MAX_ANGLE_DELTA + 1) as usize] = this_rd;

        // Collect mode stats for multiwinner mode processing
        store_winner_mode_stats(cpi.sf.winner_mode_sf.multi_winner_mode_type)?;
        if this_rd < best_rd {
            best_mbmi = mbmi;
            best_rd = this_rd;
            // Setting beat_best_rd flag because current mode rd is better than
            // best_rd passed to this function
            beat_best_rd = true;
            *rate = this_rate;
            *rate_tokenonly = this_rate_tokenonly;
            *distortion = this_distortion;
            *skippable = s;
            ctx.blk_skip[..n].copy_from_slice(&x.txfm_search_info.blk_skip[..n]);
            for (i, t) in ctx.tx_type_map[..n].iter_mut().enumerate() {
                *t = x.e_mbd.tx_type_map_get(i);
            }
        }
    }

    // Searches palette
    if try_palette {
        // (palettes need screen content tools, which are off)
        return Err(());
    }

    // Searches filter_intra
    if beat_best_rd
        && av1_filter_intra_allowed_bsize(&cpi.common, bsize)
        && rd_pick_filter_intra_sby(
            cpi,
            x,
            rate,
            rate_tokenonly,
            distortion,
            skippable,
            bsize,
            bmode_costs[DC_PRED as usize],
            best_mbmi.mode,
            &mut best_rd,
            &mut best_model_rd,
            ctx,
        )?
    {
        best_mbmi = *x.e_mbd.mbmi();
    }

    // No mode is identified with less rd value than best_rd passed to this
    // function. In such cases winner mode processing is not necessary and return
    // best_rd as INT64_MAX to indicate best mode is not identified
    if !beat_best_rd {
        return Ok(i64::MAX);
    }

    // In multi-winner mode processing, perform tx search for few best modes
    // identified during mode evaluation. Winner mode processing uses best tx
    // configuration for tx search.
    // (multi_winner_mode_type is MULTI_WINNER_MODE_OFF: checked above)
    // If previous searches use only the default tx type/no R-D optimization of
    // quantized coeffs, do an extra search for the best tx type/better R-D
    // optimization of quantized coeffs
    let mbmi = *x.e_mbd.mbmi();
    if is_winner_mode_processing_enabled(cpi, x, &mbmi, 0)? {
        // Set params for winner mode evaluation
        set_mode_eval_params(cpi, x, WINNER_MODE_EVAL);
        *x.e_mbd.mbmi_mut() = best_mbmi;
        intra_block_yrd(
            cpi,
            x,
            bsize,
            &bmode_costs,
            &mut best_rd,
            rate,
            rate_tokenonly,
            distortion,
            skippable,
            &mut best_mbmi,
            ctx,
        )?;
    }
    *x.e_mbd.mbmi_mut() = best_mbmi;
    for (i, &t) in ctx.tx_type_map[..n].iter().enumerate() {
        x.e_mbd.tx_type_map_set(i, t);
    }
    Ok(best_rd)
}
