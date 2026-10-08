// Rust translation of src/enc/filter_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Selecting filter level. The autofilter (`lf_stats_`: `DoFilter()`,
//! `GetMBSSIM()`, `VP8InitFilter()` and `VP8StoreFilterStats()`) is not
//! translated: SDL_image doesn't set `autofilter`, and without it they do
//! nothing.

use crate::webp::dec::NUM_MB_SEGMENTS;
use crate::webp::enc::vp8i_enc::VP8Encoder;

// This table gives, for a given sharpness, the filtering strength to be
// used (at least) in order to filter a given edge step delta.
// This is constructed by brute force inspection: for all delta, we iterate
// over all possible filtering strength / thresh until needs_filter() returns
// true.
const MAX_DELTA_SIZE: usize = 64;
const K_LEVELS_FROM_DELTA: [[u8; MAX_DELTA_SIZE]; 8] = [
    [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    ],
    [
        0, 1, 2, 3, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 17, 18, 20, 21, 23, 24, 26, 27, 29, 30, 32,
        33, 35, 36, 38, 39, 41, 42, 44, 45, 47, 48, 50, 51, 53, 54, 56, 57, 59, 60, 62, 63, 63, 63,
        63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    ],
    [
        0, 1, 2, 3, 5, 6, 7, 8, 9, 11, 12, 13, 14, 16, 17, 19, 20, 22, 23, 25, 26, 28, 29, 31, 32,
        34, 35, 37, 38, 40, 41, 43, 44, 46, 47, 49, 50, 52, 53, 55, 56, 58, 59, 61, 62, 63, 63, 63,
        63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    ],
    [
        0, 1, 2, 3, 5, 6, 7, 8, 9, 11, 12, 13, 15, 16, 18, 19, 21, 22, 24, 25, 27, 28, 30, 31, 33,
        34, 36, 37, 39, 40, 42, 43, 45, 46, 48, 49, 51, 52, 54, 55, 57, 58, 60, 61, 63, 63, 63, 63,
        63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    ],
    [
        0, 1, 2, 3, 5, 6, 7, 8, 9, 11, 12, 14, 15, 17, 18, 20, 21, 23, 24, 26, 27, 29, 30, 32, 33,
        35, 36, 38, 39, 41, 42, 44, 45, 47, 48, 50, 51, 53, 54, 56, 57, 59, 60, 62, 63, 63, 63, 63,
        63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    ],
    [
        0, 1, 2, 4, 5, 7, 8, 9, 11, 12, 13, 15, 16, 17, 19, 20, 22, 23, 25, 26, 28, 29, 31, 32, 34,
        35, 37, 38, 40, 41, 43, 44, 46, 47, 49, 50, 52, 53, 55, 56, 58, 59, 61, 62, 63, 63, 63, 63,
        63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    ],
    [
        0, 1, 2, 4, 5, 7, 8, 9, 11, 12, 13, 15, 16, 18, 19, 21, 22, 24, 25, 27, 28, 30, 31, 33, 34,
        36, 37, 39, 40, 42, 43, 45, 46, 48, 49, 51, 52, 54, 55, 57, 58, 60, 61, 63, 63, 63, 63, 63,
        63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    ],
    [
        0, 1, 2, 4, 5, 7, 8, 9, 11, 12, 14, 15, 17, 18, 20, 21, 23, 24, 26, 27, 29, 30, 32, 33, 35,
        36, 38, 39, 41, 42, 44, 45, 47, 48, 50, 51, 53, 54, 56, 57, 59, 60, 62, 63, 63, 63, 63, 63,
        63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    ],
];

/// returns the approximate filtering strength needed to smooth a edge
/// step of 'delta', given a sharpness parameter 'sharpness'.
/// Translation of `VP8FilterStrengthFromDelta()`.
pub(crate) fn vp8_filter_strength_from_delta(sharpness: i32, delta: i32) -> i32 {
    let pos = if delta < MAX_DELTA_SIZE as i32 {
        delta
    } else {
        MAX_DELTA_SIZE as i32 - 1
    };
    debug_assert!((0..=7).contains(&sharpness));
    K_LEVELS_FROM_DELTA[sharpness as usize][pos as usize] as i32
}

//------------------------------------------------------------------------------
// Exposed APIs: Encoder should call the following 3 functions to adjust
// loop filter strength

/// Translation of `VP8AdjustFilterStrength()` (without the autofilter's
/// statistics).
pub(crate) fn vp8_adjust_filter_strength(enc: &mut VP8Encoder<'_>) {
    if enc.config.filter_strength > 0 {
        let mut max_level = 0;
        for s in 0..NUM_MB_SEGMENTS {
            let sharpness = enc.filter_hdr.sharpness;
            let dqm = &mut enc.dqm[s];
            // this '>> 3' accounts for some inverse WHT scaling
            let delta = (dqm.max_edge * dqm.y2.q[1] as i32) >> 3;
            let level = vp8_filter_strength_from_delta(sharpness, delta);
            if level > dqm.fstrength {
                dqm.fstrength = level;
            }
            if max_level < dqm.fstrength {
                max_level = dqm.fstrength;
            }
        }
        enc.filter_hdr.level = max_level;
    }
}
