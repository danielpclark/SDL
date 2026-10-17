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
use crate::aom::av1::common::enums::*;

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
// (the non-RD pick mode only evaluates with DEFAULT_EVAL)
#[inline]
pub(crate) fn set_mode_eval_params(cpi: &Av1Comp, x: &mut Macroblock, mode_eval_type: i32) {
    let cm = &cpi.common;
    let sf = &cpi.sf;
    let winner_mode_params = &cpi.winner_mode_params;
    let txfm_params = &mut x.txfm_search_params;

    txfm_params.use_qm_dist_metric =
        (cpi.oxcf.tune_cfg.dist_metric == AOM_DIST_METRIC_QM_PSNR) as i32;

    debug_assert!(mode_eval_type == DEFAULT_EVAL);
    txfm_params.default_inter_tx_type_prob_thresh = i32::MAX;
    txfm_params.use_default_intra_tx_type = 0;
    txfm_params.skip_txfm_level = winner_mode_params.skip_txfm_level[DEFAULT_EVAL as usize];
    txfm_params.predict_dc_level = winner_mode_params.predict_dc_level[DEFAULT_EVAL as usize];
    // Set default transform domain distortion type
    set_tx_domain_dist_params(winner_mode_params, txfm_params, 0, 0);

    // Get default threshold for R-D optimization of coefficients
    get_rd_opt_coeff_thresh(&winner_mode_params.coeff_opt_thresholds, txfm_params, 0, 0);

    // Set default transform size search method
    set_tx_size_search_method(cm, winner_mode_params, txfm_params, 0, 0);
    // Set default transform type prune
    set_tx_type_prune(sf, txfm_params, 0, 0);

    // (reset_mb_rd_record(): the non-RD pick mode keeps no RD records)
    txfm_params.mode_eval_type = mode_eval_type;
}
