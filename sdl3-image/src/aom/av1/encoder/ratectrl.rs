// Rust translation of the parts of av1/encoder/ratectrl.c and ratectrl.h
// from aom (https://aomedia.googlesource.com/aom, at the revision
// SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches) the saver
// reaches: the fixed quality (AOM_Q) choice of a still key frame's
// quantizer, and the rate control initialization.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Rate control. With `AOM_Q` and a key frame that is the only frame of its
//! sequence (`frames_to_key <= 1`) the quantizer is the configured cq level,
//! clamped to the quantizer range; the bit budget bookkeeping of later
//! frames is left out (the compressor codes one frame).

use super::encoder::*;
use crate::aom::av1::common::alloccommon::av1_get_mbs;
use crate::aom::av1::common::av1_common_int::frame_is_intra_only;
use crate::aom::av1::common::blockd::{FRAME_TYPES, INTER_FRAME, KEY_FRAME};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::quant_common::av1_ac_quant_qtx;

// Bits Per MB at different Q (Multiplied by 512)
pub(crate) const BPER_MB_NORMBITS: i32 = 9;

// Use this macro to turn on/off use of alt-refs in one-pass mode.
pub(crate) const USE_ALTREF_FOR_ONE_PASS: i32 = 1;

// The maximum duration of a GF group that is static (e.g. a slide show).
pub(crate) const MAX_STATIC_GF_GROUP_LENGTH: usize = 250;

pub(crate) const MIN_GF_INTERVAL: i32 = 4;
pub(crate) const MAX_GF_INTERVAL: i32 = 32;
pub(crate) const FIXED_GF_INTERVAL: i32 = 16;

pub(crate) const MAX_ARF_LAYERS: i32 = 6;

/// `RATE_FACTOR_LEVEL`
pub(crate) const INTER_NORMAL: usize = 0;
pub(crate) const GF_ARF_LOW: usize = 1;
pub(crate) const GF_ARF_STD: usize = 2;
pub(crate) const KF_STD: usize = 3;
pub(crate) const RATE_FACTOR_LEVELS: usize = 4;

/// `FRAME_UPDATE_TYPE`
pub(crate) const KF_UPDATE: u8 = 0;
pub(crate) const LF_UPDATE: u8 = 1;
pub(crate) const GF_UPDATE: u8 = 2;
pub(crate) const ARF_UPDATE: u8 = 3;
pub(crate) const OVERLAY_UPDATE: u8 = 4;
pub(crate) const INTNL_OVERLAY_UPDATE: u8 = 5; // Internal Overlay Frame
pub(crate) const INTNL_ARF_UPDATE: u8 = 6; // Internal Altref Frame

/// `REFBUF_STATE`
pub(crate) const REFBUF_RESET: i32 = 0; // Clear reference frame buffer
pub(crate) const REFBUF_UPDATE: i32 = 1; // Refresh reference frame buffer

/// `RESIZE_STATE`
pub(crate) const ORIG: i32 = 0;

const MAX_MB_RATE: i32 = 250;
const MAXRATE_1080P: i32 = 2025000;

const FRAME_OVERHEAD_BITS: i32 = 200;

/// The reached members of `RATE_CONTROL`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RateControl {
    pub(crate) this_frame_target: i32,
    pub(crate) projected_frame_size: i32,
    pub(crate) coefficient_size: i64,
    pub(crate) frames_since_golden: i32,
    pub(crate) frames_till_gf_update_due: i32,
    pub(crate) min_gf_interval: i32,
    pub(crate) max_gf_interval: i32,
    pub(crate) static_scene_max_gf_interval: i32,
    pub(crate) frames_to_key: i32,
    pub(crate) frames_since_key: i32,
    pub(crate) frames_to_fwd_kf: i32,
    pub(crate) is_src_frame_alt_ref: i32,
    pub(crate) avg_frame_bandwidth: i32,
    pub(crate) min_frame_bandwidth: i32,
    pub(crate) max_frame_bandwidth: i32,
    pub(crate) ni_av_qi: i32,
    pub(crate) ni_tot_qi: i32,
    pub(crate) worst_quality: i32,
    pub(crate) best_quality: i32,
    pub(crate) active_worst_quality: i32,
    pub(crate) avg_frame_low_motion: i32,
    pub(crate) resize_state: i32,
    pub(crate) resize_avg_qp: i32,
    pub(crate) resize_buffer_underflow: i32,
    pub(crate) resize_count: i32,
    pub(crate) rtc_external_ratectrl: i32,
    pub(crate) frame_level_fast_extra_bits: i32,
}

