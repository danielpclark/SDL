// Rust translation of the parts of av1/encoder/encodeframe.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the coding of a key frame's tiles with the non-RD pick mode, with one
// thread.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The frame encoder: per-frame setup, and the superblock loop over the
//! tiles. While the tiles are coded the thread data is taken out of the
//! compressor (so the compressor can be shared), and the frame-wide state
//! the blocks write (mode info grid, contexts, reconstruction,
//! coefficient buffers) is moved into it (see block.rs and blockd.rs).

use super::av1_quantize::av1_frame_init_quantizer;
use super::block::*;
use super::encodeframe_utils::*;
use super::encoder::*;
use super::encodetxb::av1_get_cb_coeff_buffer;
use super::ethread::{av1_encode_tiles_row_mt, MOD_PACK_BS};
use super::global_motion_facade::av1_compute_global_motion_facade;
use super::partition_search::{av1_nonrd_use_partition, av1_set_offsets};
use super::ratectrl::LF_UPDATE;
use super::rd::av1_initialize_rd_consts;
use super::rd::av1_set_sad_per_bit;
use super::rd::reset_thresh_freq_fact;
use super::rdopt_utils::select_tx_mode;
use super::speed_features::*;
use super::var_based_part::av1_choose_var_based_partitioning;
use crate::aom::aom_dsp::variance::{aom_highbd_10_variance, aom_variance};
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::av1_loopfilter::av1_loop_filter_frame;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cfl::cfl_init;
use crate::aom::av1::common::entropymode::{
    av1_set_default_mode_deltas, av1_set_default_ref_deltas,
};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mvref_common::*;
use crate::aom::av1::common::reconinter::setup_pred_plane;
use crate::aom::av1::common::seg_common::MAX_SEGMENTS;
use crate::aom::av1::common::tile_common::*;

/// `AVG_CDF_WEIGHT_LEFT`
const AVG_CDF_WEIGHT_LEFT: i32 = 3;
/// `AVG_CDF_WEIGHT_TOP_RIGHT`
const AVG_CDF_WEIGHT_TOP_RIGHT: i32 = 1;

pub(crate) fn av1_init_rtc_counters(x: &mut Macroblock) {
    // (av1_init_cyclic_refresh_counters())
    x.actual_num_seg1_blocks = 0;
    x.actual_num_seg2_blocks = 0;
    x.cnt_zeromv = 0;
}

pub(crate) fn av1_accumulate_rtc_counters(cpi: &mut Av1Comp, x: &Macroblock) {
    // (av1_accumulate_cyclic_refresh_counters(): the cyclic refresh is off
    // for the saver)
    debug_assert!(cpi.oxcf.q_cfg.aq_mode != CYCLIC_REFRESH_AQ);
    cpi.rc.cnt_zeromv += x.cnt_zeromv;
}

// This is used as a reference when computing the source variance for the
//  purposes of activity masking.
// Eventually this should be replaced by custom no-reference routines,
//  which will be faster.
static AV1_VAR_OFFS: [u16; MAX_SB_SIZE as usize] = [128; MAX_SB_SIZE as usize];

static AV1_HIGH_VAR_OFFS_8: [u16; MAX_SB_SIZE as usize] = [128; MAX_SB_SIZE as usize];

static AV1_HIGH_VAR_OFFS_10: [u16; MAX_SB_SIZE as usize] = [128 * 4; MAX_SB_SIZE as usize];

/// Translation of `av1_get_perpixel_variance()`
/// (`cpi->ppi->fn_ptr[plane_bsize].vf` is `aom_variance<w>x<h>`, or
/// `aom_highbd_<bd>_variance<w>x<h>` for high bit depth buffers; the saver's
/// buffers are of 8 or 10 bits).
pub(crate) fn av1_get_perpixel_variance(
    xd: &MacroBlockD,
    src: &[u16],
    r: &Buf2d,
    bsize: BlockSize,
    plane: usize,
    use_hbd: bool,
) -> Result<u32, ()> {
    let subsampling_x = xd.plane[plane].subsampling_x;
    let subsampling_y = xd.plane[plane].subsampling_y;
    let plane_bsize = get_plane_block_size(bsize, subsampling_x, subsampling_y);
    let mut sse: u32 = 0;
    let w = BLOCK_SIZE_WIDE[plane_bsize as usize] as i32;
    let h = BLOCK_SIZE_HIGH[plane_bsize as usize] as i32;
    let var = if use_hbd {
        let bd = xd.bd;
        debug_assert!(bd == 8 || bd == 10 || bd == 12);
        match bd {
            8 => aom_variance(
                src,
                r.buf,
                r.stride as usize,
                &AV1_HIGH_VAR_OFFS_8,
                0,
                0,
                w,
                h,
                &mut sse,
            ),
            10 => aom_highbd_10_variance(
                src,
                r.buf,
                r.stride as usize,
                &AV1_HIGH_VAR_OFFS_10,
                0,
                0,
                w,
                h,
                &mut sse,
            ),
            // (12-bit buffers: the saver's are of 8 or 10 bits)
            _ => return Err(()),
        }
    } else {
        aom_variance(
            src,
            r.buf,
            r.stride as usize,
            &AV1_VAR_OFFS,
            0,
            0,
            w,
            h,
            &mut sse,
        )
    };
    Ok(
        ((var as u64 + ((1u64 << NUM_PELS_LOG2_LOOKUP[plane_bsize as usize]) >> 1))
            >> NUM_PELS_LOG2_LOOKUP[plane_bsize as usize]) as u32,
    )
}

pub(crate) fn av1_get_perpixel_variance_facade(
    _cpi: &Av1Comp,
    xd: &MacroBlockD,
    src: &[u16],
    r: &Buf2d,
    bsize: BlockSize,
    plane: usize,
) -> Result<u32, ()> {
    let use_hbd = is_cur_buf_hbd(xd);
    av1_get_perpixel_variance(xd, src, r, bsize, plane, use_hbd)
}

pub(crate) fn av1_setup_src_planes(
    x: &mut Macroblock,
    src: &Yv12BufferConfig,
    mi_row: i32,
    mi_col: i32,
    num_planes: usize,
    bsize: BlockSize,
) {
    // Set current frame pointer.
    x.e_mbd.cur_buf_flags = src.flags;

    // We use AOMMIN(num_planes, MAX_MB_PLANE) instead of num_planes to quiet
    // the static analysis warnings.
    for i in 0..num_planes.min(MAX_MB_PLANE) {
        let is_uv = (i > 0) as usize;
        let (ss_x, ss_y) = (
            x.e_mbd.plane[i].subsampling_x,
            x.e_mbd.plane[i].subsampling_y,
        );
        setup_pred_plane(
            &mut x.plane[i].src,
            bsize,
            src.buffers[i],
            src.crop_widths[is_uv],
            src.crop_heights[is_uv],
            src.strides[is_uv],
            mi_row,
            mi_col,
            None,
            ss_x,
            ss_y,
        );
    }
}

