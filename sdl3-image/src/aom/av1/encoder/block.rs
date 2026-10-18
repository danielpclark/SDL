// Rust translation of the parts of av1/encoder/block.h (and the enums of
// av1/encoder/enc_enums.h, av1/encoder/tokenize.h and av1/encoder/encodemb.h
// it uses) from aom (https://aomedia.googlesource.com/aom, at the revision
// SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches) the saver
// reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encoder's block state (`MACROBLOCK`). Only the members the saver's
//! intra-only path reads are kept; in particular the motion search, inter
//! mode, palette and intra block copy state is left out, as are the mode
//! costs of inter frames, palettes and segmentation (computed but never
//! read for key frames). Coefficient buffers belong to the planes (upstream
//! points them at buffers shared by every pick-mode context); the frame's
//! coefficient buffers for packing (`cpi->coeff_buffer_base`) and the
//! frame's extended mode info (`cpi->mbmi_ext_info.frame_base`) are moved
//! in while the tiles are encoded or packed, like the frame state of
//! `MacroBlockD`.

use super::enc_enums::MAX_MODES;
use crate::aom::aom_dsp::aom_dsp_common::TranLowT;
use crate::aom::av1::common::blockd::{Buf2d, CandidateMv, MacroBlockD, MAX_MB_PLANE};
use crate::aom::av1::common::entropy::*;
use crate::aom::av1::common::entropymode::*;
use crate::aom::av1::common::entropymv::{MV_JOINTS, MV_VALS};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::{SWITCHABLE_FILTERS, SWITCHABLE_FILTER_CONTEXTS};
use crate::aom::av1::common::mv::{FullMvLimits, IntMv};
use crate::aom::av1::common::seg_common::{
    MAX_SEGMENTS, SEG_TEMPORAL_PRED_CTXS, SPATIAL_PREDICTION_PROBS,
};

// (MODE_EVAL_TYPE)
pub(crate) const DEFAULT_EVAL: i32 = 0;
pub(crate) const MODE_EVAL: i32 = 1;
pub(crate) const WINNER_MODE_EVAL: i32 = 2;
pub(crate) const MODE_EVAL_TYPES: usize = 3;

// (TX_SIZE_SEARCH_METHOD)
pub(crate) const USE_FULL_RD: i32 = 0;
pub(crate) const USE_FAST_RD: i32 = 1;
pub(crate) const USE_LARGESTALL: i32 = 2;

// (RUN_TYPE)
pub(crate) const OUTPUT_ENABLED: i32 = 0;
pub(crate) const DRY_RUN_NORMAL: i32 = 1;
pub(crate) const DRY_RUN_COSTCOEFFS: i32 = 2;

// (TRELLIS_OPT_TYPE)
pub(crate) const NO_TRELLIS_OPT: i32 = 0; // No trellis optimization
pub(crate) const FULL_TRELLIS_OPT: i32 = 1; // Trellis optimization in all stages
pub(crate) const FINAL_PASS_TRELLIS_OPT: i32 = 2; // Trellis optimization in only the final encode pass
pub(crate) const NO_ESTIMATE_YRD_TRELLIS_OPT: i32 = 3; // Disable trellis in estimate_yrd_for_sb

/// The largest superblock's sample count (`MAX_SB_SQUARE`).
pub(crate) const MAX_SB_SQUARE_USIZE: usize = MAX_SB_SQUARE as usize;

/// Translation of `RD_STATS` (blockd.h).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RdStats {
    pub(crate) rate: i32,
    pub(crate) zero_rate: i32,
    pub(crate) dist: i64,
    // Please be careful of using rdcost, it's not guaranteed to be set all the
    // time.
    // TODO(angiebird): Create a set of functions to manipulate the RD_STATS. In
    // these functions, make sure rdcost is always up-to-date according to
    // rate/dist.
    pub(crate) rdcost: i64,
    pub(crate) sse: i64,
    pub(crate) skip_txfm: u8, // sse should equal to dist when skip_txfm == 1
}

/// Translation of `MACROBLOCK_PLANE`: the coefficient buffers are owned
/// (see the module documentation), the quantizer pointers are the rows of
/// the frame's quantizer tables, copied.
#[derive(Debug, Clone, Default)]
pub(crate) struct MacroblockPlane {
    /// Stores source - pred so the txfm can be computed later
    pub(crate) src_diff: Vec<i16>,
    /// Dequantized coefficients
    pub(crate) dqcoeff: Vec<TranLowT>,
    /// Quantized coefficients
    pub(crate) qcoeff: Vec<TranLowT>,
    /// Transformed coefficients
    pub(crate) coeff: Vec<TranLowT>,
    /// Location of the end of qcoeff (end of block).
    pub(crate) eobs: Vec<u16>,
    /// Contexts used to code the transform coefficients.
    pub(crate) txb_entropy_ctx: Vec<u8>,
    /// A buffer containing the source frame.
    pub(crate) src: Buf2d,

    // Quantizer factors used in quantization process.
    pub(crate) quant_fp_qtx: [i16; 8],
    pub(crate) round_fp_qtx: [i16; 8],
    pub(crate) quant_qtx: [i16; 8],
    pub(crate) round_qtx: [i16; 8],
    pub(crate) quant_shift_qtx: [i16; 8],
    pub(crate) zbin_qtx: [i16; 8],
    pub(crate) dequant_qtx: [i16; 8],
}