/// The reached members of `PRIMARY_RATE_CONTROL`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PrimaryRateControl {
    pub(crate) gfu_boost: i32,
    pub(crate) kf_boost: i32,
    pub(crate) baseline_gf_interval: i32,
    pub(crate) this_key_frame_forced: i32,
    pub(crate) next_key_frame_forced: i32,
    pub(crate) ni_frames: i32,
    pub(crate) tot_q: f64,
    pub(crate) total_actual_bits: i64,
    pub(crate) total_target_bits: i64,
    pub(crate) starting_buffer_level: i64,
    pub(crate) buffer_level: i64,
    pub(crate) bits_off_target: i64,
    pub(crate) avg_frame_qindex: [i32; FRAME_TYPES as usize],
    pub(crate) avg_q: f64,
    pub(crate) last_q: [i32; FRAME_TYPES as usize],
    pub(crate) rate_correction_factors: [f64; RATE_FACTOR_LEVELS],
    pub(crate) rolling_target_bits: i32,
    pub(crate) rolling_actual_bits: i32,
}

// These functions use formulaic calculations to make playing with the
// quantizer tables easier. If necessary they can be replaced by lookup
// tables if and when things settle down in the experimental bitstream

/// Translation of `av1_convert_qindex_to_q()`.
pub(crate) fn av1_convert_qindex_to_q(qindex: i32, bit_depth: i32) -> f64 {
    // Convert the index to a real Q value (scaled down to match old Q values)
    match bit_depth {
        AOM_BITS_8 => av1_ac_quant_qtx(qindex, 0, bit_depth) as f64 / 4.0,
        AOM_BITS_10 => av1_ac_quant_qtx(qindex, 0, bit_depth) as f64 / 16.0,
        AOM_BITS_12 => av1_ac_quant_qtx(qindex, 0, bit_depth) as f64 / 64.0,
        _ => {
            debug_assert!(
                false,
                "bit_depth should be AOM_BITS_8, AOM_BITS_10 or AOM_BITS_12"
            );
            -1.0
        }
    }
}

/// Translation of `av1_rc_get_default_min_gf_interval()`.
pub(crate) fn av1_rc_get_default_min_gf_interval(width: i32, height: i32, framerate: f64) -> i32 {
    // Assume we do not need any constraint lower than 4K 20 fps
    const FACTOR_SAFE: f64 = 3840.0 * 2160.0 * 20.0;
    let factor = width as f64 * height as f64 * framerate;
    let default_interval = ((framerate * 0.125) as i32).clamp(MIN_GF_INTERVAL, MAX_GF_INTERVAL);

    if factor <= FACTOR_SAFE {
        default_interval
    } else {
        default_interval.max((MIN_GF_INTERVAL as f64 * factor / FACTOR_SAFE + 0.5) as i32)
    }
    // Note this logic makes:
    // 4K24: 5
    // 4K30: 6
    // 4K60: 12
}

/// Translation of `av1_rc_get_default_max_gf_interval()`.
pub(crate) fn av1_rc_get_default_max_gf_interval(framerate: f64, min_gf_interval: i32) -> i32 {
    let mut interval = MAX_GF_INTERVAL.min((framerate * 0.75) as i32);
    interval += interval & 0x01; // Round to even value
    interval = MAX_GF_INTERVAL.max(interval);
    interval.max(min_gf_interval)
}