/*\brief Encode a superblock (minimal RD search involved)
 *
 * Encodes the superblock by a pre-determined partition pattern, only minor
 * rd-based searches are allowed to adjust the initial pattern. It is only used
 * by realtime encoding.
 */
fn encode_nonrd_sb(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile_data: &TileDataEnc,
    mi_row: i32,
    mi_col: i32,
    seg_skip: i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let sf = &cpi.sf;
    let tile_info = &tile_data.tile_info;
    let sb_size = cm.seq_params.sb_size;

    // Set the partition
    if sf.part_sf.partition_search_type == FIXED_PARTITION || seg_skip != 0 {
        // set a fixed-size partition
        // (the saver's speed features use the variance-based partition, and
        // segmentation is off)
        debug_assert!(false);
    } else if sf.part_sf.partition_search_type == VAR_BASED_PARTITION {
        // set a variance-based partition
        av1_set_offsets(cpi, tile_info, &mut td.mb, mi_row, mi_col, sb_size);
        av1_choose_var_based_partitioning(cpi, tile_info, td, mi_row, mi_col)?;
    }
    debug_assert!(
        sf.part_sf.partition_search_type == FIXED_PARTITION
            || seg_skip != 0
            || sf.part_sf.partition_search_type == VAR_BASED_PARTITION
    );
    td.mb.cb_offset = [0, 0];

    // Initialize the flag to skip cdef to 1.
    if sf.rt_sf.skip_cdef_sb != 0 {
        let block64_in_sb = if sb_size == BLOCK_128X128 { 2 } else { 1 };
        // If 128x128 block is used, we need to set the flag for all 4 64x64 sub
        // "blocks".
        let xd = &mut td.mb.e_mbd;
        let mi = get_mi_grid_idx(&cm.mi_params, mi_row, mi_col) as i32;
        for r in 0..block64_in_sb {
            for c in 0..block64_in_sb {
                let idx_in_sb = r * MI_SIZE_64X64 * cm.mi_params.mi_stride + c * MI_SIZE_64X64;
                let i = xd.mi_grid_base[(mi + idx_in_sb) as usize];
                if i != MI_NONE {
                    xd.mi_alloc[i as usize].cdef_strength = 1;
                }
            }
        }
    }
    av1_nonrd_use_partition(cpi, td, tile_data, mi_row, mi_col, sb_size)
}

// Check if the cost update of symbols mode, coeff and dv are tile or off.
#[inline]
fn is_mode_coeff_dv_upd_freq_tile_or_off(cpi: &Av1Comp) -> bool {
    let inter_sf = &cpi.sf.inter_sf;

    inter_sf.coeff_cost_upd_level <= INTERNAL_COST_UPD_TILE
        && inter_sf.mode_cost_upd_level <= INTERNAL_COST_UPD_TILE
        && cpi.sf.intra_sf.dv_cost_upd_level <= INTERNAL_COST_UPD_TILE
}

// When row-mt is enabled and cost update frequencies are set to off/tile,
// processing of current SB can start even before processing of top-right SB
// is finished. This function checks if it is sufficient to wait for top SB
// to finish processing before current SB starts processing.
#[inline]
fn delay_wait_for_top_right_sb(cpi: &Av1Comp) -> bool {
    let mode = cpi.oxcf.mode;
    if mode == GOOD {
        return false;
    }

    if mode == ALLINTRA {
        is_mode_coeff_dv_upd_freq_tile_or_off(cpi)
    } else if mode == REALTIME {
        is_mode_coeff_dv_upd_freq_tile_or_off(cpi)
            && cpi.sf.inter_sf.mv_cost_upd_level <= INTERNAL_COST_UPD_TILE
    } else {
        false
    }
}

/*\brief Calculate source SAD at superblock level using 64x64 block source SAD
 */
#[inline]
fn get_sb_source_sad(cpi: &Av1Comp, mi_row: i32, mi_col: i32) -> u64 {
    if cpi.src_sad_blk_64x64.is_empty() {
        return u64::MAX;
    }

    let cm = &cpi.common;
    let blk_64x64_in_mis = if cm.seq_params.sb_size == BLOCK_128X128 {
        cm.seq_params.mib_size >> 1
    } else {
        cm.seq_params.mib_size
    };
    let num_blk_64x64_cols = (cm.mi_params.mi_cols + blk_64x64_in_mis - 1) / blk_64x64_in_mis;
    let num_blk_64x64_rows = (cm.mi_params.mi_rows + blk_64x64_in_mis - 1) / blk_64x64_in_mis;
    let blk_64x64_col_index = mi_col / blk_64x64_in_mis;
    let blk_64x64_row_index = mi_row / blk_64x64_in_mis;
    let mut curr_sb_sad = u64::MAX;
    let i = (blk_64x64_col_index + blk_64x64_row_index * num_blk_64x64_cols) as usize;
    let d = |o: usize| cpi.src_sad_blk_64x64.get(i + o).copied().unwrap_or(0);
    if cm.seq_params.sb_size == BLOCK_128X128
        && blk_64x64_col_index + 1 < num_blk_64x64_cols
        && blk_64x64_row_index + 1 < num_blk_64x64_rows
    {
        // Calculate SB source SAD by accumulating source SAD of 64x64 blocks in the
        // superblock
        let n = num_blk_64x64_cols as usize;
        curr_sb_sad = d(0)
            .wrapping_add(d(1))
            .wrapping_add(d(n))
            .wrapping_add(d(n + 1));
    } else if cm.seq_params.sb_size == BLOCK_64X64 {
        curr_sb_sad = d(0);
    }
    curr_sb_sad
}

/*\brief Determine whether grading content can be skipped based on sad stat
 */
