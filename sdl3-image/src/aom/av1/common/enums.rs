// Rust translation of av1/common/enums.h, av1/common/common_data.h and the
// enums of aom_dsp/txfm_common.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The block, transform and mode enumerations (as `i32` constants, the C
//! enums' values) and the lookup tables over them.

pub(crate) type BlockSize = i32;
pub(crate) type PartitionType = i32;
pub(crate) type TxMode = i32;
pub(crate) type TxType1D = i32;
pub(crate) type PredictionMode = i32;
pub(crate) type UvPredictionMode = i32;
pub(crate) type MotionMode = i32;
pub(crate) type FilterIntraMode = i32;
pub(crate) type Av1Level = i32;
pub(crate) type TxSize = i32;
pub(crate) type TxType = i32;
pub(crate) type TxSetType = i32;
pub(crate) type PartitionContext = i8;
pub(crate) type TxfmContext = u8;
pub(crate) type MvReferenceFrame = i32;

pub(crate) const SCALE_NUMERATOR: i32 = 8;

pub(crate) const MAX_SB_SIZE_LOG2: i32 = 7;
pub(crate) const MAX_SB_SIZE: i32 = 1 << MAX_SB_SIZE_LOG2;
pub(crate) const MAX_SB_SQUARE: i32 = MAX_SB_SIZE * MAX_SB_SIZE;
pub(crate) const MIN_SB_SIZE_LOG2: i32 = 6;
pub(crate) const MI_SIZE_LOG2: i32 = 2;
pub(crate) const MI_SIZE: i32 = 1 << MI_SIZE_LOG2;
pub(crate) const MAX_MIB_SIZE_LOG2: i32 = MAX_SB_SIZE_LOG2 - MI_SIZE_LOG2;
pub(crate) const MAX_MIB_SIZE: i32 = 1 << MAX_MIB_SIZE_LOG2;
pub(crate) const MIN_MIB_SIZE_LOG2: i32 = MIN_SB_SIZE_LOG2 - MI_SIZE_LOG2;
pub(crate) const MAX_MIB_MASK: i32 = MAX_MIB_SIZE - 1;
pub(crate) const MAX_TILE_ROWS: i32 = 64;
pub(crate) const MAX_TILE_COLS: i32 = 64;
pub(crate) const MAX_VARTX_DEPTH: i32 = 2;
pub(crate) const MI_SIZE_64X64: i32 = 64 >> MI_SIZE_LOG2;
pub(crate) const MI_SIZE_128X128: i32 = 128 >> MI_SIZE_LOG2;
pub(crate) const MAX_PALETTE_SQUARE: i32 = 64 * 64;
pub(crate) const PALETTE_MAX_SIZE: i32 = 8;
pub(crate) const PALETTE_MIN_SIZE: i32 = 2;
pub(crate) const FRAME_OFFSET_BITS: i32 = 5;
pub(crate) const MAX_FRAME_DISTANCE: i32 = (1 << FRAME_OFFSET_BITS) - 1;
pub(crate) const FRAME_LF_COUNT: i32 = 4;
pub(crate) const DEFAULT_DELTA_LF_MULTI: i32 = 0;
pub(crate) const MAX_MODE_LF_DELTAS: i32 = 2;
pub(crate) const DIST_PRECISION_BITS: i32 = 4;
pub(crate) const DIST_PRECISION: i32 = 1 << DIST_PRECISION_BITS;
pub(crate) const PROFILE_BITS: i32 = 3;
// BITSTREAM_PROFILE
pub(crate) const PROFILE_0: i32 = 0;
pub(crate) const PROFILE_1: i32 = PROFILE_0 + 1;
pub(crate) const PROFILE_2: i32 = PROFILE_1 + 1;
pub(crate) const MAX_PROFILES: i32 = PROFILE_2 + 1;
pub(crate) const OP_POINTS_CNT_MINUS_1_BITS: i32 = 5;
pub(crate) const OP_POINTS_IDC_BITS: i32 = 12;
// BLOCK_SIZE
pub(crate) const BLOCK_4X4: BlockSize = 0;
pub(crate) const BLOCK_4X8: BlockSize = BLOCK_4X4 + 1;
pub(crate) const BLOCK_8X4: BlockSize = BLOCK_4X8 + 1;
pub(crate) const BLOCK_8X8: BlockSize = BLOCK_8X4 + 1;
pub(crate) const BLOCK_8X16: BlockSize = BLOCK_8X8 + 1;
pub(crate) const BLOCK_16X8: BlockSize = BLOCK_8X16 + 1;
pub(crate) const BLOCK_16X16: BlockSize = BLOCK_16X8 + 1;
pub(crate) const BLOCK_16X32: BlockSize = BLOCK_16X16 + 1;
pub(crate) const BLOCK_32X16: BlockSize = BLOCK_16X32 + 1;
pub(crate) const BLOCK_32X32: BlockSize = BLOCK_32X16 + 1;
pub(crate) const BLOCK_32X64: BlockSize = BLOCK_32X32 + 1;
pub(crate) const BLOCK_64X32: BlockSize = BLOCK_32X64 + 1;
pub(crate) const BLOCK_64X64: BlockSize = BLOCK_64X32 + 1;
pub(crate) const BLOCK_64X128: BlockSize = BLOCK_64X64 + 1;
pub(crate) const BLOCK_128X64: BlockSize = BLOCK_64X128 + 1;
pub(crate) const BLOCK_128X128: BlockSize = BLOCK_128X64 + 1;
pub(crate) const BLOCK_4X16: BlockSize = BLOCK_128X128 + 1;
pub(crate) const BLOCK_16X4: BlockSize = BLOCK_4X16 + 1;
pub(crate) const BLOCK_8X32: BlockSize = BLOCK_16X4 + 1;
pub(crate) const BLOCK_32X8: BlockSize = BLOCK_8X32 + 1;
pub(crate) const BLOCK_16X64: BlockSize = BLOCK_32X8 + 1;
pub(crate) const BLOCK_64X16: BlockSize = BLOCK_16X64 + 1;
pub(crate) const BLOCK_SIZES_ALL: BlockSize = BLOCK_64X16 + 1;
pub(crate) const BLOCK_SIZES: BlockSize = BLOCK_4X16;
pub(crate) const BLOCK_INVALID: BlockSize = 255;
pub(crate) const BLOCK_LARGEST: BlockSize = BLOCK_SIZES - 1;
pub(crate) const SQR_BLOCK_SIZES: i32 = 6;
// PARTITION_TYPE
pub(crate) const PARTITION_NONE: PartitionType = 0;
pub(crate) const PARTITION_HORZ: PartitionType = PARTITION_NONE + 1;
pub(crate) const PARTITION_VERT: PartitionType = PARTITION_HORZ + 1;
pub(crate) const PARTITION_SPLIT: PartitionType = PARTITION_VERT + 1;
pub(crate) const PARTITION_HORZ_A: PartitionType = PARTITION_SPLIT + 1;
pub(crate) const PARTITION_HORZ_B: PartitionType = PARTITION_HORZ_A + 1;
pub(crate) const PARTITION_VERT_A: PartitionType = PARTITION_HORZ_B + 1;
pub(crate) const PARTITION_VERT_B: PartitionType = PARTITION_VERT_A + 1;
pub(crate) const PARTITION_HORZ_4: PartitionType = PARTITION_VERT_B + 1;
pub(crate) const PARTITION_VERT_4: PartitionType = PARTITION_HORZ_4 + 1;
pub(crate) const EXT_PARTITION_TYPES: PartitionType = PARTITION_VERT_4 + 1;
pub(crate) const PARTITION_TYPES: PartitionType = PARTITION_SPLIT + 1;
pub(crate) const PARTITION_INVALID: PartitionType = 255;
pub(crate) const PARTITION_PLOFFSET: i32 = 4;
pub(crate) const PARTITION_BLOCK_SIZES: i32 = 5;
pub(crate) const PARTITION_CONTEXTS: i32 = PARTITION_BLOCK_SIZES * PARTITION_PLOFFSET;
pub(crate) const TX_SIZE_LUMA_MIN: i32 = TX_4X4;
pub(crate) const TX_SIZE_CTX_MIN: i32 = TX_SIZE_LUMA_MIN + 1;
pub(crate) const MAX_TX_CATS: i32 = TX_SIZES - TX_SIZE_CTX_MIN;
pub(crate) const MAX_TX_DEPTH: i32 = 2;
pub(crate) const MAX_TX_SIZE_LOG2: i32 = 6;
pub(crate) const MAX_TX_SIZE: i32 = 1 << MAX_TX_SIZE_LOG2;
pub(crate) const MIN_TX_SIZE_LOG2: i32 = 2;
pub(crate) const MIN_TX_SIZE: i32 = 1 << MIN_TX_SIZE_LOG2;
pub(crate) const MAX_TX_SQUARE: i32 = MAX_TX_SIZE * MAX_TX_SIZE;
pub(crate) const TX_PAD_HOR_LOG2: i32 = 2;
pub(crate) const TX_PAD_HOR: i32 = 4;
pub(crate) const TX_PAD_TOP: i32 = 0;
pub(crate) const TX_PAD_BOTTOM: i32 = 4;
pub(crate) const TX_PAD_VER: i32 = TX_PAD_TOP + TX_PAD_BOTTOM;
pub(crate) const TX_PAD_END: i32 = 16;
pub(crate) const TX_PAD_2D: i32 = (32 + TX_PAD_HOR) * (32 + TX_PAD_VER) + TX_PAD_END;
pub(crate) const MAX_TX_BLOCKS_IN_MAX_SB_LOG2: i32 = (MAX_SB_SIZE_LOG2 - MAX_TX_SIZE_LOG2) * 2;
pub(crate) const MAX_TX_BLOCKS_IN_MAX_SB: i32 = 1 << MAX_TX_BLOCKS_IN_MAX_SB_LOG2;
// TX_MODE
pub(crate) const ONLY_4X4: TxMode = 0;
pub(crate) const TX_MODE_LARGEST: TxMode = ONLY_4X4 + 1;
pub(crate) const TX_MODE_SELECT: TxMode = TX_MODE_LARGEST + 1;
pub(crate) const TX_MODES: TxMode = TX_MODE_SELECT + 1;
// TX_TYPE_1D
pub(crate) const DCT_1D: TxType1D = 0;
pub(crate) const ADST_1D: TxType1D = DCT_1D + 1;
pub(crate) const FLIPADST_1D: TxType1D = ADST_1D + 1;
pub(crate) const IDTX_1D: TxType1D = FLIPADST_1D + 1;
pub(crate) const TX_TYPES_1D: TxType1D = IDTX_1D + 1;
// DUAL_FILTER_TYPE
pub(crate) const REG_REG: i32 = 0;
pub(crate) const REG_SMOOTH: i32 = REG_REG + 1;
pub(crate) const REG_SHARP: i32 = REG_SMOOTH + 1;
pub(crate) const SMOOTH_REG: i32 = REG_SHARP + 1;
pub(crate) const SMOOTH_SMOOTH: i32 = SMOOTH_REG + 1;
pub(crate) const SMOOTH_SHARP: i32 = SMOOTH_SMOOTH + 1;
pub(crate) const SHARP_REG: i32 = SMOOTH_SHARP + 1;
pub(crate) const SHARP_SMOOTH: i32 = SHARP_REG + 1;
pub(crate) const SHARP_SHARP: i32 = SHARP_SMOOTH + 1;
pub(crate) const EXT_TX_SIZES: i32 = 4;
pub(crate) const EXT_TX_SETS_INTER: i32 = 4;
pub(crate) const EXT_TX_SETS_INTRA: i32 = 3;
// AOM_REFFRAME
pub(crate) const AOM_LAST_FLAG: i32 = 1 << 0;
pub(crate) const AOM_LAST2_FLAG: i32 = 1 << 1;
pub(crate) const AOM_LAST3_FLAG: i32 = 1 << 2;
pub(crate) const AOM_GOLD_FLAG: i32 = 1 << 3;
pub(crate) const AOM_BWD_FLAG: i32 = 1 << 4;
pub(crate) const AOM_ALT2_FLAG: i32 = 1 << 5;
pub(crate) const AOM_ALT_FLAG: i32 = 1 << 6;
pub(crate) const AOM_REFFRAME_ALL: i32 = (1 << 7) - 1;
// COMP_REFERENCE_TYPE
pub(crate) const UNIDIR_COMP_REFERENCE: i32 = 0;
pub(crate) const BIDIR_COMP_REFERENCE: i32 = UNIDIR_COMP_REFERENCE + 1;
pub(crate) const COMP_REFERENCE_TYPES: i32 = BIDIR_COMP_REFERENCE + 1;
// PLANE_TYPE
pub(crate) const PLANE_TYPE_Y: i32 = 0;
pub(crate) const PLANE_TYPE_UV: i32 = PLANE_TYPE_Y + 1;
pub(crate) const PLANE_TYPES: i32 = PLANE_TYPE_UV + 1;
pub(crate) const CFL_ALPHABET_SIZE_LOG2: i32 = 4;
pub(crate) const CFL_ALPHABET_SIZE: i32 = 1 << CFL_ALPHABET_SIZE_LOG2;
pub(crate) const CFL_MAGS_SIZE: i32 = (2 << CFL_ALPHABET_SIZE_LOG2) + 1;
pub(crate) const CFL_INDEX_ZERO: i32 = CFL_ALPHABET_SIZE;
// CFL_PRED_TYPE
pub(crate) const CFL_PRED_U: i32 = 0;
pub(crate) const CFL_PRED_V: i32 = CFL_PRED_U + 1;
pub(crate) const CFL_PRED_PLANES: i32 = CFL_PRED_V + 1;
// CFL_SIGN_TYPE
pub(crate) const CFL_SIGN_ZERO: i32 = 0;
pub(crate) const CFL_SIGN_NEG: i32 = CFL_SIGN_ZERO + 1;
pub(crate) const CFL_SIGN_POS: i32 = CFL_SIGN_NEG + 1;
pub(crate) const CFL_SIGNS: i32 = CFL_SIGN_POS + 1;
// CFL_ALLOWED_TYPE
pub(crate) const CFL_DISALLOWED: i32 = 0;
pub(crate) const CFL_ALLOWED: i32 = CFL_DISALLOWED + 1;
pub(crate) const CFL_ALLOWED_TYPES: i32 = CFL_ALLOWED + 1;
pub(crate) const CFL_JOINT_SIGNS: i32 = CFL_SIGNS * CFL_SIGNS - 1;
pub(crate) const CFL_ALPHA_CONTEXTS: i32 = CFL_JOINT_SIGNS + 1 - CFL_SIGNS;

