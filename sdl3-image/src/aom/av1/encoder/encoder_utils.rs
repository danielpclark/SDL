// Rust translation of the parts of av1/encoder/encoder_utils.c and
// av1/encoder/encoder_utils.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Encoder helpers: mode info sizing, frame setup and the per-frame
//! encoder state resets.

use crate::aom::aom_dsp::aom_dsp_common::{align_power_of_two, round_power_of_two};
use crate::aom::av1::common::alloccommon::calc_mi_size;
use crate::aom::av1::common::av1_common_int::CommonModeInfoParams;
use crate::aom::av1::common::enums::*;

#[inline]
pub(crate) fn size_in_mi(n: i32) -> i32 {
    align_power_of_two(n, 3) >> MI_SIZE_LOG2
}

#[inline]
pub(crate) fn set_mb_mi(mi_params: &mut CommonModeInfoParams, width: i32, height: i32) {
    mi_params.mi_cols = size_in_mi(width);
    mi_params.mi_rows = size_in_mi(height);
    mi_params.mi_stride = calc_mi_size(mi_params.mi_cols);

    mi_params.mb_cols = round_power_of_two(mi_params.mi_cols, 2);
    mi_params.mb_rows = round_power_of_two(mi_params.mi_rows, 2);
    mi_params.mbs = mi_params.mb_rows * mi_params.mb_cols;

    let mi_alloc_size_1d = MI_SIZE_WIDE[mi_params.mi_alloc_bsize as usize] as i32;
    mi_params.mi_alloc_stride = (mi_params.mi_stride + mi_alloc_size_1d - 1) / mi_alloc_size_1d;

    debug_assert!(
        MI_SIZE_WIDE[mi_params.mi_alloc_bsize as usize]
            == MI_SIZE_HIGH[mi_params.mi_alloc_bsize as usize]
    );
}

#[inline]
pub(crate) fn enc_free_mi(mi_params: &mut CommonModeInfoParams) {
    mi_params.mi_alloc = Vec::new();
    mi_params.mi_grid_base = Vec::new();
    mi_params.mi_alloc_size = 0;
    mi_params.tx_type_map = Vec::new();
}

#[inline]
pub(crate) fn enc_set_mb_mi(
    mi_params: &mut CommonModeInfoParams,
    width: i32,
    height: i32,
    min_partition_size: BlockSize,
) {
    mi_params.mi_alloc_bsize = min_partition_size;

    set_mb_mi(mi_params, width, height);
}

// ---------------------------------------------------------------------------
// encoder_utils.c
// ---------------------------------------------------------------------------

use super::encodeframe::av1_get_perpixel_variance_facade;
use super::encoder::*;
use super::ratectrl::{av1_rc_pick_q_and_bounds, FRAME_UPDATE_TYPES};
use super::rd::av1_set_rd_speed_thresholds;
use super::speed_features::*;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::Buf2d;
use crate::aom::av1::common::blockd::KEY_FRAME;
use crate::aom::av1::common::entropymode::av1_setup_past_independence;
use crate::aom::av1::common::filter::{
    EIGHTTAP_REGULAR, SWITCHABLE_FILTERS, SWITCHABLE_FILTER_CONTEXTS,
};
use crate::aom::av1::common::mv::DEFAULT_WARP_PARAMS;
use crate::aom::av1::common::seg_common::*;

/// `AM_SEGMENT_ID_INACTIVE` (encoder_utils.h)
const AM_SEGMENT_ID_INACTIVE: usize = 7;

