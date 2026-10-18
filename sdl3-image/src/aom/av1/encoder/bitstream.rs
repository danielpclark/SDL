// Rust translation of the parts of av1/encoder/bitstream.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the packing of a still picture's key frame (reduced still picture
// header) or of an image sequence's frames (real-time coding), with one
// thread and one tile group.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The bitstream writer: the sequence header OBU, then the frame OBU (the
//! uncompressed frame header and the tiles' entropy-coded modes and
//! coefficients). The paths of segmentation, palettes, intra block copy,
//! masked compound predictions, delta q and loop restoration are left out
//! (the saver's and the animation encoder's frames use none of them).

use super::encoder::*;
use super::encodetxb::{av1_get_cb_coeff_buffer, av1_write_coeffs_txb, av1_write_intra_coeffs_mb};
use crate::aom::aom_dsp::aom_dsp_common::ceil_power_of_two;
use crate::aom::aom_dsp::bitwriter::*;
use crate::aom::aom_dsp::bitwriter_buffer::*;
use crate::aom::aom_integer::{aom_uleb_decode, aom_uleb_encode, aom_uleb_size_in_bytes};
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::av1_loopfilter::av1_get_txb_size_index;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cdef::CDEF_STRENGTH_BITS;
use crate::aom::av1::common::cfl::is_cfl_allowed;
use crate::aom::av1::common::entropymode::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::LOG_SWITCHABLE_FILTERS;
use crate::aom::av1::common::filter::{
    av1_broadcast_interp_filter, av1_extract_interp_filter, av1_unswitchable_filter, SWITCHABLE,
    SWITCHABLE_FILTERS,
};
use crate::aom::av1::common::mv::IntMv;
use crate::aom::av1::common::mv::{WarpedMotionParams, DEFAULT_WARP_PARAMS, IDENTITY, TRANSLATION};
use crate::aom::av1::common::mvref_common::{
    av1_collect_neighbors_ref_counts, av1_drl_ctx, av1_ref_frame_type, COMPOUND_MODE_CTX_MAP,
};
use crate::aom::av1::common::pred_common::*;
use crate::aom::av1::common::quant_common::QINDEX_BITS;
use crate::aom::av1::common::reconinter::{
    av1_is_interp_needed, av1_is_wedge_used, is_any_masked_compound_used,
};
use crate::aom::av1::common::reconintra::{
    av1_allow_intrabc, av1_filter_intra_allowed, av1_is_directional_mode,
};
use crate::aom::av1::common::restoration::av1_loop_restoration_corners_in_sb;
use crate::aom::av1::common::seg_common::*;
use crate::aom::av1::common::tile_common::*;
use crate::aom::av1::encoder::block::MbModeInfoExtFrame;
use crate::aom::av1::encoder::block::{tx_size_to_depth, Macroblock};
use crate::aom::av1::encoder::encodemv::av1_encode_mv;

/// `AV1_MIN_TILE_SIZE_BYTES` (bitstream.h)
pub(crate) const AV1_MIN_TILE_SIZE_BYTES: u32 = 1;

// (OBU_TYPE, aom/aom_codec.h... obu_util.h)
pub(crate) const OBU_SEQUENCE_HEADER: i32 = 1;
pub(crate) const OBU_TEMPORAL_DELIMITER: i32 = 2;
pub(crate) const OBU_FRAME_HEADER: i32 = 3;
pub(crate) const OBU_TILE_GROUP: i32 = 4;
pub(crate) const OBU_METADATA: i32 = 5;
pub(crate) const OBU_FRAME: i32 = 6;

#[inline]
fn write_intra_y_mode_kf(
    frame_ctx: &mut FrameContext,
    mi: &MbModeInfo,
    above_mi: Option<&MbModeInfo>,
    left_mi: Option<&MbModeInfo>,
    mode: PredictionMode,
    w: &mut AomWriter,
) {
    debug_assert!(!is_intrabc_block(mi));
    aom_write_symbol(
        w,
        mode,
        get_y_mode_cdf(frame_ctx, above_mi, left_mi),
        INTRA_MODES,
    );
}

#[inline]
fn write_selected_tx_size(xd: &mut MacroBlockD, w: &mut AomWriter) {
    let mbmi = *xd.mbmi();
    let bsize = mbmi.bsize;
    if block_signals_txsize(bsize) {
        let tx_size = mbmi.tx_size;
        let tx_size_ctx = get_tx_size_context(xd) as usize;
        let depth = tx_size_to_depth(tx_size, bsize);
        let max_depths = bsize_to_max_depth(bsize);
        let tx_size_cat = bsize_to_tx_size_cat(bsize) as usize;

        debug_assert!(depth >= 0 && depth <= max_depths);
        debug_assert!(!is_inter_block(&mbmi));

        aom_write_symbol(
            w,
            depth,
            &mut xd.tile_ctx.tx_size_cdf[tx_size_cat][tx_size_ctx],
            max_depths + 1,
        );
    }
}

fn write_skip(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    segment_id: u8,
    mi: &MbModeInfo,
    w: &mut AomWriter,
) -> i32 {
    if segfeature_active(&cm.seg, segment_id as u8, SEG_LVL_SKIP) {
        1
    } else {
        let skip_txfm = mi.skip_txfm as i32;
        let ctx = av1_get_skip_txfm_context(xd) as usize;
        aom_write_symbol(w, skip_txfm, &mut xd.tile_ctx.skip_txfm_cdfs[ctx], 2);
        skip_txfm
    }
}

fn write_filter_intra_mode_info(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    mbmi: &MbModeInfo,
    w: &mut AomWriter,
) {
    if av1_filter_intra_allowed(cm, mbmi) {
        aom_write_symbol(
            w,
            mbmi.filter_intra_mode_info.use_filter_intra as i32,
            &mut xd.tile_ctx.filter_intra_cdfs[mbmi.bsize as usize],
            2,
        );
        if mbmi.filter_intra_mode_info.use_filter_intra != 0 {
            let mode = mbmi.filter_intra_mode_info.filter_intra_mode as i32;
            aom_write_symbol(
                w,
                mode,
                &mut xd.tile_ctx.filter_intra_mode_cdf,
                FILTER_INTRA_MODES,
            );
        }
    }
}

#[inline]
fn write_angle_delta(w: &mut AomWriter, angle_delta: i32, cdf: &mut [u16]) {
    aom_write_symbol(
        w,
        angle_delta + MAX_ANGLE_DELTA,
        cdf,
        2 * MAX_ANGLE_DELTA + 1,
    );
}

pub(crate) fn av1_write_tx_type(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    tx_type: TxType,
    tx_size: TxSize,
    w: &mut AomWriter,
) {
    let mbmi = *xd.mbmi();
    let features = &cm.features;
    let is_inter = is_inter_block(&mbmi) as i32;
    let reduced = features.reduced_tx_set_used as i32;
    if get_ext_tx_types(tx_size, is_inter, reduced) > 1
        && ((cm.seg.enabled == 0 && cm.quant_params.base_qindex > 0)
            || (cm.seg.enabled != 0 && xd.qindex[mbmi.segment_id as usize] > 0))
        && mbmi.skip_txfm == 0
        && !segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_SKIP)
    {
        let square_tx_size = TXSIZE_SQR_MAP[tx_size as usize] as usize;
        let tx_set_type = av1_get_ext_tx_set_type(tx_size, is_inter, reduced);
        let eset = get_ext_tx_set(tx_size, is_inter, reduced);
        // eset == 0 should correspond to a set with only DCT_DCT and there
        // is no need to send the tx_type
        debug_assert!(eset > 0);
        debug_assert!(AV1_EXT_TX_USED[tx_set_type as usize][tx_type as usize] != 0);
        if is_inter != 0 {
            aom_write_symbol(
                w,
                AV1_EXT_TX_IND[tx_set_type as usize][tx_type as usize],
                &mut xd.tile_ctx.inter_ext_tx_cdf[eset as usize][square_tx_size],
                AV1_NUM_EXT_TX_SET[tx_set_type as usize],
            );
        } else {
            let intra_dir = if mbmi.filter_intra_mode_info.use_filter_intra != 0 {
                FIMODE_TO_INTRADIR[mbmi.filter_intra_mode_info.filter_intra_mode as usize]
            } else {
                mbmi.mode
            };
            aom_write_symbol(
                w,
                AV1_EXT_TX_IND[tx_set_type as usize][tx_type as usize],
                &mut xd.tile_ctx.intra_ext_tx_cdf[eset as usize][square_tx_size]
                    [intra_dir as usize],
                AV1_NUM_EXT_TX_SET[tx_set_type as usize],
            );
        }
    }
}

#[inline]
fn write_intra_uv_mode(
    frame_ctx: &mut FrameContext,
    uv_mode: UvPredictionMode,
    y_mode: PredictionMode,
    cfl_allowed: i32,
    w: &mut AomWriter,
) {
    aom_write_symbol(
        w,
        uv_mode,
        &mut frame_ctx.uv_mode_cdf[cfl_allowed as usize][y_mode as usize],
        UV_INTRA_MODES - (cfl_allowed == 0) as i32,
    );
}

fn write_cfl_alphas(ec_ctx: &mut FrameContext, idx: u8, joint_sign: i8, w: &mut AomWriter) {
    let joint_sign = joint_sign as i32;
    let idx = idx as i32;
    aom_write_symbol(w, joint_sign, &mut ec_ctx.cfl_sign_cdf, CFL_JOINT_SIGNS);
    // Magnitudes are only signaled for nonzero codes.
    if cfl_sign_u(joint_sign) != CFL_SIGN_ZERO {
        let cdf_u = &mut ec_ctx.cfl_alpha_cdf[cfl_context_u(joint_sign) as usize];
        aom_write_symbol(w, cfl_idx_u(idx), cdf_u, CFL_ALPHABET_SIZE);
    }
    if cfl_sign_v(joint_sign) != CFL_SIGN_ZERO {
        let cdf_v = &mut ec_ctx.cfl_alpha_cdf[cfl_context_v(joint_sign) as usize];
        aom_write_symbol(w, cfl_idx_v(idx), cdf_v, CFL_ALPHABET_SIZE);
    }
}

fn write_cdef(cm: &Av1Common, xd: &mut MacroBlockD, w: &mut AomWriter, skip: i32) {
    if cm.features.coded_lossless || cm.features.allow_intrabc {
        return;
    }

    // At the start of a superblock, mark that we haven't yet written CDEF
    // strengths for any of the CDEF units contained in this superblock.
    let sb_mask = cm.seq_params.mib_size - 1;
    let mi_row_in_sb = xd.mi_row & sb_mask;
    let mi_col_in_sb = xd.mi_col & sb_mask;
    if mi_row_in_sb == 0 && mi_col_in_sb == 0 {
        xd.cdef_transmitted = [false; 4];
    }

    // CDEF unit size is 64x64 irrespective of the superblock size.
    let cdef_size = 1 << (6 - MI_SIZE_LOG2);

    // Find index of this CDEF unit in this superblock.
    let index_mask = cdef_size;
    let cdef_unit_row_in_sb = ((xd.mi_row & index_mask) != 0) as usize;
    let cdef_unit_col_in_sb = ((xd.mi_col & index_mask) != 0) as usize;
    let index = if cm.seq_params.sb_size == BLOCK_128X128 {
        cdef_unit_col_in_sb + 2 * cdef_unit_row_in_sb
    } else {
        0
    };

    // Write CDEF strength to the first non-skip coding block in this CDEF unit.
    if !xd.cdef_transmitted[index] && skip == 0 {
        // CDEF strength for this CDEF unit needs to be stored in the MB_MODE_INFO
        // of the 1st block in this CDEF unit.
        let first_block_mask = !(cdef_size - 1);
        let mi_params = &cm.mi_params;
        let grid_idx = get_mi_grid_idx(
            mi_params,
            xd.mi_row & first_block_mask,
            xd.mi_col & first_block_mask,
        );
        let mbmi = &xd.mi_alloc[xd.mi_grid_base[grid_idx as usize] as usize];
        aom_write_literal(w, mbmi.cdef_strength as i32, cm.cdef_info.cdef_bits);
        xd.cdef_transmitted[index] = true;
    }
}

// If delta q is present, writes delta_q index.
// Also writes delta_q loop filter levels, if present.
// (delta q is never present in the saver's frames)
#[inline]
fn write_delta_q_params(cm: &Av1Common) {
    let delta_q_info = &cm.delta_q_info;
    debug_assert!(delta_q_info.delta_q_present_flag == 0);
}

fn write_intra_prediction_modes(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    is_keyframe: bool,
    w: &mut AomWriter,
) {
    let mbmi = *xd.mbmi();
    let mode = mbmi.mode;
    let bsize = mbmi.bsize;

    // Y mode.
    if is_keyframe {
        let above_mi = xd.above_mbmi.map(|i| xd.mi_alloc[i as usize]);
        let left_mi = xd.left_mbmi.map(|i| xd.mi_alloc[i as usize]);
        write_intra_y_mode_kf(
            &mut xd.tile_ctx,
            &mbmi,
            above_mi.as_ref(),
            left_mi.as_ref(),
            mode,
            w,
        );
    } else {
        write_intra_y_mode_nonkf(&mut xd.tile_ctx, bsize, mode, w);
    }

    // Y angle delta.
    let use_angle_delta = av1_use_angle_delta(bsize);
    if use_angle_delta && av1_is_directional_mode(mode) {
        write_angle_delta(
            w,
            mbmi.angle_delta[PLANE_TYPE_Y as usize] as i32,
            &mut xd.tile_ctx.angle_delta_cdf[(mode - V_PRED) as usize],
        );
    }

    // UV mode and UV angle delta.
    if cm.seq_params.monochrome == 0 && xd.is_chroma_ref {
        let uv_mode = mbmi.uv_mode;
        let cfl_allowed = is_cfl_allowed(xd);
        write_intra_uv_mode(&mut xd.tile_ctx, uv_mode, mode, cfl_allowed, w);
        if uv_mode == UV_CFL_PRED {
            write_cfl_alphas(
                &mut xd.tile_ctx,
                mbmi.cfl_alpha_idx,
                mbmi.cfl_alpha_signs,
                w,
            );
        }
        if use_angle_delta && av1_is_directional_mode(get_uv_mode(uv_mode)) {
            write_angle_delta(
                w,
                mbmi.angle_delta[PLANE_TYPE_UV as usize] as i32,
                &mut xd.tile_ctx.angle_delta_cdf[(uv_mode - V_PRED) as usize],
            );
        }
    }

    // Palette.
    // (write_palette_mode_info(): screen content tools are off)
    debug_assert!(!av1_allow_palette(
        cm.features.allow_screen_content_tools,
        bsize
    ));

    // Filter intra.
    write_filter_intra_mode_info(cm, xd, &mbmi, w);
}

