// Rust translation of the parts of av1/encoder/rd.c and av1/encoder/rd.h
// from aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Rate-distortion constants: lambda (`RDMULT`), the mode thresholds, the
//! costs of the symbols, motion vectors and coefficients, and the
//! sad-per-bit factors.

use super::block::*;
use super::cost::*;
use super::enc_enums::*;
use super::encodemv::av1_build_nmv_cost_table;
use super::encodemv::av1_get_ref_mv_from_stack;
use super::encoder::RdOpt;
use super::encoder::{is_stat_consumption_stage, Av1Comp, ThreadData};
use super::nonrd_pickmode::{
    INTER_MODE_LIST, INTRA_MODE_LIST, MODE_IDX, RTC_INTER_MODES, RTC_INTRA_MODES, RTC_MODES,
};
use super::ratectrl::av1_convert_qindex_to_q;
use super::speed_features::*;
use crate::aom::aom_dsp::aom_dsp_common::clamp;
use crate::aom::aom_dsp::aom_dsp_common::get_msb;
use crate::aom::aom_dsp::aom_dsp_common::round_power_of_two_i64;
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;
use crate::aom::av1::common::av1_common_int::get_ref_frame_buf;
use crate::aom::av1::common::av1_common_int::{av1_num_planes, frame_is_intra_only, Av1Common};
use crate::aom::av1::common::blockd::KEY_FRAME;
use crate::aom::av1::common::blockd::{Buf2d, MacroBlockD, MAX_MB_PLANE};
use crate::aom::av1::common::entropy::*;
use crate::aom::av1::common::entropymode::*;
use crate::aom::av1::common::entropymv::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::SWITCHABLE_FILTER_CONTEXTS;
use crate::aom::av1::common::filter::{av1_extract_interp_filter, InterpFilter, SWITCHABLE};
use crate::aom::av1::common::mv::{get_mv_subpel, Mv, SubpelMvLimits};
use crate::aom::av1::common::mvref_common::clamp_mv;
use crate::aom::av1::common::pred_common::av1_get_pred_context_switchable_interp;
use crate::aom::av1::common::quant_common::{av1_dc_quant_qtx, av1_get_qindex, MAXQ};
use crate::aom::av1::common::reconinter::{setup_pred_plane, AOM_INTERP_EXTEND};
use crate::aom::av1::common::reconintra::av1_filter_intra_allowed_bsize;
use crate::aom::av1::common::scale::ScaleFactors;
use crate::aom::av1::common::seg_common::{
    MAX_SEGMENTS, SEG_TEMPORAL_PRED_CTXS, SPATIAL_PREDICTION_PROBS,
};

pub(crate) const RDDIV_BITS: i32 = 7;
pub(crate) const RD_EPB_SHIFT: i32 = 6;

/// `RDCOST(RM, R, D)` (`RM` is an `int` or, in the trellis, an `int64_t`;
/// the arithmetic wraps as the C code's does in practice)
#[inline]
pub(crate) fn rdcost<T: Into<i64>>(rm: T, r: i32, d: i64) -> i64 {
    let rm: i64 = rm.into();
    ((r as i64)
        .wrapping_mul(rm)
        .wrapping_add(1 << (AV1_PROB_COST_SHIFT - 1))
        >> AV1_PROB_COST_SHIFT)
        .wrapping_add(d.wrapping_mul(1 << RDDIV_BITS))
}

/// `RDCOST_NEG_R(RM, R, D)`
#[inline]
pub(crate) fn rdcost_neg_r(rm: i32, r: i32, d: i64) -> i64 {
    (d.wrapping_mul(1 << RDDIV_BITS)).wrapping_sub(
        ((r as i64).wrapping_mul(rm as i64) + (1 << (AV1_PROB_COST_SHIFT - 1)))
            >> AV1_PROB_COST_SHIFT,
    )
}

/// Translation of `av1_init_rd_stats()`.
#[inline]
pub(crate) fn av1_init_rd_stats(rd_stats: &mut RdStats) {
    rd_stats.rate = 0;
    rd_stats.dist = 0;
    rd_stats.rdcost = 0;
    rd_stats.sse = 0;
    rd_stats.skip_txfm = 1;
    rd_stats.zero_rate = 0;
}

/// Translation of `av1_merge_rd_stats()`.
#[inline]
pub(crate) fn av1_merge_rd_stats(rd_stats_dst: &mut RdStats, rd_stats_src: &RdStats) {
    if rd_stats_dst.rate == i32::MAX || rd_stats_src.rate == i32::MAX {
        // If rd_stats_dst or rd_stats_src has invalid rate, we will make
        // rd_stats_dst invalid.
        av1_invalid_rd_stats(rd_stats_dst);
        return;
    }
    rd_stats_dst.rate =
        (rd_stats_dst.rate as i64 + rd_stats_src.rate as i64).min(i32::MAX as i64) as i32;
    if rd_stats_dst.zero_rate == 0 {
        rd_stats_dst.zero_rate = rd_stats_src.zero_rate;
    }
    rd_stats_dst.dist += rd_stats_src.dist;
    if rd_stats_dst.sse < i64::MAX && rd_stats_src.sse < i64::MAX {
        rd_stats_dst.sse += rd_stats_src.sse;
    }
    rd_stats_dst.skip_txfm &= rd_stats_src.skip_txfm;
}

/// Translation of `av1_calculate_rd_cost()`.
#[inline]
pub(crate) fn av1_calculate_rd_cost(mult: i32, rate: i32, dist: i64) -> i64 {
    debug_assert!(mult >= 0);
    if rate >= 0 {
        return rdcost(mult, rate, dist);
    }
    rdcost_neg_r(mult, -rate, dist)
}

/// Translation of `av1_rd_cost_update()`.
#[inline]
pub(crate) fn av1_rd_cost_update(mult: i32, rd_cost: &mut RdStats) {
    if rd_cost.rate < i32::MAX && rd_cost.dist < i64::MAX && rd_cost.rdcost < i64::MAX {
        rd_cost.rdcost = av1_calculate_rd_cost(mult, rd_cost.rate, rd_cost.dist);
    } else {
        av1_invalid_rd_stats(rd_cost);
    }
}

#[inline]
pub(crate) fn av1_invalid_rd_stats(rd_stats: &mut RdStats) {
    rd_stats.rate = i32::MAX;
    rd_stats.dist = i64::MAX;
    rd_stats.rdcost = i64::MAX;
    rd_stats.sse = i64::MAX;
    rd_stats.skip_txfm = 0;
    rd_stats.zero_rate = 0;
}

#[inline]
pub(crate) fn av1_set_error_per_bit(errorperbit: &mut i32, rdmult: i32) {
    *errorperbit = (rdmult >> RD_EPB_SHIFT).max(1);
}

// (rd.c)

static USE_INTRA_EXT_TX_FOR_TXSIZE: [[i32; EXT_TX_SIZES as usize]; EXT_TX_SETS_INTRA as usize] = [
    [1, 1, 1, 1], // unused
    [1, 1, 0, 0],
    [0, 0, 1, 0],
];

static AV1_EXT_TX_SET_IDX_TO_TYPE_INTRA: [TxSetType; EXT_TX_SETS_INTRA as usize] = [
    // Intra
    EXT_TX_SET_DCTONLY,
    EXT_TX_SET_DTT4_IDTX_1DDCT,
    EXT_TX_SET_DTT4_IDTX,
];

/// `CFL_SIGN_U(js)`
#[inline]
fn cfl_sign_u(js: i32) -> i32 {
    ((js + 1) * 11) >> 5
}
/// `CFL_SIGN_V(js)`
#[inline]
fn cfl_sign_v(js: i32) -> i32 {
    (js + 1) - CFL_SIGNS * cfl_sign_u(js)
}
/// `CFL_CONTEXT_U(js)`
#[inline]
fn cfl_context_u(js: i32) -> i32 {
    js + 1 - CFL_SIGNS
}
/// `CFL_CONTEXT_V(js)`
#[inline]
fn cfl_context_v(js: i32) -> i32 {
    cfl_sign_v(js) * CFL_SIGNS + cfl_sign_u(js) - CFL_SIGNS
}

