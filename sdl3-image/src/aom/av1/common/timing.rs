// Rust translation of the parts of av1/common/timing.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The levels' maximum bitrates.

use super::enums::{LEVEL_BITS, PROFILE_BITS};

/* Tables for AV1 max bitrates for different levels of main and high tier.
 * The tables are in Kbps instead of Mbps in the specification.
 * Note that depending on the profile, a multiplier is needed.
 */
const UNDEFINED_RATE: i32 = 1 << 21; // Placeholder rate for levels with undefined rate
const INVALID_RATE: i32 = 0; // For invalid profile-level configuration, set rate to 0

/* Max Bitrates for levels of Main Tier in kbps. Bitrate in main_kbps [31] */
/* is a dummy value. The decoder model is not applicable for level 31. */
static MAIN_KBPS: [i32; 1 << LEVEL_BITS] = [
    1500,
    3000,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    6000,
    10000,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    12000,
    20000,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    30000,
    40000,
    60000,
    60000,
    60000,
    100000,
    160000,
    160000,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
];

/* Max Bitrates for levels of High Tier in kbps. Bitrate in high_kbps [31] */
/* is a dummy value. The decoder model is not applicable for level 31. */
static HIGH_KBPS: [i32; 1 << LEVEL_BITS] = [
    INVALID_RATE,
    INVALID_RATE,
    INVALID_RATE,
    INVALID_RATE,
    INVALID_RATE,
    INVALID_RATE,
    INVALID_RATE,
    INVALID_RATE,
    30000,
    50000,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    100000,
    160000,
    240000,
    240000,
    240000,
    480000,
    800000,
    800000,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
    UNDEFINED_RATE,
];

/* BitrateProfileFactor */
static BITRATE_PROFILE_FACTOR: [i32; 1 << PROFILE_BITS] = [1, 2, 3, 0, 0, 0, 0, 0];

pub(crate) fn av1_max_level_bitrate(seq_profile: i32, seq_level_idx: i32, seq_tier: i32) -> i64 {
    let bitrate: i64 = if seq_tier != 0 {
        HIGH_KBPS[seq_level_idx as usize] as i64
            * BITRATE_PROFILE_FACTOR[seq_profile as usize] as i64
    } else {
        MAIN_KBPS[seq_level_idx as usize] as i64
            * BITRATE_PROFILE_FACTOR[seq_profile as usize] as i64
    };

    bitrate * 1000
}
