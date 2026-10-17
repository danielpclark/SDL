// Rust translation of the parts of av1/encoder/encode_strategy.c and
// encode_strategy.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches)
// libavif reaches: the strategy for a still picture, a single shown key
// frame that resets the reference buffers, and the one pass real-time
// strategy of a sequence.
// Copyright (c) 2019, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encode strategy: which source frame is coded next, as which frame
//! type, with which reference buffer updates.
//!
//! The compressor of a still picture (all intra usage) codes exactly one
//! frame, the first of its key frame group. `av1_get_second_pass_params()`
//! (pass2_strategy.c) then makes the group of that key frame alone:
//! `frames_to_key` is `AOMMAX(1, key_freq_max)`, 1; the group's frame 0 is
//! a `KF_UPDATE` frame at layer depth 0 that resets the reference buffers.
//! Its results are set here directly (`set_kf_group()`); the second pass
//! statistics and the group's bit allocation only steer the bitrate driven
//! modes and later frames.
//!
//! A sequence (real-time usage, no lag) takes the one pass real-time
//! parameters (`av1_get_one_pass_rt_params()`) and the default one layer
//! reference structure of ratectrl.c.

use super::encoder::*;
use super::lookahead::*;
use super::ratectrl::*;
use super::speed_features::SpeedFeatures;
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::{INTER_FRAME, INTRA_ONLY_FRAME, KEY_FRAME, S_FRAME};
use crate::aom::av1::common::enums::*;

/// `SELECT_ALL_BUF_SLOTS`
const SELECT_ALL_BUF_SLOTS: i32 = 0xFF;
/// `TICKS_PER_SEC`
pub(crate) const TICKS_PER_SEC: i64 = 10000000;
/// `AOM_EFLAG_FORCE_KF`
pub(crate) const AOM_EFLAG_FORCE_KF: u32 = 1 << 0;
/// `AOM_EFLAG_ERROR_RESILIENT`
pub(crate) const AOM_EFLAG_ERROR_RESILIENT: u32 = 1 << 21;
/// `AOM_EFLAG_SET_S_FRAME`
pub(crate) const AOM_EFLAG_SET_S_FRAME: u32 = 1 << 22;

/// `EncodeFrameInput`: the source frames are `cpi.source_buf` and
/// `cpi.last_source_buf` (moved out of the lookahead queue while the frame
/// is coded).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EncodeFrameInput {
    pub(crate) has_last_source: bool,
    pub(crate) ts_duration: i64,
}

impl EncodeFrameInput {
    /// `frame_input->source`
    #[inline]
    pub(crate) fn source<'a>(&self, cpi: &'a Av1Comp) -> Option<&'a Yv12BufferConfig> {
        Some(&cpi.source_buf)
    }

    /// `frame_input->last_source`
    #[inline]
    pub(crate) fn last_source<'a>(&self, cpi: &'a Av1Comp) -> Option<&'a Yv12BufferConfig> {
        if self.has_last_source {
            Some(&cpi.last_source_buf)
        } else {
            None
        }
    }
}

/// Translation of `is_frame_droppable()` (encode_strategy.h).
#[inline]
pub(crate) fn is_frame_droppable(
    rtc_ref: &RtcRef,
    ext_refresh_frame_flags: &ExtRefreshFrameFlagsInfo,
) -> bool {
    // Droppable frame is only used by external refresh flags. VoD setting won't
    // trigger its use case.
    if rtc_ref.set_ref_frame_config {
        rtc_ref.non_reference_frame
    } else if ext_refresh_frame_flags.update_pending {
        !(ext_refresh_frame_flags.alt_ref_frame
            || ext_refresh_frame_flags.alt2_ref_frame
            || ext_refresh_frame_flags.bwd_ref_frame
            || ext_refresh_frame_flags.golden_frame
            || ext_refresh_frame_flags.last_frame)
    } else {
        false
    }
}

/// Translation of `get_current_frame_ref_type()` (encode_strategy.h).
#[inline]
pub(crate) fn get_current_frame_ref_type(cpi: &Av1Comp) -> i32 {
    // We choose the reference "type" of this frame from the flags which indicate
    // which reference frames will be refreshed by it. More than one of these
    // flags may be set, so the order here implies an order of precedence. This is
    // just used to choose the primary_ref_frame (as the most recent reference
    // buffer of the same reference-type as the current frame).

    match cpi.ppi.gf_group.layer_depth[cpi.gf_frame_index as usize] as i32 {
        0 => 0,
        1 => 1,
        MAX_ARF_LAYERS | 7 => 4,
        _ => 7,
    }
}

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

