// Rust translation of av1/encoder/speed_features.c and
// av1/encoder/speed_features.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches): the all intra and
// real-time speed features (the good quality usage is not translated).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Speed features: the structures whole, set exactly as upstream's
//! functions set them for the all intra and real-time usages.

use super::block::*;
use super::encoder::*;
use super::mcomp::*;
use super::ratectrl::INTNL_ARF_UPDATE;
use crate::aom::av1::common::av1_common_int::frame_is_intra_only;
use crate::aom::av1::common::blockd::KEY_FRAME;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::*;
use crate::aom::av1::common::quant_common::MAXQ;

pub(crate) const MAX_MESH_STEP: i32 = 4;

/// `MESH_PATTERN`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MeshPattern {
    pub(crate) range: i32,
    pub(crate) interval: i32,
}

// (GM_SEARCH_TYPE)
pub(crate) const GM_FULL_SEARCH: i32 = 0;
pub(crate) const GM_REDUCED_REF_SEARCH_SKIP_L2_L3: i32 = 1;
pub(crate) const GM_REDUCED_REF_SEARCH_SKIP_L2_L3_ARF2: i32 = 2;
pub(crate) const GM_DISABLE_SEARCH: i32 = 3;

// (DIST_WTD_COMP_FLAG)
pub(crate) const DIST_WTD_COMP_ENABLED: i32 = 0;
pub(crate) const DIST_WTD_COMP_SKIP_MV_SEARCH: i32 = 1;
pub(crate) const DIST_WTD_COMP_DISABLED: i32 = 2;

// (INTRA_MODE_MASKS)
pub(crate) const INTRA_ALL: i32 = (1 << DC_PRED)
    | (1 << V_PRED)
    | (1 << H_PRED)
    | (1 << D45_PRED)
    | (1 << D135_PRED)
    | (1 << D113_PRED)
    | (1 << D157_PRED)
    | (1 << D203_PRED)
    | (1 << D67_PRED)
    | (1 << SMOOTH_PRED)
    | (1 << SMOOTH_V_PRED)
    | (1 << SMOOTH_H_PRED)
    | (1 << PAETH_PRED);
pub(crate) const UV_INTRA_ALL: i32 = (1 << UV_DC_PRED)
    | (1 << UV_V_PRED)
    | (1 << UV_H_PRED)
    | (1 << UV_D45_PRED)
    | (1 << UV_D135_PRED)
    | (1 << UV_D113_PRED)
    | (1 << UV_D157_PRED)
    | (1 << UV_D203_PRED)
    | (1 << UV_D67_PRED)
    | (1 << UV_SMOOTH_PRED)
    | (1 << UV_SMOOTH_V_PRED)
    | (1 << UV_SMOOTH_H_PRED)
    | (1 << UV_PAETH_PRED)
    | (1 << UV_CFL_PRED);
pub(crate) const UV_INTRA_DC: i32 = 1 << UV_DC_PRED;
pub(crate) const UV_INTRA_DC_CFL: i32 = (1 << UV_DC_PRED) | (1 << UV_CFL_PRED);
pub(crate) const UV_INTRA_DC_TM: i32 = (1 << UV_DC_PRED) | (1 << UV_PAETH_PRED);
pub(crate) const UV_INTRA_DC_PAETH_CFL: i32 =
    (1 << UV_DC_PRED) | (1 << UV_PAETH_PRED) | (1 << UV_CFL_PRED);
pub(crate) const UV_INTRA_DC_H_V: i32 = (1 << UV_DC_PRED) | (1 << UV_V_PRED) | (1 << UV_H_PRED);
pub(crate) const UV_INTRA_DC_H_V_CFL: i32 =
    (1 << UV_DC_PRED) | (1 << UV_V_PRED) | (1 << UV_H_PRED) | (1 << UV_CFL_PRED);
pub(crate) const UV_INTRA_DC_PAETH_H_V: i32 =
    (1 << UV_DC_PRED) | (1 << UV_PAETH_PRED) | (1 << UV_V_PRED) | (1 << UV_H_PRED);
pub(crate) const UV_INTRA_DC_PAETH_H_V_CFL: i32 = (1 << UV_DC_PRED)
    | (1 << UV_PAETH_PRED)
    | (1 << UV_V_PRED)
    | (1 << UV_H_PRED)
    | (1 << UV_CFL_PRED);
pub(crate) const INTRA_DC: i32 = 1 << DC_PRED;
pub(crate) const INTRA_DC_TM: i32 = (1 << DC_PRED) | (1 << PAETH_PRED);
pub(crate) const INTRA_DC_H_V: i32 = (1 << DC_PRED) | (1 << V_PRED) | (1 << H_PRED);
pub(crate) const INTRA_DC_H_V_SMOOTH: i32 =
    (1 << DC_PRED) | (1 << V_PRED) | (1 << H_PRED) | (1 << SMOOTH_PRED);
pub(crate) const INTRA_DC_PAETH_H_V: i32 =
    (1 << DC_PRED) | (1 << PAETH_PRED) | (1 << V_PRED) | (1 << H_PRED);

// (DEV_SPEED_FEATURES)
pub(crate) const TXFM_CODING_SF: i32 = 1;
pub(crate) const INTER_PRED_SF: i32 = 2;
pub(crate) const INTRA_PRED_SF: i32 = 4;
pub(crate) const PARTITION_SF: i32 = 8;
pub(crate) const LOOP_FILTER_SF: i32 = 16;
pub(crate) const RD_SKIP_SF: i32 = 32;
pub(crate) const RESERVE_2_SF: i32 = 64;
pub(crate) const RESERVE_3_SF: i32 = 128;

// (RECODE_LOOP_TYPE)
pub(crate) const DISALLOW_RECODE: i32 = 0;
pub(crate) const ALLOW_RECODE_KFARFGF: i32 = 1;
pub(crate) const ALLOW_RECODE: i32 = 2;

// (SUBPEL_SEARCH_METHODS)
pub(crate) const SUBPEL_TREE: i32 = 0;
pub(crate) const SUBPEL_TREE_PRUNED: i32 = 1; // Prunes 1/2-pel searches
pub(crate) const SUBPEL_TREE_PRUNED_MORE: i32 = 2; // Prunes 1/2-pel searches more aggressively

// (LPF_PICK_METHOD)
// Try the full image with different values.
pub(crate) const LPF_PICK_FROM_FULL_IMAGE: i32 = 0;
// Try the full image filter search with non-dual filter only.
pub(crate) const LPF_PICK_FROM_FULL_IMAGE_NON_DUAL: i32 = 1;
// Try a small portion of the image with different values.
pub(crate) const LPF_PICK_FROM_SUBIMAGE: i32 = 2;
// Estimate the level based on quantizer and frame type
pub(crate) const LPF_PICK_FROM_Q: i32 = 3;
// Pick 0 to disable LPF if LPF was enabled last frame
pub(crate) const LPF_PICK_MINIMAL_LPF: i32 = 4;

// (CDEF_PICK_METHOD)
pub(crate) const CDEF_FULL_SEARCH: i32 = 0;
// Search among a subset of all possible filters.
pub(crate) const CDEF_FAST_SEARCH_LVL1: i32 = 1;
// Search reduced subset of filters than Level 1.
pub(crate) const CDEF_FAST_SEARCH_LVL2: i32 = 2;
// Search reduced subset of secondary filters than Level 2.
pub(crate) const CDEF_FAST_SEARCH_LVL3: i32 = 3;
// Search reduced subset of filters than Level 3.
pub(crate) const CDEF_FAST_SEARCH_LVL4: i32 = 4;
// Search reduced subset of filters than Level 4.
pub(crate) const CDEF_FAST_SEARCH_LVL5: i32 = 5;
// Estimate filter strength based on quantizer.
pub(crate) const CDEF_PICK_FROM_Q: i32 = 6;
pub(crate) const CDEF_PICK_METHODS: i32 = 7;

// (MODE_SEARCH_SKIP_LOGIC)
// Terminate search early based on distortion so far compared to
// qp step, distortion in the neighborhood of the frame, etc.
pub(crate) const FLAG_EARLY_TERMINATE: u32 = 1 << 0;

// Skips comp inter modes if the best so far is an intra mode.
pub(crate) const FLAG_SKIP_COMP_BESTINTRA: u32 = 1 << 1;

// Skips oblique intra modes if the best so far is an inter mode.
pub(crate) const FLAG_SKIP_INTRA_BESTINTER: u32 = 1 << 3;

// Skips oblique intra modes  at angles 27, 63, 117, 153 if the best
// intra so far is not one of the neighboring directions.
pub(crate) const FLAG_SKIP_INTRA_DIRMISMATCH: u32 = 1 << 4;

// Skips intra modes other than DC_PRED if the source variance is small
pub(crate) const FLAG_SKIP_INTRA_LOWVAR: u32 = 1 << 5;

// (TX_TYPE_PRUNE_MODE)
pub(crate) const TX_TYPE_PRUNE_0: i32 = 0;
pub(crate) const TX_TYPE_PRUNE_1: i32 = 1;
pub(crate) const TX_TYPE_PRUNE_2: i32 = 2;
pub(crate) const TX_TYPE_PRUNE_3: i32 = 3;
// adaptive prune rate depending on frame rate
pub(crate) const TX_TYPE_PRUNE_4: i32 = 4;
// Use ML model in addition to adaptive pruning
pub(crate) const TX_TYPE_PRUNE_5: i32 = 5;

// (OVERSHOOT_DETECTION_CBR)
pub(crate) const NO_DETECTION: i32 = 0;
// Detection based on maxq
pub(crate) const FAST_DETECTION_MAXQ: i32 = 1;

// (MULTI_WINNER_MODE_TYPE)
// Turns off multi-winner mode. So we will do txfm search on either all modes
// if winner mode is off, or we will only on txfm search on a single winner
// mode.
pub(crate) const MULTI_WINNER_MODE_OFF: i32 = 0;

// Limits the number of winner modes to at most 2
pub(crate) const MULTI_WINNER_MODE_FAST: i32 = 1;

// Uses the default number of winner modes, which is 3 for intra mode, and 1
// for inter mode.
pub(crate) const MULTI_WINNER_MODE_DEFAULT: i32 = 2;

// Maximum number of winner modes allowed.
pub(crate) const MULTI_WINNER_MODE_LEVELS: i32 = 3;

// (PRUNE_NEARMV_LEVEL)
pub(crate) const PRUNE_NEARMV_OFF: i32 = 0; // Turn off nearmv pruning
pub(crate) const PRUNE_NEARMV_LEVEL1: i32 = 1; // Prune nearmv for qindex (0-85)
pub(crate) const PRUNE_NEARMV_LEVEL2: i32 = 2; // Prune nearmv for qindex (0-170)
pub(crate) const PRUNE_NEARMV_LEVEL3: i32 = 3; // Prune nearmv more aggressively for qindex (0-170)
pub(crate) const PRUNE_NEARMV_MAX: i32 = PRUNE_NEARMV_LEVEL3;

// (PARTITION_SEARCH_TYPE)
// Search partitions using RD criterion
pub(crate) const SEARCH_PARTITION: i32 = 0;

// Always use a fixed size partition
pub(crate) const FIXED_PARTITION: i32 = 1;

// Partition using source variance
pub(crate) const VAR_BASED_PARTITION: i32 = 2;

// (CONFIG_RT_ML_PARTITIONING)
pub(crate) const ML_BASED_PARTITION: i32 = 3;

// (MAX_PART_PRED_MODE)
pub(crate) const NOT_IN_USE: i32 = 0;
pub(crate) const DIRECT_PRED: i32 = 1;
pub(crate) const RELAXED_PRED: i32 = 2;
pub(crate) const ADAPT_PRED: i32 = 3;

// (MV_PREC_LOGIC)
pub(crate) const LAST_MV_DATA: i32 = 0;
pub(crate) const CURRENT_Q: i32 = 1;
pub(crate) const QTR_ONLY: i32 = 2;

// (SUPERRES_AUTO_SEARCH_TYPE)
pub(crate) const SUPERRES_AUTO_ALL: i32 = 0; // Tries all possible superres ratios
pub(crate) const SUPERRES_AUTO_DUAL: i32 = 1; // Tries no superres and q-based superres ratios
pub(crate) const SUPERRES_AUTO_SOLO: i32 = 2; // Only apply the q-based superres ratio

// (INTERNAL_COST_UPDATE_TYPE)
pub(crate) const INTERNAL_COST_UPD_OFF: i32 = 0; // Turn off cost updates.
pub(crate) const INTERNAL_COST_UPD_TILE: i32 = 1; // Update every tile.
pub(crate) const INTERNAL_COST_UPD_SBROW_SET: i32 = 2; // Update every row_set of height 256 pixs.
pub(crate) const INTERNAL_COST_UPD_SBROW: i32 = 3; // Update every sb rows inside a tile.
pub(crate) const INTERNAL_COST_UPD_SB: i32 = 4; // Update every sb.

// (SIMPLE_MOTION_SEARCH_PRUNE_LEVEL)
pub(crate) const NO_PRUNING: i32 = -1;
pub(crate) const SIMPLE_AGG_LVL0: i32 = 0; // Simple prune aggressiveness level 0
pub(crate) const SIMPLE_AGG_LVL1: i32 = 1; // Simple prune aggressiveness level 1
pub(crate) const SIMPLE_AGG_LVL2: i32 = 2; // Simple prune aggressiveness level 2
pub(crate) const SIMPLE_AGG_LVL3: i32 = 3; // Simple prune aggressiveness level 3
pub(crate) const QIDX_BASED_AGG_LVL1: i32 = 4; // Qindex based prune aggressiveness level 1
pub(crate) const TOTAL_SIMPLE_AGG_LVLS: i32 = QIDX_BASED_AGG_LVL1;
pub(crate) const TOTAL_QINDEX_BASED_AGG_LVLS: i32 = QIDX_BASED_AGG_LVL1 - SIMPLE_AGG_LVL3;
pub(crate) const TOTAL_AGG_LVLS: i32 = TOTAL_SIMPLE_AGG_LVLS + TOTAL_QINDEX_BASED_AGG_LVLS;

// (PRUNE_MESH_SEARCH_LEVEL)
pub(crate) const PRUNE_MESH_SEARCH_DISABLED: i32 = 0; // Prune mesh search level 0
pub(crate) const PRUNE_MESH_SEARCH_LVL_1: i32 = 1; // Prune mesh search level 1
pub(crate) const PRUNE_MESH_SEARCH_LVL_2: i32 = 2; // Prune mesh search level 2

// (INTER_SEARCH_EARLY_TERM_IDX)
pub(crate) const EARLY_TERM_DISABLED: i32 = 0; // Early terminate inter mode search based on sse disabled.
pub(crate) const EARLY_TERM_IDX_1: i32 = 1; // Early terminate inter mode search based on sse, index 1.
pub(crate) const EARLY_TERM_IDX_2: i32 = 2; // Early terminate inter mode search based on sse, index 2.
pub(crate) const EARLY_TERM_IDX_3: i32 = 3; // Early terminate inter mode search based on sse, index 3.
pub(crate) const EARLY_TERM_IDX_4: i32 = 4; // Early terminate inter mode search based on sse, index 4.
pub(crate) const EARLY_TERM_INDICES: i32 = 5; // Total number of early terminate indices

pub(crate) const MAX_MESH_SPEED: i32 = 5; // Max speed setting for mesh motion method
                                          // Max speed setting for tx domain evaluation
pub(crate) const MAX_TX_DOMAIN_EVAL_SPEED: i32 = 5;
static GOOD_QUALITY_MESH_PATTERNS: [[MeshPattern; MAX_MESH_STEP as usize];
    MAX_MESH_SPEED as usize + 1] = {
    const fn m(range: i32, interval: i32) -> MeshPattern {
        MeshPattern { range, interval }
    }
    [
        [m(64, 8), m(28, 4), m(15, 1), m(7, 1)],
        [m(64, 8), m(28, 4), m(15, 1), m(7, 1)],
        [m(64, 8), m(14, 2), m(7, 1), m(7, 1)],
        [m(64, 16), m(24, 8), m(12, 4), m(7, 1)],
        [m(64, 16), m(24, 8), m(12, 4), m(7, 1)],
        [m(64, 16), m(24, 8), m(12, 4), m(7, 1)],
    ]
};

// TODO(huisu@google.com): These settings are pretty relaxed, tune them for
// each speed setting
static INTRABC_MESH_PATTERNS: [[MeshPattern; MAX_MESH_STEP as usize]; MAX_MESH_SPEED as usize + 1] = {
    const fn m(range: i32, interval: i32) -> MeshPattern {
        MeshPattern { range, interval }
    }
    [
        [m(256, 1), m(256, 1), m(0, 0), m(0, 0)],
        [m(256, 1), m(256, 1), m(0, 0), m(0, 0)],
        [m(64, 1), m(64, 1), m(0, 0), m(0, 0)],
        [m(64, 1), m(64, 1), m(0, 0), m(0, 0)],
        [m(64, 4), m(16, 1), m(0, 0), m(0, 0)],
        [m(64, 4), m(16, 1), m(0, 0), m(0, 0)],
    ]
};

// Threshold values to be used for pruning the txfm_domain_distortion
// based on block MSE
// Index 0: Default mode evaluation, Winner mode processing is not
// applicable (Eg : IntraBc). Index 1: Mode evaluation.
// Index 2: Winner mode evaluation. Index 1 and 2 are applicable when
// enable_winner_mode_for_use_tx_domain_dist speed feature is ON
// TODO(any): Experiment the threshold logic based on variance metric
static TX_DOMAIN_DIST_THRESHOLDS: [[u32; MODE_EVAL_TYPES]; 4] = [
    [u32::MAX, u32::MAX, u32::MAX],
    [22026, 22026, 22026],
    [1377, 1377, 1377],
    [0, 0, 0],
];

// Number of different levels of aggressiveness in using transform domain
// distortion during the R-D evaluation based on the speed feature
// tx_domain_dist_level.
pub(crate) const TX_DOMAIN_DIST_LEVELS: usize = 4;

// Transform domain distortion type to be used for default, mode and winner mode
// evaluation Index 0: Default mode evaluation, Winner mode processing is not
// applicable (Eg : IntraBc). Index 1: Mode evaluation. Index 2: Winner mode
// evaluation. Index 1 and 2 are applicable when
// enable_winner_mode_for_use_tx_domain_dist speed feature is ON
static TX_DOMAIN_DIST_TYPES: [[u32; MODE_EVAL_TYPES]; TX_DOMAIN_DIST_LEVELS] =
    [[0, 2, 0], [1, 2, 0], [2, 2, 0], [2, 2, 2]];

// Threshold values to be used for disabling coeff RD-optimization
// based on block MSE / qstep^2.
// TODO(any): Experiment the threshold logic based on variance metric.
// Table has satd and dist threshold value index 0 : dist,index 1: satd
// For each row, the indices are as follows.
// Index 0: Default mode evaluation, Winner mode processing is not applicable
// (Eg : IntraBc)
// Index 1: Mode evaluation.
// Index 2: Winner mode evaluation.
// Index 1 and 2 are applicable when enable_winner_mode_for_coeff_opt speed
// feature is ON
// There are 7 levels with increasing speed, mapping to vertical indices.
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

