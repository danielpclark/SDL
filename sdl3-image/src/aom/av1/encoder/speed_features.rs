// Rust translation of the parts of av1/encoder/speed_features.c and
// av1/encoder/speed_features.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the all intra speed features.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Speed features. The structures keep only the features the saver's code
//! paths read (most features steer the RD searches, the motion searches
//! and the loop filter searches, which the saver's speed never runs); the
//! functions set those features exactly as upstream's do.

use super::block::*;
use super::encoder::*;
use crate::aom::av1::common::av1_common_int::frame_is_intra_only;
use crate::aom::av1::common::enums::*;

// (DIST_WTD_COMP_FLAG)
pub(crate) const DIST_WTD_COMP_ENABLED: i32 = 0;
pub(crate) const DIST_WTD_COMP_SKIP_MV_SEARCH: i32 = 1;
pub(crate) const DIST_WTD_COMP_DISABLED: i32 = 2;

// (INTRA_MODE_MASKS)
pub(crate) const INTRA_ALL: i32 = (1 << INTRA_MODES) - 1;
pub(crate) const INTRA_DC: i32 = 1 << DC_PRED;
pub(crate) const INTRA_DC_H_V: i32 = (1 << DC_PRED) | (1 << V_PRED) | (1 << H_PRED);

// (RECODE_LOOP_TYPE)
pub(crate) const DISALLOW_RECODE: i32 = 0;
pub(crate) const ALLOW_RECODE_KFARFGF: i32 = 1;
pub(crate) const ALLOW_RECODE: i32 = 2;

// (LPF_PICK_METHOD)
pub(crate) const LPF_PICK_FROM_FULL_IMAGE: i32 = 0;
pub(crate) const LPF_PICK_FROM_FULL_IMAGE_NON_DUAL: i32 = 1;
pub(crate) const LPF_PICK_FROM_SUBIMAGE: i32 = 2;
pub(crate) const LPF_PICK_FROM_Q: i32 = 3;
pub(crate) const LPF_PICK_MINIMAL_LPF: i32 = 4;

// (MODE_SEARCH_SKIP_LOGIC)
pub(crate) const FLAG_SKIP_INTRA_DIRMISMATCH: i32 = 1 << 4;

// (TX_TYPE_PRUNE_MODE)
pub(crate) const TX_TYPE_PRUNE_0: i32 = 0;
pub(crate) const TX_TYPE_PRUNE_1: i32 = 1;
pub(crate) const TX_TYPE_PRUNE_2: i32 = 2;
pub(crate) const TX_TYPE_PRUNE_3: i32 = 3;
pub(crate) const TX_TYPE_PRUNE_4: i32 = 4;
pub(crate) const TX_TYPE_PRUNE_5: i32 = 5;

// (FAST_DETECTION_MAXQ)
pub(crate) const NO_DETECTION: i32 = 0;
pub(crate) const FAST_DETECTION_MAXQ: i32 = 1;

// (PARTITION_SEARCH_TYPE)
pub(crate) const SEARCH_PARTITION: i32 = 0;
pub(crate) const FIXED_PARTITION: i32 = 1;
pub(crate) const VAR_BASED_PARTITION: i32 = 2;
pub(crate) const ML_BASED_PARTITION: i32 = 3;

// (INTERNAL_COST_UPDATE_TYPE)
pub(crate) const INTERNAL_COST_UPD_OFF: i32 = 0; // Turn off cost updates.
pub(crate) const INTERNAL_COST_UPD_TILE: i32 = 1; // Update every tile.
pub(crate) const INTERNAL_COST_UPD_SBROW_SET: i32 = 2; // Update every row_set of height 256 pixs.
pub(crate) const INTERNAL_COST_UPD_SBROW: i32 = 3; // Update every sb rows inside a tile.
pub(crate) const INTERNAL_COST_UPD_SB: i32 = 4; // Update every sb.

// (SUBPEL_SEARCH_METHODS: only selects the fractional motion search)
pub(crate) const SUBPEL_TREE: i32 = 0;
pub(crate) const SUBPEL_TREE_PRUNED: i32 = 1;
pub(crate) const SUBPEL_TREE_PRUNED_MORE: i32 = 2;

pub(crate) const TX_DOMAIN_DIST_LEVELS: usize = 4;