#[inline]
fn is_calc_src_content_needed(cpi: &Av1Comp, x: &mut Macroblock, mi_row: i32, mi_col: i32) -> bool {
    if cpi.svc.spatial_layer_id < cpi.svc.number_spatial_layers - 1 {
        return true;
    }
    let curr_sb_sad = get_sb_source_sad(cpi, mi_row, mi_col);
    if curr_sb_sad == u64::MAX {
        return true;
    }
    if curr_sb_sad == 0 {
        x.content_state_sb.source_sad_nonrd = K_ZERO_SAD;
        return false;
    }
    let cm = &cpi.common;
    let mut do_calc_src_content = true;

    if cpi.oxcf.speed < 9 {
        return do_calc_src_content;
    }

    // TODO(yunqing): Tune/validate the thresholds for 128x128 SB size.
    if cm.width.min(cm.height) < 360 {
        // Derive Average 64x64 block source SAD from SB source SAD
        let avg_64x64_blk_sad = if cm.seq_params.sb_size == BLOCK_128X128 {
            (curr_sb_sad + 2) >> 2
        } else {
            curr_sb_sad
        };

        // The threshold is determined based on kLowSad and kHighSad threshold and
        // test results.
        let thresh_low: u64 = 15000;
        let thresh_high: u64 = 40000;

        if avg_64x64_blk_sad > thresh_low && avg_64x64_blk_sad < thresh_high {
            do_calc_src_content = false;
            // Note: set x->content_state_sb.source_sad_rd as well if this is extended
            // to RTC rd path.
            x.content_state_sb.source_sad_nonrd = K_MED_SAD;
        }
    }

    do_calc_src_content
}

/*\brief Determine whether grading content is needed based on sf and frame stat
 */
// TODO(any): consolidate sfs to make interface cleaner
#[inline]
fn grade_source_content_sb(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    tile_data: &TileDataEnc,
    mi_row: i32,
    mi_col: i32,
) {
    let cm = &cpi.common;
    if cm.current_frame.frame_type == KEY_FRAME || {
        // (no SVC)
        debug_assert!(!cpi.ppi.use_svc);
        false
    } {
        debug_assert!(x.content_state_sb.source_sad_nonrd == K_MED_SAD);
        debug_assert!(x.content_state_sb.source_sad_rd == K_MED_SAD);
        return;
    }
    let mut calc_src_content = false;

    if cpi.sf.rt_sf.source_metrics_sb_nonrd != 0 {
        if cpi.sf.rt_sf.check_scene_detection == 0 || cpi.rc.frame_source_sad > 0 {
            calc_src_content = is_calc_src_content_needed(cpi, x, mi_row, mi_col);
        } else {
            x.content_state_sb.source_sad_nonrd = K_ZERO_SAD;
        }
    } else if cpi.sf.rt_sf.var_part_based_on_qidx >= 1 && cm.width * cm.height <= 352 * 288 {
        if cpi.rc.frame_source_sad > 0 {
            calc_src_content = true;
        } else {
            x.content_state_sb.source_sad_rd = K_ZERO_SAD;
        }
    }
    if calc_src_content {
        av1_source_content_sb(cpi, x, tile_data, mi_row, mi_col);
    }
}

/*\brief Encode a superblock row by breaking it into superblocks
 *
 * Do partition and mode search for an sb row: one row of superblocks filling up
 * the width of the current tile.
 */
fn encode_sb_row(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile_data: &TileDataEnc,
    mi_row: i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let tile_info = &tile_data.tile_info;
    let row_mt_enabled = cpi.mt_info.row_mt_enabled;
    let sb_cols_in_tile = av1_get_sb_cols_in_tile(cm, tile_info);
    let mib_size = cm.seq_params.mib_size;
    let use_nonrd_mode = cpi.sf.rt_sf.use_nonrd_pick_mode;

    // Initialize the left context for the new SB row
    av1_zero_left_context(&mut td.mb.e_mbd);

    // Reset delta for quantizer and loof filters at the beginning of every tile
    if mi_row == tile_info.mi_row_start || row_mt_enabled {
        if cm.delta_q_info.delta_q_present_flag != 0 {
            td.mb.e_mbd.current_base_qindex = cm.quant_params.base_qindex;
        }
        if cm.delta_q_info.delta_lf_present_flag != 0 {
            av1_reset_loop_filter_delta(&mut td.mb.e_mbd, av1_num_planes(cm));
        }
    }

    reset_thresh_freq_fact(&mut td.mb);

    // Code each SB in the row
    let mut mi_col = tile_info.mi_col_start;
    let mut sb_col_in_tile = 0usize;
    while mi_col < tile_info.mi_col_end {
        // In realtime/allintra mode and when frequency of cost updates is off/tile,
        // wait for the top superblock to finish encoding. Otherwise, wait for the
        // top-right superblock to finish encoding.
        // (the rows are coded one after the other: see ethread.rs)
        let update_cdf = tile_data.allow_update_cdf && row_mt_enabled;
        if update_cdf && tile_info.mi_row_start != mi_row {
            let x = &mut td.mb;
            if tile_info.mi_col_start == mi_col {
                // restore frame context at the 1st column sb
                *x.e_mbd.tile_ctx = x.row_ctx[0].clone();
            } else {
                // update context
                let wt_left = AVG_CDF_WEIGHT_LEFT;
                let wt_tr = AVG_CDF_WEIGHT_TOP_RIGHT;
                let i = if tile_info.mi_col_end > mi_col + mib_size {
                    sb_col_in_tile
                } else {
                    sb_col_in_tile - 1
                };
                av1_avg_cdf_symbols(&mut x.e_mbd.tile_ctx, &x.row_ctx[i], wt_left, wt_tr);
            }
        }

        // Update the rate cost tables for some symbols
        av1_set_cost_upd_freq(cpi, td, tile_info, mi_row, mi_col)?;

        let x = &mut td.mb;
        // Reset color coding related parameters
        x.color_sensitivity_sb = [0; 2];
        x.color_sensitivity_sb_g = [0; 2];
        x.color_sensitivity = [0; 2];
        x.content_state_sb.source_sad_nonrd = K_MED_SAD;
        x.content_state_sb.source_sad_rd = K_MED_SAD;
        x.content_state_sb.lighting_change = 0;
        x.content_state_sb.low_sumdiff = 0;
        x.force_zeromv_skip_for_sb = 0;

        if cpi.oxcf.mode == ALLINTRA {
            x.intra_sb_rdmult_modifier = 128;
        }

        x.e_mbd.cur_frame_force_integer_mv = cm.features.cur_frame_force_integer_mv as i32;
        x.source_variance = u32::MAX;
        x.cb_coef_buff = av1_get_cb_coeff_buffer(cpi, mi_row, mi_col);

        // Get segment id and skip flag
        let seg = &cm.seg;
        let seg_skip = 0;
        debug_assert!(seg.enabled == 0);
        // (produce_gradients_for_sb() and init_src_var_info_of_4x4_sub_blocks():
        // the gradient and variance caches only serve the RD intra search)

        // Grade the temporal variation of the sb, the grade will be used to decide
        // fast mode search strategy for coding blocks
        grade_source_content_sb(cpi, x, tile_data, mi_row, mi_col);

        // encode the superblock
        debug_assert!(use_nonrd_mode != 0);
        encode_nonrd_sb(cpi, td, tile_data, mi_row, mi_col, seg_skip)?;

        // Update the top-right context in row_mt coding
        if update_cdf && tile_info.mi_row_end > mi_row + mib_size {
            let x = &mut td.mb;
            if sb_cols_in_tile == 1 {
                x.row_ctx[0] = (*x.e_mbd.tile_ctx).clone();
            } else if sb_col_in_tile >= 1 {
                x.row_ctx[sb_col_in_tile - 1] = (*x.e_mbd.tile_ctx).clone();
            }
        }
        mi_col += mib_size;
        sb_col_in_tile += 1;
    }
    Ok(())
}