#[inline]
pub(crate) const fn cfl_idx_u(idx: i32) -> i32 {
    idx >> CFL_ALPHABET_SIZE_LOG2
}
#[inline]
pub(crate) const fn cfl_idx_v(idx: i32) -> i32 {
    idx & (CFL_ALPHABET_SIZE - 1)
}
// CFL_SIGN_U is equivalent to (js + 1) / 3 for js in 0 to 8
#[inline]
pub(crate) const fn cfl_sign_u(js: i32) -> i32 {
    ((js + 1) * 11) >> 5
}
// CFL_SIGN_V is equivalent to (js + 1) % 3 for js in 0 to 8
#[inline]
pub(crate) const fn cfl_sign_v(js: i32) -> i32 {
    (js + 1) - CFL_SIGNS * cfl_sign_u(js)
}
// There is no context when the alpha for a given plane is zero.
// So there are 2 fewer contexts than joint signs.
#[inline]
pub(crate) const fn cfl_context_u(js: i32) -> i32 {
    js + 1 - CFL_SIGNS
}
// Also, the contexts are symmetric under swapping the planes.
#[inline]
pub(crate) const fn cfl_context_v(js: i32) -> i32 {
    cfl_sign_v(js) * CFL_SIGNS + cfl_sign_u(js) - CFL_SIGNS
}
// COLOR_MAP_TYPE
pub(crate) const PALETTE_MAP: i32 = 0;
pub(crate) const COLOR_MAP_TYPES: i32 = PALETTE_MAP + 1;
// PALETTE_SIZE
pub(crate) const TWO_COLORS: i32 = 0;
pub(crate) const THREE_COLORS: i32 = TWO_COLORS + 1;
pub(crate) const FOUR_COLORS: i32 = THREE_COLORS + 1;
pub(crate) const FIVE_COLORS: i32 = FOUR_COLORS + 1;
pub(crate) const SIX_COLORS: i32 = FIVE_COLORS + 1;
pub(crate) const SEVEN_COLORS: i32 = SIX_COLORS + 1;
pub(crate) const EIGHT_COLORS: i32 = SEVEN_COLORS + 1;
pub(crate) const PALETTE_SIZES: i32 = EIGHT_COLORS + 1;
// PALETTE_COLOR
pub(crate) const PALETTE_COLOR_ONE: i32 = 0;
pub(crate) const PALETTE_COLOR_TWO: i32 = PALETTE_COLOR_ONE + 1;
pub(crate) const PALETTE_COLOR_THREE: i32 = PALETTE_COLOR_TWO + 1;
pub(crate) const PALETTE_COLOR_FOUR: i32 = PALETTE_COLOR_THREE + 1;
pub(crate) const PALETTE_COLOR_FIVE: i32 = PALETTE_COLOR_FOUR + 1;
pub(crate) const PALETTE_COLOR_SIX: i32 = PALETTE_COLOR_FIVE + 1;
pub(crate) const PALETTE_COLOR_SEVEN: i32 = PALETTE_COLOR_SIX + 1;
pub(crate) const PALETTE_COLOR_EIGHT: i32 = PALETTE_COLOR_SEVEN + 1;
pub(crate) const PALETTE_COLORS: i32 = PALETTE_COLOR_EIGHT + 1;
// PREDICTION_MODE
pub(crate) const DC_PRED: PredictionMode = 0;
pub(crate) const V_PRED: PredictionMode = DC_PRED + 1;
pub(crate) const H_PRED: PredictionMode = V_PRED + 1;
pub(crate) const D45_PRED: PredictionMode = H_PRED + 1;
pub(crate) const D135_PRED: PredictionMode = D45_PRED + 1;
pub(crate) const D113_PRED: PredictionMode = D135_PRED + 1;
pub(crate) const D157_PRED: PredictionMode = D113_PRED + 1;
pub(crate) const D203_PRED: PredictionMode = D157_PRED + 1;
pub(crate) const D67_PRED: PredictionMode = D203_PRED + 1;
pub(crate) const SMOOTH_PRED: PredictionMode = D67_PRED + 1;
pub(crate) const SMOOTH_V_PRED: PredictionMode = SMOOTH_PRED + 1;
pub(crate) const SMOOTH_H_PRED: PredictionMode = SMOOTH_V_PRED + 1;
pub(crate) const PAETH_PRED: PredictionMode = SMOOTH_H_PRED + 1;
pub(crate) const NEARESTMV: PredictionMode = PAETH_PRED + 1;
pub(crate) const NEARMV: PredictionMode = NEARESTMV + 1;
pub(crate) const GLOBALMV: PredictionMode = NEARMV + 1;
pub(crate) const NEWMV: PredictionMode = GLOBALMV + 1;
pub(crate) const NEAREST_NEARESTMV: PredictionMode = NEWMV + 1;
pub(crate) const NEAR_NEARMV: PredictionMode = NEAREST_NEARESTMV + 1;
pub(crate) const NEAREST_NEWMV: PredictionMode = NEAR_NEARMV + 1;
pub(crate) const NEW_NEARESTMV: PredictionMode = NEAREST_NEWMV + 1;
pub(crate) const NEAR_NEWMV: PredictionMode = NEW_NEARESTMV + 1;
pub(crate) const NEW_NEARMV: PredictionMode = NEAR_NEWMV + 1;
pub(crate) const GLOBAL_GLOBALMV: PredictionMode = NEW_NEARMV + 1;
pub(crate) const NEW_NEWMV: PredictionMode = GLOBAL_GLOBALMV + 1;
pub(crate) const MB_MODE_COUNT: PredictionMode = NEW_NEWMV + 1;
pub(crate) const PRED_MODE_INVALID: PredictionMode = MB_MODE_COUNT;
pub(crate) const INTRA_MODE_START: PredictionMode = DC_PRED;
pub(crate) const INTRA_MODE_END: PredictionMode = NEARESTMV;
pub(crate) const DIR_MODE_START: PredictionMode = V_PRED;
pub(crate) const DIR_MODE_END: PredictionMode = D67_PRED + 1;
pub(crate) const INTRA_MODE_NUM: PredictionMode = INTRA_MODE_END - INTRA_MODE_START;
pub(crate) const SINGLE_INTER_MODE_START: PredictionMode = NEARESTMV;
pub(crate) const SINGLE_INTER_MODE_END: PredictionMode = NEAREST_NEARESTMV;
pub(crate) const SINGLE_INTER_MODE_NUM: PredictionMode =
    SINGLE_INTER_MODE_END - SINGLE_INTER_MODE_START;
