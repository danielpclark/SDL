// Rust translation of the parts of av1/encoder/encode_strategy.c and
// encode_strategy.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches)
// the saver reaches: the strategy for a still picture, a single shown key
// frame that resets the reference buffers.
// Copyright (c) 2019, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encode strategy: which source frame is coded next, as which frame
//! type, with which reference buffer updates.
//!
//! The compressor of a still picture codes exactly one frame, the first
//! of its key frame group. `av1_get_second_pass_params()` (pass2_strategy.c)
//! then makes the group of that key frame alone: `frames_to_key` is
//! `AOMMAX(1, key_freq_max)`, 1; the group's frame 0 is a `KF_UPDATE` frame
//! at layer depth 0 that resets the reference buffers. Its results are set
//! here directly (`set_kf_group()`); the second pass statistics and the
//! group's bit allocation only steer the bitrate driven modes and later
//! frames.

use super::encoder::*;
use super::lookahead::*;
use super::ratectrl::*;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::{INTRA_ONLY_FRAME, KEY_FRAME, S_FRAME};
use crate::aom::av1::common::enums::*;

/// `SELECT_ALL_BUF_SLOTS`
const SELECT_ALL_BUF_SLOTS: i32 = 0xFF;
/// `TICKS_PER_SEC`
pub(crate) const TICKS_PER_SEC: i64 = 10000000;
/// `AOM_EFLAG_FORCE_KF`
pub(crate) const AOM_EFLAG_FORCE_KF: u32 = 1 << 0;

/// Translation of `set_refresh_frame_flags()`.
#[inline]
fn set_refresh_frame_flags(
    refresh_frame: &mut RefreshFrameInfo,
    refresh_gf: bool,
    refresh_bwdref: bool,
    refresh_arf: bool,
) {
    refresh_frame.golden_frame = refresh_gf;
    refresh_frame.bwd_ref_frame = refresh_bwdref;
    refresh_frame.alt_ref_frame = refresh_arf;
}

/// Translation of `av1_configure_buffer_updates()` (key frames).
pub(crate) fn av1_configure_buffer_updates(
    cpi: &mut Av1Comp,
    refresh_frame: &mut RefreshFrameInfo,
    update_type: u8,
    refbuf_state: i32,
    force_refresh_all: bool,
) {
    // NOTE(weitinglin): Should we define another function to take care of
    // cpi->rc.is_$Source_Type to make this function as it is in the comment?
    let _ = refbuf_state;
    cpi.rc.is_src_frame_alt_ref = 0;

    match update_type {
        KF_UPDATE => set_refresh_frame_flags(refresh_frame, true, true, true),
        _ => {
            // (the other update types are those of inter frames)
            debug_assert!(false);
        }
    }

    debug_assert!(!cpi.ext_flags.refresh_frame_update_pending);

    if force_refresh_all {
        set_refresh_frame_flags(refresh_frame, true, true, true);
    }
}

/// Translation of `set_additional_frame_flags()`.
fn set_additional_frame_flags(cm: &Av1Common, frame_flags: &mut u32) {
    if frame_is_intra_only(cm) {
        *frame_flags |= FRAMEFLAGS_INTRAONLY;
    }
    if frame_is_sframe(cm) {
        *frame_flags |= FRAMEFLAGS_SWITCH;
    }
    if cm.features.error_resilient_mode {
        *frame_flags |= FRAMEFLAGS_ERROR_RESILIENT;
    }
}

/// Translation of `set_ext_overrides()`.
#[inline]
fn set_ext_overrides(
    cm: &mut Av1Common,
    frame_params: &mut EncodeFrameParams,
    ext_flags: &ExternalFlags,
) {
    // Overrides the defaults with the externally supplied values with
    // av1_update_reference() and av1_update_entropy() calls
    // Note: The overrides are valid only for the next frame passed
    // to av1_encode_lowlevel()

    debug_assert!(!ext_flags.use_s_frame && !ext_flags.refresh_frame_context_pending);
    cm.features.allow_ref_frame_mvs = ext_flags.use_ref_frame_mvs;

    frame_params.error_resilient_mode = ext_flags.use_error_resilient;
    // A keyframe is already error resilient and keyframes with
    // error_resilient_mode interferes with the use of show_existing_frame
    // when forward reference keyframes are enabled.
    frame_params.error_resilient_mode &= frame_params.frame_type != KEY_FRAME;
    // For bitstream conformance, s-frames must be error-resilient
    frame_params.error_resilient_mode |= frame_params.frame_type == S_FRAME;
}