// Transform size to be used for default, mode and winner mode evaluation
// Index 0: Default mode evaluation, Winner mode processing is not applicable
// (Eg : IntraBc) Index 1: Mode evaluation. Index 2: Winner mode evaluation.
// Index 1 and 2 are applicable when enable_winner_mode_for_tx_size_srch speed
// feature is ON
static TX_SIZE_SEARCH_METHODS: [[i32; MODE_EVAL_TYPES]; 4] = [
    [USE_FULL_RD, USE_LARGESTALL, USE_FULL_RD],
    [USE_FAST_RD, USE_LARGESTALL, USE_FULL_RD],
    [USE_LARGESTALL, USE_LARGESTALL, USE_FULL_RD],
    [USE_LARGESTALL, USE_LARGESTALL, USE_LARGESTALL],
];

// Predict transform skip levels to be used for default, mode and winner mode
// evaluation. Index 0: Default mode evaluation, Winner mode processing is not
// applicable. Index 1: Mode evaluation, Index 2: Winner mode evaluation
// Values indicate the aggressiveness of skip flag prediction.
// 0 : no early skip prediction
// 1 : conservative early skip prediction using DCT_DCT
// 2 : early skip prediction based on SSE
static PREDICT_SKIP_LEVELS: [[u32; MODE_EVAL_TYPES]; 3] = [[0, 0, 0], [1, 1, 1], [1, 2, 1]];

// Predict skip or DC block level used during transform type search. It is
// indexed using the following:
// First index  : Speed feature 'dc_blk_pred_level' (0 to 3)
// Second index : Mode evaluation type (DEFAULT_EVAL, MODE_EVAL and
// WINNER_MODE_EVAL).
//
// The values of predict_dc_levels[][] indicate the aggressiveness of predicting
// a block as transform skip or DC only.
// Type 0 : No skip block or DC only block prediction
// Type 1 : Prediction of skip block based on residual mean and variance
// Type 2 : Prediction of skip block or DC only block based on residual mean and
// variance
static PREDICT_DC_LEVELS: [[u32; MODE_EVAL_TYPES]; 4] =
    [[0, 0, 0], [1, 1, 0], [2, 2, 0], [2, 2, 2]];

// This table holds the maximum number of reference frames for global motion.
// The table is indexed as per the speed feature 'gm_search_type'.
// 0 : All reference frames are allowed.
// 1 : All reference frames except L2 and L3 are allowed.
// 2 : All reference frames except L2, L3 and ARF2 are allowed.
// 3 : No reference frame is allowed.
static GM_AVAILABLE_REFERENCE_FRAMES: [i32; GM_DISABLE_SEARCH as usize + 1] = [
    INTER_REFS_PER_FRAME,
    INTER_REFS_PER_FRAME - 2,
    INTER_REFS_PER_FRAME - 3,
    0,
];

// Qindex threshold levels used for selecting full-pel motion search.
// ms_qthresh[i][j][k] indicates the qindex boundary value for 'k'th qindex band
// for resolution index 'j' for aggressiveness level 'i'.
// Aggressiveness increases from i = 0 to 2.
// j = 0: lower than 720p resolution, j = 1: 720p or larger resolution.
// Currently invoked only for speed 0, 1 and 2.
static MS_QINDEX_THRESH: [[[i32; 2]; 2]; 3] = [
    [[200, 70], [MAXQ, 200]],
    [[170, 50], [MAXQ, 200]],
    [[170, 40], [200, 40]],
];

// Full-pel search methods for aggressive search based on qindex.
// Index 0 is for resolutions lower than 720p, index 1 for 720p or larger
// resolutions. Currently invoked only for speed 1 and 2.
static MOTION_SEARCH_METHOD: [i32; 2] = [CLAMPED_DIAMOND, DIAMOND];

/// `TX_TYPE_SEARCH`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TxTypeSearch {
    // (TX_TYPE_PRUNE_MODE)
    pub(crate) prune_2d_txfm_mode: i32,
    pub(crate) fast_intra_tx_type_search: i32,
    // INT_MAX: Disable fast search.
    // 1 - 1024: Probability threshold used for conditionally forcing tx type,
    // during mode search.
    // 0: Force tx type to be DCT_DCT unconditionally, during
    // mode search.
    pub(crate) fast_inter_tx_type_prob_thresh: i32,
    // Prune less likely chosen transforms for each intra mode. The speed
    // feature ranges from 0 to 2, for different speed / compression trade offs.
    pub(crate) use_reduced_intra_txset: i32,
    // Use a skip flag prediction model to detect blocks with skip = 1 early
    // and avoid doing full TX type search for such blocks.
    pub(crate) use_skip_flag_prediction: i32,
    // Threshold used by the ML based method to predict TX block split decisions.
    pub(crate) ml_tx_split_thresh: i32,
    // skip remaining transform type search when we found the rdcost of skip is
    // better than applying transform
    pub(crate) skip_tx_search: i32,
    // Prune tx type search using previous frame stats.
    pub(crate) prune_tx_type_using_stats: i32,
    // Prune tx type search using estimated RDcost
    pub(crate) prune_tx_type_est_rd: i32,
    // Flag used to control the winner mode processing for tx type pruning for
    // inter blocks. It enables further tx type mode pruning based on ML model for
    // mode evaluation and disables tx type mode pruning for winner mode
    // processing.
    pub(crate) winner_mode_tx_type_pruning: i32,
}

/// `HIGH_LEVEL_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct HighLevelSpeedFeatures {
    // Frame level coding parameter update.
    pub(crate) frame_parameter_update: i32,
    // Cases and frame types for which the recode loop is enabled.
    // /
    // (RECODE_LOOP_TYPE)
    pub(crate) recode_loop: i32,
    // Controls the tolerance vs target rate used in deciding whether to
    // recode a frame. It has no meaning if recode is disabled.
    // /
    pub(crate) recode_tolerance: i32,
    // Determine how motion vector precision is chosen. The possibilities are:
    // LAST_MV_DATA: use the mv data from the last coded frame
    // CURRENT_Q: use the current q as a threshold
    // QTR_ONLY: use quarter pel precision only.
    // /
    // (MV_PREC_LOGIC)
    pub(crate) high_precision_mv_usage: i32,
    // Always set to 0. If on it enables 0 cost background transmission
    // (except for the initial transmission of the segmentation). The feature is
    // disabled because the addition of very large block sizes make the
    // backgrounds very to cheap to encode, and the segmentation we have
    // adds overhead.
    // /
    pub(crate) static_segmentation: i32,
    // Superres-auto mode search type:
    // /
    // (SUPERRES_AUTO_SEARCH_TYPE)
    pub(crate) superres_auto_search_type: i32,
    // Enable/disable extra screen content test by encoding key frame twice.
    // /
    pub(crate) disable_extra_sc_testing: i32,
    // Enable/disable second_alt_ref temporal filtering.
    // /
    pub(crate) second_alt_ref_filtering: i32,
    // Number of frames to be used in temporal filtering controlled based on noise
    // levels and arf-q.
    // /
    pub(crate) num_frames_used_in_tf: i32,
    // Decide the bit estimation approach used in qindex decision.
    // 0: estimate bits based on a constant value;
    // 1: estimate bits more accurately based on the frame complexity.
    // /
    pub(crate) accurate_bit_estimate: i32,
}

/// `FIRST_PASS_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FirstPassSpeedFeatures {
    // \brief Reduces the mv search window.
    // By default, the initial search window is around
    // MIN(MIN(dims), MAX_FULL_PEL_VAL) = MIN(MIN(dims), 1023).
    // Each step reduction decrease the window size by about a factor of 2.
    // /
    pub(crate) reduce_mv_step_param: i32,
    // \brief Skips the motion search when the zero mv has small sse.
    // /
    pub(crate) skip_motion_search_threshold: i32,
    // \brief Skips reconstruction by using source buffers for prediction
    // /
    pub(crate) disable_recon: i32,
    // \brief Skips the motion search centered on 0,0 mv.
    // /
    pub(crate) skip_zeromv_motion_search: i32,
}

/// `TPL_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TplSpeedFeatures {
    // GOP length adaptive decision.
    // If set to 0, tpl model decides whether a shorter gf interval is better.
    // If set to 1, tpl stats of ARFs from base layer, (base+1) layer and
    // (base+2) layer decide whether a shorter gf interval is better.
    // If set to 2, tpl stats of ARFs from base layer, (base+1) layer and GF boost
    // decide whether a shorter gf interval is better.
    // If set to 3, gop length adaptive decision is disabled.
    pub(crate) gop_length_decision_method: i32,
    // Prune the intra modes search by tpl.
    // If set to 0, we will search all intra modes from DC_PRED to PAETH_PRED.
    // If set to 1, we only search DC_PRED, V_PRED, and H_PRED.
    pub(crate) prune_intra_modes: i32,
    // This parameter controls which step in the n-step process we start at.
    pub(crate) reduce_first_step_size: i32,
    // Skip motion estimation based on the precision of center MVs and the
    // difference between center MVs.
    // If set to 0, motion estimation is skipped for duplicate center MVs
    // (default). If set to 1, motion estimation is skipped for duplicate
    // full-pixel center MVs. If set to 2, motion estimation is skipped if the
    // difference between center MVs is less than the threshold.
    pub(crate) skip_alike_starting_mv: i32,
    // When to stop subpel search.
    // (SUBPEL_FORCE_STOP)
    pub(crate) subpel_force_stop: i32,
    // Which search method to use.
    // (SEARCH_METHODS)
    pub(crate) search_method: i32,
    // Prune starting mvs in TPL based on sad scores.
    pub(crate) prune_starting_mv: i32,
    // Not run TPL for filtered Key frame.
    pub(crate) disable_filtered_key_tpl: i32,
    // Prune reference frames in TPL.
    pub(crate) prune_ref_frames_in_tpl: i32,
    // Support compound predictions.
    pub(crate) allow_compound_pred: i32,
    // Calculate rate and distortion based on Y plane only.
    pub(crate) use_y_only_rate_distortion: i32,
}

/// `GLOBAL_MOTION_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GlobalMotionSpeedFeatures {
    // (GM_SEARCH_TYPE)
    pub(crate) gm_search_type: i32,
    // During global motion estimation, prune remaining reference frames in a
    // given direction(past/future), if the evaluated ref_frame in that direction
    // yields gm_type as INVALID/TRANSLATION/IDENTITY
    pub(crate) prune_ref_frame_for_gm_search: i32,
    // When the current GM type is set to ZEROMV, prune ZEROMV if its performance
    // is worse than NEWMV under SSE metric.
    // 0 : no pruning
    // 1 : conservative pruning
    // 2 : aggressive pruning
    pub(crate) prune_zero_mv_with_sse: i32,
    // Disable global motion estimation based on stats of previous frames in the
    // GF group
    pub(crate) disable_gm_search_based_on_stats: i32,
}

/// `PARTITION_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PartitionSpeedFeatures {
    // (PARTITION_SEARCH_TYPE)
    pub(crate) partition_search_type: i32,
    // Used if partition_search_type = FIXED_PARTITION
    pub(crate) fixed_partition_size: BlockSize,
    // Prune extended partition types search
    // Can take values 0 - 2, 0 referring to no pruning, and 1 - 2 increasing
    // aggressiveness of pruning in order.
    pub(crate) prune_ext_partition_types_search_level: i32,
    // Prune part4 based on block size
    pub(crate) prune_part4_search: i32,
    // Use a ML model to prune rectangular, ab and 4-way horz
    // and vert partitions
    pub(crate) ml_prune_partition: i32,
    // Use a ML model to adaptively terminate partition search after trying
    // PARTITION_SPLIT. Can take values 0 - 2, 0 meaning not being enabled, and
    // 1 - 2 increasing aggressiveness in order.
    pub(crate) ml_early_term_after_part_split_level: i32,
    // Skip rectangular partition test when partition type none gives better
    // rd than partition type split. Can take values 0 - 2, 0 referring to no
    // skipping, and 1 - 2 increasing aggressiveness of skipping in order.
    pub(crate) less_rectangular_check_level: i32,
    // Use square partition only beyond this block size.
    pub(crate) use_square_partition_only_threshold: BlockSize,
    // Sets max square partition levels for this superblock based on
    // motion vector and prediction error distribution produced from 16x16
    // simple motion search
    // (MAX_PART_PRED_MODE)
    pub(crate) auto_max_partition_based_on_simple_motion: i32,
    // Min and max square partition size we enable (block_size) as per auto
    // min max, but also used by adjust partitioning, and pick_partitioning.
    pub(crate) default_min_partition_size: BlockSize,
    pub(crate) default_max_partition_size: BlockSize,
    // Sets level of adjustment of variance-based partitioning during
    // rd_use_partition 0 - no partition adjustment, 1 - try to merge partitions
    // for small blocks and high QP, 2 - try to merge partitions, 3 - try to merge
    // and split leaf partitions and 0 - 3 decreasing aggressiveness in order.
    pub(crate) adjust_var_based_rd_partitioning: i32,
    // Partition search early breakout thresholds.
    pub(crate) partition_search_breakout_dist_thr: i64,
    pub(crate) partition_search_breakout_rate_thr: i32,
    // Thresholds for ML based partition search breakout.
    pub(crate) ml_partition_search_breakout_thresh: [i32; PARTITION_BLOCK_SIZES as usize],
    // Aggressiveness levels for pruning split and rectangular partitions based on
    // simple_motion_search. SIMPLE_AGG_LVL0 to SIMPLE_AGG_LVL3 correspond to
    // simple motion search based pruning. QIDX_BASED_AGG_LVL1 corresponds to
    // qindex based and simple motion search based pruning.
    pub(crate) simple_motion_search_prune_agg: i32,
    // Perform simple_motion_search on each possible subblock and use it to prune
    // PARTITION_HORZ and PARTITION_VERT.
    pub(crate) simple_motion_search_prune_rect: i32,
    // Perform simple motion search before none_partition to decide if we
    // want to remove all partitions other than PARTITION_SPLIT. If set to 0, this
    // model is disabled. If set to 1, the model attempts to perform
    // PARTITION_SPLIT only. If set to 2, the model also attempts to prune
    // PARTITION_SPLIT.
    pub(crate) simple_motion_search_split: i32,
    // Use features from simple_motion_search to terminate prediction block
    // partition after PARTITION_NONE
    pub(crate) simple_motion_search_early_term_none: i32,
    // Controls whether to reduce the number of motion search steps. If this is 0,
    // then simple_motion_search has the same number of steps as
    // single_motion_search (assuming no other speed features). Otherwise, reduce
    // the number of steps by the value contained in this variable.
    pub(crate) simple_motion_search_reduce_search_steps: i32,
    // This variable controls the maximum block size where intra blocks can be
    // used in inter frames.
    // TODO(aconverse): Fold this into one of the other many mode skips
    pub(crate) max_intra_bsize: BlockSize,
    // Use CNN with luma pixels on source frame on each of the 64x64 subblock to
    // perform partition pruning in intra frames.
    // 0: No Pruning
    // 1: Prune split and rectangular partitions only
    // 2: Prune none, split and rectangular partitions
    pub(crate) intra_cnn_based_part_prune_level: i32,
    // Disable extended partition search for lower block sizes.
    pub(crate) ext_partition_eval_thresh: i32,
    // Disable rectangular partitions for larger block sizes.
    pub(crate) rect_partition_eval_thresh: i32,
    // prune extended partition search
    // 0 : no pruning
    // 1 : prune 1:4 partition search using winner info from split partitions
    // 2 : prune 1:4 and AB partition search using split and HORZ/VERT info
    pub(crate) prune_ext_part_using_split_info: i32,
    // Prunt rectangular, AB and 4-way partition based on q index and block size
    // 0 : no pruning
    // 1 : prune sub_8x8 at very low quantizers
    // 2 : prune all block size based on qindex
    pub(crate) prune_rectangular_split_based_on_qidx: i32,
    // Terminate partition search for child partition,
    // when NONE and SPLIT partition rd_costs are INT64_MAX.
    pub(crate) early_term_after_none_split: i32,
    // Level used to adjust threshold for av1_ml_predict_breakout(). At lower
    // levels, more conservative threshold is used, and value of 0 indicates
    // av1_ml_predict_breakout() is disabled. Value of 3 corresponds to default
    // case with no adjustment to lbd thresholds.
    pub(crate) ml_predict_breakout_level: i32,
    // Prune sub_8x8 (BLOCK_4X4, BLOCK_4X8 and BLOCK_8X4) partitions.
    // 0 : no pruning
    // 1 : pruning based on neighbour block information
    // 2 : prune always
    pub(crate) prune_sub_8x8_partition_level: i32,
    // Prune rectangular split based on simple motion search split/no_split score.
    // 0: disable pruning, 1: enable pruning
    pub(crate) simple_motion_search_rect_split: i32,
    // The current encoder adopts a DFS search for block partitions.
    // Therefore the mode selection and associated rdcost is ready for smaller
    // blocks before the mode selection for some partition types.
    // AB partition could use previous rd information and skip mode search.
    // An example is:
    // current block
    // +---+---+
    // |       |
    // +       +
    // |       |
    // +-------+
    // SPLIT partition has been searched first before trying HORZ_A
    // +---+---+
    // | R | R |
    // +---+---+
    // | R | R |
    // +---+---+
    // HORZ_A
    // +---+---+
    // |   |   |
    // +---+---+
    // |       |
    // +-------+
    // With this speed feature, the top two sub blocks can directly use rdcost
    // searched in split partition, and the mode info is also copied from
    // saved info. Similarly, the bottom rectangular block can also use
    // the available information from previous rectangular search.
    pub(crate) reuse_prev_rd_results_for_part_ab: i32,
    // Reuse the best prediction modes found in PARTITION_SPLIT and PARTITION_RECT
    // when encoding PARTITION_AB.
    pub(crate) reuse_best_prediction_for_part_ab: i32,
    // The current partition search records the best rdcost so far and uses it
    // in mode search and transform search to early skip when some criteria is
    // met. For example, when the current rdcost is larger than the best rdcost,
    // or the model rdcost is larger than the best rdcost times some thresholds.
    // By default, this feature is turned on to speed up the encoder partition
    // search.
    // If disabling it, at speed 0, 30 frames, we could get
    // about -0.25% quality gain (psnr, ssim, vmaf), with about 13% slowdown.
    pub(crate) use_best_rd_for_pruning: i32,
    // Skip evaluation of non-square partitions based on the corresponding NONE
    // partition.
    // 0: no pruning
    // 1: prune extended partitions if NONE is skippable
    // 2: on top of 1, prune rectangular partitions if NONE is inter, not a newmv
    // mode and skippable
    pub(crate) skip_non_sq_part_based_on_none: i32,
    // Disables 8x8 and below partitions for low quantizers.
    pub(crate) disable_8x8_part_based_on_qidx: i32,
}

