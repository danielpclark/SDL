// Rust translation of the parts of av1/encoder/ratectrl.c and ratectrl.h
// from aom (https://aomedia.googlesource.com/aom, at the revision
// SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches) the AVIF
// encoders reach: the fixed quality (AOM_Q) choice of a still key frame's
// quantizer, and the one pass constant bitrate (AOM_CBR) control of the
// real-time usage, with its reference structure and scene detection.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Rate control. With `AOM_Q` and a key frame that is the only frame of its
//! sequence (`frames_to_key <= 1`) the quantizer is the configured cq level,
//! clamped to the quantizer range. The one pass CBR control (one spatial
//! and temporal layer, no cyclic refresh, no dynamic resize, no external
//! rate control) is translated whole.

use std::sync::OnceLock;

use super::encode_strategy::EncodeFrameInput;
use super::encoder::*;
use crate::aom::av1::common::alloccommon::av1_get_mbs;
use crate::aom::av1::common::av1_common_int::{frame_is_intra_only, frame_is_sframe};
use crate::aom::av1::common::blockd::{
    FrameType, FRAME_TYPES, INTER_FRAME, INTRA_ONLY_FRAME, KEY_FRAME,
};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::quant_common::{av1_ac_quant_qtx, QINDEX_RANGE};

// Bits Per MB at different Q (Multiplied by 512)
pub(crate) const BPER_MB_NORMBITS: i32 = 9;

// Use this macro to turn on/off use of alt-refs in one-pass mode.
pub(crate) const USE_ALTREF_FOR_ONE_PASS: i32 = 1;

// The maximum duration of a GF group that is static (e.g. a slide show).
pub(crate) const MAX_STATIC_GF_GROUP_LENGTH: usize = 250;

pub(crate) const MIN_GF_INTERVAL: i32 = 4;
pub(crate) const MAX_GF_INTERVAL: i32 = 32;
pub(crate) const FIXED_GF_INTERVAL: i32 = 16;
pub(crate) const MAX_GF_LENGTH_LAP: i32 = 16;

pub(crate) const FIXED_GF_INTERVAL_RT: i32 = 80;
pub(crate) const MAX_GF_INTERVAL_RT: i32 = 160;

pub(crate) const MAX_NUM_GF_INTERVALS: i32 = 15;

pub(crate) const MAX_ARF_LAYERS: i32 = 6;
// #define STRICT_RC

pub(crate) const DEFAULT_KF_BOOST_RT: i32 = 2300;
pub(crate) const DEFAULT_GF_BOOST_RT: i32 = 2000;

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
pub(crate) const FRAME_UPDATE_TYPES: usize = 7;

/// `REFBUF_STATE`
pub(crate) const REFBUF_RESET: i32 = 0; // Clear reference frame buffer
pub(crate) const REFBUF_UPDATE: i32 = 1; // Refresh reference frame buffer

/// `RESIZE_STATE`
pub(crate) const ORIG: i32 = 0;

// Max rate target for 1080P and below encodes under normal circumstances
// (1920 * 1080 / (16 * 16)) * MAX_MB_RATE bits per MB
const MAX_MB_RATE: i32 = 250;
const MAXRATE_1080P: i32 = 2025000;

const MIN_BPB_FACTOR: f64 = 0.005;
const MAX_BPB_FACTOR: f64 = 50.0;

pub(crate) const FRAME_OVERHEAD_BITS: i32 = 200;

/// The members of `RATE_CONTROL` (without the CONFIG_FPMT_TEST state).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RateControl {
    // Rate targetting variables
    pub(crate) base_frame_target: i32, // A baseline frame target before adjustment.
    pub(crate) this_frame_target: i32, // Actual frame target after rc adjustment.
    pub(crate) projected_frame_size: i32,
    pub(crate) coefficient_size: i64,
    pub(crate) sb64_target_rate: i32,
    pub(crate) frames_since_golden: i32,
    pub(crate) frames_till_gf_update_due: i32,
    pub(crate) intervals_till_gf_calculate_due: i32,
    pub(crate) min_gf_interval: i32,
    pub(crate) max_gf_interval: i32,
    pub(crate) static_scene_max_gf_interval: i32,
    pub(crate) frames_to_key: i32,
    pub(crate) frames_since_key: i32,
    pub(crate) frames_to_fwd_kf: i32,
    pub(crate) is_src_frame_alt_ref: i32,
    pub(crate) sframe_due: i32,
    pub(crate) high_source_sad: i32,
    pub(crate) avg_source_sad: u64,
    pub(crate) prev_avg_source_sad: u64,
    pub(crate) frame_source_sad: u64,
    pub(crate) avg_frame_bandwidth: i32, // Average frame size target for clip
    pub(crate) min_frame_bandwidth: i32, // Minimum allocation used for any frame
    pub(crate) max_frame_bandwidth: i32, // Maximum burst rate allowed for a frame.
    pub(crate) prev_avg_frame_bandwidth: i32,
    pub(crate) ni_av_qi: i32,
    pub(crate) ni_tot_qi: i32,
    pub(crate) decimation_factor: i32,
    pub(crate) decimation_count: i32,
    // rate control history for last frame(1) and the frame before(2).
    // -1: undershot
    //  1: overshoot
    //  0: not initialized.
    pub(crate) rc_1_frame: i32,
    pub(crate) rc_2_frame: i32,
    pub(crate) q_1_frame: i32,
    pub(crate) q_2_frame: i32,
    pub(crate) worst_quality: i32,
    pub(crate) best_quality: i32,
    pub(crate) active_worst_quality: i32,
    // Track amount of low motion in scene
    pub(crate) avg_frame_low_motion: i32,
    pub(crate) cnt_zeromv: i32,
    // signals if number of blocks with motion is high
    pub(crate) percent_blocks_with_motion: i32,
    // Maximum value of source sad across all blocks of frame.
    pub(crate) max_block_source_sad: u64,
    // For dynamic resize, 1 pass cbr.
    pub(crate) resize_state: i32,
    pub(crate) resize_avg_qp: i32,
    pub(crate) resize_buffer_underflow: i32,
    pub(crate) resize_count: i32,
    // Flag to disable content related qp adjustment.
    pub(crate) rtc_external_ratectrl: bool,
    // Stores fast_extra_bits of the current frame.
    pub(crate) frame_level_fast_extra_bits: i32,
    pub(crate) frame_level_rate_correction_factors: [f64; RATE_FACTOR_LEVELS],
    pub(crate) frame_num_last_gf_refresh: i32,
    pub(crate) prev_coded_width: i32,
    pub(crate) prev_coded_height: i32,
}

/// The members of `PRIMARY_RATE_CONTROL` the one pass control reads (the
/// two pass, look ahead and CONFIG_FPMT_TEST state left out).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PrimaryRateControl {
    pub(crate) gf_group_bits: i64,
    pub(crate) kf_boost: i32,
    pub(crate) gfu_boost: i32,
    pub(crate) baseline_gf_interval: i32,
    pub(crate) constrained_gf_group: i32,
    pub(crate) this_key_frame_forced: i32,
    pub(crate) next_key_frame_forced: i32,
    pub(crate) starting_buffer_level: i64,
    pub(crate) optimal_buffer_level: i64,
    pub(crate) maximum_buffer_size: i64,
    pub(crate) arf_q: i32,
    pub(crate) ni_frames: i32,
    pub(crate) tot_q: f64,
    pub(crate) last_kf_qindex: i32,
    pub(crate) avg_frame_qindex: [i32; FRAME_TYPES as usize],
    pub(crate) last_boosted_qindex: i32,
    pub(crate) avg_q: f64,
    pub(crate) last_q: [i32; FRAME_TYPES as usize],
    pub(crate) rate_correction_factors: [f64; RATE_FACTOR_LEVELS],
    pub(crate) total_actual_bits: i64,
    pub(crate) total_target_bits: i64,
    pub(crate) buffer_level: i64,
    pub(crate) rate_error_estimate: i32,
    pub(crate) vbr_bits_off_target: i64,
    pub(crate) vbr_bits_off_target_fast: i64,
    pub(crate) bits_off_target: i64,
    pub(crate) rolling_target_bits: i32,
    pub(crate) rolling_actual_bits: i32,
}

/// The `kf_low_motion_minq` ... `rtc_minq` tables of a bit depth.
pub(crate) struct MinqLuts {
    pub(crate) kf_low_motion_minq: [i32; QINDEX_RANGE as usize],
    pub(crate) kf_high_motion_minq: [i32; QINDEX_RANGE as usize],
    pub(crate) arfgf_low_motion_minq: [i32; QINDEX_RANGE as usize],
    pub(crate) arfgf_high_motion_minq: [i32; QINDEX_RANGE as usize],
    pub(crate) inter_minq: [i32; QINDEX_RANGE as usize],
    pub(crate) rtc_minq: [i32; QINDEX_RANGE as usize],
}

// Tables relating active max Q to active min Q
// (the _8, _10 and _12 tables, built once: aom_once(rc_init_minq_luts))
static MINQ_LUTS: OnceLock<[MinqLuts; 3]> = OnceLock::new();

const GF_HIGH: i32 = 2400;
const GF_LOW: i32 = 300;
const KF_HIGH: i32 = 5000;
const KF_LOW: i32 = 400;

/// The minq tables of `bit_depth` (`ASSIGN_MINQ_TABLE()`).
fn minq_luts(bit_depth: i32) -> &'static MinqLuts {
    let luts = MINQ_LUTS.get_or_init(rc_init_minq_luts);
    match bit_depth {
        AOM_BITS_8 => &luts[0],
        AOM_BITS_10 => &luts[1],
        _ => {
            debug_assert!(bit_depth == AOM_BITS_12);
            &luts[2]
        }
    }
}

// How many times less pixels there are to encode given the current scaling.
// Temporary replacement for rcf_mult and rate_thresh_mult.
#[inline]
fn resize_rate_factor(frm_dim_cfg: &FrameDimensionCfg, width: i32, height: i32) -> f64 {
    (frm_dim_cfg.width * frm_dim_cfg.height) as f64 / (width * height) as f64
}

// Functions to compute the active minq lookup table entries based on a
// formulaic approach to facilitate easier adjustment of the Q tables.
// The formulae were derived from computing a 3rd order polynomial best
// fit to the original data (after plotting real maxq vs minq (not q index))
fn get_minq_index(maxq: f64, x3: f64, x2: f64, x1: f64, bit_depth: i32) -> i32 {
    let minqtarget = (((x3 * maxq + x2) * maxq + x1) * maxq).min(maxq);

    // Special case handling to deal with the step from q2.0
    // down to lossless mode represented by q 1.0.
    if minqtarget <= 2.0 {
        return 0;
    }

    av1_find_qindex(minqtarget, bit_depth, 0, QINDEX_RANGE - 1)
}

fn init_minq_luts(luts: &mut MinqLuts, bit_depth: i32) {
    for i in 0..QINDEX_RANGE as usize {
        let maxq = av1_convert_qindex_to_q(i as i32, bit_depth);
        luts.kf_low_motion_minq[i] = get_minq_index(maxq, 0.000001, -0.0004, 0.150, bit_depth);
        luts.kf_high_motion_minq[i] = get_minq_index(maxq, 0.0000021, -0.00125, 0.45, bit_depth);
        luts.arfgf_low_motion_minq[i] = get_minq_index(maxq, 0.0000015, -0.0009, 0.30, bit_depth);
        luts.arfgf_high_motion_minq[i] = get_minq_index(maxq, 0.0000021, -0.00125, 0.55, bit_depth);
        luts.inter_minq[i] = get_minq_index(maxq, 0.00000271, -0.00113, 0.90, bit_depth);
        luts.rtc_minq[i] = get_minq_index(maxq, 0.00000271, -0.00113, 0.70, bit_depth);
    }
}

