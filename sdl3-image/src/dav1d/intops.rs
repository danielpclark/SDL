// Rust translation of include/common/intops.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Small integer helpers.

#![allow(dead_code)]

#[inline]
pub(crate) fn imax(a: i32, b: i32) -> i32 {
    if a > b {
        a
    } else {
        b
    }
}

#[inline]
pub(crate) fn imin(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}

#[inline]
pub(crate) fn umax(a: u32, b: u32) -> u32 {
    if a > b {
        a
    } else {
        b
    }
}

#[inline]
pub(crate) fn umin(a: u32, b: u32) -> u32 {
    if a < b {
        a
    } else {
        b
    }
}

#[inline]
pub(crate) fn iclip(v: i32, min: i32, max: i32) -> i32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

#[inline]
pub(crate) fn iclip_u8(v: i32) -> i32 {
    iclip(v, 0, 255)
}

#[inline]
pub(crate) fn apply_sign(v: i32, s: i32) -> i32 {
    if s < 0 {
        v.wrapping_neg()
    } else {
        v
    }
}

#[inline]
pub(crate) fn apply_sign64(v: i32, s: i64) -> i32 {
    if s < 0 {
        v.wrapping_neg()
    } else {
        v
    }
}

/// `ulog2()`: the index of the highest set bit (`v` must be non-zero).
#[inline]
pub(crate) fn ulog2(v: u32) -> i32 {
    31 - v.leading_zeros() as i32
}

/// `u64log2()`.
#[inline]
pub(crate) fn u64log2(v: u64) -> i32 {
    63 - v.leading_zeros() as i32
}

/// `ctz()` of common/attributes.h.
#[inline]
pub(crate) fn ctz(v: u32) -> i32 {
    v.trailing_zeros() as i32
}

/// `clz()` of common/attributes.h.
#[inline]
pub(crate) fn clz(v: u32) -> i32 {
    v.leading_zeros() as i32
}

#[inline]
pub(crate) fn inv_recenter(r: u32, v: u32) -> u32 {
    if v > (r << 1) {
        v
    } else if (v & 1) == 0 {
        (v >> 1) + r
    } else {
        r.wrapping_sub((v + 1) >> 1)
    }
}