/// Translation of `choose_primary_ref_frame()` (intra frames).
fn choose_primary_ref_frame(cpi: &Av1Comp, frame_params: &EncodeFrameParams) -> i32 {
    let intra_only =
        frame_params.frame_type == KEY_FRAME || frame_params.frame_type == INTRA_ONLY_FRAME;
    if intra_only || frame_params.error_resilient_mode || cpi.ext_flags.use_primary_ref_none {
        return PRIMARY_REF_NONE;
    }
    // (the inter frame choice)
    debug_assert!(false);
    PRIMARY_REF_NONE
}

/// Translation of `adjust_frame_rate()` (the first frame).
fn adjust_frame_rate(cpi: &mut Av1Comp, ts_start: i64, ts_end: i64) {
    let this_duration;
    let mut step = 0;

    // Clear down mmx registers

    debug_assert!(!cpi.ppi.use_svc);
    if ts_start == cpi.time_stamps.first_ts_start {
        this_duration = ts_end - ts_start;
        step = 1;
    } else {
        // (later frames)
        debug_assert!(false);
        this_duration = ts_end - ts_start;
    }

    if this_duration != 0 {
        if step != 0 {
            cpi.new_framerate = 10000000.0 / this_duration as f64;
            let framerate = cpi.new_framerate;
            av1_new_framerate(cpi, framerate);
        } else {
            debug_assert!(false);
        }
    }

    cpi.time_stamps.prev_ts_start = ts_start;
    cpi.time_stamps.prev_ts_end = ts_end;
}

/// Translation of `choose_frame_source()` (the group's frame 0 is shown and
/// coded from the queue's first entry).
fn choose_frame_source(
    cpi: &Av1Comp,
    pop_lookahead: &mut bool,
    show_frame: &mut i32,
) -> Option<usize> {
    let gf_group = &cpi.ppi.gf_group;
    // Source index in lookahead buffer.
    let src_index = gf_group.arf_src_offset[cpi.gf_frame_index as usize] as i32;

    // TODO(Aasaipriya): Forced key frames need to be fixed when rc_mode != AOM_Q
    debug_assert!(src_index == 0);

    // Don't pop the lookahead if this is an ARF or the lookahead is disabled.
    *pop_lookahead = src_index == 0;
    debug_assert!(!is_stat_generation_stage(cpi));
    *show_frame = *pop_lookahead as i32;

    debug_assert!(gf_group.src_offset[cpi.gf_frame_index as usize] == 0);
    debug_assert!(cpi.common.current_frame.frame_number == 0);
    let lookahead = cpi.ppi.lookahead.as_ref()?;
    av1_lookahead_peek(lookahead, src_index, cpi.compressor_stage)
}

/// Translation of `update_frame_flags()`.
fn update_frame_flags(cm: &Av1Common, refresh_frame: &RefreshFrameInfo, frame_flags: &mut u32) {
    debug_assert!(!encode_show_existing_frame(cm));

    if refresh_frame.golden_frame {
        *frame_flags |= FRAMEFLAGS_GOLDEN;
    } else {
        *frame_flags &= !FRAMEFLAGS_GOLDEN;
    }

    if refresh_frame.alt_ref_frame {
        *frame_flags |= FRAMEFLAGS_ALTREF;
    } else {
        *frame_flags &= !FRAMEFLAGS_ALTREF;
    }

    if refresh_frame.bwd_ref_frame {
        *frame_flags |= FRAMEFLAGS_BWDREF;
    } else {
        *frame_flags &= !FRAMEFLAGS_BWDREF;
    }

    if cm.current_frame.frame_type == KEY_FRAME {
        *frame_flags |= FRAMEFLAGS_KEY;
    } else {
        *frame_flags &= !FRAMEFLAGS_KEY;
    }
}

/// Translation of `av1_get_refresh_frame_flags()` (a group reset).
fn av1_get_refresh_frame_flags(cpi: &Av1Comp, gf_index: usize) -> i32 {
    let gf_group = &cpi.ppi.gf_group;
    debug_assert!(gf_group.refbuf_state[gf_index] == REFBUF_RESET);
    SELECT_ALL_BUF_SLOTS
}

/// The results of `av1_get_second_pass_params()` for a still picture's
/// group (see the module documentation).
fn set_kf_group(cpi: &mut Av1Comp) {
    let gf_group = &mut cpi.ppi.gf_group;
    gf_group.size = 1;
    gf_group.update_type[0] = KF_UPDATE;
    gf_group.arf_src_offset[0] = 0;
    gf_group.src_offset[0] = 0;
    gf_group.layer_depth[0] = 0;
    gf_group.frame_parallel_level[0] = 0;
    gf_group.refbuf_state[0] = REFBUF_RESET;
    gf_group.is_frame_non_ref[0] = false;
    gf_group.max_layer_depth = 0;
    cpi.gf_frame_index = 0;
    cpi.rc.frames_to_key = 1.max(cpi.oxcf.kf_cfg.key_freq_max);
    cpi.rc.frames_since_key = 0;
}

