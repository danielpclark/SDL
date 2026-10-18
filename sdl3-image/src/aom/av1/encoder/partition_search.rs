// Rust translation of the parts of av1/encoder/partition_search.c and
// av1/encoder/partition_search.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the non-RD coding of a superblock along a given partition.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Coding of a superblock along the partition the variance-based partition
//! picked: mode search and encoding of each coding block in turn.

use super::block::*;
use super::context_tree::{av1_alloc_pmc, PickModeContext};
use super::encodeframe::{av1_get_perpixel_variance_facade, av1_setup_src_planes};
use super::encodeframe_utils::*;
use super::encodemb::{av1_encode_intra_block_plane, av1_encode_sb};
use super::encodemv::{av1_get_ref_mv, av1_update_mv_stats};
use super::encoder::*;
use super::encodetxb::av1_update_intra_mb_txb_context;
use super::mcomp::av1_set_mv_limits;
use super::nonrd_pickmode::{
    av1_copy_mbmi_ext_to_mbmi_ext_frame, av1_nonrd_pick_inter_mode_sb, av1_nonrd_pick_intra_mode,
};
use super::rd::av1_setup_pred_block;
use super::rd::{av1_invalid_rd_stats, av1_set_error_per_bit};
use super::rdopt::av1_rd_pick_intra_mode_sb;
use super::rdopt_utils::{set_mode_eval_params, set_tx_size_search_method};
use super::reconinter_enc::av1_enc_build_inter_predictor;
use super::tokenize::av1_tokenize_sb_vartx;
use super::var_based_part::calc_chroma_thresh_for_zeromv_skip;
use crate::aom::aom_dsp::prob::update_cdf;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::av1_loopfilter::av1_get_txb_size_index;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cfl::cfl_store_block;
use crate::aom::av1::common::cfl::{is_cfl_allowed, store_cfl_required};
use crate::aom::av1::common::entropymode::FrameContext;
use crate::aom::av1::common::entropymv::MV_SUBPEL_NONE;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mv::Mv;
use crate::aom::av1::common::mvref_common::{
    av1_collect_neighbors_ref_counts, av1_drl_ctx, av1_mode_context_analyzer, av1_ref_frame_type,
};
use crate::aom::av1::common::pred_common::*;
use crate::aom::av1::common::reconinter::{
    av1_is_wedge_used, av1_setup_dst_planes, av1_setup_pre_planes, is_any_masked_compound_used,
    is_interinter_compound_used, is_nontrans_global_motion,
};
use crate::aom::av1::common::reconintra::av1_allow_intrabc;
use crate::aom::av1::common::seg_common::*;
use crate::aom::av1::common::tile_common::TileInfo;

// (partition_search.h)

#[inline]
fn set_cb_offsets(
    cb_offset: &mut [u16; PLANE_TYPES as usize],
    cb_offset_y: u16,
    cb_offset_uv: u16,
) {
    cb_offset[PLANE_TYPE_Y as usize] = cb_offset_y;
    cb_offset[PLANE_TYPE_UV as usize] = cb_offset_uv;
}

#[inline]
fn update_cb_offsets(x: &mut Macroblock, bsize: BlockSize, subsampling_x: i32, subsampling_y: i32) {
    x.cb_offset[PLANE_TYPE_Y as usize] +=
        (BLOCK_SIZE_WIDE[bsize as usize] as i32 * BLOCK_SIZE_HIGH[bsize as usize] as i32) as u16;
    if x.e_mbd.is_chroma_ref {
        let plane_bsize = get_plane_block_size(bsize, subsampling_x, subsampling_y);
        debug_assert!(plane_bsize != BLOCK_INVALID);
        x.cb_offset[PLANE_TYPE_UV as usize] += (BLOCK_SIZE_WIDE[plane_bsize as usize] as i32
            * BLOCK_SIZE_HIGH[plane_bsize as usize] as i32)
            as u16;
    }
}

// (partition_search.c)

fn update_txfm_count(
    x: &mut Macroblock,
    tx_size: TxSize,
    depth: i32,
    blk_row: i32,
    blk_col: i32,
    allow_update_cdf: bool,
) {
    let xd = &mut x.e_mbd;
    let mbmi = *xd.mbmi();
    let bsize = mbmi.bsize;
    let max_blocks_high = max_block_high(xd, bsize, 0);
    let max_blocks_wide = max_block_wide(xd, bsize, 0);
    let ctx = txfm_partition_context(
        xd.above_contexts.txfm[xd.above_contexts_tile_row]
            [xd.above_txfm_context + blk_col as usize],
        xd.left_txfm_context_buffer[xd.left_txfm_context + blk_row as usize],
        mbmi.bsize,
        tx_size,
    ) as usize;
    let txb_size_index = av1_get_txb_size_index(bsize, blk_row, blk_col) as usize;
    let plane_tx_size = mbmi.inter_tx_size[txb_size_index];

    if blk_row >= max_blocks_high || blk_col >= max_blocks_wide {
        return;
    }
    debug_assert!(tx_size > TX_4X4);

    let above = xd.above_txfm_context + blk_col as usize;
    let left = xd.left_txfm_context + blk_row as usize;
    let tile_row = xd.above_contexts_tile_row;
    if depth == MAX_VARTX_DEPTH {
        // Don't add to counts in this case
        xd.mbmi_mut().tx_size = tx_size;
        let MacroBlockD {
            above_contexts,
            left_txfm_context_buffer,
            ..
        } = xd;
        txfm_partition_update(
            &mut above_contexts.txfm[tile_row][above..],
            &mut left_txfm_context_buffer[left..],
            tx_size,
            tx_size,
        );
        return;
    }

    if tx_size == plane_tx_size {
        if allow_update_cdf {
            update_cdf(&mut xd.tile_ctx.txfm_partition_cdf[ctx], 0, 2);
        }
        xd.mbmi_mut().tx_size = tx_size;
        let MacroBlockD {
            above_contexts,
            left_txfm_context_buffer,
            ..
        } = xd;
        txfm_partition_update(
            &mut above_contexts.txfm[tile_row][above..],
            &mut left_txfm_context_buffer[left..],
            tx_size,
            tx_size,
        );
    } else {
        let sub_txs = SUB_TX_SIZE_MAP[tx_size as usize];
        let bsw = TX_SIZE_WIDE_UNIT[sub_txs as usize] as i32;
        let bsh = TX_SIZE_HIGH_UNIT[sub_txs as usize] as i32;

        if allow_update_cdf {
            update_cdf(&mut xd.tile_ctx.txfm_partition_cdf[ctx], 1, 2);
        }
        x.txfm_search_info.txb_split_count += 1;

        if sub_txs == TX_4X4 {
            let xd = &mut x.e_mbd;
            {
                let mbmi = xd.mbmi_mut();
                mbmi.inter_tx_size[txb_size_index] = TX_4X4;
                mbmi.tx_size = TX_4X4;
            }
            let MacroBlockD {
                above_contexts,
                left_txfm_context_buffer,
                ..
            } = xd;
            txfm_partition_update(
                &mut above_contexts.txfm[tile_row][above..],
                &mut left_txfm_context_buffer[left..],
                TX_4X4,
                tx_size,
            );
            return;
        }

        let mut row = 0;
        while row < TX_SIZE_HIGH_UNIT[tx_size as usize] as i32 {
            let mut col = 0;
            while col < TX_SIZE_WIDE_UNIT[tx_size as usize] as i32 {
                let offsetr = row;
                let offsetc = col;

                update_txfm_count(
                    x,
                    sub_txs,
                    depth + 1,
                    blk_row + offsetr,
                    blk_col + offsetc,
                    allow_update_cdf,
                );
                col += bsw;
            }
            row += bsh;
        }
    }
}

