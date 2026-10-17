// Rust translation of the parts of av1/encoder/partition_search.c and
// av1/encoder/partition_search.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the non-RD coding of a superblock along a given partition, for key
// frames, with one thread.
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
use super::encodemb::av1_encode_intra_block_plane;
use super::encoder::*;
use super::encodetxb::av1_update_intra_mb_txb_context;
use super::nonrd_pickmode::{av1_copy_mbmi_ext_to_mbmi_ext_frame, av1_nonrd_pick_intra_mode};
use super::rd::{av1_invalid_rd_stats, av1_set_error_per_bit};
use super::rdopt_utils::{set_mode_eval_params, set_tx_size_search_method};
use crate::aom::aom_dsp::prob::update_cdf;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cfl::{is_cfl_allowed, store_cfl_required};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mvref_common::av1_ref_frame_type;
use crate::aom::av1::common::pred_common::{av1_get_skip_txfm_context, get_tx_size_context};
use crate::aom::av1::common::reconinter::av1_setup_dst_planes;
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

fn encode_superblock(
    cpi: &Av1Comp,
    tile_data: &TileDataEnc,
    td: &mut ThreadData,
    dry_run: i32,
    bsize: BlockSize,
) {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);
    let x = &mut td.mb;
    let mbmi = *x.e_mbd.mbmi();
    let seg_skip = segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_SKIP);
    let mis = cm.mi_params.mi_stride;
    let mi_width = MI_SIZE_WIDE[bsize as usize] as i32;
    let mi_height = MI_SIZE_HIGH[bsize as usize] as i32;
    let is_inter = is_inter_block(&mbmi);
    // (the saver's frames are all intra)
    debug_assert!(!is_inter);

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
    {
        x.e_mbd.cfl.store_y = store_cfl_required(&cm.seq_params, &x.e_mbd);
        x.e_mbd.mbmi_mut().skip_txfm = 1;
        let src = &cpi.source().buffer_alloc;
        for plane in 0..num_planes {
            let segment_id = x.e_mbd.mbmi().segment_id as usize;
            av1_encode_intra_block_plane(
                cpi,
                x,
                src,
                bsize,
                plane,
                dry_run,
                cpi.optimize_seg_arr[segment_id],
            );
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

        av1_update_intra_mb_txb_context(cpi, td, dry_run, bsize, tile_data.allow_update_cdf as u8);
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
        } else {
            // The new intra coding scheme requires no change of transform size
            let intra_tx_size = mbmi.tx_size;

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
        // (tx_partition_set_contexts(): inter blocks only)
    } else {
        let mut tx_size = mbmi.tx_size;
        // The new intra coding scheme requires no change of transform size
        tx_size = if bsize > BLOCK_4X4 { tx_size } else { TX_4X4 };
        xd.mbmi_mut().tx_size = tx_size;
        let (w, h) = (xd.width as i32, xd.height as i32);
        let skip = (mbmi.skip_txfm != 0 || seg_skip) && is_inter_block(&mbmi);
        set_txfm_ctxs(tx_size, w, h, skip, xd);
    }

    // (cfl_store_block(): inter blocks only)
    debug_assert!(!(is_inter_block(xd.mbmi()) && !xd.is_chroma_ref && is_cfl_allowed(xd) != 0));
    // (the temporal noise estimate is a real-time feature)
    debug_assert!(dry_run != 0 || cpi.sf.rt_sf.use_temporal_noise_estimate == 0);
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
    // (av1_set_mv_limits(): the MV limits are only read by motion searches)

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
) {
    let hybrid_intra_pickmode = cpi.sf.rt_sf.hybrid_intra_pickmode;
    // Use rd pick for intra mode search based on block size and variance.
    // (hybrid_intra_pickmode is off for the saver's speed: the RD intra
    // search is never used)
    debug_assert!(!(hybrid_intra_pickmode != 0 && bsize < BLOCK_16X16));
    av1_nonrd_pick_intra_mode(cpi, x, &cpi.source().buffer_alloc, rd_cost, bsize, ctx);
}

// (wait_for_top_right_sb(): one thread, so the row synchronization never
// waits)