pub(crate) const COMP_INTER_MODE_START: PredictionMode = NEAREST_NEARESTMV;
pub(crate) const COMP_INTER_MODE_END: PredictionMode = MB_MODE_COUNT;
pub(crate) const COMP_INTER_MODE_NUM: PredictionMode = COMP_INTER_MODE_END - COMP_INTER_MODE_START;
pub(crate) const INTER_MODE_START: PredictionMode = NEARESTMV;
pub(crate) const INTER_MODE_END: PredictionMode = MB_MODE_COUNT;
pub(crate) const INTRA_MODES: PredictionMode = PAETH_PRED + 1;
pub(crate) const INTRA_INVALID: PredictionMode = MB_MODE_COUNT;
// UV_PREDICTION_MODE
pub(crate) const UV_DC_PRED: UvPredictionMode = 0;
pub(crate) const UV_V_PRED: UvPredictionMode = UV_DC_PRED + 1;
pub(crate) const UV_H_PRED: UvPredictionMode = UV_V_PRED + 1;
pub(crate) const UV_D45_PRED: UvPredictionMode = UV_H_PRED + 1;
pub(crate) const UV_D135_PRED: UvPredictionMode = UV_D45_PRED + 1;
pub(crate) const UV_D113_PRED: UvPredictionMode = UV_D135_PRED + 1;
pub(crate) const UV_D157_PRED: UvPredictionMode = UV_D113_PRED + 1;
pub(crate) const UV_D203_PRED: UvPredictionMode = UV_D157_PRED + 1;
pub(crate) const UV_D67_PRED: UvPredictionMode = UV_D203_PRED + 1;
pub(crate) const UV_SMOOTH_PRED: UvPredictionMode = UV_D67_PRED + 1;
pub(crate) const UV_SMOOTH_V_PRED: UvPredictionMode = UV_SMOOTH_PRED + 1;
pub(crate) const UV_SMOOTH_H_PRED: UvPredictionMode = UV_SMOOTH_V_PRED + 1;
pub(crate) const UV_PAETH_PRED: UvPredictionMode = UV_SMOOTH_H_PRED + 1;
pub(crate) const UV_CFL_PRED: UvPredictionMode = UV_PAETH_PRED + 1;
pub(crate) const UV_INTRA_MODES: UvPredictionMode = UV_CFL_PRED + 1;
pub(crate) const UV_MODE_INVALID: UvPredictionMode = UV_INTRA_MODES + 1;
pub(crate) const TOP_INTRA_MODEL_COUNT: i32 = 4;
pub(crate) const LUMA_MODE_COUNT: i32 = PAETH_PRED - DC_PRED + 1 + 6 * 8;
// MOTION_MODE
pub(crate) const SIMPLE_TRANSLATION: MotionMode = 0;
pub(crate) const OBMC_CAUSAL: MotionMode = SIMPLE_TRANSLATION + 1;
pub(crate) const WARPED_CAUSAL: MotionMode = OBMC_CAUSAL + 1;
pub(crate) const MOTION_MODES: MotionMode = WARPED_CAUSAL + 1;
// INTERINTRA_MODE
pub(crate) const II_DC_PRED: i32 = 0;
pub(crate) const II_V_PRED: i32 = II_DC_PRED + 1;
pub(crate) const II_H_PRED: i32 = II_V_PRED + 1;
pub(crate) const II_SMOOTH_PRED: i32 = II_H_PRED + 1;
pub(crate) const INTERINTRA_MODES: i32 = II_SMOOTH_PRED + 1;
// COMPOUND_TYPE
pub(crate) const COMPOUND_AVERAGE: i32 = 0;
pub(crate) const COMPOUND_DISTWTD: i32 = COMPOUND_AVERAGE + 1;
pub(crate) const COMPOUND_WEDGE: i32 = COMPOUND_DISTWTD + 1;
pub(crate) const COMPOUND_DIFFWTD: i32 = COMPOUND_WEDGE + 1;
pub(crate) const COMPOUND_TYPES: i32 = COMPOUND_DIFFWTD + 1;
pub(crate) const MASKED_COMPOUND_TYPES: i32 = 2;
// FILTER_INTRA_MODE
pub(crate) const FILTER_DC_PRED: FilterIntraMode = 0;
pub(crate) const FILTER_V_PRED: FilterIntraMode = FILTER_DC_PRED + 1;
pub(crate) const FILTER_H_PRED: FilterIntraMode = FILTER_V_PRED + 1;
pub(crate) const FILTER_D157_PRED: FilterIntraMode = FILTER_H_PRED + 1;
pub(crate) const FILTER_PAETH_PRED: FilterIntraMode = FILTER_D157_PRED + 1;
pub(crate) const FILTER_INTRA_MODES: FilterIntraMode = FILTER_PAETH_PRED + 1;
// AV1_LEVEL
pub(crate) const SEQ_LEVEL_2_0: Av1Level = 0;
pub(crate) const SEQ_LEVEL_2_1: Av1Level = SEQ_LEVEL_2_0 + 1;
pub(crate) const SEQ_LEVEL_2_2: Av1Level = SEQ_LEVEL_2_1 + 1;
pub(crate) const SEQ_LEVEL_2_3: Av1Level = SEQ_LEVEL_2_2 + 1;
pub(crate) const SEQ_LEVEL_3_0: Av1Level = SEQ_LEVEL_2_3 + 1;
pub(crate) const SEQ_LEVEL_3_1: Av1Level = SEQ_LEVEL_3_0 + 1;
pub(crate) const SEQ_LEVEL_3_2: Av1Level = SEQ_LEVEL_3_1 + 1;
pub(crate) const SEQ_LEVEL_3_3: Av1Level = SEQ_LEVEL_3_2 + 1;
pub(crate) const SEQ_LEVEL_4_0: Av1Level = SEQ_LEVEL_3_3 + 1;
pub(crate) const SEQ_LEVEL_4_1: Av1Level = SEQ_LEVEL_4_0 + 1;
pub(crate) const SEQ_LEVEL_4_2: Av1Level = SEQ_LEVEL_4_1 + 1;
pub(crate) const SEQ_LEVEL_4_3: Av1Level = SEQ_LEVEL_4_2 + 1;
pub(crate) const SEQ_LEVEL_5_0: Av1Level = SEQ_LEVEL_4_3 + 1;
pub(crate) const SEQ_LEVEL_5_1: Av1Level = SEQ_LEVEL_5_0 + 1;
pub(crate) const SEQ_LEVEL_5_2: Av1Level = SEQ_LEVEL_5_1 + 1;
pub(crate) const SEQ_LEVEL_5_3: Av1Level = SEQ_LEVEL_5_2 + 1;
pub(crate) const SEQ_LEVEL_6_0: Av1Level = SEQ_LEVEL_5_3 + 1;
pub(crate) const SEQ_LEVEL_6_1: Av1Level = SEQ_LEVEL_6_0 + 1;
pub(crate) const SEQ_LEVEL_6_2: Av1Level = SEQ_LEVEL_6_1 + 1;
pub(crate) const SEQ_LEVEL_6_3: Av1Level = SEQ_LEVEL_6_2 + 1;
pub(crate) const SEQ_LEVEL_7_0: Av1Level = SEQ_LEVEL_6_3 + 1;
pub(crate) const SEQ_LEVEL_7_1: Av1Level = SEQ_LEVEL_7_0 + 1;
pub(crate) const SEQ_LEVEL_7_2: Av1Level = SEQ_LEVEL_7_1 + 1;
pub(crate) const SEQ_LEVEL_7_3: Av1Level = SEQ_LEVEL_7_2 + 1;
pub(crate) const SEQ_LEVEL_8_0: Av1Level = SEQ_LEVEL_7_3 + 1;
pub(crate) const SEQ_LEVEL_8_1: Av1Level = SEQ_LEVEL_8_0 + 1;
pub(crate) const SEQ_LEVEL_8_2: Av1Level = SEQ_LEVEL_8_1 + 1;
pub(crate) const SEQ_LEVEL_8_3: Av1Level = SEQ_LEVEL_8_2 + 1;
pub(crate) const SEQ_LEVELS: Av1Level = SEQ_LEVEL_8_3 + 1;
pub(crate) const SEQ_LEVEL_MAX: Av1Level = 31;
pub(crate) const SEQ_LEVEL_KEEP_STATS: Av1Level = 32;
pub(crate) const LEVEL_BITS: i32 = 5;
pub(crate) const DIRECTIONAL_MODES: i32 = 8;
pub(crate) const MAX_ANGLE_DELTA: i32 = 3;
pub(crate) const ANGLE_STEP: i32 = 3;
pub(crate) const INTER_MODES: i32 = 1 + NEWMV - NEARESTMV;
pub(crate) const INTER_COMPOUND_MODES: i32 = 1 + NEW_NEWMV - NEAREST_NEARESTMV;
pub(crate) const SKIP_CONTEXTS: i32 = 3;
pub(crate) const SKIP_MODE_CONTEXTS: i32 = 3;
pub(crate) const COMP_INDEX_CONTEXTS: i32 = 6;
pub(crate) const COMP_GROUP_IDX_CONTEXTS: i32 = 6;
pub(crate) const NMV_CONTEXTS: i32 = 3;
pub(crate) const NEWMV_MODE_CONTEXTS: i32 = 6;
pub(crate) const GLOBALMV_MODE_CONTEXTS: i32 = 2;
pub(crate) const REFMV_MODE_CONTEXTS: i32 = 6;
pub(crate) const DRL_MODE_CONTEXTS: i32 = 3;
pub(crate) const GLOBALMV_OFFSET: i32 = 3;
pub(crate) const REFMV_OFFSET: i32 = 4;
pub(crate) const NEWMV_CTX_MASK: i32 = (1 << GLOBALMV_OFFSET) - 1;
pub(crate) const GLOBALMV_CTX_MASK: i32 = (1 << (REFMV_OFFSET - GLOBALMV_OFFSET)) - 1;
pub(crate) const REFMV_CTX_MASK: i32 = (1 << (8 - REFMV_OFFSET)) - 1;
pub(crate) const COMP_NEWMV_CTXS: i32 = 5;
pub(crate) const INTER_MODE_CONTEXTS: i32 = 8;
pub(crate) const DELTA_Q_SMALL: i32 = 3;
pub(crate) const DELTA_Q_PROBS: i32 = DELTA_Q_SMALL;
pub(crate) const DEFAULT_DELTA_Q_RES_PERCEPTUAL: i32 = 4;
pub(crate) const DEFAULT_DELTA_Q_RES_OBJECTIVE: i32 = 4;
pub(crate) const DEFAULT_DELTA_Q_RES_DUCKY_ENCODE: i32 = 4;
pub(crate) const DELTA_LF_SMALL: i32 = 3;
pub(crate) const DELTA_LF_PROBS: i32 = DELTA_LF_SMALL;
pub(crate) const DEFAULT_DELTA_LF_RES: i32 = 2;
pub(crate) const MAX_MV_REF_CANDIDATES: i32 = 2;
pub(crate) const MAX_REF_MV_STACK_SIZE: i32 = 8;
pub(crate) const USABLE_REF_MV_STACK_SIZE: i32 = 4;
pub(crate) const REF_CAT_LEVEL: i32 = 640;
pub(crate) const INTRA_INTER_CONTEXTS: i32 = 4;
pub(crate) const COMP_INTER_CONTEXTS: i32 = 5;
pub(crate) const REF_CONTEXTS: i32 = 3;
pub(crate) const COMP_REF_TYPE_CONTEXTS: i32 = 5;
pub(crate) const UNI_COMP_REF_CONTEXTS: i32 = 3;
pub(crate) const TXFM_PARTITION_CONTEXTS: i32 = (TX_SIZES - TX_8X8) * 6 - 3;
pub(crate) const NONE_FRAME: i32 = -1;
pub(crate) const INTRA_FRAME: i32 = NONE_FRAME + 1;
pub(crate) const LAST_FRAME: i32 = INTRA_FRAME + 1;
pub(crate) const LAST2_FRAME: i32 = LAST_FRAME + 1;
pub(crate) const LAST3_FRAME: i32 = LAST2_FRAME + 1;
pub(crate) const GOLDEN_FRAME: i32 = LAST3_FRAME + 1;
pub(crate) const BWDREF_FRAME: i32 = GOLDEN_FRAME + 1;
pub(crate) const ALTREF2_FRAME: i32 = BWDREF_FRAME + 1;
pub(crate) const ALTREF_FRAME: i32 = ALTREF2_FRAME + 1;
pub(crate) const REF_FRAMES: i32 = ALTREF_FRAME + 1;
pub(crate) const EXTREF_FRAME: i32 = REF_FRAMES;
pub(crate) const INTER_REFS_PER_FRAME: i32 = ALTREF_FRAME - LAST_FRAME + 1;
pub(crate) const FWD_REFS: i32 = GOLDEN_FRAME - LAST_FRAME + 1;
pub(crate) const BWD_REFS: i32 = ALTREF_FRAME - BWDREF_FRAME + 1;
pub(crate) const SINGLE_REFS: i32 = FWD_REFS + BWD_REFS;
pub(crate) const REF_FRAMES_LOG2: i32 = 3;
pub(crate) const FRAME_BUFFERS: i32 = REF_FRAMES + 1 + INTER_REFS_PER_FRAME;
pub(crate) const SELECT_ALL_BUF_SLOTS: i32 = 0xFF;
// UNIDIR_COMP_REF
pub(crate) const LAST_LAST2_FRAMES: i32 = 0;
pub(crate) const LAST_LAST3_FRAMES: i32 = LAST_LAST2_FRAMES + 1;
pub(crate) const LAST_GOLDEN_FRAMES: i32 = LAST_LAST3_FRAMES + 1;
pub(crate) const BWDREF_ALTREF_FRAMES: i32 = LAST_GOLDEN_FRAMES + 1;
pub(crate) const LAST2_LAST3_FRAMES: i32 = BWDREF_ALTREF_FRAMES + 1;
pub(crate) const LAST2_GOLDEN_FRAMES: i32 = LAST2_LAST3_FRAMES + 1;
pub(crate) const LAST3_GOLDEN_FRAMES: i32 = LAST2_GOLDEN_FRAMES + 1;
pub(crate) const BWDREF_ALTREF2_FRAMES: i32 = LAST3_GOLDEN_FRAMES + 1;
pub(crate) const ALTREF2_ALTREF_FRAMES: i32 = BWDREF_ALTREF2_FRAMES + 1;
pub(crate) const TOTAL_UNIDIR_COMP_REFS: i32 = ALTREF2_ALTREF_FRAMES + 1;
pub(crate) const UNIDIR_COMP_REFS: i32 = BWDREF_ALTREF_FRAMES + 1;
pub(crate) const TOTAL_COMP_REFS: i32 = FWD_REFS * BWD_REFS + TOTAL_UNIDIR_COMP_REFS;
pub(crate) const COMP_REFS: i32 = FWD_REFS * BWD_REFS + UNIDIR_COMP_REFS;
pub(crate) const MODE_CTX_REF_FRAMES: i32 = REF_FRAMES + TOTAL_COMP_REFS;
// RestorationType
pub(crate) const RESTORE_NONE: i32 = 0;
pub(crate) const RESTORE_WIENER: i32 = RESTORE_NONE + 1;
pub(crate) const RESTORE_SGRPROJ: i32 = RESTORE_WIENER + 1;
pub(crate) const RESTORE_SWITCHABLE: i32 = RESTORE_SGRPROJ + 1;
pub(crate) const RESTORE_SWITCHABLE_TYPES: i32 = RESTORE_SWITCHABLE;
pub(crate) const RESTORE_TYPES: i32 = 4;
pub(crate) const SUPERRES_SCALE_BITS: i32 = 3;
pub(crate) const SUPERRES_SCALE_DENOMINATOR_MIN: i32 = SCALE_NUMERATOR + 1;
pub(crate) const MAX_EXTERNAL_REFERENCES: i32 = 128;
pub(crate) const MAX_TILES: i32 = 512;