fn init_encode_frame_mb_context(cpi: &Av1Comp, x: &mut Macroblock) {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);

    // Copy data over into macro block data structures.
    av1_setup_src_planes(x, cpi.source(), 0, 0, num_planes, cm.seq_params.sb_size);

    av1_setup_block_planes(
        &mut x.e_mbd,
        cm.seq_params.subsampling_x,
        cm.seq_params.subsampling_y,
        num_planes,
    );
}

pub(crate) fn av1_alloc_tile_data(cpi: &mut Av1Comp) {
    let cm = &cpi.common;
    let tile_cols = cm.tiles.cols;
    let tile_rows = cm.tiles.rows;

    cpi.tile_data = (0..tile_cols * tile_rows)
        .map(|_| TileDataEnc::default())
        .collect();
    cpi.allocated_tiles = tile_cols * tile_rows;
}

pub(crate) fn av1_init_tile_data(cpi: &mut Av1Comp) {
    let tile_cols = cpi.common.tiles.cols;
    let tile_rows = cpi.common.tiles.rows;

    // (no palette tokens: screen content tools are off)
    debug_assert!(!cpi.common.features.allow_screen_content_tools);
    let delay = delay_wait_for_top_right_sb(cpi);
    for tile_row in 0..tile_rows {
        for tile_col in 0..tile_cols {
            let mut tile_info = TileInfo::default();
            av1_tile_init(&mut tile_info, &cpi.common, tile_row, tile_col);
            let allow_update_cdf = cpi.common.tiles.large_scale == 0
                && !cpi.common.features.disable_cdf_update
                && !delay;
            let tctx = cpi.common.fc.clone();
            let tile_data = &mut cpi.tile_data[(tile_row * tile_cols + tile_col) as usize];
            tile_data.tile_info = tile_info;
            tile_data.abs_sum_level = 0;
            tile_data.allow_update_cdf = allow_update_cdf;
            tile_data.tctx = tctx;
        }
    }
}

/*\brief Encode a superblock row
 */
pub(crate) fn av1_encode_sb_row(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile_data: &TileDataEnc,
    mi_row: i32,
) -> Result<(), ()> {
    // (get_token_start() and populate_token_count(): no palette tokens)
    encode_sb_row(cpi, td, tile_data, mi_row)
}

/*\brief Encode a tile
 */
pub(crate) fn av1_encode_tile(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile_data: &TileDataEnc,
    tile_row: i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let tile_info = &tile_data.tile_info;

    debug_assert!(cpi.sf.rt_sf.use_nonrd_pick_mode != 0);

    av1_zero_above_context(
        cm,
        &mut td.mb.e_mbd.above_contexts,
        tile_info.mi_col_start,
        tile_info.mi_col_end,
        tile_row,
    );
    av1_init_above_context(tile_row, &mut td.mb.e_mbd);

    if cpi.oxcf.intra_mode_cfg.enable_cfl_intra {
        cfl_init(&mut td.mb.e_mbd.cfl, &cm.seq_params);
    }

    let mut mi_row = tile_info.mi_row_start;
    while mi_row < tile_info.mi_row_end {
        av1_encode_sb_row(cpi, td, tile_data, mi_row)?;
        mi_row += cm.seq_params.mib_size;
    }
    Ok(())
}

/*\brief Break one frame into tiles and encode the tiles
 */
fn encode_tiles(cpi: &mut Av1Comp, td: &mut ThreadData) -> Result<(), ()> {
    let tile_cols = cpi.common.tiles.cols;
    let tile_rows = cpi.common.tiles.rows;

    if cpi.allocated_tiles < tile_cols * tile_rows {
        av1_alloc_tile_data(cpi);
    }

    av1_init_tile_data(cpi);
    av1_alloc_mb_data(cpi, &mut td.mb)?;

    for tile_row in 0..tile_rows {
        for tile_col in 0..tile_cols {
            let i = (tile_row * cpi.common.tiles.cols + tile_col) as usize;
            td.intrabc_used = 0;
            td.deltaq_used = 0;
            td.abs_sum_level = 0;
            td.rd_counts.seg_tmp_pred_cost = [0; 2];
            // (td->mb.e_mbd.tile_ctx = &this_tile->tctx: the tile's context
            // is moved in while the tile is coded)
            std::mem::swap(&mut td.mb.e_mbd.tile_ctx, &mut cpi.tile_data[i].tctx);
            av1_init_rtc_counters(&mut td.mb);
            let res = av1_encode_tile(cpi, td, &cpi.tile_data[i], tile_row);
            std::mem::swap(&mut td.mb.e_mbd.tile_ctx, &mut cpi.tile_data[i].tctx);
            res?;
            cpi.tile_data[i].abs_sum_level = td.abs_sum_level;
            if !frame_is_intra_only(&cpi.common) {
                av1_accumulate_rtc_counters(cpi, &td.mb);
            }
            cpi.intrabc_used |= td.intrabc_used;
            cpi.deltaq_used |= td.deltaq_used;
        }
    }

    av1_dealloc_mb_data(&mut td.mb);
    Ok(())
}