/// Translation of `av1_primary_rc_init()`.
pub(crate) fn av1_primary_rc_init(oxcf: &Av1EncoderConfig, p_rc: &mut PrimaryRateControl) {
    let rc_cfg = &oxcf.rc_cfg;

    let worst_allowed_q = rc_cfg.worst_allowed_q;

    let mut min_gf_interval = oxcf.gf_cfg.min_gf_interval;
    let mut max_gf_interval = oxcf.gf_cfg.max_gf_interval;
    if min_gf_interval == 0 {
        min_gf_interval = av1_rc_get_default_min_gf_interval(
            oxcf.frm_dim_cfg.width,
            oxcf.frm_dim_cfg.height,
            oxcf.input_cfg.init_framerate,
        );
    }
    if max_gf_interval == 0 {
        max_gf_interval =
            av1_rc_get_default_max_gf_interval(oxcf.input_cfg.init_framerate, min_gf_interval);
    }
    p_rc.baseline_gf_interval = (min_gf_interval + max_gf_interval) / 2;
    p_rc.this_key_frame_forced = 0;
    p_rc.next_key_frame_forced = 0;
    p_rc.ni_frames = 0;

    p_rc.tot_q = 0.0;
    p_rc.total_actual_bits = 0;
    p_rc.total_target_bits = 0;
    p_rc.buffer_level = p_rc.starting_buffer_level;

    // (no target level: worst_allowed_q stays)
    debug_assert!(oxcf.target_seq_level_idx[0] >= SEQ_LEVELS);
    if oxcf.pass == AOM_RC_ONE_PASS && rc_cfg.mode == AOM_CBR {
        p_rc.avg_frame_qindex[KEY_FRAME as usize] = worst_allowed_q;
        p_rc.avg_frame_qindex[INTER_FRAME as usize] = worst_allowed_q;
    } else {
        p_rc.avg_frame_qindex[KEY_FRAME as usize] = (worst_allowed_q + rc_cfg.best_allowed_q) / 2;
        p_rc.avg_frame_qindex[INTER_FRAME as usize] = (worst_allowed_q + rc_cfg.best_allowed_q) / 2;
    }
    p_rc.avg_q = av1_convert_qindex_to_q(rc_cfg.worst_allowed_q, oxcf.tool_cfg.bit_depth);
    p_rc.last_q[KEY_FRAME as usize] = rc_cfg.best_allowed_q;
    p_rc.last_q[INTER_FRAME as usize] = rc_cfg.worst_allowed_q;

    for i in 0..RATE_FACTOR_LEVELS {
        p_rc.rate_correction_factors[i] = 0.7;
    }
    p_rc.rate_correction_factors[KF_STD] = 1.0;
    p_rc.bits_off_target = p_rc.starting_buffer_level;

    p_rc.rolling_target_bits =
        (oxcf.rc_cfg.target_bandwidth as f64 / oxcf.input_cfg.init_framerate) as i32;
    p_rc.rolling_actual_bits =
        (oxcf.rc_cfg.target_bandwidth as f64 / oxcf.input_cfg.init_framerate) as i32;
}

/// Translation of `av1_rc_init()`.
pub(crate) fn av1_rc_init(oxcf: &Av1EncoderConfig, rc: &mut RateControl) {
    let rc_cfg = &oxcf.rc_cfg;

    rc.frames_since_key = 8; // Sensible default for first frame.
    rc.frames_to_fwd_kf = oxcf.kf_cfg.fwd_kf_dist;

    rc.frames_till_gf_update_due = 0;
    rc.ni_av_qi = rc_cfg.worst_allowed_q;
    rc.ni_tot_qi = 0;

    rc.min_gf_interval = oxcf.gf_cfg.min_gf_interval;
    rc.max_gf_interval = oxcf.gf_cfg.max_gf_interval;
    if rc.min_gf_interval == 0 {
        rc.min_gf_interval = av1_rc_get_default_min_gf_interval(
            oxcf.frm_dim_cfg.width,
            oxcf.frm_dim_cfg.height,
            oxcf.input_cfg.init_framerate,
        );
    }
    if rc.max_gf_interval == 0 {
        rc.max_gf_interval =
            av1_rc_get_default_max_gf_interval(oxcf.input_cfg.init_framerate, rc.min_gf_interval);
    }
    rc.avg_frame_low_motion = 0;

    rc.resize_state = ORIG;
    rc.resize_avg_qp = 0;
    rc.resize_buffer_underflow = 0;
    rc.resize_count = 0;
    rc.rtc_external_ratectrl = 0;
    rc.frame_level_fast_extra_bits = 0;
}