static TX_DOMAIN_DIST_THRESHOLDS: [[u32; MODE_EVAL_TYPES]; 4] = [
    [u32::MAX, u32::MAX, u32::MAX],
    [22026, 22026, 22026],
    [1377, 1377, 1377],
    [0, 0, 0],
];

static TX_DOMAIN_DIST_TYPES: [[u32; MODE_EVAL_TYPES]; TX_DOMAIN_DIST_LEVELS] =
    [[0, 2, 0], [1, 2, 0], [2, 2, 0], [2, 2, 2]];

static COEFF_OPT_THRESHOLDS: [[[u32; 2]; MODE_EVAL_TYPES]; 9] = [
    [
        [u32::MAX, u32::MAX],
        [u32::MAX, u32::MAX],
        [u32::MAX, u32::MAX],
    ],
    [[3200, u32::MAX], [250, u32::MAX], [u32::MAX, u32::MAX]],
    [[1728, u32::MAX], [142, u32::MAX], [u32::MAX, u32::MAX]],
    [[864, u32::MAX], [142, u32::MAX], [u32::MAX, u32::MAX]],
    [[432, u32::MAX], [86, u32::MAX], [u32::MAX, u32::MAX]],
    [[864, 97], [142, 16], [u32::MAX, u32::MAX]],
    [[432, 97], [86, 16], [u32::MAX, u32::MAX]],
    [[216, 25], [86, 10], [u32::MAX, u32::MAX]],
    [[216, 25], [0, 10], [u32::MAX, u32::MAX]],
];

static TX_SIZE_SEARCH_METHODS: [[i32; MODE_EVAL_TYPES]; 4] = [
    [USE_FULL_RD, USE_LARGESTALL, USE_FULL_RD],
    [USE_FAST_RD, USE_LARGESTALL, USE_FULL_RD],
    [USE_LARGESTALL, USE_LARGESTALL, USE_FULL_RD],
    [USE_LARGESTALL, USE_LARGESTALL, USE_LARGESTALL],
];

static PREDICT_SKIP_LEVELS: [[u32; MODE_EVAL_TYPES]; 3] = [[0, 0, 0], [1, 1, 1], [1, 2, 1]];

static PREDICT_DC_LEVELS: [[u32; MODE_EVAL_TYPES]; 4] =
    [[0, 0, 0], [1, 1, 0], [2, 2, 0], [2, 2, 2]];