fn update_stats(cm: &Av1Common, td: &mut ThreadData) {
    let x = &mut td.mb;
    let xd = &mut x.e_mbd;
    let mbmi = *xd.mbmi();
    let current_frame = &cm.current_frame;
    let seg_ref_active = segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_REF_FRAME);

    // (skip mode: inter frames only)
    debug_assert!(current_frame.skip_mode_info.skip_mode_flag == 0);

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

    // (intra block copy is off: allow_intrabc stays 0 with screen content
    // tools off)
    debug_assert!(!(av1_allow_intrabc(cm) && cm.features.allow_intrabc));
    // (frame_is_intra_only(cm): the rest is for inter frames)
    debug_assert!(frame_is_intra_only(cm) || mbmi.skip_mode != 0);
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
) {
    let tile = &tile_data.tile_info;
    let x = &mut td.mb;
    av1_set_offsets_without_segment_id(cpi, tile, x, mi_row, mi_col, bsize);
    let origin_mult = x.rdmult;
    setup_block_rdmult(cpi, x, mi_row, mi_col, bsize, NO_AQ);
    x.e_mbd.mbmi_mut().partition = partition;
    av1_update_state(cpi, td, ctx, mi_row, mi_col, bsize, dry_run);
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

    encode_superblock(cpi, tile_data, td, dry_run, bsize);
    let x = &mut td.mb;
    if dry_run == 0 {
        update_cb_offsets(x, bsize, subsampling_x, subsampling_y);
        let mbmi = *x.e_mbd.mbmi();
        // (compound prediction, skip mode, reference frame counts: inter
        // blocks only)
        debug_assert!(!has_second_ref(&mbmi) && mbmi.skip_mode == 0 && !is_inter_block(&mbmi));
        // (the selective loop filter control is a real-time feature)
        debug_assert!(cpi.oxcf.algo_cfg.loopfilter_control != LOOPFILTER_SELECTIVELY);
        if tile_data.allow_update_cdf {
            update_stats(&cpi.common, td);
        }
    }
    // (the cyclic refresh is a real-time feature)
    debug_assert!(cpi.oxcf.q_cfg.aq_mode != CYCLIC_REFRESH_AQ);
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
}

fn get_force_zeromv_skip_flag_for_blk(x: &Macroblock) -> i32 {
    // Force zero MV skip based on SB level decision
    // (x->force_zeromv_skip_for_sb is 0 for the saver: the decision is an
    // inter frame one)
    debug_assert!(x.force_zeromv_skip_for_sb < 2);
    x.force_zeromv_skip_for_sb
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
) {
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

    x.force_zeromv_skip_for_blk = get_force_zeromv_skip_flag_for_blk(x);

    if x.force_zeromv_skip_for_blk == 0 {
        x.source_variance = av1_get_perpixel_variance_facade(
            cpi,
            &x.e_mbd,
            &cpi.source().buffer_alloc,
            &x.plane[0].src,
            bsize,
            AOM_PLANE_Y,
        );
    }
    // Save rdmult before it might be changed, so it can be restored later.
    let orig_rdmult = x.rdmult;
    setup_block_rdmult(cpi, x, mi_row, mi_col, bsize, aq_mode);
    // Set error per bit for current rdmult
    av1_set_error_per_bit(&mut x.errorperbit, x.rdmult);
    // Find best coding mode & reconstruct the MB so it is available
    // as a predictor for MBs that follow in the SB
    debug_assert!(frame_is_intra_only(cm));
    hybrid_intra_mode_search(cpi, x, rd_cost, bsize, ctx);
    // (skip_cdef_sb is a real-time feature)
    debug_assert!(cpi.sf.rt_sf.skip_cdef_sb == 0);
    x.rdmult = orig_rdmult;
    ctx.rd_stats.rate = rd_cost.rate;
    ctx.rd_stats.dist = rd_cost.dist;
    ctx.rd_stats.rdcost = rd_cost.rdcost;
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
    );
    encode_b_nonrd(
        cpi, tile_data, td, mi_row, mi_col, 0, bsize, partition, &ctx,
    );
    Ok(())
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
    let _ = pl;

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
            // (try_merge(): inter frames only)
            debug_assert!(frame_is_intra_only(cm));
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
                // (direct_partition_merging(): inter frames only)
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