/// Translation of `av1_fill_mode_rates()`.
pub(crate) fn av1_fill_mode_rates(cm: &Av1Common, mode_costs: &mut ModeCosts, fc: &FrameContext) {
    for i in 0..PARTITION_CONTEXTS as usize {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.partition_cost[i],
            &fc.partition_cdf[i],
            None,
        );
    }

    if cm.current_frame.skip_mode_info.skip_mode_flag != 0 {
        for i in 0..SKIP_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.skip_mode_cost[i],
                &fc.skip_mode_cdfs[i],
                None,
            );
        }
    }

    for i in 0..SKIP_CONTEXTS as usize {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.skip_txfm_cost[i],
            &fc.skip_txfm_cdfs[i],
            None,
        );
    }

    for i in 0..KF_MODE_CONTEXTS as usize {
        for j in 0..KF_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(&mut mode_costs.y_mode_costs[i][j], &fc.kf_y_cdf[i][j], None);
        }
    }

    for i in 0..BLOCK_SIZE_GROUPS as usize {
        av1_cost_tokens_from_cdf(&mut mode_costs.mbmode_cost[i], &fc.y_mode_cdf[i], None);
    }
    for i in 0..CFL_ALLOWED_TYPES as usize {
        for j in 0..INTRA_MODES as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.intra_uv_mode_cost[i][j],
                &fc.uv_mode_cdf[i][j],
                None,
            );
        }
    }

    av1_cost_tokens_from_cdf(
        &mut mode_costs.filter_intra_mode_cost,
        &fc.filter_intra_mode_cdf,
        None,
    );
    for i in 0..BLOCK_SIZES_ALL {
        if av1_filter_intra_allowed_bsize(cm, i) {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.filter_intra_cost[i as usize],
                &fc.filter_intra_cdfs[i as usize],
                None,
            );
        }
    }

    for i in 0..SWITCHABLE_FILTER_CONTEXTS as usize {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.switchable_interp_costs[i],
            &fc.switchable_interp_cdf[i],
            None,
        );
    }

    for i in 0..PALATTE_BSIZE_CTXS as usize {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.palette_y_size_cost[i],
            &fc.palette_y_size_cdf[i],
            None,
        );
        av1_cost_tokens_from_cdf(
            &mut mode_costs.palette_uv_size_cost[i],
            &fc.palette_uv_size_cdf[i],
            None,
        );
        for j in 0..PALETTE_Y_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.palette_y_mode_cost[i][j],
                &fc.palette_y_mode_cdf[i][j],
                None,
            );
        }
    }

    for i in 0..PALETTE_UV_MODE_CONTEXTS as usize {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.palette_uv_mode_cost[i],
            &fc.palette_uv_mode_cdf[i],
            None,
        );
    }

    for i in 0..PALETTE_SIZES as usize {
        for j in 0..PALETTE_COLOR_INDEX_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.palette_y_color_cost[i][j],
                &fc.palette_y_color_index_cdf[i][j],
                None,
            );
            av1_cost_tokens_from_cdf(
                &mut mode_costs.palette_uv_color_cost[i][j],
                &fc.palette_uv_color_index_cdf[i][j],
                None,
            );
        }
    }

    let mut sign_cost = [0i32; CFL_JOINT_SIGNS as usize];
    av1_cost_tokens_from_cdf(&mut sign_cost, &fc.cfl_sign_cdf, None);
    for joint_sign in 0..CFL_JOINT_SIGNS {
        let js = joint_sign as usize;
        if cfl_sign_u(joint_sign) == CFL_SIGN_ZERO {
            mode_costs.cfl_cost[js][CFL_PRED_U as usize] = [0; CFL_ALPHABET_SIZE as usize];
        } else {
            let cdf_u = &fc.cfl_alpha_cdf[cfl_context_u(joint_sign) as usize];
            av1_cost_tokens_from_cdf(
                &mut mode_costs.cfl_cost[js][CFL_PRED_U as usize],
                cdf_u,
                None,
            );
        }
        if cfl_sign_v(joint_sign) == CFL_SIGN_ZERO {
            mode_costs.cfl_cost[js][CFL_PRED_V as usize] = [0; CFL_ALPHABET_SIZE as usize];
        } else {
            let cdf_v = &fc.cfl_alpha_cdf[cfl_context_v(joint_sign) as usize];
            av1_cost_tokens_from_cdf(
                &mut mode_costs.cfl_cost[js][CFL_PRED_V as usize],
                cdf_v,
                None,
            );
        }
        for u in 0..CFL_ALPHABET_SIZE as usize {
            mode_costs.cfl_cost[js][CFL_PRED_U as usize][u] += sign_cost[js];
        }
    }

    for i in 0..MAX_TX_CATS as usize {
        for j in 0..TX_SIZE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.tx_size_cost[i][j],
                &fc.tx_size_cdf[i][j],
                None,
            );
        }
    }

    for i in 0..TXFM_PARTITION_CONTEXTS as usize {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.txfm_partition_cost[i],
            &fc.txfm_partition_cdf[i],
            None,
        );
    }

    for i in TX_4X4..EXT_TX_SIZES {
        let i = i as usize;
        for s in 1..EXT_TX_SETS_INTER as usize {
            if USE_INTER_EXT_TX_FOR_TXSIZE[s][i] != 0 {
                av1_cost_tokens_from_cdf(
                    &mut mode_costs.inter_tx_type_costs[s][i],
                    &fc.inter_ext_tx_cdf[s][i],
                    Some(&AV1_EXT_TX_INV[AV1_EXT_TX_SET_IDX_TO_TYPE[1][s] as usize]),
                );
            }
        }
        for s in 1..EXT_TX_SETS_INTRA as usize {
            if USE_INTRA_EXT_TX_FOR_TXSIZE[s][i] != 0 {
                for j in 0..INTRA_MODES as usize {
                    av1_cost_tokens_from_cdf(
                        &mut mode_costs.intra_tx_type_costs[s][i][j],
                        &fc.intra_ext_tx_cdf[s][i][j],
                        Some(&AV1_EXT_TX_INV[AV1_EXT_TX_SET_IDX_TO_TYPE[0][s] as usize]),
                    );
                }
            }
        }
    }
    for i in 0..DIRECTIONAL_MODES as usize {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.angle_delta_cost[i],
            &fc.angle_delta_cdf[i],
            None,
        );
    }
    av1_cost_tokens_from_cdf(&mut mode_costs.intrabc_cost, &fc.intrabc_cdf, None);

    for i in 0..SPATIAL_PREDICTION_PROBS {
        av1_cost_tokens_from_cdf(
            &mut mode_costs.spatial_pred_cost[i],
            &fc.seg.spatial_pred_seg_cdf[i],
            None,
        );
    }

    for i in 0..SEG_TEMPORAL_PRED_CTXS {
        av1_cost_tokens_from_cdf(&mut mode_costs.tmp_pred_cost[i], &fc.seg.pred_cdf[i], None);
    }

    if !frame_is_intra_only(cm) {
        for i in 0..COMP_INTER_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.comp_inter_cost[i],
                &fc.comp_inter_cdf[i],
                None,
            );
        }

        for i in 0..REF_CONTEXTS as usize {
            for j in 0..(SINGLE_REFS - 1) as usize {
                av1_cost_tokens_from_cdf(
                    &mut mode_costs.single_ref_cost[i][j],
                    &fc.single_ref_cdf[i][j],
                    None,
                );
            }
        }

        for i in 0..COMP_REF_TYPE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.comp_ref_type_cost[i],
                &fc.comp_ref_type_cdf[i],
                None,
            );
        }

        for i in 0..UNI_COMP_REF_CONTEXTS as usize {
            for j in 0..(UNIDIR_COMP_REFS - 1) as usize {
                av1_cost_tokens_from_cdf(
                    &mut mode_costs.uni_comp_ref_cost[i][j],
                    &fc.uni_comp_ref_cdf[i][j],
                    None,
                );
            }
        }

        for i in 0..REF_CONTEXTS as usize {
            for j in 0..(FWD_REFS - 1) as usize {
                av1_cost_tokens_from_cdf(
                    &mut mode_costs.comp_ref_cost[i][j],
                    &fc.comp_ref_cdf[i][j],
                    None,
                );
            }
        }

        for i in 0..REF_CONTEXTS as usize {
            for j in 0..(BWD_REFS - 1) as usize {
                av1_cost_tokens_from_cdf(
                    &mut mode_costs.comp_bwdref_cost[i][j],
                    &fc.comp_bwdref_cdf[i][j],
                    None,
                );
            }
        }

        for i in 0..INTRA_INTER_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.intra_inter_cost[i],
                &fc.intra_inter_cdf[i],
                None,
            );
        }

        for i in 0..NEWMV_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(&mut mode_costs.newmv_mode_cost[i], &fc.newmv_cdf[i], None);
        }

        for i in 0..GLOBALMV_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(&mut mode_costs.zeromv_mode_cost[i], &fc.zeromv_cdf[i], None);
        }

        for i in 0..REFMV_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(&mut mode_costs.refmv_mode_cost[i], &fc.refmv_cdf[i], None);
        }

        for i in 0..DRL_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(&mut mode_costs.drl_mode_cost0[i], &fc.drl_cdf[i], None);
        }
        for i in 0..INTER_MODE_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.inter_compound_mode_cost[i],
                &fc.inter_compound_mode_cdf[i],
                None,
            );
        }
        for i in 0..BLOCK_SIZES_ALL as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.compound_type_cost[i],
                &fc.compound_type_cdf[i],
                None,
            );
        }
        for i in 0..BLOCK_SIZES_ALL {
            if av1_is_wedge_used(i) {
                av1_cost_tokens_from_cdf(
                    &mut mode_costs.wedge_idx_cost[i as usize],
                    &fc.wedge_idx_cdf[i as usize],
                    None,
                );
            }
        }
        for i in 0..BLOCK_SIZE_GROUPS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.interintra_cost[i],
                &fc.interintra_cdf[i],
                None,
            );
            av1_cost_tokens_from_cdf(
                &mut mode_costs.interintra_mode_cost[i],
                &fc.interintra_mode_cdf[i],
                None,
            );
        }
        for i in 0..BLOCK_SIZES_ALL as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.wedge_interintra_cost[i],
                &fc.wedge_interintra_cdf[i],
                None,
            );
        }
        for i in BLOCK_8X8 as usize..BLOCK_SIZES_ALL as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.motion_mode_cost[i],
                &fc.motion_mode_cdf[i],
                None,
            );
        }
        for i in BLOCK_8X8 as usize..BLOCK_SIZES_ALL as usize {
            av1_cost_tokens_from_cdf(&mut mode_costs.motion_mode_cost1[i], &fc.obmc_cdf[i], None);
        }
        for i in 0..COMP_INDEX_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.comp_idx_cost[i],
                &fc.compound_index_cdf[i],
                None,
            );
        }
        for i in 0..COMP_GROUP_IDX_CONTEXTS as usize {
            av1_cost_tokens_from_cdf(
                &mut mode_costs.comp_group_idx_cost[i],
                &fc.comp_group_idx_cdf[i],
                None,
            );
        }
    }
}