/// Translation of `av1_apply_active_map()`: the active map is never
/// enabled (no `AOME_SET_ACTIVEMAP`), so it is only disabled again on
/// intra frames.
pub(crate) fn av1_apply_active_map(cpi: &mut Av1Comp) {
    if frame_is_intra_only(&cpi.common) {
        cpi.active_map.enabled = false;
        cpi.active_map.update = true;
    }

    if cpi.active_map.update {
        debug_assert!(!cpi.active_map.enabled);
        let seg = &mut cpi.common.seg;
        av1_disable_segfeature(seg, AM_SEGMENT_ID_INACTIVE, SEG_LVL_SKIP);
        av1_disable_segfeature(seg, AM_SEGMENT_ID_INACTIVE, SEG_LVL_ALT_LF_Y_H);
        av1_disable_segfeature(seg, AM_SEGMENT_ID_INACTIVE, SEG_LVL_ALT_LF_Y_V);
        av1_disable_segfeature(seg, AM_SEGMENT_ID_INACTIVE, SEG_LVL_ALT_LF_U);
        av1_disable_segfeature(seg, AM_SEGMENT_ID_INACTIVE, SEG_LVL_ALT_LF_V);
        if seg.enabled != 0 {
            seg.update_data = 1;
            seg.update_map = 1;
        }
        cpi.active_map.update = false;
    }
}

/// Translation of `av1_set_size_dependent_vars()` (no TPL statistics, no
/// fixed QP offsets).
pub(crate) fn av1_set_size_dependent_vars(
    cpi: &mut Av1Comp,
    q: &mut i32,
    bottom_index: &mut i32,
    top_index: &mut i32,
) {
    // Setup variables that depend on the dimensions of the frame.
    let speed = cpi.speed;
    av1_set_speed_features_framesize_dependent(cpi, speed);

    // (process_tpl_stats_frame(): the TPL model is off)
    debug_assert!(!cpi.oxcf.algo_cfg.enable_tpl_model || cpi.oxcf.gf_cfg.lag_in_frames == 0);
    let (w, h) = (cpi.common.width, cpi.common.height);
    let gf_frame_index = cpi.gf_frame_index as usize;
    *q = av1_rc_pick_q_and_bounds(cpi, w, h, gf_frame_index, bottom_index, top_index);

    // (the TPL based quantizer adjustment and fixed QP offsets: no TPL
    // statistics, no fixed offsets)
    debug_assert!(cpi.oxcf.q_cfg.use_fixed_qp_offsets == 0);

    // Configure experimental use of segmentation for enhanced coding of
    // static regions if indicated.
    // Only allowed in the second pass of a two pass encode, as it requires
    // lagged coding, and if the relevant speed feature flag is set.
    debug_assert!(!is_stat_consumption_stage_twopass(cpi));
}

/// Translation of `av1_select_sb_size()`.
pub(crate) fn av1_select_sb_size(
    oxcf: &Av1EncoderConfig,
    width: i32,
    height: i32,
    number_spatial_layers: i32,
) -> BlockSize {
    if oxcf.tool_cfg.superblock_size == AOM_SUPERBLOCK_SIZE_64X64 {
        return BLOCK_64X64;
    }
    if oxcf.tool_cfg.superblock_size == AOM_SUPERBLOCK_SIZE_128X128 {
        return BLOCK_128X128;
    }

    // Force 64x64 superblock size to increase resolution in perceptual
    // AQ mode.
    if oxcf.mode == ALLINTRA
        && (oxcf.q_cfg.deltaq_mode == DELTA_Q_PERCEPTUAL_AI
            || oxcf.q_cfg.deltaq_mode == DELTA_Q_USER_RATING_BASED)
    {
        return BLOCK_64X64;
    }
    debug_assert!(oxcf.tool_cfg.superblock_size == AOM_SUPERBLOCK_SIZE_DYNAMIC);

    if number_spatial_layers > 1 || oxcf.resize_cfg.resize_mode != RESIZE_NONE {
        // Use the configured size (top resolution) for spatial layers or
        // on resize.
        return if oxcf.frm_dim_cfg.width.min(oxcf.frm_dim_cfg.height) > 720 {
            BLOCK_128X128
        } else {
            BLOCK_64X64
        };
    } else if oxcf.mode == REALTIME {
        if oxcf.tune_cfg.content == AOM_CONTENT_SCREEN {
            return if width.min(height) >= 720 {
                BLOCK_128X128
            } else {
                BLOCK_64X64
            };
        } else {
            return if width.min(height) > 720 {
                BLOCK_128X128
            } else {
                BLOCK_64X64
            };
        }
    }

    // TODO(any): Possibly could improve this with a heuristic.
    // When superres / resize is on, 'cm->width / height' can change between
    // calls, so we don't apply this heuristic there.
    // Things break if superblock size changes between the first pass and second
    // pass encoding, which is why this heuristic is not configured as a
    // speed-feature.
    if oxcf.superres_cfg.superres_mode == AOM_SUPERRES_NONE
        && oxcf.resize_cfg.resize_mode == RESIZE_NONE
    {
        let is_480p_or_lesser = width.min(height) <= 480;
        if oxcf.speed >= 1 && is_480p_or_lesser {
            return BLOCK_64X64;
        }

        // For 1080p and lower resolutions, choose SB size adaptively based on
        // resolution and speed level for multi-thread encode.
        let is_1080p_or_lesser = width.min(height) <= 1080;
        if !is_480p_or_lesser
            && is_1080p_or_lesser
            && oxcf.mode == GOOD
            && oxcf.row_mt
            && oxcf.max_threads > 1
            && oxcf.speed >= 5
        {
            return BLOCK_64X64;
        }
    }
    BLOCK_128X128
}