/// Costs for encoding the coefficients within a level.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LvMapCoeffCost {
    /// Cost to skip txfm for the current txfm block.
    pub(crate) txb_skip_cost: [[i32; 2]; TXB_SKIP_CONTEXTS as usize],
    /// Cost for encoding the base_eob of a level.
    pub(crate) base_eob_cost: [[i32; 3]; SIG_COEF_CONTEXTS_EOB as usize],
    /// Cost for encoding the base level of a coefficient.
    pub(crate) base_cost: [[i32; 8]; SIG_COEF_CONTEXTS as usize],
    /// Cost for encoding the last non-zero coefficient.
    pub(crate) eob_extra_cost: [[i32; 2]; EOB_COEF_CONTEXTS as usize],
    /// Cost for encoding the dc_sign
    pub(crate) dc_sign_cost: [[i32; 2]; DC_SIGN_CONTEXTS as usize],
    /// Cost for encoding an increment to the coefficient
    pub(crate) lps_cost:
        [[i32; (COEFF_BASE_RANGE + 1 + COEFF_BASE_RANGE + 1) as usize]; LEVEL_CONTEXTS as usize],
}

impl Default for LvMapCoeffCost {
    fn default() -> Self {
        LvMapCoeffCost {
            txb_skip_cost: [[0; 2]; TXB_SKIP_CONTEXTS as usize],
            base_eob_cost: [[0; 3]; SIG_COEF_CONTEXTS_EOB as usize],
            base_cost: [[0; 8]; SIG_COEF_CONTEXTS as usize],
            eob_extra_cost: [[0; 2]; EOB_COEF_CONTEXTS as usize],
            dc_sign_cost: [[0; 2]; DC_SIGN_CONTEXTS as usize],
            lps_cost: [[0; (COEFF_BASE_RANGE + 1 + COEFF_BASE_RANGE + 1) as usize];
                LEVEL_CONTEXTS as usize],
        }
    }
}

/// Costs for encoding the eob.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LvMapEobCost {
    /// eob_cost.
    pub(crate) eob_cost: [[i32; 11]; 2],
}

/// Translation of `CB_COEFF_BUFFER`: the coefficients of a superblock,
/// stored for bitstream packing.
#[derive(Debug, Clone, Default)]
pub(crate) struct CbCoeffBuffer {
    /// The quantized transform coefficients.
    pub(crate) tcoeff: [Vec<TranLowT>; MAX_MB_PLANE],
    /// The end of block.
    pub(crate) eobs: [Vec<u16>; MAX_MB_PLANE],
    /// The entropy context.
    pub(crate) entropy_ctx: [Vec<u8>; MAX_MB_PLANE],
}

/// Translation of `MB_MODE_INFO_EXT`: extended mode info derived from mbmi.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MbModeInfoExt {
    // TODO(angiebird): Reduce the buffer size according to sb_type
    /// The reference mv list for the current block.
    pub(crate) ref_mv_stack:
        [[CandidateMv; USABLE_REF_MV_STACK_SIZE as usize]; MODE_CTX_REF_FRAMES as usize],
    /// The weights used to compute the ref mvs.
    pub(crate) weight: [[u16; USABLE_REF_MV_STACK_SIZE as usize]; MODE_CTX_REF_FRAMES as usize],
    /// Number of ref mvs in the drl.
    pub(crate) ref_mv_count: [u8; MODE_CTX_REF_FRAMES as usize],
    /// Global mvs
    pub(crate) global_mvs: [IntMv; REF_FRAMES as usize],
    /// Context used to encode the current mode.
    pub(crate) mode_context: [i16; MODE_CTX_REF_FRAMES as usize],
}

impl Default for MbModeInfoExt {
    fn default() -> Self {
        MbModeInfoExt {
            ref_mv_stack: [[CandidateMv::default(); USABLE_REF_MV_STACK_SIZE as usize];
                MODE_CTX_REF_FRAMES as usize],
            weight: [[0; USABLE_REF_MV_STACK_SIZE as usize]; MODE_CTX_REF_FRAMES as usize],
            ref_mv_count: [0; MODE_CTX_REF_FRAMES as usize],
            global_mvs: [IntMv::default(); REF_FRAMES as usize],
            mode_context: [0; MODE_CTX_REF_FRAMES as usize],
        }
    }
}

/// Translation of `MB_MODE_INFO_EXT_FRAME`: stores best extended mode
/// information at frame level.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MbModeInfoExtFrame {
    /// The reference mv list for the current block.
    pub(crate) ref_mv_stack: [CandidateMv; USABLE_REF_MV_STACK_SIZE as usize],
    /// The weights used to compute the ref mvs.
    pub(crate) weight: [u16; USABLE_REF_MV_STACK_SIZE as usize],
    /// Number of ref mvs in the drl.
    pub(crate) ref_mv_count: u8,
    /// Global mvs
    pub(crate) global_mvs: [IntMv; REF_FRAMES as usize],
    /// Context used to encode the current mode.
    pub(crate) mode_context: i16,
    /// Offset of current coding block's coeff buffer relative to the sb.
    pub(crate) cb_offset: [u16; PLANE_TYPES as usize],
}