fn rc_init_minq_luts() -> [MinqLuts; 3] {
    let new = || MinqLuts {
        kf_low_motion_minq: [0; QINDEX_RANGE as usize],
        kf_high_motion_minq: [0; QINDEX_RANGE as usize],
        arfgf_low_motion_minq: [0; QINDEX_RANGE as usize],
        arfgf_high_motion_minq: [0; QINDEX_RANGE as usize],
        inter_minq: [0; QINDEX_RANGE as usize],
        rtc_minq: [0; QINDEX_RANGE as usize],
    };
    let mut luts = [new(), new(), new()];
    init_minq_luts(&mut luts[0], AOM_BITS_8);
    init_minq_luts(&mut luts[1], AOM_BITS_10);
    init_minq_luts(&mut luts[2], AOM_BITS_12);
    luts
}

/// Translation of `av1_rc_init_minq_luts()`.
pub(crate) fn av1_rc_init_minq_luts() {
    MINQ_LUTS.get_or_init(rc_init_minq_luts);
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

/// Translation of `av1_get_bpmb_enumerator()`.
pub(crate) fn av1_get_bpmb_enumerator(frame_type: FrameType, is_screen_content_type: i32) -> i32 {
    let enumerator;

    if is_screen_content_type != 0 {
        enumerator = if frame_type == KEY_FRAME {
            1000000
        } else {
            750000
        };
    } else {
        enumerator = if frame_type == KEY_FRAME {
            2000000
        } else {
            1500000
        };
    }

    enumerator
}

/// Translation of `av1_rc_bits_per_mb()` (the accurate estimate, of
/// `hl_sf.accurate_bit_estimate`, is off at the real-time speed).
pub(crate) fn av1_rc_bits_per_mb(
    cpi: &Av1Comp,
    frame_type: FrameType,
    qindex: i32,
    correction_factor: f64,
    accurate_estimate: i32,
) -> i32 {
    let cm = &cpi.common;
    let is_screen_content_type = cpi.is_screen_content_type;
    let bit_depth = cm.seq_params.bit_depth;
    let q = av1_convert_qindex_to_q(qindex, bit_depth);

    debug_assert!(!(frame_type != KEY_FRAME && accurate_estimate != 0));

    let enumerator = av1_get_bpmb_enumerator(frame_type, is_screen_content_type);
    debug_assert!(correction_factor <= MAX_BPB_FACTOR && correction_factor >= MIN_BPB_FACTOR);

    // q based adjustment to baseline enumerator
    (enumerator as f64 * correction_factor / q) as i32
}

/// Translation of `av1_estimate_bits_at_q()`.
pub(crate) fn av1_estimate_bits_at_q(cpi: &Av1Comp, q: i32, correction_factor: f64) -> i32 {
    let cm = &cpi.common;
    let frame_type = cm.current_frame.frame_type;
    let mbs = cm.mi_params.mbs;
    let bpm = av1_rc_bits_per_mb(
        cpi,
        frame_type,
        q,
        correction_factor,
        cpi.sf.hl_sf.accurate_bit_estimate,
    );
    FRAME_OVERHEAD_BITS.max(((bpm as u64).wrapping_mul(mbs as u64) as i32) >> BPER_MB_NORMBITS)
}

/// Translation of `av1_rc_clamp_iframe_target_size()`.
pub(crate) fn av1_rc_clamp_iframe_target_size(cpi: &Av1Comp, mut target: i64) -> i32 {
    let rc = &cpi.rc;
    let rc_cfg = &cpi.oxcf.rc_cfg;
    if rc_cfg.max_intra_bitrate_pct != 0 {
        let max_rate = rc.avg_frame_bandwidth as i64 * rc_cfg.max_intra_bitrate_pct as i64 / 100;
        target = target.min(max_rate);
    }
    if target > rc.max_frame_bandwidth as i64 {
        target = rc.max_frame_bandwidth as i64;
    }
    target as i32
}

// Update the buffer level: leaky bucket model.
fn update_buffer_level(cpi: &mut Av1Comp, encoded_frame_size: i32) {
    let cm = &cpi.common;
    let rc = &cpi.rc;
    let p_rc = &mut cpi.ppi.p_rc;

    // Non-viewable frames are a special case and are treated as pure overhead.
    if cm.show_frame == 0 {
        p_rc.bits_off_target -= encoded_frame_size as i64;
    } else {
        p_rc.bits_off_target += (rc.avg_frame_bandwidth - encoded_frame_size) as i64;
    }

    // Clip the buffer level to the maximum specified buffer size.
    p_rc.bits_off_target = p_rc.bits_off_target.min(p_rc.maximum_buffer_size);
    // For screen-content mode: don't let buffel level go below threshold,
    // given here as -rc->maximum_ buffer_size, to allow buffer to come back
    // up sooner after slide change with big oveshoot.
    if cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN {
        p_rc.bits_off_target = p_rc.bits_off_target.max(-p_rc.maximum_buffer_size);
    }
    p_rc.buffer_level = p_rc.bits_off_target;

    // (update_layer_buffer_level(): no layers)
    debug_assert!(!cpi.ppi.use_svc);
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

    let mut worst_allowed_q = rc_cfg.worst_allowed_q;

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

    if oxcf.target_seq_level_idx[0] < SEQ_LEVELS {
        worst_allowed_q = 255;
    }
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
    rc.rtc_external_ratectrl = false;
    rc.frame_level_fast_extra_bits = 0;
}

/// Translation of `av1_rc_drop_frame()`.
pub(crate) fn av1_rc_drop_frame(cpi: &mut Av1Comp) -> bool {
    let oxcf = &cpi.oxcf;
    let buffer_level = cpi.ppi.p_rc.buffer_level;

    if oxcf.rc_cfg.drop_frames_water_mark == 0 {
        false
    } else {
        let rc = &mut cpi.rc;
        let p_rc = &cpi.ppi.p_rc;
        if buffer_level < 0 {
            // Always drop if buffer is below 0.
            true
        } else {
            // If buffer is below drop_mark, for now just drop every other frame
            // (starting with the next frame) until it increases back over drop_mark.
            let drop_mark = (oxcf.rc_cfg.drop_frames_water_mark as i64 * p_rc.optimal_buffer_level
                / 100) as i32;
            if buffer_level > drop_mark as i64 && rc.decimation_factor > 0 {
                rc.decimation_factor -= 1;
            } else if buffer_level <= drop_mark as i64 && rc.decimation_factor == 0 {
                rc.decimation_factor = 1;
            }
            if rc.decimation_factor > 0 {
                if rc.decimation_count > 0 {
                    rc.decimation_count -= 1;
                    true
                } else {
                    rc.decimation_count = rc.decimation_factor;
                    false
                }
            } else {
                rc.decimation_count = 0;
                false
            }
        }
    }
}

fn adjust_q_cbr(
    cpi: &Av1Comp,
    mut q: i32,
    active_worst_quality: i32,
    width: i32,
    height: i32,
) -> i32 {
    let rc = &cpi.rc;
    let p_rc = &cpi.ppi.p_rc;
    let cm = &cpi.common;
    let refresh_frame = &cpi.refresh_frame;
    let max_delta_down = if cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN {
        8.min(1.max(rc.q_1_frame / 16))
    } else {
        16.min(1.max(rc.q_1_frame / 8))
    };
    let max_delta_up = 20;
    let change_avg_frame_bandwidth = (rc.avg_frame_bandwidth - rc.prev_avg_frame_bandwidth).abs()
        as f64
        > 0.1 * rc.avg_frame_bandwidth as f64;
    // If resolution changes or avg_frame_bandwidth significantly changed,
    // then set this flag to indicate change in target bits per macroblock.
    let prev_frame = cm.prev_frame.map(|i| &cm.buffer_pool.frame_bufs[i]);
    let change_target_bits_mb = prev_frame.is_some_and(|prev_frame| {
        width != prev_frame.width || height != prev_frame.height || change_avg_frame_bandwidth
    });
    // Apply some control/clamp to QP under certain conditions.
    if cm.current_frame.frame_type != KEY_FRAME
        && !cpi.ppi.use_svc
        && rc.frames_since_key > 1
        && !change_target_bits_mb
        && (cpi.oxcf.rc_cfg.gf_cbr_boost_pct == 0
            || !(refresh_frame.alt_ref_frame || refresh_frame.golden_frame))
    {
        // Make sure q is between oscillating Qs to prevent resonance.
        if rc.rc_1_frame * rc.rc_2_frame == -1 && rc.q_1_frame != rc.q_2_frame {
            let qclamp = q.clamp(
                rc.q_1_frame.min(rc.q_2_frame),
                rc.q_1_frame.max(rc.q_2_frame),
            );
            // If the previous frame had overshoot and the current q needs to
            // increase above the clamped value, reduce the clamp for faster reaction
            // to overshoot.
            if cpi.rc.rc_1_frame == -1 && q > qclamp && rc.frames_since_key > 10 {
                q = (q + qclamp) >> 1;
            } else {
                q = qclamp;
            }
        }
        // Adjust Q base on source content change from scene detection.
        if cpi.sf.rt_sf.check_scene_detection != 0
            && rc.prev_avg_source_sad > 0
            && rc.frames_since_key > 10
            && rc.frame_source_sad > 0
            && !cpi.ppi.use_svc
        {
            let bit_depth = cm.seq_params.bit_depth;
            let delta = rc.avg_source_sad as f64 / rc.prev_avg_source_sad as f64 - 1.0;
            // Push Q downwards if content change is decreasing and buffer level
            // is stable (at least 1/4-optimal level), so not overshooting. Do so
            // only for high Q to avoid excess overshoot.
            // Else reduce decrease in Q from previous frame if content change is
            // increasing and buffer is below max (so not undershooting).
            if delta < 0.0
                && p_rc.buffer_level > (p_rc.optimal_buffer_level >> 2)
                && q > (rc.worst_quality >> 1)
            {
                let q_adj_factor = 1.0 + 0.5 * (4.0 * delta).tanh();
                let q_val = av1_convert_qindex_to_q(q, bit_depth);
                q += av1_compute_qdelta(rc, q_val, q_val * q_adj_factor, bit_depth);
            } else if rc.q_1_frame - q > 0
                && delta > 0.1
                && p_rc.buffer_level < p_rc.maximum_buffer_size.min(p_rc.optimal_buffer_level << 1)
            {
                q = (3 * q + rc.q_1_frame) >> 2;
            }
        }
        // Limit the decrease in Q from previous frame.
        if rc.q_1_frame - q > max_delta_down {
            q = rc.q_1_frame - max_delta_down;
        // Limit the increase in Q from previous frame.
        } else if q - rc.q_1_frame > max_delta_up && cpi.oxcf.tune_cfg.content != AOM_CONTENT_SCREEN
        {
            q = rc.q_1_frame + max_delta_up;
        }
    }
    // For single spatial layer: if resolution has increased push q closer
    // to the active_worst to avoid excess overshoot.
    if cpi.svc.number_spatial_layers <= 1
        && prev_frame.is_some_and(|prev_frame| {
            (width * height) as f64 > 1.5 * (prev_frame.width * prev_frame.height) as f64
        })
    {
        q = (q + active_worst_quality) >> 1;
    }
    q.min(cpi.rc.worst_quality).max(cpi.rc.best_quality)
}

/*\brief Gets a rate vs Q correction factor
 *
 * This function returns the current value of a correction factor used to
 * dynamilcally adjust the relationship between Q and the expected number
 * of bits for the frame.
 *
 * \ingroup rate_control
 * \param[in]   cpi                   Top level encoder instance structure
 * \param[in]   width                 Frame width
 * \param[in]   height                Frame height
 *
 * \return Returns a correction factor for the current frame
 */
fn get_rate_correction_factor(cpi: &Av1Comp, width: i32, height: i32) -> f64 {
    let rc = &cpi.rc;
    let p_rc = &cpi.ppi.p_rc;
    let refresh_frame = &cpi.refresh_frame;
    let mut rcf;

    let parallel = cpi.ppi.gf_group.frame_parallel_level[cpi.gf_frame_index as usize] > 0;
    let rate_correction_factors_kfstd = if parallel {
        rc.frame_level_rate_correction_factors[KF_STD]
    } else {
        p_rc.rate_correction_factors[KF_STD]
    };
    let rate_correction_factors_gfarfstd = if parallel {
        rc.frame_level_rate_correction_factors[GF_ARF_STD]
    } else {
        p_rc.rate_correction_factors[GF_ARF_STD]
    };
    let rate_correction_factors_internormal = if parallel {
        rc.frame_level_rate_correction_factors[INTER_NORMAL]
    } else {
        p_rc.rate_correction_factors[INTER_NORMAL]
    };

    if cpi.common.current_frame.frame_type == KEY_FRAME {
        rcf = rate_correction_factors_kfstd;
    } else {
        // (is_stat_consumption_stage(): one pass, no look ahead)
        debug_assert!(!is_stat_consumption_stage(cpi));
        if (refresh_frame.alt_ref_frame || refresh_frame.golden_frame)
            && rc.is_src_frame_alt_ref == 0
            && !cpi.ppi.use_svc
            && (cpi.oxcf.rc_cfg.mode != AOM_CBR || cpi.oxcf.rc_cfg.gf_cbr_boost_pct > 20)
        {
            rcf = rate_correction_factors_gfarfstd;
        } else {
            rcf = rate_correction_factors_internormal;
        }
    }
    rcf *= resize_rate_factor(&cpi.oxcf.frm_dim_cfg, width, height);
    rcf.clamp(MIN_BPB_FACTOR, MAX_BPB_FACTOR)
}

/*\brief Sets a rate vs Q correction factor
 *
 * This function updates the current value of a correction factor used to
 * dynamilcally adjust the relationship between Q and the expected number
 * of bits for the frame.
 *
 * \ingroup rate_control
 * \param[in]   cpi                   Top level encoder instance structure
 * \param[in]   is_encode_stage       Indicates if recode loop or post-encode
 * \param[in]   factor                New correction factor
 * \param[in]   width                 Frame width
 * \param[in]   height                Frame height
 *
 * \remark Updates the rate correction factor for the
 *         current frame type in cpi->rc.
 */
fn set_rate_correction_factor(
    cpi: &mut Av1Comp,
    is_encode_stage: bool,
    mut factor: f64,
    width: i32,
    height: i32,
) {
    let mut update_default_rcf = true;
    // Normalize RCF to account for the size-dependent scaling factor.
    factor /= resize_rate_factor(&cpi.oxcf.frm_dim_cfg, width, height);

    factor = factor.clamp(MIN_BPB_FACTOR, MAX_BPB_FACTOR);

    let parallel = cpi.ppi.gf_group.frame_parallel_level[cpi.gf_frame_index as usize] > 0;
    let refresh_frame = cpi.refresh_frame;
    let rc = &mut cpi.rc;
    let p_rc = &mut cpi.ppi.p_rc;
    if cpi.common.current_frame.frame_type == KEY_FRAME {
        p_rc.rate_correction_factors[KF_STD] = factor;
    } else {
        // (is_stat_consumption_stage(): one pass, no look ahead)
        if (refresh_frame.alt_ref_frame || refresh_frame.golden_frame)
            && rc.is_src_frame_alt_ref == 0
            && !cpi.ppi.use_svc
            && (cpi.oxcf.rc_cfg.mode != AOM_CBR || cpi.oxcf.rc_cfg.gf_cbr_boost_pct > 20)
        {
            if is_encode_stage && parallel {
                rc.frame_level_rate_correction_factors[GF_ARF_STD] = factor;
                update_default_rcf = false;
            }
            if update_default_rcf {
                p_rc.rate_correction_factors[GF_ARF_STD] = factor;
            }
        } else {
            if is_encode_stage && parallel {
                rc.frame_level_rate_correction_factors[INTER_NORMAL] = factor;
                update_default_rcf = false;
            }
            if update_default_rcf {
                p_rc.rate_correction_factors[INTER_NORMAL] = factor;
            }
        }
    }
}

/// Translation of `av1_rc_update_rate_correction_factors()` (no cyclic
/// refresh).
pub(crate) fn av1_rc_update_rate_correction_factors(
    cpi: &mut Av1Comp,
    is_encode_stage: bool,
    width: i32,
    height: i32,
) {
    let mut correction_factor = 1.0;
    let mut rate_correction_factor = get_rate_correction_factor(cpi, width, height);
    let adjustment_limit;

    // Do not update the rate factors for arf overlay frames.
    if cpi.rc.is_src_frame_alt_ref != 0 {
        return;
    }

    // Don't update rate correction factors here on scene changes as
    // it is already reset in av1_encodedframe_overshoot_cbr(),
    // but reset variables related to previous frame q and size.
    // Note that the counter of frames since the last scene change
    // is only valid when cyclic refresh mode is enabled and that
    // this break out only applies to scene changes that are not
    // recorded as INTRA only key frames.
    // (no cyclic refresh)
    debug_assert!(cpi.oxcf.q_cfg.aq_mode != CYCLIC_REFRESH_AQ);

    // Clear down mmx registers to allow floating point in what follows

    // Work out how big we would have expected the frame to be at this Q given
    // the current correction factor.
    // Stay in double to avoid int overflow when values are large
    let projected_size_based_on_q = av1_estimate_bits_at_q(
        cpi,
        cpi.common.quant_params.base_qindex,
        rate_correction_factor,
    );
    // Work out a size correction factor.
    if projected_size_based_on_q > FRAME_OVERHEAD_BITS {
        correction_factor = cpi.rc.projected_frame_size as f64 / projected_size_based_on_q as f64;
    }

    // Clamp correction factor to prevent anything too extreme
    correction_factor = correction_factor.max(0.25);

    cpi.rc.q_2_frame = cpi.rc.q_1_frame;
    cpi.rc.q_1_frame = cpi.common.quant_params.base_qindex;
    cpi.rc.rc_2_frame = cpi.rc.rc_1_frame;
    if correction_factor > 1.1 {
        cpi.rc.rc_1_frame = -1;
    } else if correction_factor < 0.9 {
        cpi.rc.rc_1_frame = 1;
    } else {
        cpi.rc.rc_1_frame = 0;
    }

    // Decide how heavily to dampen the adjustment
    if correction_factor > 0.0 {
        if cpi.is_screen_content_type != 0 {
            adjustment_limit = 0.25 + 0.5 * 0.5f64.min(correction_factor.log10().abs());
        } else {
            adjustment_limit = 0.25 + 0.75 * 0.5f64.min(correction_factor.log10().abs());
        }
    } else {
        adjustment_limit = 0.75;
    }

    // Adjustment to delta Q and number of blocks updated in cyclic refressh
    // based on over or under shoot of target in current frame.
    // (no cyclic refresh)

    if correction_factor > 1.01 {
        // We are not already at the worst allowable quality
        correction_factor = 1.0 + ((correction_factor - 1.0) * adjustment_limit);
        rate_correction_factor *= correction_factor;
        // Keep rate_correction_factor within limits
        if rate_correction_factor > MAX_BPB_FACTOR {
            rate_correction_factor = MAX_BPB_FACTOR;
        }
    } else if correction_factor < 0.99 {
        // We are not already at the best allowable quality
        correction_factor = 1.0 / correction_factor;
        correction_factor = 1.0 + ((correction_factor - 1.0) * adjustment_limit);
        correction_factor = 1.0 / correction_factor;

        rate_correction_factor *= correction_factor;

        // Keep rate_correction_factor within limits
        if rate_correction_factor < MIN_BPB_FACTOR {
            rate_correction_factor = MIN_BPB_FACTOR;
        }
    }

    set_rate_correction_factor(cpi, is_encode_stage, rate_correction_factor, width, height);
}

// Calculate rate for the given 'q'.
// (no cyclic refresh: use_cyclic_refresh is 0)
fn get_bits_per_mb(cpi: &Av1Comp, correction_factor: f64, q: i32) -> i32 {
    let cm = &cpi.common;
    av1_rc_bits_per_mb(
        cpi,
        cm.current_frame.frame_type,
        q,
        correction_factor,
        cpi.sf.hl_sf.accurate_bit_estimate,
    )
}

/*\brief Searches for a Q index value predicted to give an average macro
 * block rate closest to the target value.
 *
 * Similar to find_qindex_by_rate() function, but returns a q index with a
 * rate just above or below the desired rate, depending on which of the two
 * rates is closer to the desired rate.
 * Also, respects the selected aq_mode when computing the rate.
 *
 * \ingroup rate_control
 * \param[in]   desired_bits_per_mb   Target bits per mb
 * \param[in]   cpi                   Top level encoder instance structure
 * \param[in]   correction_factor     Current Q to rate correction factor
 * \param[in]   best_qindex           Min allowed Q value.
 * \param[in]   worst_qindex          Max allowed Q value.
 *
 * \return Returns a correction factor for the current frame
 */
fn find_closest_qindex_by_rate(
    desired_bits_per_mb: i32,
    cpi: &Av1Comp,
    correction_factor: f64,
    best_qindex: i32,
    worst_qindex: i32,
) -> i32 {
    // (no cyclic refresh)

    // Find 'qindex' based on 'desired_bits_per_mb'.
    debug_assert!(best_qindex <= worst_qindex);
    let mut low = best_qindex;
    let mut high = worst_qindex;
    while low < high {
        let mid = (low + high) >> 1;
        let mid_bits_per_mb = get_bits_per_mb(cpi, correction_factor, mid);
        if mid_bits_per_mb > desired_bits_per_mb {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    debug_assert!(low == high);

    // Calculate rate difference of this q index from the desired rate.
    let curr_q = low;
    let curr_bits_per_mb = get_bits_per_mb(cpi, correction_factor, curr_q);
    let curr_bit_diff = if curr_bits_per_mb <= desired_bits_per_mb {
        desired_bits_per_mb - curr_bits_per_mb
    } else {
        i32::MAX
    };
    debug_assert!((curr_bit_diff != i32::MAX && curr_bit_diff >= 0) || curr_q == worst_qindex);

    // Calculate rate difference for previous q index too.
    let prev_q = curr_q - 1;
    let prev_bit_diff;
    if curr_bit_diff == i32::MAX || curr_q == best_qindex {
        prev_bit_diff = i32::MAX;
    } else {
        let prev_bits_per_mb = get_bits_per_mb(cpi, correction_factor, prev_q);
        debug_assert!(prev_bits_per_mb > desired_bits_per_mb);
        prev_bit_diff = prev_bits_per_mb - desired_bits_per_mb;
    }

    // Pick one of the two q indices, depending on which one has rate closer to
    // the desired rate.
    if curr_bit_diff <= prev_bit_diff {
        curr_q
    } else {
        prev_q
    }
}

/// Translation of `av1_rc_regulate_q()`.
pub(crate) fn av1_rc_regulate_q(
    cpi: &Av1Comp,
    target_bits_per_frame: i32,
    active_best_quality: i32,
    active_worst_quality: i32,
    width: i32,
    height: i32,
) -> i32 {
    let mbs = av1_get_mbs(width, height);
    let correction_factor = get_rate_correction_factor(cpi, width, height);
    let target_bits_per_mb =
        (((target_bits_per_frame as u64) << BPER_MB_NORMBITS) / mbs as u64) as i32;

    let q = find_closest_qindex_by_rate(
        target_bits_per_mb,
        cpi,
        correction_factor,
        active_best_quality,
        active_worst_quality,
    );
    if cpi.oxcf.rc_cfg.mode == AOM_CBR && has_no_stats_stage(cpi) {
        return adjust_q_cbr(cpi, q, active_worst_quality, width, height);
    }

    q
}

fn get_active_quality(
    q: i32,
    gfu_boost: i32,
    low: i32,
    high: i32,
    low_motion_minq: &[i32],
    high_motion_minq: &[i32],
) -> i32 {
    let q = q as usize;
    if gfu_boost > high {
        low_motion_minq[q]
    } else if gfu_boost < low {
        high_motion_minq[q]
    } else {
        let gap = high - low;
        let offset = high - gfu_boost;
        let qdiff = high_motion_minq[q] - low_motion_minq[q];
        let adjustment = ((offset * qdiff) + (gap >> 1)) / gap;
        low_motion_minq[q] + adjustment
    }
}

fn get_kf_active_quality(p_rc: &PrimaryRateControl, q: i32, bit_depth: i32) -> i32 {
    let luts = minq_luts(bit_depth);
    get_active_quality(
        q,
        p_rc.kf_boost,
        KF_LOW,
        KF_HIGH,
        &luts.kf_low_motion_minq,
        &luts.kf_high_motion_minq,
    )
}

fn get_gf_active_quality_no_rc(gfu_boost: i32, q: i32, bit_depth: i32) -> i32 {
    let luts = minq_luts(bit_depth);
    get_active_quality(
        q,
        gfu_boost,
        GF_LOW,
        GF_HIGH,
        &luts.arfgf_low_motion_minq,
        &luts.arfgf_high_motion_minq,
    )
}

fn get_gf_active_quality(p_rc: &PrimaryRateControl, q: i32, bit_depth: i32) -> i32 {
    get_gf_active_quality_no_rc(p_rc.gfu_boost, q, bit_depth)
}

// Adjust active_worst_quality level based on buffer level.
fn calc_active_worst_quality_no_stats_cbr(cpi: &Av1Comp) -> i32 {
    // Adjust active_worst_quality: If buffer is above the optimal/target level,
    // bring active_worst_quality down depending on fullness of buffer.
    // If buffer is below the optimal level, let the active_worst_quality go from
    // ambient Q (at buffer = optimal level) to worst_quality level
    // (at buffer = critical level).
    let cm = &cpi.common;
    let rc = &cpi.rc;
    let p_rc = &cpi.ppi.p_rc;
    let svc = &cpi.svc;
    let num_frames_weight_key = 5 * svc.number_temporal_layers as u32;
    // Buffer level below which we push active_worst to worst_quality.
    let critical_level = p_rc.optimal_buffer_level >> 3;
    let buff_lvl_step;
    let mut adjustment = 0;
    let mut active_worst_quality;
    let ambient_qp;
    if cm.current_frame.frame_type == KEY_FRAME {
        return rc.worst_quality;
    }
    // For ambient_qp we use minimum of avg_frame_qindex[KEY_FRAME/INTER_FRAME]
    // for the first few frames following key frame. These are both initialized
    // to worst_quality and updated with (3/4, 1/4) average in postencode_update.
    // So for first few frames following key, the qp of that key frame is weighted
    // into the active_worst_quality setting. For SVC the key frame should
    // correspond to layer (0, 0), so use that for layer context.
    let avg_qindex_key = p_rc.avg_frame_qindex[KEY_FRAME as usize];
    // (one temporal layer)
    debug_assert!(svc.number_temporal_layers <= 1);
    ambient_qp = if cm.current_frame.frame_number < num_frames_weight_key {
        p_rc.avg_frame_qindex[INTER_FRAME as usize].min(avg_qindex_key)
    } else {
        p_rc.avg_frame_qindex[INTER_FRAME as usize]
    };
    active_worst_quality = rc.worst_quality.min(ambient_qp * 5 / 4);
    if p_rc.buffer_level > p_rc.optimal_buffer_level {
        // Adjust down.
        // Maximum limit for down adjustment, ~30%.
        let max_adjustment_down = active_worst_quality / 3;
        if max_adjustment_down != 0 {
            buff_lvl_step =
                (p_rc.maximum_buffer_size - p_rc.optimal_buffer_level) / max_adjustment_down as i64;
            if buff_lvl_step != 0 {
                adjustment =
                    ((p_rc.buffer_level - p_rc.optimal_buffer_level) / buff_lvl_step) as i32;
            }
            active_worst_quality -= adjustment;
        }
    } else if p_rc.buffer_level > critical_level {
        // Adjust up from ambient Q.
        if critical_level != 0 {
            buff_lvl_step = p_rc.optimal_buffer_level - critical_level;
            if buff_lvl_step != 0 {
                adjustment = ((rc.worst_quality - ambient_qp) as i64
                    * (p_rc.optimal_buffer_level - p_rc.buffer_level)
                    / buff_lvl_step) as i32;
            }
            active_worst_quality = ambient_qp + adjustment;
        }
    } else {
        // Set to worst_quality if buffer is below critical level.
        active_worst_quality = rc.worst_quality;
    }
    active_worst_quality
}

// Calculate the active_best_quality level.
fn calc_active_best_quality_no_stats_cbr(
    cpi: &Av1Comp,
    active_worst_quality: i32,
    width: i32,
    height: i32,
) -> i32 {
    let cm = &cpi.common;
    let rc = &cpi.rc;
    let p_rc = &cpi.ppi.p_rc;
    let refresh_frame = &cpi.refresh_frame;
    let current_frame = &cm.current_frame;
    let bit_depth = cm.seq_params.bit_depth;
    let mut active_best_quality = rc.best_quality;
    let rtc_minq = &minq_luts(bit_depth).rtc_minq;

    if frame_is_intra_only(cm) {
        // Handle the special case for key frames forced when we have reached
        // the maximum key frame interval. Here force the Q to a range
        // based on the ambient Q to reduce the risk of popping.
        if p_rc.this_key_frame_forced != 0 {
            let qindex = p_rc.last_boosted_qindex;
            let last_boosted_q = av1_convert_qindex_to_q(qindex, bit_depth);
            let delta_qindex =
                av1_compute_qdelta(rc, last_boosted_q, last_boosted_q * 0.75, bit_depth);
            active_best_quality = (qindex + delta_qindex).max(rc.best_quality);
        } else if current_frame.frame_number > 0 {
            // not first frame of one pass and kf_boost is set
            let mut q_adj_factor = 1.0;
            active_best_quality =
                get_kf_active_quality(p_rc, p_rc.avg_frame_qindex[KEY_FRAME as usize], bit_depth);
            // Allow somewhat lower kf minq with small image formats.
            if (width * height) <= (352 * 288) {
                q_adj_factor -= 0.25;
            }
            // Convert the adjustment factor to a qindex delta
            // on active_best_quality.
            let q_val = av1_convert_qindex_to_q(active_best_quality, bit_depth);
            active_best_quality += av1_compute_qdelta(rc, q_val, q_val * q_adj_factor, bit_depth);
        }
    } else if rc.is_src_frame_alt_ref == 0
        && !cpi.ppi.use_svc
        && cpi.oxcf.rc_cfg.gf_cbr_boost_pct != 0
        && (refresh_frame.golden_frame || refresh_frame.alt_ref_frame)
    {
        // Use the lower of active_worst_quality and recent
        // average Q as basis for GF/ARF best Q limit unless last frame was
        // a key frame.
        let mut q = active_worst_quality;
        if rc.frames_since_key > 1
            && p_rc.avg_frame_qindex[INTER_FRAME as usize] < active_worst_quality
        {
            q = p_rc.avg_frame_qindex[INTER_FRAME as usize];
        }
        active_best_quality = get_gf_active_quality(p_rc, q, bit_depth);
    } else {
        // Use the lower of active_worst_quality and recent/average Q.
        let frame_type = if current_frame.frame_number > 1 {
            INTER_FRAME
        } else {
            KEY_FRAME
        };
        if p_rc.avg_frame_qindex[frame_type as usize] < active_worst_quality {
            active_best_quality = rtc_minq[p_rc.avg_frame_qindex[frame_type as usize] as usize];
        } else {
            active_best_quality = rtc_minq[active_worst_quality as usize];
        }
    }
    active_best_quality
}

/*\brief Picks q and q bounds given CBR rate control parameters in \c cpi->rc.
 *
 * Handles the special case when using:
 * - Constant bit-rate mode: \c cpi->oxcf.rc_cfg.mode == \ref AOM_CBR, and
 * - 1-pass encoding without LAP (look-ahead processing), so 1st pass stats are
 * NOT available.
 *
 * \ingroup rate_control
 * \param[in]       cpi          Top level encoder structure
 * \param[in]       width        Coded frame width
 * \param[in]       height       Coded frame height
 * \param[out]      bottom_index Bottom bound for q index (best quality)
 * \param[out]      top_index    Top bound for q index (worst quality)
 * \return Returns selected q index to be used for encoding this frame.
 */
fn rc_pick_q_and_bounds_no_stats_cbr(
    cpi: &Av1Comp,
    width: i32,
    height: i32,
    bottom_index: &mut i32,
    top_index: &mut i32,
) -> i32 {
    let cm = &cpi.common;
    let rc = &cpi.rc;
    let p_rc = &cpi.ppi.p_rc;
    let current_frame = &cm.current_frame;
    let mut q;
    let mut active_worst_quality = calc_active_worst_quality_no_stats_cbr(cpi);
    let mut active_best_quality =
        calc_active_best_quality_no_stats_cbr(cpi, active_worst_quality, width, height);
    debug_assert!(has_no_stats_stage(cpi));
    debug_assert!(cpi.oxcf.rc_cfg.mode == AOM_CBR);

    // Clip the active best and worst quality values to limits
    active_best_quality = active_best_quality.clamp(rc.best_quality, rc.worst_quality);
    active_worst_quality = active_worst_quality.clamp(active_best_quality, rc.worst_quality);

    *top_index = active_worst_quality;
    *bottom_index = active_best_quality;

    // Limit Q range for the adaptive loop.
    if current_frame.frame_type == KEY_FRAME
        && p_rc.this_key_frame_forced == 0
        && current_frame.frame_number != 0
    {
        let qdelta =
            av1_compute_qdelta_by_rate(cpi, current_frame.frame_type, active_worst_quality, 2.0);
        *top_index = active_worst_quality + qdelta;
        *top_index = (*top_index).max(*bottom_index);
    }

    q = av1_rc_regulate_q(
        cpi,
        rc.this_frame_target,
        active_best_quality,
        active_worst_quality,
        width,
        height,
    );
    // (RT_PASSIVE_STRATEGY is 0)
    if q > *top_index {
        // Special case when we are targeting the max allowed rate
        if rc.this_frame_target >= rc.max_frame_bandwidth {
            *top_index = q;
        } else {
            q = *top_index;
        }
    }

    // Special case: we force the first few frames to use low q such that
    // these frames are encoded at a high quality, which provides good
    // references for following frames.
    if current_frame.frame_type != KEY_FRAME
        && !cpi.ppi.use_svc
        && current_frame.frame_number >= 10
        && current_frame.frame_number <= 15
    {
        q = (p_rc.last_kf_qindex + 108).min(5.max(q - 9));
        q = q.max(rc.best_quality);
    }

    debug_assert!(*top_index <= rc.worst_quality && *top_index >= rc.best_quality);
    debug_assert!(*bottom_index <= rc.worst_quality && *bottom_index >= rc.best_quality);
    debug_assert!(q <= rc.worst_quality && q >= rc.best_quality);
    q
}

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

/// Translation of `av1_rc_pick_q_and_bounds()` (CBR without statistics,
/// `AOM_Q` otherwise).
pub(crate) fn av1_rc_pick_q_and_bounds(
    cpi: &mut Av1Comp,
    width: i32,
    height: i32,
    gf_index: usize,
    bottom_index: &mut i32,
    top_index: &mut i32,
) -> i32 {
    let q;
    // TODO(sarahparker) merge no-stats vbr and altref q computation
    // with rc_pick_q_and_bounds().
    let update_type = cpi.ppi.gf_group.update_type[gf_index];
    if (cpi.oxcf.rc_cfg.mode != AOM_Q || update_type == ARF_UPDATE) && has_no_stats_stage(cpi) {
        // (the one pass VBR choice: the encoders use AOM_Q or AOM_CBR)
        debug_assert!(cpi.oxcf.rc_cfg.mode == AOM_CBR);
        // TODO(yunqing): the results could be used for encoder optimization.
        cpi.rec_sse = u64::MAX;
        // (rc_compute_variance_onepass_rt(): accurate_bit_estimate is off at
        // the real-time speed)
        debug_assert!(
            !(cpi.sf.hl_sf.accurate_bit_estimate != 0
                && cpi.common.current_frame.frame_type != KEY_FRAME)
        );
        q = rc_pick_q_and_bounds_no_stats_cbr(cpi, width, height, bottom_index, top_index);
        // preserve copy of active worst quality selected.
        cpi.rc.active_worst_quality = *top_index;
    } else {
        q = rc_pick_q_and_bounds(cpi, gf_index, bottom_index, top_index);
    }
    if update_type == ARF_UPDATE {
        cpi.ppi.p_rc.arf_q = q;
    }

    q
}

/// Translation of `av1_rc_set_frame_target()`.
pub(crate) fn av1_rc_set_frame_target(cpi: &mut Av1Comp, target: i32, width: i32, height: i32) {
    let cm = &cpi.common;
    let rc = &mut cpi.rc;

    rc.this_frame_target = target;

    // Modify frame size target when down-scaled.
    if av1_frame_scaled(cm) && cpi.oxcf.rc_cfg.mode != AOM_CBR {
        rc.this_frame_target = (rc.this_frame_target as f64
            * resize_rate_factor(&cpi.oxcf.frm_dim_cfg, width, height))
            as i32;
    }

    // Target rate per SB64 (including partial SB64s.
    rc.sb64_target_rate = (((rc.this_frame_target as i64) << 12) / (width * height) as i64) as i32;
}

fn update_alt_ref_frame_stats(cpi: &mut Av1Comp) {
    // this frame refreshes means next frames don't unless specified by user
    let rc = &mut cpi.rc;
    rc.frames_since_golden = 0;
}

fn update_golden_frame_stats(cpi: &mut Av1Comp) {
    let show_frame = cpi.common.show_frame;
    let golden_frame = cpi.refresh_frame.golden_frame;
    let rc = &mut cpi.rc;

    // Update the Golden frame usage counts.
    if golden_frame || rc.is_src_frame_alt_ref != 0 {
        rc.frames_since_golden = 0;
    } else if show_frame != 0 {
        rc.frames_since_golden += 1;
    }
}

/// Translation of `av1_rc_postencode_update()`.
pub(crate) fn av1_rc_postencode_update(cpi: &mut Av1Comp, bytes_used: u64) {
    let is_intrnl_arf =
        cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize] == INTNL_ARF_UPDATE;

    let qindex = cpi.common.quant_params.base_qindex;
    let frame_type = cpi.common.current_frame.frame_type;

    // (RT_PASSIVE_STRATEGY is 0)

    // Update rate control heuristics
    cpi.rc.projected_frame_size = (bytes_used << 3) as i32;

    // Post encode loop adjustment of Q prediction.
    let (width, height) = (cpi.common.width, cpi.common.height);
    av1_rc_update_rate_correction_factors(cpi, false, width, height);

    let refresh_frame = cpi.refresh_frame;
    let bit_depth = cpi.common.seq_params.bit_depth;
    let rc = &mut cpi.rc;
    let p_rc = &mut cpi.ppi.p_rc;
    // Keep a record of last Q and ambient average Q.
    if frame_type == KEY_FRAME {
        p_rc.last_q[KEY_FRAME as usize] = qindex;
        p_rc.avg_frame_qindex[KEY_FRAME as usize] =
            round_power_of_two(3 * p_rc.avg_frame_qindex[KEY_FRAME as usize] + qindex, 2);
    } else {
        if (cpi.ppi.use_svc && cpi.oxcf.rc_cfg.mode == AOM_CBR)
            || rc.rtc_external_ratectrl
            || (rc.is_src_frame_alt_ref == 0
                && !(refresh_frame.golden_frame || is_intrnl_arf || refresh_frame.alt_ref_frame))
        {
            p_rc.last_q[INTER_FRAME as usize] = qindex;
            p_rc.avg_frame_qindex[INTER_FRAME as usize] =
                round_power_of_two(3 * p_rc.avg_frame_qindex[INTER_FRAME as usize] + qindex, 2);
            p_rc.ni_frames += 1;
            p_rc.tot_q += av1_convert_qindex_to_q(qindex, bit_depth);
            p_rc.avg_q = p_rc.tot_q / p_rc.ni_frames as f64;
            // Calculate the average Q for normal inter frames (not key or GFU
            // frames).
            rc.ni_tot_qi += qindex;
            rc.ni_av_qi = rc.ni_tot_qi / p_rc.ni_frames;
        }
    }
    // Keep record of last boosted (KF/GF/ARF) Q value.
    // If the current frame is coded at a lower Q then we also update it.
    // If all mbs in this group are skipped only update if the Q value is
    // better than that already stored.
    // This is used to help set quality in forced key frames to reduce popping
    if (qindex < p_rc.last_boosted_qindex)
        || (frame_type == KEY_FRAME)
        || (p_rc.constrained_gf_group == 0
            && (refresh_frame.alt_ref_frame
                || is_intrnl_arf
                || (refresh_frame.golden_frame && rc.is_src_frame_alt_ref == 0)))
    {
        p_rc.last_boosted_qindex = qindex;
    }
    if frame_type == KEY_FRAME {
        p_rc.last_kf_qindex = qindex;
    }

    let projected_frame_size = rc.projected_frame_size;
    update_buffer_level(cpi, projected_frame_size);
    let rc = &mut cpi.rc;
    rc.prev_avg_frame_bandwidth = rc.avg_frame_bandwidth;

    // Rolling monitors of whether we are over or underspending used to help
    // regulate min and Max Q in two pass.
    if av1_frame_scaled(&cpi.common) {
        rc.this_frame_target = (rc.this_frame_target as f64
            / resize_rate_factor(&cpi.oxcf.frm_dim_cfg, cpi.common.width, cpi.common.height))
            as i32;
    }
    let p_rc = &mut cpi.ppi.p_rc;
    if frame_type != KEY_FRAME {
        p_rc.rolling_target_bits = round_power_of_two_64(
            p_rc.rolling_target_bits as i64 * 3 + rc.this_frame_target as i64,
            2,
        ) as i32;
        p_rc.rolling_actual_bits = round_power_of_two_64(
            p_rc.rolling_actual_bits as i64 * 3 + rc.projected_frame_size as i64,
            2,
        ) as i32;
    }

    // Actual bits spent
    p_rc.total_actual_bits += rc.projected_frame_size as i64;
    p_rc.total_target_bits += if cpi.common.show_frame != 0 {
        rc.avg_frame_bandwidth as i64
    } else {
        0
    };

    if is_altref_enabled(
        cpi.oxcf.gf_cfg.lag_in_frames,
        cpi.oxcf.gf_cfg.enable_auto_arf,
    ) && refresh_frame.alt_ref_frame
        && (frame_type != KEY_FRAME && !frame_is_sframe(&cpi.common))
    {
        // Update the alternate reference frame stats as appropriate.
        update_alt_ref_frame_stats(cpi);
    } else {
        // Update the Golden frame stats as appropriate.
        update_golden_frame_stats(cpi);
    }

    let rc = &mut cpi.rc;
    if frame_type == KEY_FRAME {
        rc.frames_since_key = 0;
    }
    if cpi.refresh_frame.golden_frame {
        rc.frame_num_last_gf_refresh = cpi.common.current_frame.frame_number as i32;
    }
    rc.prev_coded_width = cpi.common.width;
    rc.prev_coded_height = cpi.common.height;
    // if (current_frame->frame_number == 1 && cm->show_frame)
    /*
    rc->this_frame_target =
        (int)(rc->this_frame_target / resize_rate_factor(&cpi->oxcf.frm_dim_cfg,
    cm->width, cm->height));
        */
}

/// Translation of `av1_find_qindex()`.
pub(crate) fn av1_find_qindex(
    desired_q: f64,
    bit_depth: i32,
    best_qindex: i32,
    worst_qindex: i32,
) -> i32 {
    debug_assert!(best_qindex <= worst_qindex);
    let mut low = best_qindex;
    let mut high = worst_qindex;
    while low < high {
        let mid = (low + high) >> 1;
        let mid_q = av1_convert_qindex_to_q(mid, bit_depth);
        if mid_q < desired_q {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    debug_assert!(low == high);
    debug_assert!(av1_convert_qindex_to_q(low, bit_depth) >= desired_q || low == worst_qindex);
    low
}

/// Translation of `av1_compute_qdelta()`.
pub(crate) fn av1_compute_qdelta(
    rc: &RateControl,
    qstart: f64,
    qtarget: f64,
    bit_depth: i32,
) -> i32 {
    let start_index = av1_find_qindex(qstart, bit_depth, rc.best_quality, rc.worst_quality);
    let target_index = av1_find_qindex(qtarget, bit_depth, rc.best_quality, rc.worst_quality);
    target_index - start_index
}

// Find q_index for the desired_bits_per_mb, within [best_qindex, worst_qindex],
// assuming 'correction_factor' is 1.0.
// To be precise, 'q_index' is the smallest integer, for which the corresponding
// bits per mb <= desired_bits_per_mb.
// If no such q index is found, returns 'worst_qindex'.
fn find_qindex_by_rate(
    cpi: &Av1Comp,
    desired_bits_per_mb: i32,
    frame_type: FrameType,
    best_qindex: i32,
    worst_qindex: i32,
) -> i32 {
    debug_assert!(best_qindex <= worst_qindex);
    let mut low = best_qindex;
    let mut high = worst_qindex;
    while low < high {
        let mid = (low + high) >> 1;
        let mid_bits_per_mb = av1_rc_bits_per_mb(cpi, frame_type, mid, 1.0, 0);
        if mid_bits_per_mb > desired_bits_per_mb {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    debug_assert!(low == high);
    debug_assert!(
        av1_rc_bits_per_mb(cpi, frame_type, low, 1.0, 0) <= desired_bits_per_mb
            || low == worst_qindex
    );
    low
}

/// Translation of `av1_compute_qdelta_by_rate()`.
pub(crate) fn av1_compute_qdelta_by_rate(
    cpi: &Av1Comp,
    frame_type: FrameType,
    qindex: i32,
    rate_target_ratio: f64,
) -> i32 {
    let rc = &cpi.rc;

    // Look up the current projected bits per block for the base index
    let base_bits_per_mb = av1_rc_bits_per_mb(cpi, frame_type, qindex, 1.0, 0);

    // Find the target bits per mb based on the base value and given ratio.
    let target_bits_per_mb = (rate_target_ratio * base_bits_per_mb as f64) as i32;

    let target_index = find_qindex_by_rate(
        cpi,
        target_bits_per_mb,
        frame_type,
        rc.best_quality,
        rc.worst_quality,
    );
    target_index - qindex
}

/// Translation of `av1_rc_set_gf_interval_range()`.
pub(crate) fn av1_rc_set_gf_interval_range(cpi: &Av1Comp, rc: &mut RateControl) {
    let oxcf = &cpi.oxcf;

    // Special case code for 1 pass fixed Q mode tests
    if has_no_stats_stage(cpi) && oxcf.rc_cfg.mode == AOM_Q {
        rc.max_gf_interval = oxcf.gf_cfg.max_gf_interval;
        rc.min_gf_interval = oxcf.gf_cfg.min_gf_interval;
        rc.static_scene_max_gf_interval = rc.min_gf_interval + 1;
    } else {
        // Set Maximum gf/arf interval
        rc.max_gf_interval = oxcf.gf_cfg.max_gf_interval;
        rc.min_gf_interval = oxcf.gf_cfg.min_gf_interval;
        if rc.min_gf_interval == 0 {
            rc.min_gf_interval = av1_rc_get_default_min_gf_interval(
                oxcf.frm_dim_cfg.width,
                oxcf.frm_dim_cfg.height,
                cpi.framerate,
            );
        }
        if rc.max_gf_interval == 0 {
            rc.max_gf_interval =
                av1_rc_get_default_max_gf_interval(cpi.framerate, rc.min_gf_interval);
        }
        /*
         * Extended max interval for genuinely static scenes like slide shows.
         * The no.of.stats available in the case of LAP is limited,
         * hence setting to max_gf_interval.
         */
        if cpi.ppi.lap_enabled {
            rc.static_scene_max_gf_interval = rc.max_gf_interval + 1;
        } else {
            rc.static_scene_max_gf_interval = MAX_STATIC_GF_GROUP_LENGTH as i32;
        }

        if rc.max_gf_interval > rc.static_scene_max_gf_interval {
            rc.max_gf_interval = rc.static_scene_max_gf_interval;
        }

        // Clamp min to max
        rc.min_gf_interval = rc.min_gf_interval.min(rc.max_gf_interval);
    }
}

/// Translation of `av1_rc_update_framerate()`.
pub(crate) fn av1_rc_update_framerate(cpi: &mut Av1Comp, width: i32, height: i32) {
    let oxcf = &cpi.oxcf;
    let mut rc = cpi.rc;
    let mbs = av1_get_mbs(width, height);

    rc.avg_frame_bandwidth = (oxcf.rc_cfg.target_bandwidth as f64 / cpi.framerate).round() as i32;
    rc.min_frame_bandwidth = rc
        .avg_frame_bandwidth
        .wrapping_mul(oxcf.rc_cfg.vbrmin_section)
        / 100;

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

/// Translation of `av1_calc_pframe_target_size_one_pass_cbr()`.
pub(crate) fn av1_calc_pframe_target_size_one_pass_cbr(
    cpi: &Av1Comp,
    frame_update_type: u8,
) -> i32 {
    let _ = frame_update_type;
    let oxcf = &cpi.oxcf;
    let rc = &cpi.rc;
    let p_rc = &cpi.ppi.p_rc;
    let rc_cfg = &oxcf.rc_cfg;
    let diff = p_rc.optimal_buffer_level - p_rc.buffer_level;
    let one_pct_bits = 1 + p_rc.optimal_buffer_level / 100;
    let min_frame_target = (rc.avg_frame_bandwidth >> 4).max(FRAME_OVERHEAD_BITS);
    let mut target;

    if rc_cfg.gf_cbr_boost_pct != 0 {
        let af_ratio_pct = rc_cfg.gf_cbr_boost_pct as i32 + 100;
        if frame_update_type == GF_UPDATE || frame_update_type == OVERLAY_UPDATE {
            target = (rc.avg_frame_bandwidth * p_rc.baseline_gf_interval * af_ratio_pct)
                / (p_rc.baseline_gf_interval * 100 + af_ratio_pct - 100);
        } else {
            target = (rc.avg_frame_bandwidth * p_rc.baseline_gf_interval * 100)
                / (p_rc.baseline_gf_interval * 100 + af_ratio_pct - 100);
        }
    } else {
        target = rc.avg_frame_bandwidth;
    }
    // (cpi->ppi->use_svc: one layer)
    debug_assert!(!cpi.ppi.use_svc);
    if diff > 0 {
        // Lower the target bandwidth for this frame.
        let pct_low = (diff / one_pct_bits).min(rc_cfg.under_shoot_pct as i64) as i32;
        target -= (target * pct_low) / 200;
    } else if diff < 0 {
        // Increase the target bandwidth for this frame.
        let pct_high = (-diff / one_pct_bits).min(rc_cfg.over_shoot_pct as i64) as i32;
        target += (target * pct_high) / 200;
    }
    if rc_cfg.max_inter_bitrate_pct != 0 {
        let max_rate = rc.avg_frame_bandwidth * rc_cfg.max_inter_bitrate_pct as i32 / 100;
        target = target.min(max_rate);
    }
    min_frame_target.max(target)
}

/// Translation of `av1_calc_iframe_target_size_one_pass_cbr()`.
pub(crate) fn av1_calc_iframe_target_size_one_pass_cbr(cpi: &Av1Comp) -> i32 {
    let rc = &cpi.rc;
    let p_rc = &cpi.ppi.p_rc;
    let mut target: i64;
    if cpi.common.current_frame.frame_number == 0 {
        target = if (p_rc.starting_buffer_level / 2) > i32::MAX as i64 {
            i32::MAX as i64
        } else {
            (p_rc.starting_buffer_level / 2) as i32 as i64
        };
        if cpi.svc.number_temporal_layers > 1 && target < (i32::MAX >> 2) as i64 {
            target <<= 2.min(cpi.svc.number_temporal_layers - 1);
        }
    } else {
        let mut kf_boost = 32;
        let framerate = cpi.framerate.round() as i32;

        kf_boost = kf_boost.max(2 * framerate - 16);
        if rc.frames_since_key < framerate / 2 {
            kf_boost = kf_boost * rc.frames_since_key / (framerate / 2);
        }
        target = (((16 + kf_boost) * rc.avg_frame_bandwidth) >> 4) as i64;
    }
    av1_rc_clamp_iframe_target_size(cpi, target)
}

fn set_golden_update(cpi: &mut Av1Comp) {
    let rc = &cpi.rc;
    let divisor = 10;
    // (no cyclic refresh: the divisor stays 10)
    debug_assert!(cpi.oxcf.q_cfg.aq_mode != CYCLIC_REFRESH_AQ);

    // Set minimum gf_interval for GF update to a multiple of the refresh period,
    // with some max limit. Depending on past encoding stats, GF flag may be
    // reset and update may not occur until next baseline_gf_interval.
    let gf_length_mult: [i32; 2] = [8, 4];
    let p_rc = &mut cpi.ppi.p_rc;
    if divisor > 0 {
        p_rc.baseline_gf_interval = (gf_length_mult[cpi.sf.rt_sf.gf_length_lvl as usize]
            * (100 / divisor))
            .min(MAX_GF_INTERVAL_RT);
    } else {
        p_rc.baseline_gf_interval = FIXED_GF_INTERVAL_RT;
    }
    if rc.avg_frame_low_motion != 0 && rc.avg_frame_low_motion < 40 {
        p_rc.baseline_gf_interval = 16;
    }
}

fn set_baseline_gf_interval(cpi: &mut Av1Comp, frame_type: FrameType) {
    set_golden_update(cpi);

    let rc = &mut cpi.rc;
    let p_rc = &mut cpi.ppi.p_rc;
    if p_rc.baseline_gf_interval > rc.frames_to_key && cpi.oxcf.kf_cfg.auto_key {
        p_rc.baseline_gf_interval = rc.frames_to_key;
    }
    p_rc.gfu_boost = DEFAULT_GF_BOOST_RT;
    p_rc.constrained_gf_group =
        if p_rc.baseline_gf_interval >= rc.frames_to_key && cpi.oxcf.kf_cfg.auto_key {
            1
        } else {
            0
        };
    rc.frames_till_gf_update_due = p_rc.baseline_gf_interval;
    cpi.gf_frame_index = 0;
    // SVC does not use GF as periodic boost.
    // TODO(marpan): Find better way to disable this for SVC.
    debug_assert!(!cpi.ppi.use_svc);
    let gf_group = &mut cpi.ppi.gf_group;
    gf_group.size = p_rc.baseline_gf_interval;
    gf_group.update_type[0] = if frame_type == KEY_FRAME {
        KF_UPDATE
    } else {
        GF_UPDATE
    };
    gf_group.refbuf_state[cpi.gf_frame_index as usize] = if frame_type == KEY_FRAME {
        REFBUF_RESET
    } else {
        REFBUF_UPDATE
    };
}

/// Translation of `av1_adjust_gf_refresh_qp_one_pass_rt()`.
pub(crate) fn av1_adjust_gf_refresh_qp_one_pass_rt(cpi: &mut Av1Comp) {
    let resize_pending = is_frame_resize_pending(cpi);
    if !resize_pending && cpi.rc.high_source_sad == 0 {
        // Check if we should disable GF refresh (if period is up),
        // or force a GF refresh update (if we are at least halfway through
        // period) based on QP. Look into add info on segment deltaq.
        let rc = &cpi.rc;
        let p_rc = &cpi.ppi.p_rc;
        let avg_qp = p_rc.avg_frame_qindex[INTER_FRAME as usize];
        let allow_gf_update = rc.frames_till_gf_update_due <= (p_rc.baseline_gf_interval - 10);
        let mut gf_update_changed = false;
        let thresh = 87;
        let base_qindex = cpi.common.quant_params.base_qindex;
        if (cpi.common.current_frame.frame_number as i32 - cpi.rc.frame_num_last_gf_refresh)
            < FIXED_GF_INTERVAL_RT
            && rc.frames_till_gf_update_due == 1
            && base_qindex > avg_qp
        {
            // Disable GF refresh since QP is above the running average QP.
            let rtc_ref = &mut cpi.ppi.rtc_ref;
            rtc_ref.refresh[rtc_ref.gld_idx_1layer as usize] = 0;
            gf_update_changed = true;
            cpi.refresh_frame.golden_frame = false;
        } else if allow_gf_update
            && ((base_qindex < thresh * avg_qp / 100)
                || (rc.avg_frame_low_motion != 0 && rc.avg_frame_low_motion < 20))
        {
            // Force refresh since QP is well below average QP or this is a high
            // motion frame.
            let rtc_ref = &mut cpi.ppi.rtc_ref;
            rtc_ref.refresh[rtc_ref.gld_idx_1layer as usize] = 1;
            gf_update_changed = true;
            cpi.refresh_frame.golden_frame = true;
        }
        if gf_update_changed {
            set_baseline_gf_interval(cpi, INTER_FRAME);
            let rtc_ref = &cpi.ppi.rtc_ref;
            let mut refresh_mask = 0;
            for i in 0..INTER_REFS_PER_FRAME as usize {
                let ref_frame_map_idx = rtc_ref.ref_idx[i] as usize;
                refresh_mask |= rtc_ref.refresh[ref_frame_map_idx] << ref_frame_map_idx;
            }
            cpi.common.current_frame.refresh_frame_flags = refresh_mask;
        }
    }
}

/*\brief Setup the reference prediction structure for 1 pass real-time
 *
 * Set the reference prediction structure for 1 layer.
 * Current structue is to use 3 references (LAST, GOLDEN, ALTREF),
 * where ALT_REF always behind current by lag_alt frames, and GOLDEN is
 * either updated on LAST with period baseline_gf_interval (fixed slot)
 * or always behind current by lag_gld (gld_fixed_slot = 0, lag_gld <= 7).
 *
 * \ingroup rate_control
 * \param[in]       cpi          Top level encoder structure
 * \param[in]       gf_update    Flag to indicate if GF is updated
 *
 * \remark Nothing is returned. Instead the settings for the prediction
 * structure are set in \c cpi-ext_flags; and the buffer slot index
 * (for each of 7 references) and refresh flags (for each of the 8 slots)
 * are set in \c cpi->svc.ref_idx[] and \c cpi->svc.refresh[].
 */
pub(crate) fn av1_set_rtc_reference_structure_one_layer(cpi: &mut Av1Comp, gf_update: bool) {
    let frame_number = cpi.common.current_frame.frame_number;
    let frame_type = cpi.common.current_frame.frame_type;
    let rc = &cpi.rc;
    let ext_flags = &mut cpi.ext_flags;
    let rtc_ref = &mut cpi.ppi.rtc_ref;
    let mut lag_alt: u32 = 4;
    let mut last_idx = 0;
    let last_idx_refresh;
    let gld_idx;
    let mut alt_ref_idx = 0;
    let mut last2_idx = 0;
    ext_flags.refresh_frame.update_pending = true;
    ext_flags.ref_frame_flags = 0;
    ext_flags.refresh_frame.last_frame = true;
    ext_flags.refresh_frame.golden_frame = false;
    ext_flags.refresh_frame.alt_ref_frame = false;
    // Decide altref lag adaptively for rt
    if cpi.sf.rt_sf.sad_based_adp_altref_lag != 0 {
        lag_alt = 6;
        let th_frame_sad: [[u64; 3]; 4] = [
            [18000, 18000, 18000], // HDRES CPU 9
            [25000, 25000, 25000], // MIDRES CPU 9
            [40000, 30000, 20000], // HDRES CPU10
            [30000, 25000, 20000], // MIDRES CPU 10
        ];
        let th_idx = (cpi.sf.rt_sf.sad_based_adp_altref_lag - 1) as usize;
        debug_assert!(th_idx < 4);
        if rc.avg_source_sad > th_frame_sad[th_idx][0] {
            lag_alt = 3;
        } else if rc.avg_source_sad > th_frame_sad[th_idx][1] {
            lag_alt = 4;
        } else if rc.avg_source_sad > th_frame_sad[th_idx][2] {
            lag_alt = 5;
        }
    }
    // This defines the reference structure for 1 layer (non-svc) RTC encoding.
    // To avoid the internal/default reference structure for non-realtime
    // overwriting this behavior, we use the "svc" ref parameters from the
    // external control SET_SVC_REF_FRAME_CONFIG.
    // TODO(marpan): rename that control and the related internal parameters
    // to rtc_ref.
    for i in 0..INTER_REFS_PER_FRAME as usize {
        rtc_ref.ref_idx[i] = 7;
    }
    for i in 0..REF_FRAMES as usize {
        rtc_ref.refresh[i] = 0;
    }
    // Set the reference frame flags.
    ext_flags.ref_frame_flags ^= AOM_LAST_FLAG;
    ext_flags.ref_frame_flags ^= AOM_ALT_FLAG;
    ext_flags.ref_frame_flags ^= AOM_GOLD_FLAG;
    if cpi.sf.rt_sf.ref_frame_comp_nonrd[1] != 0 {
        ext_flags.ref_frame_flags ^= AOM_LAST2_FLAG;
    }
    let sh = 6;
    // Moving index slot for last: 0 - (sh - 1).
    if frame_number > 1 {
        last_idx = ((frame_number - 1) % sh) as i32;
    }
    // Moving index for refresh of last: one ahead for next frame.
    last_idx_refresh = (frame_number % sh) as i32;
    gld_idx = 6;

    // Moving index for alt_ref, lag behind LAST by lag_alt frames.
    if frame_number > lag_alt {
        alt_ref_idx = ((frame_number - lag_alt) % sh) as i32;
    }
    if cpi.sf.rt_sf.ref_frame_comp_nonrd[1] != 0 {
        // Moving index for LAST2, lag behind LAST by 2 frames.
        if frame_number > 2 {
            last2_idx = ((frame_number - 2) % sh) as i32;
        }
    }
    rtc_ref.ref_idx[0] = last_idx; // LAST
    rtc_ref.ref_idx[1] = last_idx_refresh; // LAST2 (for refresh of last).
    if cpi.sf.rt_sf.ref_frame_comp_nonrd[1] != 0 {
        rtc_ref.ref_idx[1] = last2_idx; // LAST2
        rtc_ref.ref_idx[2] = last_idx_refresh; // LAST3 (for refresh of last).
    }
    rtc_ref.ref_idx[3] = gld_idx; // GOLDEN
    rtc_ref.ref_idx[6] = alt_ref_idx; // ALT_REF
                                      // Refresh this slot, which will become LAST on next frame.
    rtc_ref.refresh[last_idx_refresh as usize] = 1;
    // Update GOLDEN on period for fixed slot case.
    if gf_update && frame_type != KEY_FRAME {
        ext_flags.refresh_frame.golden_frame = true;
        rtc_ref.refresh[gld_idx as usize] = 1;
    }
    rtc_ref.gld_idx_1layer = gld_idx;
    // Set the flag to reduce the number of reference frame buffers used.
    // This assumes that slot 7 is never used.
    let mut rt_reduce_num_ref_buffers = true;
    rt_reduce_num_ref_buffers &= rtc_ref.ref_idx[0] < 7;
    rt_reduce_num_ref_buffers &= rtc_ref.ref_idx[1] < 7;
    rt_reduce_num_ref_buffers &= rtc_ref.ref_idx[3] < 7;
    rt_reduce_num_ref_buffers &= rtc_ref.ref_idx[6] < 7;
    if cpi.sf.rt_sf.ref_frame_comp_nonrd[1] != 0 {
        rt_reduce_num_ref_buffers &= rtc_ref.ref_idx[2] < 7;
    }
    cpi.rt_reduce_num_ref_buffers = rt_reduce_num_ref_buffers;
}

/*\brief Check for scene detection, for 1 pass real-time mode.
 *
 * Compute average source sad (temporal sad: between current source and
 * previous source) over a subset of superblocks. Use this is detect big changes
 * in content and set the \c cpi->rc.high_source_sad flag.
 *
 * \ingroup rate_control
 * \param[in]       cpi          Top level encoder structure
 * \param[in]       frame_input  Current and last input source frames
 *
 * \remark Nothing is returned. Instead the flag \c cpi->rc.high_source_sad
 * is set if scene change is detected, and \c cpi->rc.avg_source_sad is updated.
 */
fn rc_scene_detection_onepass_rt(cpi: &mut Av1Comp, frame_input: &EncodeFrameInput) {
    let cm = &cpi.common;
    let unscaled_src = frame_input.source(cpi);
    let unscaled_last_src = frame_input.last_source(cpi);
    let width = cm.width;
    let height = cm.height;
    // (one spatial layer)
    let free_sad_blk = width != cm.render_width
        || height != cm.render_height
        || unscaled_src.is_none()
        || unscaled_last_src.is_none();
    let (Some(unscaled_src), Some(unscaled_last_src)) = (unscaled_src, unscaled_last_src) else {
        if free_sad_blk {
            cpi.src_sad_blk_64x64 = Vec::new();
        }
        return;
    };
    let src_ystride = unscaled_src.y_stride() as usize;
    let src_width = unscaled_src.y_width();
    let src_height = unscaled_src.y_height();
    let last_src_ystride = unscaled_last_src.y_stride() as usize;
    let last_src_width = unscaled_last_src.y_width();
    let last_src_height = unscaled_last_src.y_height();
    if src_width != last_src_width || src_height != last_src_height {
        cpi.src_sad_blk_64x64 = Vec::new();
        return;
    }
    let rc = &cpi.rc;
    // (rc->high_source_sad, rc->percent_blocks_with_motion and
    // rc->max_block_source_sad are reset to 0 here; the new values are kept
    // in locals until they are stored at the end)
    let mut max_block_source_sad = 0u64;
    let prev_avg_source_sad = rc.avg_source_sad;
    let num_mi_cols = cm.mi_params.mi_cols;
    let num_mi_rows = cm.mi_params.mi_rows;
    let mut num_zero_temp_sad = 0;
    let mut min_thresh: u32 = 10000;
    if cpi.oxcf.tune_cfg.content != AOM_CONTENT_SCREEN {
        min_thresh = 100000;
    }
    let bsize = BLOCK_64X64;
    // Loop over sub-sample of frame, compute average sad over 64x64 blocks.
    let mut avg_sad: u64 = 0;
    let mut tmp_sad: u64;
    let mut num_samples = 0;
    let thresh: u64 = 6;
    // SAD is computed on 64x64 blocks
    let sb_size_by_mb = if cm.seq_params.sb_size == BLOCK_128X128 {
        cm.seq_params.mib_size >> 1
    } else {
        cm.seq_params.mib_size
    };
    let sb_cols = (num_mi_cols + sb_size_by_mb - 1) / sb_size_by_mb;
    let sb_rows = (num_mi_rows + sb_size_by_mb - 1) / sb_size_by_mb;
    // uint64_t sum_sq_thresh = 10000;  // sum = sqrt(thresh / 64*64)) ~1.5
    // Flag to check light change or not.
    // (check_light_change is 0: no light change detection)
    let light_change = false;
    // Store blkwise SAD for later use
    let mut src_sad_blk_64x64 = Vec::new();
    let keep_blk = width == cm.render_width && height == cm.render_height;
    if keep_blk {
        if cpi.src_sad_blk_64x64.is_empty() {
            src_sad_blk_64x64 = vec![0u64; (sb_cols * sb_rows) as usize];
        } else {
            src_sad_blk_64x64 = cpi.src_sad_blk_64x64.clone();
        }
    }
    let fn_ptr = &cpi.ppi.fn_ptr;
    let src = &unscaled_src.buffer_alloc;
    let last_src = &unscaled_last_src.buffer_alloc;
    let mut src_y = unscaled_src.buffers[0];
    let mut last_src_y = unscaled_last_src.buffers[0];
    for sbi_row in 0..sb_rows {
        for sbi_col in 0..sb_cols {
            tmp_sad = fn_ptr.sdf(
                bsize,
                src,
                src_y,
                src_ystride,
                last_src,
                last_src_y,
                last_src_ystride,
            ) as u64;
            if !src_sad_blk_64x64.is_empty() {
                src_sad_blk_64x64[(sbi_col + sbi_row * sb_cols) as usize] = tmp_sad;
            }
            avg_sad += tmp_sad;
            num_samples += 1;
            if tmp_sad == 0 {
                num_zero_temp_sad += 1;
            }
            if tmp_sad > max_block_source_sad {
                max_block_source_sad = tmp_sad;
            }

            src_y += 64;
            last_src_y += 64;
        }
        src_y = src_y + (src_ystride << 6) - ((sb_cols as usize) << 6);
        last_src_y = last_src_y + (last_src_ystride << 6) - ((sb_cols as usize) << 6);
    }
    if num_samples > 0 {
        avg_sad /= num_samples as u64;
    }
    // Set high_source_sad flag if we detect very high increase in avg_sad
    // between current and previous frame value(s). Use minimum threshold
    // for cases where there is small change from content that is completely
    // static.
    let high_source_sad = if !light_change
        && avg_sad > (min_thresh.max((rc.avg_source_sad * thresh) as u32)) as u64
        && rc.frames_since_key > 1 + cpi.svc.number_spatial_layers
        && num_zero_temp_sad < 3 * (num_samples >> 2)
    {
        1
    } else {
        0
    };
    let avg_source_sad = (3 * rc.avg_source_sad + avg_sad) >> 2;
    cpi.src_sad_blk_64x64 = src_sad_blk_64x64;
    let rc = &mut cpi.rc;
    rc.prev_avg_source_sad = prev_avg_source_sad;
    rc.max_block_source_sad = max_block_source_sad;
    rc.high_source_sad = high_source_sad;
    rc.avg_source_sad = avg_source_sad;
    rc.frame_source_sad = avg_sad;
    rc.percent_blocks_with_motion = 0;
    if num_samples > 0 {
        rc.percent_blocks_with_motion = ((num_samples - num_zero_temp_sad) * 100) / num_samples;
    }
    // Scene detection is only on base SLO, and using full/orignal resolution.
    // Pass the state to the upper spatial layers.
    // (one spatial layer)
}

/*\brief Set the GF baseline interval for 1 pass real-time mode.
 *
 *
 * \ingroup rate_control
 * \param[in]       cpi          Top level encoder structure
 * \param[in]       frame_type   frame type
 *
 * \return Return GF update flag, and update the \c cpi->rc with
 * the next GF interval settings.
 */
fn set_gf_interval_update_onepass_rt(cpi: &mut Av1Comp, frame_type: FrameType) -> bool {
    let rc = &cpi.rc;
    let mut gf_update = false;
    let resize_pending = is_frame_resize_pending(cpi);
    // GF update based on frames_till_gf_update_due, also
    // force upddate on resize pending frame or for scene change.
    if (resize_pending || rc.high_source_sad != 0 || rc.frames_till_gf_update_due == 0)
        && cpi.svc.temporal_layer_id == 0
        && cpi.svc.spatial_layer_id == 0
    {
        set_baseline_gf_interval(cpi, frame_type);
        gf_update = true;
    }
    gf_update
}

#[inline]
fn set_key_frame(cpi: &Av1Comp, frame_flags: u32) -> bool {
    let rc = &cpi.rc;
    let cm = &cpi.common;

    // Very first frame has to be key frame.
    if cm.current_frame.frame_number == 0 {
        return true;
    }
    // Set key frame if forced by frame flags.
    if frame_flags & FRAMEFLAGS_KEY != 0 {
        return true;
    }
    // Non-SVC
    // (one layer)
    debug_assert!(!cpi.ppi.use_svc);
    if cpi.oxcf.kf_cfg.auto_key && rc.frames_to_key == 0 {
        return true;
    }

    false
}

/// Translation of `av1_get_one_pass_rt_params()` (one layer, no dynamic
/// resize).
pub(crate) fn av1_get_one_pass_rt_params(
    cpi: &mut Av1Comp,
    frame_type: &mut FrameType,
    frame_input: &EncodeFrameInput,
    frame_flags: u32,
) {
    let target;
    // Turn this on to explicitly set the reference structure rather than
    // relying on internal/default structure.
    debug_assert!(!cpi.ppi.use_svc);
    // Set frame type.
    if set_key_frame(cpi, frame_flags) {
        *frame_type = KEY_FRAME;
        cpi.ppi.p_rc.this_key_frame_forced =
            (cpi.common.current_frame.frame_number != 0 && cpi.rc.frames_to_key == 0) as i32;
        cpi.rc.frames_to_key = cpi.oxcf.kf_cfg.key_freq_max;
        cpi.ppi.p_rc.kf_boost = DEFAULT_KF_BOOST_RT;
        let gf_frame_index = cpi.gf_frame_index as usize;
        let gf_group = &mut cpi.ppi.gf_group;
        gf_group.update_type[gf_frame_index] = KF_UPDATE;
        gf_group.frame_type[gf_frame_index] = KEY_FRAME;
        gf_group.refbuf_state[gf_frame_index] = REFBUF_RESET;
    } else {
        *frame_type = INTER_FRAME;
        let gf_frame_index = cpi.gf_frame_index as usize;
        let gf_group = &mut cpi.ppi.gf_group;
        gf_group.update_type[gf_frame_index] = LF_UPDATE;
        gf_group.frame_type[gf_frame_index] = INTER_FRAME;
        gf_group.refbuf_state[gf_frame_index] = REFBUF_UPDATE;
    }
    // Check for scene change: for SVC check on base spatial layer only.
    if cpi.sf.rt_sf.check_scene_detection != 0 && cpi.svc.spatial_layer_id == 0 {
        if cpi.rc.prev_coded_width == cpi.common.width
            && cpi.rc.prev_coded_height == cpi.common.height
        {
            rc_scene_detection_onepass_rt(cpi, frame_input);
        } else if !cpi.src_sad_blk_64x64.is_empty() {
            cpi.src_sad_blk_64x64 = Vec::new();
        }
    }
    // Check for dynamic resize, for single spatial layer for now.
    // For temporal layers only check on base temporal layer.
    // (no dynamic resize)
    debug_assert!(cpi.oxcf.resize_cfg.resize_mode == RESIZE_NONE);
    // Set the GF interval and update flag.
    if !cpi.rc.rtc_external_ratectrl {
        set_gf_interval_update_onepass_rt(cpi, *frame_type);
    }
    // Set target size.
    debug_assert!(cpi.oxcf.rc_cfg.mode == AOM_CBR);
    if *frame_type == KEY_FRAME || *frame_type == INTRA_ONLY_FRAME {
        target = av1_calc_iframe_target_size_one_pass_cbr(cpi);
    } else {
        let update_type = cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize];
        target = av1_calc_pframe_target_size_one_pass_cbr(cpi, update_type);
    }
    let (width, height) = (cpi.common.width, cpi.common.height);
    av1_rc_set_frame_target(cpi, target, width, height);
    cpi.rc.base_frame_target = target;
    cpi.common.current_frame.frame_type = *frame_type;
    // For fixed mode SVC: if KSVC is enabled remove inter layer
    // prediction on spatial enhancement layer frames for frames
    // whose base is not KEY frame.
    // (no SVC)
}

/// Translation of `is_frame_resize_pending()` (encoder.h).
#[inline]
pub(crate) fn is_frame_resize_pending(cpi: &Av1Comp) -> bool {
    let resize_pending_params = &cpi.resize_pending_params;
    resize_pending_params.width != 0
        && resize_pending_params.height != 0
        && (cpi.common.width != resize_pending_params.width
            || cpi.common.height != resize_pending_params.height)
}

/// Translation of `is_altref_enabled()` (encoder.h).
#[inline]
pub(crate) fn is_altref_enabled(lag_in_frames: i32, enable_auto_arf: bool) -> bool {
    lag_in_frames >= ALT_MIN_LAG && enable_auto_arf
}

/// `ALT_MIN_LAG` (encoder.h)
pub(crate) const ALT_MIN_LAG: i32 = 3;

/// `ROUND_POWER_OF_TWO()` of an int.
#[inline]
fn round_power_of_two(value: i32, n: i32) -> i32 {
    (value + ((1 << n) >> 1)) >> n
}

/// `ROUND_POWER_OF_TWO_64()`.
#[inline]
fn round_power_of_two_64(value: i64, n: i32) -> i64 {
    (value + ((1i64 << n) >> 1)) >> n
}

/// Translation of `av1_encodedframe_overshoot_cbr()`: the rate control's
/// reset for a scene or slide change coded at a high quantizer.
pub(crate) fn av1_encodedframe_overshoot_cbr(cpi: &mut Av1Comp, q: &mut i32) -> bool {
    let mut rate_correction_factor = cpi.ppi.p_rc.rate_correction_factors[INTER_NORMAL];
    let target_size = cpi.rc.avg_frame_bandwidth;
    let is_screen_content = (cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN) as i32;
    *q = (3 * cpi.rc.worst_quality + *q) >> 2;
    // For screen content use the max-q set by the user to allow for less
    // overshoot on slide changes.
    if is_screen_content != 0 {
        *q = cpi.rc.worst_quality;
    }
    cpi.cyclic_refresh.counter_encode_maxq_scene_change = 0;
    // Adjust avg_frame_qindex, buffer_level, and rate correction factors, as
    // these parameters will affect QP selection for subsequent frames. If they
    // have settled down to a very different (low QP) state, then not adjusting
    // them may cause next frame to select low QP and overshoot again.
    let p_rc = &mut cpi.ppi.p_rc;
    p_rc.avg_frame_qindex[INTER_FRAME as usize] = *q;
    p_rc.buffer_level = p_rc.optimal_buffer_level;
    p_rc.bits_off_target = p_rc.optimal_buffer_level;
    // Reset rate under/over-shoot flags.
    cpi.rc.rc_1_frame = 0;
    cpi.rc.rc_2_frame = 0;
    // Adjust rate correction factor.
    let target_bits_per_mb =
        (((target_size as u64) << BPER_MB_NORMBITS) / cpi.common.mi_params.mbs as u64) as i32;
    // Reset rate correction factor: for now base it on target_bits_per_mb
    // and qp (==max_QP). This comes from the inverse computation of
    // av1_rc_bits_per_mb().
    let q2 = av1_convert_qindex_to_q(*q, cpi.common.seq_params.bit_depth);
    // FIXME (upstream): the rate factor level INTER_NORMAL is passed as the
    // frame type, which makes it KEY_FRAME's enumerator.
    let enumerator = av1_get_bpmb_enumerator(INTER_NORMAL as FrameType, is_screen_content);
    let new_correction_factor = target_bits_per_mb as f64 * q2 / enumerator as f64;
    if new_correction_factor > rate_correction_factor {
        rate_correction_factor = (new_correction_factor + rate_correction_factor) / 2.0;
        if rate_correction_factor > MAX_BPB_FACTOR {
            rate_correction_factor = MAX_BPB_FACTOR;
        }
        cpi.ppi.p_rc.rate_correction_factors[INTER_NORMAL] = rate_correction_factor;
    }
    // For temporal layers: reset the rate control parameters across all
    // temporal layers.
    // (one temporal layer)
    debug_assert!(cpi.svc.number_temporal_layers <= 1);
    true
}
