// Rust translation of src/video/SDL_pixels.c and include/SDL3/SDL_pixels.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Pixel management.
//!
//! SDL offers facilities for pixel management: pixel format enumeration,
//! colorspace description, palettes, and converting between RGB(A) values and
//! packed pixel values.
//!
//! The surface/blit mapping half of `SDL_pixels.c` (`SDL_MapSurface`,
//! `Map1to1`, `Map1toN`) belongs with surfaces and is translated with them.

// The colour matrices below are upstream's literal constants; keeping every
// digit (even beyond f32 precision) keeps them diffable against SDL_pixels.c.
#![allow(clippy::excessive_precision)]

use std::collections::HashMap;

use crate::err;
use crate::error::{Error, Result};
use crate::stdlib::fourcc;

/// A fully opaque 8-bit alpha value. Translation of `SDL_ALPHA_OPAQUE`.
pub const ALPHA_OPAQUE: u8 = 255;
/// A fully opaque floating point alpha value. Translation of `SDL_ALPHA_OPAQUE_FLOAT`.
pub const ALPHA_OPAQUE_FLOAT: f32 = 1.0;
/// A fully transparent 8-bit alpha value. Translation of `SDL_ALPHA_TRANSPARENT`.
pub const ALPHA_TRANSPARENT: u8 = 0;
/// A fully transparent floating point alpha value. Translation of `SDL_ALPHA_TRANSPARENT_FLOAT`.
pub const ALPHA_TRANSPARENT_FLOAT: f32 = 0.0;

/// Pixel type. Translation of `SDL_PixelType`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PixelType {
    #[default]
    Unknown = 0,
    Index1,
    Index4,
    Index8,
    Packed8,
    Packed16,
    Packed32,
    ArrayU8,
    ArrayU16,
    ArrayU32,
    ArrayF16,
    ArrayF32,
    /* appended at the end for compatibility with sdl2-compat:  */
    Index2,
}

impl PixelType {
    pub const fn from_u32(v: u32) -> PixelType {
        match v {
            1 => PixelType::Index1,
            2 => PixelType::Index4,
            3 => PixelType::Index8,
            4 => PixelType::Packed8,
            5 => PixelType::Packed16,
            6 => PixelType::Packed32,
            7 => PixelType::ArrayU8,
            8 => PixelType::ArrayU16,
            9 => PixelType::ArrayU32,
            10 => PixelType::ArrayF16,
            11 => PixelType::ArrayF32,
            12 => PixelType::Index2,
            _ => PixelType::Unknown,
        }
    }
}

/// Bitmap pixel order, high bit -> low bit. Translation of `SDL_BitmapOrder`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum BitmapOrder {
    #[default]
    None = 0,
    Order4321,
    Order1234,
}

/// Packed component order, high bit -> low bit. Translation of `SDL_PackedOrder`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PackedOrder {
    #[default]
    None = 0,
    Xrgb,
    Rgbx,
    Argb,
    Rgba,
    Xbgr,
    Bgrx,
    Abgr,
    Bgra,
}

/// Array component order, low byte -> high byte. Translation of `SDL_ArrayOrder`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ArrayOrder {
    #[default]
    None = 0,
    Rgb,
    Rgba,
    Argb,
    Bgr,
    Bgra,
    Abgr,
}

/// Packed component layout. Translation of `SDL_PackedLayout`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum PackedLayout {
    #[default]
    None = 0,
    L332,
    L4444,
    L1555,
    L5551,
    L565,
    L8888,
    L2101010,
    L1010102,
}

impl PackedLayout {
    pub const fn from_u32(v: u32) -> PackedLayout {
        match v {
            1 => PackedLayout::L332,
            2 => PackedLayout::L4444,
            3 => PackedLayout::L1555,
            4 => PackedLayout::L5551,
            5 => PackedLayout::L565,
            6 => PackedLayout::L8888,
            7 => PackedLayout::L2101010,
            8 => PackedLayout::L1010102,
            _ => PackedLayout::None,
        }
    }
}

/// Translation of `SDL_DEFINE_PIXELFOURCC()`.
pub const fn define_pixelfourcc(a: u8, b: u8, c: u8, d: u8) -> PixelFormat {
    PixelFormat(fourcc(a, b, c, d))
}

/// Translation of `SDL_DEFINE_PIXELFORMAT()`.
pub const fn define_pixelformat(
    ty: PixelType,
    order: u32,
    layout: PackedLayout,
    bits: u32,
    bytes: u32,
) -> PixelFormat {
    PixelFormat(
        (1 << 28)
            | ((ty as u32) << 24)
            | (order << 20)
            | ((layout as u32) << 16)
            | (bits << 8)
            | bytes,
    )
}

/// Pixel format.
///
/// SDL's pixel formats have the following naming convention:
///
/// - Names with a list of components and a single bit count, such as RGB24 and
///   ABGR32, define a platform-independent encoding into bytes in the order
///   specified. For example, in RGB24 data, each pixel is encoded in 3 bytes
///   (red, green, blue) in that order, and in ABGR32 data, each pixel is
///   encoded in 4 bytes (alpha, blue, green, red) in that order. Use these
///   names if the property of a format that is important to you is the order
///   of the bytes in memory or on disk.
/// - Names with a bit count per component, such as ARGB8888 and XRGB1555, are
///   "packed" into an appropriately-sized integer in the platform's native
///   endianness. For example, ARGB8888 is a sequence of 32-bit integers; in
///   each integer, the most significant bits are alpha, and the least
///   significant bits are blue. On a little-endian CPU such as x86, the least
///   significant bits of each integer are arranged first in memory, but on a
///   big-endian CPU such as s390x, the most significant bits are arranged
///   first. Use these names if the property of a format that is important to
///   you is the meaning of each bit position within a native-endianness
///   integer.
/// - In indexed formats such as INDEX4LSB, each pixel is represented by
///   encoding an index into the palette into the indicated number of bits,
///   with multiple pixels packed into each byte if appropriate. In LSB
///   formats, the first (leftmost) pixel is stored in the least-significant
///   bits of the byte; in MSB formats, it's stored in the most-significant
///   bits. INDEX8 does not need LSB/MSB variants, because each pixel exactly
///   fills one byte.
///
/// Translation of `SDL_PixelFormat`. A newtype rather than an enum so that
/// unknown/foreign values round-trip, as they do in C.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PixelFormat(pub u32);

