// Rust translation of the parts of av1/encoder/rdopt_utils.h (and
// get_rd_opt_coeff_thresh() from av1/encoder/rd.h) from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2019, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transform search parameters of a mode evaluation stage.

use super::block::*;
use super::encoder::{Av1Comp, WinnerModeParams, AOM_DIST_METRIC_QM_PSNR};
use super::speed_features::*;
use crate::aom::av1::common::av1_common_int::Av1Common;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cfl::is_cfl_allowed;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::quant_common::MAXQ;

// (rd.h)
#[inline]
pub(crate) fn get_rd_opt_coeff_thresh(
    coeff_opt_threshold: &[[u32; 2]; MODE_EVAL_TYPES],
    txfm_params: &mut TxfmSearchParams,
    enable_winner_mode_for_coeff_opt: i32,
    is_winner_mode: i32,
) {
    if enable_winner_mode_for_coeff_opt == 0 {
        // Default initialization of threshold
        txfm_params.coeff_opt_thresholds[0] = coeff_opt_threshold[DEFAULT_EVAL as usize][0];
        txfm_params.coeff_opt_thresholds[1] = coeff_opt_threshold[DEFAULT_EVAL as usize][1];
        return;
    }
    // TODO(any): Experiment with coeff_opt_dist_threshold values when
    // enable_winner_mode_for_coeff_opt is ON
    // TODO(any): Skip the winner mode processing for blocks with lower residual
    // energy as R-D optimization of coefficients would have been enabled during
    // mode decision

    // Use conservative threshold during mode decision and perform R-D
    // optimization of coeffs always for winner modes
    if is_winner_mode != 0 {
        txfm_params.coeff_opt_thresholds[0] = coeff_opt_threshold[WINNER_MODE_EVAL as usize][0];
        txfm_params.coeff_opt_thresholds[1] = coeff_opt_threshold[WINNER_MODE_EVAL as usize][1];
    } else {
        txfm_params.coeff_opt_thresholds[0] = coeff_opt_threshold[MODE_EVAL as usize][0];
        txfm_params.coeff_opt_thresholds[1] = coeff_opt_threshold[MODE_EVAL as usize][1];
    }
}

// (rdopt_utils.h)

#[inline]
pub(crate) fn select_tx_mode(cm: &Av1Common, tx_size_search_method: i32) -> TxMode {
    if cm.features.coded_lossless {
        return ONLY_4X4;
    }
    if tx_size_search_method == USE_LARGESTALL {
        TX_MODE_LARGEST
    } else {
        debug_assert!(tx_size_search_method == USE_FULL_RD || tx_size_search_method == USE_FAST_RD);
        TX_MODE_SELECT
    }
}

#[inline]
pub(crate) fn set_tx_size_search_method(
    cm: &Av1Common,
    winner_mode_params: &WinnerModeParams,
    txfm_params: &mut TxfmSearchParams,
    enable_winner_mode_for_tx_size_srch: i32,
    is_winner_mode: i32,
) {
    // Populate transform size search method/transform mode appropriately
    txfm_params.tx_size_search_method =
        winner_mode_params.tx_size_search_methods[DEFAULT_EVAL as usize];
    if enable_winner_mode_for_tx_size_srch != 0 {
        if is_winner_mode != 0 {
            txfm_params.tx_size_search_method =
                winner_mode_params.tx_size_search_methods[WINNER_MODE_EVAL as usize];
        } else {
            txfm_params.tx_size_search_method =
                winner_mode_params.tx_size_search_methods[MODE_EVAL as usize];
        }
    }
    txfm_params.tx_mode_search_type = select_tx_mode(cm, txfm_params.tx_size_search_method);
}

#[inline]
pub(crate) fn set_tx_type_prune(
    sf: &SpeedFeatures,
    txfm_params: &mut TxfmSearchParams,
    winner_mode_tx_type_pruning: i32,
    is_winner_mode: i32,
) {
    // Populate prune transform mode appropriately
    txfm_params.prune_2d_txfm_mode = sf.tx_sf.tx_type_search.prune_2d_txfm_mode;
    if winner_mode_tx_type_pruning == 0 {
        return;
    }

    let prune_mode: [[i32; 2]; 4] = [
        [TX_TYPE_PRUNE_3, TX_TYPE_PRUNE_0],
        [TX_TYPE_PRUNE_4, TX_TYPE_PRUNE_0],
        [TX_TYPE_PRUNE_5, TX_TYPE_PRUNE_2],
        [TX_TYPE_PRUNE_5, TX_TYPE_PRUNE_3],
    ];
    txfm_params.prune_2d_txfm_mode =
        prune_mode[(winner_mode_tx_type_pruning - 1) as usize][is_winner_mode as usize];
}

