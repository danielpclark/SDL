// Rust translation of the parts of av1/encoder/superres_scale.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the frame size setup with neither resizing nor superres.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Frame size setup. The saver configures no resize mode and no superres
//! mode (and a reduced still picture header rules both out), so the coded
//! size is the configured size and the superres denominator is
//! `SCALE_NUMERATOR`.

use super::encoder::*;
use crate::aom::av1::common::enums::SCALE_NUMERATOR;
use crate::aom::av1::common::tile_common::av1_is_min_tile_width_satisfied;

/// `size_params_type`
#[derive(Debug, Clone, Copy)]
struct SizeParams {
    resize_width: i32,
    resize_height: i32,
    superres_denom: u8,
}

/// Translation of `calculate_next_resize_scale()`.
fn calculate_next_resize_scale(cpi: &Av1Comp) -> u8 {
    if is_stat_generation_stage(cpi) {
        return SCALE_NUMERATOR as u8;
    }
    let new_denom = SCALE_NUMERATOR as u8;

    if cpi.common.seq_params.reduced_still_picture_hdr != 0 {
        return SCALE_NUMERATOR as u8;
    }
    // (the resize modes: none is configured)
    debug_assert!(cpi.oxcf.resize_cfg.resize_mode == RESIZE_NONE);
    new_denom
}

/// Translation of `av1_superres_in_recode_allowed()`.
pub(crate) fn av1_superres_in_recode_allowed(cpi: &Av1Comp) -> bool {
    let oxcf = &cpi.oxcf;
    // Empirically found to not be beneficial for image coding.
    // (superres_auto_search_type and frames_to_key are only read for
    // AOM_SUPERRES_AUTO)
    if oxcf.superres_cfg.superres_mode != AOM_SUPERRES_AUTO {
        return false;
    }
    debug_assert!(false);
    false
}

/// Translation of `calculate_next_superres_scale()`.
fn calculate_next_superres_scale(cpi: &Av1Comp) -> u8 {
    let oxcf = &cpi.oxcf;
    let superres_cfg = &oxcf.superres_cfg;

    if is_stat_generation_stage(cpi) {
        return SCALE_NUMERATOR as u8;
    }
    let mut new_denom = SCALE_NUMERATOR as u8;

    // Make sure that superres mode of the frame is consistent with the
    // sequence-level flag.
    debug_assert!(
        superres_cfg.superres_mode == AOM_SUPERRES_NONE
            || cpi.common.seq_params.enable_superres != 0
    );
    debug_assert!(
        cpi.common.seq_params.enable_superres != 0
            || superres_cfg.superres_mode == AOM_SUPERRES_NONE
    );
    // Make sure that superres mode for current encoding is consistent with user
    // provided superres mode.
    debug_assert!(
        superres_cfg.superres_mode == AOM_SUPERRES_AUTO
            || cpi.superres_mode == superres_cfg.superres_mode
    );

    // Note: we must look at the current superres_mode to be tried in 'cpi' here,
    // not the user given mode in 'oxcf'.
    match cpi.superres_mode {
        AOM_SUPERRES_NONE => new_denom = SCALE_NUMERATOR as u8,
        _ => {
            // (the superres modes: none is configured)
            debug_assert!(false);
        }
    }
    new_denom
}

/// Translation of `dimension_is_ok()`.
#[inline]
fn dimension_is_ok(orig_dim: i32, resized_dim: i32, denom: i32) -> bool {
    resized_dim * SCALE_NUMERATOR >= orig_dim * denom / 2
}

/// Translation of `dimensions_are_ok()`.
#[inline]
fn dimensions_are_ok(owidth: i32, oheight: i32, rsz: &SizeParams) -> bool {
    // Only need to check the width, as scaling is horizontal only.
    let _ = oheight;
    dimension_is_ok(owidth, rsz.resize_width, rsz.superres_denom as i32)
}

/// Translation of `validate_size_scales()` (the sizes are always fine
/// unscaled).
fn validate_size_scales(owidth: i32, oheight: i32, rsz: &SizeParams) -> bool {
    if dimensions_are_ok(owidth, oheight, rsz) {
        // Nothing to do.
        return true;
    }
    debug_assert!(false);
    false
}

/// Translation of `calculate_next_size_params()`.
fn calculate_next_size_params(cpi: &Av1Comp) -> SizeParams {
    let frm_dim_cfg = &cpi.oxcf.frm_dim_cfg;
    let mut rsz = SizeParams {
        resize_width: frm_dim_cfg.width,
        resize_height: frm_dim_cfg.height,
        superres_denom: SCALE_NUMERATOR as u8,
    };
    debug_assert!(!cpi.ppi.use_svc);
    if is_stat_generation_stage(cpi) {
        return rsz;
    }
    // (no resize is pending)
    let resize_denom = calculate_next_resize_scale(cpi);
    rsz.resize_width = frm_dim_cfg.width;
    rsz.resize_height = frm_dim_cfg.height;
    // (av1_calculate_scaled_size(): the denominator is SCALE_NUMERATOR, so
    // the size stays)
    debug_assert!(resize_denom as i32 == SCALE_NUMERATOR);
    rsz.superres_denom = calculate_next_superres_scale(cpi);
    if !validate_size_scales(frm_dim_cfg.width, frm_dim_cfg.height, &rsz) {
        debug_assert!(false, "Invalid scale parameters");
    }
    rsz
}

/// Translation of `setup_frame_size_from_params()`.
fn setup_frame_size_from_params(cpi: &mut Av1Comp, rsz: &SizeParams) -> Result<(), ()> {
    let encode_width = rsz.resize_width;
    let encode_height = rsz.resize_height;

    let cm = &mut cpi.common;
    cm.superres_upscaled_width = encode_width;
    cm.superres_upscaled_height = encode_height;
    cm.superres_scale_denominator = rsz.superres_denom;
    // (av1_calculate_scaled_superres_size(): SCALE_NUMERATOR keeps the
    // size)
    debug_assert!(rsz.superres_denom as i32 == SCALE_NUMERATOR);
    av1_set_frame_size(cpi, encode_width, encode_height)
}

/// Translation of `av1_setup_frame_size()`.
pub(crate) fn av1_setup_frame_size(cpi: &mut Av1Comp) -> Result<(), ()> {
    // Reset superres params from previous frame.
    cpi.common.superres_scale_denominator = SCALE_NUMERATOR as u8;
    let rsz = calculate_next_size_params(cpi);
    setup_frame_size_from_params(cpi, &rsz)?;

    debug_assert!(av1_is_min_tile_width_satisfied(&cpi.common) != 0);
    Ok(())
}