#[allow(non_upper_case_globals)]
impl PixelFormat {
    pub const UNKNOWN: PixelFormat = PixelFormat(0);
    pub const INDEX1LSB: PixelFormat = define_pixelformat(
        PixelType::Index1,
        BitmapOrder::Order4321 as u32,
        PackedLayout::None,
        1,
        0,
    );
    pub const INDEX1MSB: PixelFormat = define_pixelformat(
        PixelType::Index1,
        BitmapOrder::Order1234 as u32,
        PackedLayout::None,
        1,
        0,
    );
    pub const INDEX2LSB: PixelFormat = define_pixelformat(
        PixelType::Index2,
        BitmapOrder::Order4321 as u32,
        PackedLayout::None,
        2,
        0,
    );
    pub const INDEX2MSB: PixelFormat = define_pixelformat(
        PixelType::Index2,
        BitmapOrder::Order1234 as u32,
        PackedLayout::None,
        2,
        0,
    );
    pub const INDEX4LSB: PixelFormat = define_pixelformat(
        PixelType::Index4,
        BitmapOrder::Order4321 as u32,
        PackedLayout::None,
        4,
        0,
    );
    pub const INDEX4MSB: PixelFormat = define_pixelformat(
        PixelType::Index4,
        BitmapOrder::Order1234 as u32,
        PackedLayout::None,
        4,
        0,
    );
    pub const INDEX8: PixelFormat =
        define_pixelformat(PixelType::Index8, 0, PackedLayout::None, 8, 1);
    pub const RGB332: PixelFormat = define_pixelformat(
        PixelType::Packed8,
        PackedOrder::Xrgb as u32,
        PackedLayout::L332,
        8,
        1,
    );
    pub const XRGB4444: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Xrgb as u32,
        PackedLayout::L4444,
        12,
        2,
    );
    pub const XBGR4444: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Xbgr as u32,
        PackedLayout::L4444,
        12,
        2,
    );
    pub const XRGB1555: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Xrgb as u32,
        PackedLayout::L1555,
        15,
        2,
    );
    pub const XBGR1555: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Xbgr as u32,
        PackedLayout::L1555,
        15,
        2,
    );
    pub const ARGB4444: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Argb as u32,
        PackedLayout::L4444,
        16,
        2,
    );
    pub const RGBA4444: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Rgba as u32,
        PackedLayout::L4444,
        16,
        2,
    );
    pub const ABGR4444: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Abgr as u32,
        PackedLayout::L4444,
        16,
        2,
    );
    pub const BGRA4444: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Bgra as u32,
        PackedLayout::L4444,
        16,
        2,
    );
    pub const ARGB1555: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Argb as u32,
        PackedLayout::L1555,
        16,
        2,
    );
    pub const RGBA5551: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Rgba as u32,
        PackedLayout::L5551,
        16,
        2,
    );
    pub const ABGR1555: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Abgr as u32,
        PackedLayout::L1555,
        16,
        2,
    );
    pub const BGRA5551: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Bgra as u32,
        PackedLayout::L5551,
        16,
        2,
    );
    pub const RGB565: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Xrgb as u32,
        PackedLayout::L565,
        16,
        2,
    );
    pub const BGR565: PixelFormat = define_pixelformat(
        PixelType::Packed16,
        PackedOrder::Xbgr as u32,
        PackedLayout::L565,
        16,
        2,
    );
    pub const RGB24: PixelFormat = define_pixelformat(
        PixelType::ArrayU8,
        ArrayOrder::Rgb as u32,
        PackedLayout::None,
        24,
        3,
    );
    pub const BGR24: PixelFormat = define_pixelformat(
        PixelType::ArrayU8,
        ArrayOrder::Bgr as u32,
        PackedLayout::None,
        24,
        3,
    );
    pub const XRGB8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Xrgb as u32,
        PackedLayout::L8888,
        24,
        4,
    );
    pub const RGBX8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Rgbx as u32,
        PackedLayout::L8888,
        24,
        4,
    );
    pub const XBGR8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Xbgr as u32,
        PackedLayout::L8888,
        24,
        4,
    );
    pub const BGRX8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Bgrx as u32,
        PackedLayout::L8888,
        24,
        4,
    );
    pub const ARGB8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Argb as u32,
        PackedLayout::L8888,
        32,
        4,
    );
    pub const RGBA8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Rgba as u32,
        PackedLayout::L8888,
        32,
        4,
    );
    pub const ABGR8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Abgr as u32,
        PackedLayout::L8888,
        32,
        4,
    );
    pub const BGRA8888: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Bgra as u32,
        PackedLayout::L8888,
        32,
        4,
    );
    pub const XRGB2101010: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Xrgb as u32,
        PackedLayout::L2101010,
        32,
        4,
    );
    pub const XBGR2101010: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Xbgr as u32,
        PackedLayout::L2101010,
        32,
        4,
    );
    pub const ARGB2101010: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Argb as u32,
        PackedLayout::L2101010,
        32,
        4,
    );
    pub const ABGR2101010: PixelFormat = define_pixelformat(
        PixelType::Packed32,
        PackedOrder::Abgr as u32,
        PackedLayout::L2101010,
        32,
        4,
    );
    pub const RGB48: PixelFormat = define_pixelformat(
        PixelType::ArrayU16,
        ArrayOrder::Rgb as u32,
        PackedLayout::None,
        48,
        6,
    );
    pub const BGR48: PixelFormat = define_pixelformat(
        PixelType::ArrayU16,
        ArrayOrder::Bgr as u32,
        PackedLayout::None,
        48,
        6,
    );
    pub const RGBA64: PixelFormat = define_pixelformat(
        PixelType::ArrayU16,
        ArrayOrder::Rgba as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const ARGB64: PixelFormat = define_pixelformat(
        PixelType::ArrayU16,
        ArrayOrder::Argb as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const BGRA64: PixelFormat = define_pixelformat(
        PixelType::ArrayU16,
        ArrayOrder::Bgra as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const ABGR64: PixelFormat = define_pixelformat(
        PixelType::ArrayU16,
        ArrayOrder::Abgr as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const RGB48_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF16,
        ArrayOrder::Rgb as u32,
        PackedLayout::None,
        48,
        6,
    );
    pub const BGR48_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF16,
        ArrayOrder::Bgr as u32,
        PackedLayout::None,
        48,
        6,
    );
    pub const RGBA64_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF16,
        ArrayOrder::Rgba as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const ARGB64_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF16,
        ArrayOrder::Argb as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const BGRA64_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF16,
        ArrayOrder::Bgra as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const ABGR64_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF16,
        ArrayOrder::Abgr as u32,
        PackedLayout::None,
        64,
        8,
    );
    pub const RGB96_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF32,
        ArrayOrder::Rgb as u32,
        PackedLayout::None,
        96,
        12,
    );
    pub const BGR96_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF32,
        ArrayOrder::Bgr as u32,
        PackedLayout::None,
        96,
        12,
    );
    pub const RGBA128_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF32,
        ArrayOrder::Rgba as u32,
        PackedLayout::None,
        128,
        16,
    );
    pub const ARGB128_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF32,
        ArrayOrder::Argb as u32,
        PackedLayout::None,
        128,
        16,
    );
    pub const BGRA128_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF32,
        ArrayOrder::Bgra as u32,
        PackedLayout::None,
        128,
        16,
    );
    pub const ABGR128_FLOAT: PixelFormat = define_pixelformat(
        PixelType::ArrayF32,
        ArrayOrder::Abgr as u32,
        PackedLayout::None,
        128,
        16,
    );

    /// YUV 4:2:0 8-bit planar mode: Y + V + U  (3 planes)
    pub const YV12: PixelFormat = define_pixelfourcc(b'Y', b'V', b'1', b'2');
    /// YUV 4:2:0 8-bit planar mode: Y + U + V  (3 planes)
    pub const IYUV: PixelFormat = define_pixelfourcc(b'I', b'Y', b'U', b'V');
    /// YUV 4:2:0 8-bit packed mode: Y0+U0+Y1+V0 (1 plane)
    pub const YUY2: PixelFormat = define_pixelfourcc(b'Y', b'U', b'Y', b'2');
    /// YUV 4:2:0 8-bit packed mode: U0+Y0+V0+Y1 (1 plane)
    pub const UYVY: PixelFormat = define_pixelfourcc(b'U', b'Y', b'V', b'Y');
    /// YUV 4:2:0 8-bit packed mode: Y0+V0+Y1+U0 (1 plane)
    pub const YVYU: PixelFormat = define_pixelfourcc(b'Y', b'V', b'Y', b'U');
    /// YUV 4:2:0 8-bit planar mode: Y + U/V interleaved  (2 planes)
    pub const NV12: PixelFormat = define_pixelfourcc(b'N', b'V', b'1', b'2');
    /// YUV 4:2:0 8-bit planar mode: Y + V/U interleaved  (2 planes)
    pub const NV21: PixelFormat = define_pixelfourcc(b'N', b'V', b'2', b'1');
    /// YUV 4:4:4 8-bit planar mode: Y + U + V  (3 planes)
    pub const I444: PixelFormat = define_pixelfourcc(b'I', b'4', b'4', b'4');
    /// YUV 4:2:0 16-bit planar mode: Y + U/V interleaved  (2 planes)
    pub const P010: PixelFormat = define_pixelfourcc(b'P', b'0', b'1', b'0');
    /// YUV 4:2:0 16-bit planar mode: Y + U + V  (3 planes)
    pub const I0FL: PixelFormat = define_pixelfourcc(b'I', b'0', b'F', b'L');
    /// YUV 4:4:4 16-bit planar mode: Y + U + V  (3 planes)
    pub const I4FL: PixelFormat = define_pixelfourcc(b'I', b'4', b'F', b'L');
    /// Android video texture format
    pub const EXTERNAL_OES: PixelFormat = define_pixelfourcc(b'O', b'E', b'S', b' ');
    /// Motion JPEG
    pub const MJPG: PixelFormat = define_pixelfourcc(b'M', b'J', b'P', b'G');

    /* Aliases for RGBA byte arrays of color data, for the current platform */
    #[cfg(target_endian = "big")]
    pub const RGBA32: PixelFormat = PixelFormat::RGBA8888;
    #[cfg(target_endian = "big")]
    pub const ARGB32: PixelFormat = PixelFormat::ARGB8888;
    #[cfg(target_endian = "big")]
    pub const BGRA32: PixelFormat = PixelFormat::BGRA8888;
    #[cfg(target_endian = "big")]
    pub const ABGR32: PixelFormat = PixelFormat::ABGR8888;
    #[cfg(target_endian = "big")]
    pub const RGBX32: PixelFormat = PixelFormat::RGBX8888;
    #[cfg(target_endian = "big")]
    pub const XRGB32: PixelFormat = PixelFormat::XRGB8888;
    #[cfg(target_endian = "big")]
    pub const BGRX32: PixelFormat = PixelFormat::BGRX8888;
    #[cfg(target_endian = "big")]
    pub const XBGR32: PixelFormat = PixelFormat::XBGR8888;
    #[cfg(target_endian = "little")]
    pub const RGBA32: PixelFormat = PixelFormat::ABGR8888;
    #[cfg(target_endian = "little")]
    pub const ARGB32: PixelFormat = PixelFormat::BGRA8888;
    #[cfg(target_endian = "little")]
    pub const BGRA32: PixelFormat = PixelFormat::ARGB8888;
    #[cfg(target_endian = "little")]
    pub const ABGR32: PixelFormat = PixelFormat::RGBA8888;
    #[cfg(target_endian = "little")]
    pub const RGBX32: PixelFormat = PixelFormat::XBGR8888;
    #[cfg(target_endian = "little")]
    pub const XRGB32: PixelFormat = PixelFormat::BGRX8888;
    #[cfg(target_endian = "little")]
    pub const BGRX32: PixelFormat = PixelFormat::XRGB8888;
    #[cfg(target_endian = "little")]
    pub const XBGR32: PixelFormat = PixelFormat::RGBX8888;

    /// Translation of `SDL_PIXELFLAG()`.
    #[inline]
    pub const fn pixel_flag(self) -> u32 {
        (self.0 >> 28) & 0x0F
    }
    /// Translation of `SDL_PIXELTYPE()`.
    #[inline]
    pub const fn pixel_type(self) -> PixelType {
        PixelType::from_u32((self.0 >> 24) & 0x0F)
    }
    /// Translation of `SDL_PIXELORDER()` (raw value; interpret with
    /// [`BitmapOrder`], [`PackedOrder`] or [`ArrayOrder`] depending on type).
    #[inline]
    pub const fn pixel_order(self) -> u32 {
        (self.0 >> 20) & 0x0F
    }
    /// Translation of `SDL_PIXELLAYOUT()`.
    #[inline]
    pub const fn pixel_layout(self) -> PackedLayout {
        PackedLayout::from_u32((self.0 >> 16) & 0x0F)
    }
    /// Translation of `SDL_BITSPERPIXEL()`.
    #[inline]
    pub const fn bits_per_pixel(self) -> u32 {
        if self.is_fourcc() {
            0
        } else {
            (self.0 >> 8) & 0xFF
        }
    }
    /// Translation of `SDL_BYTESPERPIXEL()`.
    #[inline]
    pub const fn bytes_per_pixel(self) -> u32 {
        if self.is_fourcc() {
            if self.0 == PixelFormat::YUY2.0
                || self.0 == PixelFormat::UYVY.0
                || self.0 == PixelFormat::YVYU.0
                || self.0 == PixelFormat::P010.0
                || self.0 == PixelFormat::I0FL.0
                || self.0 == PixelFormat::I4FL.0
            {
                2
            } else {
                1
            }
        } else {
            self.0 & 0xFF
        }
    }
    /// Translation of `SDL_ISPIXELFORMAT_INDEXED()`.
    #[inline]
    pub const fn is_indexed(self) -> bool {
        !self.is_fourcc()
            && matches!(
                self.pixel_type(),
                PixelType::Index1 | PixelType::Index2 | PixelType::Index4 | PixelType::Index8
            )
    }
    /// Translation of `SDL_ISPIXELFORMAT_PACKED()`.
    #[inline]
    pub const fn is_packed(self) -> bool {
        !self.is_fourcc()
            && matches!(
                self.pixel_type(),
                PixelType::Packed8 | PixelType::Packed16 | PixelType::Packed32
            )
    }
    /// Translation of `SDL_ISPIXELFORMAT_ARRAY()`.
    #[inline]
    pub const fn is_array(self) -> bool {
        !self.is_fourcc()
            && matches!(
                self.pixel_type(),
                PixelType::ArrayU8
                    | PixelType::ArrayU16
                    | PixelType::ArrayU32
                    | PixelType::ArrayF16
                    | PixelType::ArrayF32
            )
    }
    /// Translation of `SDL_ISPIXELFORMAT_10BIT()`.
    #[inline]
    pub const fn is_10bit(self) -> bool {
        !self.is_fourcc()
            && matches!(self.pixel_type(), PixelType::Packed32)
            && matches!(self.pixel_layout(), PackedLayout::L2101010)
    }
    /// Translation of `SDL_ISPIXELFORMAT_FLOAT()`.
    #[inline]
    pub const fn is_float(self) -> bool {
        !self.is_fourcc() && matches!(self.pixel_type(), PixelType::ArrayF16 | PixelType::ArrayF32)
    }
    /// Translation of `SDL_ISPIXELFORMAT_ALPHA()`.
    #[inline]
    pub const fn has_alpha(self) -> bool {
        let order = self.pixel_order();
        (self.is_packed()
            && (order == PackedOrder::Argb as u32
                || order == PackedOrder::Rgba as u32
                || order == PackedOrder::Abgr as u32
                || order == PackedOrder::Bgra as u32))
            || (self.is_array()
                && (order == ArrayOrder::Argb as u32
                    || order == ArrayOrder::Rgba as u32
                    || order == ArrayOrder::Abgr as u32
                    || order == ArrayOrder::Bgra as u32))
    }
    /// Translation of `SDL_ISPIXELFORMAT_FOURCC()`.
    ///
    /// The flag is set to 1 because 0x1? is not in the printable ASCII range.
    #[inline]
    pub const fn is_fourcc(self) -> bool {
        self.0 != 0 && self.pixel_flag() != 1
    }
}

/// Colorspace color type. Translation of `SDL_ColorType`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorType {
    #[default]
    Unknown = 0,
    Rgb = 1,
    Ycbcr = 2,
}

/// Colorspace color range, as described by
/// <https://www.itu.int/rec/R-REC-BT.2100-2-201807-I/en>.
///
/// Translation of `SDL_ColorRange`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorRange {
    #[default]
    Unknown = 0,
    /// Narrow range, e.g. 16-235 for 8-bit RGB and luma, and 16-240 for 8-bit chroma
    Limited = 1,
    /// Full range, e.g. 0-255 for 8-bit RGB and luma, and 1-255 for 8-bit chroma
    Full = 2,
}

/// Colorspace color primaries, as described by
/// <https://www.itu.int/rec/T-REC-H.273-201612-I/en>.
///
/// Translation of `SDL_ColorPrimaries`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorPrimaries {
    #[default]
    Unknown = 0,
    /// ITU-R BT.709-6
    Bt709 = 1,
    Unspecified = 2,
    /// ITU-R BT.470-6 System M
    Bt470m = 4,
    /// ITU-R BT.470-6 System B, G / ITU-R BT.601-7 625
    Bt470bg = 5,
    /// ITU-R BT.601-7 525, SMPTE 170M
    Bt601 = 6,
    /// SMPTE 240M, functionally the same as `Bt601`
    Smpte240 = 7,
    /// Generic film (color filters using Illuminant C)
    GenericFilm = 8,
    /// ITU-R BT.2020-2 / ITU-R BT.2100-0
    Bt2020 = 9,
    /// SMPTE ST 428-1
    Xyz = 10,
    /// SMPTE RP 431-2
    Smpte431 = 11,
    /// SMPTE EG 432-1 / DCI P3
    Smpte432 = 12,
    /// EBU Tech. 3213-E
    Ebu3213 = 22,
    Custom = 31,
}