/// Translation of `av1_encode_strategy()` for the first frame of a still
/// picture compressor: returns `None` (the C code's -1) when there is no
/// frame to code.
pub(crate) fn av1_encode_strategy(
    cpi: &mut Av1Comp,
    cpi_data: &mut Av1CompData,
    dest: &mut [u8],
) -> Result<Option<()>, ()> {
    let mut frame_params = EncodeFrameParams::default();
    let flush = cpi_data.flush;
    let mut frame_flags: u32 = 0;

    // Check if we need to stuff more src frames
    {
        let Some(lookahead) = cpi.ppi.lookahead.as_ref() else {
            return Ok(None);
        };
        if !flush {
            let srcbuf_size = av1_lookahead_depth(lookahead, cpi.compressor_stage) as i32;
            let pop_size = av1_lookahead_pop_sz(lookahead, cpi.compressor_stage);

            // Continue buffering look ahead buffer.
            if srcbuf_size < pop_size {
                return Ok(None);
            }
        }

        if av1_lookahead_peek(lookahead, 0, cpi.compressor_stage).is_none() {
            return Ok(None);
        }
    }

    // TODO(sarahparker) finish bit allocation for one pass pyramid
    if has_no_stats_stage(cpi) {
        let gf_cfg = &mut cpi.oxcf.gf_cfg;
        gf_cfg.gf_max_pyr_height = gf_cfg.gf_max_pyr_height.min(USE_ALTREF_FOR_ONE_PASS);
        gf_cfg.gf_min_pyr_height = gf_cfg.gf_min_pyr_height.min(gf_cfg.gf_max_pyr_height);
    }

    // Allocation of mi buffers.
    alloc_mb_mode_info_buffers(cpi)?;

    // (the TPL buffers: the TPL model is off)
    let use_one_pass_rt_params = is_one_pass_rt_params(cpi);
    debug_assert!(!use_one_pass_rt_params && !is_stat_generation_stage(cpi));
    // (av1_get_second_pass_params())
    set_kf_group(cpi);

    // (show existing frames: the group has none)
    frame_params.show_existing_frame = 0;

    let mut pop_lookahead = false;
    let Some(source) = choose_frame_source(cpi, &mut pop_lookahead, &mut frame_params.show_frame)
    else {
        // If no source was found, we can't encode a frame.
        return Ok(None);
    };
    cpi_data.pop_lookahead = pop_lookahead;
    // (the source entry's image moves into cpi->source while it is coded)
    let (ts_start, ts_end, source_flags) = {
        let lookahead = cpi.ppi.lookahead.as_mut().ok_or(())?;
        let entry = &mut lookahead.buf[source];
        cpi.source_buf = std::mem::take(&mut entry.img);
        (entry.ts_start, entry.ts_end, entry.flags)
    };

    cpi_data.ts_frame_start = ts_start;
    cpi_data.ts_frame_end = ts_end;
    if ts_start < cpi.time_stamps.first_ts_start {
        cpi.time_stamps.first_ts_start = ts_start;
        cpi.time_stamps.prev_ts_end = ts_start;
    }

    // (av1_apply_encoding_flags(): the saver passes no encoding flags)
    debug_assert!(source_flags == 0);
    frame_flags = if source_flags & AOM_EFLAG_FORCE_KF != 0 {
        FRAMEFLAGS_KEY
    } else {
        frame_flags
    };

    // Shown frames and arf-overlay frames need frame-rate considering
    if frame_params.show_frame != 0 {
        adjust_frame_rate(cpi, ts_start, ts_end);
    }

    let res = av1_encode_strategy_source(cpi, cpi_data, dest, &mut frame_params, &mut frame_flags);

    // (the source image moves back into its lookahead entry)
    if let Some(lookahead) = cpi.ppi.lookahead.as_mut() {
        lookahead.buf[source].img = std::mem::take(&mut cpi.source_buf);
    }
    res?;
    cpi_data.lib_flags = frame_flags;
    Ok(Some(()))
}

