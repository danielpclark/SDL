// Rust translation of av1/common/entropymode.c and av1/common/entropymode.h
// from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The frame context (`FRAME_CONTEXT`) and its defaults.

use super::av1_common_int::Av1Common;
use super::entropy::*;
use super::entropymv::{NmvContext, DEFAULT_NMV_CONTEXT};
use super::enums::*;
use super::filter::{SWITCHABLE_FILTERS, SWITCHABLE_FILTER_CONTEXTS};
use super::seg_common::{
    SegmentationProbs, MAX_SEGMENTS, SEG_TEMPORAL_PRED_CTXS, SPATIAL_PREDICTION_PROBS,
};
use crate::aom::aom_dsp::prob::cdf_pad;

pub(crate) const BLOCK_SIZE_GROUPS: i32 = 4;

pub(crate) const TX_SIZE_CONTEXTS: i32 = 3;

pub(crate) const PALETTE_COLOR_INDEX_CONTEXTS: i32 = 5;

pub(crate) const PALETTE_Y_MODE_CONTEXTS: i32 = 3;

pub(crate) const PALETTE_UV_MODE_CONTEXTS: i32 = 2;

// Map the number of pixels in a block size to a context
//   64(BLOCK_8X8, BLOCK_4x16, BLOCK_16X4)  -> 0
//  128(BLOCK_8X16, BLOCK_16x8)             -> 1
//   ...
// 4096(BLOCK_64X64)                        -> 6
pub(crate) const PALATTE_BSIZE_CTXS: i32 = 7;

pub(crate) const MAX_COLOR_CONTEXT_HASH: i32 = 8;

pub(crate) const NUM_PALETTE_NEIGHBORS: i32 = 3; // left, top-left and top.

pub(crate) const KF_MODE_CONTEXTS: i32 = 5;