impl ColorPrimaries {
    pub const fn from_u32(v: u32) -> ColorPrimaries {
        match v {
            1 => ColorPrimaries::Bt709,
            2 => ColorPrimaries::Unspecified,
            4 => ColorPrimaries::Bt470m,
            5 => ColorPrimaries::Bt470bg,
            6 => ColorPrimaries::Bt601,
            7 => ColorPrimaries::Smpte240,
            8 => ColorPrimaries::GenericFilm,
            9 => ColorPrimaries::Bt2020,
            10 => ColorPrimaries::Xyz,
            11 => ColorPrimaries::Smpte431,
            12 => ColorPrimaries::Smpte432,
            22 => ColorPrimaries::Ebu3213,
            31 => ColorPrimaries::Custom,
            _ => ColorPrimaries::Unknown,
        }
    }
}

/// Colorspace transfer characteristics.
///
/// These are as described by <https://www.itu.int/rec/T-REC-H.273-201612-I/en>.
///
/// Translation of `SDL_TransferCharacteristics`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TransferCharacteristics {
    #[default]
    Unknown = 0,
    /// Rec. ITU-R BT.709-6 / ITU-R BT1361
    Bt709 = 1,
    Unspecified = 2,
    /// ITU-R BT.470-6 System M / ITU-R BT1700 625 PAL & SECAM
    Gamma22 = 4,
    /// ITU-R BT.470-6 System B, G
    Gamma28 = 5,
    /// SMPTE ST 170M / ITU-R BT.601-7 525 or 625
    Bt601 = 6,
    /// SMPTE ST 240M
    Smpte240 = 7,
    Linear = 8,
    Log100 = 9,
    Log100Sqrt10 = 10,
    /// IEC 61966-2-4
    Iec61966 = 11,
    /// ITU-R BT1361 Extended Colour Gamut
    Bt1361 = 12,
    /// IEC 61966-2-1 (sRGB or sYCC)
    Srgb = 13,
    /// ITU-R BT2020 for 10-bit system
    Bt2020_10bit = 14,
    /// ITU-R BT2020 for 12-bit system
    Bt2020_12bit = 15,
    /// SMPTE ST 2084 for 10-, 12-, 14- and 16-bit systems
    Pq = 16,
    /// SMPTE ST 428-1
    Smpte428 = 17,
    /// ARIB STD-B67, known as "hybrid log-gamma" (HLG)
    Hlg = 18,
    Custom = 31,
}

impl TransferCharacteristics {
    pub const fn from_u32(v: u32) -> TransferCharacteristics {
        use TransferCharacteristics as T;
        match v {
            1 => T::Bt709,
            2 => T::Unspecified,
            4 => T::Gamma22,
            5 => T::Gamma28,
            6 => T::Bt601,
            7 => T::Smpte240,
            8 => T::Linear,
            9 => T::Log100,
            10 => T::Log100Sqrt10,
            11 => T::Iec61966,
            12 => T::Bt1361,
            13 => T::Srgb,
            14 => T::Bt2020_10bit,
            15 => T::Bt2020_12bit,
            16 => T::Pq,
            17 => T::Smpte428,
            18 => T::Hlg,
            31 => T::Custom,
            _ => T::Unknown,
        }
    }
}

/// Colorspace matrix coefficients.
///
/// These are as described by <https://www.itu.int/rec/T-REC-H.273-201612-I/en>.
///
/// Translation of `SDL_MatrixCoefficients`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum MatrixCoefficients {
    #[default]
    Identity = 0,
    /// ITU-R BT.709-6
    Bt709 = 1,
    Unspecified = 2,
    /// US FCC Title 47
    Fcc = 4,
    /// ITU-R BT.470-6 System B, G / ITU-R BT.601-7 625, functionally the same as `Bt601`
    Bt470bg = 5,
    /// ITU-R BT.601-7 525
    Bt601 = 6,
    /// SMPTE 240M
    Smpte240 = 7,
    Ycgco = 8,
    /// ITU-R BT.2020-2 non-constant luminance
    Bt2020Ncl = 9,
    /// ITU-R BT.2020-2 constant luminance
    Bt2020Cl = 10,
    /// SMPTE ST 2085
    Smpte2085 = 11,
    ChromaDerivedNcl = 12,
    ChromaDerivedCl = 13,
    /// ITU-R BT.2100-0 ICTCP
    Ictcp = 14,
    Custom = 31,
}

impl MatrixCoefficients {
    pub const fn from_u32(v: u32) -> MatrixCoefficients {
        use MatrixCoefficients as M;
        match v {
            0 => M::Identity,
            1 => M::Bt709,
            4 => M::Fcc,
            5 => M::Bt470bg,
            6 => M::Bt601,
            7 => M::Smpte240,
            8 => M::Ycgco,
            9 => M::Bt2020Ncl,
            10 => M::Bt2020Cl,
            11 => M::Smpte2085,
            12 => M::ChromaDerivedNcl,
            13 => M::ChromaDerivedCl,
            14 => M::Ictcp,
            31 => M::Custom,
            _ => M::Unspecified,
        }
    }
}

/// Colorspace chroma sample location. Translation of `SDL_ChromaLocation`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ChromaLocation {
    /// RGB, no chroma sampling
    #[default]
    None = 0,
    /// In MPEG-2, MPEG-4, and AVC, Cb and Cr are taken on midpoint of the left-edge of the 2x2 square.
    Left = 1,
    /// In JPEG/JFIF, H.261, and MPEG-1, Cb and Cr are taken at the center of the 2x2 square.
    Center = 2,
    /// In HEVC for BT.2020 and BT.2100 content, Cb and Cr are sampled at the same location as the group's top-left Y pixel ("co-sited", "co-located").
    TopLeft = 3,
}

impl ChromaLocation {
    pub const fn from_u32(v: u32) -> ChromaLocation {
        match v {
            1 => ChromaLocation::Left,
            2 => ChromaLocation::Center,
            3 => ChromaLocation::TopLeft,
            _ => ChromaLocation::None,
        }
    }
}

/// Translation of `SDL_DEFINE_COLORSPACE()`.
pub const fn define_colorspace(
    ty: ColorType,
    range: ColorRange,
    primaries: ColorPrimaries,
    transfer: TransferCharacteristics,
    matrix: MatrixCoefficients,
    chroma: ChromaLocation,
) -> Colorspace {
    Colorspace(
        ((ty as u32) << 28)
            | ((range as u32) << 24)
            | ((chroma as u32) << 20)
            | ((primaries as u32) << 10)
            | ((transfer as u32) << 5)
            | (matrix as u32),
    )
}

/// Colorspace definitions.
///
/// Since similar colorspaces may vary in their details (matrix, transfer
/// function, etc.), this is not an exhaustive list, but rather a
/// representative sample of the kinds of colorspaces supported in SDL.
///
/// Translation of `SDL_Colorspace`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Colorspace(pub u32);

impl Colorspace {
    pub const UNKNOWN: Colorspace = Colorspace(0);

    /// sRGB is a gamma corrected colorspace, and the default colorspace for SDL rendering and 8-bit RGB surfaces.
    /// Equivalent to DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709.
    pub const SRGB: Colorspace = define_colorspace(
        ColorType::Rgb,
        ColorRange::Full,
        ColorPrimaries::Bt709,
        TransferCharacteristics::Srgb,
        MatrixCoefficients::Identity,
        ChromaLocation::None,
    );
    /// This is a linear colorspace and the default colorspace for floating point surfaces. On Windows this is the scRGB colorspace, and on Apple platforms this is kCGColorSpaceExtendedLinearSRGB for EDR content.
    /// Equivalent to DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709.
    pub const SRGB_LINEAR: Colorspace = define_colorspace(
        ColorType::Rgb,
        ColorRange::Full,
        ColorPrimaries::Bt709,
        TransferCharacteristics::Linear,
        MatrixCoefficients::Identity,
        ChromaLocation::None,
    );
    /// HDR10 is a non-linear HDR colorspace and the default colorspace for 10-bit surfaces.
    /// Equivalent to DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020.
    pub const HDR10: Colorspace = define_colorspace(
        ColorType::Rgb,
        ColorRange::Full,
        ColorPrimaries::Bt2020,
        TransferCharacteristics::Pq,
        MatrixCoefficients::Identity,
        ChromaLocation::None,
    );
    /// Equivalent to DXGI_COLOR_SPACE_YCBCR_FULL_G22_NONE_P709_X601.
    pub const JPEG: Colorspace = define_colorspace(
        ColorType::Ycbcr,
        ColorRange::Full,
        ColorPrimaries::Bt709,
        TransferCharacteristics::Bt601,
        MatrixCoefficients::Bt601,
        ChromaLocation::None,
    );
    /// Equivalent to DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601.
    pub const BT601_LIMITED: Colorspace = define_colorspace(
        ColorType::Ycbcr,
        ColorRange::Limited,
        ColorPrimaries::Bt601,
        TransferCharacteristics::Bt601,
        MatrixCoefficients::Bt601,
        ChromaLocation::Left,
    );
    /// Equivalent to DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601.
    pub const BT601_FULL: Colorspace = define_colorspace(
        ColorType::Ycbcr,
        ColorRange::Full,
        ColorPrimaries::Bt601,
        TransferCharacteristics::Bt601,
        MatrixCoefficients::Bt601,
        ChromaLocation::Left,
    );
    /// Equivalent to DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709.
    pub const BT709_LIMITED: Colorspace = define_colorspace(
        ColorType::Ycbcr,
        ColorRange::Limited,
        ColorPrimaries::Bt709,
        TransferCharacteristics::Bt709,
        MatrixCoefficients::Bt709,
        ChromaLocation::Left,
    );
    /// Equivalent to DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709.
    pub const BT709_FULL: Colorspace = define_colorspace(
        ColorType::Ycbcr,
        ColorRange::Full,
        ColorPrimaries::Bt709,
        TransferCharacteristics::Bt709,
        MatrixCoefficients::Bt709,
        ChromaLocation::Left,
    );
    /// Equivalent to DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P2020.
    pub const BT2020_LIMITED: Colorspace = define_colorspace(
        ColorType::Ycbcr,
        ColorRange::Limited,
        ColorPrimaries::Bt2020,
        TransferCharacteristics::Pq,
        MatrixCoefficients::Bt2020Ncl,
        ChromaLocation::Left,
    );
    /// Equivalent to DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P2020.
    pub const BT2020_FULL: Colorspace = define_colorspace(
        ColorType::Ycbcr,
        ColorRange::Full,
        ColorPrimaries::Bt2020,
        TransferCharacteristics::Pq,
        MatrixCoefficients::Bt2020Ncl,
        ChromaLocation::Left,
    );
    /// The default colorspace for RGB surfaces if no colorspace is specified.
    pub const RGB_DEFAULT: Colorspace = Colorspace::SRGB;
    /// The default colorspace for YUV surfaces if no colorspace is specified.
    pub const YUV_DEFAULT: Colorspace = Colorspace::BT601_LIMITED;