#[inline]
pub(crate) fn set_tx_domain_dist_params(
    winner_mode_params: &WinnerModeParams,
    txfm_params: &mut TxfmSearchParams,
    enable_winner_mode_for_tx_domain_dist: i32,
    is_winner_mode: i32,
) {
    if txfm_params.use_qm_dist_metric != 0 {
        // QM-weighted PSNR is computed in transform space, so we need to forcibly
        // enable the use of tx domain distortion.
        txfm_params.use_transform_domain_distortion = 1;
        txfm_params.tx_domain_dist_threshold = 0;
        return;
    }

    if enable_winner_mode_for_tx_domain_dist == 0 {
        txfm_params.use_transform_domain_distortion =
            winner_mode_params.use_transform_domain_distortion[DEFAULT_EVAL as usize];
        txfm_params.tx_domain_dist_threshold =
            winner_mode_params.tx_domain_dist_threshold[DEFAULT_EVAL as usize];
        return;
    }

    if is_winner_mode != 0 {
        txfm_params.use_transform_domain_distortion =
            winner_mode_params.use_transform_domain_distortion[WINNER_MODE_EVAL as usize];
        txfm_params.tx_domain_dist_threshold =
            winner_mode_params.tx_domain_dist_threshold[WINNER_MODE_EVAL as usize];
    } else {
        txfm_params.use_transform_domain_distortion =
            winner_mode_params.use_transform_domain_distortion[MODE_EVAL as usize];
        txfm_params.tx_domain_dist_threshold =
            winner_mode_params.tx_domain_dist_threshold[MODE_EVAL as usize];
    }
}

// This function sets mode parameters for different mode evaluation stages
#[inline]
pub(crate) fn set_mode_eval_params(cpi: &Av1Comp, x: &mut Macroblock, mode_eval_type: i32) {
    let cm = &cpi.common;
    let sf = &cpi.sf;
    let winner_mode_params = &cpi.winner_mode_params;
    let txfm_params = &mut x.txfm_search_params;

    txfm_params.use_qm_dist_metric =
        (cpi.oxcf.tune_cfg.dist_metric == AOM_DIST_METRIC_QM_PSNR) as i32;

    match mode_eval_type {
        DEFAULT_EVAL => {
            txfm_params.default_inter_tx_type_prob_thresh = i32::MAX;
            txfm_params.use_default_intra_tx_type = 0;
            txfm_params.skip_txfm_level = winner_mode_params.skip_txfm_level[DEFAULT_EVAL as usize];
            txfm_params.predict_dc_level =
                winner_mode_params.predict_dc_level[DEFAULT_EVAL as usize];
            // Set default transform domain distortion type
            set_tx_domain_dist_params(winner_mode_params, txfm_params, 0, 0);

            // Get default threshold for R-D optimization of coefficients
            get_rd_opt_coeff_thresh(&winner_mode_params.coeff_opt_thresholds, txfm_params, 0, 0);

            // Set default transform size search method
            set_tx_size_search_method(cm, winner_mode_params, txfm_params, 0, 0);
            // Set default transform type prune
            set_tx_type_prune(sf, txfm_params, 0, 0);
        }
        MODE_EVAL => {
            txfm_params.use_default_intra_tx_type =
                (cpi.sf.tx_sf.tx_type_search.fast_intra_tx_type_search != 0
                    || cpi.oxcf.txfm_cfg.use_intra_default_tx_only) as i32;
            txfm_params.default_inter_tx_type_prob_thresh =
                cpi.sf.tx_sf.tx_type_search.fast_inter_tx_type_prob_thresh;
            txfm_params.skip_txfm_level = winner_mode_params.skip_txfm_level[MODE_EVAL as usize];
            txfm_params.predict_dc_level = winner_mode_params.predict_dc_level[MODE_EVAL as usize];
            // Set transform domain distortion type for mode evaluation
            set_tx_domain_dist_params(
                winner_mode_params,
                txfm_params,
                sf.winner_mode_sf.enable_winner_mode_for_use_tx_domain_dist,
                0,
            );

            // Get threshold for R-D optimization of coefficients during mode
            // evaluation
            get_rd_opt_coeff_thresh(
                &winner_mode_params.coeff_opt_thresholds,
                txfm_params,
                sf.winner_mode_sf.enable_winner_mode_for_coeff_opt,
                0,
            );

            // Set the transform size search method for mode evaluation
            set_tx_size_search_method(
                cm,
                winner_mode_params,
                txfm_params,
                sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch,
                0,
            );
            // Set transform type prune for mode evaluation
            set_tx_type_prune(
                sf,
                txfm_params,
                sf.tx_sf.tx_type_search.winner_mode_tx_type_pruning,
                0,
            );
        }
        _ => {
            debug_assert!(mode_eval_type == WINNER_MODE_EVAL);
            txfm_params.default_inter_tx_type_prob_thresh = i32::MAX;
            txfm_params.use_default_intra_tx_type = 0;
            txfm_params.skip_txfm_level =
                winner_mode_params.skip_txfm_level[WINNER_MODE_EVAL as usize];
            txfm_params.predict_dc_level =
                winner_mode_params.predict_dc_level[WINNER_MODE_EVAL as usize];

            // Set transform domain distortion type for winner mode evaluation
            set_tx_domain_dist_params(
                winner_mode_params,
                txfm_params,
                sf.winner_mode_sf.enable_winner_mode_for_use_tx_domain_dist,
                1,
            );

            // Get threshold for R-D optimization of coefficients for winner mode
            // evaluation
            get_rd_opt_coeff_thresh(
                &winner_mode_params.coeff_opt_thresholds,
                txfm_params,
                sf.winner_mode_sf.enable_winner_mode_for_coeff_opt,
                1,
            );

            // Set the transform size search method for winner mode evaluation
            set_tx_size_search_method(
                cm,
                winner_mode_params,
                txfm_params,
                sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch,
                1,
            );
            // Set default transform type prune mode for winner mode evaluation
            set_tx_type_prune(
                sf,
                txfm_params,
                sf.tx_sf.tx_type_search.winner_mode_tx_type_pruning,
                1,
            );
        }
    }

    // Rd record collected at a specific mode evaluation stage can not be used
    // across other evaluation stages as the transform parameters are different.
    // Hence, reset mb rd record whenever mode evaluation stage type changes.
    // (reset_mb_rd_record(): the RD records are only used by the RD inter
    // search; x->txfm_search_info.mb_rd_record stays NULL)
    txfm_params.mode_eval_type = mode_eval_type;
}