fn tx_partition_count_update(x: &mut Macroblock, plane_bsize: BlockSize, allow_update_cdf: bool) {
    let xd = &mut x.e_mbd;
    let mi_width = MI_SIZE_WIDE[plane_bsize as usize] as i32;
    let mi_height = MI_SIZE_HIGH[plane_bsize as usize] as i32;
    let max_tx_size = get_vartx_max_txsize(xd, plane_bsize, 0);
    let bh = TX_SIZE_HIGH_UNIT[max_tx_size as usize] as i32;
    let bw = TX_SIZE_WIDE_UNIT[max_tx_size as usize] as i32;

    xd.above_txfm_context = xd.mi_col as usize;
    xd.left_txfm_context = (xd.mi_row & MAX_MIB_MASK) as usize;

    let mut idy = 0;
    while idy < mi_height {
        let mut idx = 0;
        while idx < mi_width {
            update_txfm_count(x, max_tx_size, 0, idy, idx, allow_update_cdf);
            idx += bw;
        }
        idy += bh;
    }
}

fn set_txfm_context(xd: &mut MacroBlockD, tx_size: TxSize, blk_row: i32, blk_col: i32) {
    let mbmi = *xd.mbmi();
    let bsize = mbmi.bsize;
    let max_blocks_high = max_block_high(xd, bsize, 0);
    let max_blocks_wide = max_block_wide(xd, bsize, 0);
    let txb_size_index = av1_get_txb_size_index(bsize, blk_row, blk_col) as usize;
    let plane_tx_size = mbmi.inter_tx_size[txb_size_index];

    if blk_row >= max_blocks_high || blk_col >= max_blocks_wide {
        return;
    }

    let above = xd.above_txfm_context + blk_col as usize;
    let left = xd.left_txfm_context + blk_row as usize;
    let tile_row = xd.above_contexts_tile_row;
    if tx_size == plane_tx_size {
        xd.mbmi_mut().tx_size = tx_size;
        let MacroBlockD {
            above_contexts,
            left_txfm_context_buffer,
            ..
        } = xd;
        txfm_partition_update(
            &mut above_contexts.txfm[tile_row][above..],
            &mut left_txfm_context_buffer[left..],
            tx_size,
            tx_size,
        );
    } else {
        if tx_size == TX_8X8 {
            {
                let mbmi = xd.mbmi_mut();
                mbmi.inter_tx_size[txb_size_index] = TX_4X4;
                mbmi.tx_size = TX_4X4;
            }
            let MacroBlockD {
                above_contexts,
                left_txfm_context_buffer,
                ..
            } = xd;
            txfm_partition_update(
                &mut above_contexts.txfm[tile_row][above..],
                &mut left_txfm_context_buffer[left..],
                TX_4X4,
                tx_size,
            );
            return;
        }
        let sub_txs = SUB_TX_SIZE_MAP[tx_size as usize];
        let bsw = TX_SIZE_WIDE_UNIT[sub_txs as usize] as i32;
        let bsh = TX_SIZE_HIGH_UNIT[sub_txs as usize] as i32;
        let mut row = 0;
        while row < TX_SIZE_HIGH_UNIT[tx_size as usize] as i32 {
            let mut col = 0;
            while col < TX_SIZE_WIDE_UNIT[tx_size as usize] as i32 {
                let offsetr = blk_row + row;
                let offsetc = blk_col + col;
                if offsetr >= max_blocks_high || offsetc >= max_blocks_wide {
                    col += bsw;
                    continue;
                }
                set_txfm_context(xd, sub_txs, offsetr, offsetc);
                col += bsw;
            }
            row += bsh;
        }
    }
}

fn tx_partition_set_contexts(xd: &mut MacroBlockD, plane_bsize: BlockSize) {
    let mi_width = MI_SIZE_WIDE[plane_bsize as usize] as i32;
    let mi_height = MI_SIZE_HIGH[plane_bsize as usize] as i32;
    let max_tx_size = get_vartx_max_txsize(xd, plane_bsize, 0);
    let bh = TX_SIZE_HIGH_UNIT[max_tx_size as usize] as i32;
    let bw = TX_SIZE_WIDE_UNIT[max_tx_size as usize] as i32;

    xd.above_txfm_context = xd.mi_col as usize;
    xd.left_txfm_context = (xd.mi_row & MAX_MIB_MASK) as usize;

    let mut idy = 0;
    while idy < mi_height {
        let mut idx = 0;
        while idx < mi_width {
            set_txfm_context(xd, max_tx_size, idy, idx);
            idx += bw;
        }
        idy += bh;
    }
}