fn write_mb_modes_kf(cpi: &Av1Comp, xd: &mut MacroBlockD, w: &mut AomWriter) {
    let cm = &cpi.common;
    let seg = &cm.seg;
    let mbmi = *xd.mbmi();

    // (write_segment_id(): segmentation is off)
    debug_assert!(!(seg.segid_preskip != 0 && seg.update_map != 0));
    let skip = write_skip(cm, xd, mbmi.segment_id, &mbmi, w);

    debug_assert!(!(seg.segid_preskip == 0 && seg.update_map != 0));

    write_cdef(cm, xd, w, skip);

    write_delta_q_params(cm);

    // (write_intrabc_info(): intra block copy is off)
    debug_assert!(!(av1_allow_intrabc(cm) && cm.features.allow_intrabc));

    write_intra_prediction_modes(cm, xd, true, w);
}

#[inline]
fn write_inter_mode(
    w: &mut AomWriter,
    mode: PredictionMode,
    ec_ctx: &mut FrameContext,
    mode_ctx: i16,
) {
    let newmv_ctx = (mode_ctx & NEWMV_CTX_MASK as i16) as usize;

    aom_write_symbol(
        w,
        (mode != NEWMV) as i32,
        &mut ec_ctx.newmv_cdf[newmv_ctx],
        2,
    );

    if mode != NEWMV {
        let zeromv_ctx = ((mode_ctx >> GLOBALMV_OFFSET) & GLOBALMV_CTX_MASK as i16) as usize;
        aom_write_symbol(
            w,
            (mode != GLOBALMV) as i32,
            &mut ec_ctx.zeromv_cdf[zeromv_ctx],
            2,
        );

        if mode != GLOBALMV {
            let refmv_ctx = ((mode_ctx >> REFMV_OFFSET) & REFMV_CTX_MASK as i16) as usize;
            aom_write_symbol(
                w,
                (mode != NEARESTMV) as i32,
                &mut ec_ctx.refmv_cdf[refmv_ctx],
                2,
            );
        }
    }
}

#[inline]
fn write_drl_idx(
    ec_ctx: &mut FrameContext,
    mbmi: &MbModeInfo,
    mbmi_ext_frame: &MbModeInfoExtFrame,
    w: &mut AomWriter,
) {
    debug_assert!(mbmi.ref_mv_idx < 3);

    let new_mv = mbmi.mode == NEWMV || mbmi.mode == NEW_NEWMV;
    let ref_mv_idx = mbmi.ref_mv_idx as i32;
    if new_mv {
        for idx in 0..2 {
            if mbmi_ext_frame.ref_mv_count as i32 > idx + 1 {
                let drl_ctx = av1_drl_ctx(&mbmi_ext_frame.weight, idx as usize) as usize;

                aom_write_symbol(
                    w,
                    (ref_mv_idx != idx) as i32,
                    &mut ec_ctx.drl_cdf[drl_ctx],
                    2,
                );
                if ref_mv_idx == idx {
                    return;
                }
            }
        }
        return;
    }

    if have_nearmv_in_inter_mode(mbmi.mode) {
        // TODO(jingning): Temporary solution to compensate the NEARESTMV offset.
        for idx in 1..3 {
            if mbmi_ext_frame.ref_mv_count as i32 > idx + 1 {
                let drl_ctx = av1_drl_ctx(&mbmi_ext_frame.weight, idx as usize) as usize;
                aom_write_symbol(
                    w,
                    (ref_mv_idx != (idx - 1)) as i32,
                    &mut ec_ctx.drl_cdf[drl_ctx],
                    2,
                );
                if ref_mv_idx == (idx - 1) {
                    return;
                }
            }
        }
    }
}

#[inline]
fn write_inter_compound_mode(
    xd: &mut MacroBlockD,
    w: &mut AomWriter,
    mode: PredictionMode,
    mode_ctx: i16,
) {
    debug_assert!(is_inter_compound_mode(mode));
    aom_write_symbol(
        w,
        mode - NEAREST_NEARESTMV,
        &mut xd.tile_ctx.inter_compound_mode_cdf[mode_ctx as usize],
        INTER_COMPOUND_MODES,
    );
}

fn write_tx_size_vartx(
    xd: &mut MacroBlockD,
    mbmi: &MbModeInfo,
    tx_size: TxSize,
    depth: i32,
    blk_row: i32,
    blk_col: i32,
    w: &mut AomWriter,
) {
    let max_blocks_high = max_block_high(xd, mbmi.bsize, 0);
    let max_blocks_wide = max_block_wide(xd, mbmi.bsize, 0);

    if blk_row >= max_blocks_high || blk_col >= max_blocks_wide {
        return;
    }

    let tile_row = xd.above_contexts_tile_row;
    let a = xd.above_txfm_context + blk_col as usize;
    let l = xd.left_txfm_context + blk_row as usize;
    if depth == MAX_VARTX_DEPTH {
        txfm_partition_update(
            &mut xd.above_contexts.txfm[tile_row][a..],
            &mut xd.left_txfm_context_buffer[l..],
            tx_size,
            tx_size,
        );
        return;
    }

    let ctx = txfm_partition_context(
        xd.above_contexts.txfm[tile_row][a],
        xd.left_txfm_context_buffer[l],
        mbmi.bsize,
        tx_size,
    ) as usize;
    let txb_size_index = av1_get_txb_size_index(mbmi.bsize, blk_row, blk_col);
    let write_txfm_partition = tx_size == mbmi.inter_tx_size[txb_size_index];
    if write_txfm_partition {
        aom_write_symbol(w, 0, &mut xd.tile_ctx.txfm_partition_cdf[ctx], 2);

        txfm_partition_update(
            &mut xd.above_contexts.txfm[tile_row][a..],
            &mut xd.left_txfm_context_buffer[l..],
            tx_size,
            tx_size,
        );
        // TODO(yuec): set correct txfm partition update for qttx
    } else {
        let sub_txs = SUB_TX_SIZE_MAP[tx_size as usize];
        let bsw = TX_SIZE_WIDE_UNIT[sub_txs as usize] as i32;
        let bsh = TX_SIZE_HIGH_UNIT[sub_txs as usize] as i32;

        aom_write_symbol(w, 1, &mut xd.tile_ctx.txfm_partition_cdf[ctx], 2);

        if sub_txs == TX_4X4 {
            txfm_partition_update(
                &mut xd.above_contexts.txfm[tile_row][a..],
                &mut xd.left_txfm_context_buffer[l..],
                sub_txs,
                tx_size,
            );
            return;
        }

        debug_assert!(bsw > 0 && bsh > 0);
        let mut row = 0;
        while row < TX_SIZE_HIGH_UNIT[tx_size as usize] as i32 {
            let offsetr = blk_row + row;
            let mut col = 0;
            while col < TX_SIZE_WIDE_UNIT[tx_size as usize] as i32 {
                let offsetc = blk_col + col;
                write_tx_size_vartx(xd, mbmi, sub_txs, depth + 1, offsetr, offsetc, w);
                col += bsw;
            }
            row += bsh;
        }
    }
}

fn write_skip_mode(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    segment_id: u8,
    mi: &MbModeInfo,
    w: &mut AomWriter,
) -> i32 {
    if cm.current_frame.skip_mode_info.skip_mode_flag == 0 {
        return 0;
    }
    if segfeature_active(&cm.seg, segment_id, SEG_LVL_SKIP) {
        return 0;
    }
    let skip_mode = mi.skip_mode as i32;
    if !is_comp_ref_allowed(mi.bsize) {
        debug_assert!(skip_mode == 0);
        return 0;
    }
    if segfeature_active(&cm.seg, segment_id, SEG_LVL_REF_FRAME)
        || segfeature_active(&cm.seg, segment_id, SEG_LVL_GLOBALMV)
    {
        // These features imply single-reference mode, while skip mode implies
        // compound reference. Hence, the two are mutually exclusive.
        // In other words, skip_mode is implicitly 0 here.
        debug_assert!(skip_mode == 0);
        return 0;
    }
    let ctx = av1_get_skip_mode_context(xd) as usize;
    aom_write_symbol(w, skip_mode, &mut xd.tile_ctx.skip_mode_cdfs[ctx], 2);
    skip_mode
}

#[inline]
fn write_is_inter(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    segment_id: u8,
    w: &mut AomWriter,
    is_inter: bool,
) {
    if !segfeature_active(&cm.seg, segment_id, SEG_LVL_REF_FRAME) {
        if segfeature_active(&cm.seg, segment_id, SEG_LVL_GLOBALMV) {
            debug_assert!(is_inter);
            return;
        }
        let ctx = av1_get_intra_inter_context(xd) as usize;
        aom_write_symbol(w, is_inter as i32, &mut xd.tile_ctx.intra_inter_cdf[ctx], 2);
    }
}

#[inline]
fn write_motion_mode(cm: &Av1Common, xd: &mut MacroBlockD, mbmi: &MbModeInfo, w: &mut AomWriter) {
    let last_motion_mode_allowed = if cm.features.switchable_motion_mode {
        motion_mode_allowed(&cm.global_motion, xd, mbmi, cm.features.allow_warped_motion)
    } else {
        SIMPLE_TRANSLATION
    };
    debug_assert!(mbmi.motion_mode <= last_motion_mode_allowed);
    match last_motion_mode_allowed {
        SIMPLE_TRANSLATION => {}
        OBMC_CAUSAL => {
            aom_write_symbol(
                w,
                (mbmi.motion_mode == OBMC_CAUSAL) as i32,
                &mut xd.tile_ctx.obmc_cdf[mbmi.bsize as usize],
                2,
            );
        }
        _ => {
            aom_write_symbol(
                w,
                mbmi.motion_mode as i32,
                &mut xd.tile_ctx.motion_mode_cdf[mbmi.bsize as usize],
                MOTION_MODES,
            );
        }
    }
}

// (no token stats: CONFIG_RD_DEBUG is off)
fn pack_txb_tokens(
    w: &mut AomWriter,
    cm: &Av1Common,
    x: &mut Macroblock,
    mbmi: &MbModeInfo,
    plane: usize,
    plane_bsize: BlockSize,
    mut block: i32,
    blk_row: i32,
    blk_col: i32,
    tx_size: TxSize,
) {
    let xd = &x.e_mbd;
    let max_blocks_high = max_block_high(xd, plane_bsize, plane);
    let max_blocks_wide = max_block_wide(xd, plane_bsize, plane);

    if blk_row >= max_blocks_high || blk_col >= max_blocks_wide {
        return;
    }

    let pd = &xd.plane[plane];
    let plane_tx_size = if plane != 0 {
        av1_get_max_uv_txsize(mbmi.bsize, pd.subsampling_x, pd.subsampling_y)
    } else {
        mbmi.inter_tx_size[av1_get_txb_size_index(plane_bsize, blk_row, blk_col)]
    };

    if tx_size == plane_tx_size || plane != 0 {
        av1_write_coeffs_txb(cm, x, w, blk_row, blk_col, plane, block, tx_size);
    } else {
        let sub_txs = SUB_TX_SIZE_MAP[tx_size as usize];
        let bsw = TX_SIZE_WIDE_UNIT[sub_txs as usize] as i32;
        let bsh = TX_SIZE_HIGH_UNIT[sub_txs as usize] as i32;
        let step = bsh * bsw;
        let row_end = (TX_SIZE_HIGH_UNIT[tx_size as usize] as i32).min(max_blocks_high - blk_row);
        let col_end = (TX_SIZE_WIDE_UNIT[tx_size as usize] as i32).min(max_blocks_wide - blk_col);

        debug_assert!(bsw > 0 && bsh > 0);

        let mut r = 0;
        while r < row_end {
            let offsetr = blk_row + r;
            let mut c = 0;
            while c < col_end {
                let offsetc = blk_col + c;
                pack_txb_tokens(
                    w,
                    cm,
                    x,
                    mbmi,
                    plane,
                    plane_bsize,
                    block,
                    offsetr,
                    offsetc,
                    sub_txs,
                );
                block += step;
                c += bsw;
            }
            r += bsh;
        }
    }
}