/// Translation of `av1_setup_frame()` (key frames only).
pub(crate) fn av1_setup_frame(cpi: &mut Av1Comp) {
    let cm = &mut cpi.common;
    // Set up entropy context depending on frame type. The decoder mandates
    // the use of the default context, index 0, for keyframes and inter
    // frames where the error_resilient_mode or intra_only flag is set. For
    // other inter-frames the encoder currently uses only two contexts;
    // context 1 for ALTREF frames and context 0 for the others.

    if frame_is_intra_only(cm)
        || cm.features.error_resilient_mode
        || cpi.ext_flags.use_primary_ref_none
    {
        av1_setup_past_independence(cm);
    }

    if (cm.current_frame.frame_type == KEY_FRAME && cm.show_frame != 0) || frame_is_sframe(cm) {
        if !cpi.ppi.seq_params_locked {
            let sb_size = av1_select_sb_size(
                &cpi.oxcf,
                cm.width,
                cm.height,
                cpi.ppi.number_spatial_layers,
            );
            set_sb_size(&mut cm.seq_params, sb_size);
        }
    } else {
        match get_primary_ref_frame_buf(cm) {
            None => {
                // Reset context to default settings
                av1_setup_past_independence(cm);
            }
            Some(primary_ref_buf) => {
                *cm.fc = (*cm.buffer_pool.frame_bufs[primary_ref_buf].frame_context).clone();
            }
        }
    }

    cm.cur_frame_mut().interp_filter_selected = [0; SWITCHABLE as usize];
    cm.prev_frame = get_primary_ref_frame_buf(cm);
    // (cpi->vaq_refresh: variance AQ is off)
}

/// Translation of `fix_interp_filter()`.
fn fix_interp_filter(interp_filter: &mut i32, counts: &FrameCounts) {
    if *interp_filter == SWITCHABLE {
        // Check to see if only one of the filters is actually used
        let mut count = [0u32; SWITCHABLE_FILTERS as usize];
        let mut num_filters_used = 0;
        for i in 0..SWITCHABLE_FILTERS as usize {
            for j in 0..SWITCHABLE_FILTER_CONTEXTS {
                count[i] += counts.switchable_interp[j][i];
            }
            num_filters_used += (count[i] > 0) as i32;
        }
        if num_filters_used == 1 {
            // Only one filter is used. So set the filter at frame level
            for i in 0..SWITCHABLE_FILTERS as usize {
                if count[i] != 0 {
                    if i as i32 == EIGHTTAP_REGULAR {
                        *interp_filter = i as i32;
                    }
                    break;
                }
            }
        }
    }
}

/// Translation of `av1_finalize_encoded_frame()` (no shown existing frame,
/// no film grain).
pub(crate) fn av1_finalize_encoded_frame(cpi: &mut Av1Comp) {
    let cm = &cpi.common;

    debug_assert!(!encode_show_existing_frame(cm));
    debug_assert!(cm.seq_params.film_grain_params_present == 0);

    for tile_col in 0..cm.tiles.cols {
        for tile_row in 0..cm.tiles.rows {
            let tile_idx = (tile_row * cm.tiles.cols + tile_col) as usize;
            *cpi.tile_data[tile_idx].tctx = (*cm.fc).clone();
        }
    }

    let mut interp_filter = cpi.common.features.interp_filter;
    fix_interp_filter(&mut interp_filter, &cpi.td.counts);
    cpi.common.features.interp_filter = interp_filter;
}