// Set the relative distance of a reference frame w.r.t. current frame
fn set_rel_frame_dist(
    cm: &Av1Common,
    ref_frame_dist_info: &mut RefFrameDistanceInfo,
    ref_frame_flags: i32,
) {
    let mut min_past_dist = i32::MAX;
    let mut min_future_dist = i32::MAX;
    ref_frame_dist_info.nearest_past_ref = NONE_FRAME as i8;
    ref_frame_dist_info.nearest_future_ref = NONE_FRAME as i8;
    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        ref_frame_dist_info.ref_relative_dist[(ref_frame - LAST_FRAME) as usize] = 0;
        if ref_frame_flags & AV1_REF_FRAME_FLAG_LIST[ref_frame as usize] != 0 {
            let dist = av1_encoder_get_relative_dist(
                cm.cur_frame().ref_display_order_hint[(ref_frame - LAST_FRAME) as usize] as i32,
                cm.current_frame.display_order_hint as i32,
            );
            ref_frame_dist_info.ref_relative_dist[(ref_frame - LAST_FRAME) as usize] = dist;
            // Get the nearest ref_frame in the past
            if dist.abs() < min_past_dist && dist < 0 {
                ref_frame_dist_info.nearest_past_ref = ref_frame as i8;
                min_past_dist = dist.abs();
            }
            // Get the nearest ref_frame in the future
            if dist < min_future_dist && dist > 0 {
                ref_frame_dist_info.nearest_future_ref = ref_frame as i8;
                min_future_dist = dist;
            }
        }
    }
}

#[inline]
fn refs_are_one_sided(cm: &Av1Common) -> i32 {
    debug_assert!(!frame_is_intra_only(cm));

    let mut one_sided_refs = 1;
    let cur_display_order_hint = cm.current_frame.display_order_hint as i32;
    for ref_ in LAST_FRAME..=ALTREF_FRAME {
        let Some(buf) = get_ref_frame_buf(cm, ref_) else {
            continue;
        };
        if av1_encoder_get_relative_dist(
            cm.buffer_pool.frame_bufs[buf].display_order_hint as i32,
            cur_display_order_hint,
        ) > 0
        {
            one_sided_refs = 0; // bwd reference
            break;
        }
    }
    one_sided_refs
}

#[inline]
fn get_skip_mode_ref_offsets(cm: &Av1Common, ref_order_hint: &mut [i32; 2]) -> Result<(), ()> {
    let skip_mode_info = &cm.current_frame.skip_mode_info;
    ref_order_hint[0] = 0;
    ref_order_hint[1] = 0;
    if skip_mode_info.skip_mode_allowed == 0 {
        return Ok(());
    }

    let buf_0 = get_ref_frame_buf(cm, LAST_FRAME + skip_mode_info.ref_frame_idx_0);
    let buf_1 = get_ref_frame_buf(cm, LAST_FRAME + skip_mode_info.ref_frame_idx_1);
    let (Some(buf_0), Some(buf_1)) = (buf_0, buf_1) else {
        return Err(());
    };

    ref_order_hint[0] = cm.buffer_pool.frame_bufs[buf_0].order_hint as i32;
    ref_order_hint[1] = cm.buffer_pool.frame_bufs[buf_1].order_hint as i32;
    Ok(())
}

fn check_skip_mode_enabled(cpi: &mut Av1Comp) -> Result<i32, ()> {
    let cm = &mut cpi.common;

    av1_setup_skip_mode_allowed(cm);
    if cm.current_frame.skip_mode_info.skip_mode_allowed == 0 {
        return Ok(0);
    }

    // Turn off skip mode if the temporal distances of the reference pair to the
    // current frame are different by more than 1 frame.
    let cur_offset = cm.current_frame.order_hint as i32;
    let mut ref_offset = [0i32; 2];
    get_skip_mode_ref_offsets(cm, &mut ref_offset)?;
    let cur_to_ref0 = get_relative_dist(&cm.seq_params.order_hint_info, cur_offset, ref_offset[0]);
    let cur_to_ref1 =
        get_relative_dist(&cm.seq_params.order_hint_info, cur_offset, ref_offset[1]).abs();
    if (cur_to_ref0 - cur_to_ref1).abs() > 1 {
        return Ok(0);
    }

    // High Latency: Turn off skip mode if all refs are fwd.
    if cpi.all_one_sided_refs != 0 && cpi.oxcf.gf_cfg.lag_in_frames > 0 {
        return Ok(0);
    }

    static FLAG_LIST: [i32; REF_FRAMES as usize] = [
        0,
        AOM_LAST_FLAG,
        AOM_LAST2_FLAG,
        AOM_LAST3_FLAG,
        AOM_GOLD_FLAG,
        AOM_BWD_FLAG,
        AOM_ALT2_FLAG,
        AOM_ALT_FLAG,
    ];
    let skip_mode_info = &cpi.common.current_frame.skip_mode_info;
    let ref_frame = [
        (skip_mode_info.ref_frame_idx_0 + LAST_FRAME) as usize,
        (skip_mode_info.ref_frame_idx_1 + LAST_FRAME) as usize,
    ];
    if cpi.ref_frame_flags & FLAG_LIST[ref_frame[0]] == 0
        || cpi.ref_frame_flags & FLAG_LIST[ref_frame[1]] == 0
    {
        return Ok(0);
    }

    Ok(1)
}

fn setup_prune_ref_frame_mask(cpi: &mut Av1Comp) -> Result<(), ()> {
    if (!cpi.oxcf.ref_frm_cfg.enable_onesided_comp || cpi.sf.inter_sf.disable_onesided_comp != 0)
        && cpi.all_one_sided_refs != 0
    {
        // Disable all compound references
        cpi.prune_ref_frame_mask = (1 << MODE_CTX_REF_FRAMES) - (1 << REF_FRAMES);
    } else if cpi.sf.rt_sf.use_nonrd_pick_mode == 0 && cpi.sf.inter_sf.selective_ref_frame >= 2 {
        // (the RD pick mode's pruning of compound references)
        return Err(());
    }
    Ok(())
}

/// Translation of `allow_deltaq_mode()`.
fn allow_deltaq_mode(cpi: &Av1Comp) -> bool {
    let cm = &cpi.common;
    let sb_size = cm.seq_params.sb_size;
    let sbs_wide = MI_SIZE_WIDE[sb_size as usize] as i32;
    let sbs_high = MI_SIZE_HIGH[sb_size as usize] as i32;

    let mut delta_rdcost: i64 = 0;
    let mut mi_row = 0;
    while mi_row < cm.mi_params.mi_rows {
        let mut mi_col = 0;
        while mi_col < cm.mi_params.mi_cols {
            let mut this_delta_rdcost: i64 = 0;
            av1_get_q_for_deltaq_objective(cpi, &mut this_delta_rdcost, sb_size, mi_row, mi_col);
            delta_rdcost += this_delta_rdcost;
            mi_col += sbs_wide;
        }
        mi_row += sbs_high;
    }
    delta_rdcost < 0
}

/// `FORCE_ZMV_SKIP_128X128_BLK_DIFF`
const FORCE_ZMV_SKIP_128X128_BLK_DIFF: u32 = 10000;
/// `FORCE_ZMV_SKIP_MAX_PER_PIXEL_DIFF`
const FORCE_ZMV_SKIP_MAX_PER_PIXEL_DIFF: u32 = 4;

