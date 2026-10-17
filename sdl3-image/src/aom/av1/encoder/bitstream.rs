// Rust translation of the parts of av1/encoder/bitstream.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the packing of a still picture's key frame (reduced still picture
// header), with one thread and one tile group.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The bitstream writer: the sequence header OBU, then the frame OBU (the
//! uncompressed frame header and the tiles' entropy-coded modes and
//! coefficients). The paths of inter frames, of headers that are not
//! reduced still picture headers, of segmentation, palettes, intra block
//! copy, delta q and loop restoration are left out (the saver's frames use
//! none of them).

use super::encoder::*;
use super::encodetxb::{av1_get_cb_coeff_buffer, av1_write_intra_coeffs_mb};
use crate::aom::aom_dsp::aom_dsp_common::ceil_power_of_two;
use crate::aom::aom_dsp::bitwriter::*;
use crate::aom::aom_dsp::bitwriter_buffer::*;
use crate::aom::aom_integer::{aom_uleb_decode, aom_uleb_encode, aom_uleb_size_in_bytes};
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cdef::CDEF_STRENGTH_BITS;
use crate::aom::av1::common::cfl::is_cfl_allowed;
use crate::aom::av1::common::entropymode::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::pred_common::{av1_get_skip_txfm_context, get_tx_size_context};
use crate::aom::av1::common::quant_common::QINDEX_BITS;
use crate::aom::av1::common::reconintra::{
    av1_allow_intrabc, av1_filter_intra_allowed, av1_is_directional_mode,
};
use crate::aom::av1::common::restoration::av1_loop_restoration_corners_in_sb;
use crate::aom::av1::common::seg_common::*;
use crate::aom::av1::common::tile_common::*;
use crate::aom::av1::encoder::block::{tx_size_to_depth, Macroblock};

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
        // (intra blocks only)
        debug_assert!(is_inter == 0);
        // (no filter intra: the intra direction is the mode)
        debug_assert!(mbmi.filter_intra_mode_info.use_filter_intra == 0);
        let intra_dir = mbmi.mode;
        aom_write_symbol(
            w,
            AV1_EXT_TX_IND[tx_set_type as usize][tx_type as usize],
            &mut xd.tile_ctx.intra_ext_tx_cdf[eset as usize][square_tx_size][intra_dir as usize],
            AV1_NUM_EXT_TX_SET[tx_set_type as usize],
        );
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
    // (the saver's frames are key frames)
    debug_assert!(is_keyframe);
    {
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

fn write_mbmi_b(cpi: &Av1Comp, td: &mut ThreadData, w: &mut AomWriter) {
    let cm = &cpi.common;
    let xd = &mut td.mb.e_mbd;

    // (inter frames: pack_inter_mode_mvs())
    debug_assert!(frame_is_intra_only(cm));
    write_mb_modes_kf(cpi, xd, w);
}

fn write_tokens_b(cpi: &Av1Comp, x: &mut Macroblock, w: &mut AomWriter) {
    let cm = &cpi.common;
    let mbmi = *x.e_mbd.mbmi();
    let bsize = mbmi.bsize;

    debug_assert!(mbmi.skip_txfm == 0);

    // (intra blocks only)
    debug_assert!(!is_inter_block(&mbmi));
    av1_write_intra_coeffs_mb(cm, x, w, bsize);
}

fn write_modes_b(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile: &TileInfo,
    w: &mut AomWriter,
    mi_row: i32,
    mi_col: i32,
) {
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

    write_mbmi_b(cpi, td, w);

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
        // (intra: the inter transform partition is not written)
        debug_assert!(!is_inter_tx);
        write_selected_tx_size(xd, w);
        let (wd, ht) = (xd.width as i32, xd.height as i32);
        set_txfm_ctxs(mbmi.tx_size, wd, ht, false, xd);
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
) {
    let cm = &cpi.common;
    let mi_params = &cm.mi_params;
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let hbs = MI_SIZE_WIDE[bsize as usize] as i32 / 2;
    let quarter_step = MI_SIZE_WIDE[bsize as usize] as i32 / 4;
    let partition = get_partition(mi_params, &td.mb.e_mbd, mi_row, mi_col, bsize);
    let subsize = get_partition_subsize(bsize, partition);

    if mi_row >= mi_params.mi_rows || mi_col >= mi_params.mi_cols {
        return;
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
        PARTITION_NONE => write_modes_b(cpi, td, tile, w, mi_row, mi_col),
        PARTITION_HORZ => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col);
            if mi_row + hbs < mi_params.mi_rows {
                write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col);
            }
        }
        PARTITION_VERT => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col);
            if mi_col + hbs < mi_params.mi_cols {
                write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs);
            }
        }
        PARTITION_SPLIT => {
            write_modes_sb(cpi, td, tile, w, mi_row, mi_col, subsize);
            write_modes_sb(cpi, td, tile, w, mi_row, mi_col + hbs, subsize);
            write_modes_sb(cpi, td, tile, w, mi_row + hbs, mi_col, subsize);
            write_modes_sb(cpi, td, tile, w, mi_row + hbs, mi_col + hbs, subsize);
        }
        PARTITION_HORZ_A => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col);
            write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs);
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col);
        }
        PARTITION_HORZ_B => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col);
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col);
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col + hbs);
        }
        PARTITION_VERT_A => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col);
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col);
            write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs);
        }
        PARTITION_VERT_B => {
            write_modes_b(cpi, td, tile, w, mi_row, mi_col);
            write_modes_b(cpi, td, tile, w, mi_row, mi_col + hbs);
            write_modes_b(cpi, td, tile, w, mi_row + hbs, mi_col + hbs);
        }
        PARTITION_HORZ_4 => {
            for i in 0..4 {
                let this_mi_row = mi_row + i * quarter_step;
                if i > 0 && this_mi_row >= mi_params.mi_rows {
                    break;
                }

                write_modes_b(cpi, td, tile, w, this_mi_row, mi_col);
            }
        }
        PARTITION_VERT_4 => {
            for i in 0..4 {
                let this_mi_col = mi_col + i * quarter_step;
                if i > 0 && this_mi_col >= mi_params.mi_cols {
                    break;
                }

                write_modes_b(cpi, td, tile, w, mi_row, this_mi_col);
            }
        }
        _ => debug_assert!(false),
    }

    // update partition context
    update_ext_partition_context(&mut td.mb.e_mbd, mi_row, mi_col, subsize, bsize, partition);
}