/// Translation of `FRAME_CONTEXT` (`struct frame_contexts`).
#[derive(Debug, Clone)]
pub(crate) struct FrameContext {
    pub(crate) txb_skip_cdf: [[[u16; 2 + 1]; TXB_SKIP_CONTEXTS as usize]; TX_SIZES as usize],
    pub(crate) eob_extra_cdf:
        [[[[u16; 2 + 1]; EOB_COEF_CONTEXTS as usize]; PLANE_TYPES as usize]; TX_SIZES as usize],
    pub(crate) dc_sign_cdf: [[[u16; 2 + 1]; DC_SIGN_CONTEXTS as usize]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf16: [[[u16; 5 + 1]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf32: [[[u16; 6 + 1]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf64: [[[u16; 7 + 1]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf128: [[[u16; 8 + 1]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf256: [[[u16; 9 + 1]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf512: [[[u16; 10 + 1]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf1024: [[[u16; 11 + 1]; 2]; PLANE_TYPES as usize],
    pub(crate) coeff_base_eob_cdf:
        [[[[u16; 3 + 1]; SIG_COEF_CONTEXTS_EOB as usize]; PLANE_TYPES as usize]; TX_SIZES as usize],
    pub(crate) coeff_base_cdf:
        [[[[u16; 4 + 1]; SIG_COEF_CONTEXTS as usize]; PLANE_TYPES as usize]; TX_SIZES as usize],
    pub(crate) coeff_br_cdf: [[[[u16; BR_CDF_SIZE as usize + 1]; LEVEL_CONTEXTS as usize];
        PLANE_TYPES as usize]; TX_SIZES as usize],
    pub(crate) newmv_cdf: [[u16; 2 + 1]; NEWMV_MODE_CONTEXTS as usize],
    pub(crate) zeromv_cdf: [[u16; 2 + 1]; GLOBALMV_MODE_CONTEXTS as usize],
    pub(crate) refmv_cdf: [[u16; 2 + 1]; REFMV_MODE_CONTEXTS as usize],
    pub(crate) drl_cdf: [[u16; 2 + 1]; DRL_MODE_CONTEXTS as usize],
    pub(crate) inter_compound_mode_cdf:
        [[u16; INTER_COMPOUND_MODES as usize + 1]; INTER_MODE_CONTEXTS as usize],
    pub(crate) compound_type_cdf:
        [[u16; MASKED_COMPOUND_TYPES as usize + 1]; BLOCK_SIZES_ALL as usize],
    pub(crate) wedge_idx_cdf: [[u16; 16 + 1]; BLOCK_SIZES_ALL as usize],
    pub(crate) interintra_cdf: [[u16; 2 + 1]; BLOCK_SIZE_GROUPS as usize],
    pub(crate) wedge_interintra_cdf: [[u16; 2 + 1]; BLOCK_SIZES_ALL as usize],
    pub(crate) interintra_mode_cdf:
        [[u16; INTERINTRA_MODES as usize + 1]; BLOCK_SIZE_GROUPS as usize],
    pub(crate) motion_mode_cdf: [[u16; MOTION_MODES as usize + 1]; BLOCK_SIZES_ALL as usize],
    pub(crate) obmc_cdf: [[u16; 2 + 1]; BLOCK_SIZES_ALL as usize],
    pub(crate) palette_y_size_cdf: [[u16; PALETTE_SIZES as usize + 1]; PALATTE_BSIZE_CTXS as usize],
    pub(crate) palette_uv_size_cdf:
        [[u16; PALETTE_SIZES as usize + 1]; PALATTE_BSIZE_CTXS as usize],
    pub(crate) palette_y_color_index_cdf: [[[u16; PALETTE_COLORS as usize + 1];
        PALETTE_COLOR_INDEX_CONTEXTS as usize];
        PALETTE_SIZES as usize],
    pub(crate) palette_uv_color_index_cdf: [[[u16; PALETTE_COLORS as usize + 1];
        PALETTE_COLOR_INDEX_CONTEXTS as usize];
        PALETTE_SIZES as usize],
    pub(crate) palette_y_mode_cdf:
        [[[u16; 2 + 1]; PALETTE_Y_MODE_CONTEXTS as usize]; PALATTE_BSIZE_CTXS as usize],
    pub(crate) palette_uv_mode_cdf: [[u16; 2 + 1]; PALETTE_UV_MODE_CONTEXTS as usize],
    pub(crate) comp_inter_cdf: [[u16; 2 + 1]; COMP_INTER_CONTEXTS as usize],
    pub(crate) single_ref_cdf: [[[u16; 2 + 1]; (SINGLE_REFS - 1) as usize]; REF_CONTEXTS as usize],
    pub(crate) comp_ref_type_cdf: [[u16; 2 + 1]; COMP_REF_TYPE_CONTEXTS as usize],
    pub(crate) uni_comp_ref_cdf:
        [[[u16; 2 + 1]; (UNIDIR_COMP_REFS - 1) as usize]; UNI_COMP_REF_CONTEXTS as usize],
    pub(crate) comp_ref_cdf: [[[u16; 2 + 1]; (FWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
    pub(crate) comp_bwdref_cdf: [[[u16; 2 + 1]; (BWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
    pub(crate) txfm_partition_cdf: [[u16; 2 + 1]; TXFM_PARTITION_CONTEXTS as usize],
    pub(crate) compound_index_cdf: [[u16; 2 + 1]; COMP_INDEX_CONTEXTS as usize],
    pub(crate) comp_group_idx_cdf: [[u16; 2 + 1]; COMP_GROUP_IDX_CONTEXTS as usize],
    pub(crate) skip_mode_cdfs: [[u16; 2 + 1]; SKIP_MODE_CONTEXTS as usize],
    pub(crate) skip_txfm_cdfs: [[u16; 2 + 1]; SKIP_CONTEXTS as usize],
    pub(crate) intra_inter_cdf: [[u16; 2 + 1]; INTRA_INTER_CONTEXTS as usize],
    pub(crate) nmvc: NmvContext,
    pub(crate) ndvc: NmvContext,
    pub(crate) intrabc_cdf: [u16; 2 + 1],
    pub(crate) seg: SegmentationProbs,
    pub(crate) filter_intra_cdfs: [[u16; 2 + 1]; BLOCK_SIZES_ALL as usize],
    pub(crate) filter_intra_mode_cdf: [u16; FILTER_INTRA_MODES as usize + 1],
    pub(crate) switchable_restore_cdf: [u16; RESTORE_SWITCHABLE_TYPES as usize + 1],
    pub(crate) wiener_restore_cdf: [u16; 2 + 1],
    pub(crate) sgrproj_restore_cdf: [u16; 2 + 1],
    pub(crate) y_mode_cdf: [[u16; INTRA_MODES as usize + 1]; BLOCK_SIZE_GROUPS as usize],
    pub(crate) uv_mode_cdf:
        [[[u16; UV_INTRA_MODES as usize + 1]; INTRA_MODES as usize]; CFL_ALLOWED_TYPES as usize],
    pub(crate) partition_cdf:
        [[u16; EXT_PARTITION_TYPES as usize + 1]; PARTITION_CONTEXTS as usize],
    pub(crate) switchable_interp_cdf:
        [[u16; SWITCHABLE_FILTERS as usize + 1]; SWITCHABLE_FILTER_CONTEXTS as usize],
    /* kf_y_cdf is discarded after use, so does not require persistent storage.
       However, we keep it with the other CDFs in this struct since it needs to
       be copied to each tile to support parallelism just like the others.
    */
    pub(crate) kf_y_cdf:
        [[[u16; INTRA_MODES as usize + 1]; KF_MODE_CONTEXTS as usize]; KF_MODE_CONTEXTS as usize],
    pub(crate) angle_delta_cdf:
        [[u16; (2 * MAX_ANGLE_DELTA + 1) as usize + 1]; DIRECTIONAL_MODES as usize],
    pub(crate) tx_size_cdf:
        [[[u16; (MAX_TX_DEPTH + 1) as usize + 1]; TX_SIZE_CONTEXTS as usize]; MAX_TX_CATS as usize],
    pub(crate) delta_q_cdf: [u16; (DELTA_Q_PROBS + 1) as usize + 1],
    pub(crate) delta_lf_multi_cdf:
        [[u16; (DELTA_LF_PROBS + 1) as usize + 1]; FRAME_LF_COUNT as usize],
    pub(crate) delta_lf_cdf: [u16; (DELTA_LF_PROBS + 1) as usize + 1],
    pub(crate) intra_ext_tx_cdf: [[[[u16; TX_TYPES as usize + 1]; INTRA_MODES as usize];
        EXT_TX_SIZES as usize]; EXT_TX_SETS_INTRA as usize],
    pub(crate) inter_ext_tx_cdf:
        [[[u16; TX_TYPES as usize + 1]; EXT_TX_SIZES as usize]; EXT_TX_SETS_INTER as usize],
    pub(crate) cfl_sign_cdf: [u16; CFL_JOINT_SIGNS as usize + 1],
    pub(crate) cfl_alpha_cdf: [[u16; CFL_ALPHABET_SIZE as usize + 1]; CFL_ALPHA_CONTEXTS as usize],
    pub(crate) initialized: i32,
}

impl Default for FrameContext {
    fn default() -> Self {
        FrameContext {
            txb_skip_cdf: [[[0; 2 + 1]; TXB_SKIP_CONTEXTS as usize]; TX_SIZES as usize],
            eob_extra_cdf: [[[[0; 2 + 1]; EOB_COEF_CONTEXTS as usize]; PLANE_TYPES as usize];
                TX_SIZES as usize],
            dc_sign_cdf: [[[0; 2 + 1]; DC_SIGN_CONTEXTS as usize]; PLANE_TYPES as usize],
            eob_flag_cdf16: [[[0; 5 + 1]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf32: [[[0; 6 + 1]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf64: [[[0; 7 + 1]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf128: [[[0; 8 + 1]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf256: [[[0; 9 + 1]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf512: [[[0; 10 + 1]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf1024: [[[0; 11 + 1]; 2]; PLANE_TYPES as usize],
            coeff_base_eob_cdf: [[[[0; 3 + 1]; SIG_COEF_CONTEXTS_EOB as usize];
                PLANE_TYPES as usize]; TX_SIZES as usize],
            coeff_base_cdf: [[[[0; 4 + 1]; SIG_COEF_CONTEXTS as usize]; PLANE_TYPES as usize];
                TX_SIZES as usize],
            coeff_br_cdf: [[[[0; BR_CDF_SIZE as usize + 1]; LEVEL_CONTEXTS as usize];
                PLANE_TYPES as usize]; TX_SIZES as usize],
            newmv_cdf: [[0; 2 + 1]; NEWMV_MODE_CONTEXTS as usize],
            zeromv_cdf: [[0; 2 + 1]; GLOBALMV_MODE_CONTEXTS as usize],
            refmv_cdf: [[0; 2 + 1]; REFMV_MODE_CONTEXTS as usize],
            drl_cdf: [[0; 2 + 1]; DRL_MODE_CONTEXTS as usize],
            inter_compound_mode_cdf: [[0; INTER_COMPOUND_MODES as usize + 1];
                INTER_MODE_CONTEXTS as usize],
            compound_type_cdf: [[0; MASKED_COMPOUND_TYPES as usize + 1]; BLOCK_SIZES_ALL as usize],
            wedge_idx_cdf: [[0; 16 + 1]; BLOCK_SIZES_ALL as usize],
            interintra_cdf: [[0; 2 + 1]; BLOCK_SIZE_GROUPS as usize],
            wedge_interintra_cdf: [[0; 2 + 1]; BLOCK_SIZES_ALL as usize],
            interintra_mode_cdf: [[0; INTERINTRA_MODES as usize + 1]; BLOCK_SIZE_GROUPS as usize],
            motion_mode_cdf: [[0; MOTION_MODES as usize + 1]; BLOCK_SIZES_ALL as usize],
            obmc_cdf: [[0; 2 + 1]; BLOCK_SIZES_ALL as usize],
            palette_y_size_cdf: [[0; PALETTE_SIZES as usize + 1]; PALATTE_BSIZE_CTXS as usize],
            palette_uv_size_cdf: [[0; PALETTE_SIZES as usize + 1]; PALATTE_BSIZE_CTXS as usize],
            palette_y_color_index_cdf: [[[0; PALETTE_COLORS as usize + 1];
                PALETTE_COLOR_INDEX_CONTEXTS as usize];
                PALETTE_SIZES as usize],
            palette_uv_color_index_cdf: [[[0; PALETTE_COLORS as usize + 1];
                PALETTE_COLOR_INDEX_CONTEXTS as usize];
                PALETTE_SIZES as usize],
            palette_y_mode_cdf: [[[0; 2 + 1]; PALETTE_Y_MODE_CONTEXTS as usize];
                PALATTE_BSIZE_CTXS as usize],
            palette_uv_mode_cdf: [[0; 2 + 1]; PALETTE_UV_MODE_CONTEXTS as usize],
            comp_inter_cdf: [[0; 2 + 1]; COMP_INTER_CONTEXTS as usize],
            single_ref_cdf: [[[0; 2 + 1]; (SINGLE_REFS - 1) as usize]; REF_CONTEXTS as usize],
            comp_ref_type_cdf: [[0; 2 + 1]; COMP_REF_TYPE_CONTEXTS as usize],
            uni_comp_ref_cdf: [[[0; 2 + 1]; (UNIDIR_COMP_REFS - 1) as usize];
                UNI_COMP_REF_CONTEXTS as usize],
            comp_ref_cdf: [[[0; 2 + 1]; (FWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
            comp_bwdref_cdf: [[[0; 2 + 1]; (BWD_REFS - 1) as usize]; REF_CONTEXTS as usize],
            txfm_partition_cdf: [[0; 2 + 1]; TXFM_PARTITION_CONTEXTS as usize],
            compound_index_cdf: [[0; 2 + 1]; COMP_INDEX_CONTEXTS as usize],
            comp_group_idx_cdf: [[0; 2 + 1]; COMP_GROUP_IDX_CONTEXTS as usize],
            skip_mode_cdfs: [[0; 2 + 1]; SKIP_MODE_CONTEXTS as usize],
            skip_txfm_cdfs: [[0; 2 + 1]; SKIP_CONTEXTS as usize],
            intra_inter_cdf: [[0; 2 + 1]; INTRA_INTER_CONTEXTS as usize],
            nmvc: NmvContext::default(),
            ndvc: NmvContext::default(),
            intrabc_cdf: [0; 2 + 1],
            seg: SegmentationProbs::default(),
            filter_intra_cdfs: [[0; 2 + 1]; BLOCK_SIZES_ALL as usize],
            filter_intra_mode_cdf: [0; FILTER_INTRA_MODES as usize + 1],
            switchable_restore_cdf: [0; RESTORE_SWITCHABLE_TYPES as usize + 1],
            wiener_restore_cdf: [0; 2 + 1],
            sgrproj_restore_cdf: [0; 2 + 1],
            y_mode_cdf: [[0; INTRA_MODES as usize + 1]; BLOCK_SIZE_GROUPS as usize],
            uv_mode_cdf: [[[0; UV_INTRA_MODES as usize + 1]; INTRA_MODES as usize];
                CFL_ALLOWED_TYPES as usize],
            partition_cdf: [[0; EXT_PARTITION_TYPES as usize + 1]; PARTITION_CONTEXTS as usize],
            switchable_interp_cdf: [[0; SWITCHABLE_FILTERS as usize + 1];
                SWITCHABLE_FILTER_CONTEXTS as usize],
            kf_y_cdf: [[[0; INTRA_MODES as usize + 1]; KF_MODE_CONTEXTS as usize];
                KF_MODE_CONTEXTS as usize],
            angle_delta_cdf: [[0; (2 * MAX_ANGLE_DELTA + 1) as usize + 1];
                DIRECTIONAL_MODES as usize],
            tx_size_cdf: [[[0; (MAX_TX_DEPTH + 1) as usize + 1]; TX_SIZE_CONTEXTS as usize];
                MAX_TX_CATS as usize],
            delta_q_cdf: [0; (DELTA_Q_PROBS + 1) as usize + 1],
            delta_lf_multi_cdf: [[0; (DELTA_LF_PROBS + 1) as usize + 1]; FRAME_LF_COUNT as usize],
            delta_lf_cdf: [0; (DELTA_LF_PROBS + 1) as usize + 1],
            intra_ext_tx_cdf: [[[[0; TX_TYPES as usize + 1]; INTRA_MODES as usize];
                EXT_TX_SIZES as usize]; EXT_TX_SETS_INTRA as usize],
            inter_ext_tx_cdf: [[[0; TX_TYPES as usize + 1]; EXT_TX_SIZES as usize];
                EXT_TX_SETS_INTER as usize],
            cfl_sign_cdf: [0; CFL_JOINT_SIGNS as usize + 1],
            cfl_alpha_cdf: [[0; CFL_ALPHABET_SIZE as usize + 1]; CFL_ALPHA_CONTEXTS as usize],
            initialized: 0,
        }
    }
}

pub(crate) static AV1_EXT_TX_IND: [[i32; TX_TYPES as usize]; EXT_TX_SET_TYPES as usize] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 3, 4, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 5, 6, 4, 0, 0, 0, 0, 0, 0, 2, 3, 0, 0, 0, 0],
    [3, 4, 5, 8, 6, 7, 9, 10, 11, 0, 1, 2, 0, 0, 0, 0],
    [7, 8, 9, 12, 10, 11, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6],
];

pub(crate) static AV1_EXT_TX_INV: [[i32; TX_TYPES as usize]; EXT_TX_SET_TYPES as usize] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [9, 0, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [9, 0, 10, 11, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [9, 10, 11, 0, 1, 2, 4, 5, 3, 6, 7, 8, 0, 0, 0, 0],
    [9, 10, 11, 12, 13, 14, 15, 0, 1, 2, 4, 5, 3, 6, 7, 8],
];

/// Translation of `av1_init_mode_probs()`.
pub(crate) fn av1_init_mode_probs(fc: &mut FrameContext) {
    fc.palette_y_size_cdf = DEFAULT_PALETTE_Y_SIZE_CDF;
    fc.palette_uv_size_cdf = DEFAULT_PALETTE_UV_SIZE_CDF;
    fc.palette_y_color_index_cdf = DEFAULT_PALETTE_Y_COLOR_INDEX_CDF;
    fc.palette_uv_color_index_cdf = DEFAULT_PALETTE_UV_COLOR_INDEX_CDF;
    fc.kf_y_cdf = DEFAULT_KF_Y_MODE_CDF;
    fc.angle_delta_cdf = DEFAULT_ANGLE_DELTA_CDF;
    fc.comp_inter_cdf = DEFAULT_COMP_INTER_CDF;
    fc.comp_ref_type_cdf = DEFAULT_COMP_REF_TYPE_CDF;
    fc.uni_comp_ref_cdf = DEFAULT_UNI_COMP_REF_CDF;
    fc.palette_y_mode_cdf = DEFAULT_PALETTE_Y_MODE_CDF;
    fc.palette_uv_mode_cdf = DEFAULT_PALETTE_UV_MODE_CDF;
    fc.comp_ref_cdf = DEFAULT_COMP_REF_CDF;
    fc.comp_bwdref_cdf = DEFAULT_COMP_BWDREF_CDF;
    fc.single_ref_cdf = DEFAULT_SINGLE_REF_CDF;
    fc.txfm_partition_cdf = DEFAULT_TXFM_PARTITION_CDF;
    fc.compound_index_cdf = DEFAULT_COMPOUND_IDX_CDFS;
    fc.comp_group_idx_cdf = DEFAULT_COMP_GROUP_IDX_CDFS;
    fc.newmv_cdf = DEFAULT_NEWMV_CDF;
    fc.zeromv_cdf = DEFAULT_ZEROMV_CDF;
    fc.refmv_cdf = DEFAULT_REFMV_CDF;
    fc.drl_cdf = DEFAULT_DRL_CDF;
    fc.motion_mode_cdf = DEFAULT_MOTION_MODE_CDF;
    fc.obmc_cdf = DEFAULT_OBMC_CDF;
    fc.inter_compound_mode_cdf = DEFAULT_INTER_COMPOUND_MODE_CDF;
    fc.compound_type_cdf = DEFAULT_COMPOUND_TYPE_CDF;
    fc.wedge_idx_cdf = DEFAULT_WEDGE_IDX_CDF;
    fc.interintra_cdf = DEFAULT_INTERINTRA_CDF;
    fc.wedge_interintra_cdf = DEFAULT_WEDGE_INTERINTRA_CDF;
    fc.interintra_mode_cdf = DEFAULT_INTERINTRA_MODE_CDF;
    fc.seg.pred_cdf = DEFAULT_SEGMENT_PRED_CDF;
    fc.filter_intra_cdfs = DEFAULT_FILTER_INTRA_CDFS;
    fc.filter_intra_mode_cdf = DEFAULT_FILTER_INTRA_MODE_CDF;
    fc.switchable_restore_cdf = DEFAULT_SWITCHABLE_RESTORE_CDF;
    fc.wiener_restore_cdf = DEFAULT_WIENER_RESTORE_CDF;
    fc.sgrproj_restore_cdf = DEFAULT_SGRPROJ_RESTORE_CDF;
    fc.y_mode_cdf = DEFAULT_IF_Y_MODE_CDF;
    fc.uv_mode_cdf = DEFAULT_UV_MODE_CDF;
    fc.switchable_interp_cdf = DEFAULT_SWITCHABLE_INTERP_CDF;
    fc.partition_cdf = DEFAULT_PARTITION_CDF;
    fc.intra_ext_tx_cdf = DEFAULT_INTRA_EXT_TX_CDF;
    fc.inter_ext_tx_cdf = DEFAULT_INTER_EXT_TX_CDF;
    fc.skip_mode_cdfs = DEFAULT_SKIP_MODE_CDFS;
    fc.skip_txfm_cdfs = DEFAULT_SKIP_TXFM_CDFS;
    fc.intra_inter_cdf = DEFAULT_INTRA_INTER_CDF;
    fc.seg.spatial_pred_seg_cdf = DEFAULT_SPATIAL_PRED_SEG_TREE_CDF;
    fc.tx_size_cdf = DEFAULT_TX_SIZE_CDF;
    fc.delta_q_cdf = DEFAULT_DELTA_Q_CDF;
    fc.delta_lf_cdf = DEFAULT_DELTA_LF_CDF;
    fc.delta_lf_multi_cdf = DEFAULT_DELTA_LF_MULTI_CDF;
    fc.cfl_sign_cdf = DEFAULT_CFL_SIGN_CDF;
    fc.cfl_alpha_cdf = DEFAULT_CFL_ALPHA_CDF;
    fc.intrabc_cdf = DEFAULT_INTRABC_CDF;
}

pub(crate) fn av1_set_default_ref_deltas(ref_deltas: &mut [i8; REF_FRAMES as usize]) {
    ref_deltas[INTRA_FRAME as usize] = 1;
    ref_deltas[LAST_FRAME as usize] = 0;
    ref_deltas[LAST2_FRAME as usize] = ref_deltas[LAST_FRAME as usize];
    ref_deltas[LAST3_FRAME as usize] = ref_deltas[LAST_FRAME as usize];
    ref_deltas[BWDREF_FRAME as usize] = ref_deltas[LAST_FRAME as usize];
    ref_deltas[GOLDEN_FRAME as usize] = -1;
    ref_deltas[ALTREF2_FRAME as usize] = -1;
    ref_deltas[ALTREF_FRAME as usize] = -1;
}

pub(crate) fn av1_set_default_mode_deltas(mode_deltas: &mut [i8; MAX_MODE_LF_DELTAS as usize]) {
    mode_deltas[0] = 0;
    mode_deltas[1] = 0;
}

fn set_default_lf_deltas(lf: &mut super::av1_loopfilter::Loopfilter) {
    lf.mode_ref_delta_enabled = 1;
    lf.mode_ref_delta_update = 1;

    av1_set_default_ref_deltas(&mut lf.ref_deltas);
    av1_set_default_mode_deltas(&mut lf.mode_deltas);
}

/// Translation of `av1_init_mv_probs()` (av1/common/entropymv.c).
pub(crate) fn av1_init_mv_probs(cm: &mut Av1Common) {
    // NB: this sets CDFs too
    cm.fc.nmvc = DEFAULT_NMV_CONTEXT;
    cm.fc.ndvc = DEFAULT_NMV_CONTEXT;
}

/// Translation of `av1_setup_frame_contexts()` (no large scale tiles).
pub(crate) fn av1_setup_frame_contexts(cm: &mut Av1Common) {
    // Store the frame context into a special slot (not associated with any
    // reference buffer), so that we can set up cm->pre_fc correctly later
    // This function must ONLY be called when cm->fc has been initialized with
    // default probs, either by av1_setup_past_independence or after manually
    // initializing them
    cm.default_frame_context = cm.fc.clone();
}

/// Translation of `av1_setup_past_independence()` (no segmentation map).
pub(crate) fn av1_setup_past_independence(cm: &mut Av1Common) {
    // Reset the segment feature data to the default stats:
    // Features disabled, 0, with delta coding (Default state).
    super::seg_common::av1_clearall_segfeatures(&mut cm.seg);

    // reset mode ref deltas
    av1_set_default_ref_deltas(&mut cm.cur_frame_mut().ref_deltas);
    av1_set_default_mode_deltas(&mut cm.cur_frame_mut().mode_deltas);
    set_default_lf_deltas(&mut cm.lf);

    super::entropy::av1_default_coef_probs(cm);
    av1_init_mode_probs(&mut cm.fc);
    av1_init_mv_probs(cm);
    cm.fc.initialized = 1;
    av1_setup_frame_contexts(cm);
}

// (the default CDFs)

pub(crate) static DEFAULT_KF_Y_MODE_CDF: [[[u16; (INTRA_MODES) as usize + 1];
    KF_MODE_CONTEXTS as usize];
    KF_MODE_CONTEXTS as usize] = [
    [
        cdf_pad(&[
            15588, 17027, 19338, 20218, 20682, 21110, 21825, 23244, 24189, 28165, 29093, 30466,
        ]),
        cdf_pad(&[
            12016, 18066, 19516, 20303, 20719, 21444, 21888, 23032, 24434, 28658, 30172, 31409,
        ]),
        cdf_pad(&[
            10052, 10771, 22296, 22788, 23055, 23239, 24133, 25620, 26160, 29336, 29929, 31567,
        ]),
        cdf_pad(&[
            14091, 15406, 16442, 18808, 19136, 19546, 19998, 22096, 24746, 29585, 30958, 32462,
        ]),
        cdf_pad(&[
            12122, 13265, 15603, 16501, 18609, 20033, 22391, 25583, 26437, 30261, 31073, 32475,
        ]),
    ],
    [
        cdf_pad(&[
            10023, 19585, 20848, 21440, 21832, 22760, 23089, 24023, 25381, 29014, 30482, 31436,
        ]),
        cdf_pad(&[
            5983, 24099, 24560, 24886, 25066, 25795, 25913, 26423, 27610, 29905, 31276, 31794,
        ]),
        cdf_pad(&[
            7444, 12781, 20177, 20728, 21077, 21607, 22170, 23405, 24469, 27915, 29090, 30492,
        ]),
        cdf_pad(&[
            8537, 14689, 15432, 17087, 17408, 18172, 18408, 19825, 24649, 29153, 31096, 32210,
        ]),
        cdf_pad(&[
            7543, 14231, 15496, 16195, 17905, 20717, 21984, 24516, 26001, 29675, 30981, 31994,
        ]),
    ],
    [
        cdf_pad(&[
            12613, 13591, 21383, 22004, 22312, 22577, 23401, 25055, 25729, 29538, 30305, 32077,
        ]),
        cdf_pad(&[
            9687, 13470, 18506, 19230, 19604, 20147, 20695, 22062, 23219, 27743, 29211, 30907,
        ]),
        cdf_pad(&[
            6183, 6505, 26024, 26252, 26366, 26434, 27082, 28354, 28555, 30467, 30794, 32086,
        ]),
        cdf_pad(&[
            10718, 11734, 14954, 17224, 17565, 17924, 18561, 21523, 23878, 28975, 30287, 32252,
        ]),
        cdf_pad(&[
            9194, 9858, 16501, 17263, 18424, 19171, 21563, 25961, 26561, 30072, 30737, 32463,
        ]),
    ],
    [
        cdf_pad(&[
            12602, 14399, 15488, 18381, 18778, 19315, 19724, 21419, 25060, 29696, 30917, 32409,
        ]),
        cdf_pad(&[
            8203, 13821, 14524, 17105, 17439, 18131, 18404, 19468, 25225, 29485, 31158, 32342,
        ]),
        cdf_pad(&[
            8451, 9731, 15004, 17643, 18012, 18425, 19070, 21538, 24605, 29118, 30078, 32018,
        ]),
        cdf_pad(&[
            7714, 9048, 9516, 16667, 16817, 16994, 17153, 18767, 26743, 30389, 31536, 32528,
        ]),
        cdf_pad(&[
            8843, 10280, 11496, 15317, 16652, 17943, 19108, 22718, 25769, 29953, 30983, 32485,
        ]),
    ],
    [
        cdf_pad(&[
            12578, 13671, 15979, 16834, 19075, 20913, 22989, 25449, 26219, 30214, 31150, 32477,
        ]),
        cdf_pad(&[
            9563, 13626, 15080, 15892, 17756, 20863, 22207, 24236, 25380, 29653, 31143, 32277,
        ]),
        cdf_pad(&[
            8356, 8901, 17616, 18256, 19350, 20106, 22598, 25947, 26466, 29900, 30523, 32261,
        ]),
        cdf_pad(&[
            10835, 11815, 13124, 16042, 17018, 18039, 18947, 22753, 24615, 29489, 30883, 32482,
        ]),
        cdf_pad(&[
            7618, 8288, 9859, 10509, 15386, 18657, 22903, 28776, 29180, 31355, 31802, 32593,
        ]),
    ],
];

pub(crate) static DEFAULT_ANGLE_DELTA_CDF: [[u16; (2 * MAX_ANGLE_DELTA + 1) as usize + 1];
    DIRECTIONAL_MODES as usize] = [
    cdf_pad(&[2180, 5032, 7567, 22776, 26989, 30217]),
    cdf_pad(&[2301, 5608, 8801, 23487, 26974, 30330]),
    cdf_pad(&[3780, 11018, 13699, 19354, 23083, 31286]),
    cdf_pad(&[4581, 11226, 15147, 17138, 21834, 28397]),
    cdf_pad(&[1737, 10927, 14509, 19588, 22745, 28823]),
    cdf_pad(&[2664, 10176, 12485, 17650, 21600, 30495]),
    cdf_pad(&[2240, 11096, 15453, 20341, 22561, 28917]),
    cdf_pad(&[3605, 10428, 12459, 17676, 21244, 30655]),
];

pub(crate) static DEFAULT_IF_Y_MODE_CDF: [[u16; (INTRA_MODES) as usize + 1];
    BLOCK_SIZE_GROUPS as usize] = [
    cdf_pad(&[
        22801, 23489, 24293, 24756, 25601, 26123, 26606, 27418, 27945, 29228, 29685, 30349,
    ]),
    cdf_pad(&[
        18673, 19845, 22631, 23318, 23950, 24649, 25527, 27364, 28152, 29701, 29984, 30852,
    ]),
    cdf_pad(&[
        19770, 20979, 23396, 23939, 24241, 24654, 25136, 27073, 27830, 29360, 29730, 30659,
    ]),
    cdf_pad(&[
        20155, 21301, 22838, 23178, 23261, 23533, 23703, 24804, 25352, 26575, 27016, 28049,
    ]),
];

pub(crate) static DEFAULT_UV_MODE_CDF: [[[u16; (UV_INTRA_MODES) as usize + 1];
    INTRA_MODES as usize]; CFL_ALLOWED_TYPES as usize] = [
    [
        cdf_pad(&[
            22631, 24152, 25378, 25661, 25986, 26520, 27055, 27923, 28244, 30059, 30941, 31961,
        ]),
        cdf_pad(&[
            9513, 26881, 26973, 27046, 27118, 27664, 27739, 27824, 28359, 29505, 29800, 31796,
        ]),
        cdf_pad(&[
            9845, 9915, 28663, 28704, 28757, 28780, 29198, 29822, 29854, 30764, 31777, 32029,
        ]),
        cdf_pad(&[
            13639, 13897, 14171, 25331, 25606, 25727, 25953, 27148, 28577, 30612, 31355, 32493,
        ]),
        cdf_pad(&[
            9764, 9835, 9930, 9954, 25386, 27053, 27958, 28148, 28243, 31101, 31744, 32363,
        ]),
        cdf_pad(&[
            11825, 13589, 13677, 13720, 15048, 29213, 29301, 29458, 29711, 31161, 31441, 32550,
        ]),
        cdf_pad(&[
            14175, 14399, 16608, 16821, 17718, 17775, 28551, 30200, 30245, 31837, 32342, 32667,
        ]),
        cdf_pad(&[
            12885, 13038, 14978, 15590, 15673, 15748, 16176, 29128, 29267, 30643, 31961, 32461,
        ]),
        cdf_pad(&[
            12026, 13661, 13874, 15305, 15490, 15726, 15995, 16273, 28443, 30388, 30767, 32416,
        ]),
        cdf_pad(&[
            19052, 19840, 20579, 20916, 21150, 21467, 21885, 22719, 23174, 28861, 30379, 32175,
        ]),
        cdf_pad(&[
            18627, 19649, 20974, 21219, 21492, 21816, 22199, 23119, 23527, 27053, 31397, 32148,
        ]),
        cdf_pad(&[
            17026, 19004, 19997, 20339, 20586, 21103, 21349, 21907, 22482, 25896, 26541, 31819,
        ]),
        cdf_pad(&[
            12124, 13759, 14959, 14992, 15007, 15051, 15078, 15166, 15255, 15753, 16039, 16606,
        ]),
    ],
    [
        cdf_pad(&[
            10407, 11208, 12900, 13181, 13823, 14175, 14899, 15656, 15986, 20086, 20995, 22455,
            24212,
        ]),
        cdf_pad(&[
            4532, 19780, 20057, 20215, 20428, 21071, 21199, 21451, 22099, 24228, 24693, 27032,
            29472,
        ]),
        cdf_pad(&[
            5273, 5379, 20177, 20270, 20385, 20439, 20949, 21695, 21774, 23138, 24256, 24703, 26679,
        ]),
        cdf_pad(&[
            6740, 7167, 7662, 14152, 14536, 14785, 15034, 16741, 18371, 21520, 22206, 23389, 24182,
        ]),
        cdf_pad(&[
            4987, 5368, 5928, 6068, 19114, 20315, 21857, 22253, 22411, 24911, 25380, 26027, 26376,
        ]),
        cdf_pad(&[
            5370, 6889, 7247, 7393, 9498, 21114, 21402, 21753, 21981, 24780, 25386, 26517, 27176,
        ]),
        cdf_pad(&[
            4816, 4961, 7204, 7326, 8765, 8930, 20169, 20682, 20803, 23188, 23763, 24455, 24940,
        ]),
        cdf_pad(&[
            6608, 6740, 8529, 9049, 9257, 9356, 9735, 18827, 19059, 22336, 23204, 23964, 24793,
        ]),
        cdf_pad(&[
            5998, 7419, 7781, 8933, 9255, 9549, 9753, 10417, 18898, 22494, 23139, 24764, 25989,
        ]),
        cdf_pad(&[
            10660, 11298, 12550, 12957, 13322, 13624, 14040, 15004, 15534, 20714, 21789, 23443,
            24861,
        ]),
        cdf_pad(&[
            10522, 11530, 12552, 12963, 13378, 13779, 14245, 15235, 15902, 20102, 22696, 23774,
            25838,
        ]),
        cdf_pad(&[
            10099, 10691, 12639, 13049, 13386, 13665, 14125, 15163, 15636, 19676, 20474, 23519,
            25208,
        ]),
        cdf_pad(&[
            3144, 5087, 7382, 7504, 7593, 7690, 7801, 8064, 8232, 9248, 9875, 10521, 29048,
        ]),
    ],
];

pub(crate) static DEFAULT_PARTITION_CDF: [[u16; (EXT_PARTITION_TYPES) as usize + 1];
    PARTITION_CONTEXTS as usize] = [
    cdf_pad(&[19132, 25510, 30392]),
    cdf_pad(&[13928, 19855, 28540]),
    cdf_pad(&[12522, 23679, 28629]),
    cdf_pad(&[9896, 18783, 25853]),
    cdf_pad(&[
        15597, 20929, 24571, 26706, 27664, 28821, 29601, 30571, 31902,
    ]),
    cdf_pad(&[7925, 11043, 16785, 22470, 23971, 25043, 26651, 28701, 29834]),
    cdf_pad(&[5414, 13269, 15111, 20488, 22360, 24500, 25537, 26336, 32117]),
    cdf_pad(&[2662, 6362, 8614, 20860, 23053, 24778, 26436, 27829, 31171]),
    cdf_pad(&[
        18462, 20920, 23124, 27647, 28227, 29049, 29519, 30178, 31544,
    ]),
    cdf_pad(&[7689, 9060, 12056, 24992, 25660, 26182, 26951, 28041, 29052]),
    cdf_pad(&[6015, 9009, 10062, 24544, 25409, 26545, 27071, 27526, 32047]),
    cdf_pad(&[1394, 2208, 2796, 28614, 29061, 29466, 29840, 30185, 31899]),
    cdf_pad(&[
        20137, 21547, 23078, 29566, 29837, 30261, 30524, 30892, 31724,
    ]),
    cdf_pad(&[6732, 7490, 9497, 27944, 28250, 28515, 28969, 29630, 30104]),
    cdf_pad(&[5945, 7663, 8348, 28683, 29117, 29749, 30064, 30298, 32238]),
    cdf_pad(&[870, 1212, 1487, 31198, 31394, 31574, 31743, 31881, 32332]),
    cdf_pad(&[27899, 28219, 28529, 32484, 32539, 32619, 32639]),
    cdf_pad(&[6607, 6990, 8268, 32060, 32219, 32338, 32371]),
    cdf_pad(&[5429, 6676, 7122, 32027, 32227, 32531, 32582]),
    cdf_pad(&[711, 966, 1172, 32448, 32538, 32617, 32664]),
];

pub(crate) static DEFAULT_INTRA_EXT_TX_CDF: [[[[u16; (TX_TYPES) as usize + 1];
    INTRA_MODES as usize]; EXT_TX_SIZES as usize];
    EXT_TX_SETS_INTRA as usize] = [
    [
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
    ],
    [
        [
            cdf_pad(&[1535, 8035, 9461, 12751, 23467, 27825]),
            cdf_pad(&[564, 3335, 9709, 10870, 18143, 28094]),
            cdf_pad(&[672, 3247, 3676, 11982, 19415, 23127]),
            cdf_pad(&[5279, 13885, 15487, 18044, 23527, 30252]),
            cdf_pad(&[4423, 6074, 7985, 10416, 25693, 29298]),
            cdf_pad(&[1486, 4241, 9460, 10662, 16456, 27694]),
            cdf_pad(&[439, 2838, 3522, 6737, 18058, 23754]),
            cdf_pad(&[1190, 4233, 4855, 11670, 20281, 24377]),
            cdf_pad(&[1045, 4312, 8647, 10159, 18644, 29335]),
            cdf_pad(&[202, 3734, 4747, 7298, 17127, 24016]),
            cdf_pad(&[447, 4312, 6819, 8884, 16010, 23858]),
            cdf_pad(&[277, 4369, 5255, 8905, 16465, 22271]),
            cdf_pad(&[3409, 5436, 10599, 15599, 19687, 24040]),
        ],
        [
            cdf_pad(&[1870, 13742, 14530, 16498, 23770, 27698]),
            cdf_pad(&[326, 8796, 14632, 15079, 19272, 27486]),
            cdf_pad(&[484, 7576, 7712, 14443, 19159, 22591]),
            cdf_pad(&[1126, 15340, 15895, 17023, 20896, 30279]),
            cdf_pad(&[655, 4854, 5249, 5913, 22099, 27138]),
            cdf_pad(&[1299, 6458, 8885, 9290, 14851, 25497]),
            cdf_pad(&[311, 5295, 5552, 6885, 16107, 22672]),
            cdf_pad(&[883, 8059, 8270, 11258, 17289, 21549]),
            cdf_pad(&[741, 7580, 9318, 10345, 16688, 29046]),
            cdf_pad(&[110, 7406, 7915, 9195, 16041, 23329]),
            cdf_pad(&[363, 7974, 9357, 10673, 15629, 24474]),
            cdf_pad(&[153, 7647, 8112, 9936, 15307, 19996]),
            cdf_pad(&[3511, 6332, 11165, 15335, 19323, 23594]),
        ],
        [
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
        ],
        [
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
        ],
    ],
    [
        [
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
        ],
        [
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
        ],
        [
            cdf_pad(&[1127, 12814, 22772, 27483]),
            cdf_pad(&[145, 6761, 11980, 26667]),
            cdf_pad(&[362, 5887, 11678, 16725]),
            cdf_pad(&[385, 15213, 18587, 30693]),
            cdf_pad(&[25, 2914, 23134, 27903]),
            cdf_pad(&[60, 4470, 11749, 23991]),
            cdf_pad(&[37, 3332, 14511, 21448]),
            cdf_pad(&[157, 6320, 13036, 17439]),
            cdf_pad(&[119, 6719, 12906, 29396]),
            cdf_pad(&[47, 5537, 12576, 21499]),
            cdf_pad(&[269, 6076, 11258, 23115]),
            cdf_pad(&[83, 5615, 12001, 17228]),
            cdf_pad(&[1968, 5556, 12023, 18547]),
        ],
        [
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
        ],
    ],
];

pub(crate) static DEFAULT_FILTER_INTRA_MODE_CDF: [u16; (FILTER_INTRA_MODES) as usize + 1] =
    cdf_pad(&[8949, 12776, 17211, 29558]);

pub(crate) static DEFAULT_FILTER_INTRA_CDFS: [[u16; (2) as usize + 1]; BLOCK_SIZES_ALL as usize] = [
    cdf_pad(&[4621]),
    cdf_pad(&[6743]),
    cdf_pad(&[5893]),
    cdf_pad(&[7866]),
    cdf_pad(&[12551]),
    cdf_pad(&[9394]),
    cdf_pad(&[12408]),
    cdf_pad(&[14301]),
    cdf_pad(&[12756]),
    cdf_pad(&[22343]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[12770]),
    cdf_pad(&[10368]),
    cdf_pad(&[20229]),
    cdf_pad(&[18101]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
];

pub(crate) static DEFAULT_SKIP_TXFM_CDFS: [[u16; (2) as usize + 1]; SKIP_CONTEXTS as usize] =
    [cdf_pad(&[31671]), cdf_pad(&[16515]), cdf_pad(&[4576])];

pub(crate) static DEFAULT_TX_SIZE_CDF: [[[u16; (MAX_TX_DEPTH + 1) as usize + 1];
    TX_SIZE_CONTEXTS as usize]; MAX_TX_CATS as usize] = [
    [cdf_pad(&[19968]), cdf_pad(&[19968]), cdf_pad(&[24320])],
    [
        cdf_pad(&[12272, 30172]),
        cdf_pad(&[12272, 30172]),
        cdf_pad(&[18677, 30848]),
    ],
    [
        cdf_pad(&[12986, 15180]),
        cdf_pad(&[12986, 15180]),
        cdf_pad(&[24302, 25602]),
    ],
    [
        cdf_pad(&[5782, 11475]),
        cdf_pad(&[5782, 11475]),
        cdf_pad(&[16803, 22759]),
    ],
];

pub(crate) static DEFAULT_DELTA_Q_CDF: [u16; (DELTA_Q_PROBS + 1) as usize + 1] =
    cdf_pad(&[28160, 32120, 32677]);

pub(crate) static DEFAULT_DELTA_LF_MULTI_CDF: [[u16; (DELTA_LF_PROBS + 1) as usize + 1];
    FRAME_LF_COUNT as usize] = [
    cdf_pad(&[28160, 32120, 32677]),
    cdf_pad(&[28160, 32120, 32677]),
    cdf_pad(&[28160, 32120, 32677]),
    cdf_pad(&[28160, 32120, 32677]),
];

pub(crate) static DEFAULT_DELTA_LF_CDF: [u16; (DELTA_LF_PROBS + 1) as usize + 1] =
    cdf_pad(&[28160, 32120, 32677]);

pub(crate) static DEFAULT_CFL_SIGN_CDF: [u16; (CFL_JOINT_SIGNS) as usize + 1] =
    cdf_pad(&[1418, 2123, 13340, 18405, 26972, 28343, 32294]);

pub(crate) static DEFAULT_CFL_ALPHA_CDF: [[u16; (CFL_ALPHABET_SIZE) as usize + 1];
    CFL_ALPHA_CONTEXTS as usize] = [
    cdf_pad(&[
        7637, 20719, 31401, 32481, 32657, 32688, 32692, 32696, 32700, 32704, 32708, 32712, 32716,
        32720, 32724,
    ]),
    cdf_pad(&[
        14365, 23603, 28135, 31168, 32167, 32395, 32487, 32573, 32620, 32647, 32668, 32672, 32676,
        32680, 32684,
    ]),
    cdf_pad(&[
        11532, 22380, 28445, 31360, 32349, 32523, 32584, 32649, 32673, 32677, 32681, 32685, 32689,
        32693, 32697,
    ]),
    cdf_pad(&[
        26990, 31402, 32282, 32571, 32692, 32696, 32700, 32704, 32708, 32712, 32716, 32720, 32724,
        32728, 32732,
    ]),
    cdf_pad(&[
        17248, 26058, 28904, 30608, 31305, 31877, 32126, 32321, 32394, 32464, 32516, 32560, 32576,
        32593, 32622,
    ]),
    cdf_pad(&[
        14738, 21678, 25779, 27901, 29024, 30302, 30980, 31843, 32144, 32413, 32520, 32594, 32622,
        32656, 32660,
    ]),
];

pub(crate) static DEFAULT_INTRABC_CDF: [u16; (2) as usize + 1] = cdf_pad(&[30531]);

pub(crate) static DEFAULT_PALETTE_Y_SIZE_CDF: [[u16; (PALETTE_SIZES) as usize + 1];
    PALATTE_BSIZE_CTXS as usize] = [
    cdf_pad(&[7952, 13000, 18149, 21478, 25527, 29241]),
    cdf_pad(&[7139, 11421, 16195, 19544, 23666, 28073]),
    cdf_pad(&[7788, 12741, 17325, 20500, 24315, 28530]),
    cdf_pad(&[8271, 14064, 18246, 21564, 25071, 28533]),
    cdf_pad(&[12725, 19180, 21863, 24839, 27535, 30120]),
    cdf_pad(&[9711, 14888, 16923, 21052, 25661, 27875]),
    cdf_pad(&[14940, 20797, 21678, 24186, 27033, 28999]),
];

pub(crate) static DEFAULT_PALETTE_UV_SIZE_CDF: [[u16; (PALETTE_SIZES) as usize + 1];
    PALATTE_BSIZE_CTXS as usize] = [
    cdf_pad(&[8713, 19979, 27128, 29609, 31331, 32272]),
    cdf_pad(&[5839, 15573, 23581, 26947, 29848, 31700]),
    cdf_pad(&[4426, 11260, 17999, 21483, 25863, 29430]),
    cdf_pad(&[3228, 9464, 14993, 18089, 22523, 27420]),
    cdf_pad(&[3768, 8886, 13091, 17852, 22495, 27207]),
    cdf_pad(&[2464, 8451, 12861, 21632, 25525, 28555]),
    cdf_pad(&[1269, 5435, 10433, 18963, 21700, 25865]),
];

pub(crate) static DEFAULT_PALETTE_Y_COLOR_INDEX_CDF: [[[u16; (PALETTE_COLORS) as usize + 1];
    PALETTE_COLOR_INDEX_CONTEXTS as usize];
    PALETTE_SIZES as usize] = [
    [
        cdf_pad(&[28710]),
        cdf_pad(&[16384]),
        cdf_pad(&[10553]),
        cdf_pad(&[27036]),
        cdf_pad(&[31603]),
    ],
    [
        cdf_pad(&[27877, 30490]),
        cdf_pad(&[11532, 25697]),
        cdf_pad(&[6544, 30234]),
        cdf_pad(&[23018, 28072]),
        cdf_pad(&[31915, 32385]),
    ],
    [
        cdf_pad(&[25572, 28046, 30045]),
        cdf_pad(&[9478, 21590, 27256]),
        cdf_pad(&[7248, 26837, 29824]),
        cdf_pad(&[19167, 24486, 28349]),
        cdf_pad(&[31400, 31825, 32250]),
    ],
    [
        cdf_pad(&[24779, 26955, 28576, 30282]),
        cdf_pad(&[8669, 20364, 24073, 28093]),
        cdf_pad(&[4255, 27565, 29377, 31067]),
        cdf_pad(&[19864, 23674, 26716, 29530]),
        cdf_pad(&[31646, 31893, 32147, 32426]),
    ],
    [
        cdf_pad(&[23132, 25407, 26970, 28435, 30073]),
        cdf_pad(&[7443, 17242, 20717, 24762, 27982]),
        cdf_pad(&[6300, 24862, 26944, 28784, 30671]),
        cdf_pad(&[18916, 22895, 25267, 27435, 29652]),
        cdf_pad(&[31270, 31550, 31808, 32059, 32353]),
    ],
    [
        cdf_pad(&[23105, 25199, 26464, 27684, 28931, 30318]),
        cdf_pad(&[6950, 15447, 18952, 22681, 25567, 28563]),
        cdf_pad(&[7560, 23474, 25490, 27203, 28921, 30708]),
        cdf_pad(&[18544, 22373, 24457, 26195, 28119, 30045]),
        cdf_pad(&[31198, 31451, 31670, 31882, 32123, 32391]),
    ],
    [
        cdf_pad(&[21689, 23883, 25163, 26352, 27506, 28827, 30195]),
        cdf_pad(&[6892, 15385, 17840, 21606, 24287, 26753, 29204]),
        cdf_pad(&[5651, 23182, 25042, 26518, 27982, 29392, 30900]),
        cdf_pad(&[19349, 22578, 24418, 25994, 27524, 29031, 30448]),
        cdf_pad(&[31028, 31270, 31504, 31705, 31927, 32153, 32392]),
    ],
];

pub(crate) static DEFAULT_PALETTE_UV_COLOR_INDEX_CDF: [[[u16; (PALETTE_COLORS) as usize + 1];
    PALETTE_COLOR_INDEX_CONTEXTS as usize];
    PALETTE_SIZES as usize] = [
    [
        cdf_pad(&[29089]),
        cdf_pad(&[16384]),
        cdf_pad(&[8713]),
        cdf_pad(&[29257]),
        cdf_pad(&[31610]),
    ],
    [
        cdf_pad(&[25257, 29145]),
        cdf_pad(&[12287, 27293]),
        cdf_pad(&[7033, 27960]),
        cdf_pad(&[20145, 25405]),
        cdf_pad(&[30608, 31639]),
    ],
    [
        cdf_pad(&[24210, 27175, 29903]),
        cdf_pad(&[9888, 22386, 27214]),
        cdf_pad(&[5901, 26053, 29293]),
        cdf_pad(&[18318, 22152, 28333]),
        cdf_pad(&[30459, 31136, 31926]),
    ],
    [
        cdf_pad(&[22980, 25479, 27781, 29986]),
        cdf_pad(&[8413, 21408, 24859, 28874]),
        cdf_pad(&[2257, 29449, 30594, 31598]),
        cdf_pad(&[19189, 21202, 25915, 28620]),
        cdf_pad(&[31844, 32044, 32281, 32518]),
    ],
    [
        cdf_pad(&[22217, 24567, 26637, 28683, 30548]),
        cdf_pad(&[7307, 16406, 19636, 24632, 28424]),
        cdf_pad(&[4441, 25064, 26879, 28942, 30919]),
        cdf_pad(&[17210, 20528, 23319, 26750, 29582]),
        cdf_pad(&[30674, 30953, 31396, 31735, 32207]),
    ],
    [
        cdf_pad(&[21239, 23168, 25044, 26962, 28705, 30506]),
        cdf_pad(&[6545, 15012, 18004, 21817, 25503, 28701]),
        cdf_pad(&[3448, 26295, 27437, 28704, 30126, 31442]),
        cdf_pad(&[15889, 18323, 21704, 24698, 26976, 29690]),
        cdf_pad(&[30988, 31204, 31479, 31734, 31983, 32325]),
    ],
    [
        cdf_pad(&[21442, 23288, 24758, 26246, 27649, 28980, 30563]),
        cdf_pad(&[5863, 14933, 17552, 20668, 23683, 26411, 29273]),
        cdf_pad(&[3415, 25810, 26877, 27990, 29223, 30394, 31618]),
        cdf_pad(&[17965, 20084, 22232, 23974, 26274, 28402, 30390]),
        cdf_pad(&[31190, 31329, 31516, 31679, 31825, 32026, 32322]),
    ],
];

pub(crate) static DEFAULT_COMP_INTER_CDF: [[u16; (2) as usize + 1]; COMP_INTER_CONTEXTS as usize] = [
    cdf_pad(&[26828]),
    cdf_pad(&[24035]),
    cdf_pad(&[12031]),
    cdf_pad(&[10640]),
    cdf_pad(&[2901]),
];

pub(crate) static DEFAULT_COMP_REF_TYPE_CDF: [[u16; (2) as usize + 1];
    COMP_REF_TYPE_CONTEXTS as usize] = [
    cdf_pad(&[1198]),
    cdf_pad(&[2070]),
    cdf_pad(&[9166]),
    cdf_pad(&[7499]),
    cdf_pad(&[22475]),
];

pub(crate) static DEFAULT_UNI_COMP_REF_CDF: [[[u16; (2) as usize + 1];
    (UNIDIR_COMP_REFS - 1) as usize];
    UNI_COMP_REF_CONTEXTS as usize] = [
    [cdf_pad(&[5284]), cdf_pad(&[3865]), cdf_pad(&[3128])],
    [cdf_pad(&[23152]), cdf_pad(&[14173]), cdf_pad(&[15270])],
    [cdf_pad(&[31774]), cdf_pad(&[25120]), cdf_pad(&[26710])],
];

pub(crate) static DEFAULT_PALETTE_Y_MODE_CDF: [[[u16; (2) as usize + 1];
    PALETTE_Y_MODE_CONTEXTS as usize];
    PALATTE_BSIZE_CTXS as usize] = [
    [cdf_pad(&[31676]), cdf_pad(&[3419]), cdf_pad(&[1261])],
    [cdf_pad(&[31912]), cdf_pad(&[2859]), cdf_pad(&[980])],
    [cdf_pad(&[31823]), cdf_pad(&[3400]), cdf_pad(&[781])],
    [cdf_pad(&[32030]), cdf_pad(&[3561]), cdf_pad(&[904])],
    [cdf_pad(&[32309]), cdf_pad(&[7337]), cdf_pad(&[1462])],
    [cdf_pad(&[32265]), cdf_pad(&[4015]), cdf_pad(&[1521])],
    [cdf_pad(&[32450]), cdf_pad(&[7946]), cdf_pad(&[129])],
];

pub(crate) static DEFAULT_PALETTE_UV_MODE_CDF: [[u16; (2) as usize + 1];
    PALETTE_UV_MODE_CONTEXTS as usize] = [cdf_pad(&[32461]), cdf_pad(&[21488])];

pub(crate) static DEFAULT_COMP_REF_CDF: [[[u16; (2) as usize + 1]; (FWD_REFS - 1) as usize];
    REF_CONTEXTS as usize] = [
    [cdf_pad(&[4946]), cdf_pad(&[9468]), cdf_pad(&[1503])],
    [cdf_pad(&[19891]), cdf_pad(&[22441]), cdf_pad(&[15160])],
    [cdf_pad(&[30731]), cdf_pad(&[31059]), cdf_pad(&[27544])],
];

pub(crate) static DEFAULT_COMP_BWDREF_CDF: [[[u16; (2) as usize + 1]; (BWD_REFS - 1) as usize];
    REF_CONTEXTS as usize] = [
    [cdf_pad(&[2235]), cdf_pad(&[1423])],
    [cdf_pad(&[17182]), cdf_pad(&[15175])],
    [cdf_pad(&[30606]), cdf_pad(&[30489])],
];

pub(crate) static DEFAULT_SINGLE_REF_CDF: [[[u16; (2) as usize + 1]; (SINGLE_REFS - 1) as usize];
    REF_CONTEXTS as usize] = [
    [
        cdf_pad(&[4897]),
        cdf_pad(&[1555]),
        cdf_pad(&[4236]),
        cdf_pad(&[8650]),
        cdf_pad(&[904]),
        cdf_pad(&[1444]),
    ],
    [
        cdf_pad(&[16973]),
        cdf_pad(&[16751]),
        cdf_pad(&[19647]),
        cdf_pad(&[24773]),
        cdf_pad(&[11014]),
        cdf_pad(&[15087]),
    ],
    [
        cdf_pad(&[29744]),
        cdf_pad(&[30279]),
        cdf_pad(&[31194]),
        cdf_pad(&[31895]),
        cdf_pad(&[26875]),
        cdf_pad(&[30304]),
    ],
];

pub(crate) static DEFAULT_TXFM_PARTITION_CDF: [[u16; (2) as usize + 1];
    TXFM_PARTITION_CONTEXTS as usize] = [
    cdf_pad(&[28581]),
    cdf_pad(&[23846]),
    cdf_pad(&[20847]),
    cdf_pad(&[24315]),
    cdf_pad(&[18196]),
    cdf_pad(&[12133]),
    cdf_pad(&[18791]),
    cdf_pad(&[10887]),
    cdf_pad(&[11005]),
    cdf_pad(&[27179]),
    cdf_pad(&[20004]),
    cdf_pad(&[11281]),
    cdf_pad(&[26549]),
    cdf_pad(&[19308]),
    cdf_pad(&[14224]),
    cdf_pad(&[28015]),
    cdf_pad(&[21546]),
    cdf_pad(&[14400]),
    cdf_pad(&[28165]),
    cdf_pad(&[22401]),
    cdf_pad(&[16088]),
];

pub(crate) static DEFAULT_COMPOUND_IDX_CDFS: [[u16; (2) as usize + 1];
    COMP_INDEX_CONTEXTS as usize] = [
    cdf_pad(&[18244]),
    cdf_pad(&[12865]),
    cdf_pad(&[7053]),
    cdf_pad(&[13259]),
    cdf_pad(&[9334]),
    cdf_pad(&[4644]),
];

pub(crate) static DEFAULT_COMP_GROUP_IDX_CDFS: [[u16; (2) as usize + 1];
    COMP_GROUP_IDX_CONTEXTS as usize] = [
    cdf_pad(&[26607]),
    cdf_pad(&[22891]),
    cdf_pad(&[18840]),
    cdf_pad(&[24594]),
    cdf_pad(&[19934]),
    cdf_pad(&[22674]),
];

pub(crate) static DEFAULT_NEWMV_CDF: [[u16; (2) as usize + 1]; NEWMV_MODE_CONTEXTS as usize] = [
    cdf_pad(&[24035]),
    cdf_pad(&[16630]),
    cdf_pad(&[15339]),
    cdf_pad(&[8386]),
    cdf_pad(&[12222]),
    cdf_pad(&[4676]),
];

pub(crate) static DEFAULT_ZEROMV_CDF: [[u16; (2) as usize + 1]; GLOBALMV_MODE_CONTEXTS as usize] =
    [cdf_pad(&[2175]), cdf_pad(&[1054])];

pub(crate) static DEFAULT_REFMV_CDF: [[u16; (2) as usize + 1]; REFMV_MODE_CONTEXTS as usize] = [
    cdf_pad(&[23974]),
    cdf_pad(&[24188]),
    cdf_pad(&[17848]),
    cdf_pad(&[28622]),
    cdf_pad(&[24312]),
    cdf_pad(&[19923]),
];

pub(crate) static DEFAULT_DRL_CDF: [[u16; (2) as usize + 1]; DRL_MODE_CONTEXTS as usize] =
    [cdf_pad(&[13104]), cdf_pad(&[24560]), cdf_pad(&[18945])];

pub(crate) static DEFAULT_MOTION_MODE_CDF: [[u16; (MOTION_MODES) as usize + 1];
    BLOCK_SIZES_ALL as usize] = [
    cdf_pad(&[10923, 21845]),
    cdf_pad(&[10923, 21845]),
    cdf_pad(&[10923, 21845]),
    cdf_pad(&[7651, 24760]),
    cdf_pad(&[4738, 24765]),
    cdf_pad(&[5391, 25528]),
    cdf_pad(&[19419, 26810]),
    cdf_pad(&[5123, 23606]),
    cdf_pad(&[11606, 24308]),
    cdf_pad(&[26260, 29116]),
    cdf_pad(&[20360, 28062]),
    cdf_pad(&[21679, 26830]),
    cdf_pad(&[29516, 30701]),
    cdf_pad(&[28898, 30397]),
    cdf_pad(&[30878, 31335]),
    cdf_pad(&[32507, 32558]),
    cdf_pad(&[10923, 21845]),
    cdf_pad(&[10923, 21845]),
    cdf_pad(&[28799, 31390]),
    cdf_pad(&[26431, 30774]),
    cdf_pad(&[28973, 31594]),
    cdf_pad(&[29742, 31203]),
];

pub(crate) static DEFAULT_OBMC_CDF: [[u16; (2) as usize + 1]; BLOCK_SIZES_ALL as usize] = [
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[10437]),
    cdf_pad(&[9371]),
    cdf_pad(&[9301]),
    cdf_pad(&[17432]),
    cdf_pad(&[14423]),
    cdf_pad(&[15142]),
    cdf_pad(&[25817]),
    cdf_pad(&[22823]),
    cdf_pad(&[22083]),
    cdf_pad(&[30128]),
    cdf_pad(&[31014]),
    cdf_pad(&[31560]),
    cdf_pad(&[32638]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[23664]),
    cdf_pad(&[20901]),
    cdf_pad(&[24008]),
    cdf_pad(&[26879]),
];

pub(crate) static DEFAULT_INTER_COMPOUND_MODE_CDF: [[u16; (INTER_COMPOUND_MODES) as usize + 1];
    INTER_MODE_CONTEXTS as usize] = [
    cdf_pad(&[7760, 13823, 15808, 17641, 19156, 20666, 26891]),
    cdf_pad(&[10730, 19452, 21145, 22749, 24039, 25131, 28724]),
    cdf_pad(&[10664, 20221, 21588, 22906, 24295, 25387, 28436]),
    cdf_pad(&[13298, 16984, 20471, 24182, 25067, 25736, 26422]),
    cdf_pad(&[18904, 23325, 25242, 27432, 27898, 28258, 30758]),
    cdf_pad(&[10725, 17454, 20124, 22820, 24195, 25168, 26046]),
    cdf_pad(&[17125, 24273, 25814, 27492, 28214, 28704, 30592]),
    cdf_pad(&[13046, 23214, 24505, 25942, 27435, 28442, 29330]),
];

pub(crate) static DEFAULT_COMPOUND_TYPE_CDF: [[u16; (MASKED_COMPOUND_TYPES) as usize + 1];
    BLOCK_SIZES_ALL as usize] = [
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[23431]),
    cdf_pad(&[13171]),
    cdf_pad(&[11470]),
    cdf_pad(&[9770]),
    cdf_pad(&[9100]),
    cdf_pad(&[8233]),
    cdf_pad(&[6172]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[11820]),
    cdf_pad(&[7701]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
];

pub(crate) static DEFAULT_WEDGE_IDX_CDF: [[u16; (16) as usize + 1]; BLOCK_SIZES_ALL as usize] = [
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2438, 4440, 6599, 8663, 11005, 12874, 15751, 18094, 20359, 22362, 24127, 25702, 27752,
        29450, 31171,
    ]),
    cdf_pad(&[
        806, 3266, 6005, 6738, 7218, 7367, 7771, 14588, 16323, 17367, 18452, 19422, 22839, 26127,
        29629,
    ]),
    cdf_pad(&[
        2779, 3738, 4683, 7213, 7775, 8017, 8655, 14357, 17939, 21332, 24520, 27470, 29456, 30529,
        31656,
    ]),
    cdf_pad(&[
        1684, 3625, 5675, 7108, 9302, 11274, 14429, 17144, 19163, 20961, 22884, 24471, 26719,
        28714, 30877,
    ]),
    cdf_pad(&[
        1142, 3491, 6277, 7314, 8089, 8355, 9023, 13624, 15369, 16730, 18114, 19313, 22521, 26012,
        29550,
    ]),
    cdf_pad(&[
        2742, 4195, 5727, 8035, 8980, 9336, 10146, 14124, 17270, 20533, 23434, 25972, 27944, 29570,
        31416,
    ]),
    cdf_pad(&[
        1727, 3948, 6101, 7796, 9841, 12344, 15766, 18944, 20638, 22038, 23963, 25311, 26988,
        28766, 31012,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        154, 987, 1925, 2051, 2088, 2111, 2151, 23033, 23703, 24284, 24985, 25684, 27259, 28883,
        30911,
    ]),
    cdf_pad(&[
        1135, 1322, 1493, 2635, 2696, 2737, 2770, 21016, 22935, 25057, 27251, 29173, 30089, 30960,
        31933,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
    cdf_pad(&[
        2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
        28672, 30720,
    ]),
];

pub(crate) static DEFAULT_INTERINTRA_CDF: [[u16; (2) as usize + 1]; BLOCK_SIZE_GROUPS as usize] = [
    cdf_pad(&[16384]),
    cdf_pad(&[26887]),
    cdf_pad(&[27597]),
    cdf_pad(&[30237]),
];

pub(crate) static DEFAULT_WEDGE_INTERINTRA_CDF: [[u16; (2) as usize + 1];
    BLOCK_SIZES_ALL as usize] = [
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[20036]),
    cdf_pad(&[24957]),
    cdf_pad(&[26704]),
    cdf_pad(&[27530]),
    cdf_pad(&[29564]),
    cdf_pad(&[29444]),
    cdf_pad(&[26872]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
];

pub(crate) static DEFAULT_INTERINTRA_MODE_CDF: [[u16; (INTERINTRA_MODES) as usize + 1];
    BLOCK_SIZE_GROUPS as usize] = [
    cdf_pad(&[8192, 16384, 24576]),
    cdf_pad(&[1875, 11082, 27332]),
    cdf_pad(&[2473, 9996, 26388]),
    cdf_pad(&[4238, 11537, 25926]),
];

pub(crate) static DEFAULT_SEGMENT_PRED_CDF: [[u16; (2) as usize + 1];
    SEG_TEMPORAL_PRED_CTXS as usize] = [
    cdf_pad(&[128 * 128]),
    cdf_pad(&[128 * 128]),
    cdf_pad(&[128 * 128]),
];

pub(crate) static DEFAULT_SWITCHABLE_RESTORE_CDF: [u16; (RESTORE_SWITCHABLE_TYPES) as usize + 1] =
    cdf_pad(&[9413, 22581]);

pub(crate) static DEFAULT_WIENER_RESTORE_CDF: [u16; (2) as usize + 1] = cdf_pad(&[11570]);

pub(crate) static DEFAULT_SGRPROJ_RESTORE_CDF: [u16; (2) as usize + 1] = cdf_pad(&[16855]);

pub(crate) static DEFAULT_SWITCHABLE_INTERP_CDF: [[u16; (SWITCHABLE_FILTERS) as usize + 1];
    SWITCHABLE_FILTER_CONTEXTS as usize] = [
    cdf_pad(&[31935, 32720]),
    cdf_pad(&[5568, 32719]),
    cdf_pad(&[422, 2938]),
    cdf_pad(&[28244, 32608]),
    cdf_pad(&[31206, 31953]),
    cdf_pad(&[4862, 32121]),
    cdf_pad(&[770, 1152]),
    cdf_pad(&[20889, 25637]),
    cdf_pad(&[31910, 32724]),
    cdf_pad(&[4120, 32712]),
    cdf_pad(&[305, 2247]),
    cdf_pad(&[27403, 32636]),
    cdf_pad(&[31022, 32009]),
    cdf_pad(&[2963, 32093]),
    cdf_pad(&[601, 943]),
    cdf_pad(&[14969, 21398]),
];

pub(crate) static DEFAULT_INTER_EXT_TX_CDF: [[[u16; (TX_TYPES) as usize + 1];
    EXT_TX_SIZES as usize];
    EXT_TX_SETS_INTER as usize] = [
    [cdf_pad(&[]), cdf_pad(&[]), cdf_pad(&[]), cdf_pad(&[])],
    [
        cdf_pad(&[
            4458, 5560, 7695, 9709, 13330, 14789, 17537, 20266, 21504, 22848, 23934, 25474, 27727,
            28915, 30631,
        ]),
        cdf_pad(&[
            1645, 2573, 4778, 5711, 7807, 8622, 10522, 15357, 17674, 20408, 22517, 25010, 27116,
            28856, 30749,
        ]),
        cdf_pad(&[
            2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
            28672, 30720,
        ]),
        cdf_pad(&[
            2048, 4096, 6144, 8192, 10240, 12288, 14336, 16384, 18432, 20480, 22528, 24576, 26624,
            28672, 30720,
        ]),
    ],
    [
        cdf_pad(&[
            2731, 5461, 8192, 10923, 13653, 16384, 19115, 21845, 24576, 27307, 30037,
        ]),
        cdf_pad(&[
            2731, 5461, 8192, 10923, 13653, 16384, 19115, 21845, 24576, 27307, 30037,
        ]),
        cdf_pad(&[
            770, 2421, 5225, 12907, 15819, 18927, 21561, 24089, 26595, 28526, 30529,
        ]),
        cdf_pad(&[
            2731, 5461, 8192, 10923, 13653, 16384, 19115, 21845, 24576, 27307, 30037,
        ]),
    ],
    [
        cdf_pad(&[16384]),
        cdf_pad(&[4167]),
        cdf_pad(&[1998]),
        cdf_pad(&[748]),
    ],
];

pub(crate) static DEFAULT_SKIP_MODE_CDFS: [[u16; (2) as usize + 1]; SKIP_MODE_CONTEXTS as usize] =
    [cdf_pad(&[32621]), cdf_pad(&[20708]), cdf_pad(&[8127])];

pub(crate) static DEFAULT_INTRA_INTER_CDF: [[u16; (2) as usize + 1];
    INTRA_INTER_CONTEXTS as usize] = [
    cdf_pad(&[806]),
    cdf_pad(&[16662]),
    cdf_pad(&[20186]),
    cdf_pad(&[26538]),
];

pub(crate) static DEFAULT_SPATIAL_PRED_SEG_TREE_CDF: [[u16; (MAX_SEGMENTS) as usize + 1];
    SPATIAL_PREDICTION_PROBS as usize] = [
    cdf_pad(&[5622, 7893, 16093, 18233, 27809, 28373, 32533]),
    cdf_pad(&[14274, 18230, 22557, 24935, 29980, 30851, 32344]),
    cdf_pad(&[27527, 28487, 28723, 28890, 32397, 32647, 32679]),
];