// This function encodes the reference frame
fn write_ref_frames(cm: &Av1Common, xd: &mut MacroBlockD, w: &mut AomWriter) {
    let mbmi = *xd.mbmi();
    let is_compound = has_second_ref(&mbmi);
    let segment_id = mbmi.segment_id;

    // If segment level coding of this signal is disabled...
    // or the segment allows multiple reference frame options
    if segfeature_active(&cm.seg, segment_id, SEG_LVL_REF_FRAME) {
        debug_assert!(!is_compound);
        debug_assert!(
            mbmi.ref_frame[0] as i32 == get_segdata(&cm.seg, segment_id as i32, SEG_LVL_REF_FRAME)
        );
    } else if segfeature_active(&cm.seg, segment_id, SEG_LVL_SKIP)
        || segfeature_active(&cm.seg, segment_id, SEG_LVL_GLOBALMV)
    {
        debug_assert!(!is_compound);
        debug_assert!(mbmi.ref_frame[0] == LAST_FRAME);
    } else {
        // does the feature use compound prediction or not
        // (if not specified at the frame/segment level)
        if cm.current_frame.reference_mode == REFERENCE_MODE_SELECT {
            if is_comp_ref_allowed(mbmi.bsize) {
                let ctx = av1_get_reference_mode_context(xd) as usize;
                aom_write_symbol(
                    w,
                    is_compound as i32,
                    &mut xd.tile_ctx.comp_inter_cdf[ctx],
                    2,
                );
            }
        } else {
            debug_assert!((!is_compound) == (cm.current_frame.reference_mode == SINGLE_REFERENCE));
        }

        // (WRITE_REF_BIT(bname, pname): the bit with the cdf of
        // av1_get_pred_cdf_pname(xd))
        if is_compound {
            let comp_ref_type = if has_uni_comp_refs(&mbmi) {
                UNIDIR_COMP_REFERENCE
            } else {
                BIDIR_COMP_REFERENCE
            };
            let ctx = av1_get_comp_reference_type_context(xd) as usize;
            aom_write_symbol(w, comp_ref_type, &mut xd.tile_ctx.comp_ref_type_cdf[ctx], 2);

            if comp_ref_type == UNIDIR_COMP_REFERENCE {
                let bit = (mbmi.ref_frame[0] == BWDREF_FRAME) as i32;
                let ctx = av1_get_pred_context_uni_comp_ref_p(xd) as usize;
                aom_write_symbol(w, bit, &mut xd.tile_ctx.uni_comp_ref_cdf[ctx][0], 2);

                if bit == 0 {
                    debug_assert!(mbmi.ref_frame[0] == LAST_FRAME);
                    let bit1 = (mbmi.ref_frame[1] == LAST3_FRAME
                        || mbmi.ref_frame[1] == GOLDEN_FRAME) as i32;
                    let ctx = av1_get_pred_context_uni_comp_ref_p1(xd) as usize;
                    aom_write_symbol(w, bit1, &mut xd.tile_ctx.uni_comp_ref_cdf[ctx][1], 2);
                    if bit1 != 0 {
                        let bit2 = (mbmi.ref_frame[1] == GOLDEN_FRAME) as i32;
                        let ctx = av1_get_pred_context_uni_comp_ref_p2(xd) as usize;
                        aom_write_symbol(w, bit2, &mut xd.tile_ctx.uni_comp_ref_cdf[ctx][2], 2);
                    }
                } else {
                    debug_assert!(mbmi.ref_frame[1] == ALTREF_FRAME);
                }

                return;
            }

            debug_assert!(comp_ref_type == BIDIR_COMP_REFERENCE);

            let bit =
                (mbmi.ref_frame[0] == GOLDEN_FRAME || mbmi.ref_frame[0] == LAST3_FRAME) as i32;
            let ctx = av1_get_pred_context_comp_ref_p(xd) as usize;
            aom_write_symbol(w, bit, &mut xd.tile_ctx.comp_ref_cdf[ctx][0], 2);

            if bit == 0 {
                let bit1 = (mbmi.ref_frame[0] == LAST2_FRAME) as i32;
                let ctx = av1_get_pred_context_comp_ref_p1(xd) as usize;
                aom_write_symbol(w, bit1, &mut xd.tile_ctx.comp_ref_cdf[ctx][1], 2);
            } else {
                let bit2 = (mbmi.ref_frame[0] == GOLDEN_FRAME) as i32;
                let ctx = av1_get_pred_context_comp_ref_p2(xd) as usize;
                aom_write_symbol(w, bit2, &mut xd.tile_ctx.comp_ref_cdf[ctx][2], 2);
            }

            let bit_bwd = (mbmi.ref_frame[1] == ALTREF_FRAME) as i32;
            let ctx = av1_get_pred_context_comp_bwdref_p(xd) as usize;
            aom_write_symbol(w, bit_bwd, &mut xd.tile_ctx.comp_bwdref_cdf[ctx][0], 2);

            if bit_bwd == 0 {
                let ctx = av1_get_pred_context_comp_bwdref_p1(xd) as usize;
                aom_write_symbol(
                    w,
                    (mbmi.ref_frame[1] == ALTREF2_FRAME) as i32,
                    &mut xd.tile_ctx.comp_bwdref_cdf[ctx][1],
                    2,
                );
            }
        } else {
            let bit0 =
                (mbmi.ref_frame[0] <= ALTREF_FRAME && mbmi.ref_frame[0] >= BWDREF_FRAME) as i32;
            let ctx = av1_get_pred_context_single_ref_p1(xd) as usize;
            aom_write_symbol(w, bit0, &mut xd.tile_ctx.single_ref_cdf[ctx][0], 2);

            if bit0 != 0 {
                let bit1 = (mbmi.ref_frame[0] == ALTREF_FRAME) as i32;
                let ctx = av1_get_pred_context_single_ref_p2(xd) as usize;
                aom_write_symbol(w, bit1, &mut xd.tile_ctx.single_ref_cdf[ctx][1], 2);

                if bit1 == 0 {
                    let ctx = av1_get_pred_context_single_ref_p6(xd) as usize;
                    aom_write_symbol(
                        w,
                        (mbmi.ref_frame[0] == ALTREF2_FRAME) as i32,
                        &mut xd.tile_ctx.single_ref_cdf[ctx][5],
                        2,
                    );
                }
            } else {
                let bit2 =
                    (mbmi.ref_frame[0] == LAST3_FRAME || mbmi.ref_frame[0] == GOLDEN_FRAME) as i32;
                let ctx = av1_get_pred_context_single_ref_p3(xd) as usize;
                aom_write_symbol(w, bit2, &mut xd.tile_ctx.single_ref_cdf[ctx][2], 2);

                if bit2 == 0 {
                    let bit3 = (mbmi.ref_frame[0] != LAST_FRAME) as i32;
                    let ctx = av1_get_pred_context_single_ref_p4(xd) as usize;
                    aom_write_symbol(w, bit3, &mut xd.tile_ctx.single_ref_cdf[ctx][3], 2);
                } else {
                    let bit4 = (mbmi.ref_frame[0] != LAST3_FRAME) as i32;
                    let ctx = av1_get_pred_context_single_ref_p5(xd) as usize;
                    aom_write_symbol(w, bit4, &mut xd.tile_ctx.single_ref_cdf[ctx][4], 2);
                }
            }
        }
    }
}

fn write_mb_interp_filter(cm: &Av1Common, td: &mut ThreadData, w: &mut AomWriter) {
    let xd = &mut td.mb.e_mbd;
    let mbmi = *xd.mbmi();

    if !av1_is_interp_needed(xd, &cm.global_motion) {
        let filters =
            av1_broadcast_interp_filter(av1_unswitchable_filter(cm.features.interp_filter));
        debug_assert!(mbmi.interp_filters == filters);
        let _ = filters;
        return;
    }
    if cm.features.interp_filter == SWITCHABLE {
        for dir in 0..2 {
            let ctx = av1_get_pred_context_switchable_interp(xd, dir) as usize;
            let filter = av1_extract_interp_filter(mbmi.interp_filters, dir);
            aom_write_symbol(
                w,
                filter,
                &mut xd.tile_ctx.switchable_interp_cdf[ctx],
                SWITCHABLE_FILTERS,
            );
            td.interp_filter_selected[filter as usize] += 1;
            if cm.seq_params.enable_dual_filter == 0 {
                return;
            }
        }
    }
}

#[inline]
fn write_intra_y_mode_nonkf(
    frame_ctx: &mut FrameContext,
    bsize: BlockSize,
    mode: PredictionMode,
    w: &mut AomWriter,
) {
    aom_write_symbol(
        w,
        mode,
        &mut frame_ctx.y_mode_cdf[SIZE_GROUP_LOOKUP[bsize as usize] as usize],
        INTRA_MODES,
    );
}

// (`Err` for segment map updates: segmentation is off)
#[inline]
fn write_inter_segment_id(seg: &Segmentation, skip: i32, preskip: i32) -> Result<(), ()> {
    let _ = (skip, preskip);
    if seg.update_map != 0 {
        // (write_segment_id(), set_spatial_segment_id())
        return Err(());
    }
    Ok(())
}

#[inline]
fn mode_context_analyzer(mode_context: i16, rf: &[MvReferenceFrame; 2]) -> i16 {
    if rf[1] <= INTRA_FRAME {
        return mode_context;
    }

    let newmv_ctx = mode_context & NEWMV_CTX_MASK as i16;
    let refmv_ctx = (mode_context >> REFMV_OFFSET) & REFMV_CTX_MASK as i16;

    COMPOUND_MODE_CTX_MAP[(refmv_ctx >> 1) as usize]
        [(newmv_ctx as i32).min(COMP_NEWMV_CTXS - 1) as usize] as i16
}

#[inline]
fn get_ref_mv_from_stack(
    ref_idx: i32,
    ref_frame: &[MvReferenceFrame; 2],
    ref_mv_idx: i32,
    mbmi_ext_frame: &MbModeInfoExtFrame,
) -> IntMv {
    let ref_frame_type = av1_ref_frame_type(ref_frame) as usize;
    let curr_ref_mv_stack = &mbmi_ext_frame.ref_mv_stack;

    if ref_frame[1] > INTRA_FRAME {
        debug_assert!(ref_idx == 0 || ref_idx == 1);
        return if ref_idx != 0 {
            curr_ref_mv_stack[ref_mv_idx as usize].comp_mv
        } else {
            curr_ref_mv_stack[ref_mv_idx as usize].this_mv
        };
    }

    debug_assert!(ref_idx == 0);
    if ref_mv_idx < mbmi_ext_frame.ref_mv_count as i32 {
        curr_ref_mv_stack[ref_mv_idx as usize].this_mv
    } else {
        mbmi_ext_frame.global_mvs[ref_frame_type]
    }
}

#[inline]
fn get_ref_mv(x: &Macroblock, ref_idx: i32) -> IntMv {
    let xd = &x.e_mbd;
    let mbmi = xd.mbmi();
    let mut ref_mv_idx = mbmi.ref_mv_idx as i32;
    if mbmi.mode == NEAR_NEWMV || mbmi.mode == NEW_NEARMV {
        debug_assert!(has_second_ref(mbmi));
        ref_mv_idx += 1;
    }
    get_ref_mv_from_stack(
        ref_idx,
        &mbmi.ref_frame,
        ref_mv_idx,
        &x.mbmi_ext_frame_base[x.mbmi_ext_frame],
    )
}

fn pack_inter_mode_mvs(cpi: &Av1Comp, td: &mut ThreadData, w: &mut AomWriter) -> Result<(), ()> {
    let cm = &cpi.common;
    let seg = &cm.seg;
    let mbmi = *td.mb.e_mbd.mbmi();
    let mbmi_ext_frame = td.mb.mbmi_ext_frame_base[td.mb.mbmi_ext_frame];
    let mode = mbmi.mode;
    let segment_id = mbmi.segment_id;
    let bsize = mbmi.bsize;
    let allow_hp = cm.features.allow_high_precision_mv;
    let is_inter = is_inter_block(&mbmi);
    let is_compound = has_second_ref(&mbmi);

    write_inter_segment_id(seg, 0, 1)?;

    write_skip_mode(cm, &mut td.mb.e_mbd, segment_id, &mbmi, w);

    debug_assert!(mbmi.skip_mode == 0 || mbmi.skip_txfm != 0);
    let skip = if mbmi.skip_mode != 0 {
        1
    } else {
        write_skip(cm, &mut td.mb.e_mbd, segment_id, &mbmi, w)
    };

    write_inter_segment_id(seg, skip, 0)?;

    write_cdef(cm, &mut td.mb.e_mbd, w, skip);

    write_delta_q_params(cm);

    if mbmi.skip_mode == 0 {
        write_is_inter(cm, &mut td.mb.e_mbd, mbmi.segment_id, w, is_inter);
    }

    if mbmi.skip_mode != 0 {
        return Ok(());
    }

    if !is_inter {
        write_intra_prediction_modes(cm, &mut td.mb.e_mbd, false, w);
    } else {
        let xd = &mut td.mb.e_mbd;
        av1_collect_neighbors_ref_counts(xd);

        write_ref_frames(cm, xd, w);

        let mode_ctx = mode_context_analyzer(mbmi_ext_frame.mode_context, &mbmi.ref_frame);

        // If segment skip is not enabled code the mode.
        if !segfeature_active(seg, segment_id, SEG_LVL_SKIP) {
            if is_inter_compound_mode(mode) {
                write_inter_compound_mode(xd, w, mode, mode_ctx);
            } else if is_inter_singleref_mode(mode) {
                write_inter_mode(w, mode, &mut xd.tile_ctx, mode_ctx);
            }

            if mode == NEWMV || mode == NEW_NEWMV || have_nearmv_in_inter_mode(mode) {
                write_drl_idx(&mut xd.tile_ctx, &mbmi, &mbmi_ext_frame, w);
            } else {
                debug_assert!(mbmi.ref_mv_idx == 0);
            }
        }

        let cur_frame_force_integer_mv = cm.features.cur_frame_force_integer_mv;
        let auto_mv_step_size = cpi.sf.mv_sf.auto_mv_step_size != 0;
        if mode == NEWMV || mode == NEW_NEWMV {
            for ref_ in 0..1 + is_compound as usize {
                let ref_mv = get_ref_mv(&td.mb, ref_ as i32);
                av1_encode_mv(
                    cur_frame_force_integer_mv,
                    auto_mv_step_size,
                    w,
                    &mut td.max_mv_magnitude,
                    &mbmi.mv[ref_].as_mv(),
                    &ref_mv.as_mv(),
                    &mut td.mb.e_mbd.tile_ctx.nmvc,
                    allow_hp as i32,
                );
            }
        } else if mode == NEAREST_NEWMV || mode == NEAR_NEWMV {
            let ref_mv = get_ref_mv(&td.mb, 1);
            av1_encode_mv(
                cur_frame_force_integer_mv,
                auto_mv_step_size,
                w,
                &mut td.max_mv_magnitude,
                &mbmi.mv[1].as_mv(),
                &ref_mv.as_mv(),
                &mut td.mb.e_mbd.tile_ctx.nmvc,
                allow_hp as i32,
            );
        } else if mode == NEW_NEARESTMV || mode == NEW_NEARMV {
            let ref_mv = get_ref_mv(&td.mb, 0);
            av1_encode_mv(
                cur_frame_force_integer_mv,
                auto_mv_step_size,
                w,
                &mut td.max_mv_magnitude,
                &mbmi.mv[0].as_mv(),
                &ref_mv.as_mv(),
                &mut td.mb.e_mbd.tile_ctx.nmvc,
                allow_hp as i32,
            );
        }

        let xd = &mut td.mb.e_mbd;
        let ec_ctx: &mut FrameContext = &mut xd.tile_ctx;
        if cpi.common.current_frame.reference_mode != COMPOUND_REFERENCE
            && cpi.common.seq_params.enable_interintra_compound != 0
            && is_interintra_allowed(&mbmi)
        {
            let interintra = (mbmi.ref_frame[1] == INTRA_FRAME) as i32;
            let bsize_group = SIZE_GROUP_LOOKUP[bsize as usize] as usize;
            aom_write_symbol(w, interintra, &mut ec_ctx.interintra_cdf[bsize_group], 2);
            if interintra != 0 {
                aom_write_symbol(
                    w,
                    mbmi.interintra_mode,
                    &mut ec_ctx.interintra_mode_cdf[bsize_group],
                    INTERINTRA_MODES,
                );
                if av1_is_wedge_used(bsize) {
                    aom_write_symbol(
                        w,
                        mbmi.use_wedge_interintra as i32,
                        &mut ec_ctx.wedge_interintra_cdf[bsize as usize],
                        2,
                    );
                    if mbmi.use_wedge_interintra != 0 {
                        aom_write_symbol(
                            w,
                            mbmi.interintra_wedge_index as i32,
                            &mut ec_ctx.wedge_idx_cdf[bsize as usize],
                            16, /* MAX_WEDGE_TYPES */
                        );
                    }
                }
            }
        }

        if mbmi.ref_frame[1] != INTRA_FRAME {
            write_motion_mode(cm, xd, &mbmi, w);
        }

        // First write idx to indicate current compound inter prediction mode group
        // Group A (0): dist_wtd_comp, compound_average
        // Group B (1): interintra, compound_diffwtd, wedge
        if has_second_ref(&mbmi) {
            let masked_compound_used =
                is_any_masked_compound_used(bsize) && cm.seq_params.enable_masked_compound != 0;

            if masked_compound_used {
                let ctx_comp_group_idx = get_comp_group_idx_context(xd) as usize;
                aom_write_symbol(
                    w,
                    mbmi.comp_group_idx as i32,
                    &mut xd.tile_ctx.comp_group_idx_cdf[ctx_comp_group_idx],
                    2,
                );
            } else {
                debug_assert!(mbmi.comp_group_idx == 0);
            }

            if mbmi.comp_group_idx == 0 {
                if mbmi.compound_idx != 0 {
                    debug_assert!(mbmi.interinter_comp.type_ == COMPOUND_AVERAGE);
                }

                if cm.seq_params.order_hint_info.enable_dist_wtd_comp != 0 {
                    let comp_index_ctx = get_comp_index_context(cm, xd) as usize;
                    aom_write_symbol(
                        w,
                        mbmi.compound_idx as i32,
                        &mut xd.tile_ctx.compound_index_cdf[comp_index_ctx],
                        2,
                    );
                } else {
                    debug_assert!(mbmi.compound_idx == 1);
                }
            } else {
                debug_assert!(
                    cpi.common.current_frame.reference_mode != SINGLE_REFERENCE
                        && is_inter_compound_mode(mbmi.mode)
                        && mbmi.motion_mode == SIMPLE_TRANSLATION
                );
                debug_assert!(masked_compound_used);
                // compound_diffwtd, wedge
                // (the masked compounds are never picked by the non-RD mode
                // search)
                return Err(());
            }
        }
        write_mb_interp_filter(cm, td, w);
    }
    Ok(())
}

