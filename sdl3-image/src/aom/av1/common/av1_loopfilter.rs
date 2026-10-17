// Rust translation of the parts of av1/common/av1_loopfilter.c and
// av1/common/av1_loopfilter.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The loop filter parameters and their initialization. The saver picks
//! (and signals) filter levels but never filters: libavif asks libaom to
//! skip the post-processing filters of still pictures.

use super::enums::*;
use super::seg_common::MAX_SEGMENTS;

pub(crate) const MAX_LOOP_FILTER: usize = 63;
pub(crate) const MAX_SHARPNESS: i32 = 7;

pub(crate) const SIMD_WIDTH: usize = 16;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Loopfilter {
    pub(crate) filter_level: [i32; 2],
    pub(crate) filter_level_u: i32,
    pub(crate) filter_level_v: i32,

    pub(crate) sharpness_level: i32,

    pub(crate) mode_ref_delta_enabled: u8,
    pub(crate) mode_ref_delta_update: u8,

    // 0 = Intra, Last, Last2+Last3,
    // GF, BRF, ARF2, ARF
    pub(crate) ref_deltas: [i8; REF_FRAMES as usize],

    // 0 = ZERO_MV, MV
    pub(crate) mode_deltas: [i8; MAX_MODE_LF_DELTAS as usize],
}

// Need to align this structure so when it is declared and
// passed it can be loaded into vector registers.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LoopFilterThresh {
    pub(crate) mblim: [u8; SIMD_WIDTH],
    pub(crate) lim: [u8; SIMD_WIDTH],
    pub(crate) hev_thr: [u8; SIMD_WIDTH],
}

#[derive(Debug, Clone)]
pub(crate) struct LoopFilterInfoN {
    pub(crate) lfthr: [LoopFilterThresh; MAX_LOOP_FILTER + 1],
    pub(crate) lvl: [[[[[u8; MAX_MODE_LF_DELTAS as usize]; REF_FRAMES as usize]; 2]; MAX_SEGMENTS];
        MAX_MB_PLANE],
}

impl Default for LoopFilterInfoN {
    fn default() -> Self {
        LoopFilterInfoN {
            lfthr: [LoopFilterThresh::default(); MAX_LOOP_FILTER + 1],
            lvl: [[[[[0; MAX_MODE_LF_DELTAS as usize]; REF_FRAMES as usize]; 2]; MAX_SEGMENTS];
                MAX_MB_PLANE],
        }
    }
}

pub(crate) const MAX_MB_PLANE: usize = 3;

fn update_sharpness(lfi: &mut LoopFilterInfoN, sharpness_lvl: i32) {
    // For each possible value for the loop filter fill out limits
    for lvl in 0..=MAX_LOOP_FILTER as i32 {
        // Set loop filter parameters that control sharpness.
        let mut block_inside_limit =
            lvl >> ((sharpness_lvl > 0) as i32 + (sharpness_lvl > 4) as i32);

        if sharpness_lvl > 0 {
            if block_inside_limit > (9 - sharpness_lvl) {
                block_inside_limit = 9 - sharpness_lvl;
            }
        }

        if block_inside_limit < 1 {
            block_inside_limit = 1;
        }

        lfi.lfthr[lvl as usize].lim = [block_inside_limit as u8; SIMD_WIDTH];
        lfi.lfthr[lvl as usize].mblim = [(2 * (lvl + 2) + block_inside_limit) as u8; SIMD_WIDTH];
    }
}

pub(crate) fn av1_loop_filter_init(lfi: &mut LoopFilterInfoN, lf: &Loopfilter) {
    // init limits for given sharpness
    update_sharpness(lfi, lf.sharpness_level);

    // init hev threshold const vectors
    for lvl in 0..=MAX_LOOP_FILTER {
        lfi.lfthr[lvl].hev_thr = [(lvl >> 4) as u8; SIMD_WIDTH];
    }
}