/// Translation of `av1_configure_buffer_updates()`.
pub(crate) fn av1_configure_buffer_updates(
    cpi: &mut Av1Comp,
    refresh_frame: &mut RefreshFrameInfo,
    update_type: u8,
    refbuf_state: i32,
    force_refresh_all: bool,
) {
    // NOTE(weitinglin): Should we define another function to take care of
    // cpi->rc.is_$Source_Type to make this function as it is in the comment?
    let ext_refresh_frame_flags = cpi.ext_flags.refresh_frame;
    cpi.rc.is_src_frame_alt_ref = 0;

    match update_type {
        KF_UPDATE => set_refresh_frame_flags(refresh_frame, true, true, true),

        LF_UPDATE => set_refresh_frame_flags(refresh_frame, false, false, false),

        GF_UPDATE => set_refresh_frame_flags(refresh_frame, true, false, false),

        OVERLAY_UPDATE => {
            if refbuf_state == REFBUF_RESET {
                set_refresh_frame_flags(refresh_frame, true, true, true);
            } else {
                set_refresh_frame_flags(refresh_frame, true, false, false);
            }

            cpi.rc.is_src_frame_alt_ref = 1;
        }

        ARF_UPDATE => {
            // NOTE: BWDREF does not get updated along with ALTREF_FRAME.
            if refbuf_state == REFBUF_RESET {
                set_refresh_frame_flags(refresh_frame, true, true, true);
            } else {
                set_refresh_frame_flags(refresh_frame, false, false, true);
            }
        }

        INTNL_OVERLAY_UPDATE => {
            set_refresh_frame_flags(refresh_frame, false, false, false);
            cpi.rc.is_src_frame_alt_ref = 1;
        }

        INTNL_ARF_UPDATE => set_refresh_frame_flags(refresh_frame, false, true, false),

        _ => debug_assert!(false),
    }

    if ext_refresh_frame_flags.update_pending && !is_stat_generation_stage(cpi) {
        set_refresh_frame_flags(
            refresh_frame,
            ext_refresh_frame_flags.golden_frame,
            ext_refresh_frame_flags.bwd_ref_frame,
            ext_refresh_frame_flags.alt_ref_frame,
        );
        let gf_frame_index = cpi.gf_frame_index as usize;
        let gf_group = &mut cpi.ppi.gf_group;
        if ext_refresh_frame_flags.golden_frame {
            gf_group.update_type[gf_frame_index] = GF_UPDATE;
        }
        if ext_refresh_frame_flags.alt_ref_frame {
            gf_group.update_type[gf_frame_index] = ARF_UPDATE;
        }
        if ext_refresh_frame_flags.bwd_ref_frame {
            gf_group.update_type[gf_frame_index] = INTNL_ARF_UPDATE;
        }
    }

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
    ext_flags: &mut ExternalFlags,
) {
    // Overrides the defaults with the externally supplied values with
    // av1_update_reference() and av1_update_entropy() calls
    // Note: The overrides are valid only for the next frame passed
    // to av1_encode_lowlevel()

    if ext_flags.use_s_frame {
        frame_params.frame_type = S_FRAME;
    }

    if ext_flags.refresh_frame_context_pending {
        cm.features.refresh_frame_context = ext_flags.refresh_frame_context as i32;
        ext_flags.refresh_frame_context_pending = false;
    }
    cm.features.allow_ref_frame_mvs = ext_flags.use_ref_frame_mvs;

    frame_params.error_resilient_mode = ext_flags.use_error_resilient;
    // A keyframe is already error resilient and keyframes with
    // error_resilient_mode interferes with the use of show_existing_frame
    // when forward reference keyframes are enabled.
    frame_params.error_resilient_mode &= frame_params.frame_type != KEY_FRAME;
    // For bitstream conformance, s-frames must be error-resilient
    frame_params.error_resilient_mode |= frame_params.frame_type == S_FRAME;
}

