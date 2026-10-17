// Rust translation of the parts of av1/common/mvref_common.c and
// av1/common/mvref_common.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The frame-level reference setup a key frame goes through (no reference
//! frames, order hints off for still pictures).

use super::av1_common_int::*;
use super::enums::*;

#[inline]
pub(crate) fn av1_ref_frame_type(rf: &[MvReferenceFrame; 2]) -> i32 {
    // (compound references are never used)
    debug_assert!(rf[1] <= INTRA_FRAME);
    rf[0]
}

pub(crate) fn av1_setup_frame_buf_refs(cm: &mut Av1Common) {
    let order_hint = cm.current_frame.order_hint;
    let display_order_hint = cm.current_frame.display_order_hint;
    let pyramid_level = cm.current_frame.pyramid_level;
    {
        let cur = cm.cur_frame_mut();
        cur.order_hint = order_hint;
        cur.display_order_hint = display_order_hint;
        cur.pyramid_level = pyramid_level;
    }
    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        if let Some(buf) = get_ref_frame_buf(cm, ref_frame) {
            let (oh, doh) = {
                let b = &cm.buffer_pool.frame_bufs[buf];
                (b.order_hint, b.display_order_hint)
            };
            let cur = cm.cur_frame_mut();
            cur.ref_order_hints[(ref_frame - LAST_FRAME) as usize] = oh;
            cur.ref_display_order_hint[(ref_frame - LAST_FRAME) as usize] = doh;
        }
    }
}

pub(crate) fn av1_setup_frame_sign_bias(cm: &mut Av1Common) {
    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        let buf = get_ref_frame_buf(cm, ref_frame);
        // (order hints are off for the saver's still pictures)
        debug_assert!(!(cm.seq_params.order_hint_info.enable_order_hint != 0 && buf.is_some()));
        cm.ref_frame_sign_bias[ref_frame as usize] = 0;
    }
}

// cm->ref_frame_side is calculated here, and will be used in
// av1_copy_frame_mvs() to affect how mvs are copied.
pub(crate) fn av1_calculate_ref_frame_side(cm: &mut Av1Common) {
    let order_hint_info = &cm.seq_params.order_hint_info;

    cm.ref_frame_side = [0; REF_FRAMES as usize];
    // (order hints are off for the saver's still pictures)
    debug_assert!(order_hint_info.enable_order_hint == 0);
}

pub(crate) fn av1_setup_skip_mode_allowed(cm: &mut Av1Common) {
    let skip_mode_info = &mut cm.current_frame.skip_mode_info;

    skip_mode_info.skip_mode_allowed = 0;
    skip_mode_info.ref_frame_idx_0 = INVALID_IDX;
    skip_mode_info.ref_frame_idx_1 = INVALID_IDX;

    // (order hints are off, and the frames are key frames)
    debug_assert!(cm.seq_params.order_hint_info.enable_order_hint == 0 || frame_is_intra_only(cm));
}