    /// Translation of `SDL_COLORSPACETYPE()`.
    #[inline]
    pub const fn color_type(self) -> ColorType {
        match (self.0 >> 28) & 0x0F {
            1 => ColorType::Rgb,
            2 => ColorType::Ycbcr,
            _ => ColorType::Unknown,
        }
    }
    /// Translation of `SDL_COLORSPACERANGE()`.
    #[inline]
    pub const fn range(self) -> ColorRange {
        match (self.0 >> 24) & 0x0F {
            1 => ColorRange::Limited,
            2 => ColorRange::Full,
            _ => ColorRange::Unknown,
        }
    }
    /// Translation of `SDL_COLORSPACECHROMA()`.
    #[inline]
    pub const fn chroma(self) -> ChromaLocation {
        ChromaLocation::from_u32((self.0 >> 20) & 0x0F)
    }
    /// Translation of `SDL_COLORSPACEPRIMARIES()`.
    #[inline]
    pub const fn primaries(self) -> ColorPrimaries {
        ColorPrimaries::from_u32((self.0 >> 10) & 0x1F)
    }
    /// Translation of `SDL_COLORSPACETRANSFER()`.
    #[inline]
    pub const fn transfer(self) -> TransferCharacteristics {
        TransferCharacteristics::from_u32((self.0 >> 5) & 0x1F)
    }
    /// Translation of `SDL_COLORSPACEMATRIX()`.
    #[inline]
    pub const fn matrix(self) -> MatrixCoefficients {
        MatrixCoefficients::from_u32(self.0 & 0x1F)
    }
    /// Translation of `SDL_ISCOLORSPACE_MATRIX_BT601()`.
    #[inline]
    pub const fn is_matrix_bt601(self) -> bool {
        matches!(
            self.matrix(),
            MatrixCoefficients::Bt601 | MatrixCoefficients::Bt470bg
        )
    }
    /// Translation of `SDL_ISCOLORSPACE_MATRIX_BT709()`.
    #[inline]
    pub const fn is_matrix_bt709(self) -> bool {
        matches!(self.matrix(), MatrixCoefficients::Bt709)
    }
    /// Translation of `SDL_ISCOLORSPACE_MATRIX_BT2020_NCL()`.
    #[inline]
    pub const fn is_matrix_bt2020_ncl(self) -> bool {
        matches!(self.matrix(), MatrixCoefficients::Bt2020Ncl)
    }
    /// Translation of `SDL_ISCOLORSPACE_LIMITED_RANGE()`.
    #[inline]
    pub const fn is_limited_range(self) -> bool {
        !matches!(self.range(), ColorRange::Full)
    }
    /// Translation of `SDL_ISCOLORSPACE_FULL_RANGE()`.
    #[inline]
    pub const fn is_full_range(self) -> bool {
        matches!(self.range(), ColorRange::Full)
    }
}

/// A structure that represents a color as RGBA components.
///
/// The bits of this structure can be directly reinterpreted as an
/// integer-packed color which uses the `PixelFormat::RGBA32` format.
///
/// Translation of `SDL_Color`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// The bits of this structure can be directly reinterpreted as a float-packed
/// color which uses the `PixelFormat::RGBA128_FLOAT` format.
///
/// Translation of `SDL_FColor`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// A set of indexed colors representing a palette. Translation of `SDL_Palette`.
///
/// The C struct's `ncolors`/`colors` are [`Palette::len`]/[`Palette::colors`];
/// `refcount` is replaced by Rust ownership (share with `Arc<Palette>`), and
/// `version` is bumped automatically by every mutation so blit maps can detect
/// changes exactly as upstream does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Palette {
    colors: Vec<Color>,
    version: u32,
}

/// Details about the format of a pixel.
///
/// Translation of `SDL_PixelFormatDetails`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[allow(non_snake_case)]
pub struct PixelFormatDetails {
    pub format: PixelFormat,
    pub bits_per_pixel: u8,
    pub bytes_per_pixel: u8,
    pub padding: [u8; 2],
    pub Rmask: u32,
    pub Gmask: u32,
    pub Bmask: u32,
    pub Amask: u32,
    pub Rbits: u8,
    pub Gbits: u8,
    pub Bbits: u8,
    pub Abits: u8,
    pub Rshift: u8,
    pub Gshift: u8,
    pub Bshift: u8,
    pub Ashift: u8,
}

// ---------------------------------------------------------------------------
// Lookup tables to expand partial bytes to the full 0..255 range.
//
// Upstream ships the tables as literals generated with the "GENERATE_SHIFTS"
// program in SDL_pixels.c; here the same formula runs at compile time.
// ---------------------------------------------------------------------------

const fn expand_bits(v: u32, bits: u32) -> u8 {
    (match bits {
        1 => (v << 7) | (v << 6) | (v << 5) | (v << 4) | (v << 3) | (v << 2) | (v << 1) | v,
        2 => (v << 6) | (v << 4) | (v << 2) | v,
        3 => (v << 5) | (v << 2) | (v >> 1),
        4 => (v << 4) | v,
        5 => (v << 3) | (v >> 2),
        6 => (v << 2) | (v >> 4),
        7 => (v << 1) | (v >> 6),
        _ => v,
    }) as u8
}

const fn make_lookup<const N: usize>(bits: u32) -> [u8; N] {
    let mut table = [0u8; N];
    let mut i = 0;
    while i < N {
        table[i] = expand_bits(i as u32, bits);
        i += 1;
    }
    table
}

static LOOKUP_0: [u8; 1] = [255];
static LOOKUP_1: [u8; 2] = make_lookup::<2>(1);
static LOOKUP_2: [u8; 4] = make_lookup::<4>(2);
static LOOKUP_3: [u8; 8] = make_lookup::<8>(3);
static LOOKUP_4: [u8; 16] = make_lookup::<16>(4);
static LOOKUP_5: [u8; 32] = make_lookup::<32>(5);
static LOOKUP_6: [u8; 64] = make_lookup::<64>(6);
static LOOKUP_7: [u8; 128] = make_lookup::<128>(7);
static LOOKUP_8: [u8; 256] = make_lookup::<256>(8);

/// Translation of `SDL_expand_byte[9]`: `EXPAND_BYTE[bits][v]` maps a
/// `bits`-wide value to 0..255.
pub static EXPAND_BYTE: [&[u8]; 9] = [
    &LOOKUP_0, &LOOKUP_1, &LOOKUP_2, &LOOKUP_3, &LOOKUP_4, &LOOKUP_5, &LOOKUP_6, &LOOKUP_7,
    &LOOKUP_8,
];

const fn make_expand_byte_10() -> [u16; 256] {
    // round(v * 1023 / 255), which reproduces the upstream literal table
    let mut table = [0u16; 256];
    let mut v = 0usize;
    while v < 256 {
        table[v] = ((v as u32 * 1023 * 2 + 255) / 510) as u16;
        v += 1;
    }
    table
}

/// Lookup table to expand 8 bit to 10 bit range. Translation of `SDL_expand_byte_10`.
pub static EXPAND_BYTE_10: [u16; 256] = make_expand_byte_10();

impl Color {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }
    /// An opaque color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color {
            r,
            g,
            b,
            a: ALPHA_OPAQUE,
        }
    }
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
}

impl FColor {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> FColor {
        FColor { r, g, b, a }
    }
}

impl From<Color> for FColor {
    fn from(c: Color) -> FColor {
        FColor {
            r: c.r as f32 / 255.0,
            g: c.g as f32 / 255.0,
            b: c.b as f32 / 255.0,
            a: c.a as f32 / 255.0,
        }
    }
}

/// RGBA bit masks and depth describing a pixel format, as used by
/// [`PixelFormat::masks`] and [`PixelFormat::from_masks`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PixelMasks {
    /// Bits per pixel.
    pub bpp: u32,
    pub r: u32,
    pub g: u32,
    pub b: u32,
    pub a: u32,
}