pub(crate) const DCT_CONST_BITS: i32 = 14;
pub(crate) const DCT_CONST_ROUNDING: i32 = 1 << (DCT_CONST_BITS - 1);
pub(crate) const UNIT_QUANT_SHIFT: i32 = 2;
pub(crate) const UNIT_QUANT_FACTOR: i32 = 1 << UNIT_QUANT_SHIFT;
// TX_SIZE
pub(crate) const TX_4X4: TxSize = 0;
pub(crate) const TX_8X8: TxSize = TX_4X4 + 1;
pub(crate) const TX_16X16: TxSize = TX_8X8 + 1;
pub(crate) const TX_32X32: TxSize = TX_16X16 + 1;
pub(crate) const TX_64X64: TxSize = TX_32X32 + 1;
pub(crate) const TX_4X8: TxSize = TX_64X64 + 1;
pub(crate) const TX_8X4: TxSize = TX_4X8 + 1;
pub(crate) const TX_8X16: TxSize = TX_8X4 + 1;
pub(crate) const TX_16X8: TxSize = TX_8X16 + 1;
pub(crate) const TX_16X32: TxSize = TX_16X8 + 1;
pub(crate) const TX_32X16: TxSize = TX_16X32 + 1;
pub(crate) const TX_32X64: TxSize = TX_32X16 + 1;
pub(crate) const TX_64X32: TxSize = TX_32X64 + 1;
pub(crate) const TX_4X16: TxSize = TX_64X32 + 1;
pub(crate) const TX_16X4: TxSize = TX_4X16 + 1;
pub(crate) const TX_8X32: TxSize = TX_16X4 + 1;
pub(crate) const TX_32X8: TxSize = TX_8X32 + 1;
pub(crate) const TX_16X64: TxSize = TX_32X8 + 1;
pub(crate) const TX_64X16: TxSize = TX_16X64 + 1;
pub(crate) const TX_SIZES_ALL: TxSize = TX_64X16 + 1;
pub(crate) const TX_SIZES: TxSize = TX_4X8;
pub(crate) const TX_SIZES_LARGEST: TxSize = TX_64X64;
pub(crate) const TX_INVALID: TxSize = 255;
// TX_TYPE
pub(crate) const DCT_DCT: TxType = 0;
pub(crate) const ADST_DCT: TxType = DCT_DCT + 1;
pub(crate) const DCT_ADST: TxType = ADST_DCT + 1;
pub(crate) const ADST_ADST: TxType = DCT_ADST + 1;
pub(crate) const FLIPADST_DCT: TxType = ADST_ADST + 1;
pub(crate) const DCT_FLIPADST: TxType = FLIPADST_DCT + 1;
pub(crate) const FLIPADST_FLIPADST: TxType = DCT_FLIPADST + 1;
pub(crate) const ADST_FLIPADST: TxType = FLIPADST_FLIPADST + 1;
pub(crate) const FLIPADST_ADST: TxType = ADST_FLIPADST + 1;
pub(crate) const IDTX: TxType = FLIPADST_ADST + 1;
pub(crate) const V_DCT: TxType = IDTX + 1;
pub(crate) const H_DCT: TxType = V_DCT + 1;
pub(crate) const V_ADST: TxType = H_DCT + 1;
pub(crate) const H_ADST: TxType = V_ADST + 1;
pub(crate) const V_FLIPADST: TxType = H_ADST + 1;
pub(crate) const H_FLIPADST: TxType = V_FLIPADST + 1;
pub(crate) const TX_TYPES: TxType = H_FLIPADST + 1;
pub(crate) const DCT_ADST_TX_MASK: TxType = 0x000F;
pub(crate) const TX_TYPE_INVALID: TxType = 255;
// TxSetType
pub(crate) const EXT_TX_SET_DCTONLY: TxSetType = 0;
pub(crate) const EXT_TX_SET_DCT_IDTX: TxSetType = EXT_TX_SET_DCTONLY + 1;
pub(crate) const EXT_TX_SET_DTT4_IDTX: TxSetType = EXT_TX_SET_DCT_IDTX + 1;
pub(crate) const EXT_TX_SET_DTT4_IDTX_1DDCT: TxSetType = EXT_TX_SET_DTT4_IDTX + 1;
pub(crate) const EXT_TX_SET_DTT9_IDTX_1DDCT: TxSetType = EXT_TX_SET_DTT4_IDTX_1DDCT + 1;
pub(crate) const EXT_TX_SET_ALL16: TxSetType = EXT_TX_SET_DTT9_IDTX_1DDCT + 1;
pub(crate) const EXT_TX_SET_TYPES: TxSetType = EXT_TX_SET_ALL16 + 1;

