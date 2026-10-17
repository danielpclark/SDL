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
use super::partition_search::{av1_nonrd_use_partition, av1_set_offsets};
use super::ratectrl::LF_UPDATE;
use super::rd::av1_initialize_rd_consts;
use super::rdopt_utils::select_tx_mode;
use super::speed_features::*;
use super::var_based_part::av1_choose_var_based_partitioning;
use crate::aom::aom_dsp::variance::aom_highbd_10_variance;
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;
use crate::aom::av1::common::av1_common_int::*;
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

pub(crate) fn av1_init_rtc_counters(x: &mut Macroblock) {
    // (av1_init_cyclic_refresh_counters())
    x.actual_num_seg1_blocks = 0;
    x.actual_num_seg2_blocks = 0;
    x.cnt_zeromv = 0;
}

static AV1_HIGH_VAR_OFFS_10: [u16; MAX_SB_SIZE as usize] = [128 * 4; MAX_SB_SIZE as usize];

/// Translation of `av1_get_perpixel_variance()` for the saver's 10-bit
/// buffers (`cpi->ppi->fn_ptr[plane_bsize].vf` is
/// `aom_highbd_10_variance<w>x<h>`).
pub(crate) fn av1_get_perpixel_variance(
    xd: &MacroBlockD,
    src: &[u16],
    r: &Buf2d,
    bsize: BlockSize,
    plane: usize,
    use_hbd: bool,
) -> u32 {
    let subsampling_x = xd.plane[plane].subsampling_x;
    let subsampling_y = xd.plane[plane].subsampling_y;
    let plane_bsize = get_plane_block_size(bsize, subsampling_x, subsampling_y);
    let mut sse: u32 = 0;
    debug_assert!(use_hbd);
    let bd = xd.bd;
    debug_assert!(bd == 10);
    let var = aom_highbd_10_variance(
        src,
        r.buf,
        r.stride as usize,
        &AV1_HIGH_VAR_OFFS_10,
        0,
        0,
        BLOCK_SIZE_WIDE[plane_bsize as usize] as i32,
        BLOCK_SIZE_HIGH[plane_bsize as usize] as i32,
        &mut sse,
    );
    ((var as u64 + ((1u64 << NUM_PELS_LOG2_LOOKUP[plane_bsize as usize]) >> 1))
        >> NUM_PELS_LOG2_LOOKUP[plane_bsize as usize]) as u32
}

pub(crate) fn av1_get_perpixel_variance_facade(
    _cpi: &Av1Comp,
    xd: &MacroBlockD,
    src: &[u16],
    r: &Buf2d,
    bsize: BlockSize,
    plane: usize,
) -> u32 {
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
        av1_choose_var_based_partitioning(cpi, tile_info, td, mi_row, mi_col);
    }
    debug_assert!(
        sf.part_sf.partition_search_type == FIXED_PARTITION
            || seg_skip != 0
            || sf.part_sf.partition_search_type == VAR_BASED_PARTITION
    );
    td.mb.cb_offset = [0, 0];

    // Initialize the flag to skip cdef to 1.
    // (skip_cdef_sb is a real-time feature)
    debug_assert!(sf.rt_sf.skip_cdef_sb == 0);
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

/*\brief Determine whether grading content can be skipped based on sad stat
 */