impl PixelFormat {
    /// The format's name, e.g. `"SDL_PIXELFORMAT_ARGB8888"` (`"SDL_PIXELFORMAT_UNKNOWN"`
    /// for values that are not SDL formats). Translation of `SDL_GetPixelFormatName()`.
    pub fn name(self) -> &'static str {
        macro_rules! names {
            ($($name:ident),* $(,)?) => {
                match self {
                    $( PixelFormat::$name => concat!("SDL_PIXELFORMAT_", stringify!($name)), )*
                    _ => "SDL_PIXELFORMAT_UNKNOWN",
                }
            };
        }
        names!(
            INDEX1LSB,
            INDEX1MSB,
            INDEX2LSB,
            INDEX2MSB,
            INDEX4LSB,
            INDEX4MSB,
            INDEX8,
            RGB332,
            XRGB4444,
            XBGR4444,
            XRGB1555,
            XBGR1555,
            ARGB4444,
            RGBA4444,
            ABGR4444,
            BGRA4444,
            ARGB1555,
            RGBA5551,
            ABGR1555,
            BGRA5551,
            RGB565,
            BGR565,
            RGB24,
            BGR24,
            XRGB8888,
            RGBX8888,
            XBGR8888,
            BGRX8888,
            ARGB8888,
            RGBA8888,
            ABGR8888,
            BGRA8888,
            XRGB2101010,
            XBGR2101010,
            ARGB2101010,
            ABGR2101010,
            RGB48,
            BGR48,
            RGBA64,
            ARGB64,
            BGRA64,
            ABGR64,
            RGB48_FLOAT,
            BGR48_FLOAT,
            RGBA64_FLOAT,
            ARGB64_FLOAT,
            BGRA64_FLOAT,
            ABGR64_FLOAT,
            RGB96_FLOAT,
            BGR96_FLOAT,
            RGBA128_FLOAT,
            ARGB128_FLOAT,
            BGRA128_FLOAT,
            ABGR128_FLOAT,
            YV12,
            IYUV,
            YUY2,
            UYVY,
            YVYU,
            NV12,
            NV21,
            P010,
            I444,
            I0FL,
            I4FL,
            EXTERNAL_OES,
            MJPG,
        )
    }

    /// The bits-per-pixel and RGBA masks of this format. FourCC formats have
    /// no masks (all zero) but packed YUV formats still report a depth.
    /// Translation of `SDL_GetMasksForPixelFormat()`.
    pub fn masks(self) -> Result<PixelMasks> {
        let big_endian = cfg!(target_endian = "big");
        let mut out = PixelMasks::default();

        // Partial support for SDL_Surface with FOURCC
        if self.is_fourcc() {
            // Not a format that uses masks
            // however, some of these are packed formats, and can legit declare bits-per-pixel!
            out.bpp = match self {
                PixelFormat::YUY2 | PixelFormat::UYVY | PixelFormat::YVYU => 32,
                _ => 0, // oh well.
            };
            return Ok(out);
        }

        // Initialize the values here
        out.bpp = if self.bytes_per_pixel() <= 2 {
            self.bits_per_pixel()
        } else {
            self.bytes_per_pixel() * 8
        };

        if self.pixel_type() == PixelType::ArrayU8 {
            // (R, G, B, A) masks for (big endian, little endian)
            let masks: Option<([u32; 4], [u32; 4])> =
                match (self.bytes_per_pixel(), self.pixel_order()) {
                    (3, o) if o == ArrayOrder::Rgb as u32 => Some((
                        [0x00FF0000, 0x0000FF00, 0x000000FF, 0],
                        [0x000000FF, 0x0000FF00, 0x00FF0000, 0],
                    )),
                    (3, o) if o == ArrayOrder::Bgr as u32 => Some((
                        [0x000000FF, 0x0000FF00, 0x00FF0000, 0],
                        [0x00FF0000, 0x0000FF00, 0x000000FF, 0],
                    )),
                    (4, o) if o == ArrayOrder::Rgba as u32 => Some((
                        [0xFF000000, 0x00FF0000, 0x0000FF00, 0x000000FF],
                        [0x000000FF, 0x0000FF00, 0x00FF0000, 0xFF000000],
                    )),
                    (4, o) if o == ArrayOrder::Argb as u32 => Some((
                        [0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000],
                        [0x0000FF00, 0x00FF0000, 0xFF000000, 0x000000FF],
                    )),
                    (4, o) if o == ArrayOrder::Bgra as u32 => Some((
                        [0x0000FF00, 0x00FF0000, 0xFF000000, 0x000000FF],
                        [0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000],
                    )),
                    (4, o) if o == ArrayOrder::Abgr as u32 => Some((
                        [0x000000FF, 0x0000FF00, 0x00FF0000, 0xFF000000],
                        [0xFF000000, 0x00FF0000, 0x0000FF00, 0x000000FF],
                    )),
                    _ => None,
                };
            return match masks {
                Some((be, le)) => {
                    let m = if big_endian { be } else { le };
                    Ok(PixelMasks {
                        bpp: out.bpp,
                        r: m[0],
                        g: m[1],
                        b: m[2],
                        a: m[3],
                    })
                }
                None => Err(err!("Unknown pixel format")),
            };
        }

        if !matches!(
            self.pixel_type(),
            PixelType::Packed8 | PixelType::Packed16 | PixelType::Packed32
        ) {
            // Not a format that uses masks
            return Ok(out);
        }

        let masks: [u32; 4] = match self.pixel_layout() {
            PackedLayout::L332 => [0x00000000, 0x000000E0, 0x0000001C, 0x00000003],
            PackedLayout::L4444 => [0x0000F000, 0x00000F00, 0x000000F0, 0x0000000F],
            PackedLayout::L1555 => [0x00008000, 0x00007C00, 0x000003E0, 0x0000001F],
            PackedLayout::L5551 => [0x0000F800, 0x000007C0, 0x0000003E, 0x00000001],
            PackedLayout::L565 => [0x00000000, 0x0000F800, 0x000007E0, 0x0000001F],
            PackedLayout::L8888 => [0xFF000000, 0x00FF0000, 0x0000FF00, 0x000000FF],
            PackedLayout::L2101010 => [0xC0000000, 0x3FF00000, 0x000FFC00, 0x000003FF],
            PackedLayout::L1010102 => [0xFFC00000, 0x003FF000, 0x00000FFC, 0x00000003],
            PackedLayout::None => return Err(err!("Unknown pixel format")),
        };

        let o = self.pixel_order();
        let (r, g, b, a) = if o == PackedOrder::Xrgb as u32 {
            (masks[1], masks[2], masks[3], 0)
        } else if o == PackedOrder::Rgbx as u32 {
            (masks[0], masks[1], masks[2], 0)
        } else if o == PackedOrder::Argb as u32 {
            (masks[1], masks[2], masks[3], masks[0])
        } else if o == PackedOrder::Rgba as u32 {
            (masks[0], masks[1], masks[2], masks[3])
        } else if o == PackedOrder::Xbgr as u32 {
            (masks[3], masks[2], masks[1], 0)
        } else if o == PackedOrder::Bgrx as u32 {
            (masks[2], masks[1], masks[0], 0)
        } else if o == PackedOrder::Bgra as u32 {
            (masks[2], masks[1], masks[0], masks[3])
        } else if o == PackedOrder::Abgr as u32 {
            (masks[3], masks[2], masks[1], masks[0])
        } else {
            return Err(err!("Unknown pixel format"));
        };
        Ok(PixelMasks {
            bpp: out.bpp,
            r,
            g,
            b,
            a,
        })
    }

    /// The format matching a depth and RGBA masks, or `None`.
    /// Translation of `SDL_GetPixelFormatForMasks()`.
    #[allow(clippy::unusual_byte_groupings)]
    pub fn from_masks(m: PixelMasks) -> Option<PixelFormat> {
        let big_endian = cfg!(target_endian = "big");
        let PixelMasks { bpp, r, g, b, a } = m;
        let masks = (r, g, b, a);
        let found = match bpp {
            // SDL defaults to MSB ordering
            1 => PixelFormat::INDEX1MSB,
            2 => PixelFormat::INDEX2MSB,
            4 => PixelFormat::INDEX4MSB,
            8 => {
                if masks == (0xE0, 0x1C, 0x03, 0x00) {
                    PixelFormat::RGB332
                } else {
                    PixelFormat::INDEX8
                }
            }
            12 => match masks {
                _ if r == 0 => PixelFormat::XRGB4444,
                (0x0F00, 0x00F0, 0x000F, 0x0000) => PixelFormat::XRGB4444,
                (0x000F, 0x00F0, 0x0F00, 0x0000) => PixelFormat::XBGR4444,
                _ => return None,
            },
            15 | 16 => match masks {
                _ if bpp == 15 && r == 0 => PixelFormat::XRGB1555,
                // SDL_FALLTHROUGH into the 16 case
                _ if r == 0 => PixelFormat::RGB565,
                (0x7C00, 0x03E0, 0x001F, 0x0000) => PixelFormat::XRGB1555,
                (0x001F, 0x03E0, 0x7C00, 0x0000) => PixelFormat::XBGR1555,
                (0x0F00, 0x00F0, 0x000F, 0xF000) => PixelFormat::ARGB4444,
                (0xF000, 0x0F00, 0x00F0, 0x000F) => PixelFormat::RGBA4444,
                (0x000F, 0x00F0, 0x0F00, 0xF000) => PixelFormat::ABGR4444,
                (0x00F0, 0x0F00, 0xF000, 0x000F) => PixelFormat::BGRA4444,
                (0x7C00, 0x03E0, 0x001F, 0x8000) => PixelFormat::ARGB1555,
                (0xF800, 0x07C0, 0x003E, 0x0001) => PixelFormat::RGBA5551,
                (0x001F, 0x03E0, 0x7C00, 0x8000) => PixelFormat::ABGR1555,
                (0x003E, 0x07C0, 0xF800, 0x0001) => PixelFormat::BGRA5551,
                (0xF800, 0x07E0, 0x001F, 0x0000) => PixelFormat::RGB565,
                (0x001F, 0x07E0, 0xF800, 0x0000) => PixelFormat::BGR565,
                // Technically this would be BGR556, but Witek says this works in bug 3158
                (0x003F, 0x07C0, 0xF800, 0x0000) => PixelFormat::RGB565,
                _ => return None,
            },
            24 => match r {
                0 | 0x00FF0000 => {
                    if big_endian {
                        PixelFormat::RGB24
                    } else {
                        PixelFormat::BGR24
                    }
                }
                0x000000FF => {
                    if big_endian {
                        PixelFormat::BGR24
                    } else {
                        PixelFormat::RGB24
                    }
                }
                _ => return None,
            },
            30 => match masks {
                (0x3FF00000, 0x000FFC00, 0x000003FF, 0x00000000) => PixelFormat::XRGB2101010,
                (0x000003FF, 0x000FFC00, 0x3FF00000, 0x00000000) => PixelFormat::XBGR2101010,
                _ => return None,
            },
            32 => match masks {
                _ if r == 0 => PixelFormat::XRGB8888,
                (0x00FF0000, 0x0000FF00, 0x000000FF, 0x00000000) => PixelFormat::XRGB8888,
                (0xFF000000, 0x00FF0000, 0x0000FF00, 0x00000000) => PixelFormat::RGBX8888,
                (0x000000FF, 0x0000FF00, 0x00FF0000, 0x00000000) => PixelFormat::XBGR8888,
                (0x0000FF00, 0x00FF0000, 0xFF000000, 0x00000000) => PixelFormat::BGRX8888,
                (0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000) => PixelFormat::ARGB8888,
                (0xFF000000, 0x00FF0000, 0x0000FF00, 0x000000FF) => PixelFormat::RGBA8888,
                (0x000000FF, 0x0000FF00, 0x00FF0000, 0xFF000000) => PixelFormat::ABGR8888,
                (0x0000FF00, 0x00FF0000, 0xFF000000, 0x000000FF) => PixelFormat::BGRA8888,
                (0x3FF00000, 0x000FFC00, 0x000003FF, 0x00000000) => PixelFormat::XRGB2101010,
                (0x000003FF, 0x000FFC00, 0x3FF00000, 0x00000000) => PixelFormat::XBGR2101010,
                (0x3FF00000, 0x000FFC00, 0x000003FF, 0xC0000000) => PixelFormat::ARGB2101010,
                (0x000003FF, 0x000FFC00, 0x3FF00000, 0xC0000000) => PixelFormat::ABGR2101010,
                _ => return None,
            },
            _ => return None,
        };
        Some(found)
    }

    /// Full channel layout details. Translation of `SDL_GetPixelFormatDetails()`
    /// (upstream caches these in a hash table; `PixelFormatDetails` is `Copy`
    /// here, so it is computed on demand).
    pub fn details(self) -> Result<PixelFormatDetails> {
        PixelFormatDetails::new(self)
    }

    /// The colorspace assumed for this format when none is specified.
    /// Translation of the internal `SDL_GetDefaultColorspaceForFormat()`.
    pub fn default_colorspace(self) -> Colorspace {
        if self.is_fourcc() {
            if self == PixelFormat::MJPG {
                Colorspace::SRGB
            } else if self == PixelFormat::P010
                || self == PixelFormat::I0FL
                || self == PixelFormat::I4FL
            {
                Colorspace::HDR10
            } else {
                Colorspace::YUV_DEFAULT
            }
        } else if self.is_float() {
            Colorspace::SRGB_LINEAR
        } else if self.is_10bit() {
            Colorspace::HDR10
        } else {
            Colorspace::RGB_DEFAULT
        }
    }
}

impl std::fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

fn mask_shift_and_bits(mask: u32) -> (u8, u8) {
    let (mut shift, mut bits) = (0u8, 0u8);
    if mask != 0 {
        let mut m = mask;
        while m & 0x01 == 0 {
            shift += 1;
            m >>= 1;
        }
        while m & 0x01 != 0 {
            bits += 1;
            m >>= 1;
        }
    }
    (shift, bits)
}

impl PixelFormatDetails {
    /// Translation of `SDL_InitPixelFormatDetails()`.
    pub fn new(format: PixelFormat) -> Result<PixelFormatDetails> {
        let masks = format.masks()?;

        // Set up the format
        let mut d = PixelFormatDetails {
            format,
            ..Default::default()
        };
        d.bits_per_pixel = masks.bpp as u8;
        d.bytes_per_pixel = masks.bpp.div_ceil(8) as u8;
        d.Rmask = masks.r;
        (d.Rshift, d.Rbits) = mask_shift_and_bits(masks.r);
        d.Gmask = masks.g;
        (d.Gshift, d.Gbits) = mask_shift_and_bits(masks.g);
        d.Bmask = masks.b;
        (d.Bshift, d.Bbits) = mask_shift_and_bits(masks.b);
        d.Amask = masks.a;
        (d.Ashift, d.Abits) = mask_shift_and_bits(masks.a);
        Ok(d)
    }

    /// For an 8888 format, the mask and shift of the alpha (or unused) byte.
    /// Translation of the internal `SDL_Get8888AlphaMaskAndShift()`.
    pub fn alpha_mask_and_shift_8888(&self) -> (u32, u32) {
        if self.Amask != 0 {
            return (self.Amask, self.Ashift as u32);
        }
        let mask = !(self.Rmask | self.Gmask | self.Bmask);
        let shift = match mask {
            0x000000FF => 0,
            0x0000FF00 => 8,
            0x00FF0000 => 16,
            0xFF000000 => 24,
            _ => 0, // Should never happen
        };
        (mask, shift)
    }

    /// Pack an opaque color into a pixel value. Indexed formats need a palette
    /// and return the nearest palette index. Translation of `SDL_MapRGB()`.
    pub fn map_rgb(&self, palette: Option<&Palette>, color: Color) -> Result<u32> {
        self.map_rgba(
            palette,
            Color {
                a: ALPHA_OPAQUE,
                ..color
            },
        )
    }

    /// Pack a color into a pixel value. Indexed formats need a palette and
    /// return the nearest palette index. Translation of `SDL_MapRGBA()`.
    pub fn map_rgba(&self, palette: Option<&Palette>, c: Color) -> Result<u32> {
        if self.format.is_indexed() {
            let palette = palette.ok_or_else(|| Error::invalid_param("palette"))?;
            return Ok(palette.find_color(c) as u32);
        }

        // (a >> (8 - Abits)) with Abits == 0 is masked away by Amask (0) in C;
        // guard the 8-bit shift here.
        let a_part = if self.Abits == 0 {
            0
        } else {
            (((c.a >> (8 - self.Abits)) as u32) << self.Ashift) & self.Amask
        };

        Ok(if self.format.is_10bit() {
            ((EXPAND_BYTE_10[c.r as usize] as u32) << self.Rshift)
                | ((EXPAND_BYTE_10[c.g as usize] as u32) << self.Gshift)
                | ((EXPAND_BYTE_10[c.b as usize] as u32) << self.Bshift)
                | a_part
        } else {
            ((c.r >> (8 - self.Rbits)) as u32) << self.Rshift
                | ((c.g >> (8 - self.Gbits)) as u32) << self.Gshift
                | ((c.b >> (8 - self.Bbits)) as u32) << self.Bshift
                | a_part
        })
    }