/// Translation of `choose_primary_ref_frame()`.
fn choose_primary_ref_frame(cpi: &Av1Comp, frame_params: &EncodeFrameParams) -> i32 {
    let cm = &cpi.common;

    let intra_only =
        frame_params.frame_type == KEY_FRAME || frame_params.frame_type == INTRA_ONLY_FRAME;
    if intra_only || frame_params.error_resilient_mode || cpi.ext_flags.use_primary_ref_none {
        return PRIMARY_REF_NONE;
    }

    // (!CONFIG_REALTIME_ONLY)
    if cpi.use_ducky_encode {
        // (the ducky encoder is never used)
        return PRIMARY_REF_NONE;
    }

    // In large scale case, always use Last frame's frame contexts.
    // Note(yunqing): In other cases, primary_ref_frame is chosen based on
    // cpi->ppi->gf_group.layer_depth[cpi->gf_frame_index], which also controls
    // frame bit allocation.
    if cm.tiles.large_scale != 0 {
        return LAST_FRAME - LAST_FRAME;
    }

    // (av1_svc_primary_ref_frame(): no SVC, no external reference
    // configuration)
    debug_assert!(!cpi.ppi.use_svc && !cpi.ppi.rtc_ref.set_ref_frame_config);

    // Find the most recent reference frame with the same reference type as the
    // current frame
    let current_ref_type = get_current_frame_ref_type(cpi);
    let wanted_fb = cpi.ppi.fb_of_context_type[current_ref_type as usize];
    let mut primary_ref_frame = PRIMARY_REF_NONE;
    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        if get_ref_frame_map_idx(cm, ref_frame) == wanted_fb {
            primary_ref_frame = ref_frame - LAST_FRAME;
        }
    }

    primary_ref_frame
}

/// Translation of `adjust_frame_rate()`.
fn adjust_frame_rate(cpi: &mut Av1Comp, ts_start: i64, ts_end: i64) {
    let this_duration;
    let mut step = 0;

    // Clear down mmx registers

    // (spatial layers: one layer)
    debug_assert!(!cpi.ppi.use_svc);

    if ts_start == cpi.time_stamps.first_ts_start {
        this_duration = ts_end - ts_start;
        step = 1;
    } else {
        let last_duration = cpi.time_stamps.prev_ts_end - cpi.time_stamps.prev_ts_start;

        this_duration = ts_end - cpi.time_stamps.prev_ts_end;

        // do a step update if the duration changes by 10%
        if last_duration != 0 {
            step = (((this_duration - last_duration) * 10 / last_duration) != 0) as i32;
        }
    }

    if this_duration != 0 {
        if step != 0 {
            cpi.new_framerate = 10000000.0 / this_duration as f64;
            let framerate = cpi.new_framerate;
            av1_new_framerate(cpi, framerate);
        } else {
            // Average this frame's rate into the last second's average
            // frame rate. If we haven't seen 1 second yet, then average
            // over the whole interval seen.
            let interval = ((ts_end - cpi.time_stamps.first_ts_start) as f64).min(10000000.0);
            let mut avg_duration = 10000000.0 / cpi.framerate;
            avg_duration *= interval - avg_duration + this_duration as f64;
            avg_duration /= interval;
            cpi.new_framerate = 10000000.0 / avg_duration;
            // For parallel frames update cpi->framerate with new_framerate
            // during av1_post_encode_updates()
            let framerate =
                if cpi.ppi.gf_group.frame_parallel_level[cpi.gf_frame_index as usize] > 0 {
                    cpi.framerate
                } else {
                    cpi.new_framerate
                };
            av1_new_framerate(cpi, framerate);
        }
    }

    cpi.time_stamps.prev_ts_start = ts_start;
    cpi.time_stamps.prev_ts_end = ts_end;
}

/// Translation of `choose_frame_source()`: the lookahead indices of the
/// source and the last source.
fn choose_frame_source(
    cpi: &Av1Comp,
    pop_lookahead: &mut bool,
    last_source: &mut Option<usize>,
    show_frame: &mut i32,
) -> Option<usize> {
    let cm = &cpi.common;
    let gf_group = &cpi.ppi.gf_group;
    let lookahead = cpi.ppi.lookahead.as_ref()?;

    // Source index in lookahead buffer.
    let mut src_index = gf_group.arf_src_offset[cpi.gf_frame_index as usize] as i32;

    // TODO(Aasaipriya): Forced key frames need to be fixed when rc_mode != AOM_Q
    // (the one pass groups have no ARFs)
    debug_assert!(src_index == 0);

    // If the current frame is arf, then we should not pop from the lookahead
    // buffer. If the current frame is not arf, then pop it. This assumes the
    // first frame in the GF group is not arf. May need to change if it is not
    // true.
    *pop_lookahead = src_index == 0;
    // If this is a key frame and keyframe filtering is enabled with overlay,
    // then do not pop.
    // (keyframe filtering is off: no lag)
    debug_assert!(!(*pop_lookahead && cpi.oxcf.kf_cfg.enable_keyframe_filtering > 1));

    // LAP stage does not have ARFs or forward key-frames,
    // hence, always pop_lookahead here.
    debug_assert!(!is_stat_generation_stage(cpi));

    *show_frame = *pop_lookahead as i32;

    {
        // Future frame in parallel encode set
        if gf_group.src_offset[cpi.gf_frame_index as usize] != 0 && !is_stat_generation_stage(cpi) {
            src_index = gf_group.src_offset[cpi.gf_frame_index as usize] as i32;
        }
    }
    let source;
    if *show_frame != 0 {
        // show frame, pop from buffer
        // Get last frame source.
        if cm.current_frame.frame_number > 0 {
            *last_source = av1_lookahead_peek(lookahead, src_index - 1, cpi.compressor_stage);
        }
        // Read in the source frame.
        source = av1_lookahead_peek(lookahead, src_index, cpi.compressor_stage);
    } else {
        // no show frames are arf frames
        debug_assert!(false);
        source = None;
    }
    source
}

