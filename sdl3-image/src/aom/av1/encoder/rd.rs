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
use super::encoder::RdOpt;
use super::encoder::{is_stat_consumption_stage, Av1Comp, ThreadData};
use super::nonrd_pickmode::{
    INTER_MODE_LIST, INTRA_MODE_LIST, MODE_IDX, RTC_INTER_MODES, RTC_INTRA_MODES, RTC_MODES,
};
use super::ratectrl::av1_convert_qindex_to_q;
use super::speed_features::*;
use crate::aom::aom_dsp::aom_dsp_common::clamp;
use crate::aom::aom_dsp::aom_dsp_common::round_power_of_two_i64;
use crate::aom::av1::common::av1_common_int::{av1_num_planes, frame_is_intra_only, Av1Common};
use crate::aom::av1::common::blockd::KEY_FRAME;
use crate::aom::av1::common::entropy::*;
use crate::aom::av1::common::entropymode::*;
use crate::aom::av1::common::entropymv::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::SWITCHABLE_FILTER_CONTEXTS;
use crate::aom::av1::common::quant_common::{av1_dc_quant_qtx, av1_get_qindex, MAXQ};
use crate::aom::av1::common::reconintra::av1_filter_intra_allowed_bsize;
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
    // (frame-level mode cost updates are a real-time speed feature)
    debug_assert!(!rt_sf.frame_level_mode_cost_update);
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