/// `av1_is_wedge_used()` (reconinter.h): whether `av1_wedge_params_lookup`
/// has wedge types for the block size.
#[inline]
fn av1_is_wedge_used(sb_type: BlockSize) -> bool {
    matches!(
        sb_type,
        BLOCK_8X8
            | BLOCK_8X16
            | BLOCK_16X8
            | BLOCK_16X16
            | BLOCK_16X32
            | BLOCK_32X16
            | BLOCK_32X32
            | BLOCK_8X32
            | BLOCK_32X8
    )
}

// Returns the default rd multiplier for inter frames for a given qindex.
// The function here is a first pass estimate based on data from
// a previous Vizer run
fn def_inter_rd_multiplier(qindex: i32) -> f64 {
    3.2 + (0.0015 * qindex as f64)
}

// Returns the default rd multiplier for ARF/Golden Frames for a given qindex.
// The function here is a first pass estimate based on data from
// a previous Vizer run
fn def_arf_rd_multiplier(qindex: i32) -> f64 {
    3.25 + (0.0015 * qindex as f64)
}

// Returns the default rd multiplier for key frames for a given qindex.
// The function here is a first pass estimate based on data from
// a previous Vizer run
fn def_kf_rd_multiplier(qindex: i32) -> f64 {
    3.3 + (0.0015 * qindex as f64)
}

static RD_BOOST_FACTOR: [i32; 16] = [64, 32, 32, 32, 24, 16, 12, 12, 8, 8, 4, 4, 2, 2, 1, 0];

static RD_LAYER_DEPTH_FACTOR: [i32; 7] = [160, 160, 160, 160, 192, 208, 224];

pub(crate) fn av1_compute_rd_mult_based_on_qindex(
    bit_depth: i32,
    update_type: i32,
    qindex: i32,
) -> i32 {
    use super::ratectrl::{ARF_UPDATE, GF_UPDATE, KF_UPDATE};
    let q = av1_dc_quant_qtx(qindex, 0, bit_depth) as i32;
    let mut rdmult: i64 = (q * q) as i64;
    if update_type == KF_UPDATE as i32 {
        let def_rd_q_mult = def_kf_rd_multiplier(q);
        rdmult = (rdmult as f64 * def_rd_q_mult) as i64;
    } else if update_type == GF_UPDATE as i32 || update_type == ARF_UPDATE as i32 {
        let def_rd_q_mult = def_arf_rd_multiplier(q);
        rdmult = (rdmult as f64 * def_rd_q_mult) as i64;
    } else {
        let def_rd_q_mult = def_inter_rd_multiplier(q);
        rdmult = (rdmult as f64 * def_rd_q_mult) as i64;
    }

    match bit_depth {
        8 => {}
        10 => rdmult = round_power_of_two_i64(rdmult, 4),
        12 => rdmult = round_power_of_two_i64(rdmult, 8),
        _ => {
            debug_assert!(
                false,
                "bit_depth should be AOM_BITS_8, AOM_BITS_10 or AOM_BITS_12"
            );
            return -1;
        }
    }
    if rdmult > 0 {
        rdmult.min(i32::MAX as i64) as i32
    } else {
        1
    }
}

pub(crate) fn av1_compute_rd_mult(
    qindex: i32,
    bit_depth: i32,
    update_type: i32,
    layer_depth: i32,
    boost_index: i32,
    frame_type: i32,
    use_fixed_qp_offsets: i32,
    is_stat_consumption_stage: bool,
) -> i32 {
    let mut rdmult: i64 =
        av1_compute_rd_mult_based_on_qindex(bit_depth, update_type, qindex) as i64;
    if is_stat_consumption_stage && use_fixed_qp_offsets == 0 && frame_type != KEY_FRAME {
        // Layer depth adjustment
        rdmult = (rdmult * RD_LAYER_DEPTH_FACTOR[layer_depth as usize] as i64) >> 7;
        // ARF boost adjustment
        rdmult += (rdmult * RD_BOOST_FACTOR[boost_index as usize] as i64) >> 7;
    }
    rdmult as i32
}

pub(crate) fn av1_fill_coeff_costs(
    coeff_costs: &mut CoeffCosts,
    fc: &FrameContext,
    num_planes: usize,
) {
    let nplanes = num_planes.min(PLANE_TYPES as usize);
    for eob_multi_size in 0..7 {
        for plane in 0..nplanes {
            let pcost = &mut coeff_costs.eob_costs[eob_multi_size][plane];

            for ctx in 0..2 {
                let pcdf: &[u16] = match eob_multi_size {
                    0 => &fc.eob_flag_cdf16[plane][ctx],
                    1 => &fc.eob_flag_cdf32[plane][ctx],
                    2 => &fc.eob_flag_cdf64[plane][ctx],
                    3 => &fc.eob_flag_cdf128[plane][ctx],
                    4 => &fc.eob_flag_cdf256[plane][ctx],
                    5 => &fc.eob_flag_cdf512[plane][ctx],
                    _ => &fc.eob_flag_cdf1024[plane][ctx],
                };
                av1_cost_tokens_from_cdf(&mut pcost.eob_cost[ctx], pcdf, None);
            }
        }
    }
    for tx_size in 0..TX_SIZES as usize {
        for plane in 0..nplanes {
            let pcost = &mut coeff_costs.coeff_costs[tx_size][plane];

            for ctx in 0..TXB_SKIP_CONTEXTS as usize {
                av1_cost_tokens_from_cdf(
                    &mut pcost.txb_skip_cost[ctx],
                    &fc.txb_skip_cdf[tx_size][ctx],
                    None,
                );
            }

            for ctx in 0..SIG_COEF_CONTEXTS_EOB as usize {
                av1_cost_tokens_from_cdf(
                    &mut pcost.base_eob_cost[ctx],
                    &fc.coeff_base_eob_cdf[tx_size][plane][ctx],
                    None,
                );
            }
            for ctx in 0..SIG_COEF_CONTEXTS as usize {
                av1_cost_tokens_from_cdf(
                    &mut pcost.base_cost[ctx],
                    &fc.coeff_base_cdf[tx_size][plane][ctx],
                    None,
                );
            }

            for ctx in 0..SIG_COEF_CONTEXTS as usize {
                pcost.base_cost[ctx][4] = 0;
                pcost.base_cost[ctx][5] =
                    pcost.base_cost[ctx][1] + av1_cost_literal(1) - pcost.base_cost[ctx][0];
                pcost.base_cost[ctx][6] = pcost.base_cost[ctx][2] - pcost.base_cost[ctx][1];
                pcost.base_cost[ctx][7] = pcost.base_cost[ctx][3] - pcost.base_cost[ctx][2];
            }

            for ctx in 0..EOB_COEF_CONTEXTS as usize {
                av1_cost_tokens_from_cdf(
                    &mut pcost.eob_extra_cost[ctx],
                    &fc.eob_extra_cdf[tx_size][plane][ctx],
                    None,
                );
            }

            for ctx in 0..DC_SIGN_CONTEXTS as usize {
                av1_cost_tokens_from_cdf(
                    &mut pcost.dc_sign_cost[ctx],
                    &fc.dc_sign_cdf[plane][ctx],
                    None,
                );
            }

            for ctx in 0..LEVEL_CONTEXTS as usize {
                let mut br_rate = [0i32; BR_CDF_SIZE as usize];
                let mut prev_cost = 0;
                av1_cost_tokens_from_cdf(
                    &mut br_rate,
                    &fc.coeff_br_cdf[tx_size.min(TX_32X32 as usize)][plane][ctx],
                    None,
                );
                // printf("br_rate: ");
                // for(j = 0; j < BR_CDF_SIZE; j++)
                //  printf("%4d ", br_rate[j]);
                // printf("\n");
                let mut i = 0usize;
                while i < COEFF_BASE_RANGE as usize {
                    let mut j = 0usize;
                    while j < BR_CDF_SIZE as usize - 1 {
                        pcost.lps_cost[ctx][i + j] = prev_cost + br_rate[j];
                        j += 1;
                    }
                    prev_cost += br_rate[j];
                    i += BR_CDF_SIZE as usize - 1;
                }
                pcost.lps_cost[ctx][i] = prev_cost;
                // printf("lps_cost: %d %d %2d : ", tx_size, plane, ctx);
                // for (i = 0; i <= COEFF_BASE_RANGE; i++)
                //  printf("%5d ", pcost->lps_cost[ctx][i]);
                // printf("\n");
            }
            for ctx in 0..LEVEL_CONTEXTS as usize {
                pcost.lps_cost[ctx][COEFF_BASE_RANGE as usize + 1] = pcost.lps_cost[ctx][0];
                for i in 1..=COEFF_BASE_RANGE as usize {
                    pcost.lps_cost[ctx][i + COEFF_BASE_RANGE as usize + 1] =
                        pcost.lps_cost[ctx][i] - pcost.lps_cost[ctx][i - 1];
                }
            }
        }
    }
}