/// Translation of `av1_set_mb_ssim_rdmult_scaling()`.
pub(crate) fn av1_set_mb_ssim_rdmult_scaling(cpi: &mut Av1Comp) {
    let mut factors = std::mem::take(&mut cpi.ssim_rdmult_scaling_factors);
    let mi_params = &cpi.common.mi_params;
    let xd = &cpi.td.mb.e_mbd;
    let src = &cpi.source().buffer_alloc;
    let y_buffer = cpi.source().buffers[0];
    let y_stride = cpi.source().y_stride();
    let block_size = BLOCK_16X16;

    let num_mi_w = MI_SIZE_WIDE[block_size as usize] as i32;
    let num_mi_h = MI_SIZE_HIGH[block_size as usize] as i32;
    let num_cols = (mi_params.mi_cols + num_mi_w - 1) / num_mi_w;
    let num_rows = (mi_params.mi_rows + num_mi_h - 1) / num_mi_h;
    let mut log_sum = 0.0f64;

    // Loop through each 16x16 block.
    for row in 0..num_rows {
        for col in 0..num_cols {
            let mut var = 0.0f64;
            let mut num_of_var = 0.0f64;
            let index = (row * num_cols + col) as usize;

            // Loop through each 8x8 block.
            let mut mi_row = row * num_mi_h;
            while mi_row < mi_params.mi_rows && mi_row < (row + 1) * num_mi_h {
                let mut mi_col = col * num_mi_w;
                while mi_col < mi_params.mi_cols && mi_col < (col + 1) * num_mi_w {
                    let row_offset_y = mi_row << 2;
                    let col_offset_y = mi_col << 2;

                    let buf = Buf2d {
                        buf: y_buffer + (row_offset_y * y_stride + col_offset_y) as usize,
                        stride: y_stride,
                        ..Buf2d::default()
                    };

                    var += av1_get_perpixel_variance_facade(
                        cpi,
                        xd,
                        src,
                        &buf,
                        BLOCK_8X8,
                        AOM_PLANE_Y,
                    ) as f64;
                    num_of_var += 1.0;
                    mi_col += 2;
                }
                mi_row += 2;
            }
            var /= num_of_var;

            // Curve fitting with an exponential model on all 16x16 blocks from the
            // midres dataset.
            var = 67.035434 * (1.0 - (-0.0021489 * var).exp()) + 17.492222;
            factors[index] = var;
            log_sum += var.ln();
        }
    }
    log_sum = (log_sum / (num_rows * num_cols) as f64).exp();

    for row in 0..num_rows {
        for col in 0..num_cols {
            let index = (row * num_cols + col) as usize;
            factors[index] /= log_sum;
        }
    }
    cpi.ssim_rdmult_scaling_factors = factors;
}

// ---------------------------------------------------------------------------
// encoder_utils.h (continued)
// ---------------------------------------------------------------------------

/// Translation of `set_size_independent_vars()`.
#[inline]
pub(crate) fn set_size_independent_vars(cpi: &mut Av1Comp) {
    for i in LAST_FRAME..=ALTREF_FRAME {
        cpi.common.global_motion[i as usize] = DEFAULT_WARP_PARAMS;
    }
    cpi.gm_info.search_done = false;

    let speed = cpi.speed;
    av1_set_speed_features_framesize_independent(cpi, speed);
    av1_set_rd_speed_thresholds(cpi);
    let enable_obmc = cpi.oxcf.motion_mode_cfg.enable_obmc;
    let features = &mut cpi.common.features;
    features.interp_filter = SWITCHABLE;
    features.switchable_motion_mode =
        is_switchable_motion_mode_allowed(features.allow_warped_motion, enable_obmc);
}

