// Rust translation of the parts of av1/encoder/picklpf.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the deblocking filter levels picked from the quantizer.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loop filter level selection (`LPF_PICK_FROM_Q` at the saver's speed).

use super::encoder::*;
use super::speed_features::*;
use crate::aom::aom_dsp::aom_dsp_common::round_power_of_two;
use crate::aom::av1::common::av1_loopfilter::MAX_LOOP_FILTER;
use crate::aom::av1::common::blockd::KEY_FRAME;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::quant_common::av1_ac_quant_qtx;

/// Translation of `av1_get_max_filter_level()`.
pub(crate) fn av1_get_max_filter_level(cpi: &Av1Comp) -> i32 {
    // (two pass section intra ratings: one pass)
    debug_assert!(!is_stat_consumption_stage_twopass(cpi));
    MAX_LOOP_FILTER as i32
}

/// Translation of `av1_pick_filter_level()` for the `LPF_PICK_FROM_Q`
/// method (the source is not read).
pub(crate) fn av1_pick_filter_level(cpi: &mut Av1Comp, method: i32) {
    let max_filter_level = av1_get_max_filter_level(cpi);
    let cm = &mut cpi.common;
    let seq_params = &cm.seq_params;
    let lf = &mut cm.lf;

    lf.sharpness_level = 0;

    // (the screen content and loop filter control shortcuts: the content is
    // not screen content, and the loop filter control is LOOPFILTER_ALL)
    debug_assert!(cpi.oxcf.algo_cfg.loopfilter_control == LOOPFILTER_ALL);

    debug_assert!(method >= LPF_PICK_FROM_Q);
    let min_filter_level = 0;
    let q = av1_ac_quant_qtx(cm.quant_params.base_qindex, 0, seq_params.bit_depth) as i32;
    // based on tests result for rtc test set
    // 0.04590 boosted or 0.02295 non-booseted in 18-bit fixed point
    // (the inter frame multiplier is only read for inter frames)
    // These values were determined by linear fitting the result of the
    // searched level for 8 bit depth:
    // Keyframes: filt_guess = q * 0.06699 - 1.60817
    // Other frames: filt_guess = q * inter_frame_multiplier + 2.48225
    //
    // And high bit depth separately:
    // filt_guess = q * 0.316206 + 3.87252
    let mut filt_guess = match seq_params.bit_depth {
        AOM_BITS_8 => {
            if cm.current_frame.frame_type == KEY_FRAME {
                round_power_of_two(q * 17563 - 421574, 18)
            } else {
                debug_assert!(false);
                0
            }
        }
        AOM_BITS_10 => round_power_of_two(q * 20723 + 4060632, 20),
        AOM_BITS_12 => round_power_of_two(q * 20723 + 16242526, 22),
        _ => {
            debug_assert!(
                false,
                "bit_depth should be AOM_BITS_8, AOM_BITS_10 or AOM_BITS_12"
            );
            return;
        }
    };
    if seq_params.bit_depth != AOM_BITS_8 && cm.current_frame.frame_type == KEY_FRAME {
        filt_guess -= 4;
    }
    // TODO(chengchen): retrain the model for Y, U, V filter levels
    lf.filter_level[0] = filt_guess.clamp(min_filter_level, max_filter_level);
    lf.filter_level[1] = filt_guess.clamp(min_filter_level, max_filter_level);
    lf.filter_level_u = filt_guess.clamp(min_filter_level, max_filter_level);
    lf.filter_level_v = filt_guess.clamp(min_filter_level, max_filter_level);
    // (LOOPFILTER_SELECTIVELY: inter frames only)
}
