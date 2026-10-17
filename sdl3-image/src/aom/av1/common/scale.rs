// Rust translation of the parts of av1/common/scale.c and av1/common/scale.h
// from aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Reference frame scale factors (only ever the identity for the saver).

use crate::aom::aom_dsp::aom_dsp_common::round_power_of_two;

pub(crate) const SCALE_SUBPEL_BITS: i32 = 10;

pub(crate) const REF_SCALE_SHIFT: i32 = 14;
pub(crate) const REF_NO_SCALE: i32 = 1 << REF_SCALE_SHIFT;
pub(crate) const REF_INVALID_SCALE: i32 = -1;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ScaleFactors {
    pub(crate) x_scale_fp: i32, // horizontal fixed point scale factor
    pub(crate) y_scale_fp: i32, // vertical fixed point scale factor
    pub(crate) x_step_q4: i32,
    pub(crate) y_step_q4: i32,
}

#[inline]
pub(crate) fn av1_is_valid_scale(sf: &ScaleFactors) -> bool {
    sf.x_scale_fp != REF_INVALID_SCALE && sf.y_scale_fp != REF_INVALID_SCALE
}

#[inline]
pub(crate) fn av1_is_scaled(sf: &ScaleFactors) -> bool {
    av1_is_valid_scale(sf) && (sf.x_scale_fp != REF_NO_SCALE || sf.y_scale_fp != REF_NO_SCALE)
}

#[inline]
pub(crate) fn valid_ref_frame_size(
    ref_width: i32,
    ref_height: i32,
    this_width: i32,
    this_height: i32,
) -> bool {
    2 * this_width >= ref_width
        && 2 * this_height >= ref_height
        && this_width <= 16 * ref_width
        && this_height <= 16 * ref_height
}

// (scale.c)

fn get_fixed_point_scale_factor(other_size: i32, this_size: i32) -> i32 {
    // Calculate scaling factor once for each reference frame
    // and use fixed point scaling factors in decoding and encoding routines.
    // Hardware implementations can calculate scale factor in device driver
    // and use multiplication and shifting on hardware instead of division.
    ((other_size << REF_SCALE_SHIFT) + this_size / 2) / this_size
}

// Given the fixed point scale, calculate coarse point scale.
fn fixed_point_scale_to_coarse_point_scale(scale_fp: i32) -> i32 {
    round_power_of_two(scale_fp, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS)
}

pub(crate) fn av1_setup_scale_factors_for_frame(
    sf: &mut ScaleFactors,
    other_w: i32,
    other_h: i32,
    this_w: i32,
    this_h: i32,
) {
    if !valid_ref_frame_size(other_w, other_h, this_w, this_h) {
        sf.x_scale_fp = REF_INVALID_SCALE;
        sf.y_scale_fp = REF_INVALID_SCALE;
        return;
    }

    sf.x_scale_fp = get_fixed_point_scale_factor(other_w, this_w);
    sf.y_scale_fp = get_fixed_point_scale_factor(other_h, this_h);

    sf.x_step_q4 = fixed_point_scale_to_coarse_point_scale(sf.x_scale_fp);
    sf.y_step_q4 = fixed_point_scale_to_coarse_point_scale(sf.y_scale_fp);
}