// Populates block level thresholds for force zeromv-skip decision
fn populate_thresh_to_force_zeromv_skip(cpi: &mut Av1Comp) {
    if cpi.sf.rt_sf.part_early_exit_zeromv == 0 {
        return;
    }

    // Threshold for forcing zeromv-skip decision is as below:
    // For 128x128 blocks, threshold is 10000 and per pixel threshold is 0.6103.
    // For 64x64 blocks, threshold is 5000 and per pixel threshold is 1.221
    // allowing slightly higher error for smaller blocks.
    // Per Pixel Threshold of 64x64 block        Area of 64x64 block         1  1
    // ------------------------------------=sqrt(---------------------)=sqrt(-)=-
    // Per Pixel Threshold of 128x128 block      Area of 128x128 block       4  2
    // Thus, per pixel thresholds for blocks of size 32x32, 16x16,...  can be
    // chosen as 2.442, 4.884,.... As the per pixel error tends to be higher for
    // small blocks, the same is clipped to 4.
    let thresh_exit_128x128_part = FORCE_ZMV_SKIP_128X128_BLK_DIFF;
    let num_128x128_pix = BLOCK_SIZE_WIDE[BLOCK_128X128 as usize] as i32
        * BLOCK_SIZE_HIGH[BLOCK_128X128 as usize] as i32;

    for bsize in BLOCK_4X4..BLOCK_SIZES_ALL {
        let num_block_pix =
            BLOCK_SIZE_WIDE[bsize as usize] as i32 * BLOCK_SIZE_HIGH[bsize as usize] as i32;

        // Calculate the threshold for zeromv-skip decision based on area of the
        // partition
        let mut thresh_exit_part_blk = (thresh_exit_128x128_part as f64
            * (num_block_pix as f64 / num_128x128_pix as f64).sqrt()
            + 0.5) as u32;
        thresh_exit_part_blk =
            thresh_exit_part_blk.min(FORCE_ZMV_SKIP_MAX_PER_PIXEL_DIFF * num_block_pix as u32);
        cpi.zeromv_skip_thresh_exit_part[bsize as usize] = thresh_exit_part_blk;
    }
}

/*\brief Encoder setup(only for the current frame), encoding, and recontruction
 * for a single frame
 */
fn encode_frame_internal(cpi: &mut Av1Comp) -> Result<(), ()> {
    // (td is &cpi->td, taken out of the compressor; the frame-wide state is
    // moved into it while the frame is coded)
    let mut td = std::mem::take(&mut cpi.td);
    enc_move_frame_state_in(cpi, &mut td);
    let res = encode_frame_internal_td(cpi, &mut td);
    enc_move_frame_state_out(cpi, &mut td);
    cpi.td = td;
    res?;
    if cpi.mt_info.pipelined_lpf_pending {
        // (launch_loop_filter_rows(): libaom's encoding workers filter the
        // rows once they and the rows below them are coded; the filter's
        // output does not depend on the order, and the rows' coding never
        // reads the filtered pixels)
        cpi.mt_info.pipelined_lpf_pending = false;
        let num_planes = av1_num_planes(&cpi.common);
        let lpf_opt_level = get_lpf_opt_level(&cpi.sf);
        debug_assert!(lpf_opt_level == 2);
        let lossless = cpi.td.mb.e_mbd.lossless;
        av1_loop_filter_frame(
            &mut cpi.common,
            &lossless,
            0,
            num_planes,
            false,
            lpf_opt_level,
        )?;
    }
    Ok(())
}