/// Translation of `allow_show_existing()`: don't allow a
/// show_existing_frame to coincide with an error resilient or S-Frame. An
/// exception can be made in the case of a keyframe, since it does not
/// depend on any previous frames.
fn allow_show_existing(cpi: &Av1Comp, frame_flags: u32) -> bool {
    if cpi.common.current_frame.frame_number == 0 {
        return false;
    }

    let Some(lookahead) = cpi.ppi.lookahead.as_ref() else {
        return true;
    };
    let Some(lookahead_src) = av1_lookahead_peek(lookahead, 0, cpi.compressor_stage) else {
        return true;
    };
    let lookahead_src = &lookahead.buf[lookahead_src];

    let is_error_resilient = cpi.oxcf.tool_cfg.error_resilient_mode
        || (lookahead_src.flags & AOM_EFLAG_ERROR_RESILIENT) != 0;
    let is_s_frame =
        cpi.oxcf.kf_cfg.enable_sframe || (lookahead_src.flags & AOM_EFLAG_SET_S_FRAME) != 0;
    let is_key_frame = (cpi.rc.frames_to_key == 0) || (frame_flags & FRAMEFLAGS_KEY) != 0;
    !(is_error_resilient || is_s_frame) || is_key_frame
}

/// Translation of `update_frame_flags()`: update frame_flags to tell the
/// encoder's caller what sort of frame was encoded.
fn update_frame_flags(cm: &Av1Common, refresh_frame: &RefreshFrameInfo, frame_flags: &mut u32) {
    // (no show existing frames)
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

/// Translation of `av1_get_refresh_frame_flags()`.
fn av1_get_refresh_frame_flags(
    cpi: &Av1Comp,
    frame_params: &EncodeFrameParams,
    gf_index: usize,
) -> i32 {
    let ext_refresh_frame_flags = &cpi.ext_flags.refresh_frame;

    let gf_group = &cpi.ppi.gf_group;
    if gf_group.refbuf_state[gf_index] == REFBUF_RESET {
        return SELECT_ALL_BUF_SLOTS;
    }

    // TODO(jingning): Deprecate the following operations.
    // Switch frames and shown key-frames overwrite all reference slots
    if frame_params.frame_type == S_FRAME {
        return SELECT_ALL_BUF_SLOTS;
    }

    // show_existing_frames don't actually send refresh_frame_flags so set the
    // flags to 0 to keep things consistent.
    if frame_params.show_existing_frame != 0 {
        return 0;
    }

    let rtc_ref = &cpi.ppi.rtc_ref;
    if is_frame_droppable(rtc_ref, ext_refresh_frame_flags) {
        return 0;
    }

    // (!CONFIG_REALTIME_ONLY: the ducky encoder is never used)

    let mut refresh_mask = 0;
    if ext_refresh_frame_flags.update_pending {
        if rtc_ref.set_ref_frame_config || use_rtc_reference_structure_one_layer(cpi) {
            for i in 0..INTER_REFS_PER_FRAME as usize {
                let ref_frame_map_idx = rtc_ref.ref_idx[i];
                refresh_mask |= rtc_ref.refresh[ref_frame_map_idx as usize] << ref_frame_map_idx;
            }
            return refresh_mask;
        }
        // Unfortunately the encoder interface reflects the old refresh_*_frame
        // flags so we have to replicate the old refresh_frame_flags logic here in
        // order to preserve the behaviour of the flag overrides.
        // (the default one layer reference structure always applies)
        debug_assert!(false);
    }

    // Search for the open slot to store the current frame.
    // (the one pass real-time frames always have their update pending, the
    // all intra frames reset the buffers)
    debug_assert!(false);
    refresh_mask
}

/// Translation of `init_ref_map_pair()` (encoder.h).
fn init_ref_map_pair(
    cpi: &Av1Comp,
    ref_frame_map_pairs: &mut [RefFrameMapPair; REF_FRAMES as usize],
) {
    if cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize] == KF_UPDATE {
        *ref_frame_map_pairs = [RefFrameMapPair {
            pyr_level: -1,
            disp_order: -1,
        }; REF_FRAMES as usize];
        return;
    }
    *ref_frame_map_pairs = [RefFrameMapPair::default(); REF_FRAMES as usize];
    let cm = &cpi.common;
    for map_idx in 0..REF_FRAMES as usize {
        // Get reference frame buffer.
        let buf = cm.ref_frame_map[map_idx];
        if ref_frame_map_pairs[map_idx].disp_order == -1 {
            continue;
        }
        let Some(buf) = buf else {
            ref_frame_map_pairs[map_idx].disp_order = -1;
            ref_frame_map_pairs[map_idx].pyr_level = -1;
            continue;
        };
        if cm.buffer_pool.frame_bufs[buf].ref_count > 1 {
            // Once the keyframe is coded, the slots in ref_frame_map will all
            // point to the same frame. In that case, all subsequent pointers
            // matching the current are considered "free" slots. This will find
            // the next occurrence of the current pointer if ref_count indicates
            // there are multiple instances of it and mark it as free.
            for idx2 in map_idx + 1..REF_FRAMES as usize {
                let buf2 = cm.ref_frame_map[idx2];
                if buf2 == Some(buf) {
                    ref_frame_map_pairs[idx2].disp_order = -1;
                    ref_frame_map_pairs[idx2].pyr_level = -1;
                }
            }
        }
        let b = &cm.buffer_pool.frame_bufs[buf];
        ref_frame_map_pairs[map_idx].disp_order = b.display_order_hint as i32;
        ref_frame_map_pairs[map_idx].pyr_level = b.pyramid_level as i32;
    }
}