// (get_rate_correction_factor() and the rate correction updates: the rate
// model only matters for the quantizer choice of the bitrate driven modes
// and for later frames)

/// Translation of `get_active_cq_level()`.
fn get_active_cq_level(
    oxcf: &Av1EncoderConfig,
    intra_only: bool,
    superres_mode: i32,
    superres_denom: i32,
) -> i32 {
    let rc_cfg = &oxcf.rc_cfg;
    let active_cq_level = rc_cfg.cq_level;
    if rc_cfg.mode == AOM_CQ || rc_cfg.mode == AOM_Q {
        // printf("Superres %d %d %d = %d\n", superres_denom, intra_only,
        //        rc->frames_to_key, !(intra_only && rc->frames_to_key <= 1));
        // (superres is off)
        debug_assert!(superres_mode != AOM_SUPERRES_QTHRESH && superres_mode != AOM_SUPERRES_AUTO);
        let _ = (intra_only, superres_denom);
    }
    // (AOM_CQ is not used)
    debug_assert!(rc_cfg.mode != AOM_CQ);
    active_cq_level
}

/// Translation of `get_intra_q_and_bounds()`: the key frame is the only
/// frame of its sequence, so the cq level is the quantizer.
fn get_intra_q_and_bounds(
    cpi: &Av1Comp,
    active_best: &mut i32,
    active_worst: &mut i32,
    cq_level: i32,
) {
    let rc = &cpi.rc;
    let oxcf = &cpi.oxcf;
    let active_best_quality;
    let active_worst_quality;

    if rc.frames_to_key <= 1 && oxcf.rc_cfg.mode == AOM_Q {
        // If the next frame is also a key frame or the current frame is the
        // only frame in the sequence in AOM_Q mode, just use the cq_level
        // as q.
        active_best_quality = cq_level;
        active_worst_quality = cq_level;
    } else {
        // (forced key frames and key frame boosts: a still picture's key
        // frame is always the last of its sequence)
        debug_assert!(false);
        active_best_quality = cq_level;
        active_worst_quality = cq_level;
    }
    *active_best = active_best_quality;
    *active_worst = active_worst_quality;
}

/// Translation of `rc_pick_q_and_bounds_q_mode()`.
fn rc_pick_q_and_bounds_q_mode(cpi: &Av1Comp, bottom_index: &mut i32, top_index: &mut i32) -> i32 {
    let cm = &cpi.common;
    let rc = &cpi.rc;
    let oxcf = &cpi.oxcf;
    let cq_level = get_active_cq_level(
        oxcf,
        frame_is_intra_only(cm),
        cpi.superres_mode,
        cm.superres_scale_denominator as i32,
    );
    let mut active_best_quality = 0;
    let mut active_worst_quality = rc.active_worst_quality;

    debug_assert!(frame_is_intra_only(cm));
    get_intra_q_and_bounds(
        cpi,
        &mut active_best_quality,
        &mut active_worst_quality,
        cq_level,
    );

    *top_index = active_worst_quality;
    *bottom_index = active_best_quality;

    *top_index = (*top_index).max(rc.best_quality);
    *top_index = (*top_index).min(rc.worst_quality);

    *bottom_index = (*bottom_index).max(rc.best_quality);
    *bottom_index = (*bottom_index).min(rc.worst_quality);

    let mut q = active_best_quality;

    q = q.max(rc.best_quality);
    q = q.min(rc.worst_quality);

    debug_assert!(*top_index <= rc.worst_quality && *top_index >= rc.best_quality);
    debug_assert!(*bottom_index <= rc.worst_quality && *bottom_index >= rc.best_quality);
    debug_assert!(q <= rc.worst_quality && q >= rc.best_quality);

    q
}