/// `init_me_luts_bd()` for one entry: the sad per bit factor of a
/// quantizer index (libaom fills `sad_per_bit_lut_8/10/12` once with these
/// values, in `av1_init_me_luts()`; computing the entry when it is read
/// gives the same numbers).
// Initialize the sad lut tables using a formulaic calculation for now.
// This is to make it easier to resolve the impact of experimental changes
// to the quantizer tables.
fn sad_per_bit_lut(qindex: i32, bit_depth: i32) -> i32 {
    let q = av1_convert_qindex_to_q(qindex, bit_depth);
    (0.0418 * q + 2.4107) as i32
}

const RD_THRESH_POW: f64 = 1.25;

// The baseline rd thresholds for breaking out of the rd loop for
// certain modes are assumed to be based on 8x8 blocks.
// This table is used to correct for block size.
// The factors here are << 2 (2 = x0.5, 32 = x8 etc).
static RD_THRESH_BLOCK_SIZE_FACTOR: [u8; BLOCK_SIZES_ALL as usize] = [
    2, 3, 3, 4, 6, 6, 8, 12, 12, 16, 24, 24, 32, 48, 48, 64, 4, 4, 8, 8, 16, 16,
];

static USE_INTER_EXT_TX_FOR_TXSIZE: [[i32; EXT_TX_SIZES as usize]; EXT_TX_SETS_INTER as usize] = [
    [1, 1, 1, 1], // unused
    [1, 1, 0, 0],
    [0, 0, 1, 0],
    [0, 1, 1, 1],
];

static AV1_EXT_TX_SET_IDX_TO_TYPE: [[TxSetType; 4]; 2] = [
    [
        // Intra
        EXT_TX_SET_DCTONLY,
        EXT_TX_SET_DTT4_IDTX_1DDCT,
        EXT_TX_SET_DTT4_IDTX,
        0,
    ],
    [
        // Inter
        EXT_TX_SET_DCTONLY,
        EXT_TX_SET_ALL16,
        EXT_TX_SET_DTT9_IDTX_1DDCT,
        EXT_TX_SET_DCT_IDTX,
    ],
];

/// `RTC_REFS` (rd.h)
pub(crate) const RTC_REFS: usize = 4;
pub(crate) static REAL_TIME_REF_COMBOS: [[MvReferenceFrame; 2]; RTC_REFS] = [
    [LAST_FRAME, NONE_FRAME],
    [ALTREF_FRAME, NONE_FRAME],
    [GOLDEN_FRAME, NONE_FRAME],
    [INTRA_FRAME, NONE_FRAME],
];

/// Translation of `mode_offset()` (rd.h).
#[inline]
pub(crate) fn mode_offset(mode: PredictionMode) -> Result<usize, ()> {
    if mode >= NEARESTMV {
        Ok((mode - NEARESTMV) as usize)
    } else {
        match mode {
            DC_PRED => Ok(0),
            V_PRED => Ok(1),
            H_PRED => Ok(2),
            SMOOTH_PRED => Ok(3),
            _ => Err(()),
        }
    }
}

/// Translation of `compute_rd_thresh_factor()`.
fn compute_rd_thresh_factor(qindex: i32, bit_depth: i32) -> i32 {
    let q = match bit_depth {
        8 => av1_dc_quant_qtx(qindex, 0, 8) as f64 / 4.0,
        10 => av1_dc_quant_qtx(qindex, 0, 10) as f64 / 16.0,
        12 => av1_dc_quant_qtx(qindex, 0, 12) as f64 / 64.0,
        _ => {
            debug_assert!(
                false,
                "bit_depth should be AOM_BITS_8, AOM_BITS_10 or AOM_BITS_12"
            );
            return -1;
        }
    };
    // TODO(debargha): Adjust the function below.
    ((q.powf(RD_THRESH_POW) * 5.12) as i32).max(8)
}

/// Translation of `av1_set_sad_per_bit()`.
pub(crate) fn av1_set_sad_per_bit(cpi: &Av1Comp, sadperbit: &mut i32, qindex: i32) {
    let bd = cpi.common.seq_params.bit_depth;
    match bd {
        8 | 10 | 12 => *sadperbit = sad_per_bit_lut(qindex, bd),
        _ => debug_assert!(
            false,
            "bit_depth should be AOM_BITS_8, AOM_BITS_10 or AOM_BITS_12"
        ),
    }
}

/// Translation of `set_block_thresholds()`.
fn set_block_thresholds(
    cm: &Av1Common,
    rd: &mut RdOpt,
    use_nonrd_pick_mode: bool,
) -> Result<(), ()> {
    let mut mode_indices = [0i32; RTC_REFS * RTC_MODES];
    let mut num_modes_count = if use_nonrd_pick_mode {
        0
    } else {
        MAX_MODES as usize
    };

    if use_nonrd_pick_mode {
        for r_idx in 0..RTC_REFS {
            let ref_ = REAL_TIME_REF_COMBOS[r_idx][0];
            if ref_ != INTRA_FRAME {
                for i in 0..RTC_INTER_MODES as usize {
                    mode_indices[num_modes_count] =
                        MODE_IDX[ref_ as usize][mode_offset(INTER_MODE_LIST[i])?];
                    num_modes_count += 1;
                }
            } else {
                for i in 0..RTC_INTRA_MODES as usize {
                    mode_indices[num_modes_count] =
                        MODE_IDX[ref_ as usize][mode_offset(INTRA_MODE_LIST[i])?];
                    num_modes_count += 1;
                }
            }
        }
    }

    for segment_id in 0..MAX_SEGMENTS {
        let qindex = clamp(
            av1_get_qindex(&cm.seg, segment_id as i32, cm.quant_params.base_qindex)
                + cm.quant_params.y_dc_delta_q,
            0,
            MAXQ,
        );
        let q = compute_rd_thresh_factor(qindex, cm.seq_params.bit_depth);

        for bsize in 0..BLOCK_SIZES_ALL as usize {
            // Threshold here seems unnecessarily harsh but fine given actual
            // range of values used for cpi->sf.thresh_mult[].
            let t = q * RD_THRESH_BLOCK_SIZE_FACTOR[bsize] as i32;
            let thresh_max = i32::MAX / t;

            for i in 0..num_modes_count {
                let mode_index = if use_nonrd_pick_mode {
                    mode_indices[i] as usize
                } else {
                    i
                };
                rd.threshes[segment_id][bsize][mode_index] =
                    if rd.thresh_mult[mode_index] < thresh_max {
                        rd.thresh_mult[mode_index] * t / 4
                    } else {
                        i32::MAX
                    };
            }
        }
    }
    Ok(())
}

/// Translation of `av1_fill_mv_costs()`.
pub(crate) fn av1_fill_mv_costs(
    nmvc: &NmvContext,
    integer_mv: bool,
    usehp: bool,
    mv_costs: Option<&mut MvCosts>,
) {
    // Avoid accessing 'mv_costs' when it is not allocated.
    let Some(mv_costs) = mv_costs else {
        return;
    };

    if integer_mv {
        mv_costs.mv_cost_stack_hp = false;
        av1_build_nmv_cost_table(mv_costs, nmvc, MV_SUBPEL_NONE);
    } else {
        mv_costs.mv_cost_stack_hp = usehp;
        av1_build_nmv_cost_table(mv_costs, nmvc, usehp as i32);
    }
}