// Get transform block visible dimensions cropped to the MI units.
/// Translation of `get_txb_dimensions()`.
#[inline]
pub(crate) fn get_txb_dimensions(
    xd: &MacroBlockD,
    plane: usize,
    plane_bsize: BlockSize,
    blk_row: i32,
    blk_col: i32,
    tx_bsize: BlockSize,
    width: Option<&mut i32>,
    height: Option<&mut i32>,
    visible_width: &mut i32,
    visible_height: &mut i32,
) {
    debug_assert!(tx_bsize <= plane_bsize);
    let txb_height = BLOCK_SIZE_HIGH[tx_bsize as usize] as i32;
    let txb_width = BLOCK_SIZE_WIDE[tx_bsize as usize] as i32;
    let pd = &xd.plane[plane];

    // TODO(aconverse@google.com): Investigate using crop_width/height here rather
    // than the MI size
    if xd.mb_to_bottom_edge >= 0 {
        *visible_height = txb_height;
    } else {
        let block_height = BLOCK_SIZE_HIGH[plane_bsize as usize] as i32;
        let block_rows = (xd.mb_to_bottom_edge >> (3 + pd.subsampling_y)) + block_height;
        *visible_height = (block_rows - (blk_row << MI_SIZE_LOG2)).clamp(0, txb_height);
    }
    if let Some(height) = height {
        *height = txb_height;
    }

    if xd.mb_to_right_edge >= 0 {
        *visible_width = txb_width;
    } else {
        let block_width = BLOCK_SIZE_WIDE[plane_bsize as usize] as i32;
        let block_cols = (xd.mb_to_right_edge >> (3 + pd.subsampling_x)) + block_width;
        *visible_width = (block_cols - (blk_col << MI_SIZE_LOG2)).clamp(0, txb_width);
    }
    if let Some(width) = width {
        *width = txb_width;
    }
}

// Checks the conditions to disable winner mode processing
#[inline]
fn bypass_winner_mode_processing(
    x: &Macroblock,
    sf: &SpeedFeatures,
    use_txfm_skip: i32,
    actual_txfm_skip: i32,
    best_mode: PredictionMode,
) -> bool {
    let prune_winner_mode_eval_level = sf.winner_mode_sf.prune_winner_mode_eval_level;

    // Disable winner mode processing for blocks with low source variance.
    // The aggressiveness of this pruning logic reduces as qindex increases.
    // The threshold decreases linearly from 64 as qindex varies from 0 to 255.
    if prune_winner_mode_eval_level == 1 {
        let src_var_thresh = (64 - 48 * x.qindex / (MAXQ + 1)) as u32;
        if x.source_variance < src_var_thresh {
            return true;
        }
    } else if prune_winner_mode_eval_level == 2 {
        // Skip winner mode processing of blocks for which transform turns out to be
        // skip due to nature of eob alone except NEWMV mode.
        if !have_newmv_in_inter_mode(best_mode) && actual_txfm_skip != 0 {
            return true;
        }
    } else if prune_winner_mode_eval_level == 3 {
        // Skip winner mode processing of blocks for which transform turns out to be
        // skip except NEWMV mode and considered based on the quantizer.
        // At high quantizers: Take conservative approach by considering transform
        // skip based on eob alone.
        // At low quantizers: Consider transform skip based on eob nature or RD cost
        // evaluation.
        let is_txfm_skip = if x.qindex > 127 {
            actual_txfm_skip != 0
        } else {
            actual_txfm_skip != 0 || use_txfm_skip != 0
        };

        if !have_newmv_in_inter_mode(best_mode) && is_txfm_skip {
            return true;
        }
    } else if prune_winner_mode_eval_level >= 4 {
        // Do not skip winner mode evaluation at low quantizers if normal mode's
        // transform search was too aggressive.
        if sf.rd_sf.perform_coeff_opt >= 5 && x.qindex <= 70 {
            return false;
        }

        if use_txfm_skip != 0 || actual_txfm_skip != 0 {
            return true;
        }
    }

    false
}