fn write_inter_txb_coeff(
    cm: &Av1Common,
    x: &mut Macroblock,
    mbmi: &MbModeInfo,
    w: &mut AomWriter,
    row: i32,
    col: i32,
    block: &mut i32,
    plane: usize,
) {
    let xd = &x.e_mbd;
    let pd = &xd.plane[plane];
    let bsize = mbmi.bsize;
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let ss_x = pd.subsampling_x;
    let ss_y = pd.subsampling_y;
    let plane_bsize = get_plane_block_size(bsize, ss_x, ss_y);
    debug_assert!(plane_bsize < BLOCK_SIZES_ALL);
    let max_tx_size = get_vartx_max_txsize(xd, plane_bsize, plane);
    let step = TX_SIZE_WIDE_UNIT[max_tx_size as usize] as i32
        * TX_SIZE_HIGH_UNIT[max_tx_size as usize] as i32;
    let bkw = TX_SIZE_WIDE_UNIT[max_tx_size as usize] as i32;
    let bkh = TX_SIZE_HIGH_UNIT[max_tx_size as usize] as i32;
    let max_unit_bsize = get_plane_block_size(BLOCK_64X64, ss_x, ss_y);
    let num_4x4_w = MI_SIZE_WIDE[plane_bsize as usize] as i32;
    let num_4x4_h = MI_SIZE_HIGH[plane_bsize as usize] as i32;
    let mu_blocks_wide = MI_SIZE_WIDE[max_unit_bsize as usize] as i32;
    let mu_blocks_high = MI_SIZE_HIGH[max_unit_bsize as usize] as i32;
    let unit_height = (mu_blocks_high + (row >> ss_y)).min(num_4x4_h);
    let unit_width = (mu_blocks_wide + (col >> ss_x)).min(num_4x4_w);
    let mut blk_row = row >> ss_y;
    while blk_row < unit_height {
        let mut blk_col = col >> ss_x;
        while blk_col < unit_width {
            pack_txb_tokens(
                w,
                cm,
                x,
                mbmi,
                plane,
                plane_bsize,
                *block,
                blk_row,
                blk_col,
                max_tx_size,
            );
            *block += step;
            blk_col += bkw;
        }
        blk_row += bkh;
    }
}

fn write_mbmi_b(cpi: &Av1Comp, td: &mut ThreadData, w: &mut AomWriter) -> Result<(), ()> {
    let cm = &cpi.common;
    let xd = &mut td.mb.e_mbd;
    let m = *xd.mbmi();

    if frame_is_intra_only(cm) {
        write_mb_modes_kf(cpi, xd, w);
    } else {
        // has_subpel_mv_component needs the ref frame buffers set up to look
        // up if they are scaled. has_subpel_mv_component is in turn needed by
        // write_switchable_interp_filter, which is called by pack_inter_mode_mvs.
        set_ref_ptrs(cm, xd, m.ref_frame[0], m.ref_frame[1])?;

        // (ENC_MISMATCH_DEBUG is off)

        pack_inter_mode_mvs(cpi, td, w)?;
    }
    Ok(())
}

fn write_tokens_b(cpi: &Av1Comp, x: &mut Macroblock, w: &mut AomWriter) {
    let cm = &cpi.common;
    let mbmi = *x.e_mbd.mbmi();
    let bsize = mbmi.bsize;

    debug_assert!(mbmi.skip_txfm == 0);

    let is_inter = is_inter_block(&mbmi);
    if !is_inter {
        av1_write_intra_coeffs_mb(cm, x, w, bsize);
    } else {
        let mut block = [0i32; MAX_MB_PLANE];
        let xd = &x.e_mbd;
        debug_assert!(
            bsize
                == get_plane_block_size(
                    bsize,
                    xd.plane[0].subsampling_x,
                    xd.plane[0].subsampling_y
                )
        );
        let num_4x4_w = MI_SIZE_WIDE[bsize as usize] as i32;
        let num_4x4_h = MI_SIZE_HIGH[bsize as usize] as i32;
        // (init_token_stats(): CONFIG_RD_DEBUG is off)

        let max_unit_bsize = BLOCK_64X64;
        let mut mu_blocks_wide = MI_SIZE_WIDE[max_unit_bsize as usize] as i32;
        let mut mu_blocks_high = MI_SIZE_HIGH[max_unit_bsize as usize] as i32;
        mu_blocks_wide = num_4x4_w.min(mu_blocks_wide);
        mu_blocks_high = num_4x4_h.min(mu_blocks_high);

        let num_planes = av1_num_planes(cm);
        let mut row = 0;
        while row < num_4x4_h {
            let mut col = 0;
            while col < num_4x4_w {
                for plane in 0..num_planes {
                    if plane != 0 && !x.e_mbd.is_chroma_ref {
                        break;
                    }
                    write_inter_txb_coeff(cm, x, &mbmi, w, row, col, &mut block[plane], plane);
                }
                col += mu_blocks_wide;
            }
            row += mu_blocks_high;
        }
    }
}

fn write_modes_b(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile: &TileInfo,
    w: &mut AomWriter,
    mi_row: i32,
    mi_col: i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let mi_params = &cm.mi_params;
    let grid_idx = (mi_row * mi_params.mi_stride + mi_col) as usize;
    let x = &mut td.mb;
    x.e_mbd.mi = grid_idx;
    x.mbmi_ext_frame = get_mi_ext_idx(
        mi_row,
        mi_col,
        cm.mi_params.mi_alloc_bsize,
        cpi.mbmi_ext_info.stride,
    ) as usize;
    let xd = &mut x.e_mbd;
    xd.tx_type_map = TxTypeMapRef::Frame(grid_idx);
    xd.tx_type_map_stride = mi_params.mi_stride;

    let mbmi = *xd.mbmi();
    let bsize = mbmi.bsize;
    debug_assert!(
        bsize <= cm.seq_params.sb_size || (bsize >= BLOCK_SIZES && bsize < BLOCK_SIZES_ALL)
    );

    let bh = MI_SIZE_HIGH[bsize as usize] as i32;
    let bw = MI_SIZE_WIDE[bsize as usize] as i32;
    set_mi_row_col(
        xd,
        tile,
        mi_row,
        bh,
        mi_col,
        bw,
        mi_params.mi_rows,
        mi_params.mi_cols,
    );

    xd.above_txfm_context = mi_col as usize;
    xd.above_contexts_tile_row = tile.tile_row as usize;
    xd.left_txfm_context = (mi_row & MAX_MIB_MASK) as usize;

    write_mbmi_b(cpi, td, w)?;

    // (palette color maps: no palettes)
    debug_assert!(mbmi.palette_mode_info.palette_size == [0, 0]);

    let xd = &mut td.mb.e_mbd;
    let is_inter_tx = is_inter_block(&mbmi);
    let skip_txfm = mbmi.skip_txfm;
    let segment_id = mbmi.segment_id;
    if cm.features.tx_mode == TX_MODE_SELECT
        && block_signals_txsize(bsize)
        && !(is_inter_tx && skip_txfm != 0)
        && xd.lossless[segment_id as usize] == 0
    {
        if is_inter_tx {
            // This implies skip flag is 0.
            let max_tx_size = get_vartx_max_txsize(xd, bsize, 0);
            let txbh = TX_SIZE_HIGH_UNIT[max_tx_size as usize] as i32;
            let txbw = TX_SIZE_WIDE_UNIT[max_tx_size as usize] as i32;
            let width = MI_SIZE_WIDE[bsize as usize] as i32;
            let height = MI_SIZE_HIGH[bsize as usize] as i32;
            let mut idy = 0;
            while idy < height {
                let mut idx = 0;
                while idx < width {
                    write_tx_size_vartx(xd, &mbmi, max_tx_size, 0, idy, idx, w);
                    idx += txbw;
                }
                idy += txbh;
            }
        } else {
            write_selected_tx_size(xd, w);
            let (wd, ht) = (xd.width as i32, xd.height as i32);
            set_txfm_ctxs(mbmi.tx_size, wd, ht, false, xd);
        }
    } else {
        let (wd, ht) = (xd.width as i32, xd.height as i32);
        set_txfm_ctxs(mbmi.tx_size, wd, ht, skip_txfm != 0 && is_inter_tx, xd);
    }

    if mbmi.skip_txfm == 0 {
        let start = aom_tell_size(w);

        write_tokens_b(cpi, &mut td.mb, w);

        let end = aom_tell_size(w);
        td.coefficient_size += (end - start) as i64;
    }
    Ok(())
}

fn write_partition(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    hbs: i32,
    mi_row: i32,
    mi_col: i32,
    p: PartitionType,
    bsize: BlockSize,
    w: &mut AomWriter,
) {
    let is_partition_point = bsize >= BLOCK_8X8;

    if !is_partition_point {
        return;
    }

    let has_rows = (mi_row + hbs) < cm.mi_params.mi_rows;
    let has_cols = (mi_col + hbs) < cm.mi_params.mi_cols;
    let ctx = partition_plane_context(xd, mi_row, mi_col, bsize) as usize;
    let ec_ctx = &mut xd.tile_ctx;

    if !has_rows && !has_cols {
        debug_assert!(p == PARTITION_SPLIT);
        return;
    }

    if has_rows && has_cols {
        aom_write_symbol(
            w,
            p,
            &mut ec_ctx.partition_cdf[ctx],
            partition_cdf_length(bsize),
        );
    } else if !has_rows && has_cols {
        debug_assert!(p == PARTITION_SPLIT || p == PARTITION_HORZ);
        debug_assert!(bsize > BLOCK_8X8);
        let mut cdf = [0u16; 3];
        partition_gather_vert_alike(&mut cdf, &ec_ctx.partition_cdf[ctx], bsize);
        aom_write_cdf(w, (p == PARTITION_SPLIT) as i32, &cdf, 2);
    } else {
        debug_assert!(has_rows && !has_cols);
        debug_assert!(p == PARTITION_SPLIT || p == PARTITION_VERT);
        debug_assert!(bsize > BLOCK_8X8);
        let mut cdf = [0u16; 3];
        partition_gather_horz_alike(&mut cdf, &ec_ctx.partition_cdf[ctx], bsize);
        aom_write_cdf(w, (p == PARTITION_SPLIT) as i32, &cdf, 2);
    }
}

fn write_modes_sb(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile: &TileInfo,
    w: &mut AomWriter,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let mi_params = &cm.mi_params;
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let hbs = MI_SIZE_WIDE[bsize as usize] as i32 / 2;
    let quarter_step = MI_SIZE_WIDE[bsize as usize] as i32 / 4;
    let partition = get_partition(mi_params, &td.mb.e_mbd, mi_row, mi_col, bsize);
    let subsize = get_partition_subsize(bsize, partition);

    if mi_row >= mi_params.mi_rows || mi_col >= mi_params.mi_cols {
        return Ok(());
    }

    let num_planes = av1_num_planes(cm);
    for plane in 0..num_planes {
        // (loop restoration is off: no restoration coefficients to write)
        let r = av1_loop_restoration_corners_in_sb(cm, plane, bsize);
        debug_assert!(r == 0);
    }

    write_partition(
        cm,
        &mut td.mb.e_mbd,
        hbs,
        mi_row,
        mi_col,
        partition,
        bsize,
        w,
    );
    match partition {
        PARTITION_NONE => write_modes_b(cpi, td, tile, w, mi_row, mi_col)?,
        PARTITION_HORZ => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col)?;
            if mi_row + hbs < mi_params.mi_rows {
                write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col)?;
            }
        }
        PARTITION_VERT => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col)?;
            if mi_col + hbs < mi_params.mi_cols {
                write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs)?;
            }
        }
        PARTITION_SPLIT => {
            write_modes_sb(cpi, td, tile, w, mi_row, mi_col, subsize)?;
            write_modes_sb(cpi, td, tile, w, mi_row, mi_col + hbs, subsize)?;
            write_modes_sb(cpi, td, tile, w, mi_row + hbs, mi_col, subsize)?;
            write_modes_sb(cpi, td, tile, w, mi_row + hbs, mi_col + hbs, subsize)?;
        }
        PARTITION_HORZ_A => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col)?;
            write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs)?;
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col)?;
        }
        PARTITION_HORZ_B => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col)?;
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col)?;
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col + hbs)?;
        }
        PARTITION_VERT_A => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col)?;
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col)?;
            write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs)?;
        }
        PARTITION_VERT_B => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col)?;
            write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs)?;
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col + hbs)?;
        }
        PARTITION_HORZ_4 => {
            for i in 0..4 {
                let this_mi_row = mi_row + i * quarter_step;
                if i > 0 && this_mi_row >= mi_params.mi_rows {
                    break;
                }

                write_modes_b(cpi, td, tile, w, this_mi_row, mi_col)?;
            }
        }
        PARTITION_VERT_4 => {
            for i in 0..4 {
                let this_mi_col = mi_col + i * quarter_step;
                if i > 0 && this_mi_col >= mi_params.mi_cols {
                    break;
                }

                write_modes_b(cpi, td, tile, w, mi_row, this_mi_col)?;
            }
        }
        _ => debug_assert!(false),
    }

    // update partition context
    update_ext_partition_context(&mut td.mb.e_mbd, mi_row, mi_col, subsize, bsize, partition);
    Ok(())
}