fn encode_superblock(
    cpi: &Av1Comp,
    tile_data: &TileDataEnc,
    td: &mut ThreadData,
    dry_run: i32,
    bsize: BlockSize,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);
    let x = &mut td.mb;
    let mbmi = *x.e_mbd.mbmi();
    let seg_skip = segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_SKIP);
    let mis = cm.mi_params.mi_stride;
    let mi_width = MI_SIZE_WIDE[bsize as usize] as i32;
    let mi_height = MI_SIZE_HIGH[bsize as usize] as i32;
    let is_inter = is_inter_block(&mbmi);

    // Initialize tx_mode and tx_size_search_method
    set_tx_size_search_method(
        cm,
        &cpi.winner_mode_params,
        &mut x.txfm_search_params,
        cpi.sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch,
        1,
    );

    let mi_row = x.e_mbd.mi_row;
    let mi_col = x.e_mbd.mi_col;
    if is_inter {
        let is_compound = has_second_ref(&mbmi);
        let xd = &mut x.e_mbd;
        set_ref_ptrs(cm, xd, mbmi.ref_frame[0], mbmi.ref_frame[1])?;
        for r in 0..1 + is_compound as usize {
            let cfg = get_ref_frame_yv12_buf(cm, mbmi.ref_frame[r]);
            debug_assert!(is_intrabc_block(&mbmi) || cfg.is_some());
            let sf = xd.block_ref_scale_factors[r];
            av1_setup_pre_planes(xd, r, cfg, mi_row, mi_col, Some(&sf), num_planes);
        }
        // Predicted sample of inter mode (for Luma plane) cannot be reused if
        // nonrd_check_partition_split speed feature is enabled, Since in such cases
        // the buffer may not contain the predicted sample of best mode.
        let start_plane = if x.reuse_inter_pred
            && cpi.sf.rt_sf.nonrd_check_partition_split == 0
            && cm.seq_params.bit_depth == AOM_BITS_8
        {
            1
        } else {
            0
        };
        av1_enc_build_inter_predictor(
            cm,
            &mut x.e_mbd,
            mi_row,
            mi_col,
            None,
            bsize,
            start_plane,
            av1_num_planes(cm) - 1,
        )?;
        if x.e_mbd.mbmi().motion_mode == OBMC_CAUSAL {
            debug_assert!(cpi.oxcf.motion_mode_cfg.enable_obmc);
            // (av1_build_obmc_inter_predictors_sb(): OBMC is never picked by
            // the non-RD mode search)
            return Err(());
        }

        av1_encode_sb(cpi, x, bsize, dry_run)?;
        av1_tokenize_sb_vartx(cpi, td, dry_run, bsize, tile_data.allow_update_cdf)?;
    } else {
        x.e_mbd.cfl.store_y = store_cfl_required(&cm.seq_params, &x.e_mbd);
        x.e_mbd.mbmi_mut().skip_txfm = 1;
        for plane in 0..num_planes {
            let segment_id = x.e_mbd.mbmi().segment_id as usize;
            with_source(x, |x, src| {
                av1_encode_intra_block_plane(
                    cpi,
                    x,
                    src,
                    bsize,
                    plane,
                    dry_run,
                    cpi.optimize_seg_arr[segment_id],
                )
            })?;
        }

        // If there is at least one lossless segment, force the skip for intra
        // block to be 0, in order to avoid the segment_id to be changed by in
        // write_segment_id().
        if cpi.common.seg.segid_preskip == 0
            && cpi.common.seg.update_map != 0
            && cpi.enc_seg.has_lossless_segment != 0
        {
            x.e_mbd.mbmi_mut().skip_txfm = 0;
        }

        x.e_mbd.cfl.store_y = 0;
        // (palettes: screen content tools are off)
        debug_assert!(!av1_allow_palette(
            cm.features.allow_screen_content_tools,
            bsize
        ));

        av1_update_intra_mb_txb_context(cpi, td, dry_run, bsize, tile_data.allow_update_cdf as u8)?;
    }

    let x = &mut td.mb;
    let txfm_params = x.txfm_search_params;
    let mbmi = *x.e_mbd.mbmi();
    if dry_run == 0 {
        if av1_allow_intrabc(cm) && is_intrabc_block(&mbmi) {
            td.intrabc_used = 1;
        }
        let x = &mut td.mb;
        let xd = &mut x.e_mbd;
        if txfm_params.tx_mode_search_type == TX_MODE_SELECT
            && xd.lossless[mbmi.segment_id as usize] == 0
            && mbmi.bsize > BLOCK_4X4
            && !(is_inter && (mbmi.skip_txfm != 0 || seg_skip))
        {
            if is_inter {
                tx_partition_count_update(x, bsize, tile_data.allow_update_cdf);
            } else {
                if mbmi.tx_size != MAX_TXSIZE_RECT_LOOKUP[bsize as usize] {
                    x.txfm_search_info.txb_split_count += 1;
                }
                if block_signals_txsize(bsize) {
                    let tx_size_ctx = get_tx_size_context(xd) as usize;
                    let tx_size_cat = bsize_to_tx_size_cat(bsize) as usize;
                    let depth = tx_size_to_depth(mbmi.tx_size, bsize);
                    let max_depths = bsize_to_max_depth(bsize);

                    if tile_data.allow_update_cdf {
                        update_cdf(
                            &mut xd.tile_ctx.tx_size_cdf[tx_size_cat][tx_size_ctx],
                            depth as i8,
                            max_depths + 1,
                        );
                    }
                }
            }
        } else {
            // The new intra coding scheme requires no change of transform size
            let intra_tx_size = if is_inter {
                if xd.lossless[mbmi.segment_id as usize] != 0 {
                    TX_4X4
                } else {
                    tx_size_from_tx_mode(bsize, txfm_params.tx_mode_search_type)
                }
            } else {
                mbmi.tx_size
            };

            let cols = (cm.mi_params.mi_cols - mi_col).min(mi_width);
            let rows = (cm.mi_params.mi_rows - mi_row).min(mi_height);
            for j in 0..rows {
                for i in 0..cols {
                    let idx = xd.mi_grid_base[(xd.mi as i32 + mis * j + i) as usize] as usize;
                    xd.mi_alloc[idx].tx_size = intra_tx_size;
                }
            }

            if intra_tx_size != MAX_TXSIZE_RECT_LOOKUP[bsize as usize] {
                x.txfm_search_info.txb_split_count += 1;
            }
        }
    }

    let x = &mut td.mb;
    let xd = &mut x.e_mbd;
    let mbmi = *xd.mbmi();
    if txfm_params.tx_mode_search_type == TX_MODE_SELECT
        && block_signals_txsize(mbmi.bsize)
        && is_inter
        && !(mbmi.skip_txfm != 0 || seg_skip)
        && xd.lossless[mbmi.segment_id as usize] == 0
    {
        if dry_run != 0 {
            tx_partition_set_contexts(xd, bsize);
        }
    } else {
        let mut tx_size = mbmi.tx_size;
        // The new intra coding scheme requires no change of transform size
        if is_inter {
            if xd.lossless[mbmi.segment_id as usize] != 0 {
                tx_size = TX_4X4;
            } else {
                tx_size = tx_size_from_tx_mode(bsize, txfm_params.tx_mode_search_type);
            }
        } else {
            tx_size = if bsize > BLOCK_4X4 { tx_size } else { TX_4X4 };
        }
        xd.mbmi_mut().tx_size = tx_size;
        let (w, h) = (xd.width as i32, xd.height as i32);
        let skip = (mbmi.skip_txfm != 0 || seg_skip) && is_inter_block(&mbmi);
        set_txfm_ctxs(tx_size, w, h, skip, xd);
    }

    if is_inter_block(xd.mbmi()) && !xd.is_chroma_ref && is_cfl_allowed(xd) != 0 {
        let (b, t) = (xd.mbmi().bsize, xd.mbmi().tx_size);
        cfl_store_block(xd, b, t)?;
    }
    if dry_run == 0
        && cpi.oxcf.pass == AOM_RC_ONE_PASS
        && cpi.svc.temporal_layer_id == 0
        && cpi.sf.rt_sf.use_temporal_noise_estimate != 0
        && !cpi.ppi.use_svc
    {
        let (mi_row, mi_col) = (x.e_mbd.mi_row, x.e_mbd.mi_col);
        update_zeromv_cnt(cpi, &mbmi, mi_row, mi_col, bsize);
    }
    Ok(())
}

/// Translation of `update_zeromv_cnt()`.
fn update_zeromv_cnt(cpi: &Av1Comp, mi: &MbModeInfo, mi_row: i32, mi_col: i32, bsize: BlockSize) {
    if mi.ref_frame[0] != LAST_FRAME || !is_inter_block(mi) || mi.segment_id > 2
    /* CR_SEGMENT_ID_BOOST2 */
    {
        return;
    }
    let cm = &cpi.common;
    let mv = mi.mv[0].as_mv();
    let bw = MI_SIZE_WIDE[bsize as usize] as i32 >> 1;
    let bh = MI_SIZE_HIGH[bsize as usize] as i32 >> 1;
    let xmis = ((cm.mi_params.mi_cols - mi_col) >> 1).min(bw);
    let ymis = ((cm.mi_params.mi_rows - mi_row) >> 1).min(bh);
    let block_index = (mi_row >> 1) * (cm.mi_params.mi_cols >> 1) + (mi_col >> 1);
    for y in 0..ymis {
        for x in 0..xmis {
            // consec_zero_mv is in the scale of 8x8 blocks
            let map_offset = (block_index + y * (cm.mi_params.mi_cols >> 1) + x) as usize;
            let Some(c) = cpi.consec_zero_mv.get(map_offset) else {
                continue;
            };
            if (mv.row as i32).abs() < 10 && (mv.col as i32).abs() < 10 {
                if c.get() < 255 {
                    c.set(c.get() + 1);
                }
            } else {
                c.set(0);
            }
        }
    }
}