/// `MV_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MvSpeedFeatures {
    // Motion search method (Diamond, NSTEP, Hex, Big Diamond, Square, etc).
    // (SEARCH_METHODS)
    pub(crate) search_method: i32,
    // Enable the use of faster, less accurate mv search method
    // 0: disable, 1: if bsize >= BLOCK_32X32, 2: based on bsize, SAD and qp
    // TODO(chiyotsai@google.com): Take the clip's resolution and mv activity into
    // account.
    pub(crate) use_bsize_dependent_search_method: i32,
    // If this is set to 1, we limit the motion search range to 2 times the
    // largest motion vector found in the last frame.
    pub(crate) auto_mv_step_size: i32,
    // Subpel_search_method can only be subpel_tree which does a subpixel
    // logarithmic search that keeps stepping at 1/2 pixel units until
    // you stop getting a gain, and then goes on to 1/4 and repeats
    // the same process. Along the way it skips many diagonals.
    // (SUBPEL_SEARCH_METHODS)
    pub(crate) subpel_search_method: i32,
    // Maximum number of steps in logarithmic subpel search before giving up.
    pub(crate) subpel_iters_per_step: i32,
    // When to stop subpel search.
    // (SUBPEL_FORCE_STOP)
    pub(crate) subpel_force_stop: i32,
    // When to stop subpel search in simple motion search.
    // (SUBPEL_FORCE_STOP)
    pub(crate) simple_motion_subpel_force_stop: i32,
    // If true, sub-pixel search uses the exact convolve function used for final
    // encoding and decoding; otherwise, it uses bilinear interpolation.
    // (SUBPEL_SEARCH_TYPE)
    pub(crate) use_accurate_subpel_search: i32,
    // Threshold for allowing exhaustive motion search.
    pub(crate) exhaustive_searches_thresh: i32,
    // Pattern to be used for any exhaustive mesh searches (except intraBC ME).
    pub(crate) mesh_patterns: [MeshPattern; MAX_MESH_STEP as usize],
    // Pattern to be used for exhaustive mesh searches of intraBC ME.
    pub(crate) intrabc_mesh_patterns: [MeshPattern; MAX_MESH_STEP as usize],
    // Reduce single motion search range based on MV result of prior ref_mv_idx.
    pub(crate) reduce_search_range: i32,
    // Prune mesh search.
    // (PRUNE_MESH_SEARCH_LEVEL)
    pub(crate) prune_mesh_search: i32,
    // Use the rd cost around the best FULLPEL_MV to speed up subpel search
    pub(crate) use_fullpel_costlist: i32,
    // Set the full pixel search level of obmc
    // 0: obmc_full_pixel_diamond
    // 1: obmc_refining_search_sad (faster)
    pub(crate) obmc_full_pixel_search_level: i32,
    // Accurate full pixel motion search based on TPL stats.
    pub(crate) full_pixel_search_level: i32,
    // Whether to downsample the rows in sad calculation during motion search.
    // This is only active when there are at least 16 rows.
    pub(crate) use_downsampled_sad: i32,
    // Enable/disable extensive joint motion search.
    pub(crate) disable_extensive_joint_motion_search: i32,
    // Enable second best mv check in joint mv search.
    // 0: allow second MV (use rd cost as the metric)
    // 1: use var as the metric
    // 2: disable second MV
    pub(crate) disable_second_mv: i32,
    // Skips full pixel search based on start mv of prior ref_mv_idx.
    pub(crate) skip_fullpel_search_using_startmv: i32,
}

/// `INTER_MODE_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InterModeSpeedFeatures {
    // 2-pass inter mode model estimation where the preliminary pass skips
    // transform search and uses a model to estimate rd, while the final pass
    // computes the full transform search. Two types of models are supported:
    // 0: not used
    // 1: used with online dynamic rd model
    // 2: used with static rd model
    pub(crate) inter_mode_rd_model_estimation: i32,
    // Bypass transform search based on skip rd
    pub(crate) txfm_rd_gate_level: i32,
    // Limit the inter mode tested in the RD loop
    pub(crate) reduce_inter_modes: i32,
    // This variable is used to cap the maximum number of times we skip testing a
    // mode to be evaluated. A high value means we will be faster.
    pub(crate) adaptive_rd_thresh: i32,
    // Aggressively prune inter modes when best mode is skippable.
    pub(crate) prune_inter_modes_if_skippable: i32,
    // Drop less likely to be picked reference frames in the RD search.
    // Has seven levels for now: 0, 1, 2, 3, 4, 5 and 6 where higher levels prune
    // more aggressively than lower ones. (0 means no pruning).
    pub(crate) selective_ref_frame: i32,
    // Prune reference frames for rectangular partitions.
    // 0 implies no pruning
    // 1 implies prune for extended partition
    // 2 implies prune horiz, vert and extended partition
    pub(crate) prune_ref_frame_for_rect_partitions: i32,
    // Prune inter modes w.r.t past reference frames
    // 0 no pruning
    // 1 prune inter modes w.r.t ALTREF2 and ALTREF reference frames
    // 2 prune inter modes w.r.t BWDREF, ALTREF2 and ALTREF reference frames
    pub(crate) alt_ref_search_fp: i32,
    // Prune compound reference frames
    // 0 no pruning
    // 1 prune compound references which do not satisfy the two conditions:
    // a) The references are at a nearest distance from the current frame in
    // both past and future direction.
    // b) The references have minimum pred_mv_sad in both past and future
    // direction.
    // 2 prune compound references except the one with nearest distance from the
    // current frame in both past and future direction.
    pub(crate) prune_comp_ref_frames: i32,
    // Skip the current ref_mv in NEW_MV mode based on mv, rate cost, etc.
    // This speed feature equaling 0 means no skipping.
    // If the speed feature equals 1 or 2, skip the current ref_mv in NEW_MV mode
    // if we have already encountered ref_mv in the drl such that:
    // 1. The other drl has the same mv during the SIMPLE_TRANSLATION search
    // process as the current mv.
    // 2. The rate needed to encode the current mv is larger than that for the
    // other ref_mv.
    // The speed feature equaling 1 means using subpel mv in the comparison.
    // The speed feature equaling 2 means using fullpel mv in the comparison.
    // If the speed feature >= 3, skip the current ref_mv in NEW_MV mode based on
    // known full_mv bestsme and drl cost.
    pub(crate) skip_newmv_in_drl: i32,
    // This speed feature checks duplicate ref MVs among NEARESTMV, NEARMV,
    // GLOBALMV and skips NEARMV or GLOBALMV (in order) if a duplicate is found
    // TODO(any): Instead of skipping repeated ref mv, use the recalculated
    // rd-cost based on mode rate and skip the mode evaluation
    pub(crate) skip_repeated_ref_mv: i32,
    // Flag used to control the ref_best_rd based gating for chroma
    pub(crate) perform_best_rd_based_gating_for_chroma: i32,
    // Reuse the inter_intra_mode search result from NEARESTMV mode to other
    // single ref modes
    pub(crate) reuse_inter_intra_mode: i32,
    // prune wedge and compound segment approximate rd evaluation based on
    // compound average modeled rd
    pub(crate) prune_comp_type_by_model_rd: i32,
    // prune wedge and compound segment approximate rd evaluation based on
    // compound average rd/ref_best_rd
    pub(crate) prune_comp_type_by_comp_avg: i32,
    // Skip some ref frames in compound motion search by single motion search
    // result. Has three levels for now: 0 referring to no skipping, and 1 - 3
    // increasing aggressiveness of skipping in order.
    // Note: The search order might affect the result. It assumes that the single
    // reference modes are searched before compound modes. It is better to search
    // same single inter mode as a group.
    pub(crate) prune_comp_search_by_single_result: i32,
    // Instead of performing a full MV search, do a simple translation first
    // and only perform a full MV search on the motion vectors that performed
    // well.
    pub(crate) prune_mode_search_simple_translation: i32,
    // Only search compound modes with at least one "good" reference frame.
    // A reference frame is good if, after looking at its performance among
    // the single reference modes, it is one of the two best performers.
    pub(crate) prune_compound_using_single_ref: i32,
    // Skip extended compound mode (NEAREST_NEWMV, NEW_NEARESTMV, NEAR_NEWMV,
    // NEW_NEARMV) using ref frames of above and left neighbor
    // blocks.
    // 0 : no pruning
    // 1 : prune ext compound modes using neighbor blocks (less aggressiveness)
    // 2 : prune ext compound modes using neighbor blocks (high aggressiveness)
    // 3 : prune ext compound modes unconditionally (highest aggressiveness)
    pub(crate) prune_ext_comp_using_neighbors: i32,
    // Skip NEW_NEARMV and NEAR_NEWMV extended compound modes
    pub(crate) skip_ext_comp_nearmv_mode: i32,
    // Skip extended compound mode when ref frame corresponding to NEWMV does not
    // have NEWMV as single mode winner.
    // 0 : no pruning
    // 1 : prune extended compound mode (less aggressiveness)
    // 2 : prune extended compound mode (high aggressiveness)
    pub(crate) prune_comp_using_best_single_mode_ref: i32,
    // Skip NEARESTMV and NEARMV using weight computed in ref mv list population
    // This speed feature sometimes leads to severe visual artifacts for
    // the overlay frame. It makes inter RD mode search skip NEARESTMV
    // and NEARMV, and no valid inter mode is evaluated when the NEWMV mode
    // is also early terminated due to the constraint that it does not handle
    // zero mv difference. In this cases, intra modes will be chosen, leading
    // to bad prediction and flickering artifacts.
    // Turn off this feature for now. Be careful to check visual quality if
    // anyone is going to turn it on.
    pub(crate) prune_nearest_near_mv_using_refmv_weight: i32,
    // Based on previous ref_mv_idx search result, prune the following search.
    pub(crate) prune_ref_mv_idx_search: i32,
    // Disable one sided compound modes.
    pub(crate) disable_onesided_comp: i32,
    // Prune obmc search using previous frame stats.
    // INT_MAX : disable obmc search
    pub(crate) prune_obmc_prob_thresh: i32,
    // Prune warped motion search using previous frame stats.
    pub(crate) prune_warped_prob_thresh: i32,
    // Variance threshold to enable/disable Interintra wedge search
    pub(crate) disable_interintra_wedge_var_thresh: u32,
    // Variance threshold to enable/disable Interinter wedge search
    pub(crate) disable_interinter_wedge_var_thresh: u32,
    // De-couple wedge and mode search during interintra RDO.
    pub(crate) fast_interintra_wedge_search: i32,
    // Whether fast wedge sign estimate is used
    pub(crate) fast_wedge_sign_estimate: i32,
    // Enable/disable ME for interinter wedge search.
    pub(crate) disable_interinter_wedge_newmv_search: i32,
    // Decide when and how to use joint_comp.
    // (DIST_WTD_COMP_FLAG)
    pub(crate) use_dist_wtd_comp_flag: i32,
    // Clip the frequency of updating the mv cost.
    // (INTERNAL_COST_UPDATE_TYPE)
    pub(crate) mv_cost_upd_level: i32,
    // Clip the frequency of updating the coeff cost.
    // (INTERNAL_COST_UPDATE_TYPE)
    pub(crate) coeff_cost_upd_level: i32,
    // Clip the frequency of updating the mode cost.
    // (INTERNAL_COST_UPDATE_TYPE)
    pub(crate) mode_cost_upd_level: i32,
    // Prune inter modes based on tpl stats
    // 0 : no pruning
    // 1 - 3 indicate increasing aggressiveness in order.
    pub(crate) prune_inter_modes_based_on_tpl: i32,
    // Skip NEARMV and NEAR_NEARMV modes using ref frames of above and left
    // neighbor blocks and qindex.
    // (PRUNE_NEARMV_LEVEL)
    pub(crate) prune_nearmv_using_neighbors: i32,
    // Model based breakout after interpolation filter search
    // 0: no breakout
    // 1: use model based rd breakout
    pub(crate) model_based_post_interp_filter_breakout: i32,
    // Reuse compound type rd decision when exact match is found
    // 0: No reuse
    // 1: Reuse the compound type decision
    pub(crate) reuse_compound_type_decision: i32,
    // Enable/disable masked compound.
    pub(crate) disable_masked_comp: i32,
    // Enable/disable the fast compound mode search.
    pub(crate) enable_fast_compound_mode_search: i32,
    // Reuse masked compound type search results
    pub(crate) reuse_mask_search_results: i32,
    // Enable/disable fast search for wedge masks
    pub(crate) enable_fast_wedge_mask_search: i32,
    // Early breakout from transform search of inter modes
    pub(crate) inter_mode_txfm_breakout: i32,
    // Limit number of inter modes for txfm search if a newmv mode gets
    // evaluated among the top modes.
    // 0: no pruning
    // 1 to 3 indicate increasing order of aggressiveness
    pub(crate) limit_inter_mode_cands: i32,
    // Cap the no. of txfm searches for a given prediction mode.
    // 0: no cap, 1: cap beyond first 4 searches, 2: cap beyond first 3 searches.
    pub(crate) limit_txfm_eval_per_mode: i32,
    // Prune warped motion search based on block size.
    pub(crate) extra_prune_warped: i32,
    // Do not search compound modes for ARF.
    // The intuition is that ARF is predicted by frames far away from it,
    // whose temporal correlations with the ARF are likely low.
    // It is therefore likely that compound modes do not work as well for ARF
    // as other inter frames.
    // Speed/quality impact:
    // Speed 1: 12% faster, 0.1% psnr loss.
    // Speed 2: 2%  faster, 0.05% psnr loss.
    // No change for speed 3 and up, because |disable_onesided_comp| is true.
    pub(crate) skip_arf_compound: i32,
}

/// `INTERP_FILTER_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InterpFilterSpeedFeatures {
    // Do limited interpolation filter search for dual filters, since best choice
    // usually includes EIGHTTAP_REGULAR.
    pub(crate) use_fast_interpolation_filter_search: i32,
    // Disable dual filter
    pub(crate) disable_dual_filter: i32,
    // Save results of av1_interpolation_filter_search for a block
    // Check mv and ref_frames before search, if they are very close with previous
    // saved results, filter search can be skipped.
    pub(crate) use_interp_filter: i32,
    // skip sharp_filter evaluation based on regular and smooth filter rd for
    // dual_filter=0 case
    pub(crate) skip_sharp_interp_filter_search: i32,
    // skip interpolation filter search for a block in chessboard pattern
    pub(crate) cb_pred_filter_search: i32,
    // adaptive interp_filter search to allow skip of certain filter types.
    pub(crate) adaptive_interp_filter_search: i32,
}

/// `INTRA_MODE_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IntraModeSpeedFeatures {
    // These bit masks allow you to enable or disable intra modes for each
    // transform size separately.
    pub(crate) intra_y_mode_mask: [i32; TX_SIZES as usize],
    pub(crate) intra_uv_mode_mask: [i32; TX_SIZES as usize],
    // flag to allow skipping intra mode for inter frame prediction
    pub(crate) skip_intra_in_interframe: i32,
    // Prune intra mode candidates based on source block histogram of gradient.
    // Applies to luma plane only.
    // Feasible values are 0..4. The feature is disabled for 0. An increasing
    // value indicates more aggressive pruning threshold.
    pub(crate) intra_pruning_with_hog: i32,
    // Prune intra mode candidates based on source block histogram of gradient.
    // Applies to chroma plane only.
    // Feasible values are 0..4. The feature is disabled for 0. An increasing
    // value indicates more aggressive pruning threshold.
    pub(crate) chroma_intra_pruning_with_hog: i32,
    // Enable/disable smooth intra modes.
    pub(crate) disable_smooth_intra: i32,
    // Prune UV_SMOOTH_PRED mode for chroma based on chroma source variance.
    // false : No pruning
    // true  : Prune UV_SMOOTH_PRED mode based on chroma source variance
    // For allintra encode, this speed feature reduces instruction count
    // by 1.90%, 2.21% and 1.97% for speed 6, 7 and 8 with coding performance
    // change less than 0.04%. For AVIF image encode, this speed feature reduces
    // encode time by 1.56%, 2.14% and 0.90% for speed 6, 7 and 8 on a typical
    // image dataset with coding performance change less than 0.05%.
    pub(crate) prune_smooth_intra_mode_for_chroma: bool,
    // Prune filter intra modes in intra frames.
    // 0 : No pruning
    // 1 : Evaluate applicable filter intra modes based on best intra mode so far
    // 2 : Do not evaluate filter intra modes
    pub(crate) prune_filter_intra_level: i32,
    // prune palette search
    // 0: No pruning
    // 1: Perform coarse search to prune the palette colors. For winner colors,
    // neighbors are also evaluated using a finer search.
    // 2: Perform 2 way palette search from max colors to min colors (and min
    // colors to remaining colors) and terminate the search if current number of
    // palette colors is not the winner.
    pub(crate) prune_palette_search_level: i32,
    // Terminate early in luma palette_size search. Speed feature values indicate
    // increasing level of pruning.
    // 0: No early termination
    // 1: Terminate early for higher luma palette_size, if header rd cost of lower
    // palette_size is more than 2 * best_rd. This level of pruning is more
    // conservative when compared to sf level 2 as the cases which will get pruned
    // with sf level 1 is a subset of the cases which will get pruned with sf
    // level 2.
    // 2: Terminate early for higher luma palette_size, if header rd cost of lower
    // palette_size is more than best_rd.
    // For allintra encode, this sf reduces instruction count by 2.49%, 1.07%,
    // 2.76%, 2.30%, 1.84%, 2.69%, 2.04%, 2.05% and 1.44% for speed 0, 1, 2, 3, 4,
    // 5, 6, 7 and 8 on screen content set with coding performance change less
    // than 0.01% for speed <= 2 and less than 0.03% for speed >= 3. For AVIF
    // image encode, this sf reduces instruction count by 1.94%, 1.13%, 1.29%,
    // 0.93%, 0.89%, 1.03%, 1.07%, 1.20% and 0.18% for speed 0, 1, 2, 3, 4, 5, 6,
    // 7 and 8 on a typical image dataset with coding performance change less than
    // 0.01%.
    pub(crate) prune_luma_palette_size_search_level: i32,
    // Prune chroma intra modes based on luma intra mode winner.
    // 0: No pruning
    // 1: Prune chroma intra modes other than UV_DC_PRED, UV_SMOOTH_PRED,
    // UV_CFL_PRED and the mode that corresponds to luma intra mode winner.
    pub(crate) prune_chroma_modes_using_luma_winner: i32,
    // Clip the frequency of updating the mv cost for intrabc.
    // (INTERNAL_COST_UPDATE_TYPE)
    pub(crate) dv_cost_upd_level: i32,
    // We use DCT_DCT transform followed by computing SATD (Sum of Absolute
    // Transformed Differences) as an estimation of RD score to quickly find the
    // best possible Chroma from Luma (CFL) parameter. Then we do a full RD search
    // near the best possible parameter. The search range is set here.
    // The range of cfl_searh_range should be [1, 33], and the following are the
    // recommended values.
    // 1: Fastest mode.
    // 3: Default mode that provides good speedup without losing compression
    // performance at speed 0.
    // 33: Exhaustive rd search (33 == CFL_MAGS_SIZE). This mode should only
    // be used for debugging purpose.
    pub(crate) cfl_search_range: i32,
    // TOP_INTRA_MODEL_COUNT is 4 that is the number of top model rd to store in
    // intra mode decision. Here, add a speed feature to reduce this number for
    // higher speeds.
    pub(crate) top_intra_model_count_allowed: i32,
    // Adapt top_intra_model_count_allowed locally to prune luma intra modes using
    // neighbor block and quantizer information.
    pub(crate) adapt_top_model_rd_count_using_neighbors: i32,
    // Prune the evaluation of odd delta angles of directional luma intra modes by
    // using the rdcosts of neighbouring delta angles.
    // For allintra encode, this speed feature reduces instruction count
    // by 4.461%, 3.699% and 3.536% for speed 6, 7 and 8 on a typical video
    // dataset with coding performance change less than 0.26%. For AVIF image
    // encode, this speed feature reduces encode time by 2.849%, 2.471%,
    // and 2.051% for speed 6, 7 and 8 on a typical image dataset with coding
    // performance change less than 0.27%.
    pub(crate) prune_luma_odd_delta_angles_in_intra: i32,
    // Terminate early in chroma palette_size search.
    // 0: No early termination
    // 1: Terminate early for higher palette_size, if header rd cost of lower
    // palette_size is more than best_rd.
    // For allintra encode, this sf reduces instruction count by 0.45%,
    // 0.62%, 1.73%, 2.50%, 2.89%, 3.09% and 3.86% for speed 0 to 6 on screen
    // content set with coding performance change less than 0.01%.
    // For AVIF image encode, this sf reduces instruction count by 0.45%, 0.81%,
    // 0.85%, 1.05%, 1.45%, 1.66% and 1.95% for speed 0 to 6 on a typical image
    // dataset with no quality drop.
    pub(crate) early_term_chroma_palette_size_search: i32,
    // Skips the evaluation of filter intra modes in inter frames if rd evaluation
    // of luma intra dc mode results in invalid rd stats.
    pub(crate) skip_filter_intra_in_inter_frames: i32,
}

