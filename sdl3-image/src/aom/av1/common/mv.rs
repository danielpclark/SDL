// Rust translation of the parts of av1/common/mv.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion vectors (intra frames only reset and store them).

pub(crate) const INVALID_MV: u32 = 0x80008000;

#[inline]
pub(crate) const fn get_mv_subpel(x: i32) -> i32 {
    x * 8
}

/// Translation of `int_mv`: the union's `as_int` view (`as_mv` is
/// `{ row, col }` in the low and high halves).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct IntMv {
    pub(crate) as_int: u32,
}

// The mv limit for fullpel mvs
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FullMvLimits {
    pub(crate) col_min: i32,
    pub(crate) col_max: i32,
    pub(crate) row_min: i32,
    pub(crate) row_max: i32,
}