fn write_modes(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile: &TileInfo,
    w: &mut AomWriter,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let mi_row_start = tile.mi_row_start;
    let mi_row_end = tile.mi_row_end;
    let mi_col_start = tile.mi_col_start;
    let mi_col_end = tile.mi_col_end;

    let xd = &mut td.mb.e_mbd;
    av1_zero_above_context(
        cm,
        &mut xd.above_contexts,
        mi_col_start,
        mi_col_end,
        tile.tile_row,
    );
    av1_init_above_context(tile.tile_row, xd);

    // (delta q is never present)
    debug_assert!(cpi.common.delta_q_info.delta_q_present_flag == 0);

    let mut mi_row = mi_row_start;
    while mi_row < mi_row_end {
        // (get_token_pointers(): no palette tokens)

        av1_zero_left_context(&mut td.mb.e_mbd);

        let mut mi_col = mi_col_start;
        while mi_col < mi_col_end {
            td.mb.cb_coef_buff = av1_get_cb_coeff_buffer(cpi, mi_row, mi_col);
            write_modes_sb(cpi, td, tile, w, mi_row, mi_col, cm.seq_params.sb_size)?;
            mi_col += cm.seq_params.mib_size;
        }
        mi_row += cm.seq_params.mib_size;
    }
    Ok(())
}

fn encode_restoration_mode(cm: &Av1Common) -> Result<(), ()> {
    debug_assert!(!cm.features.all_lossless);
    if cm.seq_params.enable_restoration == 0 {
        return Ok(());
    }
    // (loop restoration is disabled in the saver's sequences)
    Err(())
}

// Only write out the ref delta section if any of the elements
// will signal a delta.
fn is_mode_ref_delta_meaningful(cm: &Av1Common) -> bool {
    let lf = &cm.lf;
    if lf.mode_ref_delta_update == 0 {
        return false;
    }
    let buf = get_primary_ref_frame_buf(cm);
    let mut last_ref_deltas = [0i8; REF_FRAMES as usize];
    let mut last_mode_deltas = [0i8; MAX_MODE_LF_DELTAS as usize];
    match buf {
        None => {
            av1_set_default_ref_deltas(&mut last_ref_deltas);
            av1_set_default_mode_deltas(&mut last_mode_deltas);
        }
        Some(b) => {
            last_ref_deltas = cm.buffer_pool.frame_bufs[b].ref_deltas;
            last_mode_deltas = cm.buffer_pool.frame_bufs[b].mode_deltas;
        }
    }
    for i in 0..REF_FRAMES as usize {
        if lf.ref_deltas[i] != last_ref_deltas[i] {
            return true;
        }
    }
    for i in 0..MAX_MODE_LF_DELTAS as usize {
        if lf.mode_deltas[i] != last_mode_deltas[i] {
            return true;
        }
    }
    false
}

fn encode_loopfilter(cm: &Av1Common, wb: &mut AomWriteBitBuffer) {
    debug_assert!(!cm.features.coded_lossless);
    if cm.features.allow_intrabc {
        return;
    }
    let num_planes = av1_num_planes(cm);
    let lf = &cm.lf;

    // Encode the loop filter level and type
    aom_wb_write_literal(wb, lf.filter_level[0], 6);
    aom_wb_write_literal(wb, lf.filter_level[1], 6);
    if num_planes > 1 && (lf.filter_level[0] != 0 || lf.filter_level[1] != 0) {
        aom_wb_write_literal(wb, lf.filter_level_u, 6);
        aom_wb_write_literal(wb, lf.filter_level_v, 6);
    }
    aom_wb_write_literal(wb, lf.sharpness_level, 3);

    aom_wb_write_bit(wb, lf.mode_ref_delta_enabled as i32);

    // Write out loop filter deltas applied at the MB level based on mode or
    // ref frame (if they are enabled), only if there is information to write.
    let meaningful = is_mode_ref_delta_meaningful(cm);
    aom_wb_write_bit(wb, meaningful as i32);
    if !meaningful {
        return;
    }

    let buf = get_primary_ref_frame_buf(cm);
    let mut last_ref_deltas = [0i8; REF_FRAMES as usize];
    let mut last_mode_deltas = [0i8; MAX_MODE_LF_DELTAS as usize];
    match buf {
        None => {
            av1_set_default_ref_deltas(&mut last_ref_deltas);
            av1_set_default_mode_deltas(&mut last_mode_deltas);
        }
        Some(b) => {
            last_ref_deltas = cm.buffer_pool.frame_bufs[b].ref_deltas;
            last_mode_deltas = cm.buffer_pool.frame_bufs[b].mode_deltas;
        }
    }
    for i in 0..REF_FRAMES as usize {
        let delta = lf.ref_deltas[i] as i32;
        let changed = delta != last_ref_deltas[i] as i32;
        aom_wb_write_bit(wb, changed as i32);
        if changed {
            aom_wb_write_inv_signed_literal(wb, delta, 6);
        }
    }
    for i in 0..MAX_MODE_LF_DELTAS as usize {
        let delta = lf.mode_deltas[i] as i32;
        let changed = delta != last_mode_deltas[i] as i32;
        aom_wb_write_bit(wb, changed as i32);
        if changed {
            aom_wb_write_inv_signed_literal(wb, delta, 6);
        }
    }
}

fn encode_cdef(cm: &Av1Common, wb: &mut AomWriteBitBuffer) {
    debug_assert!(!cm.features.coded_lossless);
    if cm.seq_params.enable_cdef == 0 {
        return;
    }
    if cm.features.allow_intrabc {
        return;
    }
    let num_planes = av1_num_planes(cm);
    aom_wb_write_literal(wb, cm.cdef_info.cdef_damping - 3, 2);
    aom_wb_write_literal(wb, cm.cdef_info.cdef_bits, 2);
    for i in 0..cm.cdef_info.nb_cdef_strengths as usize {
        aom_wb_write_literal(wb, cm.cdef_info.cdef_strengths[i], CDEF_STRENGTH_BITS);
        if num_planes > 1 {
            aom_wb_write_literal(wb, cm.cdef_info.cdef_uv_strengths[i], CDEF_STRENGTH_BITS);
        }
    }
}

fn write_delta_q(wb: &mut AomWriteBitBuffer, delta_q: i32) {
    if delta_q != 0 {
        aom_wb_write_bit(wb, 1);
        aom_wb_write_inv_signed_literal(wb, delta_q, 6);
    } else {
        aom_wb_write_bit(wb, 0);
    }
}

fn encode_quantization(
    quant_params: &CommonQuantParams,
    num_planes: usize,
    separate_uv_delta_q: bool,
    wb: &mut AomWriteBitBuffer,
) {
    aom_wb_write_literal(wb, quant_params.base_qindex, QINDEX_BITS);
    write_delta_q(wb, quant_params.y_dc_delta_q);
    if num_planes > 1 {
        let diff_uv_delta = (quant_params.u_dc_delta_q != quant_params.v_dc_delta_q)
            || (quant_params.u_ac_delta_q != quant_params.v_ac_delta_q);
        if separate_uv_delta_q {
            aom_wb_write_bit(wb, diff_uv_delta as i32);
        }
        write_delta_q(wb, quant_params.u_dc_delta_q);
        write_delta_q(wb, quant_params.u_ac_delta_q);
        if diff_uv_delta {
            write_delta_q(wb, quant_params.v_dc_delta_q);
            write_delta_q(wb, quant_params.v_ac_delta_q);
        }
    }
    aom_wb_write_bit(wb, quant_params.using_qmatrix as i32);
    // (quantizer matrices are off)
    debug_assert!(!quant_params.using_qmatrix);
}

fn encode_segmentation(cm: &Av1Common, wb: &mut AomWriteBitBuffer) -> Result<(), ()> {
    let seg = &cm.seg;

    aom_wb_write_bit(wb, seg.enabled as i32);
    if seg.enabled == 0 {
        return Ok(());
    }
    // (segmentation is off for the saver)
    Err(())
}

fn write_frame_interp_filter(filter: i32, wb: &mut AomWriteBitBuffer) {
    aom_wb_write_bit(wb, (filter == SWITCHABLE) as i32);
    if filter != SWITCHABLE {
        aom_wb_write_literal(wb, filter, LOG_SWITCHABLE_FILTERS);
    }
}

fn write_tile_info_max_tile(cm: &Av1Common, wb: &mut AomWriteBitBuffer) -> Result<(), ()> {
    let tiles = &cm.tiles;

    aom_wb_write_bit(wb, tiles.uniform_spacing as i32);

    if tiles.uniform_spacing == 0 {
        // (the saver's tiles are uniformly spaced)
        return Err(());
    }
    let mut ones = tiles.log2_cols - tiles.min_log2_cols;
    while ones > 0 {
        aom_wb_write_bit(wb, 1);
        ones -= 1;
    }
    if tiles.log2_cols < tiles.max_log2_cols {
        aom_wb_write_bit(wb, 0);
    }

    // rows
    ones = tiles.log2_rows - tiles.min_log2_rows;
    while ones > 0 {
        aom_wb_write_bit(wb, 1);
        ones -= 1;
    }
    if tiles.log2_rows < tiles.max_log2_rows {
        aom_wb_write_bit(wb, 0);
    }
    let _ = (
        ceil_power_of_two(cm.mi_params.mi_cols, cm.seq_params.mib_size_log2),
        ceil_power_of_two(cm.mi_params.mi_rows, cm.seq_params.mib_size_log2),
    );
    Ok(())
}

fn write_tile_info(
    cm: &Av1Common,
    saved_wb: &mut SavedWb,
    wb: &mut AomWriteBitBuffer,
) -> Result<(), ()> {
    write_tile_info_max_tile(cm, wb)?;

    *saved_wb = wb.save();
    if cm.tiles.rows * cm.tiles.cols > 1 {
        // tile id used for cdf update
        aom_wb_write_literal(wb, 0, cm.tiles.log2_cols + cm.tiles.log2_rows);
        // Number of bytes in tile size - 1
        aom_wb_write_literal(wb, 3, 2);
    }
    Ok(())
}

fn write_render_size(cm: &Av1Common, wb: &mut AomWriteBitBuffer) {
    let scaling_active = av1_resize_scaled(cm);
    aom_wb_write_bit(wb, scaling_active as i32);
    if scaling_active {
        aom_wb_write_literal(wb, cm.render_width - 1, 16);
        aom_wb_write_literal(wb, cm.render_height - 1, 16);
    }
}

fn write_superres_scale(cm: &Av1Common) -> Result<(), ()> {
    let seq_params = &cm.seq_params;
    if seq_params.enable_superres == 0 {
        debug_assert!(cm.superres_scale_denominator as i32 == SCALE_NUMERATOR);
        return Ok(());
    }
    // (superres is disabled in the saver's sequences)
    Err(())
}

fn write_frame_size(
    cm: &Av1Common,
    frame_size_override: bool,
    wb: &mut AomWriteBitBuffer,
) -> Result<(), ()> {
    if frame_size_override {
        // (the saver's frames all have the sequence's size)
        return Err(());
    }

    write_superres_scale(cm)?;
    write_render_size(cm, wb);
    Ok(())
}

#[inline]
fn write_profile(profile: i32, wb: &mut AomWriteBitBuffer) {
    debug_assert!((PROFILE_0..MAX_PROFILES).contains(&profile));
    aom_wb_write_literal(wb, profile, PROFILE_BITS);
}

#[inline]
fn write_bitdepth(seq_params: &SequenceHeader, wb: &mut AomWriteBitBuffer) {
    // Profile 0/1: [0] for 8 bit, [1]  10-bit
    // Profile   2: [0] for 8 bit, [10] 10-bit, [11] - 12-bit
    aom_wb_write_bit(
        wb,
        if seq_params.bit_depth == AOM_BITS_8 {
            0
        } else {
            1
        },
    );
    if seq_params.profile == PROFILE_2 && seq_params.bit_depth != AOM_BITS_8 {
        aom_wb_write_bit(
            wb,
            if seq_params.bit_depth == AOM_BITS_10 {
                0
            } else {
                1
            },
        );
    }
}