/// `TX_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TxSpeedFeatures {
    // Init search depth for square and rectangular transform partitions.
    // Values:
    // 0 - search full tree, 1: search 1 level, 2: search the highest level only
    pub(crate) inter_tx_size_search_init_depth_sqr: i32,
    pub(crate) inter_tx_size_search_init_depth_rect: i32,
    pub(crate) intra_tx_size_search_init_depth_sqr: i32,
    pub(crate) intra_tx_size_search_init_depth_rect: i32,
    // If any dimension of a coding block size above 64, always search the
    // largest transform only, since the largest transform block size is 64x64.
    pub(crate) tx_size_search_lgr_block: i32,
    pub(crate) tx_type_search: TxTypeSearch,
    // Skip split transform block partition when the collocated bigger block
    // is selected as all zero coefficients.
    pub(crate) txb_split_cap: i32,
    // Shortcut the transform block partition and type search when the target
    // rdcost is relatively lower.
    // Values are 0 (not used) , or 1 - 2 with progressively increasing
    // aggressiveness
    pub(crate) adaptive_txb_search_level: i32,
    // Prune level for tx_size_type search for inter based on rd model
    // 0: no pruning
    // 1-2: progressively increasing aggressiveness of pruning
    pub(crate) model_based_prune_tx_search_level: i32,
    // Refine TX type after fast TX search.
    pub(crate) refine_fast_tx_search_results: i32,
    // Prune transform split/no_split eval based on residual properties. A value
    // of 0 indicates no pruning, and the aggressiveness of pruning progressively
    // increases from levels 1 to 3.
    pub(crate) prune_tx_size_level: i32,
    // Prune the evaluation of transform depths as decided by the NN model.
    // false: No pruning.
    // true : Avoid the evaluation of specific transform depths using NN model.
    // For allintra encode, this speed feature reduces instruction count
    // by 4.76%, 8.92% and 11.28% for speed 6, 7 and 8 with coding performance
    // change less than 0.32%. For AVIF image encode, this speed feature reduces
    // encode time by 4.65%, 9.16% and 10.45% for speed 6, 7 and 8 on a typical
    // image dataset with coding performance change less than 0.19%.
    pub(crate) prune_intra_tx_depths_using_nn: bool,
}

/// `RD_CALC_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RdCalcSpeedFeatures {
    // Fast approximation of av1_model_rd_from_var_lapndz
    pub(crate) simple_model_rd_from_var: i32,
    // Perform faster distortion computation during the R-D evaluation by trying
    // to approximate the prediction error with transform coefficients (faster but
    // less accurate) rather than computing distortion in the pixel domain (slower
    // but more accurate). The following methods are used for distortion
    // computation:
    // Method 0: Always compute distortion in the pixel domain
    // Method 1: Based on block error, try using transform domain distortion for
    // tx_type search and compute distortion in pixel domain for final RD_STATS
    // Method 2: Based on block error, try to compute distortion in transform
    // domain
    // Methods 1 and 2 may fallback to computing distortion in the pixel domain in
    // case the block error is less than the threshold, which is controlled by the
    // speed feature tx_domain_dist_thres_level.
    // The speed feature tx_domain_dist_level decides which of the above methods
    // needs to be used across different mode evaluation stages as described
    // below:
    // Eval type:    Default      Mode        Winner
    // Level 0  :    Method 0    Method 2    Method 0
    // Level 1  :    Method 1    Method 2    Method 0
    // Level 2  :    Method 2    Method 2    Method 0
    // Level 3  :    Method 2    Method 2    Method 2
    pub(crate) tx_domain_dist_level: i32,
    // Transform domain distortion threshold level
    pub(crate) tx_domain_dist_thres_level: i32,
    // Trellis (dynamic programming) optimization of quantized values
    // (TRELLIS_OPT_TYPE)
    pub(crate) optimize_coefficients: i32,
    // Use hash table to store macroblock RD search results
    // to avoid repeated search on the same residue signal.
    pub(crate) use_mb_rd_hash: i32,
    // Flag used to control the extent of coeff R-D optimization
    pub(crate) perform_coeff_opt: i32,
}

/// `WINNER_MODE_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WinnerModeSpeedFeatures {
    // Flag used to control the winner mode processing for better R-D optimization
    // of quantized coeffs
    pub(crate) enable_winner_mode_for_coeff_opt: i32,
    // Flag used to control the winner mode processing for transform size
    // search method
    pub(crate) enable_winner_mode_for_tx_size_srch: i32,
    // Control transform size search level
    // Eval type: Default       Mode        Winner
    // Level 0  : FULL RD     LARGEST ALL   FULL RD
    // Level 1  : FAST RD     LARGEST ALL   FULL RD
    // Level 2  : LARGEST ALL LARGEST ALL   FULL RD
    // Level 3 :  LARGEST ALL LARGEST ALL   LARGEST ALL
    pub(crate) tx_size_search_level: i32,
    // Flag used to control the winner mode processing for use transform
    // domain distortion
    pub(crate) enable_winner_mode_for_use_tx_domain_dist: i32,
    // Flag used to enable processing of multiple winner modes
    // (MULTI_WINNER_MODE_TYPE)
    pub(crate) multi_winner_mode_type: i32,
    // Motion mode for winner candidates:
    // 0: speed feature OFF
    // 1 / 2 : Use configured number of winner candidates
    pub(crate) motion_mode_for_winner_cand: i32,
    // Controls the prediction of transform skip block or DC only block.
    // Different speed feature values (0 to 3) decide the aggressiveness of
    // prediction (refer to predict_dc_levels[][] in speed_features.c) to be used
    // during different mode evaluation stages.
    pub(crate) dc_blk_pred_level: i32,
    // If on, disables interpolation filter search in handle_inter_mode loop, and
    // performs it during winner mode processing by \ref
    // tx_search_best_inter_candidates.
    pub(crate) winner_mode_ifs: i32,
    // Controls the disabling of winner mode processing. Speed feature levels
    // are ordered in increasing aggressiveness of pruning. The method considered
    // for disabling, depends on the sf level value and it is described as below.
    // 0: Do not disable
    // 1: Disable for blocks with low source variance.
    // 2: Disable for blocks which turn out to be transform skip (skipped based on
    // eob) during MODE_EVAL stage except NEWMV mode.
    // 3: Disable for blocks which turn out to be transform skip during MODE_EVAL
    // stage except NEWMV mode. For high quantizers, prune conservatively based on
    // transform skip (skipped based on eob) except for NEWMV mode.
    // 4: Disable for blocks which turn out to be transform skip during MODE_EVAL
    // stage.
    pub(crate) prune_winner_mode_eval_level: i32,
}

/// `LOOP_FILTER_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LoopFilterSpeedFeatures {
    // This feature controls how the loop filter level is determined.
    // (LPF_PICK_METHOD)
    pub(crate) lpf_pick: i32,
    // Skip some final iterations in the determination of the best loop filter
    // level.
    pub(crate) use_coarse_filter_level_search: i32,
    // Control how the CDEF strength is determined.
    // (CDEF_PICK_METHOD)
    pub(crate) cdef_pick_method: i32,
    // Decoder side speed feature to add penalty for use of dual-sgr filters.
    // Takes values 0 - 10, 0 indicating no penalty and each additional level
    // adding a penalty of 1%
    pub(crate) dual_sgr_penalty_level: i32,
    // prune sgr ep using binary search like mechanism
    pub(crate) enable_sgr_ep_pruning: i32,
    // Disable loop restoration for Chroma plane
    pub(crate) disable_loop_restoration_chroma: i32,
    // Disable loop restoration for luma plane
    pub(crate) disable_loop_restoration_luma: i32,
    // Prune RESTORE_WIENER evaluation based on source variance
    // 0 : no pruning
    // 1 : conservative pruning
    // 2 : aggressive pruning
    pub(crate) prune_wiener_based_on_src_var: i32,
    // Prune self-guided loop restoration based on wiener search results
    // 0 : no pruning
    // 1 : pruning based on rdcost ratio of RESTORE_WIENER and RESTORE_NONE
    // 2 : pruning based on winner restoration type among RESTORE_WIENER and
    // RESTORE_NONE
    pub(crate) prune_sgr_based_on_wiener: i32,
    // Reduce the wiener filter win size for luma
    pub(crate) reduce_wiener_window_size: i32,
    // Disable loop restoration filter
    pub(crate) disable_lr_filter: i32,
    // Whether to downsample the rows in computation of wiener stats.
    pub(crate) use_downsampled_wiener_stats: i32,
}

/// `REAL_TIME_SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RealTimeSpeedFeatures {
    // check intra prediction for non-RD mode.
    pub(crate) check_intra_pred_nonrd: i32,
    // Skip checking intra prediction.
    // 0 - don't skip
    // 1 - skip if TX is skipped and best mode is not NEWMV
    // 2 - skip if TX is skipped
    // Skipping aggressiveness increases from level 1 to 2.
    pub(crate) skip_intra_pred: i32,
    // Perform coarse ME before calculating variance in variance-based partition
    pub(crate) estimate_motion_for_var_based_partition: i32,
    // For nonrd_use_partition: mode of extra check of leaf partition
    // 0 - don't check merge
    // 1 - always check merge
    // 2 - check merge and prune checking final split
    // 3 - check merge and prune checking final split based on bsize and qindex
    pub(crate) nonrd_check_partition_merge_mode: i32,
    // For nonrd_use_partition: check of leaf partition extra split
    pub(crate) nonrd_check_partition_split: i32,
    // Implements various heuristics to skip searching modes
    // The heuristics selected are based on  flags
    // defined in the MODE_SEARCH_SKIP_HEURISTICS enum
    pub(crate) mode_search_skip_flags: u32,
    // For nonrd: Reduces ref frame search.
    // 0 - low level of search prune in non last frames
    // 1 - pruned search in non last frames
    // 2 - more pruned search in non last frames
    pub(crate) nonrd_prune_ref_frame_search: i32,
    // This flag controls the use of non-RD mode decision.
    pub(crate) use_nonrd_pick_mode: i32,
    // Use ALTREF frame in non-RD mode decision.
    pub(crate) use_nonrd_altref_frame: i32,
    // Use compound reference for non-RD mode.
    pub(crate) use_comp_ref_nonrd: i32,
    // Reference frames for compound prediction for nonrd pickmode:
    // LAST_GOLDEN (0), LAST_LAST2 (1), or LAST_ALTREF (2).
    pub(crate) ref_frame_comp_nonrd: [i32; 3],
    // use reduced ref set for real-time mode
    pub(crate) use_real_time_ref_set: i32,
    // Skip a number of expensive mode evaluations for blocks with very low
    // temporal variance.
    pub(crate) short_circuit_low_temp_var: i32,
    // Reuse inter prediction in fast non-rd mode.
    pub(crate) reuse_inter_pred_nonrd: i32,
    // Number of best inter modes to search transform. INT_MAX - search all.
    pub(crate) num_inter_modes_for_tx_search: i32,
    // Use interpolation filter search in non-RD mode decision.
    pub(crate) use_nonrd_filter_search: i32,
    // Use simplified RD model for interpolation search and Intra
    pub(crate) use_simple_rd_model: i32,
    // If set forces interpolation filter to EIGHTTAP_REGULAR
    pub(crate) skip_interp_filter_search: i32,
    // For nonrd mode: use hybrid intra mode search for intra only frames based on
    // block properties.
    // 0 : use nonrd pick intra for all blocks
    // 1 : use rd for bsize < 16x16, nonrd otherwise
    // 2 : use rd for bsize < 16x16 and src var >= 101, nonrd otherwise
    pub(crate) hybrid_intra_pickmode: i32,
    // Compute variance/sse on source difference, prior to encoding superblock.
    pub(crate) source_metrics_sb_nonrd: i32,
    // Flag to indicate process for handling overshoot on slide/scene change,
    // for real-time CBR mode.
    // (OVERSHOOT_DETECTION_CBR)
    pub(crate) overshoot_detection_cbr: i32,
    // Check for scene/content change detection on every frame before encoding.
    pub(crate) check_scene_detection: i32,
    // For nonrd mode: Prefer larger partition blks in variance based partitioning
    // 0: disabled, 1-3: increasing aggressiveness
    pub(crate) prefer_large_partition_blocks: i32,
    // uses results of temporal noise estimate
    pub(crate) use_temporal_noise_estimate: i32,
    // Parameter indicating initial search window to be used in full-pixel search
    // for nonrd_pickmode. Range [0, MAX_MVSEARCH_STEPS - 1]. Lower value
    // indicates larger window. If set to 0, step_param is set based on internal
    // logic in set_mv_search_params().
    pub(crate) fullpel_search_step_param: i32,
    // Bit mask to enable or disable intra modes for each prediction block size
    // separately, for nonrd_pickmode.  Currently, the sf is not respected when
    // 'force_intra_check' is true in 'estimate_intra_mode()' function. Also, H
    // and V pred modes allowed through this sf can be further pruned when
    // 'prune_hv_pred_modes_using_src_sad' sf is true.
    pub(crate) intra_y_mode_bsize_mask_nrd: [i32; BLOCK_SIZES as usize],
    // Prune H and V intra predition modes evalution in inter frame.
    // The sf does not have any impact.
    // i. when frame_source_sad is 1.1 times greater than avg_source_sad
    // ii. when cyclic_refresh_segment_id_boosted is enabled
    // iii. when SB level source sad is greater than kMedSad
    // iv. when color sensitivity is non zero for both the chroma channels
    pub(crate) prune_hv_pred_modes_using_src_sad: bool,
    // Skips mode checks more aggressively in nonRD mode
    pub(crate) nonrd_aggressive_skip: i32,
    // Skip cdef on 64x64 blocks when NEWMV or INTRA is not picked or color
    // sensitivity is off. When color sensitivity is on for a superblock, all
    // 64x64 blocks within will not skip.
    pub(crate) skip_cdef_sb: i32,
    // Forces larger partition blocks in variance based partitioning for intra
    // frames
    pub(crate) force_large_partition_blocks_intra: i32,
    // Skip evaluation of no split in tx size selection for merge partition
    pub(crate) skip_tx_no_split_var_based_partition: i32,
    // Intermediate termination of newMV mode evaluation based on so far best mode
    // sse
    pub(crate) skip_newmv_mode_based_on_sse: i32,
    // Define gf length multiplier.
    // Level 0: use large multiplier, level 1: use medium multiplier.
    pub(crate) gf_length_lvl: i32,
    // Prune inter modes with golden frame as reference for NEARMV and NEWMV modes
    pub(crate) prune_inter_modes_with_golden_ref: i32,
    // Prune inter modes w.r.t golden or alt-ref frame based on sad
    pub(crate) prune_inter_modes_wrt_gf_arf_based_on_sad: i32,
    // Prune inter mode search in rd path based on current block's temporal
    // variance wrt LAST reference.
    pub(crate) prune_inter_modes_using_temp_var: i32,
    // Reduce MV precision to halfpel for higher int MV value & frame-level motion
    // 0: disabled
    // 1-2: Reduce precision to halfpel, fullpel based on conservative
    // thresholds, aggressiveness increases with increase in level
    // 3: Reduce precision to halfpel using more aggressive thresholds
    pub(crate) reduce_mv_pel_precision_highmotion: i32,
    // Reduce MV precision for low complexity blocks
    // 0: disabled
    // 1: Reduce the mv resolution for zero mv if the variance is low
    // 2: Switch to halfpel, fullpel based on low block spatial-temporal
    // complexity.
    pub(crate) reduce_mv_pel_precision_lowcomplex: i32,
    // Prune intra mode evaluation in inter frames based on mv range.
    pub(crate) prune_intra_mode_based_on_mv_range: BlockSize,
    // The number of times to left shift the splitting thresholds in variance
    // based partitioning. The minimum values should be 7 to avoid left shifting
    // by a negative number.
    pub(crate) var_part_split_threshold_shift: i32,
    // Qindex based variance partition threshold index, which determines
    // the aggressiveness of partition pruning
    // 0: disabled for speeds 9,10
    // 1,2: (rd-path) lowers qindex thresholds conditionally (for low SAD sb)
    // 3,4: (non-rd path) uses pre-tuned qindex thresholds
    pub(crate) var_part_based_on_qidx: i32,
    // Enable GF refresh based on Q value.
    pub(crate) gf_refresh_based_on_qp: i32,
    // Temporal filtering
    // The value can be 1 or 2, which indicates the threshold to use.
    pub(crate) use_rtc_tf: i32,
    // Prune the use of the identity transform in nonrd_pickmode,
    // used for screen content mode: only for smaller blocks
    // and higher spatial variance, and when skip_txfm is not
    // already set.
    pub(crate) prune_idtx_nonrd: i32,
    // Skip loopfilter, for static content after slide change
    // or key frame, once quality has ramped up.
    pub(crate) skip_lf_screen: i32,
    // For nonrd: early exit out of variance partition that sets the
    // block size to superblock size, and sets mode to zeromv-last skip.
    // 0: disabled
    // 1: zeromv-skip is enabled at SB level only
    // 2: zeromv-skip is enabled at SB level and coding block level
    pub(crate) part_early_exit_zeromv: i32,
    // Early terminate inter mode search based on sse in non-rd path.
    // (INTER_SEARCH_EARLY_TERM_IDX)
    pub(crate) sse_early_term_inter_search: i32,
    // SAD based adaptive altref selection
    pub(crate) sad_based_adp_altref_lag: i32,
    // Enable/disable partition direct merging.
    pub(crate) partition_direct_merging: i32,
    // Level of aggressiveness for obtaining tx size based on qstep
    pub(crate) tx_size_level_based_on_qstep: i32,
    // Avoid the partitioning of a 16x16 block in variance based partitioning
    // (VBP) by making use of minimum and maximum sub-block variances.
    // For allintra encode, this speed feature reduces instruction count by 5.39%
    // for speed 9 on a typical video dataset with coding performance gain
    // of 1.44%.
    // For AVIF image encode, this speed feature reduces encode time
    // by 8.44% for speed 9 on a typical image dataset with coding performance
    // gain of 0.78%.
    pub(crate) vbp_prune_16x16_split_using_min_max_sub_blk_var: bool,
    // A qindex threshold that determines whether to use qindex based CDEF filter
    // strength estimation for screen content types. The strength estimation model
    // used for screen contents prefers to allow cdef filtering for more frames.
    // This sf is used to limit the frames which go through cdef filtering and
    // following explains the setting of the same.
    // MAXQ (255): This disables the usage of this sf. Here, frame does not use a
    // screen content model thus reduces the number of frames that go through cdef
    // filtering.
    // MINQ (0): Frames always use screen content model thus increasing the number
    // of frames that go through cdef filtering.
    // This speed feature has a substantial gain on coding metrics, with moderate
    // increase encoding time. Select threshold based on speed vs quality
    // trade-off.
    pub(crate) screen_content_cdef_filter_qindex_thresh: i32,
    // Prune compound mode if its variance is higher than the variance of single
    // modes.
    pub(crate) prune_compoundmode_with_singlecompound_var: bool,
    // Allow mode cost update at frame level every couple frames. This
    // overrides the command line setting --mode-cost-upd-freq=3 (never update
    // except on key frame and first delta).
    pub(crate) frame_level_mode_cost_update: bool,
    // Prune H_PRED during intra mode evaluation in the nonrd path based on best
    // mode so far.
    // For allintra encode, this speed feature reduces instruction count by 1.10%
    // for speed 9 with coding performance change less than 0.04%.
    // For AVIF image encode, this speed feature reduces encode time by 1.03% for
    // speed 9 on a typical image dataset with coding performance change less than
    // 0.08%.
    pub(crate) prune_h_pred_using_best_mode_so_far: bool,
    // If compound is enabled, and the current block size is \geq BLOCK_16X16,
    // limit the compound modes to GLOBAL_GLOBALMV. This does not apply to the
    // base layer of svc.
    pub(crate) check_only_zero_zeromv_on_large_blocks: bool,
    // Allow for disabling cdf update for non reference frames in svc mode.
    pub(crate) disable_cdf_update_non_reference_frame: bool,
    // Prune compound modes if the single modes variances do not perform well.
    pub(crate) prune_compoundmode_with_singlemode_var: bool,
    // Skip searching all compound mode if the variance of single_mode residue is
    // sufficiently low.
    pub(crate) skip_compound_based_on_var: bool,
    // Sets force_zeromv_skip based on the source sad available. Aggressiveness
    // increases with increase in the level set for speed feature.
    // 0: No setting
    // 1: If source sad is kZeroSad
    // 2: If source sad <= kVeryLowSad
    pub(crate) set_zeromv_skip_based_on_source_sad: i32,
    // Downgrades the block-level subpel motion search to
    // av1_find_best_sub_pixel_tree_pruned_more for higher QP and when fullpel
    // search performed well, zeromv has low sad or low source_var
    pub(crate) use_adaptive_subpel_search: bool,
    // A flag used in RTC case to control frame_refs_short_signaling. Note that
    // the final decision is made in check_frame_refs_short_signaling(). The flag
    // can only be turned on when res < 360p and speed >= 9, in which case only
    // LAST and GOLDEN ref frames are used now.
    pub(crate) enable_ref_short_signaling: bool,
    // A flag that controls if we check or bypass GLOBALMV in rtc single ref frame
    // case.
    pub(crate) check_globalmv_on_single_ref: bool,
}