/// The high level speed features read by the saver's paths.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HighLevelSpeedFeatures {
    pub(crate) frame_parameter_update: i32,
    pub(crate) recode_loop: i32,
    pub(crate) accurate_bit_estimate: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TplSpeedFeatures {
    pub(crate) disable_filtered_key_tpl: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PartitionSpeedFeatures {
    pub(crate) partition_search_type: i32,
    pub(crate) default_max_partition_size: BlockSize,
    pub(crate) default_min_partition_size: BlockSize,
    pub(crate) max_intra_bsize: BlockSize,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MvSpeedFeatures {
    pub(crate) auto_mv_step_size: i32,
    pub(crate) subpel_search_method: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InterModeSpeedFeatures {
    pub(crate) selective_ref_frame: i32,
    pub(crate) use_dist_wtd_comp_flag: i32,
    pub(crate) mv_cost_upd_level: i32,
    pub(crate) coeff_cost_upd_level: i32,
    pub(crate) mode_cost_upd_level: i32,
    pub(crate) disable_onesided_comp: i32,
    pub(crate) prune_obmc_prob_thresh: i32,
    pub(crate) disable_interintra_wedge_var_thresh: u32,
    pub(crate) prune_warped_prob_thresh: i32,
    pub(crate) disable_masked_comp: i32,
    pub(crate) inter_mode_rd_model_estimation: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InterpFilterSpeedFeatures {
    pub(crate) disable_dual_filter: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IntraModeSpeedFeatures {
    pub(crate) dv_cost_upd_level: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TxTypeSearch {
    pub(crate) prune_2d_txfm_mode: i32,
    pub(crate) use_skip_flag_prediction: i32,
    pub(crate) fast_inter_tx_type_prob_thresh: i32,
    pub(crate) prune_tx_type_using_stats: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TxSpeedFeatures {
    pub(crate) tx_type_search: TxTypeSearch,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RdCalcSpeedFeatures {
    pub(crate) optimize_coefficients: i32,
    pub(crate) tx_domain_dist_level: i32,
    pub(crate) tx_domain_dist_thres_level: i32,
    pub(crate) perform_coeff_opt: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WinnerModeSpeedFeatures {
    pub(crate) tx_size_search_level: i32,
    pub(crate) enable_winner_mode_for_tx_size_srch: i32,
    pub(crate) dc_blk_pred_level: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LoopFilterSpeedFeatures {
    pub(crate) lpf_pick: i32,
    pub(crate) disable_lr_filter: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RealTimeSpeedFeatures {
    pub(crate) nonrd_check_partition_merge_mode: i32,
    pub(crate) nonrd_check_partition_split: i32,
    pub(crate) mode_search_skip_flags: i32,
    pub(crate) use_nonrd_pick_mode: i32,
    pub(crate) use_comp_ref_nonrd: i32,
    pub(crate) short_circuit_low_temp_var: i32,
    pub(crate) reuse_inter_pred_nonrd: i32,
    pub(crate) hybrid_intra_pickmode: i32,
    pub(crate) overshoot_detection_cbr: i32,
    pub(crate) use_temporal_noise_estimate: i32,
    pub(crate) intra_y_mode_bsize_mask_nrd: [i32; BLOCK_SIZES as usize],
    pub(crate) skip_cdef_sb: i32,
    pub(crate) force_large_partition_blocks_intra: i32,
    pub(crate) var_part_split_threshold_shift: i32,
    pub(crate) gf_refresh_based_on_qp: i32,
    pub(crate) use_rtc_tf: i32,
    pub(crate) part_early_exit_zeromv: i32,
    pub(crate) vbp_prune_16x16_split_using_min_max_sub_blk_var: bool,
    pub(crate) frame_level_mode_cost_update: i32,
    pub(crate) prune_h_pred_using_best_mode_so_far: bool,
    pub(crate) disable_cdf_update_non_reference_frame: bool,
    pub(crate) set_zeromv_skip_based_on_source_sad: i32,
    pub(crate) enable_ref_short_signaling: i32,
}

/// Translation of `SPEED_FEATURES` (see the module documentation).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SpeedFeatures {
    pub(crate) hl_sf: HighLevelSpeedFeatures,
    pub(crate) tpl_sf: TplSpeedFeatures,
    pub(crate) part_sf: PartitionSpeedFeatures,
    pub(crate) mv_sf: MvSpeedFeatures,
    pub(crate) inter_sf: InterModeSpeedFeatures,
    pub(crate) interp_sf: InterpFilterSpeedFeatures,
    pub(crate) intra_sf: IntraModeSpeedFeatures,
    pub(crate) tx_sf: TxSpeedFeatures,
    pub(crate) rd_sf: RdCalcSpeedFeatures,
    pub(crate) winner_mode_sf: WinnerModeSpeedFeatures,
    pub(crate) lpf_sf: LoopFilterSpeedFeatures,
    pub(crate) rt_sf: RealTimeSpeedFeatures,
}

// Intra only frames, golden frames (except alt ref overlays) and
// alt ref frames tend to be coded at a higher than ambient quality
#[inline]
fn frame_is_boosted(cpi: &Av1Comp) -> bool {
    frame_is_kf_gf_arf(cpi)
}

fn set_allintra_speed_feature_framesize_dependent(
    cpi: &Av1Comp,
    sf: &mut SpeedFeatures,
    speed: i32,
) {
    let cm = &cpi.common;
    let is_720p_or_larger = cm.width.min(cm.height) >= 720;
    let is_1080p_or_larger = cm.width.min(cm.height) >= 1080;
    let is_4k_or_larger = cm.width.min(cm.height) >= 2160;

    if is_4k_or_larger {
        sf.part_sf.default_min_partition_size = BLOCK_8X8;
    }

    if speed >= 3 && !is_720p_or_larger {
        sf.part_sf.max_intra_bsize = BLOCK_32X32;
    }

    if speed >= 4 {
        let is_480p_or_larger = cm.width.min(cm.height) >= 480;
        if is_480p_or_larger {
            sf.tx_sf.tx_type_search.prune_tx_type_using_stats = 2;
        }
    } else if speed >= 2 {
        let is_480p_or_larger = cm.width.min(cm.height) >= 480;
        if is_480p_or_larger {
            sf.tx_sf.tx_type_search.prune_tx_type_using_stats = 1;
        }
    }

    if speed >= 6 && is_1080p_or_larger {
        sf.part_sf.default_min_partition_size = BLOCK_8X8;
    }

    if speed >= 8 {
        let is_480p_or_larger = cm.width.min(cm.height) >= 480;
        if !is_480p_or_larger {
            sf.rt_sf.nonrd_check_partition_merge_mode = 2;
        }
        if is_720p_or_larger {
            sf.rt_sf.force_large_partition_blocks_intra = 1;
        }
    }

    if speed >= 9 {
        // TODO(kyslov): add more speed features to control speed/quality
        if !is_4k_or_larger {
            sf.inter_sf.coeff_cost_upd_level = INTERNAL_COST_UPD_OFF;
            sf.inter_sf.mode_cost_upd_level = INTERNAL_COST_UPD_OFF;
        }
    }
}

fn set_allintra_speed_features_framesize_independent(
    cpi: &Av1Comp,
    sf: &mut SpeedFeatures,
    speed: i32,
) {
    let _ = cpi;
    sf.intra_sf.dv_cost_upd_level = INTERNAL_COST_UPD_OFF;

    sf.rt_sf.use_nonrd_pick_mode = 0;

    if speed >= 1 {
        sf.tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_2;

        sf.rd_sf.perform_coeff_opt = 2;
        sf.rd_sf.tx_domain_dist_level = 1;
        sf.rd_sf.tx_domain_dist_thres_level = 1;
    }

    if speed >= 2 {
        sf.mv_sf.auto_mv_step_size = 1;

        sf.rd_sf.perform_coeff_opt = 3;
    }

    if speed >= 3 {
        sf.hl_sf.recode_loop = ALLOW_RECODE_KFARFGF;

        sf.tx_sf.tx_type_search.use_skip_flag_prediction = 2;
    }

    if speed >= 4 {
        sf.mv_sf.subpel_search_method = SUBPEL_TREE_PRUNED_MORE;

        sf.tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_3;

        sf.rd_sf.perform_coeff_opt = 5;
        sf.rd_sf.tx_domain_dist_thres_level = 3;

        sf.lpf_sf.lpf_pick = LPF_PICK_FROM_FULL_IMAGE_NON_DUAL;

        sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch = 1;
    }

    if speed >= 5 {
        sf.lpf_sf.disable_lr_filter = 1;
    }

    if speed >= 6 {
        // TODO(jingning): This might not be a good trade off if the
        // target image quality is very low.
        sf.part_sf.default_max_partition_size = BLOCK_32X32;

        sf.rd_sf.perform_coeff_opt = 6;
        sf.rd_sf.tx_domain_dist_level = 3;

        sf.lpf_sf.lpf_pick = LPF_PICK_FROM_Q;

        sf.winner_mode_sf.dc_blk_pred_level = 1;
    }
    // The following should make all-intra mode speed 7 approximately equal
    // to real-time speed 6,
    // all-intra speed 8 close to real-time speed 7, and all-intra speed 9
    // close to real-time speed 8
    if speed >= 7 {
        sf.part_sf.default_min_partition_size = BLOCK_8X8;
        sf.part_sf.partition_search_type = VAR_BASED_PARTITION;
        sf.rt_sf.mode_search_skip_flags |= FLAG_SKIP_INTRA_DIRMISMATCH;
        sf.rt_sf.var_part_split_threshold_shift = 7;
    }

    if speed >= 8 {
        sf.rt_sf.hybrid_intra_pickmode = 1;
        sf.rt_sf.use_nonrd_pick_mode = 1;
        sf.rt_sf.nonrd_check_partition_merge_mode = 1;
        sf.rt_sf.var_part_split_threshold_shift = 8;
        // Set mask for intra modes.
        for i in 0..BLOCK_SIZES {
            if i >= BLOCK_32X32 {
                sf.rt_sf.intra_y_mode_bsize_mask_nrd[i as usize] = INTRA_DC;
            } else {
                // Use DC, H, V intra mode for block sizes < 32X32.
                sf.rt_sf.intra_y_mode_bsize_mask_nrd[i as usize] = INTRA_DC_H_V;
            }
        }
    }

    if speed >= 9 {
        sf.inter_sf.coeff_cost_upd_level = INTERNAL_COST_UPD_SBROW;
        sf.inter_sf.mode_cost_upd_level = INTERNAL_COST_UPD_SBROW;

        sf.rt_sf.nonrd_check_partition_merge_mode = 0;
        sf.rt_sf.hybrid_intra_pickmode = 0;
        sf.rt_sf.var_part_split_threshold_shift = 9;
        sf.rt_sf.vbp_prune_16x16_split_using_min_max_sub_blk_var = true;
        sf.rt_sf.prune_h_pred_using_best_mode_so_far = true;
    }
}

#[inline]
fn init_hl_sf(hl_sf: &mut HighLevelSpeedFeatures) {
    // best quality defaults
    hl_sf.frame_parameter_update = 1;
    hl_sf.recode_loop = ALLOW_RECODE;
    hl_sf.accurate_bit_estimate = 0;
}

#[inline]
fn init_tpl_sf(tpl_sf: &mut TplSpeedFeatures) {
    tpl_sf.disable_filtered_key_tpl = 0;
}

#[inline]
fn init_part_sf(part_sf: &mut PartitionSpeedFeatures) {
    part_sf.partition_search_type = SEARCH_PARTITION;
    part_sf.default_max_partition_size = BLOCK_LARGEST;
    part_sf.default_min_partition_size = BLOCK_4X4;
    part_sf.max_intra_bsize = BLOCK_LARGEST;
}

#[inline]
fn init_mv_sf(mv_sf: &mut MvSpeedFeatures) {
    mv_sf.auto_mv_step_size = 0;
    mv_sf.subpel_search_method = SUBPEL_TREE;
}

#[inline]
fn init_inter_sf(inter_sf: &mut InterModeSpeedFeatures) {
    inter_sf.selective_ref_frame = 0;
    inter_sf.use_dist_wtd_comp_flag = DIST_WTD_COMP_ENABLED;
    inter_sf.mv_cost_upd_level = INTERNAL_COST_UPD_SB;
    inter_sf.coeff_cost_upd_level = INTERNAL_COST_UPD_SB;
    inter_sf.mode_cost_upd_level = INTERNAL_COST_UPD_SB;
    inter_sf.inter_mode_rd_model_estimation = 0;
    inter_sf.disable_onesided_comp = 0;
    inter_sf.prune_obmc_prob_thresh = 0;
    inter_sf.disable_interintra_wedge_var_thresh = 0;
    inter_sf.prune_warped_prob_thresh = 0;
    inter_sf.disable_masked_comp = 0;
}

#[inline]
fn init_interp_sf(interp_sf: &mut InterpFilterSpeedFeatures) {
    interp_sf.disable_dual_filter = 0;
}

#[inline]
fn init_intra_sf(intra_sf: &mut IntraModeSpeedFeatures) {
    intra_sf.dv_cost_upd_level = INTERNAL_COST_UPD_SB;
}

#[inline]
fn init_tx_sf(tx_sf: &mut TxSpeedFeatures) {
    tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_1;
    tx_sf.tx_type_search.use_skip_flag_prediction = 1;
    tx_sf.tx_type_search.fast_inter_tx_type_prob_thresh = i32::MAX;
    tx_sf.tx_type_search.prune_tx_type_using_stats = 0;
}

#[inline]
fn init_rd_sf(rd_sf: &mut RdCalcSpeedFeatures, oxcf: &Av1EncoderConfig) {
    let disable_trellis_quant = oxcf.algo_cfg.disable_trellis_quant;
    if disable_trellis_quant == 3 {
        rd_sf.optimize_coefficients = if !is_lossless_requested(&oxcf.rc_cfg) {
            NO_ESTIMATE_YRD_TRELLIS_OPT
        } else {
            NO_TRELLIS_OPT
        };
    } else if disable_trellis_quant == 2 {
        rd_sf.optimize_coefficients = if !is_lossless_requested(&oxcf.rc_cfg) {
            FINAL_PASS_TRELLIS_OPT
        } else {
            NO_TRELLIS_OPT
        };
    } else if disable_trellis_quant == 0 {
        if is_lossless_requested(&oxcf.rc_cfg) {
            rd_sf.optimize_coefficients = NO_TRELLIS_OPT;
        } else {
            rd_sf.optimize_coefficients = FULL_TRELLIS_OPT;
        }
    } else if disable_trellis_quant == 1 {
        rd_sf.optimize_coefficients = NO_TRELLIS_OPT;
    } else {
        debug_assert!(false, "Invalid disable_trellis_quant value");
    }
    rd_sf.tx_domain_dist_level = 0;
    rd_sf.tx_domain_dist_thres_level = 0;
    rd_sf.perform_coeff_opt = 0;
}

#[inline]
fn init_winner_mode_sf(winner_mode_sf: &mut WinnerModeSpeedFeatures) {
    // Set this at the appropriate speed levels
    winner_mode_sf.tx_size_search_level = 0;
    winner_mode_sf.enable_winner_mode_for_tx_size_srch = 0;
    winner_mode_sf.dc_blk_pred_level = 0;
}

#[inline]
fn init_lpf_sf(lpf_sf: &mut LoopFilterSpeedFeatures) {
    lpf_sf.lpf_pick = LPF_PICK_FROM_FULL_IMAGE;
    lpf_sf.disable_lr_filter = 0;
}

#[inline]
fn init_rt_sf(rt_sf: &mut RealTimeSpeedFeatures) {
    rt_sf.nonrd_check_partition_merge_mode = 0;
    rt_sf.nonrd_check_partition_split = 0;
    rt_sf.mode_search_skip_flags = 0;
    rt_sf.use_nonrd_pick_mode = 0;
    rt_sf.use_comp_ref_nonrd = 0;
    rt_sf.short_circuit_low_temp_var = 0;
    rt_sf.reuse_inter_pred_nonrd = 0;
    rt_sf.hybrid_intra_pickmode = 0;
    rt_sf.overshoot_detection_cbr = NO_DETECTION;
    rt_sf.use_temporal_noise_estimate = 0;
    for i in 0..BLOCK_SIZES as usize {
        rt_sf.intra_y_mode_bsize_mask_nrd[i] = INTRA_ALL;
    }
    rt_sf.skip_cdef_sb = 0;
    rt_sf.force_large_partition_blocks_intra = 0;
    rt_sf.var_part_split_threshold_shift = 7;
    rt_sf.gf_refresh_based_on_qp = 0;
    rt_sf.use_rtc_tf = 0;
    rt_sf.part_early_exit_zeromv = 0;
    rt_sf.vbp_prune_16x16_split_using_min_max_sub_blk_var = false;
    rt_sf.frame_level_mode_cost_update = 0;
    rt_sf.prune_h_pred_using_best_mode_so_far = false;
    rt_sf.disable_cdf_update_non_reference_frame = false;
    rt_sf.set_zeromv_skip_based_on_source_sad = 1;
    rt_sf.enable_ref_short_signaling = 0;
}

// (set_subpel_search_method(): only selects the fractional motion search
// function, never called for intra frames)

pub(crate) fn av1_set_speed_features_framesize_dependent(cpi: &mut Av1Comp, speed: i32) {
    let mut sf = cpi.sf;

    debug_assert!(cpi.oxcf.mode == ALLINTRA);
    set_allintra_speed_feature_framesize_dependent(cpi, &mut sf, speed);

    if !cpi.ppi.seq_params_locked {
        let seq_params = &mut cpi.common.seq_params;
        seq_params.enable_masked_compound &= (sf.inter_sf.disable_masked_comp == 0) as u8;
        seq_params.enable_interintra_compound &=
            (sf.inter_sf.disable_interintra_wedge_var_thresh != u32::MAX) as u8;
    }

    // For multi-thread use case with row_mt enabled, cost update for a set of
    // SB rows is not desirable. Hence, the sf mv_cost_upd_level is set to
    // INTERNAL_COST_UPD_SBROW in such cases.
    // (one thread)
    cpi.sf = sf;
}

pub(crate) fn av1_set_speed_features_framesize_independent(cpi: &mut Av1Comp, speed: i32) {
    let mut sf = SpeedFeatures::default();

    init_hl_sf(&mut sf.hl_sf);
    init_tpl_sf(&mut sf.tpl_sf);
    init_part_sf(&mut sf.part_sf);
    init_mv_sf(&mut sf.mv_sf);
    init_inter_sf(&mut sf.inter_sf);
    init_interp_sf(&mut sf.interp_sf);
    init_intra_sf(&mut sf.intra_sf);
    init_tx_sf(&mut sf.tx_sf);
    init_rd_sf(&mut sf.rd_sf, &cpi.oxcf);
    init_winner_mode_sf(&mut sf.winner_mode_sf);
    init_lpf_sf(&mut sf.lpf_sf);
    init_rt_sf(&mut sf.rt_sf);

    debug_assert!(cpi.oxcf.mode == ALLINTRA);
    set_allintra_speed_features_framesize_independent(cpi, &mut sf, speed);

    // (enable_tx_size_search is on)
    debug_assert!(cpi.oxcf.txfm_cfg.enable_tx_size_search || sf.rt_sf.use_nonrd_pick_mode != 0);

    if !cpi.ppi.seq_params_locked {
        let seq_params = &mut cpi.common.seq_params;
        seq_params.order_hint_info.enable_dist_wtd_comp &=
            (sf.inter_sf.use_dist_wtd_comp_flag != DIST_WTD_COMP_DISABLED) as i32;
        seq_params.enable_dual_filter &= (sf.interp_sf.disable_dual_filter == 0) as u8;
        seq_params.enable_restoration &= (sf.lpf_sf.disable_lr_filter == 0) as u8;

        seq_params.enable_interintra_compound &=
            (sf.inter_sf.disable_interintra_wedge_var_thresh != u32::MAX) as u8;
    }

    // (the mesh patterns only steer exhaustive motion searches)

    // Slow quant, dct and trellis not worthwhile for first pass
    // so make sure they are always turned off.
    if is_stat_generation_stage(cpi) {
        sf.rd_sf.optimize_coefficients = NO_TRELLIS_OPT;
    }

    // No recode for 1 pass.
    if cpi.oxcf.pass == AOM_RC_ONE_PASS && has_no_stats_stage(cpi) {
        sf.hl_sf.recode_loop = DISALLOW_RECODE;
    }

    let winner_mode_params = &mut cpi.winner_mode_params;
    // assert ensures that tx_domain_dist_level is accessed correctly
    debug_assert!(
        sf.rd_sf.tx_domain_dist_thres_level >= 0 && sf.rd_sf.tx_domain_dist_thres_level < 4
    );
    winner_mode_params.tx_domain_dist_threshold =
        TX_DOMAIN_DIST_THRESHOLDS[sf.rd_sf.tx_domain_dist_thres_level as usize];

    debug_assert!(
        sf.rd_sf.tx_domain_dist_level >= 0
            && (sf.rd_sf.tx_domain_dist_level as usize) < TX_DOMAIN_DIST_LEVELS
    );
    winner_mode_params.use_transform_domain_distortion =
        TX_DOMAIN_DIST_TYPES[sf.rd_sf.tx_domain_dist_level as usize];

    // assert ensures that coeff_opt_thresholds is accessed correctly
    debug_assert!(sf.rd_sf.perform_coeff_opt >= 0 && sf.rd_sf.perform_coeff_opt < 9);
    winner_mode_params.coeff_opt_thresholds =
        COEFF_OPT_THRESHOLDS[sf.rd_sf.perform_coeff_opt as usize];

    // assert ensures that predict_skip_levels is accessed correctly
    debug_assert!(
        sf.tx_sf.tx_type_search.use_skip_flag_prediction >= 0
            && sf.tx_sf.tx_type_search.use_skip_flag_prediction < 3
    );
    winner_mode_params.skip_txfm_level =
        PREDICT_SKIP_LEVELS[sf.tx_sf.tx_type_search.use_skip_flag_prediction as usize];

    // assert ensures that tx_size_search_level is accessed correctly
    debug_assert!(
        sf.winner_mode_sf.tx_size_search_level >= 0 && sf.winner_mode_sf.tx_size_search_level <= 3
    );
    winner_mode_params.tx_size_search_methods =
        TX_SIZE_SEARCH_METHODS[sf.winner_mode_sf.tx_size_search_level as usize];
    winner_mode_params.predict_dc_level =
        PREDICT_DC_LEVELS[sf.winner_mode_sf.dc_blk_pred_level as usize];

    // (one thread)

    if cpi.oxcf.rc_cfg.gf_cbr_boost_pct > 0 {
        sf.rt_sf.gf_refresh_based_on_qp = 0;
    }
    cpi.sf = sf;
}

// Override some speed features based on qindex
// (the saver's speed (9) and key frames leave every feature the paths read
// as it is: the overrides only change partition and loop restoration
// searches of inter frames and lower speeds)
pub(crate) fn av1_set_speed_features_qindex_dependent(cpi: &mut Av1Comp, speed: i32) {
    let _ = frame_is_boosted(cpi);
    debug_assert!(speed >= 4 && frame_is_intra_only(&cpi.common));
}