/// Translation of `rc_pick_q_and_bounds()` (`AOM_Q` only).
fn rc_pick_q_and_bounds(
    cpi: &Av1Comp,
    gf_index: usize,
    bottom_index: &mut i32,
    top_index: &mut i32,
) -> i32 {
    let gf_group = &cpi.ppi.gf_group;
    debug_assert!(
        !has_no_stats_stage(cpi)
            || (cpi.oxcf.rc_cfg.mode == AOM_Q && gf_group.update_type[gf_index] != ARF_UPDATE)
    );

    debug_assert!(cpi.oxcf.rc_cfg.mode == AOM_Q);
    rc_pick_q_and_bounds_q_mode(cpi, bottom_index, top_index)
}

/// Translation of `av1_rc_pick_q_and_bounds()`.
pub(crate) fn av1_rc_pick_q_and_bounds(
    cpi: &mut Av1Comp,
    width: i32,
    height: i32,
    gf_index: usize,
    bottom_index: &mut i32,
    top_index: &mut i32,
) -> i32 {
    let _ = (width, height);
    // TODO(sarahparker) merge no-stats vbr and altref q computation
    // with rc_pick_q_and_bounds().
    let gf_group = &cpi.ppi.gf_group;
    // (the one pass VBR and CBR quantizer choices: the saver uses AOM_Q)
    debug_assert!(cpi.oxcf.rc_cfg.mode == AOM_Q && gf_group.update_type[gf_index] != ARF_UPDATE);
    rc_pick_q_and_bounds(cpi, gf_index, bottom_index, top_index)
    // (p_rc->arf_q: no ARF)
}

/// Translation of `av1_rc_set_gf_interval_range()` (fixed Q, one pass).
pub(crate) fn av1_rc_set_gf_interval_range(cpi: &Av1Comp, rc: &mut RateControl) {
    let oxcf = &cpi.oxcf;

    // Special case code for 1 pass fixed Q mode tests
    debug_assert!(has_no_stats_stage(cpi) && oxcf.rc_cfg.mode == AOM_Q);
    rc.max_gf_interval = oxcf.gf_cfg.max_gf_interval;
    rc.min_gf_interval = oxcf.gf_cfg.min_gf_interval;
    rc.static_scene_max_gf_interval = rc.min_gf_interval + 1;
}

/// Translation of `av1_rc_update_framerate()`.
pub(crate) fn av1_rc_update_framerate(cpi: &mut Av1Comp, width: i32, height: i32) {
    let oxcf = &cpi.oxcf;
    let mut rc = cpi.rc;
    let mbs = av1_get_mbs(width, height);

    rc.avg_frame_bandwidth = (oxcf.rc_cfg.target_bandwidth as f64 / cpi.framerate).round() as i32;
    rc.min_frame_bandwidth =
        (rc.avg_frame_bandwidth as i64 * oxcf.rc_cfg.vbrmin_section as i64 / 100) as i32;

    rc.min_frame_bandwidth = rc.min_frame_bandwidth.max(FRAME_OVERHEAD_BITS);

    // A maximum bitrate for a frame is defined.
    // The baseline for this aligns with HW implementations that
    // can support decode of 1080P content up to a bitrate of MAX_MB_RATE bits
    // per 16x16 MB (averaged over a frame). However this limit is extended if
    // a very high rate is given on the command line or the the rate cannnot
    // be acheived because of a user specificed max q (e.g. when the user
    // specifies lossless encode.
    let vbr_max_bits =
        ((rc.avg_frame_bandwidth as i64 * oxcf.rc_cfg.vbrmax_section as i64) / 100) as i32;
    rc.max_frame_bandwidth = (mbs.wrapping_mul(MAX_MB_RATE))
        .max(MAXRATE_1080P)
        .max(vbr_max_bits);

    av1_rc_set_gf_interval_range(cpi, &mut rc);
    cpi.rc = rc;
}

// (av1_rc_postencode_update(), av1_rc_set_frame_target() and
// av1_calc_iframe_target_size_one_pass_vbr(): the frame size target only
// steers the bitrate driven modes and later frames)