fn write_color_config(seq_params: &SequenceHeader, wb: &mut AomWriteBitBuffer) {
    write_bitdepth(seq_params, wb);
    let is_monochrome = seq_params.monochrome != 0;
    // monochrome bit
    if seq_params.profile != PROFILE_1 {
        aom_wb_write_bit(wb, is_monochrome as i32);
    } else {
        debug_assert!(!is_monochrome);
    }
    if seq_params.color_primaries == AOM_CICP_CP_UNSPECIFIED
        && seq_params.transfer_characteristics == AOM_CICP_TC_UNSPECIFIED
        && seq_params.matrix_coefficients == AOM_CICP_MC_UNSPECIFIED
    {
        aom_wb_write_bit(wb, 0); // No color description present
    } else {
        aom_wb_write_bit(wb, 1); // Color description present
        aom_wb_write_literal(wb, seq_params.color_primaries, 8);
        aom_wb_write_literal(wb, seq_params.transfer_characteristics, 8);
        aom_wb_write_literal(wb, seq_params.matrix_coefficients, 8);
    }
    if is_monochrome {
        // 0: [16, 235] (i.e. xvYCC), 1: [0, 255]
        aom_wb_write_bit(wb, seq_params.color_range);
        return;
    }
    if seq_params.color_primaries == AOM_CICP_CP_BT_709
        && seq_params.transfer_characteristics == AOM_CICP_TC_SRGB
        && seq_params.matrix_coefficients == AOM_CICP_MC_IDENTITY
    {
        debug_assert!(seq_params.subsampling_x == 0 && seq_params.subsampling_y == 0);
        debug_assert!(
            seq_params.profile == PROFILE_1
                || (seq_params.profile == PROFILE_2 && seq_params.bit_depth == AOM_BITS_12)
        );
    } else {
        // 0: [16, 235] (i.e. xvYCC), 1: [0, 255]
        aom_wb_write_bit(wb, seq_params.color_range);
        if seq_params.profile == PROFILE_0 {
            // 420 only
            debug_assert!(seq_params.subsampling_x == 1 && seq_params.subsampling_y == 1);
        } else if seq_params.profile == PROFILE_1 {
            // 444 only
            debug_assert!(seq_params.subsampling_x == 0 && seq_params.subsampling_y == 0);
        } else if seq_params.profile == PROFILE_2 {
            if seq_params.bit_depth == AOM_BITS_12 {
                // 444, 422 or 420
                aom_wb_write_bit(wb, seq_params.subsampling_x);
                if seq_params.subsampling_x == 0 {
                    debug_assert!(
                        seq_params.subsampling_y == 0,
                        "4:4:0 subsampling not allowed in AV1"
                    );
                } else {
                    aom_wb_write_bit(wb, seq_params.subsampling_y);
                }
            } else {
                // 422 only
                debug_assert!(seq_params.subsampling_x == 1 && seq_params.subsampling_y == 0);
            }
        }
        if seq_params.matrix_coefficients == AOM_CICP_MC_IDENTITY {
            debug_assert!(seq_params.subsampling_x == 0 && seq_params.subsampling_y == 0);
        }
        if seq_params.subsampling_x == 1 && seq_params.subsampling_y == 1 {
            aom_wb_write_literal(wb, seq_params.chroma_sample_position, 2);
        }
    }
    aom_wb_write_bit(wb, seq_params.separate_uv_delta_q as i32);
}

#[inline]
fn write_sb_size(seq_params: &SequenceHeader, wb: &mut AomWriteBitBuffer) {
    debug_assert!(seq_params.mib_size == MI_SIZE_WIDE[seq_params.sb_size as usize] as i32);
    debug_assert!(seq_params.mib_size == 1 << seq_params.mib_size_log2);
    debug_assert!(seq_params.sb_size == BLOCK_128X128 || seq_params.sb_size == BLOCK_64X64);
    aom_wb_write_bit(
        wb,
        if seq_params.sb_size == BLOCK_128X128 {
            1
        } else {
            0
        },
    );
}

fn write_sequence_header(
    seq_params: &SequenceHeader,
    wb: &mut AomWriteBitBuffer,
) -> Result<(), ()> {
    aom_wb_write_literal(wb, seq_params.num_bits_width - 1, 4);
    aom_wb_write_literal(wb, seq_params.num_bits_height - 1, 4);
    aom_wb_write_literal(
        wb,
        seq_params.max_frame_width - 1,
        seq_params.num_bits_width,
    );
    aom_wb_write_literal(
        wb,
        seq_params.max_frame_height - 1,
        seq_params.num_bits_height,
    );

    if seq_params.reduced_still_picture_hdr == 0 {
        aom_wb_write_bit(wb, seq_params.frame_id_numbers_present_flag as i32);
        if seq_params.frame_id_numbers_present_flag != 0 {
            // (frame ids are off for the saver)
            return Err(());
        }
    }

    write_sb_size(seq_params, wb);

    aom_wb_write_bit(wb, seq_params.enable_filter_intra as i32);
    aom_wb_write_bit(wb, seq_params.enable_intra_edge_filter as i32);

    if seq_params.reduced_still_picture_hdr == 0 {
        aom_wb_write_bit(wb, seq_params.enable_interintra_compound as i32);
        aom_wb_write_bit(wb, seq_params.enable_masked_compound as i32);
        aom_wb_write_bit(wb, seq_params.enable_warped_motion as i32);
        aom_wb_write_bit(wb, seq_params.enable_dual_filter as i32);

        aom_wb_write_bit(wb, seq_params.order_hint_info.enable_order_hint);

        if seq_params.order_hint_info.enable_order_hint != 0 {
            aom_wb_write_bit(wb, seq_params.order_hint_info.enable_dist_wtd_comp);
            aom_wb_write_bit(wb, seq_params.order_hint_info.enable_ref_frame_mvs);
        }
        if seq_params.force_screen_content_tools == 2 {
            aom_wb_write_bit(wb, 1);
        } else {
            aom_wb_write_bit(wb, 0);
            aom_wb_write_bit(wb, seq_params.force_screen_content_tools as i32);
        }
        if seq_params.force_screen_content_tools > 0 {
            if seq_params.force_integer_mv == 2 {
                aom_wb_write_bit(wb, 1);
            } else {
                aom_wb_write_bit(wb, 0);
                aom_wb_write_bit(wb, seq_params.force_integer_mv as i32);
            }
        } else {
            debug_assert!(seq_params.force_integer_mv == 2);
        }
        if seq_params.order_hint_info.enable_order_hint != 0 {
            aom_wb_write_literal(wb, seq_params.order_hint_info.order_hint_bits_minus_1, 3);
        }
    }

    aom_wb_write_bit(wb, seq_params.enable_superres as i32);
    aom_wb_write_bit(wb, seq_params.enable_cdef as i32);
    aom_wb_write_bit(wb, seq_params.enable_restoration as i32);
    Ok(())
}

fn write_global_motion_params(
    params: &WarpedMotionParams,
    _ref_params: &WarpedMotionParams,
    wb: &mut AomWriteBitBuffer,
    _allow_hp: bool,
) -> Result<(), ()> {
    let type_ = params.wmtype;

    // As a workaround for an AV1 spec bug, we avoid choosing TRANSLATION
    // type models. Check here that we don't accidentally pick one somehow.
    // See comments in gm_get_motion_vector() for details on the bug we're
    // working around here
    debug_assert!(type_ != TRANSLATION);

    aom_wb_write_bit(wb, (type_ != IDENTITY) as i32);
    if type_ != IDENTITY {
        // (the global motion search is off for the saver's speed: the
        // models stay the identity)
        return Err(());
    }
    Ok(())
}

fn write_global_motion(cpi: &Av1Comp, wb: &mut AomWriteBitBuffer) -> Result<(), ()> {
    let cm = &cpi.common;
    for frame in LAST_FRAME..=ALTREF_FRAME {
        let ref_params = match cm.prev_frame {
            Some(prev) => &cm.buffer_pool.frame_bufs[prev].global_motion[frame as usize],
            None => &DEFAULT_WARP_PARAMS,
        };
        write_global_motion_params(
            &cm.global_motion[frame as usize],
            ref_params,
            wb,
            cm.features.allow_high_precision_mv,
        )?;
        // TODO(sarahparker, debargha): The logic in the commented out code below
        // does not work currently and causes mismatches when resize is on.
        // Fix it before turning the optimization back on.
        // (the commented out code)
    }
    Ok(())
}

// New function based on HLS R18
fn write_uncompressed_header_obu(
    cpi: &mut Av1Comp,
    saved_wb: &mut SavedWb,
    wb: &mut AomWriteBitBuffer,
) -> Result<(), ()> {
    let enable_ref_short_signaling = cpi.sf.rt_sf.enable_ref_short_signaling;
    let cm = &mut cpi.common;
    let seq_params = &cm.seq_params;

    if !enable_ref_short_signaling
        || seq_params.order_hint_info.enable_order_hint == 0
        || seq_params.order_hint_info.enable_ref_frame_mvs != 0
    {
        cm.current_frame.frame_refs_short_signaling = 0;
    } else {
        cm.current_frame.frame_refs_short_signaling = 1;
    }

    let cm = &cpi.common;
    let seq_params = &cm.seq_params;
    let quant_params = &cm.quant_params;
    let current_frame = &cm.current_frame;
    let features = &cm.features;

    if seq_params.still_picture != 0 {
        debug_assert!(cm.show_existing_frame == 0);
        debug_assert!(cm.show_frame == 1);
        debug_assert!(current_frame.frame_type == KEY_FRAME);
    }
    if seq_params.reduced_still_picture_hdr == 0 {
        if encode_show_existing_frame(cm) {
            // (the saver never shows an existing frame)
            return Err(());
        } else {
            aom_wb_write_bit(wb, 0); // show_existing_frame
        }

        aom_wb_write_literal(wb, current_frame.frame_type as i32, 2);

        aom_wb_write_bit(wb, cm.show_frame);
        if cm.show_frame != 0 {
            if seq_params.decoder_model_info_present_flag != 0
                && seq_params.timing_info.equal_picture_interval == 0
            {
                // (no decoder model info)
                return Err(());
            }
        } else {
            // (the saver's frames are all shown)
            return Err(());
        }
        if frame_is_sframe(cm) {
            debug_assert!(features.error_resilient_mode);
        } else if !(current_frame.frame_type == KEY_FRAME && cm.show_frame != 0) {
            aom_wb_write_bit(wb, features.error_resilient_mode as i32);
        }
    }
    aom_wb_write_bit(wb, features.disable_cdf_update as i32);

    if seq_params.force_screen_content_tools == 2 {
        aom_wb_write_bit(wb, features.allow_screen_content_tools as i32);
    } else {
        debug_assert!(
            features.allow_screen_content_tools as u8 == seq_params.force_screen_content_tools
        );
    }

    if features.allow_screen_content_tools {
        // (screen content tools are off)
        return Err(());
    } else {
        debug_assert!(!features.cur_frame_force_integer_mv);
    }

    let mut frame_size_override_flag = false;

    if seq_params.reduced_still_picture_hdr != 0 {
        debug_assert!(
            cm.superres_upscaled_width == seq_params.max_frame_width
                && cm.superres_upscaled_height == seq_params.max_frame_height
        );
    } else {
        if seq_params.frame_id_numbers_present_flag != 0 {
            // (frame ids are off for the saver)
            return Err(());
        }

        if cm.superres_upscaled_width > seq_params.max_frame_width
            || cm.superres_upscaled_height > seq_params.max_frame_height
        {
            // "Frame dimensions are larger than the maximum values"
            // (AOM_CODEC_UNSUP_BITSTREAM)
            return Err(());
        }

        frame_size_override_flag = if frame_is_sframe(cm) {
            true
        } else {
            cm.superres_upscaled_width != seq_params.max_frame_width
                || cm.superres_upscaled_height != seq_params.max_frame_height
        };
        if !frame_is_sframe(cm) {
            aom_wb_write_bit(wb, frame_size_override_flag as i32);
        }

        if seq_params.order_hint_info.enable_order_hint != 0 {
            aom_wb_write_literal(
                wb,
                current_frame.order_hint as i32,
                seq_params.order_hint_info.order_hint_bits_minus_1 + 1,
            );
        }

        if !features.error_resilient_mode && !frame_is_intra_only(cm) {
            aom_wb_write_literal(wb, features.primary_ref_frame, PRIMARY_REF_BITS);
        }
    }

    if seq_params.decoder_model_info_present_flag != 0 {
        // (no decoder model info)
        return Err(());
    }

    // Shown keyframes and switch-frames automatically refreshes all reference
    // frames.  For all other frame types, we need to write refresh_frame_flags.
    if (current_frame.frame_type == KEY_FRAME && cm.show_frame == 0)
        || current_frame.frame_type == INTER_FRAME
        || current_frame.frame_type == INTRA_ONLY_FRAME
    {
        aom_wb_write_literal(wb, current_frame.refresh_frame_flags, REF_FRAMES as i32);
    }

    if !frame_is_intra_only(cm) || current_frame.refresh_frame_flags != 0xff {
        // Write all ref frame order hints if error_resilient_mode == 1
        if features.error_resilient_mode && seq_params.order_hint_info.enable_order_hint != 0 {
            // (the saver's frames are not error resilient)
            return Err(());
        }
    }

    if current_frame.frame_type == KEY_FRAME {
        write_frame_size(cm, frame_size_override_flag, wb)?;
        debug_assert!(!av1_superres_scaled(cm) || !features.allow_intrabc);
        if features.allow_screen_content_tools && !av1_superres_scaled(cm) {
            aom_wb_write_bit(wb, features.allow_intrabc as i32);
        }
    } else if current_frame.frame_type == INTRA_ONLY_FRAME {
        // (the saver's intra frames are key frames)
        return Err(());
    } else if current_frame.frame_type == INTER_FRAME || frame_is_sframe(cm) {
        // NOTE: Error resilient mode turns off frame_refs_short_signaling
        //       automatically.
        // (FRAME_REFS_SHORT_SIGNALING is 0)

        if current_frame.frame_refs_short_signaling != 0 {
            // (enable_ref_short_signaling is off for the saver's speed)
            return Err(());
        }

        if seq_params.order_hint_info.enable_order_hint != 0 {
            aom_wb_write_bit(wb, current_frame.frame_refs_short_signaling);
        }

        for ref_frame in LAST_FRAME..=ALTREF_FRAME {
            debug_assert!(get_ref_frame_map_idx(cm, ref_frame) != INVALID_IDX);
            if current_frame.frame_refs_short_signaling == 0 {
                aom_wb_write_literal(wb, get_ref_frame_map_idx(cm, ref_frame), REF_FRAMES_LOG2);
            }
            if seq_params.frame_id_numbers_present_flag != 0 {
                // (frame ids are off for the saver)
                return Err(());
            }
        }

        if !features.error_resilient_mode && frame_size_override_flag {
            // (write_frame_size_with_refs(): the saver's frames all have the
            // sequence's size)
            return Err(());
        } else {
            write_frame_size(cm, frame_size_override_flag, wb)?;
        }

        if !features.cur_frame_force_integer_mv {
            aom_wb_write_bit(wb, features.allow_high_precision_mv as i32);
        }
        write_frame_interp_filter(features.interp_filter, wb);
        aom_wb_write_bit(wb, features.switchable_motion_mode as i32);
        if frame_might_allow_ref_frame_mvs(cm) {
            aom_wb_write_bit(wb, features.allow_ref_frame_mvs as i32);
        } else {
            debug_assert!(!features.allow_ref_frame_mvs);
        }
    }

    let might_bwd_adapt = seq_params.reduced_still_picture_hdr == 0 && !features.disable_cdf_update;
    if cm.tiles.large_scale != 0 {
        debug_assert!(features.refresh_frame_context == REFRESH_FRAME_CONTEXT_DISABLED);
    }

    if might_bwd_adapt {
        aom_wb_write_bit(
            wb,
            (features.refresh_frame_context == REFRESH_FRAME_CONTEXT_DISABLED) as i32,
        );
    }

    write_tile_info(cm, saved_wb, wb)?;
    encode_quantization(
        quant_params,
        av1_num_planes(cm),
        cm.seq_params.separate_uv_delta_q != 0,
        wb,
    );
    encode_segmentation(cm, wb)?;

    let delta_q_info = &cm.delta_q_info;
    if delta_q_info.delta_q_present_flag != 0 {
        debug_assert!(quant_params.base_qindex > 0);
    }
    if quant_params.base_qindex > 0 {
        aom_wb_write_bit(wb, delta_q_info.delta_q_present_flag);
        if delta_q_info.delta_q_present_flag != 0 {
            // (delta q is never present: the TPL statistics the objective
            // needs are never there)
            return Err(());
        }
    }

    if features.all_lossless {
        debug_assert!(!av1_superres_scaled(cm));
    } else {
        if !features.coded_lossless {
            encode_loopfilter(cm, wb);
            encode_cdef(cm, wb);
        }
        encode_restoration_mode(cm)?;
    }

    // Write TX mode
    if features.coded_lossless {
        debug_assert!(features.tx_mode == ONLY_4X4);
    } else {
        aom_wb_write_bit(wb, (features.tx_mode == TX_MODE_SELECT) as i32);
    }

    if !frame_is_intra_only(cm) {
        let use_hybrid_pred = current_frame.reference_mode == REFERENCE_MODE_SELECT;

        aom_wb_write_bit(wb, use_hybrid_pred as i32);
    }

    if current_frame.skip_mode_info.skip_mode_allowed != 0 {
        aom_wb_write_bit(wb, current_frame.skip_mode_info.skip_mode_flag);
    }

    if frame_might_allow_warped_motion(cm) {
        aom_wb_write_bit(wb, features.allow_warped_motion as i32);
    } else {
        debug_assert!(!features.allow_warped_motion);
    }

    aom_wb_write_bit(wb, features.reduced_tx_set_used as i32);

    if !frame_is_intra_only(cm) {
        write_global_motion(cpi, wb)?;
    }

    if seq_params.film_grain_params_present != 0 && (cm.show_frame != 0 || cm.showable_frame != 0) {
        // (no film grain)
        return Err(());
    }

    if cm.tiles.large_scale != 0 {
        // (write_ext_tile_info(): no large scale tiles)
        return Err(());
    }
    Ok(())
}