/// Translation of `av1_set_rd_speed_thresholds()`.
pub(crate) fn av1_set_rd_speed_thresholds(cpi: &mut Av1Comp) {
    let rd = &mut cpi.rd;

    // Set baseline threshold values.
    rd.thresh_mult = [0; MAX_MODES as usize];

    rd.thresh_mult[THR_NEARESTMV as usize] = 300;
    rd.thresh_mult[THR_NEARESTL2 as usize] = 300;
    rd.thresh_mult[THR_NEARESTL3 as usize] = 300;
    rd.thresh_mult[THR_NEARESTB as usize] = 300;
    rd.thresh_mult[THR_NEARESTA2 as usize] = 300;
    rd.thresh_mult[THR_NEARESTA as usize] = 300;
    rd.thresh_mult[THR_NEARESTG as usize] = 300;

    rd.thresh_mult[THR_NEWMV as usize] = 1000;
    rd.thresh_mult[THR_NEWL2 as usize] = 1000;
    rd.thresh_mult[THR_NEWL3 as usize] = 1000;
    rd.thresh_mult[THR_NEWB as usize] = 1000;
    rd.thresh_mult[THR_NEWA2 as usize] = 1100;
    rd.thresh_mult[THR_NEWA as usize] = 1000;
    rd.thresh_mult[THR_NEWG as usize] = 1000;

    rd.thresh_mult[THR_NEARMV as usize] = 1000;
    rd.thresh_mult[THR_NEARL2 as usize] = 1000;
    rd.thresh_mult[THR_NEARL3 as usize] = 1000;
    rd.thresh_mult[THR_NEARB as usize] = 1000;
    rd.thresh_mult[THR_NEARA2 as usize] = 1000;
    rd.thresh_mult[THR_NEARA as usize] = 1000;
    rd.thresh_mult[THR_NEARG as usize] = 1000;

    rd.thresh_mult[THR_GLOBALMV as usize] = 2200;
    rd.thresh_mult[THR_GLOBALL2 as usize] = 2000;
    rd.thresh_mult[THR_GLOBALL3 as usize] = 2000;
    rd.thresh_mult[THR_GLOBALB as usize] = 2400;
    rd.thresh_mult[THR_GLOBALA2 as usize] = 2000;
    rd.thresh_mult[THR_GLOBALG as usize] = 2000;
    rd.thresh_mult[THR_GLOBALA as usize] = 2400;

    rd.thresh_mult[THR_COMP_NEAREST_NEARESTLA as usize] = 1100;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTL2A as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTL3A as usize] = 800;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTGA as usize] = 900;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTLB as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTL2B as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTL3B as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTGB as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTLA2 as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTL2A2 as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTL3A2 as usize] = 1000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTGA2 as usize] = 1000;

    rd.thresh_mult[THR_COMP_NEAREST_NEARESTLL2 as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTLL3 as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTLG as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEAREST_NEARESTBA as usize] = 2000;

    rd.thresh_mult[THR_COMP_NEAR_NEARLA as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWLA as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTLA as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWLA as usize] = 1530;
    rd.thresh_mult[THR_COMP_NEW_NEARLA as usize] = 1870;
    rd.thresh_mult[THR_COMP_NEW_NEWLA as usize] = 2400;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALLA as usize] = 2750;

    rd.thresh_mult[THR_COMP_NEAR_NEARL2A as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWL2A as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTL2A as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWL2A as usize] = 1870;
    rd.thresh_mult[THR_COMP_NEW_NEARL2A as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWL2A as usize] = 1800;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALL2A as usize] = 2500;

    rd.thresh_mult[THR_COMP_NEAR_NEARL3A as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWL3A as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTL3A as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWL3A as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEARL3A as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWL3A as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALL3A as usize] = 3000;

    rd.thresh_mult[THR_COMP_NEAR_NEARGA as usize] = 1320;
    rd.thresh_mult[THR_COMP_NEAREST_NEWGA as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTGA as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWGA as usize] = 2040;
    rd.thresh_mult[THR_COMP_NEW_NEARGA as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWGA as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALGA as usize] = 2250;

    rd.thresh_mult[THR_COMP_NEAR_NEARLB as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWLB as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTLB as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWLB as usize] = 1360;
    rd.thresh_mult[THR_COMP_NEW_NEARLB as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWLB as usize] = 2400;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALLB as usize] = 2250;

    rd.thresh_mult[THR_COMP_NEAR_NEARL2B as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWL2B as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTL2B as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWL2B as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEARL2B as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWL2B as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALL2B as usize] = 2500;

    rd.thresh_mult[THR_COMP_NEAR_NEARL3B as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWL3B as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTL3B as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWL3B as usize] = 1870;
    rd.thresh_mult[THR_COMP_NEW_NEARL3B as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWL3B as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALL3B as usize] = 2500;

    rd.thresh_mult[THR_COMP_NEAR_NEARGB as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWGB as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTGB as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWGB as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEARGB as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWGB as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALGB as usize] = 2500;

    rd.thresh_mult[THR_COMP_NEAR_NEARLA2 as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWLA2 as usize] = 1800;
    rd.thresh_mult[THR_COMP_NEW_NEARESTLA2 as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWLA2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEARLA2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWLA2 as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALLA2 as usize] = 2500;

    rd.thresh_mult[THR_COMP_NEAR_NEARL2A2 as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWL2A2 as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTL2A2 as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWL2A2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEARL2A2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWL2A2 as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALL2A2 as usize] = 2500;

    rd.thresh_mult[THR_COMP_NEAR_NEARL3A2 as usize] = 1440;
    rd.thresh_mult[THR_COMP_NEAREST_NEWL3A2 as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTL3A2 as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWL3A2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEARL3A2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWL3A2 as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALL3A2 as usize] = 2500;

    rd.thresh_mult[THR_COMP_NEAR_NEARGA2 as usize] = 1200;
    rd.thresh_mult[THR_COMP_NEAREST_NEWGA2 as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEW_NEARESTGA2 as usize] = 1500;
    rd.thresh_mult[THR_COMP_NEAR_NEWGA2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEARGA2 as usize] = 1700;
    rd.thresh_mult[THR_COMP_NEW_NEWGA2 as usize] = 2000;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALGA2 as usize] = 2750;

    rd.thresh_mult[THR_COMP_NEAR_NEARLL2 as usize] = 1600;
    rd.thresh_mult[THR_COMP_NEAREST_NEWLL2 as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEW_NEARESTLL2 as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEAR_NEWLL2 as usize] = 2640;
    rd.thresh_mult[THR_COMP_NEW_NEARLL2 as usize] = 2200;
    rd.thresh_mult[THR_COMP_NEW_NEWLL2 as usize] = 2400;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALLL2 as usize] = 3200;

    rd.thresh_mult[THR_COMP_NEAR_NEARLL3 as usize] = 1600;
    rd.thresh_mult[THR_COMP_NEAREST_NEWLL3 as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEW_NEARESTLL3 as usize] = 1800;
    rd.thresh_mult[THR_COMP_NEAR_NEWLL3 as usize] = 2200;
    rd.thresh_mult[THR_COMP_NEW_NEARLL3 as usize] = 2200;
    rd.thresh_mult[THR_COMP_NEW_NEWLL3 as usize] = 2400;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALLL3 as usize] = 3200;

    rd.thresh_mult[THR_COMP_NEAR_NEARLG as usize] = 1760;
    rd.thresh_mult[THR_COMP_NEAREST_NEWLG as usize] = 2400;
    rd.thresh_mult[THR_COMP_NEW_NEARESTLG as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEAR_NEWLG as usize] = 1760;
    rd.thresh_mult[THR_COMP_NEW_NEARLG as usize] = 2640;
    rd.thresh_mult[THR_COMP_NEW_NEWLG as usize] = 2400;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALLG as usize] = 3200;

    rd.thresh_mult[THR_COMP_NEAR_NEARBA as usize] = 1600;
    rd.thresh_mult[THR_COMP_NEAREST_NEWBA as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEW_NEARESTBA as usize] = 2000;
    rd.thresh_mult[THR_COMP_NEAR_NEWBA as usize] = 2200;
    rd.thresh_mult[THR_COMP_NEW_NEARBA as usize] = 1980;
    rd.thresh_mult[THR_COMP_NEW_NEWBA as usize] = 2640;
    rd.thresh_mult[THR_COMP_GLOBAL_GLOBALBA as usize] = 3200;

    rd.thresh_mult[THR_DC as usize] = 1000;
    rd.thresh_mult[THR_PAETH as usize] = 1000;
    rd.thresh_mult[THR_SMOOTH as usize] = 2200;
    rd.thresh_mult[THR_SMOOTH_V as usize] = 2000;
    rd.thresh_mult[THR_SMOOTH_H as usize] = 2000;
    rd.thresh_mult[THR_H_PRED as usize] = 2000;
    rd.thresh_mult[THR_V_PRED as usize] = 1800;
    rd.thresh_mult[THR_D135_PRED as usize] = 2500;
    rd.thresh_mult[THR_D203_PRED as usize] = 2000;
    rd.thresh_mult[THR_D157_PRED as usize] = 2500;
    rd.thresh_mult[THR_D67_PRED as usize] = 2000;
    rd.thresh_mult[THR_D113_PRED as usize] = 2500;
    rd.thresh_mult[THR_D45_PRED as usize] = 2500;
}

// Checks if entropy costs should be initialized/updated at frame level or not.
#[inline]
fn is_frame_level_cost_upd_freq_set(
    cm: &Av1Common,
    cost_upd_level: i32,
    use_nonrd_pick_mode: bool,
    frames_since_key: i32,
) -> bool {
    let fill_costs = frame_is_intra_only(cm)
        || if use_nonrd_pick_mode {
            frames_since_key < 2
        } else {
            (cm.current_frame.frame_number & 0x07) == 1
        };
    (!use_nonrd_pick_mode && cost_upd_level != INTERNAL_COST_UPD_OFF)
        || cost_upd_level == INTERNAL_COST_UPD_TILE
        || fill_costs
}

// Populates speed features based on codec control settings (of type
// COST_UPDATE_TYPE) and expected speed feature settings (of type
// INTERNAL_COST_UPDATE_TYPE) by considering the least frequent cost update.
// The populated/updated speed features are used for cost updates in the
// encoder.
// WARNING: Population of unified cost update frequency needs to be taken care
// accordingly, in case of any modifications/additions to the enum
// COST_UPDATE_TYPE/INTERNAL_COST_UPDATE_TYPE.
#[inline]
fn populate_unified_cost_update_freq(
    cost_upd_freq: &super::encoder::CostUpdateFreq,
    sf: &mut SpeedFeatures,
) {
    let inter_sf = &mut sf.inter_sf;
    // Mapping of entropy cost update frequency from the encoder's codec control
    // settings of type COST_UPDATE_TYPE to speed features of type
    // INTERNAL_COST_UPDATE_TYPE.
    static MAP_COST_UPD_TO_INTERNAL_COST_UPD: [i32; 4] = [
        INTERNAL_COST_UPD_SB,
        INTERNAL_COST_UPD_SBROW,
        INTERNAL_COST_UPD_TILE,
        INTERNAL_COST_UPD_OFF,
    ];

    inter_sf.mv_cost_upd_level = inter_sf
        .mv_cost_upd_level
        .min(MAP_COST_UPD_TO_INTERNAL_COST_UPD[cost_upd_freq.mv as usize]);
    inter_sf.coeff_cost_upd_level = inter_sf
        .coeff_cost_upd_level
        .min(MAP_COST_UPD_TO_INTERNAL_COST_UPD[cost_upd_freq.coeff as usize]);
    inter_sf.mode_cost_upd_level = inter_sf
        .mode_cost_upd_level
        .min(MAP_COST_UPD_TO_INTERNAL_COST_UPD[cost_upd_freq.mode as usize]);
    sf.intra_sf.dv_cost_upd_level = sf
        .intra_sf
        .dv_cost_upd_level
        .min(MAP_COST_UPD_TO_INTERNAL_COST_UPD[cost_upd_freq.dv as usize]);
}

