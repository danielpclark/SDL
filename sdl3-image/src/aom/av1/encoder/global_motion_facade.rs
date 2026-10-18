// Rust translation of the parts of av1/encoder/global_motion_facade.c from
// aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The frame's global motion models. The real-time speed features turn the
//! global motion search off (`GM_DISABLE_SEARCH`), so the models of an
//! inter frame's references are reset to the identity and no reference is
//! searched; the search itself is not translated.

use super::encoder::*;
use super::speed_features::{SpeedFeatures, DISALLOW_RECODE, GM_DISABLE_SEARCH};
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::INTER_FRAME;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mv::DEFAULT_WARP_PARAMS;

/// Translation of `do_gm_search_logic()`.
fn do_gm_search_logic(sf: &SpeedFeatures, frame: MvReferenceFrame) -> bool {
    let _ = frame;
    // (GM_FULL_SEARCH, GM_REDUCED_REF_SEARCH_SKIP_L2_L3 and
    // GM_REDUCED_REF_SEARCH_SKIP_L2_L3_ARF2 search some references)
    sf.gm_sf.gm_search_type != GM_DISABLE_SEARCH
}

// Populates valid reference frames in past/future directions in
// 'reference_frames' and their count in 'num_ref_frames'.
/// Translation of `update_valid_ref_frames_for_gm()`: `Err` when a
/// reference is to be searched.
fn update_valid_ref_frames_for_gm(cpi: &mut Av1Comp) -> Result<(), ()> {
    // (is_frame_eligible_for_ref_pruning(), prune_ref_by_selective_ref_frame()
    // and disable_gm_search_based_on_stats() only decide which references
    // are searched)
    let mut frame = ALTREF_FRAME;
    while frame >= LAST_FRAME {
        let buf = get_ref_frame_buf(&cpi.common, frame);
        let ref_disabled = cpi.ref_frame_flags & AV1_REF_FRAME_FLAG_LIST[frame as usize] == 0;
        cpi.common.global_motion[frame as usize] = DEFAULT_WARP_PARAMS;
        // Skip global motion estimation for invalid ref frames
        if buf.is_none() || (ref_disabled && cpi.sf.hl_sf.recode_loop != DISALLOW_RECODE) {
            frame -= 1;
            continue;
        }

        if let Some(b) = buf {
            let ref_buf = &cpi.common.buffer_pool.frame_bufs[b].buf;
            if ref_buf.y_crop_width() == cpi.source().y_crop_width()
                && ref_buf.y_crop_height() == cpi.source().y_crop_height()
                && do_gm_search_logic(&cpi.sf, frame)
            {
                // (the global motion search is off for the saver's speed)
                return Err(());
            }
        }
        frame -= 1;
    }
    Ok(())
}

// Initializes parameters used for computing global motion.
/// Translation of `setup_global_motion_info_params()`: no reference frame
/// is searched, so no corners are computed. (`av1_downconvert_frame()`
/// only makes the 8-bit copy of a high bit depth source the search reads.)
fn setup_global_motion_info_params(cpi: &mut Av1Comp) -> Result<(), ()> {
    // Populate ref_buf for valid ref frames in global motion
    update_valid_ref_frames_for_gm(cpi)?;

    // Sort the past and future ref frames in the ascending order of their
    // distance from the current frame. reference_frames[0] => past direction
    // and reference_frames[1] => future direction.
    // (none: no corners to detect either)
    Ok(())
}

// Global motion estimation for the current frame is computed.This computation
// happens once per frame and the winner motion model parameters are stored in
// cm->cur_frame->global_motion.
/// Translation of `av1_compute_global_motion_facade()` (and of
/// `global_motion_estimation()` and `av1_global_motion_estimation_mt()`,
/// which compute nothing without references to search).
pub(crate) fn av1_compute_global_motion_facade(cpi: &mut Av1Comp) -> Result<(), ()> {
    // (cpi->ppi->valid_gm_model_found[]: only read by the global motion
    // search)

    if cpi.common.current_frame.frame_type == INTER_FRAME
        && cpi.superres_mode == AOM_SUPERRES_NONE
        && cpi.oxcf.tool_cfg.enable_global_motion
        && !cpi.gm_info.search_done
    {
        setup_global_motion_info_params(cpi)?;
        cpi.gm_info.search_done = true;
    }
    let global_motion = cpi.common.global_motion;
    cpi.common.cur_frame_mut().global_motion = global_motion;
    Ok(())
}
