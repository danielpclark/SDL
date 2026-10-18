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

/// `SCALE_EXTRA_BITS`
pub(crate) const SCALE_EXTRA_BITS: i32 = SCALE_SUBPEL_BITS - super::filter::SUBPEL_BITS;
/// `SCALE_SUBPEL_SHIFTS`
pub(crate) const SCALE_SUBPEL_SHIFTS: i32 = 1 << SCALE_SUBPEL_BITS;
/// `SCALE_SUBPEL_MASK`
pub(crate) const SCALE_SUBPEL_MASK: i32 = SCALE_SUBPEL_SHIFTS - 1;
/// `SCALE_EXTRA_OFF`
pub(crate) const SCALE_EXTRA_OFF: i32 = (1 << SCALE_EXTRA_BITS) / 2;

// Note: Expect val to be in q4 precision
#[inline]
pub(crate) fn av1_scaled_x(val: i32, sf: &ScaleFactors) -> i32 {
    let off = (sf.x_scale_fp - (1 << REF_SCALE_SHIFT)) * (1 << (super::filter::SUBPEL_BITS - 1));
    let tval = val as i64 * sf.x_scale_fp as i64 + off as i64;
    round_power_of_two_signed_64(tval, REF_SCALE_SHIFT - SCALE_EXTRA_BITS) as i32
}

// Note: Expect val to be in q4 precision
#[inline]
pub(crate) fn av1_scaled_y(val: i32, sf: &ScaleFactors) -> i32 {
    let off = (sf.y_scale_fp - (1 << REF_SCALE_SHIFT)) * (1 << (super::filter::SUBPEL_BITS - 1));
    let tval = val as i64 * sf.y_scale_fp as i64 + off as i64;
    round_power_of_two_signed_64(tval, REF_SCALE_SHIFT - SCALE_EXTRA_BITS) as i32
}

// Note: Expect val to be in q4 precision
#[inline]
pub(crate) fn av1_unscaled_value(val: i32, sf: &ScaleFactors) -> i32 {
    let _ = sf;
    val * (1 << SCALE_EXTRA_BITS)
}

/// `ROUND_POWER_OF_TWO_SIGNED_64()`
#[inline]
fn round_power_of_two_signed_64(value: i64, n: i32) -> i64 {
    if value < 0 {
        -((-value + ((1i64 << n) >> 1)) >> n)
    } else {
        (value + ((1i64 << n) >> 1)) >> n
    }
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