fn setup_block_rdmult(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    aq_mode: i32,
) {
    x.rdmult = cpi.rd.rdmult;

    // (adaptive quantization is off for the saver)
    debug_assert!(aq_mode == NO_AQ);

    let cm = &cpi.common;
    // (av1_get_cb_rdmult(): the RD pick mode only)
    debug_assert!(
        !(cm.delta_q_info.delta_q_present_flag != 0 && cpi.sf.rt_sf.use_nonrd_pick_mode == 0)
    );

    if cpi.oxcf.tune_cfg.tuning == AOM_TUNE_SSIM {
        let Macroblock {
            errorperbit,
            rdmult,
            ..
        } = x;
        av1_set_ssim_rdmult(cpi, errorperbit, bsize, mi_row, mi_col, rdmult);
    }
    if cpi.oxcf.mode == ALLINTRA {
        x.rdmult = ((x.rdmult as i64 * x.intra_sb_rdmult_modifier as i64) >> 7) as i32;
    }

    // Check to make sure that the adjustments above have not caused the
    // rd multiplier to be truncated to 0.
    x.rdmult = if x.rdmult > 0 { x.rdmult } else { 1 };
}

pub(crate) fn av1_set_offsets_without_segment_id(
    cpi: &Av1Comp,
    tile: &TileInfo,
    x: &mut Macroblock,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let mi_width = MI_SIZE_WIDE[bsize as usize] as i32;
    let mi_height = MI_SIZE_HIGH[bsize as usize] as i32;

    set_mode_info_offsets(&cpi.common.mi_params, &cpi.mbmi_ext_info, x, mi_row, mi_col);

    let xd = &mut x.e_mbd;
    set_entropy_context(xd, mi_row, mi_col, num_planes);
    xd.above_txfm_context = mi_col as usize;
    xd.left_txfm_context = (mi_row & MAX_MIB_MASK) as usize;
    xd.above_contexts_tile_row = tile.tile_row as usize;

    // Set up destination pointers.
    {
        let MacroBlockD { plane, recon, .. } = xd;
        av1_setup_dst_planes(plane, bsize, recon, mi_row, mi_col, 0, num_planes);
    }

    // Set up limit values for MV components.
    // Mv beyond the range do not produce new/different prediction block.
    av1_set_mv_limits(
        &cm.mi_params,
        &mut x.mv_limits,
        mi_row,
        mi_col,
        mi_height,
        mi_width,
        cpi.oxcf.border_in_pixels,
    );

    let xd = &mut x.e_mbd;
    set_plane_n4(xd, mi_width, mi_height, num_planes);

    // Set up distance of MB to edge of frame in 1/8th pel units.
    debug_assert!((mi_col & (mi_width - 1)) == 0 && (mi_row & (mi_height - 1)) == 0);
    set_mi_row_col(
        xd,
        tile,
        mi_row,
        mi_height,
        mi_col,
        mi_width,
        cm.mi_params.mi_rows,
        cm.mi_params.mi_cols,
    );

    // Set up source buffers.
    av1_setup_src_planes(x, cpi.source(), mi_row, mi_col, num_planes, bsize);

    // required by av1_append_sub8x8_mvs_for_idx() and av1_find_best_ref_mvs()
    x.e_mbd.tile = *tile;
}

pub(crate) fn av1_set_offsets(
    cpi: &Av1Comp,
    tile: &TileInfo,
    x: &mut Macroblock,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) {
    let cm = &cpi.common;
    let seg = &cm.seg;

    av1_set_offsets_without_segment_id(cpi, tile, x, mi_row, mi_col, bsize);

    // Setup segment ID.
    let mbmi = x.e_mbd.mbmi_mut();
    mbmi.segment_id = 0;
    // (segmentation is off for the saver)
    debug_assert!(seg.enabled == 0);
}

fn hybrid_intra_mode_search(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rd_cost: &mut RdStats,
    bsize: BlockSize,
    ctx: &mut PickModeContext,
) -> Result<(), ()> {
    let mut use_rdopt = false;
    let hybrid_intra_pickmode = cpi.sf.rt_sf.hybrid_intra_pickmode;
    // Use rd pick for intra mode search based on block size and variance.
    if hybrid_intra_pickmode != 0 && bsize < BLOCK_16X16 {
        let var_thresh: [u32; 3] = [0, 101, 201];
        debug_assert!(hybrid_intra_pickmode <= 3);
        if x.source_variance >= var_thresh[(hybrid_intra_pickmode - 1) as usize] {
            use_rdopt = true;
        }
    }

    if use_rdopt {
        av1_rd_pick_intra_mode_sb(cpi, x, rd_cost, bsize, ctx, i64::MAX)
    } else {
        with_source(x, |x, src| {
            av1_nonrd_pick_intra_mode(cpi, x, src, rd_cost, bsize, ctx)
        })
    }
}

// (wait_for_top_right_sb(): the rows are coded one after the other, so the
// row synchronization never waits)

/// `INTER_COMPOUND_OFFSET()`
#[inline]
fn inter_compound_offset(mode: PredictionMode) -> i8 {
    (mode - NEAREST_NEARESTMV) as i8
}