// (common_data.h)

pub(crate) static MI_SIZE_WIDE_LOG2: [u8; BLOCK_SIZES_ALL as usize] = [
    0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 0, 2, 1, 3, 2, 4,
];

pub(crate) static MI_SIZE_HIGH_LOG2: [u8; BLOCK_SIZES_ALL as usize] = [
    0, 1, 0, 1, 2, 1, 2, 3, 2, 3, 4, 3, 4, 5, 4, 5, 2, 0, 3, 1, 4, 2,
];

pub(crate) static MI_SIZE_WIDE: [u8; BLOCK_SIZES_ALL as usize] = [
    1, 1, 2, 2, 2, 4, 4, 4, 8, 8, 8, 16, 16, 16, 32, 32, 1, 4, 2, 8, 4, 16,
];

pub(crate) static MI_SIZE_HIGH: [u8; BLOCK_SIZES_ALL as usize] = [
    1, 2, 1, 2, 4, 2, 4, 8, 4, 8, 16, 8, 16, 32, 16, 32, 4, 1, 8, 2, 16, 4,
];

pub(crate) static BLOCK_SIZE_WIDE: [u8; BLOCK_SIZES_ALL as usize] = [
    4, 4, 8, 8, 8, 16, 16, 16, 32, 32, 32, 64, 64, 64, 128, 128, 4, 16, 8, 32, 16, 64,
];