    /// Unpack a pixel value into a color; the inverse of [`map_rgba`](Self::map_rgba).
    /// Channels narrower than 8 bits are expanded to the full 0–255 range.
    /// Translation of `SDL_GetRGBA()`.
    pub fn get_rgba(&self, pixel: u32, palette: Option<&Palette>) -> Color {
        if self.format.is_indexed() {
            return match palette {
                Some(p) if (pixel as usize) < p.colors.len() => p.colors[pixel as usize],
                _ => Color::new(0, 0, 0, 0),
            };
        }
        let a = EXPAND_BYTE[self.Abits as usize][((pixel & self.Amask) >> self.Ashift) as usize];
        if self.format.is_10bit() {
            Color {
                r: (((pixel & self.Rmask) >> self.Rshift) >> 2) as u8,
                g: (((pixel & self.Gmask) >> self.Gshift) >> 2) as u8,
                b: (((pixel & self.Bmask) >> self.Bshift) >> 2) as u8,
                a,
            }
        } else {
            Color {
                r: EXPAND_BYTE[self.Rbits as usize][((pixel & self.Rmask) >> self.Rshift) as usize],
                g: EXPAND_BYTE[self.Gbits as usize][((pixel & self.Gmask) >> self.Gshift) as usize],
                b: EXPAND_BYTE[self.Bbits as usize][((pixel & self.Bmask) >> self.Bshift) as usize],
                a,
            }
        }
    }

    /// Unpack a pixel value into an opaque color. Translation of `SDL_GetRGB()`.
    pub fn get_rgb(&self, pixel: u32, palette: Option<&Palette>) -> Color {
        let c = self.get_rgba(pixel, palette);
        if self.format.is_indexed() && !palette.is_some_and(|p| (pixel as usize) < p.colors.len()) {
            return Color::new(0, 0, 0, ALPHA_OPAQUE);
        }
        Color {
            a: ALPHA_OPAQUE,
            ..c
        }
    }
}

/// Translation of `SDL_sRGBtoLinear()`.
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Translation of `SDL_sRGBfromLinear()`.
pub fn srgb_from_linear(v: f32) -> f32 {
    if v <= 0.0031308 {
        v * 12.92
    } else {
        v.powf(1.0 / 2.4) * 1.055 - 0.055
    }
}

/// SMPTE ST 2084 perceptual quantizer to nits. Translation of `SDL_PQtoNits()`.
pub fn pq_to_nits(v: f32) -> f32 {
    let c1 = 0.8359375f32;
    let c2 = 18.8515625f32;
    let c3 = 18.6875f32;
    let oo_m1 = 1.0f32 / 0.1593017578125;
    let oo_m2 = 1.0f32 / 78.84375;

    let num = (v.powf(oo_m2) - c1).max(0.0);
    let den = c2 - c3 * v.powf(oo_m2);
    10000.0 * (num / den).powf(oo_m1)
}

/// Nits to SMPTE ST 2084 perceptual quantizer. Translation of `SDL_PQfromNits()`.
pub fn pq_from_nits(v: f32) -> f32 {
    let c1 = 0.8359375f32;
    let c2 = 18.8515625f32;
    let c3 = 18.6875f32;
    let m1 = 0.1593017578125f32;
    let m2 = 78.84375f32;

    let y = (v / 10000.0).clamp(0.0, 1.0);
    let num = c1 + c2 * y.powf(m1);
    let den = 1.0 + c3 * y.powf(m1);
    (num / den).powf(m2)
}

/// A YCbCr → RGB conversion: `rgb = coeff * (ycbcr + offset)`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct YCbCrMatrix {
    /// Added to (Y, Cb, Cr) before applying the coefficients.
    pub offset: [f32; 3],
    /// Rows are R, G, B; columns multiply Y, Cb, Cr.
    pub coeff: [[f32; 3]; 3],
}

/* This is a helpful tool for deriving these:
 * https://kdashg.github.io/misc/colors/from-coeffs.html
 */
static MAT_BT601_LIMITED_8BIT: YCbCrMatrix = YCbCrMatrix {
    offset: [-0.0627451017, -0.501960814, -0.501960814],
    coeff: [
        [1.1644, 0.0000, 1.5960],
        [1.1644, -0.3918, -0.8130],
        [1.1644, 2.0172, 0.0000],
    ],
};
static MAT_BT601_FULL_8BIT: YCbCrMatrix = YCbCrMatrix {
    offset: [0.0, -0.501960814, -0.501960814],
    coeff: [
        [1.0000, 0.0000, 1.4075],
        [1.0000, -0.3455, -0.7169],
        [1.0000, 1.7790, 0.0000],
    ],
};
static MAT_BT709_LIMITED_8BIT: YCbCrMatrix = YCbCrMatrix {
    offset: [-0.0627451017, -0.501960814, -0.501960814],
    coeff: [
        [1.1644, 0.0000, 1.7927],
        [1.1644, -0.2132, -0.5329],
        [1.1644, 2.1124, 0.0000],
    ],
};
static MAT_BT709_FULL_8BIT: YCbCrMatrix = YCbCrMatrix {
    offset: [0.0, -0.501960814, -0.501960814],
    coeff: [
        [1.0000, 0.0000, 1.5810],
        [1.0000, -0.1881, -0.4700],
        [1.0000, 1.8629, 0.0000],
    ],
};
static MAT_BT2020_LIMITED_10BIT: YCbCrMatrix = YCbCrMatrix {
    offset: [-0.062561095, -0.500488759, -0.500488759],
    coeff: [
        [1.1678, 0.0000, 1.6836],
        [1.1678, -0.1879, -0.6523],
        [1.1678, 2.1481, 0.0000],
    ],
};
static MAT_BT2020_FULL_10BIT: YCbCrMatrix = YCbCrMatrix {
    offset: [0.0, -0.500488759, -0.500488759],
    coeff: [
        [1.0000, 0.0000, 1.4760],
        [1.0000, -0.1647, -0.5719],
        [1.0000, 1.8832, 0.0000],
    ],
};

impl Colorspace {
    fn bt601_matrix(self) -> &'static YCbCrMatrix {
        match self.range() {
            ColorRange::Full => &MAT_BT601_FULL_8BIT,
            ColorRange::Limited | ColorRange::Unknown => &MAT_BT601_LIMITED_8BIT,
        }
    }
    fn bt709_matrix(self) -> &'static YCbCrMatrix {
        match self.range() {
            ColorRange::Full => &MAT_BT709_FULL_8BIT,
            ColorRange::Limited | ColorRange::Unknown => &MAT_BT709_LIMITED_8BIT,
        }
    }
    fn bt2020_matrix(self) -> &'static YCbCrMatrix {
        match self.range() {
            ColorRange::Full => &MAT_BT2020_FULL_10BIT,
            ColorRange::Limited | ColorRange::Unknown => &MAT_BT2020_LIMITED_10BIT,
        }
    }

    /// The YCbCr → RGB matrix for this colorspace. When the matrix
    /// coefficients are unspecified, SDL picks BT.601 for SD content (height
    /// ≤ 576) and BT.709 for HD at 8 bits, and BT.2020 at 10/16 bits.
    /// Translation of the internal `SDL_GetYCbCRtoRGBConversionMatrix()`.
    pub fn ycbcr_to_rgb_matrix(
        self,
        height: u32,
        bits_per_pixel: u32,
    ) -> Option<&'static YCbCrMatrix> {
        const YUV_SD_THRESHOLD: u32 = 576;
        match self.matrix() {
            MatrixCoefficients::Bt601 | MatrixCoefficients::Bt470bg => Some(self.bt601_matrix()),
            MatrixCoefficients::Bt709 => Some(self.bt709_matrix()),
            MatrixCoefficients::Bt2020Ncl => Some(self.bt2020_matrix()),
            MatrixCoefficients::Unspecified => match bits_per_pixel {
                8 => Some(if height <= YUV_SD_THRESHOLD {
                    self.bt601_matrix()
                } else {
                    self.bt709_matrix()
                }),
                10 | 16 => Some(self.bt2020_matrix()),
                _ => None,
            },
            _ => None,
        }
    }
}

impl std::fmt::Display for Colorspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Colorspace({:#010x})", self.0)
    }
}

/// A 3×3 RGB → RGB matrix (rows multiply an `[r, g, b]` column vector).
pub type PrimariesMatrix = [[f32; 3]; 3];

/* Conversion matrices generated using gamescope color helpers and the primaries definitions at:
 * https://www.itu.int/rec/T-REC-H.273-201612-S/en
 *
 * You can also generate these online using the RGB-XYZ matrix calculator, and then multiplying
 * XYZ_to_dst * src_to_XYZ to get the combined conversion matrix:
 * https://www.russellcottrell.com/photo/matrixCalculator.htm
 */
static MAT601TO709: PrimariesMatrix = [
    [0.939542, 0.050181, 0.010277],
    [0.017772, 0.965793, 0.016435],
    [-0.001622, -0.004370, 1.005991],
];
static MAT601TO2020: PrimariesMatrix = [
    [0.595254, 0.349314, 0.055432],
    [0.081244, 0.891503, 0.027253],
    [0.015512, 0.081912, 0.902576],
];
static MAT709TO601: PrimariesMatrix = [
    [1.065379, -0.055401, -0.009978],
    [-0.019633, 1.036363, -0.016731],
    [0.001632, 0.004412, 0.993956],
];
static MAT709TO2020: PrimariesMatrix = [
    [0.627404, 0.329283, 0.043313],
    [0.069097, 0.919541, 0.011362],
    [0.016391, 0.088013, 0.895595],
];
static MAT2020TO601: PrimariesMatrix = [
    [1.776133, -0.687820, -0.088313],
    [-0.161376, 1.187315, -0.025940],
    [-0.015881, -0.095931, 1.111812],
];
static MAT2020TO709: PrimariesMatrix = [
    [1.660496, -0.587656, -0.072840],
    [-0.124547, 1.132895, -0.008348],
    [-0.018154, -0.100597, 1.118751],
];
static MATSMPTE431TO709: PrimariesMatrix = [
    [1.120713, -0.234649, 0.000000],
    [-0.038478, 1.087034, 0.000000],
    [-0.017967, -0.082030, 0.954576],
];
static MATSMPTE431TO2020: PrimariesMatrix = [
    [0.689691, 0.207169, 0.041346],
    [0.041852, 0.982426, 0.010846],
    [-0.001107, 0.018362, 0.854914],
];
static MATSMPTE432TO709: PrimariesMatrix = [
    [1.224940, -0.224940, -0.000000],
    [-0.042057, 1.042057, 0.000000],
    [-0.019638, -0.078636, 1.098273],
];
static MATSMPTE432TO2020: PrimariesMatrix = [
    [0.753833, 0.198597, 0.047570],
    [0.045744, 0.941777, 0.012479],
    [-0.001210, 0.017602, 0.983609],
];

impl ColorPrimaries {
    /// The matrix converting linear RGB in `self` to linear RGB in `dst`, for
    /// the pairs SDL knows. Translation of the internal `SDL_GetColorPrimariesConversionMatrix()`.
    pub fn conversion_matrix_to(self, dst: ColorPrimaries) -> Option<&'static PrimariesMatrix> {
        use ColorPrimaries as P;
        match dst {
            P::Bt601 | P::Smpte240 => match self {
                P::Bt709 => Some(&MAT709TO601),
                P::Bt2020 => Some(&MAT2020TO601),
                _ => None,
            },
            P::Bt709 => match self {
                P::Bt601 | P::Smpte240 => Some(&MAT601TO709),
                P::Bt2020 => Some(&MAT2020TO709),
                P::Smpte431 => Some(&MATSMPTE431TO709),
                P::Smpte432 => Some(&MATSMPTE432TO709),
                _ => None,
            },
            P::Bt2020 => match self {
                P::Bt601 | P::Smpte240 => Some(&MAT601TO2020),
                P::Bt709 => Some(&MAT709TO2020),
                P::Smpte431 => Some(&MATSMPTE431TO2020),
                P::Smpte432 => Some(&MATSMPTE432TO2020),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Apply a primaries conversion matrix to a linear `[r, g, b]`.
/// Translation of the internal `SDL_ConvertColorPrimaries()`.
pub fn convert_color_primaries(rgb: [f32; 3], matrix: &PrimariesMatrix) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (o, row) in out.iter_mut().zip(matrix) {
        *o = row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2];
    }
    out
}

/// Translation of the internal `SDL_ConvertColor709to2020()`.
pub fn convert_color_709_to_2020(rgb: [f32; 3]) -> [f32; 3] {
    convert_color_primaries(rgb, &MAT709TO2020)
}

/// Whether a palette's alpha channel carries information. Result of [`Palette::alpha_info`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PaletteAlpha {
    /// Every entry is fully opaque or every entry is fully transparent.
    pub is_opaque: bool,
    /// The alpha values are meaningful (false only when all entries are fully transparent,
    /// which SDL treats as "no alpha channel").
    pub has_alpha_channel: bool,
}