fn choose_size_bytes(mut size: u32, spare_msbs: i32) -> i32 {
    // Choose the number of bytes required to represent size, without
    // using the 'spare_msbs' number of most significant bits.

    // Make sure we will fit in 4 bytes to start with..
    if spare_msbs > 0 && size >> (32 - spare_msbs) != 0 {
        return -1;
    }

    // Normalise to 32 bits
    size <<= spare_msbs;

    if size >> 24 != 0 {
        4
    } else if size >> 16 != 0 {
        3
    } else if size >> 8 != 0 {
        2
    } else {
        1
    }
}

#[inline]
fn mem_put_varsize(dst: &mut [u8], sz: i32, val: i32) {
    match sz {
        1 => dst[0] = (val & 0xff) as u8,
        2 => dst[..2].copy_from_slice(&(val as u16).to_le_bytes()),
        3 => dst[..3].copy_from_slice(&(val as u32).to_le_bytes()[..3]),
        4 => dst[..4].copy_from_slice(&(val as u32).to_le_bytes()),
        _ => debug_assert!(false, "Invalid size"),
    }
}

/// Translation of `remux_tiles()` for tiles that are not large scale:
/// `dst` is the tile data.
fn remux_tiles(
    tiles: &CommonTileParams,
    dst: &mut [u8],
    data_size: u32,
    max_tile_size: u32,
    tile_size_bytes: &mut i32,
    tile_col_size_bytes: &mut i32,
) -> u32 {
    // Choose the tile size bytes (tsb) and tile column size bytes (tcsb)
    debug_assert!(tiles.large_scale == 0);
    let tsb = choose_size_bytes(max_tile_size, 0);
    let tcsb = 4; // This is ignored

    debug_assert!(tsb > 0);
    debug_assert!(tcsb > 0);

    *tile_size_bytes = tsb;
    *tile_col_size_bytes = tcsb;
    if tsb == 4 && tcsb == 4 {
        return data_size;
    }

    let mut wpos: u32 = 0;
    let mut rpos: u32 = 0;

    let n_tiles = tiles.cols * tiles.rows;

    for n in 0..n_tiles {
        let mut tile_size: u32;

        if n == n_tiles - 1 {
            tile_size = data_size - rpos;
        } else {
            let r = rpos as usize;
            tile_size = u32::from_le_bytes([dst[r], dst[r + 1], dst[r + 2], dst[r + 3]]);
            rpos += 4;
            mem_put_varsize(&mut dst[wpos as usize..], tsb, tile_size as i32);
            tile_size += AV1_MIN_TILE_SIZE_BYTES;
            wpos += tsb as u32;
        }

        dst.copy_within(rpos as usize..(rpos + tile_size) as usize, wpos as usize);

        rpos += tile_size;
        wpos += tile_size;
    }

    debug_assert!(rpos > wpos);
    debug_assert!(rpos == data_size);

    wpos
}

pub(crate) fn av1_write_obu_header(obu_type: i32, obu_extension: i32, dst: &mut [u8]) -> u32 {
    // (no level statistics are kept)
    let mut wb = AomWriteBitBuffer::new(dst, 0);

    aom_wb_write_literal(&mut wb, 0, 1); // forbidden bit.
    aom_wb_write_literal(&mut wb, obu_type, 4);
    aom_wb_write_literal(&mut wb, if obu_extension != 0 { 1 } else { 0 }, 1);
    aom_wb_write_literal(&mut wb, 1, 1); // obu_has_payload_length_field
    aom_wb_write_literal(&mut wb, 0, 1); // reserved

    if obu_extension != 0 {
        aom_wb_write_literal(&mut wb, obu_extension & 0xFF, 8);
    }

    aom_wb_bytes_written(&wb)
}

/// Translation of `av1_write_uleb_obu_size()`: `Err` for
/// `AOM_CODEC_ERROR`.
pub(crate) fn av1_write_uleb_obu_size(
    obu_header_size: usize,
    obu_payload_size: usize,
    dest: &mut [u8],
) -> Result<(), ()> {
    let offset = obu_header_size;
    let obu_size = obu_payload_size as u32;
    debug_assert!(obu_size as usize == obu_payload_size);

    match aom_uleb_encode(obu_size as u64, &mut dest[offset..offset + 4]) {
        Some(_) => Ok(()),
        None => Err(()),
    }
}

pub(crate) fn av1_obu_memmove(
    obu_header_size: usize,
    obu_payload_size: usize,
    data: &mut [u8],
) -> usize {
    let length_field_size = aom_uleb_size_in_bytes(obu_payload_size as u64);
    let move_dst_offset = length_field_size + obu_header_size;
    let move_src_offset = obu_header_size;
    let move_size = obu_payload_size;
    data.copy_within(
        move_src_offset..move_src_offset + move_size,
        move_dst_offset,
    );
    length_field_size
}

#[inline]
fn add_trailing_bits(wb: &mut AomWriteBitBuffer) {
    if aom_wb_is_byte_aligned(wb) {
        aom_wb_write_literal(wb, 0x80, 8);
    } else {
        // assumes that the other bits are already 0s
        aom_wb_write_bit(wb, 1);
    }
}

#[inline]
fn write_bitstream_level(seq_level_idx: Av1Level, wb: &mut AomWriteBitBuffer) {
    debug_assert!(is_valid_seq_level_idx(seq_level_idx));
    aom_wb_write_literal(wb, seq_level_idx, LEVEL_BITS);
}

pub(crate) fn av1_write_sequence_header_obu(
    seq_params: &SequenceHeader,
    dst: &mut [u8],
) -> Result<u32, ()> {
    let mut wb = AomWriteBitBuffer::new(dst, 0);

    write_profile(seq_params.profile, &mut wb);

    // Still picture or not
    aom_wb_write_bit(&mut wb, seq_params.still_picture as i32);
    debug_assert!(seq_params.still_picture != 0 || seq_params.reduced_still_picture_hdr == 0);
    // whether to use reduced still picture header
    aom_wb_write_bit(&mut wb, seq_params.reduced_still_picture_hdr as i32);

    if seq_params.reduced_still_picture_hdr != 0 {
        debug_assert!(seq_params.timing_info_present == 0);
        debug_assert!(seq_params.decoder_model_info_present_flag == 0);
        debug_assert!(seq_params.display_model_info_present_flag == 0);
        write_bitstream_level(seq_params.seq_level_idx[0], &mut wb);
    } else {
        aom_wb_write_bit(&mut wb, seq_params.timing_info_present); // timing info present flag

        if seq_params.timing_info_present != 0 {
            // (libavif sets no timing info)
            return Err(());
        }
        aom_wb_write_bit(&mut wb, seq_params.display_model_info_present_flag as i32);
        aom_wb_write_literal(
            &mut wb,
            seq_params.operating_points_cnt_minus_1,
            OP_POINTS_CNT_MINUS_1_BITS,
        );
        for i in 0..(seq_params.operating_points_cnt_minus_1 + 1) as usize {
            aom_wb_write_literal(
                &mut wb,
                seq_params.operating_point_idc[i],
                OP_POINTS_IDC_BITS,
            );
            write_bitstream_level(seq_params.seq_level_idx[i], &mut wb);
            if seq_params.seq_level_idx[i] >= SEQ_LEVEL_4_0 {
                aom_wb_write_bit(&mut wb, seq_params.tier[i] as i32);
            }
            if seq_params.decoder_model_info_present_flag != 0 {
                // (no decoder model info)
                return Err(());
            }
            if seq_params.display_model_info_present_flag != 0 {
                // (no display model info)
                return Err(());
            }
        }
    }
    write_sequence_header(seq_params, &mut wb)?;

    write_color_config(seq_params, &mut wb);

    aom_wb_write_bit(&mut wb, seq_params.film_grain_params_present as i32);

    add_trailing_bits(&mut wb);

    Ok(aom_wb_bytes_written(&wb))
}

fn write_frame_header_obu(
    cpi: &mut Av1Comp,
    saved_wb: &mut SavedWb,
    buf: &mut [u8],
    dst: usize,
    append_trailing_bits: bool,
) -> Result<u32, ()> {
    let mut wb = AomWriteBitBuffer::new(buf, dst);
    write_uncompressed_header_obu(cpi, saved_wb, &mut wb)?;
    if append_trailing_bits {
        add_trailing_bits(&mut wb);
    }
    Ok(aom_wb_bytes_written(&wb))
}

fn write_tile_group_header(
    dst: &mut [u8],
    start_tile: i32,
    end_tile: i32,
    tiles_log2: i32,
    tile_start_and_end_present_flag: bool,
) -> u32 {
    let mut wb = AomWriteBitBuffer::new(dst, 0);

    if tiles_log2 == 0 {
        return 0;
    }

    aom_wb_write_bit(&mut wb, tile_start_and_end_present_flag as i32);

    if tile_start_and_end_present_flag {
        aom_wb_write_literal(&mut wb, start_tile, tiles_log2);
        aom_wb_write_literal(&mut wb, end_tile, tiles_log2);
    }

    aom_wb_bytes_written(&wb)
}

/// Translation of `PackBSParams` (the buffer pointers are offsets into
/// the output).
struct PackBsParams {
    curr_tg_hdr_size: i32,
    is_last_tile_in_tg: bool,
    new_tg: bool,
    obu_extn_header: i32,
    obu_header_size: u32,
    tile_col: i32,
    tile_row: i32,
    tile_data_curr: usize,
    buf_data: usize,
    buf_size: u32,
}

// Writes obu, tile group and uncompressed headers to bitstream.
fn av1_write_obu_tg_tile_headers(
    cpi: &mut Av1Comp,
    dst: &mut [u8],
    saved_wb: &mut SavedWb,
    total_size: &mut u32,
    pack_bs_params: &mut PackBsParams,
    tile_idx: i32,
) -> Result<(), ()> {
    let tiles = cpi.common.tiles;
    let tg_size = (tiles.rows * tiles.cols + cpi.num_tg - 1) / cpi.num_tg;

    // Write Tile group, frame and OBU header
    // A new tile group begins at this tile.  Write the obu header and
    // tile group header
    let obu_type = if cpi.num_tg == 1 {
        OBU_FRAME
    } else {
        OBU_TILE_GROUP
    };
    let tdc = pack_bs_params.tile_data_curr;
    let mut curr_tg_hdr_size =
        av1_write_obu_header(obu_type, pack_bs_params.obu_extn_header, &mut dst[tdc..]) as i32;
    pack_bs_params.obu_header_size = curr_tg_hdr_size as u32;

    if cpi.num_tg == 1 {
        curr_tg_hdr_size +=
            write_frame_header_obu(cpi, saved_wb, dst, tdc + curr_tg_hdr_size as usize, false)?
                as i32;
    }
    curr_tg_hdr_size += write_tile_group_header(
        &mut dst[tdc + curr_tg_hdr_size as usize..],
        tile_idx,
        (tile_idx + tg_size - 1).min(tiles.cols * tiles.rows - 1),
        tiles.log2_rows + tiles.log2_cols,
        cpi.num_tg > 1,
    ) as i32;
    pack_bs_params.curr_tg_hdr_size = curr_tg_hdr_size;
    *total_size += curr_tg_hdr_size as u32;
    Ok(())
}