fn encode_frame_internal_td(cpi: &mut Av1Comp, td: &mut ThreadData) -> Result<(), ()> {
    let deltaq_mode = cpi.oxcf.q_cfg.deltaq_mode;

    if cpi.sf.rt_sf.use_nonrd_pick_mode == 0 {
        // (mi_params->setup_mi(): the RD pick mode)
        return Err(());
    }

    set_mi_offsets(&cpi.common.mi_params, &mut td.mb.e_mbd, 0, 0);

    td.counts = FrameCounts::default();
    td.rd_counts.tx_type_used = [[0; TX_TYPES as usize]; TX_SIZES_ALL as usize];
    td.rd_counts.obmc_used = [[0; 2]; BLOCK_SIZES_ALL as usize];
    td.rd_counts.warped_used = [0; 2];
    td.rd_counts.seg_tmp_pred_cost = [0; 2];

    // Reset the flag.
    cpi.intrabc_used = 0;
    // Need to disable intrabc when superres is selected
    if av1_superres_scaled(&cpi.common) {
        cpi.common.features.allow_intrabc = false;
    }

    cpi.common.features.allow_intrabc &= cpi.oxcf.kf_cfg.enable_intrabc;

    if cpi.common.features.allow_warped_motion && cpi.sf.inter_sf.prune_warped_prob_thresh > 0 {
        let update_type = get_frame_update_type(&cpi.ppi.gf_group, cpi.gf_frame_index as i32);
        let warped_probability = cpi.ppi.frame_probs.warped_probs[update_type as usize];
        if warped_probability < cpi.sf.inter_sf.prune_warped_prob_thresh {
            cpi.common.features.allow_warped_motion = false;
        }
    }

    // (av1_use_hash_me(): the hash-based motion estimation of the RD pick
    // mode)

    {
        let cm = &cpi.common;
        let quant_params = &cm.quant_params;
        let xd = &mut td.mb.e_mbd;
        if cm.seg.enabled != 0 {
            // (av1_get_qindex(): segmentation is off for the saver)
            return Err(());
        }
        for i in 0..MAX_SEGMENTS {
            let qindex = quant_params.base_qindex;
            xd.lossless[i] = (qindex == 0
                && quant_params.y_dc_delta_q == 0
                && quant_params.u_dc_delta_q == 0
                && quant_params.u_ac_delta_q == 0
                && quant_params.v_dc_delta_q == 0
                && quant_params.v_ac_delta_q == 0) as i32;
            if xd.lossless[i] != 0 {
                cpi.enc_seg.has_lossless_segment = 1;
            }
            xd.qindex[i] = qindex;
            if xd.lossless[i] != 0 {
                cpi.optimize_seg_arr[i] = NO_TRELLIS_OPT;
            } else {
                cpi.optimize_seg_arr[i] = cpi.sf.rd_sf.optimize_coefficients;
            }
        }
    }
    cpi.common.features.coded_lossless = is_coded_lossless(&cpi.common, &td.mb.e_mbd);
    cpi.common.features.all_lossless =
        cpi.common.features.coded_lossless && !av1_superres_scaled(&cpi.common);

    // Fix delta q resolution for the moment

    cpi.common.delta_q_info.delta_q_res = 0;
    debug_assert!(!cpi.use_ducky_encode);
    debug_assert!(cpi.oxcf.q_cfg.aq_mode != CYCLIC_REFRESH_AQ);
    {
        if deltaq_mode == DELTA_Q_OBJECTIVE {
            cpi.common.delta_q_info.delta_q_res = DEFAULT_DELTA_Q_RES_OBJECTIVE;
        } else {
            // (DELTA_Q_PERCEPTUAL and the perceptual AI modes: the saver's
            // delta q mode is the objective one, libaom's all intra default)
            debug_assert!(deltaq_mode == NO_DELTA_Q || deltaq_mode == DELTA_Q_OBJECTIVE);
        }
        cpi.common.delta_q_info.delta_lf_res = DEFAULT_DELTA_LF_RES;
        cpi.common.delta_q_info.delta_q_present_flag = (deltaq_mode != NO_DELTA_Q) as i32;

        // Turn off cm->delta_q_info.delta_q_present_flag if objective delta_q
        // is used for ineligible frames. That effectively will turn off row_mt
        // usage. Note objective delta_q and tpl eligible frames are only altref
        // frames currently.
        let gf_group = &cpi.ppi.gf_group;
        if cpi.common.delta_q_info.delta_q_present_flag != 0 {
            if deltaq_mode == DELTA_Q_OBJECTIVE
                && gf_group.update_type[cpi.gf_frame_index as usize] == LF_UPDATE
            {
                cpi.common.delta_q_info.delta_q_present_flag = 0;
            }

            if deltaq_mode == DELTA_Q_OBJECTIVE && cpi.common.delta_q_info.delta_q_present_flag != 0
            {
                cpi.common.delta_q_info.delta_q_present_flag &= allow_deltaq_mode(cpi) as i32;
            }
        }

        // Reset delta_q_used flag
        cpi.deltaq_used = 0;

        cpi.common.delta_q_info.delta_lf_present_flag =
            (cpi.common.delta_q_info.delta_q_present_flag != 0
                && cpi.oxcf.tool_cfg.enable_deltalf_mode) as i32;
        cpi.common.delta_q_info.delta_lf_multi = DEFAULT_DELTA_LF_MULTI;

        // update delta_q_present_flag and delta_lf_present_flag based on
        // base_qindex
        let base_q_positive = (cpi.common.quant_params.base_qindex > 0) as i32;
        cpi.common.delta_q_info.delta_q_present_flag &= base_q_positive;
        cpi.common.delta_q_info.delta_lf_present_flag &= base_q_positive;
    }
    // (the saver's frames never code delta q: the TPL statistics the
    // objective needs are never there)
    debug_assert!(cpi.common.delta_q_info.delta_q_present_flag == 0);

    av1_frame_init_quantizer(cpi, &mut td.mb);
    init_encode_frame_mb_context(cpi, &mut td.mb);
    // (set_default_interp_skip_flags(): the interpolation filter search is an
    // inter frame one)

    // (cm->last_frame_seg_map: segmentation is off)
    {
        let cm = &mut cpi.common;
        if cm.features.allow_intrabc || cm.features.coded_lossless {
            av1_set_default_ref_deltas(&mut cm.lf.ref_deltas);
            av1_set_default_mode_deltas(&mut cm.lf.mode_deltas);
        } else if let Some(prev) = cm.prev_frame {
            let pool = &cm.buffer_pool.frame_bufs;
            cm.lf.ref_deltas = pool[prev].ref_deltas;
            cm.lf.mode_deltas = pool[prev].mode_deltas;
        }
        let (ref_deltas, mode_deltas) = (cm.lf.ref_deltas, cm.lf.mode_deltas);
        cm.cur_frame_mut().ref_deltas = ref_deltas;
        cm.cur_frame_mut().mode_deltas = mode_deltas;
    }

    cpi.all_one_sided_refs = if frame_is_intra_only(&cpi.common) {
        0
    } else {
        refs_are_one_sided(&cpi.common)
    };

    cpi.prune_ref_frame_mask = 0;
    // Figure out which ref frames can be skipped at frame level.
    setup_prune_ref_frame_mask(cpi)?;

    td.mb.txfm_search_info.txb_split_count = 0;

    av1_compute_global_motion_facade(cpi)?;

    av1_calculate_ref_frame_side(&mut cpi.common);
    if cpi.common.features.allow_ref_frame_mvs {
        av1_setup_motion_field(&mut cpi.common);
    }

    cpi.common.current_frame.skip_mode_info.skip_mode_flag = check_skip_mode_enabled(cpi)?;

    // Initialization of skip mode cost depends on the value of
    // 'skip_mode_flag'. This initialization happens in the function
    // av1_fill_mode_rates(), which is in turn called in
    // av1_initialize_rd_consts(). Thus, av1_initialize_rd_consts()
    // has to be called after 'skip_mode_flag' is initialized.
    av1_initialize_rd_consts(cpi, td)?;
    av1_set_sad_per_bit(
        cpi,
        &mut td.mb.sadperbit,
        cpi.common.quant_params.base_qindex,
    );
    populate_thresh_to_force_zeromv_skip(cpi);

    // (the row synchronization: the rows are coded in order, see ethread.rs)
    cpi.mt_info.row_mt_enabled = false;
    cpi.mt_info.pack_bs_mt_enabled = cpi.mt_info.num_mod_workers[MOD_PACK_BS]
        .min(cpi.common.tiles.cols * cpi.common.tiles.rows)
        > 1;

    if cpi.oxcf.row_mt && cpi.mt_info.num_workers > 1 {
        cpi.mt_info.row_mt_enabled = true;
        av1_encode_tiles_row_mt(cpi, td)?;
    } else if cpi
        .mt_info
        .num_workers
        .min(cpi.common.tiles.cols * cpi.common.tiles.rows)
        > 1
    {
        // (av1_encode_tiles_mt(): libavif leaves row multi-threading on)
        return Err(());
    } else {
        // (td->rt_pc_root: the partition tree is implicit, see context_tree.rs)
        encode_tiles(cpi, td)?;
    }

    let cm = &mut cpi.common;
    // If intrabc is allowed but never selected, reset the allow_intrabc flag.
    if cm.features.allow_intrabc && cpi.intrabc_used == 0 {
        cm.features.allow_intrabc = false;
    }
    if cm.features.allow_intrabc {
        cm.delta_q_info.delta_lf_present_flag = 0;
    }

    if cm.delta_q_info.delta_q_present_flag != 0 && cpi.deltaq_used == 0 {
        cm.delta_q_info.delta_q_present_flag = 0;
    }

    let eval_type = if cpi.sf.winner_mode_sf.enable_winner_mode_for_tx_size_srch != 0 {
        WINNER_MODE_EVAL
    } else {
        DEFAULT_EVAL
    };
    let tx_search_type = cpi.winner_mode_params.tx_size_search_methods[eval_type as usize];
    debug_assert!(cpi.oxcf.txfm_cfg.enable_tx64 || tx_search_type != USE_LARGESTALL);
    let tx_mode = select_tx_mode(&cpi.common, tx_search_type);
    cpi.common.features.tx_mode = tx_mode;

    // Retain the frame level probability update conditions for parallel frames.
    // These conditions will be consumed during postencode stage to update the
    // probability.
    if cpi.ppi.gf_group.frame_parallel_level[cpi.gf_frame_index as usize] > 0 {
        // (no parallel frames)
        return Err(());
    }

    if cpi.sf.tx_sf.tx_type_search.prune_tx_type_using_stats != 0
        || ((cpi.sf.tx_sf.tx_type_search.fast_inter_tx_type_prob_thresh != i32::MAX)
            && (cpi.sf.tx_sf.tx_type_search.fast_inter_tx_type_prob_thresh != 0))
    {
        // (the transform type probabilities: they only steer the RD transform
        // type search)
        return Err(());
    }

    if cpi.common.seg.enabled != 0 {
        // (segmentation is off for the saver)
        return Err(());
    }

    if cpi.sf.inter_sf.prune_obmc_prob_thresh > 0
        && cpi.sf.inter_sf.prune_obmc_prob_thresh < i32::MAX
    {
        // (the OBMC probabilities: they only steer the RD motion mode search)
        return Err(());
    }

    if cpi.common.features.allow_warped_motion && cpi.sf.inter_sf.prune_warped_prob_thresh > 0 {
        let update_type = get_frame_update_type(&cpi.ppi.gf_group, cpi.gf_frame_index as i32);
        let update_warp_frameprobs = true;
        let mut sum = 0;
        for i in 0..2 {
            sum += td.rd_counts.warped_used[i];
        }
        let new_prob = if sum != 0 {
            128 * td.rd_counts.warped_used[1] / sum
        } else {
            0
        };
        // Track the frame probabilities of parallel encode frames to update
        // during postencode stage.
        // (no parallel frames)
        if update_warp_frameprobs {
            let p = &mut cpi.ppi.frame_probs.warped_probs[update_type as usize];
            *p = (*p + new_prob) >> 1;
        }
    }

    if cpi.common.current_frame.frame_type != KEY_FRAME
        && cpi.sf.interp_sf.adaptive_interp_filter_search == 2
        && cpi.common.features.interp_filter == SWITCHABLE
    {
        // (the switchable filter probabilities: they only steer the RD
        // interpolation filter search)
        return Err(());
    }
    Ok(())
}