impl Palette {
    /// A palette of `ncolors` entries, all initialized to opaque white.
    /// Translation of `SDL_CreatePalette()`.
    pub fn new(ncolors: usize) -> Result<Palette> {
        if ncolors < 1 {
            return Err(Error::invalid_param("ncolors"));
        }
        Ok(Palette {
            colors: vec![Color::WHITE; ncolors],
            version: 1,
        })
    }

    /// Build from existing colors.
    pub fn from_colors(colors: Vec<Color>) -> Result<Palette> {
        if colors.is_empty() {
            return Err(Error::invalid_param("ncolors"));
        }
        Ok(Palette { colors, version: 1 })
    }

    /// Number of entries (`ncolors`).
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn colors(&self) -> &[Color] {
        &self.colors
    }

    /// Mutable access; bumps the version.
    pub fn colors_mut(&mut self) -> &mut [Color] {
        self.bump_version();
        &mut self.colors
    }

    /// Change counter: bumped on every mutation, never 0 (`SDL_Palette::version`).
    pub fn version(&self) -> u32 {
        self.version
    }

    fn bump_version(&mut self) {
        self.version = self.version.wrapping_add(1);
        if self.version == 0 {
            self.version = 1;
        }
    }

    /// Overwrite entries starting at `first`; colors past the end are
    /// ignored. Fails if `first` is out of range. Translation of `SDL_SetPaletteColors()`.
    pub fn set_colors(&mut self, first: usize, colors: &[Color]) -> Result<()> {
        if first > self.colors.len() {
            return Err(Error::invalid_param("firstcolor"));
        }
        let n = colors.len().min(self.colors.len() - first);
        self.colors[first..first + n].copy_from_slice(&colors[..n]);
        self.bump_version();
        Ok(())
    }

    /// Fill a 256-entry palette with the 3-3-2 "dithered" RGB cube; no-op for
    /// other sizes. Translation of the internal `SDL_DitherPalette()`.
    pub fn dither(&mut self) {
        if self.colors.len() != 256 {
            return; // only 8bpp supported right now
        }
        for (i, c) in self.colors.iter_mut().enumerate() {
            /* map each bit field to the full [0, 255] interval,
            so 0 is mapped to (0, 0, 0) and 255 to (255, 255, 255) */
            let mut r = i & 0xe0;
            r |= r >> 3 | r >> 6;
            let mut g = (i << 3) & 0xe0;
            g |= g >> 3 | g >> 6;
            let mut b = i & 0x3;
            b |= b << 2;
            b |= b << 4;
            *c = Color::new(r as u8, g as u8, b as u8, ALPHA_OPAQUE);
        }
        self.bump_version();
    }

    /// Index of the entry nearest to `c` (Euclidean distance in RGBA).
    /// Translation of the internal `SDL_FindColor()`.
    pub fn find_color(&self, c: Color) -> u8 {
        // Do colorspace distance matching
        let mut smallest = u32::MAX;
        let mut pixelvalue = 0u8;
        for (i, p) in self.colors.iter().enumerate() {
            let rd = p.r as i32 - c.r as i32;
            let gd = p.g as i32 - c.g as i32;
            let bd = p.b as i32 - c.b as i32;
            let ad = p.a as i32 - c.a as i32;
            let distance = ((rd * rd) + (gd * gd) + (bd * bd) + (ad * ad)) as u32;
            if distance < smallest {
                pixelvalue = i as u8;
                if distance == 0 {
                    // Perfect match!
                    break;
                }
                smallest = distance;
            }
        }
        pixelvalue
    }

    /// Translation of the internal `SDL_DetectPalette()`.
    pub fn alpha_info(&self) -> PaletteAlpha {
        if self.colors.iter().all(|c| c.a == ALPHA_OPAQUE) {
            // Palette is opaque, with an alpha channel
            PaletteAlpha {
                is_opaque: true,
                has_alpha_channel: true,
            }
        } else if self.colors.iter().all(|c| c.a == ALPHA_TRANSPARENT) {
            // Palette is opaque, without an alpha channel
            PaletteAlpha {
                is_opaque: true,
                has_alpha_channel: false,
            }
        } else {
            // Palette has alpha values
            PaletteAlpha {
                is_opaque: false,
                has_alpha_channel: true,
            }
        }
    }

    /// Whether `dst` begins with exactly this palette's colors (so no
    /// remapping is needed). Translation of the internal `SDL_IsSamePalette()`.
    pub fn is_prefix_of(&self, dst: &Palette) -> bool {
        self.colors.len() <= dst.colors.len()
            && (std::ptr::eq(self, dst) || self.colors[..] == dst.colors[..self.colors.len()])
    }
}

/// Memoized RGBA32 → palette index lookups (translation of the `palette_map`
/// hash table in `SDL_BlitMap`).
#[derive(Clone, Debug, Default)]
pub struct PaletteMap {
    map: HashMap<u32, u8>,
}

impl PaletteMap {
    pub fn new() -> PaletteMap {
        PaletteMap::default()
    }