// (grade_source_content_sb(): returns at once for key frames)
#[inline]
fn grade_source_content_sb(cpi: &Av1Comp, x: &Macroblock) {
    let cm = &cpi.common;
    debug_assert!(cm.current_frame.frame_type == KEY_FRAME);
    debug_assert!(x.content_state_sb.source_sad_nonrd == K_MED_SAD);
    debug_assert!(x.content_state_sb.source_sad_rd == K_MED_SAD);
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
    // (reset_thresh_freq_fact(): the thresholds only steer inter mode search)

    // Code each SB in the row
    let mut mi_col = tile_info.mi_col_start;
    while mi_col < tile_info.mi_col_end {
        // In realtime/allintra mode and when frequency of cost updates is off/tile,
        // wait for the top superblock to finish encoding. Otherwise, wait for the
        // top-right superblock to finish encoding.
        // (one thread: the row synchronization never waits)
        let update_cdf = tile_data.allow_update_cdf && row_mt_enabled;
        debug_assert!(!update_cdf);
        av1_set_cost_upd_freq(cpi, td, tile_info, mi_row, mi_col);

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
        grade_source_content_sb(cpi, x);

        // encode the superblock
        debug_assert!(use_nonrd_mode != 0);
        encode_nonrd_sb(cpi, td, tile_data, mi_row, mi_col, seg_skip)?;
        mi_col += mib_size;
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
            debug_assert!(frame_is_intra_only(&cpi.common));
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

// (check_skip_mode_enabled(): skip mode is never allowed in an intra frame)
fn check_skip_mode_enabled(cpi: &mut Av1Comp) -> i32 {
    let cm = &mut cpi.common;

    av1_setup_skip_mode_allowed(cm);
    if cm.current_frame.skip_mode_info.skip_mode_allowed == 0 {
        return 0;
    }
    // (the rest compares the distances of the reference pair: inter frames
    // only)
    debug_assert!(false);
    0
}

// (setup_prune_ref_frame_mask(): no reference frames to prune in an intra
// frame)

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

// (populate_thresh_to_force_zeromv_skip(): part_early_exit_zeromv is a
// real-time feature)

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
    res
}

fn encode_frame_internal_td(cpi: &mut Av1Comp, td: &mut ThreadData) -> Result<(), ()> {
    let deltaq_mode = cpi.oxcf.q_cfg.deltaq_mode;

    debug_assert!(cpi.sf.rt_sf.use_nonrd_pick_mode != 0);
    set_mi_offsets(&cpi.common.mi_params, &mut td.mb.e_mbd, 0, 0);

    td.rd_counts.tx_type_used = [[0; TX_TYPES as usize]; TX_SIZES_ALL as usize];
    td.rd_counts.seg_tmp_pred_cost = [0; 2];

    // Reset the flag.
    cpi.intrabc_used = 0;
    // Need to disable intrabc when superres is selected
    if av1_superres_scaled(&cpi.common) {
        cpi.common.features.allow_intrabc = false;
    }

    cpi.common.features.allow_intrabc &= cpi.oxcf.kf_cfg.enable_intrabc;

    // (warped motion and hash-based motion estimation: inter frames only)
    debug_assert!(!cpi.common.features.allow_warped_motion);

    {
        let cm = &cpi.common;
        let quant_params = &cm.quant_params;
        let xd = &mut td.mb.e_mbd;
        for i in 0..MAX_SEGMENTS {
            debug_assert!(cm.seg.enabled == 0);
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

    debug_assert!(frame_is_intra_only(&cpi.common));
    cpi.all_one_sided_refs = 0;

    cpi.prune_ref_frame_mask = 0;
    // Figure out which ref frames can be skipped at frame level.
    // (setup_prune_ref_frame_mask(): none in an intra frame)

    td.mb.txfm_search_info.txb_split_count = 0;

    // (av1_compute_global_motion_facade(): no global motion in an intra
    // frame)

    av1_calculate_ref_frame_side(&mut cpi.common);
    debug_assert!(!cpi.common.features.allow_ref_frame_mvs);

    cpi.common.current_frame.skip_mode_info.skip_mode_flag = check_skip_mode_enabled(cpi);

    // Initialization of skip mode cost depends on the value of
    // 'skip_mode_flag'. This initialization happens in the function
    // av1_fill_mode_rates(), which is in turn called in
    // av1_initialize_rd_consts(). Thus, av1_initialize_rd_consts()
    // has to be called after 'skip_mode_flag' is initialized.
    av1_initialize_rd_consts(cpi, td);
    // (av1_set_sad_per_bit(): the SAD per bit only steers motion searches)
    // (populate_thresh_to_force_zeromv_skip(): a real-time feature)

    // (one thread: the dummy row synchronization, no row or tile
    // multi-threading)
    cpi.mt_info.row_mt_enabled = false;
    cpi.mt_info.pack_bs_mt_enabled = false;

    // (td->rt_pc_root: the partition tree is implicit, see context_tree.rs)
    encode_tiles(cpi, td)?;

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
    // (no parallel frames)

    // (the transform type probabilities of prune_tx_type_using_stats only
    // steer the RD transform type search)

    // (segmentation, OBMC, warped motion and interpolation filter
    // statistics: off, or inter frames only)
    Ok(())
}

/*\brief Setup reference frame buffers and encode a frame
 */
pub(crate) fn av1_encode_frame(cpi: &mut Av1Comp) -> Result<(), ()> {
    // Indicates whether or not to use a default reduced set for ext-tx
    // rather than the potential full set of 16 transforms
    cpi.common.features.reduced_tx_set_used = cpi.oxcf.txfm_cfg.reduced_tx_type_set;

    // Make sure segment_id is no larger than last_active_segid.
    debug_assert!(cpi.common.seg.enabled == 0);

    av1_setup_frame_buf_refs(&mut cpi.common);
    enforce_max_ref_frames(cpi);
    let ref_frame_flags = cpi.ref_frame_flags;
    let mut ref_frame_dist_info = cpi.ref_frame_dist_info;
    set_rel_frame_dist(&cpi.common, &mut ref_frame_dist_info, ref_frame_flags);
    cpi.ref_frame_dist_info = ref_frame_dist_info;
    av1_setup_frame_sign_bias(&mut cpi.common);

    cpi.td.rd_counts.newmv_or_intra_blocks = 0;

    debug_assert!(cpi.sf.hl_sf.frame_parameter_update != 0 || cpi.sf.rt_sf.use_comp_ref_nonrd != 0);
    {
        debug_assert!(frame_is_intra_only(&cpi.common));
        cpi.common.current_frame.reference_mode = SINGLE_REFERENCE;
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
    }
    Ok(())
}