fn write_modes(cpi: &Av1Comp, td: &mut ThreadData, tile: &TileInfo, w: &mut AomWriter) {
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
            write_modes_sb(cpi, td, tile, w, mi_row, mi_col, cm.seq_params.sb_size);
            mi_col += cm.seq_params.mib_size;
        }
        mi_row += cm.seq_params.mib_size;
    }
}

fn encode_restoration_mode(cm: &Av1Common) {
    debug_assert!(!cm.features.all_lossless);
    // (loop restoration is disabled in the sequence)
    debug_assert!(cm.seq_params.enable_restoration == 0);
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

fn encode_segmentation(cm: &Av1Common, wb: &mut AomWriteBitBuffer) {
    let seg = &cm.seg;

    aom_wb_write_bit(wb, seg.enabled as i32);
    // (segmentation is off)
    debug_assert!(seg.enabled == 0);
}

fn write_tile_info_max_tile(cm: &Av1Common, wb: &mut AomWriteBitBuffer) {
    let tiles = &cm.tiles;

    aom_wb_write_bit(wb, tiles.uniform_spacing as i32);

    // (the saver's tiles are uniformly spaced)
    debug_assert!(tiles.uniform_spacing != 0);
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
}

fn write_tile_info(cm: &Av1Common, saved_wb: &mut SavedWb, wb: &mut AomWriteBitBuffer) {
    write_tile_info_max_tile(cm, wb);

    *saved_wb = wb.save();
    if cm.tiles.rows * cm.tiles.cols > 1 {
        // tile id used for cdf update
        aom_wb_write_literal(wb, 0, cm.tiles.log2_cols + cm.tiles.log2_rows);
        // Number of bytes in tile size - 1
        aom_wb_write_literal(wb, 3, 2);
    }
}

fn write_render_size(cm: &Av1Common, wb: &mut AomWriteBitBuffer) {
    let scaling_active = av1_resize_scaled(cm);
    aom_wb_write_bit(wb, scaling_active as i32);
    if scaling_active {
        aom_wb_write_literal(wb, cm.render_width - 1, 16);
        aom_wb_write_literal(wb, cm.render_height - 1, 16);
    }
}

fn write_superres_scale(cm: &Av1Common) {
    let seq_params = &cm.seq_params;
    // (superres is disabled in the sequence)
    debug_assert!(seq_params.enable_superres == 0);
    debug_assert!(cm.superres_scale_denominator as i32 == SCALE_NUMERATOR);
}

fn write_frame_size(cm: &Av1Common, frame_size_override: bool, wb: &mut AomWriteBitBuffer) {
    // (no frame size override in a reduced still picture header)
    debug_assert!(!frame_size_override);

    write_superres_scale(cm);
    write_render_size(cm, wb);
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

fn write_sequence_header(seq_params: &SequenceHeader, wb: &mut AomWriteBitBuffer) {
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

    // (the saver writes reduced still picture headers)
    debug_assert!(seq_params.reduced_still_picture_hdr != 0);

    write_sb_size(seq_params, wb);

    aom_wb_write_bit(wb, seq_params.enable_filter_intra as i32);
    aom_wb_write_bit(wb, seq_params.enable_intra_edge_filter as i32);

    aom_wb_write_bit(wb, seq_params.enable_superres as i32);
    aom_wb_write_bit(wb, seq_params.enable_cdef as i32);
    aom_wb_write_bit(wb, seq_params.enable_restoration as i32);
}

// New function based on HLS R18
fn write_uncompressed_header_obu(
    cpi: &mut Av1Comp,
    saved_wb: &mut SavedWb,
    wb: &mut AomWriteBitBuffer,
) {
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
    // (the saver writes reduced still picture headers: no show existing
    // frame, frame type, show frame or error resilient flags)
    debug_assert!(seq_params.reduced_still_picture_hdr != 0);
    aom_wb_write_bit(wb, features.disable_cdf_update as i32);

    if seq_params.force_screen_content_tools == 2 {
        aom_wb_write_bit(wb, features.allow_screen_content_tools as i32);
    } else {
        debug_assert!(
            features.allow_screen_content_tools as u8 == seq_params.force_screen_content_tools
        );
    }

    // (screen content tools are off)
    debug_assert!(!features.allow_screen_content_tools);
    debug_assert!(!features.cur_frame_force_integer_mv);

    let frame_size_override_flag = false;

    debug_assert!(
        cm.superres_upscaled_width == seq_params.max_frame_width
            && cm.superres_upscaled_height == seq_params.max_frame_height
    );

    // (no decoder model info; a shown key frame refreshes all the
    // references: no refresh flags or order hints)
    debug_assert!(seq_params.decoder_model_info_present_flag == 0);
    debug_assert!(current_frame.frame_type == KEY_FRAME && cm.show_frame != 0);
    debug_assert!(current_frame.refresh_frame_flags == 0xff);

    write_frame_size(cm, frame_size_override_flag, wb);
    debug_assert!(!av1_superres_scaled(cm) || !features.allow_intrabc);

    let might_bwd_adapt = seq_params.reduced_still_picture_hdr == 0 && !features.disable_cdf_update;
    debug_assert!(!might_bwd_adapt);

    write_tile_info(cm, saved_wb, wb);
    encode_quantization(
        quant_params,
        av1_num_planes(cm),
        cm.seq_params.separate_uv_delta_q != 0,
        wb,
    );
    encode_segmentation(cm, wb);

    let delta_q_info = &cm.delta_q_info;
    if delta_q_info.delta_q_present_flag != 0 {
        debug_assert!(quant_params.base_qindex > 0);
    }
    if quant_params.base_qindex > 0 {
        aom_wb_write_bit(wb, delta_q_info.delta_q_present_flag);
        // (delta q is never present)
        debug_assert!(delta_q_info.delta_q_present_flag == 0);
    }

    if features.all_lossless {
        debug_assert!(!av1_superres_scaled(cm));
    } else {
        if !features.coded_lossless {
            encode_loopfilter(cm, wb);
            encode_cdef(cm, wb);
        }
        encode_restoration_mode(cm);
    }

    // Write TX mode
    if features.coded_lossless {
        debug_assert!(features.tx_mode == ONLY_4X4);
    } else {
        aom_wb_write_bit(wb, (features.tx_mode == TX_MODE_SELECT) as i32);
    }

    // (an intra frame: no reference mode)
    debug_assert!(frame_is_intra_only(cm));

    if current_frame.skip_mode_info.skip_mode_allowed != 0 {
        aom_wb_write_bit(wb, current_frame.skip_mode_info.skip_mode_flag);
    }

    if frame_might_allow_warped_motion(cm) {
        aom_wb_write_bit(wb, features.allow_warped_motion as i32);
    } else {
        debug_assert!(!features.allow_warped_motion);
    }

    aom_wb_write_bit(wb, features.reduced_tx_set_used as i32);

    // (no global motion in an intra frame, no film grain, no large scale
    // tiles)
    debug_assert!(seq_params.film_grain_params_present == 0);
    debug_assert!(cm.tiles.large_scale == 0);
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

pub(crate) fn av1_write_sequence_header_obu(seq_params: &SequenceHeader, dst: &mut [u8]) -> u32 {
    let mut wb = AomWriteBitBuffer::new(dst, 0);

    write_profile(seq_params.profile, &mut wb);

    // Still picture or not
    aom_wb_write_bit(&mut wb, seq_params.still_picture as i32);
    debug_assert!(seq_params.still_picture != 0 || seq_params.reduced_still_picture_hdr == 0);
    // whether to use reduced still picture header
    aom_wb_write_bit(&mut wb, seq_params.reduced_still_picture_hdr as i32);

    // (the saver writes reduced still picture headers)
    debug_assert!(seq_params.reduced_still_picture_hdr != 0);
    debug_assert!(seq_params.timing_info_present == 0);
    debug_assert!(seq_params.decoder_model_info_present_flag == 0);
    debug_assert!(seq_params.display_model_info_present_flag == 0);
    write_bitstream_level(seq_params.seq_level_idx[0], &mut wb);

    write_sequence_header(seq_params, &mut wb);

    write_color_config(seq_params, &mut wb);

    aom_wb_write_bit(&mut wb, seq_params.film_grain_params_present as i32);

    add_trailing_bits(&mut wb);

    aom_wb_bytes_written(&wb)
}

fn write_frame_header_obu(
    cpi: &mut Av1Comp,
    saved_wb: &mut SavedWb,
    buf: &mut [u8],
    dst: usize,
    append_trailing_bits: bool,
) -> u32 {
    let mut wb = AomWriteBitBuffer::new(buf, dst);
    write_uncompressed_header_obu(cpi, saved_wb, &mut wb);
    if append_trailing_bits {
        add_trailing_bits(&mut wb);
    }
    aom_wb_bytes_written(&wb)
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
) {
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
            write_frame_header_obu(cpi, saved_wb, dst, tdc + curr_tg_hdr_size as usize, false)
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
    write_modes(cpi, td, &tile_info, &mut mode_bc);
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

            if new_tg {
                av1_write_obu_tg_tile_headers(
                    cpi,
                    dst,
                    saved_wb,
                    total_size,
                    &mut pack_bs_params,
                    tile_idx,
                );
            }

            let res = av1_pack_tile_info(cpi, td, dst, total_size, &mut pack_bs_params);
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
        ) as usize;
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