fn update_stats(cm: &Av1Common, td: &mut ThreadData) -> Result<(), ()> {
    let x = &mut td.mb;
    let xd = &mut x.e_mbd;
    let mbmi = *xd.mbmi();
    let current_frame = &cm.current_frame;
    let bsize = mbmi.bsize;
    let seg_ref_active = segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_REF_FRAME);

    if current_frame.skip_mode_info.skip_mode_flag != 0
        && !seg_ref_active
        && is_comp_ref_allowed(bsize)
    {
        let skip_mode_ctx = av1_get_skip_mode_context(xd) as usize;
        update_cdf(
            &mut xd.tile_ctx.skip_mode_cdfs[skip_mode_ctx],
            mbmi.skip_mode as i8,
            2,
        );
    }

    if mbmi.skip_mode == 0 && !seg_ref_active {
        let skip_ctx = av1_get_skip_txfm_context(xd) as usize;
        update_cdf(
            &mut xd.tile_ctx.skip_txfm_cdfs[skip_ctx],
            mbmi.skip_txfm as i8,
            2,
        );
    }

    if !is_inter_block(&mbmi) {
        av1_sum_intra_stats(cm, xd, frame_is_intra_only(cm));
    }

    if av1_allow_intrabc(cm) {
        // (intra block copy is off: allow_intrabc stays 0 with screen
        // content tools off)
        return Err(());
    }

    if frame_is_intra_only(cm) || mbmi.skip_mode != 0 {
        return Ok(());
    }

    let inter_block = is_inter_block(&mbmi);

    if !seg_ref_active {
        let ctx = av1_get_intra_inter_context(xd) as usize;
        update_cdf(&mut xd.tile_ctx.intra_inter_cdf[ctx], inter_block as i8, 2);
        // If the segment reference feature is enabled we have only a single
        // reference frame allowed for the segment so exclude it from
        // the reference frame counts used to work out probabilities.
        if inter_block {
            let ref0 = mbmi.ref_frame[0];
            let ref1 = mbmi.ref_frame[1];
            if current_frame.reference_mode == REFERENCE_MODE_SELECT && is_comp_ref_allowed(bsize) {
                let ctx = av1_get_reference_mode_context(xd) as usize;
                update_cdf(
                    &mut xd.tile_ctx.comp_inter_cdf[ctx],
                    has_second_ref(&mbmi) as i8,
                    2,
                );
            }

            if has_second_ref(&mbmi) {
                let comp_ref_type = if has_uni_comp_refs(&mbmi) {
                    UNIDIR_COMP_REFERENCE
                } else {
                    BIDIR_COMP_REFERENCE
                };
                let ctx = av1_get_comp_reference_type_context(xd) as usize;
                update_cdf(
                    &mut xd.tile_ctx.comp_ref_type_cdf[ctx],
                    comp_ref_type as i8,
                    COMP_REFERENCE_TYPES,
                );

                if comp_ref_type == UNIDIR_COMP_REFERENCE {
                    let bit = (ref0 == BWDREF_FRAME) as i8;
                    let ctx = av1_get_pred_context_uni_comp_ref_p(xd) as usize;
                    update_cdf(&mut xd.tile_ctx.uni_comp_ref_cdf[ctx][0], bit, 2);
                    if bit == 0 {
                        let bit1 = (ref1 == LAST3_FRAME || ref1 == GOLDEN_FRAME) as i8;
                        let ctx = av1_get_pred_context_uni_comp_ref_p1(xd) as usize;
                        update_cdf(&mut xd.tile_ctx.uni_comp_ref_cdf[ctx][1], bit1, 2);
                        if bit1 != 0 {
                            let ctx = av1_get_pred_context_uni_comp_ref_p2(xd) as usize;
                            update_cdf(
                                &mut xd.tile_ctx.uni_comp_ref_cdf[ctx][2],
                                (ref1 == GOLDEN_FRAME) as i8,
                                2,
                            );
                        }
                    }
                } else {
                    let bit = (ref0 == GOLDEN_FRAME || ref0 == LAST3_FRAME) as i8;
                    let ctx = av1_get_pred_context_comp_ref_p(xd) as usize;
                    update_cdf(&mut xd.tile_ctx.comp_ref_cdf[ctx][0], bit, 2);
                    if bit == 0 {
                        let ctx = av1_get_pred_context_comp_ref_p1(xd) as usize;
                        update_cdf(
                            &mut xd.tile_ctx.comp_ref_cdf[ctx][1],
                            (ref0 == LAST2_FRAME) as i8,
                            2,
                        );
                    } else {
                        let ctx = av1_get_pred_context_comp_ref_p2(xd) as usize;
                        update_cdf(
                            &mut xd.tile_ctx.comp_ref_cdf[ctx][2],
                            (ref0 == GOLDEN_FRAME) as i8,
                            2,
                        );
                    }
                    let ctx = av1_get_pred_context_comp_bwdref_p(xd) as usize;
                    update_cdf(
                        &mut xd.tile_ctx.comp_bwdref_cdf[ctx][0],
                        (ref1 == ALTREF_FRAME) as i8,
                        2,
                    );
                    if ref1 != ALTREF_FRAME {
                        let ctx = av1_get_pred_context_comp_bwdref_p1(xd) as usize;
                        update_cdf(
                            &mut xd.tile_ctx.comp_bwdref_cdf[ctx][1],
                            (ref1 == ALTREF2_FRAME) as i8,
                            2,
                        );
                    }
                }
            } else {
                let bit = (ref0 >= BWDREF_FRAME) as i8;
                let ctx = av1_get_pred_context_single_ref_p1(xd) as usize;
                update_cdf(&mut xd.tile_ctx.single_ref_cdf[ctx][0], bit, 2);
                if bit != 0 {
                    debug_assert!(ref0 <= ALTREF_FRAME);
                    let ctx = av1_get_pred_context_single_ref_p2(xd) as usize;
                    update_cdf(
                        &mut xd.tile_ctx.single_ref_cdf[ctx][1],
                        (ref0 == ALTREF_FRAME) as i8,
                        2,
                    );
                    if ref0 != ALTREF_FRAME {
                        let ctx = av1_get_pred_context_single_ref_p6(xd) as usize;
                        update_cdf(
                            &mut xd.tile_ctx.single_ref_cdf[ctx][5],
                            (ref0 == ALTREF2_FRAME) as i8,
                            2,
                        );
                    }
                } else {
                    let bit1 = (!(ref0 == LAST2_FRAME || ref0 == LAST_FRAME)) as i8;
                    let ctx = av1_get_pred_context_single_ref_p3(xd) as usize;
                    update_cdf(&mut xd.tile_ctx.single_ref_cdf[ctx][2], bit1, 2);
                    if bit1 == 0 {
                        let ctx = av1_get_pred_context_single_ref_p4(xd) as usize;
                        update_cdf(
                            &mut xd.tile_ctx.single_ref_cdf[ctx][3],
                            (ref0 != LAST_FRAME) as i8,
                            2,
                        );
                    } else {
                        let ctx = av1_get_pred_context_single_ref_p5(xd) as usize;
                        update_cdf(
                            &mut xd.tile_ctx.single_ref_cdf[ctx][4],
                            (ref0 != LAST3_FRAME) as i8,
                            2,
                        );
                    }
                }
            }

            let fc: &mut FrameContext = &mut xd.tile_ctx;
            if cm.seq_params.enable_interintra_compound != 0 && is_interintra_allowed(&mbmi) {
                let bsize_group = SIZE_GROUP_LOOKUP[bsize as usize] as usize;
                if mbmi.ref_frame[1] == INTRA_FRAME {
                    update_cdf(&mut fc.interintra_cdf[bsize_group], 1, 2);
                    update_cdf(
                        &mut fc.interintra_mode_cdf[bsize_group],
                        mbmi.interintra_mode as i8,
                        INTERINTRA_MODES,
                    );
                    if av1_is_wedge_used(bsize) {
                        update_cdf(
                            &mut fc.wedge_interintra_cdf[bsize as usize],
                            mbmi.use_wedge_interintra as i8,
                            2,
                        );
                        if mbmi.use_wedge_interintra != 0 {
                            update_cdf(
                                &mut fc.wedge_idx_cdf[bsize as usize],
                                mbmi.interintra_wedge_index as i8,
                                16,
                            );
                        }
                    }
                } else {
                    update_cdf(&mut fc.interintra_cdf[bsize_group], 0, 2);
                }
            }

            let motion_allowed = if cm.features.switchable_motion_mode {
                motion_mode_allowed(
                    &cm.global_motion,
                    xd,
                    &mbmi,
                    cm.features.allow_warped_motion,
                )
            } else {
                SIMPLE_TRANSLATION
            };
            let fc: &mut FrameContext = &mut xd.tile_ctx;
            if mbmi.ref_frame[1] != INTRA_FRAME {
                if motion_allowed == WARPED_CAUSAL {
                    update_cdf(
                        &mut fc.motion_mode_cdf[bsize as usize],
                        mbmi.motion_mode as i8,
                        MOTION_MODES,
                    );
                } else if motion_allowed == OBMC_CAUSAL {
                    update_cdf(
                        &mut fc.obmc_cdf[bsize as usize],
                        (mbmi.motion_mode == OBMC_CAUSAL) as i8,
                        2,
                    );
                }
            }

            if has_second_ref(&mbmi) {
                debug_assert!(
                    current_frame.reference_mode != SINGLE_REFERENCE
                        && is_inter_compound_mode(mbmi.mode)
                        && mbmi.motion_mode == SIMPLE_TRANSLATION
                );

                let masked_compound_used =
                    is_any_masked_compound_used(bsize) && cm.seq_params.enable_masked_compound != 0;
                if masked_compound_used {
                    let comp_group_idx_ctx = get_comp_group_idx_context(xd) as usize;
                    update_cdf(
                        &mut xd.tile_ctx.comp_group_idx_cdf[comp_group_idx_ctx],
                        mbmi.comp_group_idx as i8,
                        2,
                    );
                }

                if mbmi.comp_group_idx == 0 {
                    let comp_index_ctx = get_comp_index_context(cm, xd) as usize;
                    update_cdf(
                        &mut xd.tile_ctx.compound_index_cdf[comp_index_ctx],
                        mbmi.compound_idx as i8,
                        2,
                    );
                } else {
                    debug_assert!(masked_compound_used);
                    if is_interinter_compound_used(COMPOUND_WEDGE, bsize) {
                        update_cdf(
                            &mut xd.tile_ctx.compound_type_cdf[bsize as usize],
                            (mbmi.interinter_comp.type_ as i32 - COMPOUND_WEDGE) as i8,
                            MASKED_COMPOUND_TYPES,
                        );
                    }
                }
            }
            if mbmi.interinter_comp.type_ as i32 == COMPOUND_WEDGE
                && is_interinter_compound_used(COMPOUND_WEDGE, bsize)
            {
                update_cdf(
                    &mut xd.tile_ctx.wedge_idx_cdf[bsize as usize],
                    mbmi.interinter_comp.wedge_index as i8,
                    16,
                );
            }
        }
    }

    if inter_block
        && cm.features.interp_filter == SWITCHABLE
        && mbmi.motion_mode != WARPED_CAUSAL
        && !is_nontrans_global_motion(&cm.global_motion, &mbmi)
    {
        update_filter_type_cdf(xd, &mbmi, cm.seq_params.enable_dual_filter != 0);
    }
    if inter_block && !segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_SKIP) {
        let mode = mbmi.mode;
        let mbmi_ext = &x.mbmi_ext;
        let mode_ctx = av1_mode_context_analyzer(&mbmi_ext.mode_context, &mbmi.ref_frame);
        let fc: &mut FrameContext = &mut x.e_mbd.tile_ctx;
        if has_second_ref(&mbmi) {
            update_cdf(
                &mut fc.inter_compound_mode_cdf[mode_ctx as usize],
                inter_compound_offset(mode),
                INTER_COMPOUND_MODES,
            );
        } else {
            av1_update_inter_mode_stats(fc, mode, mode_ctx);
        }

        let new_mv = mbmi.mode == NEWMV || mbmi.mode == NEW_NEWMV;
        if new_mv {
            let ref_frame_type = av1_ref_frame_type(&mbmi.ref_frame) as usize;
            for idx in 0..2 {
                if mbmi_ext.ref_mv_count[ref_frame_type] as i32 > idx + 1 {
                    let drl_ctx = av1_drl_ctx(&mbmi_ext.weight[ref_frame_type], idx as usize);
                    update_cdf(
                        &mut fc.drl_cdf[drl_ctx as usize],
                        (mbmi.ref_mv_idx as i32 != idx) as i8,
                        2,
                    );
                    if mbmi.ref_mv_idx as i32 == idx {
                        break;
                    }
                }
            }
        }

        if have_nearmv_in_inter_mode(mbmi.mode) {
            let ref_frame_type = av1_ref_frame_type(&mbmi.ref_frame) as usize;
            for idx in 1..3 {
                if mbmi_ext.ref_mv_count[ref_frame_type] as i32 > idx + 1 {
                    let drl_ctx = av1_drl_ctx(&mbmi_ext.weight[ref_frame_type], idx as usize);
                    update_cdf(
                        &mut fc.drl_cdf[drl_ctx as usize],
                        (mbmi.ref_mv_idx as i32 != idx - 1) as i8,
                        2,
                    );
                    if mbmi.ref_mv_idx as i32 == idx - 1 {
                        break;
                    }
                }
            }
        }
        if have_newmv_in_inter_mode(mbmi.mode) {
            let allow_hp = if cm.features.cur_frame_force_integer_mv {
                MV_SUBPEL_NONE
            } else {
                cm.features.allow_high_precision_mv as i32
            };
            let update = |x: &mut Macroblock, r: usize| {
                let ref_mv = av1_get_ref_mv(x, r as i32);
                let mv: Mv = mbmi.mv[r].as_mv();
                av1_update_mv_stats(&mv, &ref_mv.as_mv(), &mut x.e_mbd.tile_ctx.nmvc, allow_hp);
            };
            if new_mv {
                for r in 0..1 + has_second_ref(&mbmi) as usize {
                    update(x, r);
                }
            } else if mbmi.mode == NEAREST_NEWMV || mbmi.mode == NEAR_NEWMV {
                update(x, 1);
            } else if mbmi.mode == NEW_NEARESTMV || mbmi.mode == NEW_NEARMV {
                update(x, 0);
            }
        }
    }
    Ok(())
}