// Decide whether we want to update the mode entropy cost for the current frame.
// The logit is currently inherited from selective_disable_cdf_rtc.
#[inline]
fn should_force_mode_cost_update(cpi: &Av1Comp) -> bool {
    let rt_sf = &cpi.sf.rt_sf;
    if !rt_sf.frame_level_mode_cost_update {
        return false;
    }

    if cpi.oxcf.algo_cfg.cdf_update_mode == 2 {
        return cpi.frames_since_last_update == 1;
    } else if cpi.oxcf.algo_cfg.cdf_update_mode == 1 {
        if cpi.svc.number_spatial_layers == 1 && cpi.svc.number_temporal_layers == 1 {
            let cm = &cpi.common;
            let rc = &cpi.rc;

            return frame_is_intra_only(cm)
                || super::ratectrl::is_frame_resize_pending(cpi)
                || rc.high_source_sad != 0
                || rc.frames_since_key < 10
                || cpi.cyclic_refresh.counter_encode_maxq_scene_change < 10
                || cm.current_frame.frame_number % 8 == 0;
        } else if cpi.svc.number_temporal_layers > 1 {
            return cpi.svc.temporal_layer_id != cpi.svc.number_temporal_layers - 1;
        }
    }

    false
}

/// Translation of `av1_initialize_rd_consts()`: `td` is `&cpi->td`, moved
/// out of `cpi` by the caller.
pub(crate) fn av1_initialize_rd_consts(cpi: &mut Av1Comp, td: &mut ThreadData) -> Result<(), ()> {
    let use_nonrd_pick_mode = cpi.sf.rt_sf.use_nonrd_pick_mode != 0;
    let frames_since_key = cpi.rc.frames_since_key;

    let gf_group = &cpi.ppi.gf_group;
    let boost_index = 15.min(cpi.ppi.p_rc.gfu_boost / 100);
    let layer_depth = (gf_group.layer_depth[cpi.gf_frame_index as usize] as i32).min(6);
    let frame_type = cpi.common.current_frame.frame_type;

    let qindex_rdmult = cpi.common.quant_params.base_qindex + cpi.common.quant_params.y_dc_delta_q;
    cpi.rd.rdmult = av1_compute_rd_mult(
        qindex_rdmult,
        cpi.common.seq_params.bit_depth,
        cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize] as i32,
        layer_depth,
        boost_index,
        frame_type,
        cpi.oxcf.q_cfg.use_fixed_qp_offsets,
        is_stat_consumption_stage(cpi),
    );

    let x = &mut td.mb;
    av1_set_error_per_bit(&mut x.errorperbit, cpi.rd.rdmult);

    set_block_thresholds(&cpi.common, &mut cpi.rd, use_nonrd_pick_mode)?;

    populate_unified_cost_update_freq(&cpi.oxcf.cost_upd_freq, &mut cpi.sf);
    let inter_sf = &cpi.sf.inter_sf;
    let cm = &cpi.common;
    // Frame level mv cost update
    if is_frame_level_cost_upd_freq_set(
        cm,
        inter_sf.mv_cost_upd_level,
        use_nonrd_pick_mode,
        frames_since_key,
    ) {
        av1_fill_mv_costs(
            &cm.fc.nmvc,
            cm.features.cur_frame_force_integer_mv,
            cm.features.allow_high_precision_mv,
            x.mv_costs.as_deref_mut(),
        );
    }

    // Frame level coefficient cost update
    if is_frame_level_cost_upd_freq_set(
        cm,
        inter_sf.coeff_cost_upd_level,
        use_nonrd_pick_mode,
        frames_since_key,
    ) {
        av1_fill_coeff_costs(&mut x.coeff_costs, &cm.fc, av1_num_planes(cm));
    }

    // Frame level mode cost update
    if should_force_mode_cost_update(cpi)
        || is_frame_level_cost_upd_freq_set(
            cm,
            inter_sf.mode_cost_upd_level,
            use_nonrd_pick_mode,
            frames_since_key,
        )
    {
        av1_fill_mode_rates(cm, &mut x.mode_costs, &cm.fc);
    }

    // Frame level dv cost update
    // (av1_need_dv_costs(): intra block copy is only allowed with screen
    // content tools on intra frames, which libavif never enables)
    debug_assert!(!cm.features.allow_intrabc);
    Ok(())
}

/// Translation of `av1_get_entropy_contexts()`: copies the plane's above
/// and left contexts of the block.
pub(crate) fn av1_get_entropy_contexts(
    plane_bsize: BlockSize,
    xd: &crate::aom::av1::common::blockd::MacroBlockD,
    plane: usize,
    t_above: &mut [EntropyContext; MAX_MIB_SIZE as usize],
    t_left: &mut [EntropyContext; MAX_MIB_SIZE as usize],
) {
    debug_assert!(plane_bsize < BLOCK_SIZES_ALL);
    let num_4x4_w = MI_SIZE_WIDE[plane_bsize as usize] as usize;
    let num_4x4_h = MI_SIZE_HIGH[plane_bsize as usize] as usize;
    let pd = &xd.plane[plane];
    let above = &xd.above_entropy(plane)[pd.above_entropy_context..];
    let left = &xd.left_entropy_context[plane][pd.left_entropy_context..];

    t_above[..num_4x4_w].copy_from_slice(&above[..num_4x4_w]);
    t_left[..num_4x4_h].copy_from_slice(&left[..num_4x4_h]);
}

fn model_rd_norm(xsq_q10: i32, r_q10: &mut i32, d_q10: &mut i32) {
    // NOTE: The tables below must be of the same size.

    // The functions described below are sampled at the four most significant
    // bits of x^2 + 8 / 256.

    // Normalized rate:
    // This table models the rate for a Laplacian source with given variance
    // when quantized with a uniform quantizer with given stepsize. The
    // closed form expression is:
    // Rn(x) = H(sqrt(r)) + sqrt(r)*[1 + H(r)/(1 - r)],
    // where r = exp(-sqrt(2) * x) and x = qpstep / sqrt(variance),
    // and H(x) is the binary entropy function.
    const RATE_TAB_Q10: [i32; 104] = [
        65536, 6086, 5574, 5275, 5063, 4899, 4764, 4651, 4553, 4389, 4255, 4142, 4044, 3958, 3881,
        3811, 3748, 3635, 3538, 3453, 3376, 3307, 3244, 3186, 3133, 3037, 2952, 2877, 2809, 2747,
        2690, 2638, 2589, 2501, 2423, 2353, 2290, 2232, 2179, 2130, 2084, 2001, 1928, 1862, 1802,
        1748, 1698, 1651, 1608, 1530, 1460, 1398, 1342, 1290, 1243, 1199, 1159, 1086, 1021, 963,
        911, 864, 821, 781, 745, 680, 623, 574, 530, 490, 455, 424, 395, 345, 304, 269, 239, 213,
        190, 171, 154, 126, 104, 87, 73, 61, 52, 44, 38, 28, 21, 16, 12, 10, 8, 6, 5, 3, 2, 1, 1,
        1, 0, 0,
    ];

    // Normalized distortion:
    // This table models the normalized distortion for a Laplacian source
    // with given variance when quantized with a uniform quantizer
    // with given stepsize. The closed form expression is:
    // Dn(x) = 1 - 1/sqrt(2) * x / sinh(x/sqrt(2))
    // where x = qpstep / sqrt(variance).
    // Note the actual distortion is Dn * variance.
    const DIST_TAB_Q10: [i32; 104] = [
        0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 5, 5, 6, 7, 7, 8, 9, 11, 12, 13, 15, 16, 17, 18, 21, 24,
        26, 29, 31, 34, 36, 39, 44, 49, 54, 59, 64, 69, 73, 78, 88, 97, 106, 115, 124, 133, 142,
        151, 167, 184, 200, 215, 231, 245, 260, 274, 301, 327, 351, 375, 397, 418, 439, 458, 495,
        528, 559, 587, 613, 637, 659, 680, 717, 749, 777, 801, 823, 842, 859, 874, 899, 919, 936,
        949, 960, 969, 977, 983, 994, 1001, 1006, 1010, 1013, 1015, 1017, 1018, 1020, 1022, 1022,
        1023, 1023, 1023, 1024,
    ];
    const XSQ_IQ_Q10: [i32; 104] = [
        0, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 72, 80, 88, 96, 112, 128, 144, 160, 176,
        192, 208, 224, 256, 288, 320, 352, 384, 416, 448, 480, 544, 608, 672, 736, 800, 864, 928,
        992, 1120, 1248, 1376, 1504, 1632, 1760, 1888, 2016, 2272, 2528, 2784, 3040, 3296, 3552,
        3808, 4064, 4576, 5088, 5600, 6112, 6624, 7136, 7648, 8160, 9184, 10208, 11232, 12256,
        13280, 14304, 15328, 16352, 18400, 20448, 22496, 24544, 26592, 28640, 30688, 32736, 36832,
        40928, 45024, 49120, 53216, 57312, 61408, 65504, 73696, 81888, 90080, 98272, 106464,
        114656, 122848, 131040, 147424, 163808, 180192, 196576, 212960, 229344, 245728,
    ];
    let tmp = (xsq_q10 >> 2) + 8;
    let k = get_msb(tmp as u32) - 3;
    let xq = ((k << 3) + ((tmp >> k) & 0x7)) as usize;
    let one_q10 = 1 << 10;
    let a_q10 = ((xsq_q10 - XSQ_IQ_Q10[xq]) << 10) >> (2 + k);
    let b_q10 = one_q10 - a_q10;
    *r_q10 = (RATE_TAB_Q10[xq] * b_q10 + RATE_TAB_Q10[xq + 1] * a_q10) >> 10;
    *d_q10 = (DIST_TAB_Q10[xq] * b_q10 + DIST_TAB_Q10[xq + 1] * a_q10) >> 10;
}