// Pack tile data in the bitstream with tile_group, frame
// and OBU header.
fn av1_pack_tile_info(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    dst: &mut [u8],
    total_size: &mut u32,
    pack_bs_params: &mut PackBsParams,
) -> Result<(), ()> {
    let mut mode_bc = AomWriter::default();
    let cm = &cpi.common;
    let tile_row = pack_bs_params.tile_row;
    let tile_col = pack_bs_params.tile_col;
    let mut tile_info = TileInfo::default();
    av1_tile_set_col(&mut tile_info, cm, tile_col);
    av1_tile_set_row(&mut tile_info, cm, tile_row);
    mode_bc.allow_update_cdf = 1;
    mode_bc.allow_update_cdf =
        (mode_bc.allow_update_cdf != 0 && !cm.features.disable_cdf_update) as u8;

    // (av1_reset_loop_restoration(): no loop restoration)

    pack_bs_params.buf_data = *total_size as usize;

    // The last tile of the tile group does not have a header.
    if !pack_bs_params.is_last_tile_in_tg {
        *total_size += 4;
    }

    // Pack tile data
    aom_start_encode(&mut mode_bc);
    write_modes(cpi, td, &tile_info, &mut mode_bc)?;
    let ts = *total_size as usize;
    if aom_stop_encode(&mut mode_bc, &mut dst[ts..]) < 0 {
        return Err(());
    }
    let tile_size = mode_bc.pos;
    debug_assert!(tile_size >= AV1_MIN_TILE_SIZE_BYTES);

    pack_bs_params.buf_size = tile_size;

    // Write tile size
    if !pack_bs_params.is_last_tile_in_tg {
        // size of this tile
        let d = pack_bs_params.buf_data;
        dst[d..d + 4].copy_from_slice(&(tile_size - AV1_MIN_TILE_SIZE_BYTES).to_le_bytes());
    }
    Ok(())
}

fn av1_write_last_tile_info(
    cpi: &Av1Comp,
    dst: &mut [u8],
    saved_wb: &mut SavedWb,
    curr_tg_data_size: &mut usize,
    curr_tg_start: usize,
    total_size: &mut u32,
    tile_data_start: &mut usize,
    is_first_tg: &mut bool,
    obu_header_size: u32,
) -> Result<(), ()> {
    // write current tile group size
    let obu_payload_size = *curr_tg_data_size as u32 - obu_header_size;
    let length_field_size = av1_obu_memmove(
        obu_header_size as usize,
        obu_payload_size as usize,
        &mut dst[curr_tg_start..],
    );
    av1_write_uleb_obu_size(
        obu_header_size as usize,
        obu_payload_size as usize,
        &mut dst[curr_tg_start..],
    )?;
    *curr_tg_data_size += length_field_size;
    *total_size += length_field_size as u32;
    *tile_data_start += length_field_size;
    if cpi.num_tg == 1 {
        // if this tg is combined with the frame header then update saved
        // frame header base offset according to length field size
        saved_wb.bit_buffer += length_field_size;
    }

    // (error resilient mode is off: no duplicate frame header)
    debug_assert!(*is_first_tg || !cpi.common.features.error_resilient_mode);
    *is_first_tg = false;
    Ok(())
}

pub(crate) fn av1_reset_pack_bs_thread_data(td: &mut ThreadData) {
    td.coefficient_size = 0;
    td.max_mv_magnitude = 0;
    td.interp_filter_selected = [0; SWITCHABLE as usize];
}

pub(crate) fn av1_accumulate_pack_bs_thread_data(cpi: &mut Av1Comp, td: &ThreadData) {
    let mut do_max_mv_magnitude_update = true;
    cpi.rc.coefficient_size += td.coefficient_size;

    // Disable max_mv_magnitude update for parallel frames based on update flag.
    if !cpi.do_frame_data_update {
        do_max_mv_magnitude_update = false;
    }

    if cpi.sf.mv_sf.auto_mv_step_size != 0 && do_max_mv_magnitude_update {
        cpi.mv_search_params.max_mv_magnitude = cpi
            .mv_search_params
            .max_mv_magnitude
            .max(td.max_mv_magnitude);
    }

    for filter in EIGHTTAP_REGULAR..SWITCHABLE {
        cpi.common.cur_frame_mut().interp_filter_selected[filter as usize] +=
            td.interp_filter_selected[filter as usize];
    }
}

// Store information related to each default tile in the OBU header.
fn write_tile_obu(
    cpi: &mut Av1Comp,
    td: &mut ThreadData,
    dst: &mut [u8],
    total_size: &mut u32,
    saved_wb: &mut SavedWb,
    obu_extn_header: i32,
    largest_tile_id: &mut i32,
    max_tile_size: &mut u32,
    obu_header_size: &mut u32,
    tile_data_start: &mut usize,
) -> Result<(), ()> {
    let tiles = cpi.common.tiles;
    let tile_cols = tiles.cols;
    let tile_rows = tiles.rows;
    // Fixed size tile groups for the moment
    let num_tg_hdrs = cpi.num_tg;
    let tg_size = (tile_rows * tile_cols + num_tg_hdrs - 1) / num_tg_hdrs;
    let mut tile_count = 0;
    let mut curr_tg_data_size: usize = 0;
    let mut tile_data_curr: usize = 0;
    let mut new_tg = true;
    let mut is_first_tg = true;

    av1_reset_pack_bs_thread_data(td);
    for tile_row in 0..tile_rows {
        for tile_col in 0..tile_cols {
            let tile_idx = tile_row * tile_cols + tile_col;

            let mut is_last_tile_in_tg = false;
            if new_tg {
                tile_data_curr = *total_size as usize;
                tile_count = 0;
            }
            tile_count += 1;

            if tile_count == tg_size || tile_idx == (tile_cols * tile_rows - 1) {
                is_last_tile_in_tg = true;
            }

            // (xd->tile_ctx = &this_tile->tctx: moved in while the tile is
            // packed)
            std::mem::swap(
                &mut td.mb.e_mbd.tile_ctx,
                &mut cpi.tile_data[tile_idx as usize].tctx,
            );

            // PackBSParams stores all parameters required to pack tile and header
            // info.
            let mut pack_bs_params = PackBsParams {
                curr_tg_hdr_size: 0,
                is_last_tile_in_tg,
                new_tg,
                obu_extn_header,
                obu_header_size: 0,
                tile_col,
                tile_row,
                tile_data_curr,
                buf_data: 0,
                buf_size: 0,
            };

            let hdr = if new_tg {
                av1_write_obu_tg_tile_headers(
                    cpi,
                    dst,
                    saved_wb,
                    total_size,
                    &mut pack_bs_params,
                    tile_idx,
                )
            } else {
                Ok(())
            };

            let res = hdr
                .and_then(|()| av1_pack_tile_info(cpi, td, dst, total_size, &mut pack_bs_params));
            std::mem::swap(
                &mut td.mb.e_mbd.tile_ctx,
                &mut cpi.tile_data[tile_idx as usize].tctx,
            );
            res?;

            if new_tg {
                curr_tg_data_size = pack_bs_params.curr_tg_hdr_size as usize;
                *tile_data_start += pack_bs_params.curr_tg_hdr_size as usize;
                *obu_header_size = pack_bs_params.obu_header_size;
                new_tg = false;
            }
            if is_last_tile_in_tg {
                new_tg = true;
            }

            curr_tg_data_size +=
                (pack_bs_params.buf_size + if is_last_tile_in_tg { 0 } else { 4 }) as usize;

            if pack_bs_params.buf_size > *max_tile_size {
                *largest_tile_id = tile_idx;
                *max_tile_size = pack_bs_params.buf_size;
            }

            if is_last_tile_in_tg {
                av1_write_last_tile_info(
                    cpi,
                    dst,
                    saved_wb,
                    &mut curr_tg_data_size,
                    tile_data_curr,
                    total_size,
                    tile_data_start,
                    &mut is_first_tg,
                    *obu_header_size,
                )?;
            }
            *total_size += pack_bs_params.buf_size;
        }
    }
    av1_accumulate_pack_bs_thread_data(cpi, td);
    Ok(())
}

// Write total buffer size and related information into the OBU header for
// default tile case.
fn write_tile_obu_size(
    cpi: &Av1Comp,
    dst: &mut [u8],
    saved_wb: &SavedWb,
    largest_tile_id: i32,
    total_size: &mut u32,
    max_tile_size: u32,
    obu_header_size: u32,
    tile_data_start: usize,
) -> Result<(), ()> {
    let tiles = &cpi.common.tiles;

    // Fill in context_update_tile_id indicating the tile to use for the
    // cdf update. The encoder currently sets it to the largest tile
    // (but is up to the encoder)
    let mut wb = AomWriteBitBuffer::from_saved(dst, *saved_wb);
    aom_wb_overwrite_literal(&mut wb, largest_tile_id, tiles.log2_cols + tiles.log2_rows);
    let saved_after_id = wb.save();
    // If more than one tile group. tile_size_bytes takes the default value 4
    // and does not need to be set. For a single tile group it is set in the
    // section below.
    if cpi.num_tg != 1 {
        return Ok(());
    }
    let mut tile_size_bytes = 4;
    let mut unused = 0;
    let tile_data_offset = tile_data_start as u32;
    let tile_data_size = *total_size - tile_data_offset;

    *total_size = remux_tiles(
        tiles,
        &mut dst[tile_data_start..],
        tile_data_size,
        max_tile_size,
        &mut tile_size_bytes,
        &mut unused,
    );
    *total_size += tile_data_offset;
    debug_assert!((1..=4).contains(&tile_size_bytes));

    let mut wb = AomWriteBitBuffer::from_saved(dst, saved_after_id);
    aom_wb_overwrite_literal(&mut wb, tile_size_bytes - 1, 2);

    // Update the OBU length if remux_tiles() reduced the size.
    let ohs = obu_header_size as usize;
    let (payload_size, length_field_size) =
        aom_uleb_decode(&dst[ohs..*total_size as usize]).ok_or(())?;

    let new_payload_size = *total_size as u64 - obu_header_size as u64 - length_field_size as u64;
    if new_payload_size != payload_size {
        let new_length_field_size =
            aom_uleb_encode(new_payload_size, &mut dst[ohs..ohs + length_field_size]).ok_or(())?;
        if new_length_field_size < length_field_size {
            let src_offset = ohs + length_field_size;
            let dst_offset = ohs + new_length_field_size;
            dst.copy_within(src_offset..*total_size as usize, dst_offset);
            *total_size -= (length_field_size - new_length_field_size) as u32;
        }
    }
    Ok(())
}

// (calc_pack_bs_mt_workers(): one thread)

fn pack_tiles_in_tg_obus(
    cpi: &mut Av1Comp,
    td: &mut ThreadData,
    dst: &mut [u8],
    saved_wb: &mut SavedWb,
    obu_extension_header: i32,
    largest_tile_id: &mut i32,
) -> Result<u32, ()> {
    let tiles = cpi.common.tiles;
    let mut total_size: u32 = 0;
    let mut max_tile_size: u32 = 0;
    let mut obu_header_size: u32 = 0;
    let mut tile_data_start: usize = 0;
    let tile_cols = tiles.cols;
    let tile_rows = tiles.rows;
    let num_tiles = tile_rows * tile_cols;

    // (calc_pack_bs_mt_workers(): one worker)
    debug_assert!(!cpi.mt_info.pack_bs_mt_enabled);
    write_tile_obu(
        cpi,
        td,
        dst,
        &mut total_size,
        saved_wb,
        obu_extension_header,
        largest_tile_id,
        &mut max_tile_size,
        &mut obu_header_size,
        &mut tile_data_start,
    )?;

    if num_tiles > 1 {
        write_tile_obu_size(
            cpi,
            dst,
            saved_wb,
            *largest_tile_id,
            &mut total_size,
            max_tile_size,
            obu_header_size,
            tile_data_start,
        )?;
    }
    Ok(total_size)
}

fn write_tiles_in_tg_obus(
    cpi: &mut Av1Comp,
    td: &mut ThreadData,
    dst: &mut [u8],
    saved_wb: &mut SavedWb,
    obu_extension_header: i32,
    largest_tile_id: &mut i32,
) -> Result<u32, ()> {
    *largest_tile_id = 0;

    // Select the coding strategy (temporal or spatial)
    // (segmentation is off; no large scale tiles)
    debug_assert!(cpi.common.seg.enabled == 0);
    debug_assert!(cpi.common.tiles.large_scale == 0);
    pack_tiles_in_tg_obus(
        cpi,
        td,
        dst,
        saved_wb,
        obu_extension_header,
        largest_tile_id,
    )
}

// (av1_write_metadata_array(): libavif attaches no metadata to the images
// it hands aom)

/// Translation of `av1_pack_bitstream()`: writes the frame's OBUs to the
/// start of `dst`, returning their size. The thread data and the frame
/// state are moved as for the frame's encoding (see encodeframe.rs).
pub(crate) fn av1_pack_bitstream(
    cpi: &mut Av1Comp,
    dst: &mut [u8],
    largest_tile_id: &mut i32,
) -> Result<usize, ()> {
    let mut td = std::mem::take(&mut cpi.td);
    enc_move_frame_state_in(cpi, &mut td);
    let res = av1_pack_bitstream_td(cpi, &mut td, dst, largest_tile_id);
    enc_move_frame_state_out(cpi, &mut td);
    cpi.td = td;
    res
}

fn av1_pack_bitstream_td(
    cpi: &mut Av1Comp,
    td: &mut ThreadData,
    dst: &mut [u8],
    largest_tile_id: &mut i32,
) -> Result<usize, ()> {
    let mut data: usize = 0;
    let obu_extension_header =
        (cpi.common.temporal_layer_id << 5 | cpi.common.spatial_layer_id << 3) as i32;

    // If no non-zero delta_q has been used, reset delta_q_present_flag
    debug_assert!(cpi.common.delta_q_info.delta_q_present_flag == 0);
    cpi.frame_header_count = 0;

    // The TD is now written outside the frame encode loop

    // write sequence header obu at each key frame or intra_only frame,
    // preceded by 4-byte size
    if cpi.common.current_frame.frame_type == INTRA_ONLY_FRAME
        || cpi.common.current_frame.frame_type == KEY_FRAME
    {
        let obu_header_size =
            av1_write_obu_header(OBU_SEQUENCE_HEADER, 0, &mut dst[data..]) as usize;
        let obu_payload_size = av1_write_sequence_header_obu(
            &cpi.common.seq_params,
            &mut dst[data + obu_header_size..],
        )? as usize;
        let length_field_size =
            av1_obu_memmove(obu_header_size, obu_payload_size, &mut dst[data..]);
        av1_write_uleb_obu_size(obu_header_size, obu_payload_size, &mut dst[data..])?;

        data += obu_header_size + obu_payload_size + length_field_size;
    }

    // write metadata obus before the frame obu that has the show_frame flag set
    // (no metadata)

    let write_frame_header = cpi.num_tg > 1 || encode_show_existing_frame(&cpi.common);
    // (one tile group, no shown existing frame: the frame header goes into
    // the frame OBU)
    debug_assert!(!write_frame_header);
    let mut saved_wb = SavedWb::default();
    let data_size = write_tiles_in_tg_obus(
        cpi,
        td,
        &mut dst[data..],
        &mut saved_wb,
        obu_extension_header,
        largest_tile_id,
    )?;
    data += data_size as usize;
    Ok(data)
}