fn encode_b_nonrd(
    cpi: &Av1Comp,
    tile_data: &TileDataEnc,
    td: &mut ThreadData,
    mi_row: i32,
    mi_col: i32,
    dry_run: i32,
    bsize: BlockSize,
    partition: PartitionType,
    ctx: &PickModeContext,
) -> Result<(), ()> {
    let tile = &tile_data.tile_info;
    let x = &mut td.mb;
    av1_set_offsets_without_segment_id(cpi, tile, x, mi_row, mi_col, bsize);
    let origin_mult = x.rdmult;
    setup_block_rdmult(cpi, x, mi_row, mi_col, bsize, NO_AQ);
    x.e_mbd.mbmi_mut().partition = partition;
    av1_update_state(cpi, td, ctx, mi_row, mi_col, bsize, dry_run)?;
    let subsampling_x = cpi.common.seq_params.subsampling_x;
    let subsampling_y = cpi.common.seq_params.subsampling_y;
    let x = &mut td.mb;
    if dry_run == 0 {
        let (y, uv) = (
            x.cb_offset[PLANE_TYPE_Y as usize],
            x.cb_offset[PLANE_TYPE_UV as usize],
        );
        let i = x.mbmi_ext_frame;
        set_cb_offsets(&mut x.mbmi_ext_frame_base[i].cb_offset, y, uv);
        debug_assert!(
            (x.cb_offset[PLANE_TYPE_Y as usize] as i32)
                < (1 << NUM_PELS_LOG2_LOOKUP[cpi.common.seq_params.sb_size as usize])
        );
        debug_assert!(
            (x.cb_offset[PLANE_TYPE_UV as usize] as i32)
                < ((1 << NUM_PELS_LOG2_LOOKUP[cpi.common.seq_params.sb_size as usize])
                    >> (subsampling_x + subsampling_y))
        );
    }

    encode_superblock(cpi, tile_data, td, dry_run, bsize)?;
    let cm = &cpi.common;
    let x = &mut td.mb;
    if dry_run == 0 {
        update_cb_offsets(x, bsize, subsampling_x, subsampling_y);
        let xd = &mut x.e_mbd;
        if has_second_ref(xd.mbmi()) {
            let mbmi = xd.mbmi_mut();
            if mbmi.compound_idx == 0 || mbmi.interinter_comp.type_ as i32 == COMPOUND_AVERAGE {
                mbmi.comp_group_idx = 0;
            } else {
                mbmi.comp_group_idx = 1;
            }
            mbmi.compound_idx = 1;
        }
        let mbmi = *xd.mbmi();
        let rdc = &mut td.rd_counts;
        if mbmi.skip_mode != 0 {
            debug_assert!(!frame_is_intra_only(cm));
            rdc.skip_mode_used_flag = 1;
            if cm.current_frame.reference_mode == REFERENCE_MODE_SELECT && has_second_ref(&mbmi) {
                rdc.compound_ref_used_flag = 1;
            }
            set_ref_ptrs(cm, xd, mbmi.ref_frame[0], mbmi.ref_frame[1])?;
        } else {
            let seg_ref_active = segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_REF_FRAME);
            if !seg_ref_active {
                // If the segment reference feature is enabled we have only a single
                // reference frame allowed for the segment so exclude it from
                // the reference frame counts used to work out probabilities.
                if is_inter_block(&mbmi) {
                    av1_collect_neighbors_ref_counts(xd);
                    if cm.current_frame.reference_mode == REFERENCE_MODE_SELECT
                        && has_second_ref(&mbmi)
                    {
                        // This flag is also updated for 4x4 blocks
                        rdc.compound_ref_used_flag = 1;
                    }
                    set_ref_ptrs(cm, xd, mbmi.ref_frame[0], mbmi.ref_frame[1])?;
                }
            }
        }
        if cpi.oxcf.algo_cfg.loopfilter_control == LOOPFILTER_SELECTIVELY
            && (mbmi.mode == NEWMV || mbmi.mode < INTRA_MODE_END)
        {
            let blocks = MI_SIZE_HIGH[bsize as usize] as i32 * MI_SIZE_WIDE[bsize as usize] as i32;
            rdc.newmv_or_intra_blocks += blocks;
        }
        if tile_data.allow_update_cdf {
            update_stats(&cpi.common, td)?;
        }
    }
    let x = &mut td.mb;
    if cpi.oxcf.q_cfg.aq_mode == CYCLIC_REFRESH_AQ
        && x.e_mbd.mbmi().skip_txfm != 0
        && !cpi.rc.rtc_external_ratectrl
        && cpi.common.seg.enabled != 0
    {
        // (av1_cyclic_reset_segment_skip(): the cyclic refresh is off for
        // the saver)
        return Err(());
    }
    let x = &mut td.mb;
    let ref_frame_type = av1_ref_frame_type(&x.e_mbd.mbmi().ref_frame) as u8;
    let i = x.mbmi_ext_frame;
    let Macroblock {
        mbmi_ext_frame_base,
        mbmi_ext,
        ..
    } = x;
    av1_copy_mbmi_ext_to_mbmi_ext_frame(&mut mbmi_ext_frame_base[i], mbmi_ext, ref_frame_type);
    x.rdmult = origin_mult;
    Ok(())
}