fn av1_encode_strategy_source(
    cpi: &mut Av1Comp,
    cpi_data: &mut Av1CompData,
    dest: &mut [u8],
    frame_params: &mut EncodeFrameParams,
    frame_flags: &mut u32,
) -> Result<(), ()> {
    if frame_params.show_existing_frame == 0 {
        let cm = &mut cpi.common;
        let film_grain_params_present = cm.seq_params.film_grain_params_present;
        cm.cur_frame_mut().film_grain_params_present = film_grain_params_present;

        // only one operating point supported now
        let pts64 = ticks_to_timebase_units(cpi_data.timestamp_ratio, cpi_data.ts_frame_start);
        if pts64 < 0 || pts64 > u32::MAX as i64 {
            return Err(());
        }

        cm.frame_presentation_time = pts64 as u32;
    }

    let frame_update_type = cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize];

    frame_params.speed = cpi.oxcf.speed;

    // Work out some encoding parameters specific to the pass:
    if has_no_stats_stage(cpi) {
        // (the first frame, of a sequence with key_freq_max 0, is a key
        // frame)
        let kf_requested = cpi.common.current_frame.frame_number == 0
            || cpi.oxcf.kf_cfg.key_freq_max == 0
            || (*frame_flags & FRAMEFLAGS_KEY) != 0;
        if kf_requested
            && frame_update_type != OVERLAY_UPDATE
            && frame_update_type != INTNL_OVERLAY_UPDATE
        {
            frame_params.frame_type = KEY_FRAME;
        } else {
            debug_assert!(false);
        }
    }

    // (cyclic refresh: no AQ)
    debug_assert!(cpi.oxcf.q_cfg.aq_mode != CYCLIC_REFRESH_AQ);

    // Apply external override flags.
    {
        let ext_flags = cpi.ext_flags;
        set_ext_overrides(&mut cpi.common, frame_params, &ext_flags);
    }

    // Work out which reference frame slots may be used.
    let force_refresh_all = ((frame_params.frame_type == KEY_FRAME
        && frame_params.show_frame != 0)
        || frame_params.frame_type == S_FRAME)
        && frame_params.show_existing_frame == 0;

    let gf_index = cpi.gf_frame_index as usize;
    let refbuf_state = cpi.ppi.gf_group.refbuf_state[gf_index];
    let mut refresh_frame = frame_params.refresh_frame;
    av1_configure_buffer_updates(
        cpi,
        &mut refresh_frame,
        frame_update_type,
        refbuf_state,
        force_refresh_all,
    );
    frame_params.refresh_frame = refresh_frame;

    {
        // (init_ref_map_pair() and av1_get_ref_frames(): with no reference
        // buffers every slot maps to buffer 0)
        let cm = &mut cpi.common;
        debug_assert!(cm.ref_frame_map.iter().all(|r| r.is_none()));
        cm.remapped_ref_idx = [0; REF_FRAMES as usize];

        // (get_ref_frame_flags(): no reference buffer exists)
        frame_params.ref_frame_flags = 0;

        frame_params.primary_ref_frame = choose_primary_ref_frame(cpi, frame_params);

        frame_params.order_offset = cpi.ppi.gf_group.arf_src_offset[gf_index] as i32;

        frame_params.refresh_frame_flags = av1_get_refresh_frame_flags(cpi, gf_index);

        frame_params.existing_fb_idx_to_show = INVALID_IDX;
    }

    frame_params.remapped_ref_idx = cpi.common.remapped_ref_idx;

    cpi.td.mb.rdmult_delta_qindex = 0;
    cpi.td.mb.delta_qindex = 0;

    if frame_params.show_existing_frame == 0 {
        cpi.common.quant_params.using_qmatrix = cpi.oxcf.q_cfg.using_qm;
    }

    // (denoise_and_encode(): temporal filtering is off, so the frame is
    // coded directly)
    {
        let cm = &mut cpi.common;
        cm.show_frame = frame_params.show_frame;
        cm.current_frame.frame_type = frame_params.frame_type;
    }
    let (w, h) = (cpi.common.width, cpi.common.height);
    av1_set_frame_size(cpi, w, h)?;
    // set_mv_params
    av1_set_mv_search_params(cpi);
    // (av1_init_tpl_stats(): no TPL statistics)
    cpi_data.frame_size = av1_encode(cpi, dest, frame_params)?;

    // Update frame_flags for the next time encode_strategy is called
    update_frame_flags(&cpi.common, &cpi.refresh_frame, frame_flags);
    set_additional_frame_flags(&cpi.common, frame_flags);

    // (droppable: no external reference updates)
    cpi.droppable = false;
    Ok(())
}

/// Translation of `ticks_to_timebase_units()` (av1_iface_common.h);
/// `timestamp_ratio` is `{ num, den }`.
#[inline]
pub(crate) fn ticks_to_timebase_units(timestamp_ratio: (i64, i64), n: i64) -> i64 {
    let mut round = timestamp_ratio.0 / 2;
    if round > 0 {
        round -= 1;
    }
    (n * timestamp_ratio.1 + round) / timestamp_ratio.0
}

/// Translation of `timebase_units_to_ticks()` (av1_iface_common.h).
#[inline]
pub(crate) fn timebase_units_to_ticks(timestamp_ratio: (i64, i64), n: i64) -> i64 {
    n * timestamp_ratio.0 / timestamp_ratio.1
}