/// `SPEED_FEATURES`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SpeedFeatures {
    // Sequence/frame level speed features:
    // /
    pub(crate) hl_sf: HighLevelSpeedFeatures,
    // Speed features for the first pass.
    // /
    pub(crate) fp_sf: FirstPassSpeedFeatures,
    // Speed features related to how tpl's searches are done.
    // /
    pub(crate) tpl_sf: TplSpeedFeatures,
    // Global motion speed features:
    // /
    pub(crate) gm_sf: GlobalMotionSpeedFeatures,
    // Partition search speed features:
    // /
    pub(crate) part_sf: PartitionSpeedFeatures,
    // Motion search speed features:
    // /
    pub(crate) mv_sf: MvSpeedFeatures,
    // Inter mode search speed features:
    // /
    pub(crate) inter_sf: InterModeSpeedFeatures,
    // Interpolation filter search speed features:
    // /
    pub(crate) interp_sf: InterpFilterSpeedFeatures,
    // Intra mode search speed features:
    // /
    pub(crate) intra_sf: IntraModeSpeedFeatures,
    // Transform size/type search speed features:
    // /
    pub(crate) tx_sf: TxSpeedFeatures,
    // RD calculation speed features:
    // /
    pub(crate) rd_sf: RdCalcSpeedFeatures,
    // Two-pass mode evaluation features:
    // /
    pub(crate) winner_mode_sf: WinnerModeSpeedFeatures,
    // In-loop filter speed features:
    // /
    pub(crate) lpf_sf: LoopFilterSpeedFeatures,
    // Real-time mode speed features:
    // /
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
    let is_480p_or_larger = cm.width.min(cm.height) >= 480;
    let is_720p_or_larger = cm.width.min(cm.height) >= 720;
    let is_1080p_or_larger = cm.width.min(cm.height) >= 1080;
    let is_4k_or_larger = cm.width.min(cm.height) >= 2160;
    let use_hbd = cpi.oxcf.use_highbitdepth;

    if is_480p_or_larger {
        sf.part_sf.use_square_partition_only_threshold = BLOCK_128X128;
        if is_720p_or_larger {
            sf.part_sf.auto_max_partition_based_on_simple_motion = ADAPT_PRED;
        } else {
            sf.part_sf.auto_max_partition_based_on_simple_motion = RELAXED_PRED;
        }
    } else {
        sf.part_sf.use_square_partition_only_threshold = BLOCK_64X64;
        sf.part_sf.auto_max_partition_based_on_simple_motion = DIRECT_PRED;
        if use_hbd {
            sf.tx_sf.prune_tx_size_level = 1;
        }
    }

    if is_4k_or_larger {
        sf.part_sf.default_min_partition_size = BLOCK_8X8;
    }

    // TODO(huisu@google.com): train models for 720P and above.
    if !is_720p_or_larger {
        sf.part_sf.ml_partition_search_breakout_thresh[0] = 200; // BLOCK_8X8
        sf.part_sf.ml_partition_search_breakout_thresh[1] = 250; // BLOCK_16X16
        sf.part_sf.ml_partition_search_breakout_thresh[2] = 300; // BLOCK_32X32
        sf.part_sf.ml_partition_search_breakout_thresh[3] = 500; // BLOCK_64X64
        sf.part_sf.ml_partition_search_breakout_thresh[4] = -1; // BLOCK_128X128
        sf.part_sf.ml_early_term_after_part_split_level = 1;
    }

    if is_720p_or_larger {
        // TODO(chiyotsai@google.com): make this speed feature adaptive based on
        // current block's vertical texture instead of hardcoded with resolution
        sf.mv_sf.use_downsampled_sad = 1;
    }

    if speed >= 1 {
        if is_720p_or_larger {
            sf.part_sf.use_square_partition_only_threshold = BLOCK_128X128;
        } else if is_480p_or_larger {
            sf.part_sf.use_square_partition_only_threshold = BLOCK_64X64;
        } else {
            sf.part_sf.use_square_partition_only_threshold = BLOCK_32X32;
        }

        if !is_720p_or_larger {
            sf.part_sf.ml_partition_search_breakout_thresh[0] = 200; // BLOCK_8X8
            sf.part_sf.ml_partition_search_breakout_thresh[1] = 250; // BLOCK_16X16
            sf.part_sf.ml_partition_search_breakout_thresh[2] = 300; // BLOCK_32X32
            sf.part_sf.ml_partition_search_breakout_thresh[3] = 300; // BLOCK_64X64
            sf.part_sf.ml_partition_search_breakout_thresh[4] = -1; // BLOCK_128X128
        }
        sf.part_sf.ml_early_term_after_part_split_level = 2;
    }

    if speed >= 2 {
        if is_720p_or_larger {
            sf.part_sf.use_square_partition_only_threshold = BLOCK_64X64;
        } else if is_480p_or_larger {
            sf.part_sf.use_square_partition_only_threshold = BLOCK_32X32;
        } else {
            sf.part_sf.use_square_partition_only_threshold = BLOCK_32X32;
        }

        if is_720p_or_larger {
            sf.part_sf.partition_search_breakout_dist_thr = 1 << 24;
            sf.part_sf.partition_search_breakout_rate_thr = 120;
        } else {
            sf.part_sf.partition_search_breakout_dist_thr = 1 << 22;
            sf.part_sf.partition_search_breakout_rate_thr = 100;
        }

        if is_480p_or_larger {
            sf.tx_sf.tx_type_search.prune_tx_type_using_stats = 1;
            if use_hbd {
                sf.tx_sf.prune_tx_size_level = 2;
            }
        } else {
            if use_hbd {
                sf.tx_sf.prune_tx_size_level = 3;
            }
        }
    }

    if speed >= 3 {
        sf.part_sf.ml_early_term_after_part_split_level = 0;

        if is_720p_or_larger {
            sf.part_sf.partition_search_breakout_dist_thr = 1 << 25;
            sf.part_sf.partition_search_breakout_rate_thr = 200;
        } else {
            sf.part_sf.max_intra_bsize = BLOCK_32X32;
            sf.part_sf.partition_search_breakout_dist_thr = 1 << 23;
            sf.part_sf.partition_search_breakout_rate_thr = 120;
        }
        if use_hbd {
            sf.tx_sf.prune_tx_size_level = 3;
        }
    }

    if speed >= 4 {
        if is_720p_or_larger {
            sf.part_sf.partition_search_breakout_dist_thr = 1 << 26;
        } else {
            sf.part_sf.partition_search_breakout_dist_thr = 1 << 24;
        }

        if is_480p_or_larger {
            sf.tx_sf.tx_type_search.prune_tx_type_using_stats = 2;
        }
    }

    if speed >= 6 {
        if is_720p_or_larger {
            sf.part_sf.auto_max_partition_based_on_simple_motion = NOT_IN_USE;
        } else if is_480p_or_larger {
            sf.part_sf.auto_max_partition_based_on_simple_motion = DIRECT_PRED;
        }

        if is_1080p_or_larger {
            sf.part_sf.default_min_partition_size = BLOCK_8X8;
        }

        sf.part_sf.use_square_partition_only_threshold = BLOCK_16X16;
    }

    if speed >= 7 {
        // TODO(kyslov): add more speed features to control speed/quality
    }

    if speed >= 8 {
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
    let cm = &cpi.common;
    let allow_screen_content_tools = cm.features.allow_screen_content_tools;
    let use_hbd = cpi.oxcf.use_highbitdepth;

    sf.part_sf.less_rectangular_check_level = 1;
    sf.part_sf.ml_prune_partition = 1;
    sf.part_sf.prune_ext_partition_types_search_level = 1;
    sf.part_sf.prune_part4_search = 2;
    sf.part_sf.simple_motion_search_prune_rect = 1;
    sf.part_sf.ml_predict_breakout_level = if use_hbd { 1 } else { 3 };
    sf.part_sf.reuse_prev_rd_results_for_part_ab = 1;
    sf.part_sf.use_best_rd_for_pruning = 1;

    sf.intra_sf.intra_pruning_with_hog = 1;
    sf.intra_sf.prune_luma_palette_size_search_level = 1;
    sf.intra_sf.dv_cost_upd_level = INTERNAL_COST_UPD_OFF;
    sf.intra_sf.early_term_chroma_palette_size_search = 1;

    sf.tx_sf.adaptive_txb_search_level = 1;
    sf.tx_sf.intra_tx_size_search_init_depth_sqr = 1;
    sf.tx_sf.model_based_prune_tx_search_level = 1;
    sf.tx_sf.tx_type_search.use_reduced_intra_txset = 1;

    sf.rt_sf.use_nonrd_pick_mode = 0;
    sf.rt_sf.use_real_time_ref_set = 0;

    // (cpi->twopass_frame.fr_content_type: one pass, FC_NORMAL)
    if cpi.use_screen_content_tools != 0 {
        sf.mv_sf.exhaustive_searches_thresh = 1 << 20;
    } else {
        sf.mv_sf.exhaustive_searches_thresh = 1 << 25;
    }

    sf.rd_sf.perform_coeff_opt = 1;
    sf.hl_sf.superres_auto_search_type = SUPERRES_AUTO_DUAL;

    if speed >= 1 {
        sf.part_sf.intra_cnn_based_part_prune_level =
            if allow_screen_content_tools { 0 } else { 2 };
        sf.part_sf.simple_motion_search_early_term_none = 1;
        // TODO(Venkat): Clean-up frame type dependency for
        // simple_motion_search_split in partition search function and set the
        // speed feature accordingly
        sf.part_sf.simple_motion_search_split = if allow_screen_content_tools { 1 } else { 2 };
        sf.part_sf.ml_predict_breakout_level = if use_hbd { 2 } else { 3 };
        sf.part_sf.reuse_best_prediction_for_part_ab = 1;

        sf.mv_sf.exhaustive_searches_thresh <<= 1;

        sf.intra_sf.prune_palette_search_level = 1;
        sf.intra_sf.prune_luma_palette_size_search_level = 2;
        sf.intra_sf.top_intra_model_count_allowed = 3;

        sf.tx_sf.adaptive_txb_search_level = 2;
        sf.tx_sf.inter_tx_size_search_init_depth_rect = 1;
        sf.tx_sf.inter_tx_size_search_init_depth_sqr = 1;
        sf.tx_sf.intra_tx_size_search_init_depth_rect = 1;
        sf.tx_sf.model_based_prune_tx_search_level = 0;
        sf.tx_sf.tx_type_search.ml_tx_split_thresh = 4000;
        sf.tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_2;
        sf.tx_sf.tx_type_search.skip_tx_search = 1;

        sf.rd_sf.perform_coeff_opt = 2;
        sf.rd_sf.tx_domain_dist_level = 1;
        sf.rd_sf.tx_domain_dist_thres_level = 1;

        sf.lpf_sf.cdef_pick_method = CDEF_FAST_SEARCH_LVL1;
        sf.lpf_sf.dual_sgr_penalty_level = 1;
        sf.lpf_sf.enable_sgr_ep_pruning = 1;
    }

    if speed >= 2 {
        sf.mv_sf.auto_mv_step_size = 1;

        sf.intra_sf.disable_smooth_intra = 1;
        sf.intra_sf.intra_pruning_with_hog = 2;
        sf.intra_sf.prune_filter_intra_level = 1;

        sf.rd_sf.perform_coeff_opt = 3;

        sf.lpf_sf.prune_wiener_based_on_src_var = 1;
        sf.lpf_sf.prune_sgr_based_on_wiener = 1;
    }

    if speed >= 3 {
        sf.hl_sf.high_precision_mv_usage = CURRENT_Q;
        sf.hl_sf.recode_loop = ALLOW_RECODE_KFARFGF;

        sf.part_sf.less_rectangular_check_level = 2;
        sf.part_sf.simple_motion_search_prune_agg = SIMPLE_AGG_LVL1;
        sf.part_sf.prune_ext_part_using_split_info = 1;

        sf.mv_sf.full_pixel_search_level = 1;
        sf.mv_sf.search_method = DIAMOND;

        // TODO(chiyotsai@google.com): the thresholds chosen for intra hog are
        // inherited directly from luma hog with some minor tweaking. Eventually we
        // should run this with a bayesian optimizer to find the Pareto frontier.
        sf.intra_sf.chroma_intra_pruning_with_hog = 2;
        sf.intra_sf.intra_pruning_with_hog = 3;
        sf.intra_sf.prune_palette_search_level = 2;

        sf.tx_sf.adaptive_txb_search_level = 2;
        sf.tx_sf.tx_type_search.use_skip_flag_prediction = 2;

        // TODO(any): evaluate if these lpf features can be moved to speed 2.
        // For screen content, "prune_sgr_based_on_wiener = 2" cause large quality
        // loss.
        sf.lpf_sf.prune_sgr_based_on_wiener = if allow_screen_content_tools { 1 } else { 2 };
        sf.lpf_sf.disable_loop_restoration_chroma = 0;
        sf.lpf_sf.reduce_wiener_window_size = 1;
        sf.lpf_sf.prune_wiener_based_on_src_var = 2;
    }

    if speed >= 4 {
        sf.mv_sf.subpel_search_method = SUBPEL_TREE_PRUNED_MORE;

        sf.part_sf.simple_motion_search_prune_agg = SIMPLE_AGG_LVL2;
        sf.part_sf.simple_motion_search_reduce_search_steps = 4;
        sf.part_sf.prune_ext_part_using_split_info = 2;
        sf.part_sf.early_term_after_none_split = 1;
        sf.part_sf.ml_predict_breakout_level = 3;

        sf.intra_sf.prune_chroma_modes_using_luma_winner = 1;

        sf.mv_sf.simple_motion_subpel_force_stop = HALF_PEL;

        sf.tpl_sf.prune_starting_mv = 2;
        sf.tpl_sf.subpel_force_stop = HALF_PEL;
        sf.tpl_sf.search_method = FAST_BIGDIA;

        sf.tx_sf.tx_type_search.winner_mode_tx_type_pruning = 2;
        sf.tx_sf.tx_type_search.fast_intra_tx_type_search = 1;
        sf.tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_3;
        sf.tx_sf.tx_type_search.prune_tx_type_est_rd = 1;

        sf.rd_sf.perform_coeff_opt = 5;
        sf.rd_sf.tx_domain_dist_thres_level = 3;

        sf.lpf_sf.lpf_pick = LPF_PICK_FROM_FULL_IMAGE_NON_DUAL;
        sf.lpf_sf.cdef_pick_method = CDEF_FAST_SEARCH_LVL3;

        sf.mv_sf.reduce_search_range = 1;

        sf.winner_mode_sf.enable_winner_mode_for_coeff_opt = 1;
        sf.winner_mode_sf.enable_winner_mode_for_use_tx_domain_dist = 1;
        sf.winner_mode_sf.multi_winner_mode_type = MULTI_WINNER_MODE_DEFAULT;
        sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch = 1;
    }

    if speed >= 5 {
        sf.part_sf.simple_motion_search_prune_agg = SIMPLE_AGG_LVL3;
        sf.part_sf.ext_partition_eval_thresh = if allow_screen_content_tools {
            BLOCK_8X8
        } else {
            BLOCK_16X16
        };
        sf.part_sf.intra_cnn_based_part_prune_level =
            if allow_screen_content_tools { 1 } else { 2 };

        sf.intra_sf.chroma_intra_pruning_with_hog = 3;

        sf.lpf_sf.use_coarse_filter_level_search = 0;
        sf.lpf_sf.disable_lr_filter = 1;

        sf.mv_sf.prune_mesh_search = PRUNE_MESH_SEARCH_LVL_2;

        sf.winner_mode_sf.multi_winner_mode_type = MULTI_WINNER_MODE_FAST;
    }

    if speed >= 6 {
        sf.intra_sf.prune_smooth_intra_mode_for_chroma = true;
        sf.intra_sf.prune_filter_intra_level = 2;
        sf.intra_sf.chroma_intra_pruning_with_hog = 4;
        sf.intra_sf.intra_pruning_with_hog = 4;
        sf.intra_sf.cfl_search_range = 1;
        sf.intra_sf.top_intra_model_count_allowed = 2;
        sf.intra_sf.adapt_top_model_rd_count_using_neighbors = 1;
        sf.intra_sf.prune_luma_odd_delta_angles_in_intra = 1;

        sf.part_sf.prune_rectangular_split_based_on_qidx =
            if allow_screen_content_tools { 0 } else { 2 };
        sf.part_sf.prune_sub_8x8_partition_level = if allow_screen_content_tools { 0 } else { 1 };
        sf.part_sf.prune_part4_search = 3;
        // TODO(jingning): This might not be a good trade off if the
        // target image quality is very low.
        sf.part_sf.default_max_partition_size = BLOCK_32X32;

        sf.mv_sf.use_bsize_dependent_search_method = 1;

        sf.tx_sf.tx_type_search.winner_mode_tx_type_pruning = 3;
        sf.tx_sf.tx_type_search.prune_tx_type_est_rd = 0;
        sf.tx_sf.prune_intra_tx_depths_using_nn = true;

        sf.rd_sf.perform_coeff_opt = 6;
        sf.rd_sf.tx_domain_dist_level = 3;

        sf.lpf_sf.cdef_pick_method = CDEF_FAST_SEARCH_LVL4;
        sf.lpf_sf.lpf_pick = LPF_PICK_FROM_Q;

        sf.winner_mode_sf.multi_winner_mode_type = MULTI_WINNER_MODE_OFF;
        sf.winner_mode_sf.prune_winner_mode_eval_level = 1;
        sf.winner_mode_sf.dc_blk_pred_level = 1;
    }
    // The following should make all-intra mode speed 7 approximately equal
    // to real-time speed 6,
    // all-intra speed 8 close to real-time speed 7, and all-intra speed 9
    // close to real-time speed 8
    if speed >= 7 {
        sf.part_sf.default_min_partition_size = BLOCK_8X8;
        sf.part_sf.partition_search_type = VAR_BASED_PARTITION;
        sf.lpf_sf.cdef_pick_method = CDEF_PICK_FROM_Q;
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

    // As the speed feature prune_chroma_modes_using_luma_winner already
    // constrains the number of chroma directional mode evaluations to a maximum
    // of 1, the HOG computation and the associated pruning logic does not seem to
    // help speed-up the chroma mode evaluations. Hence disable the speed feature
    // chroma_intra_pruning_with_hog when prune_chroma_modes_using_luma_winner is
    // enabled.
    if sf.intra_sf.prune_chroma_modes_using_luma_winner != 0 {
        sf.intra_sf.chroma_intra_pruning_with_hog = 0;
    }
}

// (set_good_speed_feature_framesize_dependent() and
// set_good_speed_features_framesize_independent(): the good quality usage is
// not translated, see av1_cx_iface.rs)

fn set_rt_speed_feature_framesize_dependent(cpi: &Av1Comp, sf: &mut SpeedFeatures, speed: i32) {
    let cm = &cpi.common;
    let boosted = frame_is_boosted(cpi);
    let is_1080p_or_larger = cm.width.min(cm.height) >= 1080;
    let is_720p_or_larger = cm.width.min(cm.height) >= 720;
    let is_480p_or_larger = cm.width.min(cm.height) >= 480;
    let is_360p_or_larger = cm.width.min(cm.height) >= 360;

    if !is_360p_or_larger {
        sf.rt_sf.prune_intra_mode_based_on_mv_range = 1;
        sf.rt_sf.prune_inter_modes_wrt_gf_arf_based_on_sad = 1;
        if speed >= 6 {
            sf.winner_mode_sf.prune_winner_mode_eval_level = if boosted { 0 } else { 2 };
        }
        if speed == 7 {
            sf.rt_sf.prefer_large_partition_blocks = 2;
        }
        if speed >= 7 {
            sf.lpf_sf.cdef_pick_method = CDEF_PICK_FROM_Q;
            sf.rt_sf.check_only_zero_zeromv_on_large_blocks = true;
            sf.rt_sf.use_rtc_tf = 2;
        }
        if speed == 8 {
            sf.rt_sf.prefer_large_partition_blocks = 1;
        }
        if speed >= 8 {
            sf.rt_sf.use_nonrd_filter_search = 1;
            sf.rt_sf.tx_size_level_based_on_qstep = 1;
        }
        if speed >= 9 {
            sf.rt_sf.use_comp_ref_nonrd = 0;
            sf.rt_sf.nonrd_aggressive_skip = 1;
            sf.rt_sf.skip_intra_pred = 1;
            // Only turn on enable_ref_short_signaling for low resolution when only
            // LAST and GOLDEN ref frames are used.
            sf.rt_sf.enable_ref_short_signaling = sf.rt_sf.use_nonrd_altref_frame == 0
                && (sf.rt_sf.use_comp_ref_nonrd == 0
                    || (sf.rt_sf.ref_frame_comp_nonrd[1] == 0
                        && sf.rt_sf.ref_frame_comp_nonrd[2] == 0));

            // TODO(kyslov) Re-enable when AV1 models are trained
            // (#if 0: CONFIG_RT_ML_PARTITIONING)
            sf.rt_sf.use_adaptive_subpel_search = false;
        }
        if speed >= 10 {
            sf.rt_sf.skip_intra_pred = 2;
            sf.rt_sf.hybrid_intra_pickmode = 3;
            sf.rt_sf.reduce_mv_pel_precision_lowcomplex = 1;
            sf.rt_sf.reduce_mv_pel_precision_highmotion = 2;
            sf.rt_sf.use_nonrd_filter_search = 0;
        }
    } else {
        sf.rt_sf.prune_intra_mode_based_on_mv_range = 2;
        sf.intra_sf.skip_filter_intra_in_inter_frames = 1;
        if speed <= 5 {
            sf.tx_sf.tx_type_search.fast_inter_tx_type_prob_thresh =
                if boosted { i32::MAX } else { 350 };
            sf.winner_mode_sf.prune_winner_mode_eval_level = if boosted { 0 } else { 2 };
        }
        if speed == 6 {
            sf.part_sf.disable_8x8_part_based_on_qidx = 1;
        }
        if speed >= 6 {
            sf.rt_sf.skip_newmv_mode_based_on_sse = 2;
        }
        if speed == 7 {
            sf.rt_sf.prefer_large_partition_blocks = 1;
            // Enable this feature for [360p, 720p] resolution range initially.
            if !cpi.rc.rtc_external_ratectrl && cm.width.min(cm.height) <= 720 {
                sf.hl_sf.accurate_bit_estimate = (cpi.oxcf.q_cfg.aq_mode == NO_AQ) as i32;
            }
        }
        if speed >= 7 {
            sf.rt_sf.use_rtc_tf = 1;
        }
        if speed == 8 && !cpi.ppi.use_svc {
            sf.rt_sf.short_circuit_low_temp_var = 0;
            sf.rt_sf.use_nonrd_altref_frame = 1;
        }
        if speed >= 8 {
            sf.rt_sf.tx_size_level_based_on_qstep = 2;
        }
        if speed >= 9 {
            sf.rt_sf.gf_length_lvl = 1;
            sf.rt_sf.skip_cdef_sb = 1;
            sf.rt_sf.sad_based_adp_altref_lag = 2;
            sf.rt_sf.reduce_mv_pel_precision_highmotion = 2;
            sf.rt_sf.use_adaptive_subpel_search = true;
            sf.interp_sf.cb_pred_filter_search = 1;
        }
        if speed >= 10 {
            sf.rt_sf.hybrid_intra_pickmode = 2;
            sf.rt_sf.sad_based_adp_altref_lag = 4;
            sf.rt_sf.tx_size_level_based_on_qstep = 0;
            sf.rt_sf.reduce_mv_pel_precision_highmotion = 3;
            sf.rt_sf.use_adaptive_subpel_search = false;
            sf.interp_sf.cb_pred_filter_search = 2;
        }
    }
    if !is_480p_or_larger {
        if speed == 7 {
            sf.rt_sf.nonrd_check_partition_merge_mode = 2;
        }
        if speed >= 8 {
            sf.rt_sf.estimate_motion_for_var_based_partition = 1;
        }
        if speed >= 9 {
            sf.rt_sf.estimate_motion_for_var_based_partition = 0;
        }
    }
    if !is_720p_or_larger {
        if speed >= 9 {
            sf.rt_sf.force_large_partition_blocks_intra = 1;
        }
    } else {
        if speed >= 6 {
            sf.rt_sf.skip_newmv_mode_based_on_sse = 3;
        }
        if speed == 7 {
            sf.rt_sf.prefer_large_partition_blocks = 0;
        }
        if speed >= 7 {
            sf.rt_sf.reduce_mv_pel_precision_lowcomplex = 2;
            sf.rt_sf.reduce_mv_pel_precision_highmotion = 1;
        }
        if speed >= 9 {
            sf.rt_sf.sad_based_adp_altref_lag = 1;
            sf.rt_sf.reduce_mv_pel_precision_lowcomplex = 0;
            sf.rt_sf.reduce_mv_pel_precision_highmotion = 2;
        }
        if speed >= 10 {
            sf.rt_sf.sad_based_adp_altref_lag = 3;
            sf.rt_sf.reduce_mv_pel_precision_highmotion = 3;
        }
    }
    // TODO(Any): Check/Tune settings of other sfs for 1080p.
    if is_1080p_or_larger {
        if speed >= 7 {
            sf.rt_sf.reduce_mv_pel_precision_highmotion = 0;
            sf.rt_sf.use_adaptive_subpel_search = false;
        }
        if speed >= 9 {
            sf.interp_sf.cb_pred_filter_search = 0;
        }
    } else {
        if speed >= 9 {
            sf.lpf_sf.cdef_pick_method = CDEF_PICK_FROM_Q;
        }
        if speed >= 10 {
            sf.rt_sf.nonrd_aggressive_skip = 1;
        }
    }

    // Setting for SVC, or when the ref_frame_config control is
    // used to set the reference structure.
    // (neither: one layer, no external reference configuration)
    debug_assert!(!cpi.ppi.use_svc && !cpi.ppi.rtc_ref.set_ref_frame_config);
    // Screen settings.
    // (the content type is never AOM_CONTENT_SCREEN)
    debug_assert!(cpi.oxcf.tune_cfg.content != AOM_CONTENT_SCREEN);
}

// TODO(kyslov): now this is very similar to
// set_good_speed_features_framesize_independent
// except it sets non-rd flag on speed 8. This function will likely
// be modified in the future with RT-specific speed features.
fn set_rt_speed_features_framesize_independent(cpi: &Av1Comp, sf: &mut SpeedFeatures, speed: i32) {
    let cm = &cpi.common;
    let boosted = frame_is_boosted(cpi);

    // Currently, rt speed 0, 1, 2, 3, 4, 5 are the same.
    // Following set of speed features are not impacting encoder's decisions as
    // the relevant tools are disabled by default.
    sf.gm_sf.gm_search_type = GM_DISABLE_SEARCH;
    sf.hl_sf.recode_loop = ALLOW_RECODE_KFARFGF;
    sf.inter_sf.reuse_inter_intra_mode = 1;
    sf.inter_sf.prune_compound_using_single_ref = 0;
    sf.inter_sf.prune_comp_search_by_single_result = 2;
    sf.inter_sf.prune_comp_type_by_comp_avg = 2;
    sf.inter_sf.fast_wedge_sign_estimate = 1;
    sf.inter_sf.use_dist_wtd_comp_flag = DIST_WTD_COMP_DISABLED;
    sf.inter_sf.mv_cost_upd_level = INTERNAL_COST_UPD_SBROW;
    sf.inter_sf.disable_interinter_wedge_var_thresh = 100;
    sf.interp_sf.cb_pred_filter_search = 0;
    sf.part_sf.ml_prune_partition = 1;
    sf.part_sf.reuse_prev_rd_results_for_part_ab = 1;
    sf.part_sf.prune_ext_partition_types_search_level = 2;
    sf.part_sf.less_rectangular_check_level = 2;
    sf.mv_sf.obmc_full_pixel_search_level = 1;
    sf.intra_sf.dv_cost_upd_level = INTERNAL_COST_UPD_OFF;
    sf.tx_sf.model_based_prune_tx_search_level = 0;
    sf.lpf_sf.dual_sgr_penalty_level = 1;
    sf.lpf_sf.disable_lr_filter = 1;
    sf.rt_sf.skip_interp_filter_search = 1;
    sf.intra_sf.prune_palette_search_level = 2;
    sf.intra_sf.prune_luma_palette_size_search_level = 2;
    sf.intra_sf.early_term_chroma_palette_size_search = 1;

    // End of set

    // TODO(any, yunqing): tune these features for real-time use cases.
    sf.hl_sf.superres_auto_search_type = SUPERRES_AUTO_SOLO;
    sf.hl_sf.frame_parameter_update = 0;

    sf.inter_sf.model_based_post_interp_filter_breakout = 1;
    // TODO(any): As per the experiments, this speed feature is doing redundant
    // computation since the model rd based pruning logic is similar to model rd
    // based gating when inter_mode_rd_model_estimation = 2. Enable this SF if
    // either of the condition becomes true.
    //    (1) inter_mode_rd_model_estimation != 2
    //    (2) skip_interp_filter_search == 0
    //    (3) Motion mode or compound mode is enabled */
    sf.inter_sf.prune_mode_search_simple_translation = 0;
    sf.inter_sf.prune_ref_frame_for_rect_partitions = !boosted as i32;
    sf.inter_sf.disable_interintra_wedge_var_thresh = u32::MAX;
    sf.inter_sf.selective_ref_frame = 4;
    sf.inter_sf.alt_ref_search_fp = 2;
    sf.inter_sf.txfm_rd_gate_level = if boosted { 0 } else { 4 };
    sf.inter_sf.limit_txfm_eval_per_mode = 3;

    sf.inter_sf.adaptive_rd_thresh = 4;
    sf.inter_sf.inter_mode_rd_model_estimation = 2;
    sf.inter_sf.prune_inter_modes_if_skippable = 1;
    sf.inter_sf.prune_nearmv_using_neighbors = PRUNE_NEARMV_LEVEL3;
    sf.inter_sf.reduce_inter_modes = if boosted { 1 } else { 3 };
    sf.inter_sf.skip_newmv_in_drl = 4;

    sf.interp_sf.use_fast_interpolation_filter_search = 1;
    sf.interp_sf.use_interp_filter = 1;
    sf.interp_sf.adaptive_interp_filter_search = 1;
    sf.interp_sf.disable_dual_filter = 1;

    sf.part_sf.default_max_partition_size = BLOCK_128X128;
    sf.part_sf.default_min_partition_size = BLOCK_8X8;
    sf.part_sf.use_best_rd_for_pruning = 1;
    sf.part_sf.early_term_after_none_split = 1;
    sf.part_sf.partition_search_breakout_dist_thr = 1 << 25;
    sf.part_sf.max_intra_bsize = BLOCK_16X16;
    sf.part_sf.partition_search_breakout_rate_thr = 500;
    sf.part_sf.partition_search_type = VAR_BASED_PARTITION;
    sf.part_sf.adjust_var_based_rd_partitioning = 2;

    sf.mv_sf.full_pixel_search_level = 1;
    sf.mv_sf.exhaustive_searches_thresh = i32::MAX;
    sf.mv_sf.auto_mv_step_size = 1;
    sf.mv_sf.subpel_iters_per_step = 1;
    sf.mv_sf.use_accurate_subpel_search = USE_2_TAPS;
    sf.mv_sf.search_method = FAST_DIAMOND;
    sf.mv_sf.subpel_force_stop = EIGHTH_PEL;
    sf.mv_sf.subpel_search_method = SUBPEL_TREE_PRUNED;

    for i in 0..TX_SIZES as usize {
        sf.intra_sf.intra_y_mode_mask[i] = INTRA_DC;
        sf.intra_sf.intra_uv_mode_mask[i] = UV_INTRA_DC_CFL;
    }
    sf.intra_sf.skip_intra_in_interframe = 5;
    sf.intra_sf.disable_smooth_intra = 1;
    sf.intra_sf.skip_filter_intra_in_inter_frames = 1;

    sf.tx_sf.intra_tx_size_search_init_depth_sqr = 1;
    sf.tx_sf.tx_type_search.use_reduced_intra_txset = 1;
    sf.tx_sf.adaptive_txb_search_level = 2;
    sf.tx_sf.intra_tx_size_search_init_depth_rect = 1;
    sf.tx_sf.tx_size_search_lgr_block = 1;
    sf.tx_sf.tx_type_search.ml_tx_split_thresh = 4000;
    sf.tx_sf.tx_type_search.skip_tx_search = 1;
    sf.tx_sf.inter_tx_size_search_init_depth_rect = 1;
    sf.tx_sf.inter_tx_size_search_init_depth_sqr = 1;
    sf.tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_3;
    sf.tx_sf.refine_fast_tx_search_results = 0;
    sf.tx_sf.tx_type_search.fast_intra_tx_type_search = 1;
    sf.tx_sf.tx_type_search.use_skip_flag_prediction = 2;
    sf.tx_sf.tx_type_search.winner_mode_tx_type_pruning = 4;

    sf.rd_sf.optimize_coefficients = NO_TRELLIS_OPT;
    sf.rd_sf.simple_model_rd_from_var = 1;
    sf.rd_sf.tx_domain_dist_level = 2;
    sf.rd_sf.tx_domain_dist_thres_level = 2;

    sf.lpf_sf.cdef_pick_method = CDEF_FAST_SEARCH_LVL4;
    sf.lpf_sf.lpf_pick = LPF_PICK_FROM_Q;

    sf.winner_mode_sf.dc_blk_pred_level = if frame_is_intra_only(cm) { 0 } else { 3 };
    sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch = 1;
    sf.winner_mode_sf.tx_size_search_level = 1;
    sf.winner_mode_sf.winner_mode_ifs = 1;

    sf.rt_sf.check_intra_pred_nonrd = 1;
    sf.rt_sf.estimate_motion_for_var_based_partition = 1;
    sf.rt_sf.hybrid_intra_pickmode = 1;
    sf.rt_sf.use_comp_ref_nonrd = 0;
    sf.rt_sf.ref_frame_comp_nonrd[0] = 0;
    sf.rt_sf.ref_frame_comp_nonrd[1] = 0;
    sf.rt_sf.ref_frame_comp_nonrd[2] = 0;
    sf.rt_sf.use_nonrd_filter_search = 1;
    sf.rt_sf.mode_search_skip_flags |= FLAG_SKIP_INTRA_DIRMISMATCH;
    sf.rt_sf.num_inter_modes_for_tx_search = 5;
    sf.rt_sf.prune_inter_modes_using_temp_var = 1;
    sf.rt_sf.use_real_time_ref_set = 1;
    sf.rt_sf.use_simple_rd_model = 1;
    sf.rt_sf.prune_inter_modes_with_golden_ref = if boosted { 0 } else { 1 };
    // TODO(any): This sf could be removed.
    sf.rt_sf.short_circuit_low_temp_var = 1;
    sf.rt_sf.check_scene_detection = 1;
    if cpi.rc.rtc_external_ratectrl {
        sf.rt_sf.check_scene_detection = 0;
    }
    if cm.current_frame.frame_type != KEY_FRAME && cpi.oxcf.rc_cfg.mode == AOM_CBR {
        sf.rt_sf.overshoot_detection_cbr = FAST_DETECTION_MAXQ;
    }
    // Enable noise estimation only for high resolutions for now.
    //
    // Since use_temporal_noise_estimate has no effect for all-intra frame
    // encoding, it is disabled for this case.
    if cpi.oxcf.kf_cfg.key_freq_max != 0 && cm.width * cm.height > 640 * 480 {
        sf.rt_sf.use_temporal_noise_estimate = 1;
    }
    sf.rt_sf.skip_tx_no_split_var_based_partition = 1;
    sf.rt_sf.skip_newmv_mode_based_on_sse = 1;
    sf.rt_sf.mode_search_skip_flags = if cm.current_frame.frame_type == KEY_FRAME {
        0
    } else {
        FLAG_SKIP_INTRA_DIRMISMATCH
            | FLAG_SKIP_INTRA_BESTINTER
            | FLAG_SKIP_COMP_BESTINTRA
            | FLAG_SKIP_INTRA_LOWVAR
            | FLAG_EARLY_TERMINATE
    };
    sf.rt_sf.var_part_split_threshold_shift = 5;
    if !frame_is_intra_only(&cpi.common) {
        sf.rt_sf.var_part_based_on_qidx = 1;
    }

    if speed >= 6 {
        sf.mv_sf.use_fullpel_costlist = 1;

        sf.rd_sf.tx_domain_dist_thres_level = 3;

        sf.tx_sf.tx_type_search.fast_inter_tx_type_prob_thresh = 0;
        sf.inter_sf.limit_inter_mode_cands = 4;
        sf.inter_sf.prune_warped_prob_thresh = 8;
        sf.inter_sf.extra_prune_warped = 1;

        sf.rt_sf.gf_refresh_based_on_qp = 1;
        sf.rt_sf.prune_inter_modes_wrt_gf_arf_based_on_sad = 1;
        sf.rt_sf.var_part_split_threshold_shift = 7;
        if !frame_is_intra_only(&cpi.common) {
            sf.rt_sf.var_part_based_on_qidx = 2;
        }

        sf.winner_mode_sf.prune_winner_mode_eval_level = if boosted { 0 } else { 3 };
    }

    if speed >= 7 {
        sf.rt_sf.sse_early_term_inter_search = EARLY_TERM_IDX_1;
        sf.rt_sf.use_comp_ref_nonrd = 1;
        sf.rt_sf.ref_frame_comp_nonrd[2] = 1; // LAST_ALTREF
        sf.tx_sf.intra_tx_size_search_init_depth_sqr = 2;
        sf.part_sf.partition_search_type = VAR_BASED_PARTITION;
        sf.part_sf.max_intra_bsize = BLOCK_32X32;

        sf.mv_sf.search_method = FAST_DIAMOND;
        sf.mv_sf.subpel_force_stop = QUARTER_PEL;

        sf.inter_sf.inter_mode_rd_model_estimation = 2;
        // This sf is not applicable in non-rd path.
        sf.inter_sf.skip_newmv_in_drl = 0;

        // Disable intra_y_mode_mask pruning since the performance at speed 7 isn't
        // good. May need more study.
        for i in 0..TX_SIZES as usize {
            sf.intra_sf.intra_y_mode_mask[i] = INTRA_ALL;
        }

        sf.lpf_sf.lpf_pick = LPF_PICK_FROM_Q;
        sf.lpf_sf.cdef_pick_method = CDEF_FAST_SEARCH_LVL5;

        sf.rt_sf.mode_search_skip_flags |= FLAG_SKIP_INTRA_DIRMISMATCH;
        sf.rt_sf.nonrd_prune_ref_frame_search = 1;
        // This is for rd path only.
        sf.rt_sf.prune_inter_modes_using_temp_var = 0;
        sf.rt_sf.prune_inter_modes_wrt_gf_arf_based_on_sad = 0;
        sf.rt_sf.prune_intra_mode_based_on_mv_range = 0;
        // (!CONFIG_REALTIME_ONLY)
        sf.rt_sf.reuse_inter_pred_nonrd = (!cpi.oxcf.motion_mode_cfg.enable_warped_motion) as i32;
        sf.rt_sf.short_circuit_low_temp_var = 0;
        sf.rt_sf.skip_interp_filter_search = 0;
        // For spatial layers, only LAST and GOLDEN are currently used in the SVC
        // for nonrd. The flag use_nonrd_altref_frame can disable GOLDEN in the
        // get_ref_frame_flags() for some patterns, so disable it here for
        // spatial layers.
        sf.rt_sf.use_nonrd_altref_frame = if cpi.svc.number_spatial_layers > 1 {
            0
        } else {
            1
        };
        sf.rt_sf.use_nonrd_pick_mode = 1;
        sf.rt_sf.nonrd_check_partition_merge_mode = 3;
        sf.rt_sf.skip_intra_pred = 1;
        sf.rt_sf.source_metrics_sb_nonrd = 1;
        // Set mask for intra modes.
        for i in 0..BLOCK_SIZES {
            if i >= BLOCK_32X32 {
                sf.rt_sf.intra_y_mode_bsize_mask_nrd[i as usize] = INTRA_DC;
            } else {
                // Use DC, H, V intra mode for block sizes < 32X32.
                sf.rt_sf.intra_y_mode_bsize_mask_nrd[i as usize] = INTRA_DC_H_V;
            }
        }

        sf.winner_mode_sf.dc_blk_pred_level = 0;
        sf.rt_sf.var_part_based_on_qidx = 3;
        sf.rt_sf.prune_compoundmode_with_singlecompound_var = true;
        sf.rt_sf.prune_compoundmode_with_singlemode_var = true;
        sf.rt_sf.skip_compound_based_on_var = true;
        sf.rt_sf.use_adaptive_subpel_search = true;
    }

    if speed >= 8 {
        sf.rt_sf.sse_early_term_inter_search = EARLY_TERM_IDX_2;
        sf.intra_sf.intra_pruning_with_hog = 1;
        sf.rt_sf.estimate_motion_for_var_based_partition = 1;
        sf.rt_sf.short_circuit_low_temp_var = 1;
        sf.rt_sf.use_nonrd_altref_frame = 0;
        sf.rt_sf.nonrd_prune_ref_frame_search = 2;
        sf.rt_sf.nonrd_check_partition_merge_mode = 0;
        sf.rt_sf.var_part_split_threshold_shift = 8;
        sf.rt_sf.var_part_based_on_qidx = 4;
        sf.rt_sf.partition_direct_merging = 1;
        sf.rt_sf.prune_compoundmode_with_singlemode_var = false;
        sf.mv_sf.use_bsize_dependent_search_method = 2;
        sf.rt_sf.prune_hv_pred_modes_using_src_sad = true;
    }
    if speed >= 9 {
        sf.rt_sf.sse_early_term_inter_search = EARLY_TERM_IDX_3;
        sf.rt_sf.estimate_motion_for_var_based_partition = 0;
        sf.rt_sf.prefer_large_partition_blocks = 3;
        sf.rt_sf.skip_intra_pred = 2;
        sf.rt_sf.var_part_split_threshold_shift = 9;
        for i in 0..BLOCK_SIZES as usize {
            sf.rt_sf.intra_y_mode_bsize_mask_nrd[i] = INTRA_DC;
        }
        sf.rt_sf.var_part_based_on_qidx = 0;
        sf.rt_sf.frame_level_mode_cost_update = true;
        sf.rt_sf.check_only_zero_zeromv_on_large_blocks = true;
        sf.rt_sf.reduce_mv_pel_precision_highmotion = 0;
        sf.rt_sf.use_adaptive_subpel_search = true;
        sf.mv_sf.use_bsize_dependent_search_method = 0;
    }
    if speed >= 10 {
        sf.rt_sf.sse_early_term_inter_search = EARLY_TERM_IDX_4;
        sf.rt_sf.nonrd_prune_ref_frame_search = 3;
        sf.rt_sf.var_part_split_threshold_shift = 10;
        sf.mv_sf.subpel_search_method = SUBPEL_TREE_PRUNED_MORE;
    }
}

#[inline]
fn init_hl_sf(hl_sf: &mut HighLevelSpeedFeatures) {
    // best quality defaults
    hl_sf.frame_parameter_update = 1;
    hl_sf.recode_loop = ALLOW_RECODE;
    // Recode loop tolerance %.
    hl_sf.recode_tolerance = 25;
    hl_sf.high_precision_mv_usage = CURRENT_Q;
    hl_sf.superres_auto_search_type = SUPERRES_AUTO_ALL;
    hl_sf.disable_extra_sc_testing = 0;
    hl_sf.second_alt_ref_filtering = 1;
    hl_sf.num_frames_used_in_tf = i32::MAX;
    hl_sf.accurate_bit_estimate = 0;
}

#[inline]
fn init_fp_sf(fp_sf: &mut FirstPassSpeedFeatures) {
    fp_sf.reduce_mv_step_param = 3;
    fp_sf.skip_motion_search_threshold = 0;
    fp_sf.disable_recon = 0;
    fp_sf.skip_zeromv_motion_search = 0;
}

#[inline]
fn init_tpl_sf(tpl_sf: &mut TplSpeedFeatures) {
    tpl_sf.gop_length_decision_method = 0;
    tpl_sf.prune_intra_modes = 0;
    tpl_sf.prune_starting_mv = 0;
    tpl_sf.reduce_first_step_size = 0;
    tpl_sf.skip_alike_starting_mv = 0;
    tpl_sf.subpel_force_stop = EIGHTH_PEL;
    tpl_sf.search_method = NSTEP;
    tpl_sf.disable_filtered_key_tpl = 0;
    tpl_sf.prune_ref_frames_in_tpl = 0;
    tpl_sf.allow_compound_pred = 1;
    tpl_sf.use_y_only_rate_distortion = 0;
}

#[inline]
fn init_gm_sf(gm_sf: &mut GlobalMotionSpeedFeatures) {
    gm_sf.gm_search_type = GM_FULL_SEARCH;
    gm_sf.prune_ref_frame_for_gm_search = 0;
    gm_sf.prune_zero_mv_with_sse = 0;
    gm_sf.disable_gm_search_based_on_stats = 0;
}

#[inline]
fn init_part_sf(part_sf: &mut PartitionSpeedFeatures) {
    part_sf.partition_search_type = SEARCH_PARTITION;
    part_sf.less_rectangular_check_level = 0;
    part_sf.use_square_partition_only_threshold = BLOCK_128X128;
    part_sf.auto_max_partition_based_on_simple_motion = NOT_IN_USE;
    part_sf.default_max_partition_size = BLOCK_LARGEST;
    part_sf.default_min_partition_size = BLOCK_4X4;
    part_sf.adjust_var_based_rd_partitioning = 0;
    part_sf.max_intra_bsize = BLOCK_LARGEST;
    // This setting only takes effect when partition_search_type is set
    // to FIXED_PARTITION.
    part_sf.fixed_partition_size = BLOCK_16X16;
    // Recode loop tolerance %.
    part_sf.partition_search_breakout_dist_thr = 0;
    part_sf.partition_search_breakout_rate_thr = 0;
    part_sf.prune_ext_partition_types_search_level = 0;
    part_sf.prune_part4_search = 0;
    part_sf.ml_prune_partition = 0;
    part_sf.ml_early_term_after_part_split_level = 0;
    for i in 0..PARTITION_BLOCK_SIZES as usize {
        part_sf.ml_partition_search_breakout_thresh[i] = -1; // -1 means not enabled.
    }
    part_sf.simple_motion_search_prune_agg = SIMPLE_AGG_LVL0;
    part_sf.simple_motion_search_split = 0;
    part_sf.simple_motion_search_prune_rect = 0;
    part_sf.simple_motion_search_early_term_none = 0;
    part_sf.simple_motion_search_reduce_search_steps = 0;
    part_sf.intra_cnn_based_part_prune_level = 0;
    part_sf.ext_partition_eval_thresh = BLOCK_8X8;
    part_sf.rect_partition_eval_thresh = BLOCK_128X128;
    part_sf.prune_ext_part_using_split_info = 0;
    part_sf.prune_rectangular_split_based_on_qidx = 0;
    part_sf.early_term_after_none_split = 0;
    part_sf.ml_predict_breakout_level = 0;
    part_sf.prune_sub_8x8_partition_level = 0;
    part_sf.simple_motion_search_rect_split = 0;
    part_sf.reuse_prev_rd_results_for_part_ab = 0;
    part_sf.reuse_best_prediction_for_part_ab = 0;
    part_sf.use_best_rd_for_pruning = 0;
    part_sf.skip_non_sq_part_based_on_none = 0;
    part_sf.disable_8x8_part_based_on_qidx = 0;
}

#[inline]
fn init_mv_sf(mv_sf: &mut MvSpeedFeatures) {
    mv_sf.full_pixel_search_level = 0;
    mv_sf.auto_mv_step_size = 0;
    mv_sf.exhaustive_searches_thresh = 0;
    mv_sf.obmc_full_pixel_search_level = 0;
    mv_sf.prune_mesh_search = PRUNE_MESH_SEARCH_DISABLED;
    mv_sf.reduce_search_range = 0;
    mv_sf.search_method = NSTEP;
    mv_sf.simple_motion_subpel_force_stop = EIGHTH_PEL;
    mv_sf.subpel_force_stop = EIGHTH_PEL;
    mv_sf.subpel_iters_per_step = 2;
    mv_sf.subpel_search_method = SUBPEL_TREE;
    mv_sf.use_accurate_subpel_search = USE_8_TAPS;
    mv_sf.use_bsize_dependent_search_method = 0;
    mv_sf.use_fullpel_costlist = 0;
    mv_sf.use_downsampled_sad = 0;
    mv_sf.disable_extensive_joint_motion_search = 0;
    mv_sf.disable_second_mv = 0;
    mv_sf.skip_fullpel_search_using_startmv = 0;
}

#[inline]
fn init_inter_sf(inter_sf: &mut InterModeSpeedFeatures) {
    inter_sf.adaptive_rd_thresh = 0;
    inter_sf.model_based_post_interp_filter_breakout = 0;
    inter_sf.reduce_inter_modes = 0;
    inter_sf.alt_ref_search_fp = 0;
    inter_sf.prune_comp_ref_frames = 0;
    inter_sf.selective_ref_frame = 0;
    inter_sf.prune_ref_frame_for_rect_partitions = 0;
    inter_sf.fast_wedge_sign_estimate = 0;
    inter_sf.use_dist_wtd_comp_flag = DIST_WTD_COMP_ENABLED;
    inter_sf.reuse_inter_intra_mode = 0;
    inter_sf.mv_cost_upd_level = INTERNAL_COST_UPD_SB;
    inter_sf.coeff_cost_upd_level = INTERNAL_COST_UPD_SB;
    inter_sf.mode_cost_upd_level = INTERNAL_COST_UPD_SB;
    inter_sf.prune_inter_modes_based_on_tpl = 0;
    inter_sf.prune_nearmv_using_neighbors = PRUNE_NEARMV_OFF;
    inter_sf.prune_comp_search_by_single_result = 0;
    inter_sf.skip_repeated_ref_mv = 0;
    inter_sf.skip_newmv_in_drl = 0;
    inter_sf.inter_mode_rd_model_estimation = 0;
    inter_sf.prune_compound_using_single_ref = 0;
    inter_sf.prune_ext_comp_using_neighbors = 0;
    inter_sf.skip_ext_comp_nearmv_mode = 0;
    inter_sf.prune_comp_using_best_single_mode_ref = 0;
    inter_sf.prune_nearest_near_mv_using_refmv_weight = 0;
    inter_sf.disable_onesided_comp = 0;
    inter_sf.prune_mode_search_simple_translation = 0;
    inter_sf.prune_comp_type_by_comp_avg = 0;
    inter_sf.disable_interinter_wedge_newmv_search = 0;
    inter_sf.fast_interintra_wedge_search = 0;
    inter_sf.prune_comp_type_by_model_rd = 0;
    inter_sf.perform_best_rd_based_gating_for_chroma = 0;
    inter_sf.prune_obmc_prob_thresh = 0;
    inter_sf.disable_interinter_wedge_var_thresh = 0;
    inter_sf.disable_interintra_wedge_var_thresh = 0;
    inter_sf.prune_ref_mv_idx_search = 0;
    inter_sf.prune_warped_prob_thresh = 0;
    inter_sf.reuse_compound_type_decision = 0;
    inter_sf.txfm_rd_gate_level = 0;
    inter_sf.prune_inter_modes_if_skippable = 0;
    inter_sf.disable_masked_comp = 0;
    inter_sf.enable_fast_compound_mode_search = 0;
    inter_sf.reuse_mask_search_results = 0;
    inter_sf.enable_fast_wedge_mask_search = 0;
    inter_sf.inter_mode_txfm_breakout = 0;
    inter_sf.limit_inter_mode_cands = 0;
    inter_sf.limit_txfm_eval_per_mode = 0;
    inter_sf.skip_arf_compound = 0;
}

#[inline]
fn init_interp_sf(interp_sf: &mut InterpFilterSpeedFeatures) {
    interp_sf.adaptive_interp_filter_search = 0;
    interp_sf.cb_pred_filter_search = 0;
    interp_sf.disable_dual_filter = 0;
    interp_sf.skip_sharp_interp_filter_search = 0;
    interp_sf.use_fast_interpolation_filter_search = 0;
    interp_sf.use_interp_filter = 0;
}

#[inline]
fn init_intra_sf(intra_sf: &mut IntraModeSpeedFeatures) {
    intra_sf.dv_cost_upd_level = INTERNAL_COST_UPD_SB;
    intra_sf.skip_intra_in_interframe = 1;
    intra_sf.intra_pruning_with_hog = 0;
    intra_sf.chroma_intra_pruning_with_hog = 0;
    intra_sf.prune_palette_search_level = 0;
    intra_sf.prune_luma_palette_size_search_level = 0;

    for i in 0..TX_SIZES as usize {
        intra_sf.intra_y_mode_mask[i] = INTRA_ALL;
        intra_sf.intra_uv_mode_mask[i] = UV_INTRA_ALL;
    }
    intra_sf.disable_smooth_intra = 0;
    intra_sf.prune_smooth_intra_mode_for_chroma = false;
    intra_sf.prune_filter_intra_level = 0;
    intra_sf.prune_chroma_modes_using_luma_winner = 0;
    intra_sf.cfl_search_range = 3;
    intra_sf.top_intra_model_count_allowed = TOP_INTRA_MODEL_COUNT;
    intra_sf.adapt_top_model_rd_count_using_neighbors = 0;
    intra_sf.early_term_chroma_palette_size_search = 0;
    intra_sf.skip_filter_intra_in_inter_frames = 0;
    intra_sf.prune_luma_odd_delta_angles_in_intra = 0;
}

#[inline]
fn init_tx_sf(tx_sf: &mut TxSpeedFeatures) {
    tx_sf.inter_tx_size_search_init_depth_sqr = 0;
    tx_sf.inter_tx_size_search_init_depth_rect = 0;
    tx_sf.intra_tx_size_search_init_depth_rect = 0;
    tx_sf.intra_tx_size_search_init_depth_sqr = 0;
    tx_sf.tx_size_search_lgr_block = 0;
    tx_sf.model_based_prune_tx_search_level = 0;
    tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_1;
    tx_sf.tx_type_search.ml_tx_split_thresh = 8500;
    tx_sf.tx_type_search.use_skip_flag_prediction = 1;
    tx_sf.tx_type_search.use_reduced_intra_txset = 0;
    tx_sf.tx_type_search.fast_intra_tx_type_search = 0;
    tx_sf.tx_type_search.fast_inter_tx_type_prob_thresh = i32::MAX;
    tx_sf.tx_type_search.skip_tx_search = 0;
    tx_sf.tx_type_search.prune_tx_type_using_stats = 0;
    tx_sf.tx_type_search.prune_tx_type_est_rd = 0;
    tx_sf.tx_type_search.winner_mode_tx_type_pruning = 0;
    tx_sf.txb_split_cap = 1;
    tx_sf.adaptive_txb_search_level = 0;
    tx_sf.refine_fast_tx_search_results = 1;
    tx_sf.prune_tx_size_level = 0;
    tx_sf.prune_intra_tx_depths_using_nn = false;
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
    rd_sf.use_mb_rd_hash = 0;
    rd_sf.simple_model_rd_from_var = 0;
    rd_sf.tx_domain_dist_level = 0;
    rd_sf.tx_domain_dist_thres_level = 0;
    rd_sf.perform_coeff_opt = 0;
}

#[inline]
fn init_winner_mode_sf(winner_mode_sf: &mut WinnerModeSpeedFeatures) {
    winner_mode_sf.motion_mode_for_winner_cand = 0;
    // Set this at the appropriate speed levels
    winner_mode_sf.tx_size_search_level = 0;
    winner_mode_sf.enable_winner_mode_for_coeff_opt = 0;
    winner_mode_sf.enable_winner_mode_for_tx_size_srch = 0;
    winner_mode_sf.enable_winner_mode_for_use_tx_domain_dist = 0;
    winner_mode_sf.multi_winner_mode_type = 0;
    winner_mode_sf.dc_blk_pred_level = 0;
    winner_mode_sf.winner_mode_ifs = 0;
    winner_mode_sf.prune_winner_mode_eval_level = 0;
}

#[inline]
fn init_lpf_sf(lpf_sf: &mut LoopFilterSpeedFeatures) {
    lpf_sf.disable_loop_restoration_chroma = 0;
    lpf_sf.disable_loop_restoration_luma = 0;
    lpf_sf.prune_wiener_based_on_src_var = 0;
    lpf_sf.prune_sgr_based_on_wiener = 0;
    lpf_sf.enable_sgr_ep_pruning = 0;
    lpf_sf.reduce_wiener_window_size = 0;
    lpf_sf.lpf_pick = LPF_PICK_FROM_FULL_IMAGE;
    lpf_sf.use_coarse_filter_level_search = 0;
    lpf_sf.cdef_pick_method = CDEF_FULL_SEARCH;
    // Set decoder side speed feature to use less dual sgr modes
    lpf_sf.dual_sgr_penalty_level = 0;
    lpf_sf.disable_lr_filter = 0;
    lpf_sf.use_downsampled_wiener_stats = 0;
}

#[inline]
fn init_rt_sf(rt_sf: &mut RealTimeSpeedFeatures) {
    rt_sf.check_intra_pred_nonrd = 0;
    rt_sf.skip_intra_pred = 0;
    rt_sf.estimate_motion_for_var_based_partition = 0;
    rt_sf.nonrd_check_partition_merge_mode = 0;
    rt_sf.nonrd_check_partition_split = 0;
    rt_sf.mode_search_skip_flags = 0;
    rt_sf.nonrd_prune_ref_frame_search = 0;
    rt_sf.use_nonrd_pick_mode = 0;
    rt_sf.use_nonrd_altref_frame = 0;
    rt_sf.use_comp_ref_nonrd = 0;
    rt_sf.use_real_time_ref_set = 0;
    rt_sf.short_circuit_low_temp_var = 0;
    rt_sf.reuse_inter_pred_nonrd = 0;
    rt_sf.num_inter_modes_for_tx_search = i32::MAX;
    rt_sf.use_nonrd_filter_search = 0;
    rt_sf.use_simple_rd_model = 0;
    rt_sf.skip_interp_filter_search = 0;
    rt_sf.hybrid_intra_pickmode = 0;
    rt_sf.source_metrics_sb_nonrd = 0;
    rt_sf.overshoot_detection_cbr = NO_DETECTION;
    rt_sf.check_scene_detection = 0;
    rt_sf.prefer_large_partition_blocks = 0;
    rt_sf.use_temporal_noise_estimate = 0;
    rt_sf.fullpel_search_step_param = 0;
    for i in 0..BLOCK_SIZES as usize {
        rt_sf.intra_y_mode_bsize_mask_nrd[i] = INTRA_ALL;
    }
    rt_sf.prune_hv_pred_modes_using_src_sad = false;
    rt_sf.nonrd_aggressive_skip = 0;
    rt_sf.skip_cdef_sb = 0;
    rt_sf.force_large_partition_blocks_intra = 0;
    rt_sf.skip_tx_no_split_var_based_partition = 0;
    rt_sf.skip_newmv_mode_based_on_sse = 0;
    rt_sf.gf_length_lvl = 0;
    rt_sf.prune_inter_modes_with_golden_ref = 0;
    rt_sf.prune_inter_modes_wrt_gf_arf_based_on_sad = 0;
    rt_sf.prune_inter_modes_using_temp_var = 0;
    rt_sf.reduce_mv_pel_precision_highmotion = 0;
    rt_sf.reduce_mv_pel_precision_lowcomplex = 0;
    rt_sf.prune_intra_mode_based_on_mv_range = 0;
    rt_sf.var_part_split_threshold_shift = 7;
    rt_sf.gf_refresh_based_on_qp = 0;
    rt_sf.use_rtc_tf = 0;
    rt_sf.prune_idtx_nonrd = 0;
    rt_sf.part_early_exit_zeromv = 0;
    rt_sf.sse_early_term_inter_search = EARLY_TERM_DISABLED;
    rt_sf.skip_lf_screen = 0;
    rt_sf.sad_based_adp_altref_lag = 0;
    rt_sf.partition_direct_merging = 0;
    rt_sf.var_part_based_on_qidx = 0;
    rt_sf.tx_size_level_based_on_qstep = 0;
    rt_sf.vbp_prune_16x16_split_using_min_max_sub_blk_var = false;
    rt_sf.prune_compoundmode_with_singlecompound_var = false;
    rt_sf.frame_level_mode_cost_update = false;
    rt_sf.prune_h_pred_using_best_mode_so_far = false;
    rt_sf.check_only_zero_zeromv_on_large_blocks = false;
    rt_sf.disable_cdf_update_non_reference_frame = false;
    rt_sf.prune_compoundmode_with_singlemode_var = false;
    rt_sf.skip_compound_based_on_var = false;
    rt_sf.set_zeromv_skip_based_on_source_sad = 1;
    rt_sf.use_adaptive_subpel_search = false;
    rt_sf.screen_content_cdef_filter_qindex_thresh = 0;
    rt_sf.enable_ref_short_signaling = false;
    rt_sf.check_globalmv_on_single_ref = true;
}

// Populate appropriate sub-pel search method based on speed feature and user
// specified settings
// (mv_search_params->find_fractional_mv_step: the method itself; no motion
// vector unit test)
fn set_subpel_search_method(
    mv_search_params: &mut MotionVectorSearchParams,
    motion_vector_unit_test: u32,
    subpel_search_method: i32,
) {
    debug_assert!(
        subpel_search_method == SUBPEL_TREE
            || subpel_search_method == SUBPEL_TREE_PRUNED
            || subpel_search_method == SUBPEL_TREE_PRUNED_MORE
    );
    mv_search_params.find_fractional_mv_step = subpel_search_method;

    // This is only used in motion vector unit test.
    debug_assert!(motion_vector_unit_test == 0);
}

pub(crate) fn av1_set_speed_features_framesize_dependent(cpi: &mut Av1Comp, speed: i32) {
    let mut sf = cpi.sf;

    match cpi.oxcf.mode {
        ALLINTRA => set_allintra_speed_feature_framesize_dependent(cpi, &mut sf, speed),
        REALTIME => set_rt_speed_feature_framesize_dependent(cpi, &mut sf, speed),
        // (GOOD: the good quality usage is not translated)
        _ => {}
    }

    if !cpi.ppi.seq_params_locked {
        let seq_params = &mut cpi.common.seq_params;
        seq_params.enable_masked_compound &= (sf.inter_sf.disable_masked_comp == 0) as u8;
        seq_params.enable_interintra_compound &=
            (sf.inter_sf.disable_interintra_wedge_var_thresh != u32::MAX) as u8;
    }

    set_subpel_search_method(&mut cpi.mv_search_params, 0, sf.mv_sf.subpel_search_method);

    // For multi-thread use case with row_mt enabled, cost update for a set of
    // SB rows is not desirable. Hence, the sf mv_cost_upd_level is set to
    // INTERNAL_COST_UPD_SBROW in such cases.
    if cpi.oxcf.row_mt && cpi.mt_info.num_workers > 1 {
        if sf.inter_sf.mv_cost_upd_level == INTERNAL_COST_UPD_SBROW_SET {
            // Set mv_cost_upd_level to use row level update.
            sf.inter_sf.mv_cost_upd_level = INTERNAL_COST_UPD_SBROW;
        }
    }
    cpi.sf = sf;
}

pub(crate) fn av1_set_speed_features_framesize_independent(cpi: &mut Av1Comp, speed: i32) {
    let mut sf = cpi.sf;

    init_hl_sf(&mut sf.hl_sf);
    init_fp_sf(&mut sf.fp_sf);
    init_tpl_sf(&mut sf.tpl_sf);
    init_gm_sf(&mut sf.gm_sf);
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

    match cpi.oxcf.mode {
        ALLINTRA => set_allintra_speed_features_framesize_independent(cpi, &mut sf, speed),
        REALTIME => set_rt_speed_features_framesize_independent(cpi, &mut sf, speed),
        // (GOOD: the good quality usage is not translated)
        _ => {}
    }

    // Note: when use_nonrd_pick_mode is true, the transform size is the
    // minimum of 16x16 and the largest possible size of the current block,
    // which conflicts with the speed feature "enable_tx_size_search".
    if !cpi.oxcf.txfm_cfg.enable_tx_size_search && sf.rt_sf.use_nonrd_pick_mode == 0 {
        sf.winner_mode_sf.tx_size_search_level = 3;
    }

    if !cpi.ppi.seq_params_locked {
        let seq_params = &mut cpi.common.seq_params;
        seq_params.order_hint_info.enable_dist_wtd_comp &=
            (sf.inter_sf.use_dist_wtd_comp_flag != DIST_WTD_COMP_DISABLED) as i32;
        seq_params.enable_dual_filter &= (sf.interp_sf.disable_dual_filter == 0) as u8;
        seq_params.enable_restoration &= (sf.lpf_sf.disable_lr_filter == 0) as u8;

        seq_params.enable_interintra_compound &=
            (sf.inter_sf.disable_interintra_wedge_var_thresh != u32::MAX) as u8;
    }

    let mesh_speed = speed.min(MAX_MESH_SPEED) as usize;
    for i in 0..MAX_MESH_STEP as usize {
        sf.mv_sf.mesh_patterns[i].range = GOOD_QUALITY_MESH_PATTERNS[mesh_speed][i].range;
        sf.mv_sf.mesh_patterns[i].interval = GOOD_QUALITY_MESH_PATTERNS[mesh_speed][i].interval;
    }

    // Update the mesh pattern of exhaustive motion search for intraBC
    // Though intraBC mesh pattern is populated for all frame types, it is used
    // only for intra frames of screen contents
    for i in 0..MAX_MESH_STEP as usize {
        sf.mv_sf.intrabc_mesh_patterns[i].range = INTRABC_MESH_PATTERNS[mesh_speed][i].range;
        sf.mv_sf.intrabc_mesh_patterns[i].interval = INTRABC_MESH_PATTERNS[mesh_speed][i].interval;
    }

    // Slow quant, dct and trellis not worthwhile for first pass
    // so make sure they are always turned off.
    if is_stat_generation_stage(cpi) {
        sf.rd_sf.optimize_coefficients = NO_TRELLIS_OPT;
    }

    // No recode for 1 pass.
    if cpi.oxcf.pass == AOM_RC_ONE_PASS && has_no_stats_stage(cpi) {
        sf.hl_sf.recode_loop = DISALLOW_RECODE;
    }

    set_subpel_search_method(&mut cpi.mv_search_params, 0, sf.mv_sf.subpel_search_method);

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

    if cpi.oxcf.row_mt && cpi.mt_info.num_workers > 1 {
        if sf.inter_sf.inter_mode_rd_model_estimation == 1 {
            // Revert to type 2
            sf.inter_sf.inter_mode_rd_model_estimation = 2;
        }

        // Disable the speed feature 'prune_ref_frame_for_gm_search' to achieve
        // better parallelism when number of threads available are greater than or
        // equal to maximum number of reference frames allowed for global motion.
        if sf.gm_sf.gm_search_type != GM_DISABLE_SEARCH
            && cpi.mt_info.num_workers
                >= GM_AVAILABLE_REFERENCE_FRAMES[sf.gm_sf.gm_search_type as usize]
        {
            sf.gm_sf.prune_ref_frame_for_gm_search = 0;
        }
    }

    // This only applies to the real time mode. Adaptive gf refresh is disabled if
    // gf_cbr_boost_pct that is set by the user is larger than 0.
    if cpi.oxcf.rc_cfg.gf_cbr_boost_pct > 0 {
        sf.rt_sf.gf_refresh_based_on_qp = 0;
    }
    cpi.sf = sf;
}

// Override some speed features based on qindex
pub(crate) fn av1_set_speed_features_qindex_dependent(cpi: &mut Av1Comp, speed: i32) {
    let mut sf = cpi.sf;
    let cm = &cpi.common;
    let boosted = frame_is_boosted(cpi);
    let is_480p_or_larger = cm.width.min(cm.height) >= 480;
    let is_720p_or_larger = cm.width.min(cm.height) >= 720;
    let is_1080p_or_larger = cm.width.min(cm.height) >= 1080;
    let is_arf2_bwd_type =
        cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize] == INTNL_ARF_UPDATE;

    if cpi.oxcf.mode == REALTIME {
        if speed >= 6 {
            let qindex_thresh = if boosted {
                190
            } else if is_720p_or_larger {
                120
            } else {
                150
            };
            sf.part_sf.adjust_var_based_rd_partitioning = if frame_is_intra_only(cm) {
                0
            } else {
                (cm.quant_params.base_qindex > qindex_thresh) as i32
            };
        }
        cpi.sf = sf;
        return;
    }

    if speed == 0 {
        // qindex_thresh for resolution < 720p
        let qindex_thresh = if boosted {
            70
        } else if is_arf2_bwd_type {
            110
        } else {
            140
        };
        if !is_720p_or_larger && cm.quant_params.base_qindex <= qindex_thresh {
            sf.part_sf.simple_motion_search_split = if cm.features.allow_screen_content_tools {
                1
            } else {
                2
            };
            sf.part_sf.simple_motion_search_early_term_none = 1;
            sf.tx_sf.model_based_prune_tx_search_level = 0;
        }

        if is_720p_or_larger && cm.quant_params.base_qindex <= 128 {
            sf.rd_sf.perform_coeff_opt = 2 + is_1080p_or_larger as i32;
            cpi.winner_mode_params.coeff_opt_thresholds =
                COEFF_OPT_THRESHOLDS[sf.rd_sf.perform_coeff_opt as usize];
            sf.part_sf.simple_motion_search_split = if cm.features.allow_screen_content_tools {
                1
            } else {
                2
            };
            sf.tx_sf.inter_tx_size_search_init_depth_rect = 1;
            sf.tx_sf.inter_tx_size_search_init_depth_sqr = 1;
            sf.tx_sf.intra_tx_size_search_init_depth_rect = 1;
            sf.tx_sf.model_based_prune_tx_search_level = 0;

            if is_1080p_or_larger && cm.quant_params.base_qindex <= 108 {
                sf.inter_sf.selective_ref_frame = 2;
                sf.rd_sf.tx_domain_dist_level = if boosted { 1 } else { 2 };
                sf.rd_sf.tx_domain_dist_thres_level = 1;
                sf.part_sf.simple_motion_search_early_term_none = 1;
                sf.tx_sf.tx_type_search.ml_tx_split_thresh = 4000;
                sf.interp_sf.cb_pred_filter_search = 0;
                sf.tx_sf.tx_type_search.prune_2d_txfm_mode = TX_TYPE_PRUNE_2;
                sf.tx_sf.tx_type_search.skip_tx_search = 1;
            }
        }
    }

    if speed >= 2 {
        // Disable extended partitions for lower quantizers
        let aggr = 3.min(speed - 2) as usize;
        let qindex_thresh1: [i32; 4] = [50, 50, 80, 100];
        let qindex_thresh2: [i32; 4] = [80, 100, 120, 160];
        let qindex_thresh;
        let disable_ext_part;
        if aggr <= 1 {
            let qthresh2 = if aggr == 0 && !is_480p_or_larger {
                70
            } else {
                qindex_thresh2[aggr]
            };
            qindex_thresh = if cm.features.allow_screen_content_tools {
                qindex_thresh1[aggr]
            } else {
                qthresh2
            };
            disable_ext_part = !boosted;
        } else {
            qindex_thresh = if boosted {
                qindex_thresh1[aggr]
            } else {
                qindex_thresh2[aggr]
            };
            disable_ext_part = !frame_is_intra_only(cm);
        }
        if cm.quant_params.base_qindex <= qindex_thresh && disable_ext_part {
            sf.part_sf.ext_partition_eval_thresh = BLOCK_128X128;
        }
    }

    if speed >= 4 {
        // Disable rectangular partitions for lower quantizers
        let aggr = 1.min(speed - 4) as usize;
        let qindex_thresh: [i32; 2] = [65, 80];
        let disable_rect_part = !boosted;
        if cm.quant_params.base_qindex <= qindex_thresh[aggr]
            && disable_rect_part
            && is_480p_or_larger
        {
            sf.part_sf.rect_partition_eval_thresh = BLOCK_8X8;
        }
    }

    if speed <= 2 {
        if !is_stat_generation_stage(cpi) {
            // Use faster full-pel motion search for high quantizers.
            // Also use reduced total search range for low resolutions at high
            // quantizers.
            let aggr = speed as usize;
            let qindex_thresh1 = MS_QINDEX_THRESH[aggr][is_720p_or_larger as usize][0];
            let qindex_thresh2 = MS_QINDEX_THRESH[aggr][is_720p_or_larger as usize][1];
            let search_method = MOTION_SEARCH_METHOD[is_720p_or_larger as usize];
            if cm.quant_params.base_qindex > qindex_thresh1 {
                sf.mv_sf.search_method = search_method;
                sf.tpl_sf.search_method = search_method;
            } else if cm.quant_params.base_qindex > qindex_thresh2 {
                sf.mv_sf.search_method = NSTEP_8PT;
            }
        }
    }

    if speed >= 4 {
        // Disable LR search at low and high quantizers and enable only for
        // mid-quantizer range.
        if !boosted && !is_arf2_bwd_type {
            let qindex_low: [i32; 2] = [100, 60];
            let qindex_high: [i32; 2] = [180, 160];
            if cm.quant_params.base_qindex <= qindex_low[is_720p_or_larger as usize]
                || cm.quant_params.base_qindex > qindex_high[is_720p_or_larger as usize]
            {
                sf.lpf_sf.disable_loop_restoration_luma = 1;
            }
        }
    }

    if speed == 1 {
        // Reuse interinter wedge mask search from first search for non-boosted
        // non-internal-arf frames, except at very high quantizers.
        if cm.quant_params.base_qindex <= 200 {
            if !boosted && !is_arf2_bwd_type {
                sf.inter_sf.reuse_mask_search_results = 1;
            }
        }
    }

    set_subpel_search_method(&mut cpi.mv_search_params, 0, sf.mv_sf.subpel_search_method);
    cpi.sf = sf;
}