fn get_force_zeromv_skip_flag_for_blk(cpi: &Av1Comp, x: &Macroblock, bsize: BlockSize) -> i32 {
    // Force zero MV skip based on SB level decision
    if x.force_zeromv_skip_for_sb < 2 {
        return x.force_zeromv_skip_for_sb;
    }

    // For blocks of size equal to superblock size, the decision would have been
    // already done at superblock level. Hence zeromv-skip decision is skipped.
    let cm = &cpi.common;
    if bsize == cm.seq_params.sb_size {
        return 0;
    }

    let num_planes = av1_num_planes(cm);
    let xd = &x.e_mbd;
    let thresh_exit_part_y = cpi.zeromv_skip_thresh_exit_part[bsize as usize];
    let thresh_exit_part_uv = calc_chroma_thresh_for_zeromv_skip(thresh_exit_part_y);
    let thresh_exit_part = [thresh_exit_part_y, thresh_exit_part_uv, thresh_exit_part_uv];
    let Some((yv12, yv12_idx)) = get_ref_frame_yv12_buf(cm, LAST_FRAME) else {
        return 0;
    };
    let sf = get_ref_scale_factors_const(cm, LAST_FRAME);

    let mut yv12_mb = [Buf2d::default(); MAX_MB_PLANE];
    av1_setup_pred_block(xd, &mut yv12_mb, yv12, yv12_idx, sf, sf, num_planes);

    for plane in 0..num_planes {
        let p = &x.plane[plane];
        let pd = &xd.plane[plane];
        let bs = get_plane_block_size(bsize, pd.subsampling_x, pd.subsampling_y);
        let plane_sad = cpi.ppi.fn_ptr.sdf(
            bs,
            &x.source_alloc,
            p.src.buf,
            p.src.stride as usize,
            &yv12.buffer_alloc,
            yv12_mb[plane].buf,
            yv12_mb[plane].stride as usize,
        );
        debug_assert!(plane < MAX_MB_PLANE);
        if plane_sad >= thresh_exit_part[plane] {
            return 0;
        }
    }
    1
}

/*\brief Top level function to pick block mode for non-RD optimized case
 *
 * Searches prediction modes, transform, and coefficient coding modes for an
 * individual coding block. This function is the top-level function that is
 * used for non-RD optimized mode search (controlled by
 * \c cpi->sf.rt_sf.use_nonrd_pick_mode). Depending on frame type it calls
 * inter/skip/hybrid-intra mode search functions
 *
 * \remark Nothing is returned. Instead, the chosen modes and contexts necessary
 * for reconstruction are stored in ctx, the rate-distortion stats are stored in
 * rd_cost. If no valid mode leading to rd_cost <= best_rd, the status will be
 * signalled by an INT64_MAX rd_cost->rdcost.
 */
fn pick_sb_modes_nonrd(
    cpi: &Av1Comp,
    tile_data: &TileDataEnc,
    x: &mut Macroblock,
    mi_row: i32,
    mi_col: i32,
    rd_cost: &mut RdStats,
    bsize: BlockSize,
    ctx: &mut PickModeContext,
) -> Result<(), ()> {
    // For nonrd mode, av1_set_offsets is already called at the superblock level
    // in encode_nonrd_sb when we determine the partitioning.
    if bsize != cpi.common.seq_params.sb_size || cpi.sf.rt_sf.nonrd_check_partition_split == 1 {
        av1_set_offsets(cpi, &tile_data.tile_info, x, mi_row, mi_col, bsize);
    }
    let cm = &cpi.common;
    let aq_mode = cpi.oxcf.q_cfg.aq_mode;

    // This is only needed for real time/allintra row-mt enabled multi-threaded
    // encoding with cost update frequency set to COST_UPD_TILE/COST_UPD_OFF.
    // (wait_for_top_right_sb(): one thread)

    // Sets up the tx_type_map buffer in MACROBLOCKD.
    x.e_mbd.tx_type_map = TxTypeMapRef::Block;
    x.e_mbd.tx_type_map_stride = MI_SIZE_WIDE[bsize as usize] as i32;
    // (the coefficient buffers are the planes': see context_tree.rs)

    x.force_zeromv_skip_for_blk = get_force_zeromv_skip_flag_for_blk(cpi, x, bsize);

    if x.force_zeromv_skip_for_blk == 0 {
        x.source_variance = av1_get_perpixel_variance_facade(
            cpi,
            &x.e_mbd,
            &x.source_alloc,
            &x.plane[0].src,
            bsize,
            AOM_PLANE_Y,
        )?;
    }
    // Save rdmult before it might be changed, so it can be restored later.
    let orig_rdmult = x.rdmult;
    setup_block_rdmult(cpi, x, mi_row, mi_col, bsize, aq_mode);
    // Set error per bit for current rdmult
    av1_set_error_per_bit(&mut x.errorperbit, x.rdmult);
    // Find best coding mode & reconstruct the MB so it is available
    // as a predictor for MBs that follow in the SB
    if frame_is_intra_only(cm) {
        hybrid_intra_mode_search(cpi, x, rd_cost, bsize, ctx)?;
    } else if segfeature_active(&cm.seg, x.e_mbd.mbmi().segment_id, SEG_LVL_SKIP) {
        // TODO(kyslov): add av1_nonrd_pick_inter_mode_sb_seg_skip
        // (av1_rd_pick_inter_mode_sb_seg_skip(): segmentation is off)
        return Err(());
    } else {
        av1_nonrd_pick_inter_mode_sb(cpi, tile_data, x, rd_cost, bsize, ctx)?;
    }
    if cpi.sf.rt_sf.skip_cdef_sb != 0 {
        // cdef_strength is initialized to 1 which means skip_cdef, and is updated
        // here. Check to see is skipping cdef is allowed.
        let allow_cdef_skipping = cpi.rc.frames_since_key > 10
            && cpi.rc.high_source_sad == 0
            && !(x.color_sensitivity[0] != 0 || x.color_sensitivity[1] != 0);

        // Find the corresponding 64x64 block. It'll be the 128x128 block if that's
        // the block size.
        let mi_row_sb = mi_row - mi_row % MI_SIZE_64X64;
        let mi_col_sb = mi_col - mi_col % MI_SIZE_64X64;
        let xd = &mut x.e_mbd;
        let mode = xd.mbmi().mode;
        let sb =
            xd.mi_grid_base[get_mi_grid_idx(&cm.mi_params, mi_row_sb, mi_col_sb) as usize] as usize;
        // Do not skip if intra or new mv is picked, or color sensitivity is set.
        // Never skip on slide/scene change.
        xd.mi_alloc[sb].cdef_strength = (xd.mi_alloc[sb].cdef_strength != 0
            && allow_cdef_skipping
            && !(mode < INTRA_MODES || mode == NEWMV))
            as i8;
        // Store in the pickmode context.
        ctx.mic.cdef_strength = xd.mi_alloc[sb].cdef_strength;
    }
    x.rdmult = orig_rdmult;
    ctx.rd_stats.rate = rd_cost.rate;
    ctx.rd_stats.dist = rd_cost.dist;
    ctx.rd_stats.rdcost = rd_cost.rdcost;
    Ok(())
}