/// Parameters used for transform search.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TxfmSearchParams {
    /// Whether to limit the intra txfm search type to the default txfm.
    pub(crate) use_default_intra_tx_type: i32,
    /// Whether to limit the inter txfm search type to the default txfm.
    pub(crate) default_inter_tx_type_prob_thresh: i32,
    /// Whether to prune 2d transforms based on 1d transform results.
    pub(crate) prune_2d_txfm_mode: i32,
    /// Variable from \ref WinnerModeParams based on current eval mode.
    pub(crate) coeff_opt_thresholds: [u32; 2],
    pub(crate) tx_domain_dist_threshold: u32,
    pub(crate) tx_size_search_method: i32,
    pub(crate) use_transform_domain_distortion: u32,
    pub(crate) skip_txfm_level: u32,
    /// How to search for the optimal tx_size
    pub(crate) tx_mode_search_type: TxMode,
    /// Determines whether a block can be predicted as transform skip or DC only
    /// based on residual information.
    pub(crate) predict_dc_level: u32,
    /// Whether or not we should use the quantization matrix as weights for PSNR
    /// during RD search.
    pub(crate) use_qm_dist_metric: i32,
    /// Keep track of previous mode evaluation stage type. This will be used to
    /// reset mb rd hash record when mode evaluation type changes.
    pub(crate) mode_eval_type: i32,
    /// Indicates the transform depths for which RD evaluation is skipped.
    pub(crate) nn_prune_depths_for_intra_tx: i32,
    /// Indicates if NN model should be invoked to prune transform depths.
    pub(crate) enable_nn_prune_intra_tx_depths: bool,
}

/// Results from transform search.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TxfmSearchInfo {
    /// Whether to skip transform and quantization on a partition block level.
    pub(crate) skip_txfm: u8,
    /// Whether to skip transform and quantization on a txfm block level.
    pub(crate) blk_skip: [u8; (MAX_MIB_SIZE * MAX_MIB_SIZE) as usize],
    /// Number of txb splits.
    pub(crate) txb_split_count: u32,
}

impl Default for TxfmSearchInfo {
    fn default() -> Self {
        TxfmSearchInfo {
            skip_txfm: 0,
            blk_skip: [0; (MAX_MIB_SIZE * MAX_MIB_SIZE) as usize],
            txb_split_count: 0,
        }
    }
}

