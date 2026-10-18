// Rust translation of the parts of av1/encoder/av1_noise_estimate.c and
// av1_noise_estimate.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the AVIF animation encoder reaches.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The temporal noise estimate of the real-time usage. Its speed feature,
//! `use_temporal_noise_estimate`, is on for frames over 640x480, but the
//! estimate is only enabled with the cyclic refresh adaptive quantization,
//! which libavif does not set (`aq_mode` stays `NO_AQ`): the update only
//! finds it disabled (and the estimate is read nowhere else then).
//! (CONFIG_AV1_TEMPORAL_DENOISING is off.)

use super::encoder::{Av1Comp, AOM_CBR, AOM_CONTENT_SCREEN, AOM_RC_ONE_PASS, CYCLIC_REFRESH_AQ};
use super::ratectrl::is_frame_resize_pending;

/// `NOISE_LEVEL`: `kLowLow`
pub(crate) const K_LOW_LOW: i32 = 0;
/// `NOISE_LEVEL`: `kLow`
pub(crate) const K_LOW: i32 = 1;

/// `NOISE_ESTIMATE`
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NoiseEstimate {
    pub(crate) enabled: i32,
    pub(crate) level: i32,
    pub(crate) value: i32,
    pub(crate) thresh: i32,
    pub(crate) adapt_thresh: i32,
    pub(crate) count: i32,
    pub(crate) last_w: i32,
    pub(crate) last_h: i32,
    pub(crate) num_frames_estimate: i32,
}

/// Translation of `av1_noise_estimate_init()`.
pub(crate) fn av1_noise_estimate_init(ne: &mut NoiseEstimate, width: i32, height: i32) {
    ne.enabled = 0;
    ne.level = if width * height < 1280 * 720 {
        K_LOW_LOW
    } else {
        K_LOW
    };
    ne.value = 0;
    ne.count = 0;
    ne.thresh = 90;
    ne.last_w = 0;
    ne.last_h = 0;
    if width * height >= 1920 * 1080 {
        ne.thresh = 200;
    } else if width * height >= 1280 * 720 {
        ne.thresh = 140;
    } else if width * height >= 640 * 360 {
        ne.thresh = 115;
    }
    ne.num_frames_estimate = 15;
    ne.adapt_thresh = (3 * ne.thresh) >> 1;
}

/// Translation of `enable_noise_estimation()`.
fn enable_noise_estimation(cpi: &Av1Comp) -> i32 {
    let resize_pending = is_frame_resize_pending(cpi);

    // (CONFIG_AV1_HIGHBITDEPTH)
    if cpi.common.seq_params.use_highbitdepth != 0 {
        return 0;
    }
    // Only allow noise estimate under certain encoding mode.
    // Enabled for 1 pass CBR, speed >=5, and if resolution is same as original.
    // Not enabled for SVC mode and screen_content_mode.
    // Not enabled for low resolutions.
    if cpi.oxcf.pass == AOM_RC_ONE_PASS
        && cpi.oxcf.rc_cfg.mode == AOM_CBR
        && cpi.oxcf.q_cfg.aq_mode == CYCLIC_REFRESH_AQ
        && cpi.oxcf.speed >= 5
        && !resize_pending
        && !cpi.ppi.use_svc
        && cpi.oxcf.tune_cfg.content != AOM_CONTENT_SCREEN
        && cpi.common.width * cpi.common.height >= 640 * 360
    {
        1
    } else {
        0
    }
}

/// Translation of `av1_update_noise_estimate()`: `Err` for an enabled
/// estimate (only with the cyclic refresh adaptive quantization).
pub(crate) fn av1_update_noise_estimate(cpi: &mut Av1Comp) -> Result<(), ()> {
    // Estimate of noise level every frame_period frames.
    let frame_period = 8;
    let frame_counter = cpi.common.current_frame.frame_number as i32;
    // Estimate is between current source and last source.
    // (cpi->last_source: the last source, there from the second frame on)
    let has_last_source = cpi.common.current_frame.frame_number > 0;
    cpi.noise_estimate.enabled = enable_noise_estimation(cpi);
    // (one spatial layer)
    let ne = &mut cpi.noise_estimate;
    if ne.enabled == 0
        || frame_counter % frame_period != 0
        || !has_last_source
        || (ne.last_w != cpi.common.width || ne.last_h != cpi.common.height)
    {
        if has_last_source {
            ne.last_w = cpi.common.width;
            ne.last_h = cpi.common.height;
        }
        return Ok(());
    }
    // (the estimate itself: the cyclic refresh adaptive quantization is
    // never set)
    Err(())
}