/// The pick and encode of one coding block along the partition
/// (`pick_sb_modes_nonrd()` and `encode_b_nonrd()` with a fresh
/// context; see context_tree.rs).
fn pick_and_encode_b_nonrd(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile_data: &TileDataEnc,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    partition: PartitionType,
) -> Result<(), ()> {
    let mut dummy_cost = RdStats::default();
    av1_invalid_rd_stats(&mut dummy_cost);
    let mut ctx = av1_alloc_pmc(bsize)?;
    pick_sb_modes_nonrd(
        cpi,
        tile_data,
        &mut td.mb,
        mi_row,
        mi_col,
        &mut dummy_cost,
        bsize,
        &mut ctx,
    )?;
    encode_b_nonrd(
        cpi, tile_data, td, mi_row, mi_col, 0, bsize, partition, &ctx,
    )
}

/*\brief AV1 block partition application (minimal RD search).
 *
 * This function applies proposed partitioning to a superblock and performs
 * mode search. This function is used in nonrd_pickmode encoding flow.
 * \remark Nothing is returned. The pc_tree struct is modified to store the
 * picked partition and modes.
 */
// (the partition tree is implicit: see context_tree.rs)
pub(crate) fn av1_nonrd_use_partition(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile_data: &TileDataEnc,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let mi_params = &cm.mi_params;
    let tile_info = &tile_data.tile_info;
    // Only square blocks from 8x8 to 128x128 are supported
    debug_assert!(bsize >= BLOCK_8X8 && bsize <= BLOCK_128X128);
    let bs = MI_SIZE_WIDE[bsize as usize] as i32;
    let hbs = bs / 2;
    let x = &mut td.mb;
    let partition = if bsize >= BLOCK_8X8 {
        get_partition(mi_params, &x.e_mbd, mi_row, mi_col, bsize)
    } else {
        PARTITION_NONE
    };
    let subsize = get_partition_subsize(bsize, partition);
    debug_assert!(subsize <= BLOCK_LARGEST);
    let pl = if bsize >= BLOCK_8X8 {
        partition_plane_context(&x.e_mbd, mi_row, mi_col, bsize)
    } else {
        0
    };

    if mi_row >= mi_params.mi_rows || mi_col >= mi_params.mi_cols {
        return Ok(());
    }

    debug_assert!(MI_SIZE_WIDE[bsize as usize] == MI_SIZE_HIGH[bsize as usize]);

    x.e_mbd.above_txfm_context = mi_col as usize;
    x.e_mbd.left_txfm_context = (mi_row & MAX_MIB_MASK) as usize;
    x.e_mbd.above_contexts_tile_row = tile_info.tile_row as usize;

    // Initialize default mode evaluation params
    set_mode_eval_params(cpi, x, DEFAULT_EVAL);

    x.reuse_inter_pred = cpi.sf.rt_sf.reuse_inter_pred_nonrd != 0;

    let change_none_to_split = 0;
    // (nonrd_check_partition_split is a real-time feature)
    debug_assert!(!(partition == PARTITION_NONE && cpi.sf.rt_sf.nonrd_check_partition_split == 1));

    match partition {
        PARTITION_NONE => {
            pick_and_encode_b_nonrd(cpi, td, tile_data, mi_row, mi_col, bsize, partition)?;
        }
        PARTITION_VERT => {
            pick_and_encode_b_nonrd(cpi, td, tile_data, mi_row, mi_col, subsize, PARTITION_VERT)?;
            if mi_col + hbs < mi_params.mi_cols && bsize > BLOCK_8X8 {
                pick_and_encode_b_nonrd(
                    cpi,
                    td,
                    tile_data,
                    mi_row,
                    mi_col + hbs,
                    subsize,
                    PARTITION_VERT,
                )?;
            }
        }
        PARTITION_HORZ => {
            pick_and_encode_b_nonrd(cpi, td, tile_data, mi_row, mi_col, subsize, PARTITION_HORZ)?;

            if mi_row + hbs < mi_params.mi_rows && bsize > BLOCK_8X8 {
                pick_and_encode_b_nonrd(
                    cpi,
                    td,
                    tile_data,
                    mi_row + hbs,
                    mi_col,
                    subsize,
                    PARTITION_HORZ,
                )?;
            }
        }
        PARTITION_SPLIT => {
            if cpi.sf.rt_sf.nonrd_check_partition_merge_mode != 0
                && av1_is_leaf_split_partition(cm, &x.e_mbd, mi_row, mi_col, bsize)
                && !frame_is_intra_only(cm)
                && bsize <= BLOCK_64X64
            {
                // (try_merge(): the speed features' merge mode check is off)
                return Err(());
            }
            for i in 0..SUB_PARTITIONS_SPLIT {
                let x_idx = (i & 1) * hbs;
                let y_idx = (i >> 1) * hbs;
                if (mi_row + y_idx >= mi_params.mi_rows) || (mi_col + x_idx >= mi_params.mi_cols) {
                    continue;
                }
                av1_nonrd_use_partition(
                    cpi,
                    td,
                    tile_data,
                    mi_row + y_idx,
                    mi_col + x_idx,
                    subsize,
                )?;
            }

            if change_none_to_split == 0 {
                // Note: Palette, cfl are not supported.
                let x = &td.mb;
                if !frame_is_intra_only(cm)
                    && !tile_data.allow_update_cdf
                    && cpi.sf.rt_sf.partition_direct_merging != 0
                    && x.mode_costs.partition_cost[pl as usize][PARTITION_NONE as usize]
                        < x.mode_costs.partition_cost[pl as usize][PARTITION_SPLIT as usize]
                    && (mi_row + bs <= mi_params.mi_rows)
                    && (mi_col + bs <= mi_params.mi_cols)
                {
                    // (direct_partition_merging(): the CDFs are updated, which
                    // rules the merging out)
                    return Err(());
                }
            }
        }
        _ => {
            // (PARTITION_VERT_A, PARTITION_VERT_B, PARTITION_HORZ_A,
            // PARTITION_HORZ_B, PARTITION_HORZ_4, PARTITION_VERT_4)
            debug_assert!(false, "Cannot handle extended partition types");
        }
    }
    Ok(())
}