pub(crate) fn av1_model_rd_from_var_lapndz(
    var: i64,
    n_log2: u32,
    qstep: u32,
    rate: &mut i32,
    dist: &mut i64,
) {
    // This function models the rate and distortion for a Laplacian
    // source with given variance when quantized with a uniform quantizer
    // with given stepsize. The closed form expressions are in:
    // Hang and Chen, "Source Model for transform video coder and its
    // application - Part I: Fundamental Theory", IEEE Trans. Circ.
    // Sys. for Video Tech., April 1997.
    if var == 0 {
        *rate = 0;
        *dist = 0;
    } else {
        let mut d_q10 = 0;
        let mut r_q10 = 0;
        const MAX_XSQ_Q10: u32 = 245727;
        let xsq_q10_64: u64 = (((qstep as u64 * qstep as u64) << (n_log2 + 10))
            .wrapping_add((var >> 1) as u64))
            / var as u64;
        let xsq_q10 = xsq_q10_64.min(MAX_XSQ_Q10 as u64) as i32;
        model_rd_norm(xsq_q10, &mut r_q10, &mut d_q10);
        let n = 10 - AV1_PROB_COST_SHIFT;
        *rate = ((r_q10 << n_log2) + ((1 << n) >> 1)) >> n;
        *dist = (var * d_q10 as i64 + 512) >> 10;
    }
}

#[inline]
fn interp_cubic(p: &[f64], x: f64) -> f64 {
    p[1] + 0.5
        * x
        * (p[2] - p[0]
            + x * (2.0 * p[0] - 5.0 * p[1] + 4.0 * p[2] - p[3]
                + x * (3.0 * (p[1] - p[2]) + p[3] - p[0])))
}

const BSIZE_CURVFIT_MODEL_CAT_LOOKUP: [u8; BLOCK_SIZES_ALL as usize] = [
    0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 1, 1, 2, 2, 3, 3,
];

#[inline]
fn sse_norm_curvfit_model_cat_lookup(sse_norm: f64) -> usize {
    (sse_norm > 16.0) as usize
}

pub(crate) fn av1_model_rd_curvfit(
    bsize: BlockSize,
    sse_norm: f64,
    mut xqr: f64,
    rate_f: &mut f64,
    distbysse_f: &mut f64,
) {
    let x_start = -15.5;
    let x_end = 16.5;
    let x_step = 0.5;
    let epsilon = 1e-6;
    let rcat = BSIZE_CURVFIT_MODEL_CAT_LOOKUP[bsize as usize] as usize;
    let dcat = sse_norm_curvfit_model_cat_lookup(sse_norm);

    xqr = xqr.max(x_start + x_step + epsilon);
    xqr = xqr.min(x_end - x_step - epsilon);
    let x = (xqr - x_start) / x_step;
    let xi = x.floor() as i32;
    let xo = x - xi as f64;

    debug_assert!(xi > 0);

    let prate = &INTERP_RGRID_CURV[rcat][(xi - 1) as usize..];
    *rate_f = interp_cubic(prate, xo);
    let pdist = &INTERP_DGRID_CURV[dcat][(xi - 1) as usize..];
    *distbysse_f = interp_cubic(pdist, xo);
}

// Special clamping used in the encoder when calculating a prediction
//
// Logically, all pixel fetches used for prediction are clamped against the
// edges of the frame. But doing this directly is slow, so instead we allocate
// a finite border around the frame and fill it with copies of the outermost
// pixels.
//
// Since this border is finite, we need to clamp the motion vector before
// prediction in order to avoid out-of-bounds reads. At the same time, this
// clamp must not change the prediction result.
//
// We can balance both of these concerns by calculating how far we would have
// to go in each direction before the extended prediction region (the current
// block + AOM_INTERP_EXTEND many pixels around the block) would be mapped
// so that it touches the frame only at one row or column. This is a special
// point because any more extreme MV will always lead to the same prediction.
// So it is safe to clamp at that point.
//
// In the worst case, this requires a border of
//   max_block_width + 2*AOM_INTERP_EXTEND = 128 + 2*4 = 136 pixels
// around the frame edges.
#[inline]
fn enc_clamp_mv(cm: &Av1Common, xd: &MacroBlockD, mv: &mut Mv) {
    let bw = (xd.width as i32) << MI_SIZE_LOG2;
    let bh = (xd.height as i32) << MI_SIZE_LOG2;

    let px_to_left_edge = xd.mi_col << MI_SIZE_LOG2;
    let px_to_right_edge = (cm.mi_params.mi_cols - xd.mi_col) << MI_SIZE_LOG2;
    let px_to_top_edge = xd.mi_row << MI_SIZE_LOG2;
    let px_to_bottom_edge = (cm.mi_params.mi_rows - xd.mi_row) << MI_SIZE_LOG2;

    let mv_limits = SubpelMvLimits {
        col_min: -get_mv_subpel(px_to_left_edge + bw + AOM_INTERP_EXTEND),
        col_max: get_mv_subpel(px_to_right_edge + AOM_INTERP_EXTEND),
        row_min: -get_mv_subpel(px_to_top_edge + bh + AOM_INTERP_EXTEND),
        row_max: get_mv_subpel(px_to_bottom_edge + AOM_INTERP_EXTEND),
    };
    clamp_mv(mv, &mv_limits);
}

/// Translation of `av1_mv_pred()`: `ref_y_buffer` is the reference's luma
/// plane (the frame buffer and the offset of the block).
pub(crate) fn av1_mv_pred(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    ref_y_buffer: (&[u16], usize),
    ref_y_stride: i32,
    ref_frame: MvReferenceFrame,
    block_size: BlockSize,
) {
    with_source(x, |x, src| {
        mv_pred(
            cpi,
            x,
            src,
            ref_y_buffer,
            ref_y_stride,
            ref_frame,
            block_size,
        )
    })
}

/// `av1_mv_pred()` on the source samples `src`.
fn mv_pred(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    src: &[u16],
    ref_y_buffer: (&[u16], usize),
    ref_y_stride: i32,
    ref_frame: MvReferenceFrame,
    block_size: BlockSize,
) {
    let ref_frames = [ref_frame, NONE_FRAME];
    let ref_mv = av1_get_ref_mv_from_stack(0, &ref_frames, 0, &x.mbmi_ext);
    let ref_mv1 = av1_get_ref_mv_from_stack(0, &ref_frames, 1, &x.mbmi_ext);
    let mut pred_mv = [Mv::default(); MAX_MV_REF_CANDIDATES as usize + 1];
    let mut num_mv_refs = 0;
    pred_mv[num_mv_refs] = ref_mv.as_mv();
    num_mv_refs += 1;
    if ref_mv.as_int != ref_mv1.as_int {
        pred_mv[num_mv_refs] = ref_mv1.as_mv();
        num_mv_refs += 1;
    }

    debug_assert!(num_mv_refs <= pred_mv.len());

    let src_y_ptr = x.plane[0].src.buf;
    let mut zero_seen = false;
    let mut best_sad = i32::MAX;
    let mut max_mv = 0;
    let rf = ref_frame as usize;
    // Get the sad for each candidate reference mv.
    for (i, this_mv) in pred_mv.iter_mut().enumerate().take(num_mv_refs) {
        enc_clamp_mv(&cpi.common, &x.e_mbd, this_mv);

        let (r, c) = (this_mv.row as i32, this_mv.col as i32);
        let fp_row = (r + 3 + (r >= 0) as i32) >> 3;
        let fp_col = (c + 3 + (c >= 0) as i32) >> 3;
        max_mv = max_mv.max(r.abs().max(c.abs()) >> 3);

        if fp_row == 0 && fp_col == 0 && zero_seen {
            continue;
        }
        zero_seen |= fp_row == 0 && fp_col == 0;

        let ref_y_ptr = (ref_y_buffer.1 as i64 + (ref_y_stride * fp_row + fp_col) as i64) as usize;
        // Find sad for current vector.
        let this_sad = cpi.ppi.fn_ptr.sdf(
            block_size,
            src,
            src_y_ptr,
            x.plane[0].src.stride as usize,
            ref_y_buffer.0,
            ref_y_ptr,
            ref_y_stride as usize,
        ) as i32;
        // Note if it is the best so far.
        if this_sad < best_sad {
            best_sad = this_sad;
        }
        if i == 0 {
            x.pred_mv0_sad[rf] = this_sad;
        } else if i == 1 {
            x.pred_mv1_sad[rf] = this_sad;
        }
    }

    // Note the index of the mv that worked best in the reference list.
    x.max_mv_context[rf] = max_mv;
    x.pred_mv_sad[rf] = best_sad;
}