/// `RefFrameMapPair` (encoder.h)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RefFrameMapPair {
    pub(crate) pyr_level: i32,
    pub(crate) disp_order: i32,
}

/// `ref_frame_priority_order[]` (encoder.h)
const REF_FRAME_PRIORITY_ORDER: [MvReferenceFrame; INTER_REFS_PER_FRAME as usize] = [
    LAST_FRAME,
    ALTREF_FRAME,
    BWDREF_FRAME,
    GOLDEN_FRAME,
    ALTREF2_FRAME,
    LAST2_FRAME,
    LAST3_FRAME,
];

/// Translation of `get_ref_frame_flags()` (encoder.h); the reference
/// frames are buffer pool indices.
fn get_ref_frame_flags(
    sf: &SpeedFeatures,
    use_one_pass_rt_params: bool,
    ref_frames: &[Option<usize>; INTER_REFS_PER_FRAME as usize],
    ext_ref_frame_flags: i32,
) -> i32 {
    // cpi->ext_flags.ref_frame_flags allows certain reference types to be
    // disabled by the external interface.  These are set by
    // av1_apply_encoding_flags(). Start with what the external interface allows,
    // then suppress any reference types which we have found to be duplicates.
    let mut flags = ext_ref_frame_flags;

    for i in 1..INTER_REFS_PER_FRAME as usize {
        let this_ref = ref_frames[i];
        // If this_ref has appeared before, mark the corresponding ref frame as
        // invalid. For one_pass_rt mode, only disable GOLDEN_FRAME if it's the
        // same as LAST_FRAME or ALTREF_FRAME (if ALTREF is being used in nonrd).
        let index = if use_one_pass_rt_params && REF_FRAME_PRIORITY_ORDER[i] == GOLDEN_FRAME {
            (1 + sf.rt_sf.use_nonrd_altref_frame) as usize
        } else {
            i
        };
        for j in 0..index {
            // If this_ref has appeared before (same as the reference corresponding
            // to lower index j), remove it as a reference only if that reference
            // (for index j) is actually used as a reference.
            if this_ref == ref_frames[j] && (flags & (1 << (REF_FRAME_PRIORITY_ORDER[j] - 1))) != 0
            {
                flags &= !(1 << (REF_FRAME_PRIORITY_ORDER[i] - 1));
                break;
            }
        }
    }
    flags
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

/// Translation of `av1_encode_strategy()`: returns `None` (the C code's -1)
/// when there is no frame to code.
pub(crate) fn av1_encode_strategy(
    cpi: &mut Av1Comp,
    cpi_data: &mut Av1CompData,
    dest: &mut [u8],
) -> Result<Option<()>, ()> {
    let mut frame_input = EncodeFrameInput::default();
    let mut frame_params = EncodeFrameParams::default();
    let flush = cpi_data.flush;

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
            // (!CONFIG_REALTIME_ONLY: no first pass)
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
    if !use_one_pass_rt_params && !is_stat_generation_stage(cpi) {
        // (av1_get_second_pass_params())
        set_kf_group(cpi);
    }

    debug_assert!(!is_stat_generation_stage(cpi));
    {
        let gf_group = &cpi.ppi.gf_group;
        // TODO(jingning): fwd key frame always uses show existing frame?
        // (no overlays: no ARFs)
        let update_type = gf_group.update_type[cpi.gf_frame_index as usize];
        debug_assert!(update_type != OVERLAY_UPDATE && update_type != INTNL_OVERLAY_UPDATE);
        frame_params.show_existing_frame = 0;
    }
    frame_params.show_existing_frame &= allow_show_existing(cpi, cpi_data.lib_flags) as i32;

    let mut pop_lookahead = false;
    let mut last_source = None;
    let Some(source) = choose_frame_source(
        cpi,
        &mut pop_lookahead,
        &mut last_source,
        &mut frame_params.show_frame,
    ) else {
        // If no source was found, we can't encode a frame.
        return Ok(None);
    };
    cpi_data.pop_lookahead = pop_lookahead;

    // reset src_offset to allow actual encode call for this frame to get its
    // source.
    let gf_frame_index = cpi.gf_frame_index as usize;
    cpi.ppi.gf_group.src_offset[gf_frame_index] = 0;

    // Source may be changed if temporal filtered later.
    // (the source entry's image, and the last source's, move into
    // cpi->source_buf and cpi->last_source_buf while the frame is coded)
    let (ts_start, ts_end, source_flags) = {
        let lookahead = cpi.ppi.lookahead.as_mut().ok_or(())?;
        let entry = &mut lookahead.buf[source];
        cpi.source_buf = std::mem::take(&mut entry.img);
        let r = (entry.ts_start, entry.ts_end, entry.flags);
        if let Some(last) = last_source {
            cpi.last_source_buf = std::mem::take(&mut lookahead.buf[last].img);
        }
        r
    };
    debug_assert!(!cpi.ppi.use_svc);
    frame_input.has_last_source = last_source.is_some();
    cpi.has_last_source = last_source.is_some();
    frame_input.ts_duration = ts_end - ts_start;

    let res = av1_encode_strategy_source(
        cpi,
        cpi_data,
        dest,
        &frame_input,
        &mut frame_params,
        ts_start,
        ts_end,
        source_flags,
    );

    // (the source images move back into their lookahead entries)
    if let Some(lookahead) = cpi.ppi.lookahead.as_mut() {
        lookahead.buf[source].img = std::mem::take(&mut cpi.source_buf);
        if let Some(last) = last_source {
            lookahead.buf[last].img = std::mem::take(&mut cpi.last_source_buf);
        }
    }
    cpi.has_last_source = false;
    res?;
    Ok(Some(()))
}

#[allow(clippy::too_many_arguments)]
fn av1_encode_strategy_source(
    cpi: &mut Av1Comp,
    cpi_data: &mut Av1CompData,
    dest: &mut [u8],
    frame_input: &EncodeFrameInput,
    frame_params: &mut EncodeFrameParams,
    ts_start: i64,
    ts_end: i64,
    source_flags: u32,
) -> Result<(), ()> {
    // Save unfiltered source. It is used in av1_get_second_pass_params().
    // (cpi->unfiltered_source is the source)

    cpi_data.ts_frame_start = ts_start;
    cpi_data.ts_frame_end = ts_end;
    if ts_start < cpi.time_stamps.first_ts_start {
        cpi.time_stamps.first_ts_start = ts_start;
        cpi.time_stamps.prev_ts_end = ts_start;
    }

    av1_apply_encoding_flags(cpi, source_flags);
    let mut frame_flags: u32 = if source_flags & AOM_EFLAG_FORCE_KF != 0 {
        FRAMEFLAGS_KEY
    } else {
        0
    };

    // Shown frames and arf-overlay frames need frame-rate considering
    if frame_params.show_frame != 0 {
        adjust_frame_rate(cpi, ts_start, ts_end);
    }

    if frame_params.show_existing_frame == 0 {
        let cm = &mut cpi.common;
        // (no film grain table)
        let film_grain_params_present = cm.seq_params.film_grain_params_present;
        cm.cur_frame_mut().film_grain_params_present = film_grain_params_present;

        // only one operating point supported now
        let pts64 = ticks_to_timebase_units(cpi_data.timestamp_ratio, ts_start);
        if pts64 < 0 || pts64 > u32::MAX as i64 {
            return Err(());
        }

        cm.frame_presentation_time = pts64 as u32;
    }

    let use_one_pass_rt_params = is_one_pass_rt_params(cpi);
    if use_one_pass_rt_params {
        av1_get_one_pass_rt_params(cpi, &mut frame_params.frame_type, frame_input, frame_flags);
        if use_rtc_reference_structure_one_layer(cpi) {
            let gf_update = cpi.gf_frame_index == 0;
            av1_set_rtc_reference_structure_one_layer(cpi, gf_update);
        }
    }

    let frame_update_type = cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize];

    // (no show existing frames)

    // Per-frame encode speed.  In theory this can vary, but things may have
    // been written assuming speed-level will not change within a sequence, so
    // this parameter should be used with caution.
    frame_params.speed = cpi.oxcf.speed;

    // (!CONFIG_REALTIME_ONLY)
    // Set forced key frames when necessary. For two-pass encoding / lap mode,
    // this is already handled by av1_get_second_pass_params. However when no
    // stats are available, we still need to check if the new frame is a keyframe.
    // For one pass rt, this is already checked in av1_get_one_pass_rt_params.
    if !use_one_pass_rt_params && (is_stat_generation_stage(cpi) || has_no_stats_stage(cpi)) {
        // Current frame is coded as a key-frame for any of the following cases:
        // 1) First frame of a video
        // 2) For all-intra frame encoding
        // 3) When a key-frame is forced
        let kf_requested = cpi.common.current_frame.frame_number == 0
            || cpi.oxcf.kf_cfg.key_freq_max == 0
            || (frame_flags & FRAMEFLAGS_KEY) != 0;
        if kf_requested
            && frame_update_type != OVERLAY_UPDATE
            && frame_update_type != INTNL_OVERLAY_UPDATE
        {
            frame_params.frame_type = KEY_FRAME;
        } else if is_stat_generation_stage(cpi) {
            // For stats generation, set the frame type to inter here.
            frame_params.frame_type = INTER_FRAME;
        }
    }

    // Work out some encoding parameters specific to the pass:
    if has_no_stats_stage(cpi) && cpi.oxcf.q_cfg.aq_mode == CYCLIC_REFRESH_AQ {
        // (cyclic refresh: no AQ)
        debug_assert!(false);
    }

    // Apply external override flags.
    {
        let mut ext_flags = cpi.ext_flags;
        set_ext_overrides(&mut cpi.common, frame_params, &mut ext_flags);
        cpi.ext_flags = ext_flags;
    }

    // Shown keyframes and S frames refresh all reference buffers
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
        let mut ref_frame_map_pairs = [RefFrameMapPair::default(); REF_FRAMES as usize];
        init_ref_map_pair(cpi, &mut ref_frame_map_pairs);
        let gf_group = &cpi.ppi.gf_group;
        let order_offset = gf_group.arf_src_offset[gf_index] as i32;
        let _cur_frame_disp = cpi.common.current_frame.frame_number as i32 + order_offset;

        if gf_group.frame_parallel_level[gf_index] == 0 {
            if !cpi.ext_flags.refresh_frame.update_pending {
                // (av1_get_ref_frames(): the still picture's key frame maps
                // every slot to the buffer it resets them to)
                debug_assert!(cpi.common.ref_frame_map.iter().all(|r| r.is_none()));
                cpi.common.remapped_ref_idx = [0; REF_FRAMES as usize];
            } else if cpi.ppi.rtc_ref.set_ref_frame_config
                || use_rtc_reference_structure_one_layer(cpi)
            {
                for i in 0..INTER_REFS_PER_FRAME as usize {
                    cpi.common.remapped_ref_idx[i] = cpi.ppi.rtc_ref.ref_idx[i];
                }
            }
        }

        // Get the reference frames
        let mut ref_frame_buf = [None; INTER_REFS_PER_FRAME as usize];
        let mut has_ref_frames = false;
        for i in 0..INTER_REFS_PER_FRAME as usize {
            let ref_frame = get_ref_frame_buf(&cpi.common, REF_FRAME_PRIORITY_ORDER[i]);
            ref_frame_buf[i] = ref_frame;
            if ref_frame.is_some() {
                has_ref_frames = true;
            }
        }
        if !has_ref_frames
            && (frame_params.frame_type == INTER_FRAME || frame_params.frame_type == S_FRAME)
        {
            return Err(());
        }

        // Work out which reference frame slots may be used.
        frame_params.ref_frame_flags = get_ref_frame_flags(
            &cpi.sf,
            is_one_pass_rt_params(cpi),
            &ref_frame_buf,
            cpi.ext_flags.ref_frame_flags,
        );

        // Set primary_ref_frame of non-reference frames as PRIMARY_REF_NONE.
        if cpi.ppi.gf_group.is_frame_non_ref[gf_index] {
            frame_params.primary_ref_frame = PRIMARY_REF_NONE;
        } else {
            frame_params.primary_ref_frame = choose_primary_ref_frame(cpi, frame_params);
        }

        frame_params.order_offset = cpi.ppi.gf_group.arf_src_offset[gf_index] as i32;

        // Call av1_get_refresh_frame_flags() if refresh index not available.
        debug_assert!(!cpi.refresh_idx_available);
        frame_params.refresh_frame_flags = av1_get_refresh_frame_flags(cpi, frame_params, gf_index);

        // Make the frames marked as is_frame_non_ref to non-reference frames.
        if cpi.ppi.gf_group.is_frame_non_ref[gf_index] {
            frame_params.refresh_frame_flags = 0;
        }

        frame_params.existing_fb_idx_to_show = INVALID_IDX;
        // Find the frame buffer to show based on display order.
        // (no show existing frames)
    }

    // The way frame_params->remapped_ref_idx is setup is a placeholder.
    // Currently, reference buffer assignment is done by update_ref_frame_map()
    // which is called by high-level strategy AFTER encoding a frame.  It
    // modifies cm->remapped_ref_idx.  If you want to use an alternative method
    // to determine reference buffer assignment, just put your assignments into
    // frame_params->remapped_ref_idx here and they will be used when encoding
    // this frame.  If frame_params->remapped_ref_idx is setup independently of
    // cm->remapped_ref_idx then update_ref_frame_map() will have no effect.
    frame_params.remapped_ref_idx = cpi.common.remapped_ref_idx;

    cpi.td.mb.rdmult_delta_qindex = 0;
    cpi.td.mb.delta_qindex = 0;

    if frame_params.show_existing_frame == 0 {
        cpi.common.quant_params.using_qmatrix = cpi.oxcf.q_cfg.using_qm;
    }

    // (!CONFIG_REALTIME_ONLY)
    if has_no_stats_stage(cpi) && cpi.oxcf.mode == REALTIME && cpi.oxcf.gf_cfg.lag_in_frames == 0 {
        cpi_data.frame_size = av1_encode(cpi, dest, frame_input, frame_params)?;
    } else {
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
        cpi_data.frame_size = av1_encode(cpi, dest, frame_input, frame_params)?;
    }

    // This is used in rtc temporal filter case. Use true source in the PSNR
    // calculation.
    // (no PSNR)

    if !is_stat_generation_stage(cpi) {
        // First pass doesn't modify reference buffer assignment or produce frame
        // flags
        update_frame_flags(&cpi.common, &cpi.refresh_frame, &mut frame_flags);
        set_additional_frame_flags(&cpi.common, &mut frame_flags);
    }
    cpi_data.lib_flags = frame_flags;

    // Unpack frame_results:
    // Leave a signal for a higher level caller about if this frame is droppable
    if cpi_data.frame_size > 0 {
        cpi.droppable = is_frame_droppable(&cpi.ppi.rtc_ref, &cpi.ext_flags.refresh_frame);
    }

    // For SVC: keep track of the (unscaled) source corresponding to the
    // refresh of LAST reference (base temporal layer- TL0). Copy only for the
    // top spatial enhancement layer so all spatial layers of the next
    // superframe have last_source to be aligned with previous TL0 superframe.
    // Avoid cases where resolution changes for unscaled source (top spatial
    // layer).
    // (no SVC)
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