pub(crate) static BLOCK_SIZE_HIGH: [u8; BLOCK_SIZES_ALL as usize] = [
    4, 8, 4, 8, 16, 8, 16, 32, 16, 32, 64, 32, 64, 128, 64, 128, 16, 4, 32, 8, 64, 16,
];

pub(crate) static SIZE_GROUP_LOOKUP: [u8; BLOCK_SIZES_ALL as usize] = [
    0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 0, 0, 1, 1, 2, 2,
];

pub(crate) static NUM_PELS_LOG2_LOOKUP: [u8; BLOCK_SIZES_ALL as usize] = [
    4, 5, 5, 6, 7, 7, 8, 9, 9, 10, 11, 11, 12, 13, 13, 14, 6, 6, 8, 8, 10, 10,
];

pub(crate) static SUBSIZE_LOOKUP: [[BlockSize; SQR_BLOCK_SIZES as usize];
    EXT_PARTITION_TYPES as usize] = [
    [
        BLOCK_4X4,
        BLOCK_8X8,
        BLOCK_16X16,
        BLOCK_32X32,
        BLOCK_64X64,
        BLOCK_128X128,
    ],
    [
        BLOCK_INVALID,
        BLOCK_8X4,
        BLOCK_16X8,
        BLOCK_32X16,
        BLOCK_64X32,
        BLOCK_128X64,
    ],
    [
        BLOCK_INVALID,
        BLOCK_4X8,
        BLOCK_8X16,
        BLOCK_16X32,
        BLOCK_32X64,
        BLOCK_64X128,
    ],
    [
        BLOCK_INVALID,
        BLOCK_4X4,
        BLOCK_8X8,
        BLOCK_16X16,
        BLOCK_32X32,
        BLOCK_64X64,
    ],
    [
        BLOCK_INVALID,
        BLOCK_INVALID,
        BLOCK_16X8,
        BLOCK_32X16,
        BLOCK_64X32,
        BLOCK_128X64,
    ],
    [
        BLOCK_INVALID,
        BLOCK_INVALID,
        BLOCK_16X8,
        BLOCK_32X16,
        BLOCK_64X32,
        BLOCK_128X64,
    ],
    [
        BLOCK_INVALID,
        BLOCK_INVALID,
        BLOCK_8X16,
        BLOCK_16X32,
        BLOCK_32X64,
        BLOCK_64X128,
    ],
    [
        BLOCK_INVALID,
        BLOCK_INVALID,
        BLOCK_8X16,
        BLOCK_16X32,
        BLOCK_32X64,
        BLOCK_64X128,
    ],
    [
        BLOCK_INVALID,
        BLOCK_INVALID,
        BLOCK_16X4,
        BLOCK_32X8,
        BLOCK_64X16,
        BLOCK_INVALID,
    ],
    [
        BLOCK_INVALID,
        BLOCK_INVALID,
        BLOCK_4X16,
        BLOCK_8X32,
        BLOCK_16X64,
        BLOCK_INVALID,
    ],
];