/// Translation of `av1_setup_pred_block()`: `src` is the frame at buffer
/// pool index `frame_buf`.
pub(crate) fn av1_setup_pred_block(
    xd: &MacroBlockD,
    dst: &mut [Buf2d; MAX_MB_PLANE],
    src: &Yv12BufferConfig,
    frame_buf: usize,
    scale: Option<&ScaleFactors>,
    scale_uv: Option<&ScaleFactors>,
    num_planes: usize,
) {
    dst[0].buf = src.buffers[0];
    dst[0].stride = src.strides[0];
    dst[1].buf = src.buffers[1];
    dst[2].buf = src.buffers[2];
    dst[1].stride = src.strides[1];
    dst[2].stride = src.strides[1];

    let mi_row = xd.mi_row;
    let mi_col = xd.mi_col;
    for (i, d) in dst.iter_mut().enumerate().take(num_planes) {
        let is_uv = (i > 0) as usize;
        let (buf, stride) = (d.buf, d.stride);
        setup_pred_plane(
            d,
            xd.mbmi().bsize,
            buf,
            src.crop_widths[is_uv],
            src.crop_heights[is_uv],
            stride,
            mi_row,
            mi_col,
            if i != 0 { scale_uv } else { scale },
            xd.plane[i].subsampling_x,
            xd.plane[i].subsampling_y,
        );
        d.frame_buf = frame_buf;
    }
}

/// Translation of `av1_get_scaled_ref_frame()`: the buffer pool index of
/// the scaled reference, if it is not the reference itself.
pub(crate) fn av1_get_scaled_ref_frame(
    cpi: &Av1Comp,
    ref_frame: MvReferenceFrame,
) -> Option<usize> {
    debug_assert!((LAST_FRAME..=ALTREF_FRAME).contains(&ref_frame));
    let scaled_buf = cpi.scaled_ref_buf[(ref_frame - 1) as usize];
    let ref_buf = get_ref_frame_buf(&cpi.common, ref_frame);
    if scaled_buf != ref_buf && scaled_buf.is_some() {
        scaled_buf
    } else {
        None
    }
}

/// `SWITCHABLE_INTERP_RATE_FACTOR` (rd.h)
pub(crate) const SWITCHABLE_INTERP_RATE_FACTOR: i32 = 1;

pub(crate) fn av1_get_switchable_rate(
    x: &Macroblock,
    xd: &MacroBlockD,
    interp_filter: InterpFilter,
    dual_filter: bool,
) -> i32 {
    if interp_filter == SWITCHABLE {
        let mbmi = xd.mbmi();
        let mut inter_filter_cost = 0;
        for dir in 0..2 {
            if dir != 0 && !dual_filter {
                break;
            }
            let ctx = av1_get_pred_context_switchable_interp(xd, dir) as usize;
            let filter = av1_extract_interp_filter(mbmi.interp_filters, dir) as usize;
            inter_filter_cost += x.mode_costs.switchable_interp_costs[ctx][filter];
        }
        SWITCHABLE_INTERP_RATE_FACTOR * inter_filter_cost
    } else {
        0
    }
}

pub(crate) fn av1_get_intra_cost_penalty(qindex: i32, qdelta: i32, bit_depth: i32) -> i32 {
    let q = av1_dc_quant_qtx(qindex, qdelta, bit_depth) as i32;
    match bit_depth {
        8 => 20 * q,
        10 => 5 * q,
        12 => (5 * q + 2) >> 2,
        _ => {
            debug_assert!(
                false,
                "bit_depth should be AOM_BITS_8, AOM_BITS_10 or AOM_BITS_12"
            );
            -1
        }
    }
}

static INTERP_RGRID_CURV: [[f64; 65]; 4] = [
    [
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        118.257702,
        120.210658,
        121.434853,
        122.100487,
        122.377758,
        122.436865,
        72.290102,
        96.974289,
        101.652727,
        126.830141,
        140.417377,
        157.644879,
        184.315291,
        215.823873,
        262.300169,
        335.919859,
        420.624173,
        519.185032,
        619.854243,
        726.053595,
        827.663369,
        933.127475,
        1037.988755,
        1138.839609,
        1233.342933,
        1333.508064,
        1428.760126,
        1533.396364,
        1616.952052,
        1744.539319,
        1803.413586,
        1951.466618,
        1994.227838,
        2086.031680,
        2148.635443,
        2239.068450,
        2222.590637,
        2338.859809,
        2402.929011,
        2418.727875,
        2435.342670,
        2471.159469,
        2523.187446,
        2591.183827,
        2674.905840,
        2774.110714,
        2888.555675,
        3017.997952,
        3162.194773,
        3320.903365,
        3493.880956,
        3680.884773,
        3881.672045,
        4096.000000,
    ],
    [
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        13.087244,
        15.919735,
        25.930313,
        24.412411,
        28.567417,
        29.924194,
        30.857010,
        32.742979,
        36.382570,
        39.210386,
        42.265690,
        47.378572,
        57.014850,
        82.740067,
        137.346562,
        219.968084,
        316.781856,
        415.643773,
        516.706538,
        614.914364,
        714.303763,
        815.512135,
        911.210485,
        1008.501528,
        1109.787854,
        1213.772279,
        1322.922561,
        1414.752579,
        1510.505641,
        1615.741888,
        1697.989032,
        1780.123933,
        1847.453790,
        1913.742309,
        1960.828122,
        2047.500168,
        2085.454095,
        2129.230668,
        2158.171824,
        2182.231724,
        2217.684864,
        2269.589211,
        2337.264824,
        2420.618694,
        2519.557814,
        2633.989178,
        2763.819779,
        2908.956609,
        3069.306660,
        3244.776927,
        3435.274401,
        3640.706076,
        3860.978945,
        4096.000000,
    ],
    [
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        4.656893,
        5.123633,
        5.594132,
        6.162376,
        6.918433,
        7.768444,
        8.739415,
        10.105862,
        11.477328,
        13.236604,
        15.421030,
        19.093623,
        25.801871,
        46.724612,
        98.841054,
        181.113466,
        272.586364,
        359.499769,
        445.546343,
        525.944439,
        605.188743,
        681.793483,
        756.668359,
        838.486885,
        926.950356,
        1015.482542,
        1113.353926,
        1204.897193,
        1288.871992,
        1373.464145,
        1455.746628,
        1527.796460,
        1588.475066,
        1658.144771,
        1710.302500,
        1807.563351,
        1863.197608,
        1927.281616,
        1964.450872,
        2022.719898,
        2100.041145,
        2185.205712,
        2280.993936,
        2387.616216,
        2505.282950,
        2634.204540,
        2774.591385,
        2926.653884,
        3090.602436,
        3266.647443,
        3454.999303,
        3655.868416,
        3869.465182,
        4096.000000,
    ],
    [
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.000000,
        0.337370,
        0.391916,
        0.468839,
        0.566334,
        0.762564,
        1.069225,
        1.384361,
        1.787581,
        2.293948,
        3.251909,
        4.412991,
        8.050068,
        11.606073,
        27.668092,
        65.227758,
        128.463938,
        202.097653,
        262.715851,
        312.464873,
        355.601398,
        400.609054,
        447.201352,
        495.761568,
        552.871938,
        619.067625,
        691.984883,
        773.753288,
        860.628503,
        946.262808,
        1019.805896,
        1106.061360,
        1178.422145,
        1244.852258,
        1302.173987,
        1399.650266,
        1548.092912,
        1545.928652,
        1670.817500,
        1694.523823,
        1779.195362,
        1882.155494,
        1990.662097,
        2108.325181,
        2235.456119,
        2372.366287,
        2519.367059,
        2676.769812,
        2844.885918,
        3024.026754,
        3214.503695,
        3416.628115,
        3630.711389,
        3857.064892,
        4096.000000,
    ],
];

static INTERP_DGRID_CURV: [[f64; 65]; 3] = [
    [
        16.000000, 15.962891, 15.925174, 15.886888, 15.848074, 15.808770, 15.769015, 15.728850,
        15.688313, 15.647445, 15.606284, 15.564870, 15.525918, 15.483820, 15.373330, 15.126844,
        14.637442, 14.184387, 13.560070, 12.880717, 12.165995, 11.378144, 10.438769, 9.130790,
        7.487633, 5.688649, 4.267515, 3.196300, 2.434201, 1.834064, 1.369920, 1.035921, 0.775279,
        0.574895, 0.427232, 0.314123, 0.233236, 0.171440, 0.128188, 0.092762, 0.067569, 0.049324,
        0.036330, 0.027008, 0.019853, 0.015539, 0.011093, 0.008733, 0.007624, 0.008105, 0.005427,
        0.004065, 0.003427, 0.002848, 0.002328, 0.001865, 0.001457, 0.001103, 0.000801, 0.000550,
        0.000348, 0.000193, 0.000085, 0.000021, 0.000000,
    ],
    [
        16.000000, 15.996116, 15.984769, 15.966413, 15.941505, 15.910501, 15.873856, 15.832026,
        15.785466, 15.734633, 15.679981, 15.621967, 15.560961, 15.460157, 15.288367, 15.052462,
        14.466922, 13.921212, 13.073692, 12.222005, 11.237799, 9.985848, 8.898823, 7.423519,
        5.995325, 4.773152, 3.744032, 2.938217, 2.294526, 1.762412, 1.327145, 1.020728, 0.765535,
        0.570548, 0.425833, 0.313825, 0.232959, 0.171324, 0.128174, 0.092750, 0.067558, 0.049319,
        0.036330, 0.027008, 0.019853, 0.015539, 0.011093, 0.008733, 0.007624, 0.008105, 0.005427,
        0.004065, 0.003427, 0.002848, 0.002328, 0.001865, 0.001457, 0.001103, 0.000801, 0.000550,
        0.000348, 0.000193, 0.000085, 0.000021, -0.000000,
    ],
    [0.0; 65],
];
