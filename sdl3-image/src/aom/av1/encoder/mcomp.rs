// Rust translation of the parts of av1/encoder/mcomp.c, mcomp.h and
// mcomp_structs.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches)
// the real-time encoder reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion search.

// (SEARCH_METHODS)
// Search 8-points in the radius grid around center, up to 11 search stages.
pub(crate) const DIAMOND: i32 = 0;
// Search 12-points in the radius/tan_radius grid around center,
// up to 15 search stages.
pub(crate) const NSTEP: i32 = 1;
// Search 8-points in the radius grid around center, up to 16 search stages.
pub(crate) const NSTEP_8PT: i32 = 2;
// Search 8-points in the radius grid around center, upto 11 search stages
// with clamping of search radius.
pub(crate) const CLAMPED_DIAMOND: i32 = 3;
// Search maximum 8-points in the radius grid around center,
// up to 11 search stages. First stage consists of 8 search points
// and the rest with 6 search points each in hex shape.
pub(crate) const HEX: i32 = 4;
// Search maximum 8-points in the radius grid around center,
// up to 11 search stages. First stage consists of 4 search
// points and the rest with 8 search points each.
pub(crate) const BIGDIA: i32 = 5;
// Search 8-points in the square grid around center, up to 11 search stages.
pub(crate) const SQUARE: i32 = 6;
// HEX search with up to 2 stages.
pub(crate) const FAST_HEX: i32 = 7;
// BIGDIA search with up to 2 stages.
pub(crate) const FAST_DIAMOND: i32 = 8;
// BIGDIA search with up to 3 stages.
pub(crate) const FAST_BIGDIA: i32 = 9;
// BIGDIA search with up to 1 stage.
pub(crate) const VFAST_DIAMOND: i32 = 10;
// Total number of search methods.
pub(crate) const NUM_SEARCH_METHODS: i32 = 11;
// Number of distinct search methods.
pub(crate) const NUM_DISTINCT_SEARCH_METHODS: i32 = SQUARE + 1;

// =============================================================================
//  Subpixel Motion Search
// =============================================================================
// (SUBPEL_FORCE_STOP)
pub(crate) const EIGHTH_PEL: i32 = 0;
pub(crate) const QUARTER_PEL: i32 = 1;
pub(crate) const HALF_PEL: i32 = 2;
pub(crate) const FULL_PEL: i32 = 3;