pub(crate) static MAX_TXSIZE_LOOKUP: [TxSize; BLOCK_SIZES_ALL as usize] = [
    TX_4X4, TX_4X4, TX_4X4, TX_8X8, TX_8X8, TX_8X8, TX_16X16, TX_16X16, TX_16X16, TX_32X32,
    TX_32X32, TX_32X32, TX_64X64, TX_64X64, TX_64X64, TX_64X64, TX_4X4, TX_4X4, TX_8X8, TX_8X8,
    TX_16X16, TX_16X16,
];

pub(crate) static MAX_TXSIZE_RECT_LOOKUP: [TxSize; BLOCK_SIZES_ALL as usize] = [
    TX_4X4, TX_4X8, TX_8X4, TX_8X8, TX_8X16, TX_16X8, TX_16X16, TX_16X32, TX_32X16, TX_32X32,
    TX_32X64, TX_64X32, TX_64X64, TX_64X64, TX_64X64, TX_64X64, TX_4X16, TX_16X4, TX_8X32, TX_32X8,
    TX_16X64, TX_64X16,
];

pub(crate) static VTX_TAB: [TxType1D; TX_TYPES as usize] = [
    DCT_1D,
    ADST_1D,
    DCT_1D,
    ADST_1D,
    FLIPADST_1D,
    DCT_1D,
    FLIPADST_1D,
    ADST_1D,
    FLIPADST_1D,
    IDTX_1D,
    DCT_1D,
    IDTX_1D,
    ADST_1D,
    IDTX_1D,
    FLIPADST_1D,
    IDTX_1D,
];

pub(crate) static HTX_TAB: [TxType1D; TX_TYPES as usize] = [
    DCT_1D,
    DCT_1D,
    ADST_1D,
    ADST_1D,
    DCT_1D,
    FLIPADST_1D,
    FLIPADST_1D,
    FLIPADST_1D,
    ADST_1D,
    IDTX_1D,
    IDTX_1D,
    DCT_1D,
    IDTX_1D,
    ADST_1D,
    IDTX_1D,
    FLIPADST_1D,
];

pub(crate) static SUB_TX_SIZE_MAP: [TxSize; TX_SIZES_ALL as usize] = [
    TX_4X4, TX_4X4, TX_8X8, TX_16X16, TX_32X32, TX_4X4, TX_4X4, TX_8X8, TX_8X8, TX_16X16, TX_16X16,
    TX_32X32, TX_32X32, TX_4X8, TX_8X4, TX_8X16, TX_16X8, TX_16X32, TX_32X16,
];

pub(crate) static TXSIZE_HORZ_MAP: [TxSize; TX_SIZES_ALL as usize] = [
    TX_4X4, TX_8X8, TX_16X16, TX_32X32, TX_64X64, TX_4X4, TX_8X8, TX_8X8, TX_16X16, TX_16X16,
    TX_32X32, TX_32X32, TX_64X64, TX_4X4, TX_16X16, TX_8X8, TX_32X32, TX_16X16, TX_64X64,
];

pub(crate) static TXSIZE_VERT_MAP: [TxSize; TX_SIZES_ALL as usize] = [
    TX_4X4, TX_8X8, TX_16X16, TX_32X32, TX_64X64, TX_8X8, TX_4X4, TX_16X16, TX_8X8, TX_32X32,
    TX_16X16, TX_64X64, TX_32X32, TX_16X16, TX_4X4, TX_32X32, TX_8X8, TX_64X64, TX_16X16,
];

pub(crate) static TX_SIZE_WIDE: [i32; TX_SIZES_ALL as usize] = [
    4, 8, 16, 32, 64, 4, 8, 8, 16, 16, 32, 32, 64, 4, 16, 8, 32, 16, 64,
];

pub(crate) static TX_SIZE_HIGH: [i32; TX_SIZES_ALL as usize] = [
    4, 8, 16, 32, 64, 8, 4, 16, 8, 32, 16, 64, 32, 16, 4, 32, 8, 64, 16,
];

pub(crate) static TX_SIZE_WIDE_UNIT: [i32; TX_SIZES_ALL as usize] =
    [1, 2, 4, 8, 16, 1, 2, 2, 4, 4, 8, 8, 16, 1, 4, 2, 8, 4, 16];

pub(crate) static TX_SIZE_HIGH_UNIT: [i32; TX_SIZES_ALL as usize] =
    [1, 2, 4, 8, 16, 2, 1, 4, 2, 8, 4, 16, 8, 4, 1, 8, 2, 16, 4];

pub(crate) static TX_SIZE_WIDE_LOG2: [i32; TX_SIZES_ALL as usize] =
    [2, 3, 4, 5, 6, 2, 3, 3, 4, 4, 5, 5, 6, 2, 4, 3, 5, 4, 6];

pub(crate) static TX_SIZE_WIDE_UNIT_LOG2: [i32; TX_SIZES_ALL as usize] =
    [0, 1, 2, 3, 4, 0, 1, 1, 2, 2, 3, 3, 4, 0, 2, 1, 3, 2, 4];

pub(crate) static TX_SIZE_HIGH_LOG2: [i32; TX_SIZES_ALL as usize] =
    [2, 3, 4, 5, 6, 3, 2, 4, 3, 5, 4, 6, 5, 4, 2, 5, 3, 6, 4];

pub(crate) static TX_SIZE_HIGH_UNIT_LOG2: [i32; TX_SIZES_ALL as usize] =
    [0, 1, 2, 3, 4, 1, 0, 2, 1, 3, 2, 4, 3, 2, 0, 3, 1, 4, 2];

pub(crate) static TX_SIZE_2D: [i32; (TX_SIZES_ALL + 1) as usize] = [
    16, 64, 256, 1024, 4096, 32, 32, 128, 128, 512, 512, 2048, 2048, 64, 64, 256, 256, 1024, 1024,
    0,
];

pub(crate) static TXSIZE_TO_BSIZE: [BlockSize; TX_SIZES_ALL as usize] = [
    BLOCK_4X4,
    BLOCK_8X8,
    BLOCK_16X16,
    BLOCK_32X32,
    BLOCK_64X64,
    BLOCK_4X8,
    BLOCK_8X4,
    BLOCK_8X16,
    BLOCK_16X8,
    BLOCK_16X32,
    BLOCK_32X16,
    BLOCK_32X64,
    BLOCK_64X32,
    BLOCK_4X16,
    BLOCK_16X4,
    BLOCK_8X32,
    BLOCK_32X8,
    BLOCK_16X64,
    BLOCK_64X16,
];

pub(crate) static TXSIZE_SQR_MAP: [TxSize; TX_SIZES_ALL as usize] = [
    TX_4X4, TX_8X8, TX_16X16, TX_32X32, TX_64X64, TX_4X4, TX_4X4, TX_8X8, TX_8X8, TX_16X16,
    TX_16X16, TX_32X32, TX_32X32, TX_4X4, TX_4X4, TX_8X8, TX_8X8, TX_16X16, TX_16X16,
];

pub(crate) static TXSIZE_SQR_UP_MAP: [TxSize; TX_SIZES_ALL as usize] = [
    TX_4X4, TX_8X8, TX_16X16, TX_32X32, TX_64X64, TX_8X8, TX_8X8, TX_16X16, TX_16X16, TX_32X32,
    TX_32X32, TX_64X64, TX_64X64, TX_16X16, TX_16X16, TX_32X32, TX_32X32, TX_64X64, TX_64X64,
];