/// Holds the costs of the mode symbols (`ModeCosts`).
#[derive(Debug, Clone)]
pub(crate) struct ModeCosts {
    /// Cost for coding the partition.
    pub(crate) partition_cost: [[i32; EXT_PARTITION_TYPES as usize]; PARTITION_CONTEXTS as usize],
    /// Luma mode cost for inter frame.
    pub(crate) mbmode_cost: [[i32; INTRA_MODES as usize]; BLOCK_SIZE_GROUPS as usize],
    /// Luma mode cost for intra frame.
    pub(crate) y_mode_costs:
        [[[i32; INTRA_MODES as usize]; INTRA_MODES as usize]; INTRA_MODES as usize],
    /// Chroma mode cost
    pub(crate) intra_uv_mode_cost:
        [[[i32; UV_INTRA_MODES as usize]; INTRA_MODES as usize]; CFL_ALLOWED_TYPES as usize],
    /// filter_intra_cost
    pub(crate) filter_intra_cost: [[i32; 2]; BLOCK_SIZES_ALL as usize],
    /// filter_intra_mode_cost
    pub(crate) filter_intra_mode_cost: [i32; FILTER_INTRA_MODES as usize],
    /// angle_delta_cost
    pub(crate) angle_delta_cost:
        [[i32; (2 * MAX_ANGLE_DELTA + 1) as usize]; DIRECTIONAL_MODES as usize],
    /// Rate rate associated with each alpha codeword
    pub(crate) cfl_cost:
        [[[i32; CFL_ALPHABET_SIZE as usize]; CFL_PRED_PLANES as usize]; CFL_JOINT_SIGNS as usize],
    /// intrabc_cost
    pub(crate) intrabc_cost: [i32; 2],
    /// palette_y_size_cost
    pub(crate) palette_y_size_cost: [[i32; PALETTE_SIZES as usize]; PALATTE_BSIZE_CTXS as usize],
    /// palette_uv_size_cost
    pub(crate) palette_uv_size_cost: [[i32; PALETTE_SIZES as usize]; PALATTE_BSIZE_CTXS as usize],
    /// palette_y_color_cost
    pub(crate) palette_y_color_cost: [[[i32; PALETTE_COLORS as usize];
        PALETTE_COLOR_INDEX_CONTEXTS as usize];
        PALETTE_SIZES as usize],
    /// palette_uv_color_cost
    pub(crate) palette_uv_color_cost: [[[i32; PALETTE_COLORS as usize];
        PALETTE_COLOR_INDEX_CONTEXTS as usize];
        PALETTE_SIZES as usize],
    /// palette_y_mode_cost
    pub(crate) palette_y_mode_cost:
        [[[i32; 2]; PALETTE_Y_MODE_CONTEXTS as usize]; PALATTE_BSIZE_CTXS as usize],
    /// palette_uv_mode_cost
    pub(crate) palette_uv_mode_cost: [[i32; 2]; PALETTE_UV_MODE_CONTEXTS as usize],
    /// skip_mode_cost
    pub(crate) skip_mode_cost: [[i32; 2]; SKIP_MODE_CONTEXTS as usize],
    /// newmv_mode_cost
    pub(crate) newmv_mode_cost: [[i32; 2]; NEWMV_MODE_CONTEXTS as usize],
    /// zeromv_mode_cost
    pub(crate) zeromv_mode_cost: [[i32; 2]; GLOBALMV_MODE_CONTEXTS as usize],
    /// refmv_mode_cost
    pub(crate) refmv_mode_cost: [[i32; 2]; REFMV_MODE_CONTEXTS as usize],
    /// drl_mode_cost0
    pub(crate) drl_mode_cost0: [[i32; 2]; DRL_MODE_CONTEXTS as usize],
    /// single_ref_cost
    pub(crate) single_ref_cost: [[[i32; 2]; (SINGLE_REFS - 1) as usize]; REF_CONTEXTS as usize],
    /// comp_inter_cost
    pub(crate) comp_inter_cost: [[i32; 2]; COMP_INTER_CONTEXTS as usize],
    /// comp_ref_type_cost
    pub(crate) comp_ref_type_cost:
        [[i32; COMP_REFERENCE_TYPES as usize + 1]; COMP_REF_TYPE_CONTEXTS as usize],
    /// uni_comp_ref_cost
    pub(crate) uni_comp_ref_cost:
        [[[i32; 2 + 1]; (UNIDIR_COMP_REFS - 1) as usize]; UNI_COMP_REF_CONTEXTS as usize],
    /// Cost for signaling ref_frame[0] in bidir-comp mode * * Includes LAST_FRAME, LAST2_FRAME, LAST3_FRAME, and GOLDEN_FRAME.
    pub(crate) comp_ref_cost: [[[i32; 2]; (FWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
    /// Cost for signaling ref_frame[1] in bidir-comp mode * * Includes ALTREF_FRAME, ALTREF2_FRAME, and BWDREF_FRAME.
    pub(crate) comp_bwdref_cost: [[[i32; 2]; (BWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
    /// intra_inter_cost
    pub(crate) intra_inter_cost: [[i32; 2]; INTRA_INTER_CONTEXTS as usize],
    /// inter_compound_mode_cost
    pub(crate) inter_compound_mode_cost:
        [[i32; INTER_COMPOUND_MODES as usize]; INTER_MODE_CONTEXTS as usize],
    /// compound_type_cost
    pub(crate) compound_type_cost:
        [[i32; MASKED_COMPOUND_TYPES as usize]; BLOCK_SIZES_ALL as usize],
    /// wedge_idx_cost
    pub(crate) wedge_idx_cost: [[i32; 16]; BLOCK_SIZES_ALL as usize],
    /// interintra_cost
    pub(crate) interintra_cost: [[i32; 2]; BLOCK_SIZE_GROUPS as usize],
    /// wedge_interintra_cost
    pub(crate) wedge_interintra_cost: [[i32; 2]; BLOCK_SIZES_ALL as usize],
    /// interintra_mode_cost
    pub(crate) interintra_mode_cost: [[i32; INTERINTRA_MODES as usize]; BLOCK_SIZE_GROUPS as usize],
    /// comp_idx_cost
    pub(crate) comp_idx_cost: [[i32; 2]; COMP_INDEX_CONTEXTS as usize],
    /// comp_group_idx_cost
    pub(crate) comp_group_idx_cost: [[i32; 2]; COMP_GROUP_IDX_CONTEXTS as usize],
    /// motion_mode_cost
    pub(crate) motion_mode_cost: [[i32; MOTION_MODES as usize]; BLOCK_SIZES_ALL as usize],
    /// motion_mode_cost1
    pub(crate) motion_mode_cost1: [[i32; 2]; BLOCK_SIZES_ALL as usize],
    /// switchable_interp_costs
    pub(crate) switchable_interp_costs:
        [[i32; SWITCHABLE_FILTERS as usize]; SWITCHABLE_FILTER_CONTEXTS as usize],
    /// skip_txfm_cost
    pub(crate) skip_txfm_cost: [[i32; 2]; SKIP_CONTEXTS as usize],
    /// tx_size_cost
    pub(crate) tx_size_cost:
        [[[i32; TX_SIZES as usize]; TX_SIZE_CONTEXTS as usize]; (TX_SIZES - 1) as usize],
    /// txfm_partition_cost
    pub(crate) txfm_partition_cost: [[i32; 2]; TXFM_PARTITION_CONTEXTS as usize],
    /// inter_tx_type_costs
    pub(crate) inter_tx_type_costs:
        [[[i32; TX_TYPES as usize]; EXT_TX_SIZES as usize]; EXT_TX_SETS_INTER as usize],
    /// intra_tx_type_costs
    pub(crate) intra_tx_type_costs: [[[[i32; TX_TYPES as usize]; INTRA_MODES as usize];
        EXT_TX_SIZES as usize]; EXT_TX_SETS_INTRA as usize],
    /// switchable_restore_cost
    pub(crate) switchable_restore_cost: [i32; RESTORE_SWITCHABLE_TYPES as usize],
    /// wiener_restore_cost
    pub(crate) wiener_restore_cost: [i32; 2],
    /// sgrproj_restore_cost
    pub(crate) sgrproj_restore_cost: [i32; 2],
    /// tmp_pred_cost
    pub(crate) tmp_pred_cost: [[i32; 2]; SEG_TEMPORAL_PRED_CTXS as usize],
    /// spatial_pred_cost
    pub(crate) spatial_pred_cost: [[i32; MAX_SEGMENTS as usize]; SPATIAL_PREDICTION_PROBS as usize],
}

impl Default for ModeCosts {
    fn default() -> Self {
        ModeCosts {
            partition_cost: [[0; EXT_PARTITION_TYPES as usize]; PARTITION_CONTEXTS as usize],
            mbmode_cost: [[0; INTRA_MODES as usize]; BLOCK_SIZE_GROUPS as usize],
            y_mode_costs: [[[0; INTRA_MODES as usize]; INTRA_MODES as usize]; INTRA_MODES as usize],
            intra_uv_mode_cost: [[[0; UV_INTRA_MODES as usize]; INTRA_MODES as usize];
                CFL_ALLOWED_TYPES as usize],
            filter_intra_cost: [[0; 2]; BLOCK_SIZES_ALL as usize],
            filter_intra_mode_cost: [0; FILTER_INTRA_MODES as usize],
            angle_delta_cost: [[0; (2 * MAX_ANGLE_DELTA + 1) as usize]; DIRECTIONAL_MODES as usize],
            cfl_cost: [[[0; CFL_ALPHABET_SIZE as usize]; CFL_PRED_PLANES as usize];
                CFL_JOINT_SIGNS as usize],
            intrabc_cost: [0; 2],
            palette_y_size_cost: [[0; PALETTE_SIZES as usize]; PALATTE_BSIZE_CTXS as usize],
            palette_uv_size_cost: [[0; PALETTE_SIZES as usize]; PALATTE_BSIZE_CTXS as usize],
            palette_y_color_cost: [[[0; PALETTE_COLORS as usize];
                PALETTE_COLOR_INDEX_CONTEXTS as usize];
                PALETTE_SIZES as usize],
            palette_uv_color_cost: [[[0; PALETTE_COLORS as usize];
                PALETTE_COLOR_INDEX_CONTEXTS as usize];
                PALETTE_SIZES as usize],
            palette_y_mode_cost: [[[0; 2]; PALETTE_Y_MODE_CONTEXTS as usize];
                PALATTE_BSIZE_CTXS as usize],
            palette_uv_mode_cost: [[0; 2]; PALETTE_UV_MODE_CONTEXTS as usize],
            skip_mode_cost: [[0; 2]; SKIP_MODE_CONTEXTS as usize],
            newmv_mode_cost: [[0; 2]; NEWMV_MODE_CONTEXTS as usize],
            zeromv_mode_cost: [[0; 2]; GLOBALMV_MODE_CONTEXTS as usize],
            refmv_mode_cost: [[0; 2]; REFMV_MODE_CONTEXTS as usize],
            drl_mode_cost0: [[0; 2]; DRL_MODE_CONTEXTS as usize],
            single_ref_cost: [[[0; 2]; (SINGLE_REFS - 1) as usize]; REF_CONTEXTS as usize],
            comp_inter_cost: [[0; 2]; COMP_INTER_CONTEXTS as usize],
            comp_ref_type_cost: [[0; COMP_REFERENCE_TYPES as usize + 1];
                COMP_REF_TYPE_CONTEXTS as usize],
            uni_comp_ref_cost: [[[0; 2 + 1]; (UNIDIR_COMP_REFS - 1) as usize];
                UNI_COMP_REF_CONTEXTS as usize],
            comp_ref_cost: [[[0; 2]; (FWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
            comp_bwdref_cost: [[[0; 2]; (BWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
            intra_inter_cost: [[0; 2]; INTRA_INTER_CONTEXTS as usize],
            inter_compound_mode_cost: [[0; INTER_COMPOUND_MODES as usize];
                INTER_MODE_CONTEXTS as usize],
            compound_type_cost: [[0; MASKED_COMPOUND_TYPES as usize]; BLOCK_SIZES_ALL as usize],
            wedge_idx_cost: [[0; 16]; BLOCK_SIZES_ALL as usize],
            interintra_cost: [[0; 2]; BLOCK_SIZE_GROUPS as usize],
            wedge_interintra_cost: [[0; 2]; BLOCK_SIZES_ALL as usize],
            interintra_mode_cost: [[0; INTERINTRA_MODES as usize]; BLOCK_SIZE_GROUPS as usize],
            comp_idx_cost: [[0; 2]; COMP_INDEX_CONTEXTS as usize],
            comp_group_idx_cost: [[0; 2]; COMP_GROUP_IDX_CONTEXTS as usize],
            motion_mode_cost: [[0; MOTION_MODES as usize]; BLOCK_SIZES_ALL as usize],
            motion_mode_cost1: [[0; 2]; BLOCK_SIZES_ALL as usize],
            switchable_interp_costs: [[0; SWITCHABLE_FILTERS as usize];
                SWITCHABLE_FILTER_CONTEXTS as usize],
            skip_txfm_cost: [[0; 2]; SKIP_CONTEXTS as usize],
            tx_size_cost: [[[0; TX_SIZES as usize]; TX_SIZE_CONTEXTS as usize];
                (TX_SIZES - 1) as usize],
            txfm_partition_cost: [[0; 2]; TXFM_PARTITION_CONTEXTS as usize],
            inter_tx_type_costs: [[[0; TX_TYPES as usize]; EXT_TX_SIZES as usize];
                EXT_TX_SETS_INTER as usize],
            intra_tx_type_costs: [[[[0; TX_TYPES as usize]; INTRA_MODES as usize];
                EXT_TX_SIZES as usize];
                EXT_TX_SETS_INTRA as usize],
            switchable_restore_cost: [0; RESTORE_SWITCHABLE_TYPES as usize],
            wiener_restore_cost: [0; 2],
            sgrproj_restore_cost: [0; 2],
            tmp_pred_cost: [[0; 2]; SEG_TEMPORAL_PRED_CTXS as usize],
            spatial_pred_cost: [[0; MAX_SEGMENTS as usize]; SPATIAL_PREDICTION_PROBS as usize],
        }
    }
}

/// Holds mv costs for encoding and motion search (`MvCosts`). The
/// `nmv_cost` and `nmv_cost_hp` pointers to the middle of the allocations
/// are the offset `MV_MAX` into them, and `mv_cost_stack` (the table in
/// use) is `mv_cost_stack_hp` choosing between them.
#[derive(Debug, Clone)]
pub(crate) struct MvCosts {
    /// Costs for coding the zero components.
    pub(crate) nmv_joint_cost: [i32; MV_JOINTS as usize],
    /// Allocates memory for 1/4-pel motion vector costs.
    pub(crate) nmv_cost_alloc: [Vec<i32>; 2],
    /// Allocates memory for 1/8-pel motion vector costs.
    pub(crate) nmv_cost_hp_alloc: [Vec<i32>; 2],
    /// Whether `mv_cost_stack` points to `nmv_cost_hp` (else `nmv_cost`).
    pub(crate) mv_cost_stack_hp: bool,
}

impl MvCosts {
    /// The zeroed allocation (`aom_calloc()`).
    pub(crate) fn new() -> Result<Box<MvCosts>, ()> {
        let n = MV_VALS as usize;
        let z = || crate::aom::av1::common::alloccommon::try_vec(n, 0i32);
        Ok(Box::new(MvCosts {
            nmv_joint_cost: [0; MV_JOINTS as usize],
            nmv_cost_alloc: [z()?, z()?],
            nmv_cost_hp_alloc: [z()?, z()?],
            mv_cost_stack_hp: false,
        }))
    }

    /// `mv_cost_stack[comp]`: the cost table in use, indexed by
    /// `MV_MAX + v`.
    #[inline]
    pub(crate) fn mv_cost_stack(&self, comp: usize) -> &[i32] {
        if self.mv_cost_stack_hp {
            &self.nmv_cost_hp_alloc[comp]
        } else {
            &self.nmv_cost_alloc[comp]
        }
    }
}

/// Holds the costs needed to encode the coefficients
#[derive(Debug, Clone)]
pub(crate) struct CoeffCosts {
    /// Costs for coding the coefficients.
    pub(crate) coeff_costs: [[LvMapCoeffCost; PLANE_TYPES as usize]; TX_SIZES as usize],
    /// Costs for coding the eobs.
    pub(crate) eob_costs: [[LvMapEobCost; 2]; 7],
}

impl Default for CoeffCosts {
    fn default() -> Self {
        CoeffCosts {
            coeff_costs: [[LvMapCoeffCost::default(); PLANE_TYPES as usize]; TX_SIZES as usize],
            eob_costs: [[LvMapEobCost::default(); 2]; 7],
        }
    }
}

// (SOURCE_SAD)
pub(crate) const K_ZERO_SAD: i32 = 0;
pub(crate) const K_VERY_LOW_SAD: i32 = 1;
pub(crate) const K_LOW_SAD: i32 = 2;
pub(crate) const K_MED_SAD: i32 = 3;
pub(crate) const K_HIGH_SAD: i32 = 4;

/// `PartitionSearchInfo` (the reached member)
#[derive(Debug, Clone, Copy)]
pub(crate) struct PartitionSearchInfo {
    /// Stores the flags of the blocks of the superblock whose temporal
    /// variance is low (the variance-based partitioning sets it, the
    /// non-RD mode search reads it):
    /// - 0 - 128x128 (or 64x64 for the 64x64 superblocks)
    /// - 1-2 - 128x64, 3-4 - 64x128, 5-8 - 64x64 (or 32x32)
    /// - 9-16 - 64x32 (or 16x16), 17-24 - 32x64, 25-40 - 32x32
    /// - 41-104 - 16x16
    pub(crate) variance_low: [u8; 105],
}

impl Default for PartitionSearchInfo {
    fn default() -> Self {
        PartitionSearchInfo {
            variance_low: [0; 105],
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ContentStateSb {
    pub(crate) source_sad_nonrd: i32,
    pub(crate) source_sad_rd: i32,
    pub(crate) lighting_change: i32,
    pub(crate) low_sumdiff: i32,
}

/// Translation of `WARP_SAMPLE_INFO`: the warp motion samples.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WarpSampleInfo {
    /// Number of samples.
    pub(crate) num: i32,
    /// Sample locations in current frame.
    pub(crate) pts: [i32; 16],
    /// Sample location in the reference frame.
    pub(crate) pts_inref: [i32; 16],
}

/// Translation of `MACROBLOCK` (see the module documentation).
#[derive(Debug)]
pub(crate) struct Macroblock {
    /// Each of the encoding plane.
    pub(crate) plane: [MacroblockPlane; MAX_MB_PLANE],

    /// Decoder's view of current coding block.
    pub(crate) e_mbd: MacroBlockD,

    /// Derived coding information.
    pub(crate) mbmi_ext: MbModeInfoExt,
    /// `x->mbmi_ext_frame`: an index into `mbmi_ext_frame_base`.
    pub(crate) mbmi_ext_frame: usize,
    /// `cpi->mbmi_ext_info.frame_base`, moved in (see the module
    /// documentation).
    pub(crate) mbmi_ext_frame_base: Vec<MbModeInfoExtFrame>,
    /// `x->cb_coef_buff`: an index into `cb_coeff_buffers`.
    pub(crate) cb_coef_buff: usize,
    /// `cpi->coeff_buffer_base`, moved in (see the module documentation).
    pub(crate) cb_coeff_buffers: Vec<CbCoeffBuffer>,
    /// Offset of current coding block's coeff buffer relative to the sb.
    pub(crate) cb_offset: [u16; PLANE_TYPES as usize],

    /// Current qindex (before being adjusted by delta_q_res) used to derive
    /// rdmult_delta_qindex.
    pub(crate) qindex: i32,
    /// Difference between frame-level qindex and current qindex.
    pub(crate) delta_qindex: i32,
    /// Difference between frame-level qindex and qindex used to
    /// compute rdmult (lambda).
    pub(crate) rdmult_delta_qindex: i32,
    /// Current qindex (before being adjusted by delta_q_res) used to derive
    /// rdmult_delta_qindex.
    pub(crate) rdmult_cur_qindex: i32,
    /// Rate-distortion multiplier.
    pub(crate) rdmult: i32,
    /// Intra only, per sb rd adjustment.
    pub(crate) intra_sb_rdmult_modifier: i32,
    /// Superblock level distortion propagation factor.
    pub(crate) rb: f64,
    /// Energy in the current source coding block. Used to calculate \ref rdmult
    pub(crate) mb_energy: i32,
    /// Energy in the current source superblock. Used to calculate \ref rdmult
    pub(crate) sb_energy_level: i32,

    /// The rate needed to signal a mode to the bitstream.
    pub(crate) mode_costs: Box<ModeCosts>,
    /// The rate needed to encode a new motion vector to the bitstream and some
    /// multipliers for motion search.
    pub(crate) mv_costs: Option<Box<MvCosts>>,
    /// The rate needed to signal the txfm coefficients to the bitstream.
    pub(crate) coeff_costs: Box<CoeffCosts>,

    /// A multiplier that converts mv cost to l2 error.
    pub(crate) errorperbit: i32,
    /// A multiplier that converts mv cost to l1 error.
    pub(crate) sadperbit: i32,
    /// Whether the current block is skipped by the segment level skip.
    pub(crate) seg_skip_block: i32,
    /// Number of segment 1 blocks
    /// Actual number of (4x4) blocks that were applied delta-q,
    /// for segment 1.
    pub(crate) actual_num_seg1_blocks: i32,
    /// Number of segment 2 blocks
    /// Actual number of (4x4) blocks that were applied delta-q,
    /// for segment 2.
    pub(crate) actual_num_seg2_blocks: i32,
    /// Number of zero motion vectors
    pub(crate) cnt_zeromv: i32,
    /// Flag to force zeromv-skip at superblock level, for nonrd path.
    pub(crate) force_zeromv_skip_for_sb: i32,
    /// Flag to force zeromv-skip at block level, for nonrd path.
    pub(crate) force_zeromv_skip_for_blk: i32,
    /// Previous segment id for which qmatrices were updated.
    pub(crate) prev_segment_id: i32,

    /// Information on the source content of the superblock.
    pub(crate) content_state_sb: ContentStateSb,
    /// Stores some partition-search related buffers.
    pub(crate) part_search_info: PartitionSearchInfo,
    /// Reference frame search pruning level of the non-RD mode search.
    pub(crate) nonrd_prune_ref_frame_search: i32,
    /// The fullpel motion vector limits of the current block.
    pub(crate) mv_limits: FullMvLimits,
    /// The samples of `cpi->source` (`buffer_alloc`), moved in while the
    /// frame is coded: the in-place temporal filter of
    /// `av1_source_content_sb()` writes them (see `with_source()`).
    pub(crate) source_alloc: Vec<u16>,
    /// The sum of absolute differences of the best reference motion vector
    /// candidate of each reference (`av1_mv_pred()`).
    pub(crate) pred_mv_sad: [i32; REF_FRAMES as usize],
    /// The best `pred_mv_sad` of the past and the future references.
    pub(crate) best_pred_mv_sad: [i32; 2],
    /// The sums of absolute differences of the first and second candidates.
    pub(crate) pred_mv0_sad: [i32; REF_FRAMES as usize],
    pub(crate) pred_mv1_sad: [i32; REF_FRAMES as usize],
    /// The largest candidate motion vector component of each reference
    /// (in full pixels).
    pub(crate) max_mv_context: [i32; REF_FRAMES as usize],
    /// The sum of squared errors of the last prediction of each reference.
    pub(crate) pred_sse: [u32; REF_FRAMES as usize],
    /// Warp motion samples buffer.
    pub(crate) warp_sample_info: [WarpSampleInfo; REF_FRAMES as usize],
    /// The adaptive mode search thresholds' factors
    /// (`thresh_freq_fact[BLOCK_SIZES_ALL][MAX_MODES]`).
    pub(crate) thresh_freq_fact: Box<[[i32; MAX_MODES as usize]; BLOCK_SIZES_ALL as usize]>,

    /// Whether to reuse the prediction for an inter block.
    pub(crate) reuse_inter_pred: bool,

    /// Parameters for transform size/type search.
    pub(crate) txfm_search_params: TxfmSearchParams,
    /// Results of the transform search.
    pub(crate) txfm_search_info: TxfmSearchInfo,

    /// Color sensitivity flags for the superblock.
    pub(crate) color_sensitivity_sb: [u8; 2],
    /// Color sensitivity flags for the superblock for golden reference.
    pub(crate) color_sensitivity_sb_g: [u8; 2],
    /// Color sensitivity flags for the coding block.
    pub(crate) color_sensitivity: [u8; 2],

    /// Variance of the source frame.
    pub(crate) source_variance: u32,

    /// Determines the RD model used by the transform search
    /// (`FULL_TXFM_RD` or `LOW_TXFM_RD`).
    pub(crate) rd_model: i32,
    /// The scratch buffer `search_tx_type()` swaps with a plane's
    /// `dqcoeff` (upstream's `this_dqcoeff`, on its stack).
    pub(crate) search_tx_dqcoeff: Vec<TranLowT>,
    /// The low precision coefficients of `block_yrd()`'s dual Hadamard
    /// transform (upstream's `p->coeff` of the luma plane, viewed as an
    /// `int16_t` array).
    pub(crate) coeff_lp: Vec<i16>,
    /// `row_ctx`: the tile's contexts for the rows below (row
    /// multi-threading), moved in from the tile data while the tile is
    /// coded.
    pub(crate) row_ctx: Vec<FrameContext>,
}

impl Default for Macroblock {
    fn default() -> Self {
        Macroblock {
            plane: [
                MacroblockPlane::default(),
                MacroblockPlane::default(),
                MacroblockPlane::default(),
            ],
            e_mbd: MacroBlockD::default(),
            mbmi_ext: MbModeInfoExt::default(),
            mbmi_ext_frame: 0,
            mbmi_ext_frame_base: Vec::new(),
            cb_coef_buff: 0,
            cb_coeff_buffers: Vec::new(),
            cb_offset: [0; PLANE_TYPES as usize],
            qindex: 0,
            delta_qindex: 0,
            rdmult_delta_qindex: 0,
            rdmult_cur_qindex: 0,
            rdmult: 0,
            intra_sb_rdmult_modifier: 0,
            rb: 0.0,
            mb_energy: 0,
            sb_energy_level: 0,
            mode_costs: Box::default(),
            mv_costs: None,
            coeff_costs: Box::default(),
            errorperbit: 0,
            sadperbit: 0,
            seg_skip_block: 0,
            actual_num_seg1_blocks: 0,
            actual_num_seg2_blocks: 0,
            cnt_zeromv: 0,
            force_zeromv_skip_for_sb: 0,
            force_zeromv_skip_for_blk: 0,
            prev_segment_id: 0,
            content_state_sb: ContentStateSb::default(),
            part_search_info: PartitionSearchInfo::default(),
            nonrd_prune_ref_frame_search: 0,
            mv_limits: FullMvLimits::default(),
            source_alloc: Vec::new(),
            pred_mv_sad: [0; REF_FRAMES as usize],
            best_pred_mv_sad: [0; 2],
            pred_mv0_sad: [0; REF_FRAMES as usize],
            pred_mv1_sad: [0; REF_FRAMES as usize],
            max_mv_context: [0; REF_FRAMES as usize],
            pred_sse: [0; REF_FRAMES as usize],
            warp_sample_info: [WarpSampleInfo::default(); REF_FRAMES as usize],
            thresh_freq_fact: Box::new([[0; MAX_MODES as usize]; BLOCK_SIZES_ALL as usize]),
            reuse_inter_pred: false,
            txfm_search_params: TxfmSearchParams::default(),
            txfm_search_info: TxfmSearchInfo::default(),
            color_sensitivity_sb: [0; 2],
            color_sensitivity_sb_g: [0; 2],
            color_sensitivity: [0; 2],
            source_variance: 0,
            rd_model: 0,
            search_tx_dqcoeff: Vec::new(),
            coeff_lp: Vec::new(),
            row_ctx: Vec::new(),
        }
    }
}

/// Runs `f` on the macroblock and the source samples (`x.source_alloc`,
/// moved out of `x` while `f` runs, so `f` must not read them through
/// `x`).
#[inline]
pub(crate) fn with_source<R>(
    x: &mut Macroblock,
    f: impl FnOnce(&mut Macroblock, &[u16]) -> R,
) -> R {
    let src = std::mem::take(&mut x.source_alloc);
    let r = f(x, &src);
    x.source_alloc = src;
    r
}

// Only consider full SB, MAX_SB_SIZE_LOG2 - 1 = 6 (64x64)
#[inline]
pub(crate) fn tx_size_to_depth(tx_size: TxSize, bsize: BlockSize) -> i32 {
    let mut ctx_size = MAX_TXSIZE_RECT_LOOKUP[bsize as usize];
    let mut depth = 0;
    while tx_size != ctx_size {
        depth += 1;
        ctx_size = SUB_TX_SIZE_MAP[ctx_size as usize];
        debug_assert!(depth <= MAX_TX_DEPTH);
    }
    depth
}

/// Translation of `set_blk_skip()`.
#[inline]
pub(crate) fn set_blk_skip(txb_skip: &mut [u8], plane: usize, blk_idx: usize, skip: bool) {
    if skip {
        txb_skip[blk_idx] |= 1 << plane;
    } else {
        txb_skip[blk_idx] &= !(1 << plane);
    }
}

#[inline]
pub(crate) fn is_blk_skip(txb_skip: &[u8], plane: usize, blk_idx: usize) -> bool {
    (txb_skip[blk_idx] >> plane) & 1 != 0
}
