// Rust translation of include/common/bitdepth.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The bit depth templates: dav1d compiles its `*_tmpl.c` files twice, with
//! `BITDEPTH` 8 (`pixel` is `uint8_t`) and 16 (`uint16_t`, for 10 and 12
//! bits/component); here they are generic over the [`Pixel`] type.
//!
//! The 16 bpc template's `bitdepth_max` argument (`HIGHBD_DECL_SUFFIX`) is
//! passed to both instantiations (it is 255 for 8 bpc, the value
//! `BITDEPTH_MAX` has there). `coef` (`int16_t` for 8 bpc, `int32_t` for
//! 16) is `i32` in both: the dequantizer clips the 8 bpc coefficients to
//! the `int16_t` range, so no value changes.

use std::fmt::Debug;

use super::picture::{PicPlanes, PictureData};

/// A pixel type: `u8` for the 8 bpc template, `u16` for the 16 bpc one.
pub(crate) trait Pixel: Copy + Default + PartialEq + Debug + Send + Sync + 'static {
    /// `BITDEPTH == 8`.
    const BPC8: bool;
    /// The film grain `entry` type (`int8_t` or `int16_t`).
    type Entry: Copy + Default + Debug;
    /// The film grain `SCALING_SIZE`.
    const SCALING_SIZE: usize;

    /// The value as an `int` (C's integer promotion).
    fn to_i32(self) -> i32;
    /// The C conversion of an `int` to `pixel` (truncating).
    fn from_i32(v: i32) -> Self;

    fn entry_to_i32(e: Self::Entry) -> i32;
    /// The C conversion of an `int` to `entry` (truncating).
    fn entry_from_i32(v: i32) -> Self::Entry;

    fn planes(p: &PictureData) -> &[Vec<Self>; 3];
    fn planes_mut(p: &mut PictureData) -> &mut [Vec<Self>; 3];
}

impl Pixel for u8 {
    const BPC8: bool = true;
    type Entry = i8;
    const SCALING_SIZE: usize = 256;

    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u8
    }
    #[inline(always)]
    fn entry_to_i32(e: i8) -> i32 {
        e as i32
    }
    #[inline(always)]
    fn entry_from_i32(v: i32) -> i8 {
        v as i8
    }

    fn planes(p: &PictureData) -> &[Vec<u8>; 3] {
        match &p.planes {
            PicPlanes::U8(p) => p,
            PicPlanes::U16(_) => unreachable!("8 bpc template on a 16 bpc picture"),
        }
    }
    fn planes_mut(p: &mut PictureData) -> &mut [Vec<u8>; 3] {
        match &mut p.planes {
            PicPlanes::U8(p) => p,
            PicPlanes::U16(_) => unreachable!("8 bpc template on a 16 bpc picture"),
        }
    }
}

impl Pixel for u16 {
    const BPC8: bool = false;
    type Entry = i16;
    const SCALING_SIZE: usize = 4096;

    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u16
    }
    #[inline(always)]
    fn entry_to_i32(e: i16) -> i32 {
        e as i32
    }
    #[inline(always)]
    fn entry_from_i32(v: i32) -> i16 {
        v as i16
    }

    fn planes(p: &PictureData) -> &[Vec<u16>; 3] {
        match &p.planes {
            PicPlanes::U16(p) => p,
            PicPlanes::U8(_) => unreachable!("16 bpc template on an 8 bpc picture"),
        }
    }
    fn planes_mut(p: &mut PictureData) -> &mut [Vec<u16>; 3] {
        match &mut p.planes {
            PicPlanes::U16(p) => p,
            PicPlanes::U8(_) => unreachable!("16 bpc template on an 8 bpc picture"),
        }
    }
}

/// `iclip_pixel()`: `iclip(x, 0, bitdepth_max)`.
#[inline(always)]
pub(crate) fn iclip_pixel<P: Pixel>(x: i32, bitdepth_max: i32) -> P {
    P::from_i32(if x < 0 {
        0
    } else if x > bitdepth_max {
        bitdepth_max
    } else {
        x
    })
}

/// `bitdepth_from_max()`.
#[inline(always)]
pub(crate) fn bitdepth_from_max(bitdepth_max: i32) -> i32 {
    32 - (bitdepth_max as u32).leading_zeros() as i32
}

/// The offset `k` elements away from `off` (C pointer arithmetic with a
/// possibly negative step on a non-negative result).
#[inline(always)]
pub(crate) fn ix(off: usize, k: isize) -> usize {
    (off as isize + k) as usize
}