pub(crate) static TXSIZE_LOG2_MINUS4: [i8; TX_SIZES_ALL as usize] =
    [0, 2, 4, 6, 6, 1, 1, 3, 3, 5, 5, 6, 6, 2, 2, 4, 4, 5, 5];

pub(crate) static TX_MODE_TO_BIGGEST_TX_SIZE: [TxSize; TX_MODES as usize] =
    [TX_4X4, TX_64X64, TX_64X64];

pub(crate) static INTRA_MODE_CONTEXT: [i32; INTRA_MODES as usize] =
    [0, 1, 2, 3, 4, 4, 4, 4, 3, 0, 1, 2, 0];

// (common_data.c)
// The Subsampled_Size table in the spec (Section 5.11.38. Get plane residual
// size function).
pub(crate) static AV1_SS_SIZE_LOOKUP: [[[BlockSize; 2]; 2]; BLOCK_SIZES_ALL as usize] = [
    //  ss_x == 0      ss_x == 0          ss_x == 1      ss_x == 1
    //  ss_y == 0      ss_y == 1          ss_y == 0      ss_y == 1
    [[BLOCK_4X4, BLOCK_4X4], [BLOCK_4X4, BLOCK_4X4]],
    [[BLOCK_4X8, BLOCK_4X4], [BLOCK_INVALID, BLOCK_4X4]],
    [[BLOCK_8X4, BLOCK_INVALID], [BLOCK_4X4, BLOCK_4X4]],
    [[BLOCK_8X8, BLOCK_8X4], [BLOCK_4X8, BLOCK_4X4]],
    [[BLOCK_8X16, BLOCK_8X8], [BLOCK_INVALID, BLOCK_4X8]],
    [[BLOCK_16X8, BLOCK_INVALID], [BLOCK_8X8, BLOCK_8X4]],
    [[BLOCK_16X16, BLOCK_16X8], [BLOCK_8X16, BLOCK_8X8]],
    [[BLOCK_16X32, BLOCK_16X16], [BLOCK_INVALID, BLOCK_8X16]],
    [[BLOCK_32X16, BLOCK_INVALID], [BLOCK_16X16, BLOCK_16X8]],
    [[BLOCK_32X32, BLOCK_32X16], [BLOCK_16X32, BLOCK_16X16]],
    [[BLOCK_32X64, BLOCK_32X32], [BLOCK_INVALID, BLOCK_16X32]],
    [[BLOCK_64X32, BLOCK_INVALID], [BLOCK_32X32, BLOCK_32X16]],
    [[BLOCK_64X64, BLOCK_64X32], [BLOCK_32X64, BLOCK_32X32]],
    [[BLOCK_64X128, BLOCK_64X64], [BLOCK_INVALID, BLOCK_32X64]],
    [[BLOCK_128X64, BLOCK_INVALID], [BLOCK_64X64, BLOCK_64X32]],
    [[BLOCK_128X128, BLOCK_128X64], [BLOCK_64X128, BLOCK_64X64]],
    [[BLOCK_4X16, BLOCK_4X8], [BLOCK_INVALID, BLOCK_4X8]],
    [[BLOCK_16X4, BLOCK_INVALID], [BLOCK_8X4, BLOCK_8X4]],
    [[BLOCK_8X32, BLOCK_8X16], [BLOCK_INVALID, BLOCK_4X16]],
    [[BLOCK_32X8, BLOCK_INVALID], [BLOCK_16X8, BLOCK_16X4]],
    [[BLOCK_16X64, BLOCK_16X32], [BLOCK_INVALID, BLOCK_8X32]],
    [[BLOCK_64X16, BLOCK_INVALID], [BLOCK_32X16, BLOCK_32X8]],
];

// Generates 5 bit field in which each bit set to 1 represents
// a blocksize partition  11111 means we split 128x128, 64x64, 32x32, 16x16
// and 8x8.  10000 means we just split the 128x128 to 64x64
#[derive(Clone, Copy)]
pub(crate) struct PartitionContextPair {
    pub(crate) above: PartitionContext,
    pub(crate) left: PartitionContext,
}

const fn pcl(above: PartitionContext, left: PartitionContext) -> PartitionContextPair {
    PartitionContextPair { above, left }
}

pub(crate) static PARTITION_CONTEXT_LOOKUP: [PartitionContextPair; BLOCK_SIZES_ALL as usize] = [
    pcl(31, 31), // 4X4   - {0b11111, 0b11111}
    pcl(31, 30), // 4X8   - {0b11111, 0b11110}
    pcl(30, 31), // 8X4   - {0b11110, 0b11111}
    pcl(30, 30), // 8X8   - {0b11110, 0b11110}
    pcl(30, 28), // 8X16  - {0b11110, 0b11100}
    pcl(28, 30), // 16X8  - {0b11100, 0b11110}
    pcl(28, 28), // 16X16 - {0b11100, 0b11100}
    pcl(28, 24), // 16X32 - {0b11100, 0b11000}
    pcl(24, 28), // 32X16 - {0b11000, 0b11100}
    pcl(24, 24), // 32X32 - {0b11000, 0b11000}
    pcl(24, 16), // 32X64 - {0b11000, 0b10000}
    pcl(16, 24), // 64X32 - {0b10000, 0b11000}
    pcl(16, 16), // 64X64 - {0b10000, 0b10000}
    pcl(16, 0),  // 64X128- {0b10000, 0b00000}
    pcl(0, 16),  // 128X64- {0b00000, 0b10000}
    pcl(0, 0),   // 128X128-{0b00000, 0b00000}
    pcl(31, 28), // 4X16  - {0b11111, 0b11100}
    pcl(28, 31), // 16X4  - {0b11100, 0b11111}
    pcl(30, 24), // 8X32  - {0b11110, 0b11000}
    pcl(24, 30), // 32X8  - {0b11000, 0b11110}
    pcl(28, 16), // 16X64 - {0b11100, 0b10000}
    pcl(16, 28), // 64X16 - {0b10000, 0b11100}
];

// aom_bit_depth_t (aom/aom_codec.h)
pub(crate) const AOM_BITS_8: i32 = 8; // 8 bits
pub(crate) const AOM_BITS_10: i32 = 10; // 10 bits
pub(crate) const AOM_BITS_12: i32 = 12; // 12 bits

// The CICP values of aom/aom_image.h the encoder compares against.
pub(crate) const AOM_CICP_CP_BT_709: i32 = 1; // BT.709
pub(crate) const AOM_CICP_CP_UNSPECIFIED: i32 = 2; // Unspecified
pub(crate) const AOM_CICP_CP_BT_2020: i32 = 9; // BT.2020, BT.2100
pub(crate) const AOM_CICP_TC_UNSPECIFIED: i32 = 2; // Unspecified
pub(crate) const AOM_CICP_TC_SRGB: i32 = 13; // sRGB or sYCC
pub(crate) const AOM_CICP_MC_IDENTITY: i32 = 0; // Identity matrix
pub(crate) const AOM_CICP_MC_UNSPECIFIED: i32 = 2; // Unspecified

// (aom/aom_image.h)
pub(crate) const AOM_PLANE_Y: usize = 0; // Y (Luminance) plane
pub(crate) const AOM_PLANE_U: usize = 1; // U (Chroma) plane
pub(crate) const AOM_PLANE_V: usize = 2; // V (Chroma) plane

// (common_data.h)
pub(crate) const TX_SIZE_W_MIN: i32 = 4;
pub(crate) const TX_SIZE_H_MIN: i32 = 4;
