// Rust translation of celt/modes.h and celt/modes.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Copyright (c) 2008 Gregory Maxwell
// Written by Jean-Marc Valin and Gregory Maxwell
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The CELT mode. Without `CUSTOM_MODES` (as SDL_mixer's CMake builds
//! libopus), `opus_custom_mode_create()` only looks up the static 48 kHz,
//! 960-sample mode (see `static_modes_float.rs`, which also holds this
//! file's `eband5ms` and `band_allocation` tables).

use super::mdct::MdctLookup;
use super::static_modes_float::MODE48000_960_120;

pub(crate) const MAX_PERIOD: i32 = 1024;

/// Translation of `PulseCache`.
pub(crate) struct PulseCache {
    #[allow(dead_code)]
    pub size: i32,
    pub index: &'static [i16],
    pub bits: &'static [u8],
    pub caps: &'static [u8],
}

/* Mode definition (opaque)
@brief Mode definition
*/
/// Translation of `struct OpusCustomMode` (`CELTMode`).
pub(crate) struct OpusCustomMode {
    pub fs: i32,
    pub overlap: i32,
    pub nb_ebands: i32,
    pub eff_ebands: i32,
    pub preemph: [f32; 4],
    /* Definition for each "pseudo-critical band" */
    pub ebands: &'static [i16],

    pub max_lm: i32,
    pub nb_short_mdcts: i32,
    pub short_mdct_size: i32,

    /* Number of lines in the matrix below */
    pub nb_alloc_vectors: i32,
    /* Number of bits in each band for several rates */
    pub alloc_vectors: &'static [u8],
    pub log_n: &'static [i16],

    pub window: &'static [f32],
    pub mdct: MdctLookup,
    pub cache: PulseCache,
}

/// Translation of `opus_custom_mode_create()` (without `CUSTOM_MODES`).
pub(crate) fn opus_custom_mode_create(fs: i32, frame_size: i32) -> Option<&'static OpusCustomMode> {
    /* List of all the available modes */
    let static_mode_list: [&'static OpusCustomMode; 1] = [&MODE48000_960_120];
    for m in static_mode_list {
        for j in 0..4 {
            if fs == m.fs && (frame_size << j) == m.short_mdct_size * m.nb_short_mdcts {
                return Some(m);
            }
        }
    }
    None
}