    /// Nearest palette index for an RGBA32-packed pixel (R in the top byte);
    /// 0 without a palette. Translation of the internal `SDL_LookupRGBAColor()`.
    pub fn lookup(&mut self, rgba32: u32, palette: Option<&Palette>) -> u8 {
        let Some(pal) = palette else { return 0 };
        *self.map.entry(rgba32).or_insert_with(|| {
            pal.find_color(Color::new(
                ((rgba32 >> 24) & 0xFF) as u8,
                ((rgba32 >> 16) & 0xFF) as u8,
                ((rgba32 >> 8) & 0xFF) as u8,
                (rgba32 & 0xFF) as u8,
            ))
        })
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_values_match_header() {
        // Numeric values as listed in SDL_pixels.h
        assert_eq!(PixelFormat::INDEX1LSB.0, 0x11100100);
        assert_eq!(PixelFormat::INDEX1MSB.0, 0x11200100);
        assert_eq!(PixelFormat::INDEX2LSB.0, 0x1c100200);
        assert_eq!(PixelFormat::INDEX4MSB.0, 0x12200400);
        assert_eq!(PixelFormat::INDEX8.0, 0x13000801);
        assert_eq!(PixelFormat::RGB332.0, 0x14110801);
        assert_eq!(PixelFormat::XRGB4444.0, 0x15120c02);
        assert_eq!(PixelFormat::ARGB1555.0, 0x15331002);
        assert_eq!(PixelFormat::RGB565.0, 0x15151002);
        assert_eq!(PixelFormat::RGB24.0, 0x17101803);
        assert_eq!(PixelFormat::XRGB8888.0, 0x16161804);
        assert_eq!(PixelFormat::ARGB8888.0, 0x16362004);
        assert_eq!(PixelFormat::ABGR2101010.0, 0x16772004);
        assert_eq!(PixelFormat::RGBA64.0, 0x18204008);
        assert_eq!(PixelFormat::RGBA64_FLOAT.0, 0x1a204008);
        assert_eq!(PixelFormat::ABGR128_FLOAT.0, 0x1b608010);
        assert_eq!(PixelFormat::YV12.0, 0x32315659);
        assert_eq!(PixelFormat::NV21.0, 0x3132564e);
        assert_eq!(PixelFormat::P010.0, 0x30313050);
        assert_eq!(PixelFormat::MJPG.0, 0x47504a4d);
        assert_eq!(PixelFormat::EXTERNAL_OES.0, 0x2053454f);
        assert_eq!(Colorspace::SRGB.0, 0x120005a0);
        assert_eq!(Colorspace::SRGB_LINEAR.0, 0x12000500);
        assert_eq!(Colorspace::HDR10.0, 0x12002600);
        assert_eq!(Colorspace::JPEG.0, 0x220004c6);
        assert_eq!(Colorspace::BT601_LIMITED.0, 0x211018c6);
        assert_eq!(Colorspace::BT709_FULL.0, 0x22100421);
        assert_eq!(Colorspace::BT2020_LIMITED.0, 0x21102609);
    }

    #[test]
    fn format_predicates() {
        assert!(PixelFormat::INDEX4LSB.is_indexed());
        assert!(PixelFormat::ARGB8888.is_packed());
        assert!(PixelFormat::ARGB8888.has_alpha());
        assert!(!PixelFormat::XRGB8888.has_alpha());
        assert!(PixelFormat::RGBA64.is_array());
        assert!(PixelFormat::RGBA64.has_alpha());
        assert!(PixelFormat::RGBA64_FLOAT.is_float());
        assert!(PixelFormat::ARGB2101010.is_10bit());
        assert!(PixelFormat::YV12.is_fourcc());
        assert!(!PixelFormat::UNKNOWN.is_fourcc());
        assert_eq!(PixelFormat::YV12.bits_per_pixel(), 0);
        assert_eq!(PixelFormat::YV12.bytes_per_pixel(), 1);
        assert_eq!(PixelFormat::YUY2.bytes_per_pixel(), 2);
        assert_eq!(PixelFormat::RGB565.bits_per_pixel(), 16);
        assert_eq!(PixelFormat::RGB24.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::RGB24.pixel_type(), PixelType::ArrayU8);
        assert_eq!(PixelFormat::ARGB8888.pixel_layout(), PackedLayout::L8888);
        assert_eq!(
            Colorspace::BT601_LIMITED.matrix(),
            MatrixCoefficients::Bt601
        );
        assert!(Colorspace::BT601_LIMITED.is_limited_range());
        assert!(Colorspace::JPEG.is_full_range());
        assert_eq!(Colorspace::HDR10.transfer(), TransferCharacteristics::Pq);
        assert_eq!(Colorspace::HDR10.primaries(), ColorPrimaries::Bt2020);
        assert_eq!(Colorspace::BT709_LIMITED.chroma(), ChromaLocation::Left);
        assert_eq!(Colorspace::SRGB.color_type(), ColorType::Rgb);
        assert_eq!(PixelFormat::ARGB8888.name(), "SDL_PIXELFORMAT_ARGB8888");
        assert_eq!(PixelFormat(12345).to_string(), "SDL_PIXELFORMAT_UNKNOWN");
        assert_eq!(
            PixelFormat::RGBA32.name(),
            if cfg!(target_endian = "little") {
                "SDL_PIXELFORMAT_ABGR8888"
            } else {
                "SDL_PIXELFORMAT_RGBA8888"
            }
        );
        assert_eq!(Colorspace::SRGB.to_string(), "Colorspace(0x120005a0)");
    }

    #[test]
    fn expand_tables_match_upstream_literals() {
        assert_eq!(EXPAND_BYTE[0], &[255]);
        assert_eq!(EXPAND_BYTE[1], &[0, 255]);
        assert_eq!(EXPAND_BYTE[2], &[0, 85, 170, 255]);
        assert_eq!(EXPAND_BYTE[3], &[0, 36, 73, 109, 146, 182, 219, 255]);
        assert_eq!(EXPAND_BYTE[4][7], 119);
        assert_eq!(&EXPAND_BYTE[5][..5], &[0, 8, 16, 24, 33]);
        assert_eq!(EXPAND_BYTE[5][31], 255);
        assert_eq!(EXPAND_BYTE[6][16], 65);
        assert_eq!(EXPAND_BYTE[6][63], 255);
        assert_eq!(EXPAND_BYTE[7][64], 129);
        assert_eq!(EXPAND_BYTE[8][200], 200);
        assert_eq!(EXPAND_BYTE_10[0], 0);
        assert_eq!(EXPAND_BYTE_10[1], 4);
        assert_eq!(EXPAND_BYTE_10[42], 168);
        assert_eq!(EXPAND_BYTE_10[43], 173);
        assert_eq!(EXPAND_BYTE_10[127], 509);
        assert_eq!(EXPAND_BYTE_10[128], 514);
        assert_eq!(EXPAND_BYTE_10[212], 850);
        assert_eq!(EXPAND_BYTE_10[213], 855);
        assert_eq!(EXPAND_BYTE_10[255], 1023);
    }

    #[test]
    fn masks_roundtrip() {
        for f in [
            PixelFormat::RGB332,
            PixelFormat::XRGB4444,
            PixelFormat::XBGR4444,
            PixelFormat::XRGB1555,
            PixelFormat::XBGR1555,
            PixelFormat::ARGB4444,
            PixelFormat::RGBA4444,
            PixelFormat::ABGR4444,
            PixelFormat::BGRA4444,
            PixelFormat::ARGB1555,
            PixelFormat::RGBA5551,
            PixelFormat::ABGR1555,
            PixelFormat::BGRA5551,
            PixelFormat::RGB565,
            PixelFormat::BGR565,
            PixelFormat::RGB24,
            PixelFormat::BGR24,
            PixelFormat::XRGB8888,
            PixelFormat::RGBX8888,
            PixelFormat::XBGR8888,
            PixelFormat::BGRX8888,
            PixelFormat::ARGB8888,
            PixelFormat::RGBA8888,
            PixelFormat::ABGR8888,
            PixelFormat::BGRA8888,
            PixelFormat::XRGB2101010,
            PixelFormat::XBGR2101010,
            PixelFormat::ARGB2101010,
            PixelFormat::ABGR2101010,
        ] {
            let m = f.masks().unwrap_or_else(|e| panic!("{f}: {e}"));
            assert_eq!(PixelFormat::from_masks(m), Some(f), "{f} -> {m:?}");
        }
        let m = PixelFormat::ARGB8888.masks().unwrap();
        assert_eq!(
            m,
            PixelMasks {
                bpp: 32,
                r: 0x00FF0000,
                g: 0x0000FF00,
                b: 0x000000FF,
                a: 0xFF000000
            }
        );
        assert_eq!(
            PixelFormat::YUY2.masks().unwrap(),
            PixelMasks {
                bpp: 32,
                ..Default::default()
            }
        );
        assert_eq!(
            PixelFormat::INDEX8.masks().unwrap(),
            PixelMasks {
                bpp: 8,
                ..Default::default()
            }
        );
        let only_bpp = |bpp| PixelMasks {
            bpp,
            ..Default::default()
        };
        assert_eq!(
            PixelFormat::from_masks(only_bpp(8)),
            Some(PixelFormat::INDEX8)
        );
        assert_eq!(
            PixelFormat::from_masks(only_bpp(1)),
            Some(PixelFormat::INDEX1MSB)
        );
        assert_eq!(
            PixelFormat::from_masks(only_bpp(15)),
            Some(PixelFormat::XRGB1555)
        );
        assert_eq!(
            PixelFormat::from_masks(only_bpp(16)),
            Some(PixelFormat::RGB565)
        );
        assert_eq!(
            PixelFormat::from_masks(only_bpp(32)),
            Some(PixelFormat::XRGB8888)
        );
        assert_eq!(
            PixelFormat::from_masks(PixelMasks {
                bpp: 16,
                r: 0x003F,
                g: 0x07C0,
                b: 0xF800,
                a: 0
            }),
            Some(PixelFormat::RGB565)
        );
        assert_eq!(
            PixelFormat::from_masks(PixelMasks {
                bpp: 7,
                r: 1,
                g: 2,
                b: 3,
                a: 4
            }),
            None
        );
        assert!(PixelFormat(0x17901803).masks().is_err()); // ArrayU8 with a bogus order
    }

    #[test]
    fn details_and_mapping() {
        let d = PixelFormat::ARGB8888.details().unwrap();
        assert_eq!((d.bits_per_pixel, d.bytes_per_pixel), (32, 4));
        assert_eq!((d.Rshift, d.Gshift, d.Bshift, d.Ashift), (16, 8, 0, 24));
        assert_eq!((d.Rbits, d.Gbits, d.Bbits, d.Abits), (8, 8, 8, 8));
        assert_eq!(
            d.map_rgba(None, Color::new(0x11, 0x22, 0x33, 0x44)),
            Ok(0x44112233)
        );
        assert_eq!(
            d.map_rgb(None, Color::new(0x11, 0x22, 0x33, 0)),
            Ok(0xFF112233)
        );
        assert_eq!(
            d.get_rgba(0x44112233, None),
            Color::new(0x11, 0x22, 0x33, 0x44)
        );
        assert_eq!(d.get_rgb(0x44112233, None), Color::rgb(0x11, 0x22, 0x33));

        let d565 = PixelFormat::RGB565.details().unwrap();
        assert_eq!(
            (d565.Rbits, d565.Gbits, d565.Bbits, d565.Abits),
            (5, 6, 5, 0)
        );
        assert_eq!(d565.map_rgb(None, Color::WHITE), Ok(0xFFFF));
        assert_eq!(d565.map_rgba(None, Color::new(255, 0, 0, 0)), Ok(0xF800));
        assert_eq!(d565.get_rgba(0xF800, None), Color::new(255, 0, 0, 255));
        assert_eq!(d565.get_rgb(0x0020, None), Color::rgb(0, 4, 0)); // 1 of 6 bits -> 4

        let d10 = PixelFormat::XRGB2101010.details().unwrap();
        assert_eq!(
            d10.map_rgb(None, Color::rgb(255, 128, 0)),
            Ok((1023 << 20) | (514 << 10))
        );
        assert_eq!(
            d10.get_rgb((1023 << 20) | (514 << 10), None),
            Color::rgb(255, 128, 0)
        );
        assert_eq!(
            PixelFormat::XRGB8888
                .details()
                .unwrap()
                .alpha_mask_and_shift_8888(),
            (0xFF000000, 24)
        );
        assert_eq!(d.alpha_mask_and_shift_8888(), (0xFF000000, 24));

        assert_eq!(PixelFormat::ARGB8888.default_colorspace(), Colorspace::SRGB);
        assert_eq!(
            PixelFormat::RGBA64_FLOAT.default_colorspace(),
            Colorspace::SRGB_LINEAR
        );
        assert_eq!(
            PixelFormat::ARGB2101010.default_colorspace(),
            Colorspace::HDR10
        );
        assert_eq!(
            PixelFormat::NV12.default_colorspace(),
            Colorspace::BT601_LIMITED
        );
        assert_eq!(PixelFormat::P010.default_colorspace(), Colorspace::HDR10);
        assert_eq!(PixelFormat::MJPG.default_colorspace(), Colorspace::SRGB);
        assert_eq!(
            FColor::from(Color::new(255, 0, 51, 255)),
            FColor::new(1.0, 0.0, 0.2, 1.0)
        );
    }

    #[test]
    fn palettes() {
        assert!(Palette::new(0).is_err());
        let mut pal = Palette::new(4).unwrap();
        assert_eq!(pal.len(), 4);
        assert!(pal.colors().iter().all(|c| *c == Color::WHITE));
        let colors = [Color::rgb(0, 0, 0), Color::rgb(255, 0, 0)];
        pal.set_colors(1, &colors).unwrap();
        assert_eq!(pal.version(), 2);
        assert_eq!(pal.colors()[1], colors[0]);
        assert_eq!(pal.colors()[2], colors[1]);
        pal.set_colors(3, &colors).unwrap(); // clipped to 1
        assert_eq!(pal.colors()[3], colors[0]);
        assert!(pal.set_colors(5, &colors).is_err());
        assert_eq!(pal.find_color(Color::rgb(250, 0, 0)), 2);
        assert_eq!(pal.find_color(Color::rgb(10, 10, 10)), 1);
        let d8 = PixelFormat::INDEX8.details().unwrap();
        assert_eq!(d8.map_rgb(Some(&pal), Color::rgb(250, 0, 0)), Ok(2));
        assert_eq!(
            d8.map_rgb(None, Color::rgb(250, 0, 0)).unwrap_err().kind(),
            crate::ErrorKind::InvalidParam
        );
        assert_eq!(d8.get_rgba(2, Some(&pal)), Color::rgb(255, 0, 0));
        assert_eq!(d8.get_rgba(9, Some(&pal)), Color::new(0, 0, 0, 0));
        assert_eq!(d8.get_rgb(9, Some(&pal)), Color::rgb(0, 0, 0));
        assert_eq!(
            pal.alpha_info(),
            PaletteAlpha {
                is_opaque: true,
                has_alpha_channel: true
            }
        );
        pal.colors_mut()[0].a = 0;
        assert_eq!(
            pal.alpha_info(),
            PaletteAlpha {
                is_opaque: false,
                has_alpha_channel: true
            }
        );
        for c in pal.colors_mut() {
            c.a = 0;
        }
        assert_eq!(
            pal.alpha_info(),
            PaletteAlpha {
                is_opaque: true,
                has_alpha_channel: false
            }
        );
        let mut map = PaletteMap::new();
        assert_eq!(map.lookup(0xFF000000, Some(&pal)), 2);
        assert_eq!(map.lookup(0xFF000000, Some(&pal)), 2);
        assert_eq!(map.lookup(0xFF000000, None), 0);
        map.clear();
        let pal2 = pal.clone();
        assert!(pal.is_prefix_of(&pal2));
        assert!(!Palette::new(5).unwrap().is_prefix_of(&pal));
        let mut dith = Palette::new(256).unwrap();
        dith.dither();
        assert_eq!(dith.colors()[0], Color::rgb(0, 0, 0));
        assert_eq!(dith.colors()[255], Color::WHITE);
        assert_eq!(dith.colors()[0xE0].r, 255);
        assert_eq!(Palette::from_colors(vec![Color::BLACK]).unwrap().len(), 1);
        assert!(Palette::from_colors(vec![]).is_err());
    }

    #[test]
    fn transfer_functions_and_matrices() {
        assert!((srgb_from_linear(srgb_to_linear(0.5)) - 0.5).abs() < 1e-5);
        assert!((srgb_to_linear(0.04) - 0.04 / 12.92).abs() < 1e-7);
        assert!((pq_from_nits(pq_to_nits(0.5)) - 0.5).abs() < 1e-4);
        assert!((pq_to_nits(1.0) - 10000.0).abs() < 1.0);
        let r_cr = |cs: Colorspace, h, bpp| cs.ycbcr_to_rgb_matrix(h, bpp).map(|m| m.coeff[0][2]);
        assert_eq!(
            Colorspace::BT601_LIMITED
                .ycbcr_to_rgb_matrix(0, 8)
                .map(|m| m.coeff[0][0]),
            Some(1.1644)
        );
        assert_eq!(
            Colorspace::JPEG
                .ycbcr_to_rgb_matrix(0, 8)
                .map(|m| m.coeff[0][0]),
            Some(1.0)
        );
        assert_eq!(r_cr(Colorspace::BT709_LIMITED, 0, 8), Some(1.7927));
        assert_eq!(r_cr(Colorspace::BT2020_FULL, 0, 10), Some(1.4760));
        let unspecified = define_colorspace(
            ColorType::Ycbcr,
            ColorRange::Limited,
            ColorPrimaries::Unspecified,
            TransferCharacteristics::Unspecified,
            MatrixCoefficients::Unspecified,
            ChromaLocation::None,
        );
        assert_eq!(r_cr(unspecified, 480, 8), Some(1.5960));
        assert_eq!(r_cr(unspecified, 1080, 8), Some(1.7927));
        assert_eq!(r_cr(unspecified, 1080, 16), Some(1.6836));
        assert_eq!(r_cr(unspecified, 1080, 12), None);
        assert_eq!(r_cr(Colorspace::SRGB, 0, 8), None);
        assert!(ColorPrimaries::Bt709
            .conversion_matrix_to(ColorPrimaries::Bt2020)
            .is_some());
        assert!(ColorPrimaries::Bt709
            .conversion_matrix_to(ColorPrimaries::Bt709)
            .is_none());
        let [r, g, b] = convert_color_709_to_2020([1.0, 1.0, 1.0]);
        assert!((r - 1.0).abs() < 1e-3 && (g - 1.0).abs() < 1e-3 && (b - 1.0).abs() < 1e-3);
        let m = ColorPrimaries::Bt709
            .conversion_matrix_to(ColorPrimaries::Bt2020)
            .unwrap();
        let [r, g, b] = convert_color_primaries([1.0, 0.0, 0.0], m);
        assert!(
            (r - 0.627404).abs() < 1e-6
                && (g - 0.069097).abs() < 1e-6
                && (b - 0.016391).abs() < 1e-6
        );
    }
}