// Checks the conditions to enable winner mode processing
/// Translation of `is_winner_mode_processing_enabled()`.
#[inline]
pub(crate) fn is_winner_mode_processing_enabled(
    cpi: &Av1Comp,
    x: &Macroblock,
    mbmi: &MbModeInfo,
    actual_txfm_skip: i32,
) -> Result<bool, ()> {
    let sf = &cpi.sf;
    let best_mode = mbmi.mode;

    if bypass_winner_mode_processing(x, sf, mbmi.skip_txfm as i32, actual_txfm_skip, best_mode) {
        return Ok(false);
    }

    // TODO(any): Move block independent condition checks to frame level
    if is_inter_block(mbmi) {
        // (the inter RD search: never used)
        return Err(());
    } else if sf.tx_sf.tx_type_search.fast_intra_tx_type_search != 0
        && !cpi.oxcf.txfm_cfg.use_intra_default_tx_only
        && !cpi.oxcf.txfm_cfg.use_intra_dct_only
    {
        return Ok(true);
    }

    // Check speed feature related to winner mode processing
    if sf.winner_mode_sf.enable_winner_mode_for_coeff_opt != 0
        && cpi.optimize_seg_arr[mbmi.segment_id as usize] != NO_TRELLIS_OPT
        && cpi.optimize_seg_arr[mbmi.segment_id as usize] != FINAL_PASS_TRELLIS_OPT
    {
        return Ok(true);
    }
    if sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch != 0 {
        return Ok(true);
    }

    Ok(false)
}

// (MULTI_WINNER_MODE_TYPE winner_mode_count_allowed[])
pub(crate) static WINNER_MODE_COUNT_ALLOWED: [i32; MULTI_WINNER_MODE_LEVELS as usize] = [
    1, // MULTI_WINNER_MODE_OFF
    2, // MULTI_WINNER_MODE_FAST
    3, // MULTI_WINNER_MODE_DEFAULT
];

#[inline]
pub(crate) fn init_sbuv_mode(mbmi: &mut MbModeInfo) {
    mbmi.uv_mode = UV_DC_PRED;
    mbmi.palette_mode_info.palette_size[1] = 0;
}

/// Translation of `store_cfl_required_rdo()`.
#[inline]
pub(crate) fn store_cfl_required_rdo(cm: &Av1Common, x: &Macroblock) -> i32 {
    let xd = &x.e_mbd;

    if cm.seq_params.monochrome != 0 || !xd.is_chroma_ref {
        return CFL_DISALLOWED;
    }

    if !xd.is_chroma_ref {
        // For non-chroma-reference blocks, we should always store the luma pixels,
        // in case the corresponding chroma-reference block uses CfL.
        // Note that this can only happen for block sizes which are <8 on
        // their shortest side, as otherwise they would be chroma reference
        // blocks.
        return CFL_ALLOWED;
    }

    // For chroma reference blocks, we should store data in the encoder iff we're
    // allowed to try out CfL.
    is_cfl_allowed(xd)
}

/// Translation of `store_winner_mode_stats()`: the stats are only kept for
/// multi-winner mode processing, which the saver's speed features turn
/// off.
#[inline]
pub(crate) fn store_winner_mode_stats(multi_winner_mode_type: i32) -> Result<(), ()> {
    // Mode stat is not required when multiwinner mode processing is disabled
    if multi_winner_mode_type == MULTI_WINNER_MODE_OFF {
        return Ok(());
    }
    Err(())
}

/// Translation of `bsize_to_num_blk()`.
#[inline]
pub(crate) fn bsize_to_num_blk(bsize: BlockSize) -> i32 {
    1 << (NUM_PELS_LOG2_LOOKUP[bsize as usize] as i32 - 2 * MI_SIZE_LOG2)
}