/*\brief Setup reference frame buffers and encode a frame
 */
pub(crate) fn av1_encode_frame(cpi: &mut Av1Comp) -> Result<(), ()> {
    // Indicates whether or not to use a default reduced set for ext-tx
    // rather than the potential full set of 16 transforms
    cpi.common.features.reduced_tx_set_used = cpi.oxcf.txfm_cfg.reduced_tx_type_set;

    // Make sure segment_id is no larger than last_active_segid.
    if cpi.common.seg.enabled != 0 && cpi.common.seg.update_map != 0 {
        // (segmentation is off for the saver)
        return Err(());
    }

    av1_setup_frame_buf_refs(&mut cpi.common);
    enforce_max_ref_frames(cpi);
    let ref_frame_flags = cpi.ref_frame_flags;
    let mut ref_frame_dist_info = cpi.ref_frame_dist_info;
    set_rel_frame_dist(&cpi.common, &mut ref_frame_dist_info, ref_frame_flags);
    cpi.ref_frame_dist_info = ref_frame_dist_info;
    av1_setup_frame_sign_bias(&mut cpi.common);

    cpi.td.rd_counts.newmv_or_intra_blocks = 0;

    if cpi.sf.hl_sf.frame_parameter_update != 0 || cpi.sf.rt_sf.use_comp_ref_nonrd != 0 {
        if frame_is_intra_only(&cpi.common) {
            cpi.common.current_frame.reference_mode = SINGLE_REFERENCE;
        } else {
            cpi.common.current_frame.reference_mode = REFERENCE_MODE_SELECT;
        }

        cpi.common.features.interp_filter = SWITCHABLE;
        if cpi.common.tiles.large_scale != 0 {
            cpi.common.features.interp_filter = EIGHTTAP_REGULAR;
        }

        cpi.common.features.switchable_motion_mode = is_switchable_motion_mode_allowed(
            cpi.common.features.allow_warped_motion,
            cpi.oxcf.motion_mode_cfg.enable_obmc,
        );

        cpi.td.rd_counts.compound_ref_used_flag = 0;
        cpi.td.rd_counts.skip_mode_used_flag = 0;

        encode_frame_internal(cpi)?;

        let cm = &mut cpi.common;
        if cm.current_frame.reference_mode == REFERENCE_MODE_SELECT {
            // Use a flag that includes 4x4 blocks
            if cpi.td.rd_counts.compound_ref_used_flag == 0 {
                cm.current_frame.reference_mode = SINGLE_REFERENCE;
            }
        }
        // Re-check on the skip mode status as reference mode may have been
        // changed.
        if frame_is_intra_only(cm) || cm.current_frame.reference_mode == SINGLE_REFERENCE {
            let skip_mode_info = &mut cm.current_frame.skip_mode_info;
            skip_mode_info.skip_mode_allowed = 0;
            skip_mode_info.skip_mode_flag = 0;
        }
        if cm.current_frame.skip_mode_info.skip_mode_flag != 0
            && cpi.td.rd_counts.skip_mode_used_flag == 0
        {
            cm.current_frame.skip_mode_info.skip_mode_flag = 0;
        }

        if cm.tiles.large_scale == 0
            && cm.features.tx_mode == TX_MODE_SELECT
            && cpi.td.mb.txfm_search_info.txb_split_count == 0
        {
            cm.features.tx_mode = TX_MODE_LARGEST;
        }
    } else {
        // This is needed if real-time speed setting is changed on the fly
        // from one using compound prediction to one using single reference.
        if cpi.common.current_frame.reference_mode == REFERENCE_MODE_SELECT {
            cpi.common.current_frame.reference_mode = SINGLE_REFERENCE;
        }
        encode_frame_internal(cpi)?;
    }
    Ok(())
}