/// Translation of `release_scaled_references()`.
#[inline]
pub(crate) fn release_scaled_references(cpi: &mut Av1Comp) {
    // TODO(isbs): only refresh the necessary frames, rather than all of them
    for i in 0..INTER_REFS_PER_FRAME as usize {
        if let Some(buf) = cpi.scaled_ref_buf[i].take() {
            cpi.common.buffer_pool.frame_bufs[buf].ref_count -= 1;
        }
    }
}

/// Translation of `reduce_num_ref_buffers()`.
#[inline]
fn reduce_num_ref_buffers(cpi: &Av1Comp) -> bool {
    let seq_params = &cpi.common.seq_params;
    is_one_pass_rt_params(cpi)
        && use_rtc_reference_structure_one_layer(cpi)
        && seq_params.order_hint_info.enable_order_hint == 0
        && cpi.rt_reduce_num_ref_buffers
}

// Refresh reference frame buffers according to refresh_frame_flags.
/// Translation of `refresh_reference_frames()`.
#[inline]
pub(crate) fn refresh_reference_frames(cpi: &mut Av1Comp) {
    // All buffers are refreshed for shown keyframes and S-frames.
    // In case of RT, golden frame refreshes the 6th slot and other reference
    // frames refresh slots 0 to 5. Slot 7 is not refreshed by any reference
    // frame. Thus, only 7 buffers are refreshed for keyframes and S-frames
    // instead of 8.
    let mut num_ref_buffers = REF_FRAMES;
    if reduce_num_ref_buffers(cpi) {
        num_ref_buffers -= 1;
    }
    let cm = &mut cpi.common;
    for ref_frame in 0..num_ref_buffers {
        if ((cm.current_frame.refresh_frame_flags >> ref_frame) & 1) == 1 {
            if let Some(cur) = cm.cur_frame {
                assign_frame_buffer_p(
                    &mut cm.buffer_pool,
                    &mut cm.ref_frame_map[ref_frame as usize],
                    cur,
                );
            }
        }
    }
}

/// `default_warped_probs[]`
const DEFAULT_WARPED_PROBS: [i32; FRAME_UPDATE_TYPES] = [64, 64, 64, 64, 64, 64, 64];

/// Translation of `copy_frame_prob_info()`.
#[inline]
pub(crate) fn copy_frame_prob_info(cpi: &mut Av1Comp) {
    let frame_probs = &mut cpi.ppi.frame_probs;
    // (the transform type, OBMC and interpolation filter probabilities are
    // not used by the real-time speed features)
    debug_assert!(cpi.sf.tx_sf.tx_type_search.prune_tx_type_using_stats == 0);
    debug_assert!(
        !(cpi.sf.inter_sf.prune_obmc_prob_thresh > 0
            && cpi.sf.inter_sf.prune_obmc_prob_thresh < i32::MAX)
    );
    if cpi.sf.inter_sf.prune_warped_prob_thresh > 0 {
        frame_probs.warped_probs = DEFAULT_WARPED_PROBS;
    }
    debug_assert!(cpi.sf.interp_sf.adaptive_interp_filter_search != 2);
}

/// Translation of `av1_scale_references()`: the references are never
/// scaled (no resizing), so the scaled references are the references.
pub(crate) fn av1_scale_references(cpi: &mut Av1Comp) {
    let no_stats = has_no_stats_stage(cpi);
    let cm = &mut cpi.common;

    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        // Need to convert from AOM_REFFRAME to index into ref_mask (subtract 1).
        if cpi.ref_frame_flags & AV1_REF_FRAME_FLAG_LIST[ref_frame as usize] != 0 {
            let Some(buf) = get_ref_frame_buf(cm, ref_frame) else {
                // (aom_internal_error(): "Inter ref is NULL")
                cpi.scaled_ref_buf[(ref_frame - 1) as usize] = None;
                continue;
            };
            let b = &mut cm.buffer_pool.frame_bufs[buf];
            debug_assert!(b.buf.y_crop_width() == cm.width && b.buf.y_crop_height() == cm.height);
            cpi.scaled_ref_buf[(ref_frame - 1) as usize] = Some(buf);
            b.ref_count += 1;
        } else if !no_stats {
            cpi.scaled_ref_buf[(ref_frame - 1) as usize] = None;
        }
    }
}
