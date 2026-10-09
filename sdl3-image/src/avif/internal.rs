// Rust translation of include/avif/internal.h from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! libavif's internal declarations: the check macros, the alpha and
//! reformat parameters, the decode input samples, the codec configuration
//! and the sequence header. As SDL_image builds libavif, none of the
//! experimental features (gain maps, sample transforms, the reduced
//! header, YCgCo-R) are enabled, and libyuv and libsharpyuv are not used.

use super::avif::{AvifChromaSamplePosition, AvifPixelFormat, AvifRange};

// Yes, clamp macros are nasty. Do not use them.
// (AVIF_CLAMP(), AVIF_MIN() and AVIF_MAX() are written out where they are
// used.)

// Used for debugging. Define AVIF_BREAK_ON_ERROR to catch the earliest failure during encoding or decoding.
// (avifBreakOnError() is a no-op in SDL_image's build.)

/// Translation of `AVIF_CHECK()`: used by stream related things.
macro_rules! avif_check {
    ($a:expr) => {
        if !($a) {
            return false;
        }
    };
}
pub(crate) use avif_check;

/// Translation of `AVIF_CHECKERR()`: used instead of CHECK if needing to
/// return a specific error on failure, instead of AVIF_FALSE.
macro_rules! avif_checkerr {
    ($a:expr, $err:expr) => {
        if !($a) {
            return $err;
        }
    };
}
pub(crate) use avif_checkerr;

/// Translation of `AVIF_CHECKRES()`: forward any error to the caller now
/// or continue execution.
macro_rules! avif_checkres {
    ($a:expr) => {
        let result__ = $a;
        if result__ != $crate::avif::avif::AvifResult::Ok {
            return result__;
        }
    };
}
pub(crate) use avif_checkres;

/// Translation of `AVIF_ASSERT_OR_RETURN()`, as built with `NDEBUG` (a
/// release build, as SDL_image's): `AVIF_CHECKERR((A),
/// AVIF_RESULT_INTERNAL_ERROR)`.
macro_rules! avif_assert_or_return {
    ($a:expr) => {
        if !($a) {
            return $crate::avif::avif::AvifResult::InternalError;
        }
    };
}
pub(crate) use avif_assert_or_return;

// ---------------------------------------------------------------------------
// URNs and Content-Types

pub(crate) const AVIF_URN_ALPHA0: &[u8] = b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha";
pub(crate) const AVIF_URN_ALPHA1: &[u8] = b"urn:mpeg:hevc:2015:auxid:1";

pub(crate) const AVIF_CONTENT_TYPE_XMP: &[u8] = b"application/rdf+xml";

// ---------------------------------------------------------------------------
// Alpha

/// Translation of `avifAlphaParams`: the planes are byte slices (from the
/// first byte upstream's pointers address).
pub(crate) struct AvifAlphaParams<'s, 'd> {
    pub(crate) width: u32,
    pub(crate) height: u32,

    pub(crate) src_depth: u32,
    pub(crate) src_plane: AlphaSrc<'s>,
    pub(crate) src_row_bytes: u32,
    pub(crate) src_offset_bytes: u32,
    pub(crate) src_pixel_bytes: u32,

    pub(crate) dst_depth: u32,
    pub(crate) dst_plane: &'d mut [u8],
    pub(crate) dst_row_bytes: u32,
    pub(crate) dst_offset_bytes: u32,
    pub(crate) dst_pixel_bytes: u32,
}

/// The source plane of [`AvifAlphaParams`]: 8-bit samples, or 16-bit ones
/// (upstream's `uint16_t` reads through a byte pointer; the offsets stay in
/// bytes).
#[derive(Clone, Copy)]
pub(crate) enum AlphaSrc<'s> {
    None,
    U8(&'s [u8]),
    U16(&'s [u16]),
}

/// Translation of `avifReformatMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifReformatMode {
    /// Normal YUV conversion using coefficients
    #[default]
    YuvCoefficients = 0,
    /// Pack GBR directly into YUV planes (AVIF_MATRIX_COEFFICIENTS_IDENTITY)
    Identity,
    /// YUV conversion using AVIF_MATRIX_COEFFICIENTS_YCGCO
    Ycgco,
}

/// Translation of `avifAlphaMultiplyMode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AvifAlphaMultiplyMode {
    NoOp = 0,
    Multiply,
    Unmultiply,
}

/// Information about an RGB color space. Translation of
/// `avifRGBColorSpaceInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifRgbColorSpaceInfo {
    /// Number of bytes per channel.
    pub(crate) channel_bytes: u32,
    /// Number of bytes per pixel (= channelBytes * num channels).
    pub(crate) pixel_bytes: u32,
    /// Offset in bytes of the red channel in a pixel.
    pub(crate) offset_bytes_r: u32,
    /// Offset in bytes of the green channel in a pixel.
    pub(crate) offset_bytes_g: u32,
    /// Offset in bytes of the blue channel in a pixel.
    pub(crate) offset_bytes_b: u32,
    /// Offset in bytes of the alpha channel in a pixel.
    pub(crate) offset_bytes_a: u32,

    /// Maximum value for a channel (e.g. 255 for 8 bit).
    pub(crate) max_channel: i32,
    /// Same as maxChannel but as a float.
    pub(crate) max_channel_f: f32,
}

/// Information about a YUV color space. Translation of
/// `avifYUVColorSpaceInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifYuvColorSpaceInfo {
    // YUV coefficients. Y = kr*R + kg*G + kb*B.
    pub(crate) kr: f32,
    pub(crate) kg: f32,
    pub(crate) kb: f32,

    /// Number of bytes per channel.
    pub(crate) channel_bytes: u32,
    /// Bit depth.
    pub(crate) depth: u32,
    /// Full or limited range.
    pub(crate) range: AvifRange,
    /// Maximum value for a channel (e.g. 255 for 8 bit).
    pub(crate) max_channel: i32,
    /// Minimum Y value.
    pub(crate) bias_y: f32,
    /// The value of 0.5 for the appropriate bit depth (128 for 8 bit, 512 for 10 bit, 2048 for 12 bit).
    pub(crate) bias_uv: f32,
    /// Difference between max and min Y.
    pub(crate) range_y: f32,
    /// Difference between max and min UV.
    pub(crate) range_uv: f32,

    /// Chroma subsampling information.
    pub(crate) format_info: super::avif::AvifPixelFormatInfo,
    /// Appropriate RGB<->YUV conversion mode.
    pub(crate) mode: AvifReformatMode,
}

/// Translation of `avifReformatState`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifReformatState {
    pub(crate) rgb: AvifRgbColorSpaceInfo,
    pub(crate) yuv: AvifYuvColorSpaceInfo,
}

// ---------------------------------------------------------------------------
// AVIF item category

/// Translation of `avifItemCategory`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifItemCategory {
    #[default]
    Color = 0,
    Alpha = 1,
}

/// `AVIF_ITEM_CATEGORY_COUNT`
pub(crate) const AVIF_ITEM_CATEGORY_COUNT: usize = 2;

impl AvifItemCategory {
    /// The category of index `c` (`(avifItemCategory)c`).
    pub(crate) fn from_index(c: usize) -> AvifItemCategory {
        if c == 0 {
            AvifItemCategory::Color
        } else {
            AvifItemCategory::Alpha
        }
    }
}

/// Translation of `avifIsAlpha()` (avif.c).
pub(crate) fn avif_is_alpha(item_category: AvifItemCategory) -> bool {
    if item_category == AvifItemCategory::Alpha {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// avifCodecDecodeInput

/// Legal spatial_id values are [0,1,2,3], so this serves as a sentinel value for "do not filter by spatial_id"
pub(crate) const AVIF_SPATIAL_ID_UNSET: u8 = 0xff;

/// The bytes of a sample (`avifDecodeSample.data`): not read yet (`NULL`),
/// a copy the sample owns (`ownsData`), or a range of the merged extents of
/// the item the sample comes from (upstream's pointer into
/// `avifDecoderItem.mergedExtents`).
#[derive(Clone, Debug, Default)]
pub(crate) enum SampleData {
    #[default]
    None,
    Owned(Vec<u8>),
    Item {
        offset: usize,
        size: usize,
    },
}

impl SampleData {
    /// `data.size`
    pub(crate) fn size(&self) -> usize {
        match self {
            SampleData::None => 0,
            SampleData::Owned(v) => v.len(),
            SampleData::Item { size, .. } => *size,
        }
    }
}

/// Translation of `avifDecodeSample`.
#[derive(Clone, Debug, Default)]
pub(crate) struct AvifDecodeSample {
    pub(crate) data: SampleData,
    /// if true, data exists but doesn't have all of the sample in it
    pub(crate) partial_data: bool,

    /// if non-zero, data comes from a mergedExtents buffer in an avifDecoderItem, not a file offset
    pub(crate) item_id: u32,
    /// additional offset into data. Can be used to offset into an itemID's payload as well.
    pub(crate) offset: u64,
    pub(crate) size: usize,
    /// If set to a value other than AVIF_SPATIAL_ID_UNSET, output frames from this sample should be
    /// skipped until the output frame's spatial_id matches this ID.
    pub(crate) spatial_id: u8,
    /// is sync sample (keyframe)
    pub(crate) sync: bool,
}

/// Translation of `avifCodecDecodeInput` (`avifCodecDecodeInputCreate()`
/// is `Default`, `avifCodecDecodeInputDestroy()` dropping it).
#[derive(Clone, Debug, Default)]
pub(crate) struct AvifCodecDecodeInput {
    pub(crate) samples: Vec<AvifDecodeSample>,
    /// if true, the underlying codec must decode all layers, not just the best layer
    pub(crate) all_layers: bool,
    /// category of item being decoded
    pub(crate) item_category: AvifItemCategory,
}

// ---------------------------------------------------------------------------
// avifCodecType (underlying video format)

/// Alliance for Open Media video formats that can be used in the AVIF
/// image format. Translation of `avifCodecType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifCodecType {
    #[default]
    Unknown,
    Av1,
}

// ---------------------------------------------------------------------------
// avifStream

/// Translation of `avifBoxHeader`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifBoxHeader {
    /// If set to AVIF_TRUE, it means that the box goes on until the end of the
    /// stream. So, |size| must be set to the number of bytes left in the input
    /// stream. If set to AVIF_FALSE, |size| indicates the size of the box in
    /// bytes, excluding the box header.
    pub(crate) is_size_zero_box: bool,
    /// Size of the box in bytes, excluding the box header.
    pub(crate) size: usize,

    pub(crate) type_: [u8; 4],
}

/// Used for both av1C and av2C. Translation of
/// `avifCodecConfigurationBox`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AvifCodecConfigurationBox {
    // [skipped; is constant] unsigned int (1)marker = 1;
    // [skipped; is constant] unsigned int (7)version = 1;
    /// unsigned int (3) seq_profile;
    pub(crate) seq_profile: u8,
    /// unsigned int (5) seq_level_idx_0;
    pub(crate) seq_level_idx0: u8,
    /// unsigned int (1) seq_tier_0;
    pub(crate) seq_tier0: u8,
    /// unsigned int (1) high_bitdepth;
    pub(crate) high_bitdepth: u8,
    /// unsigned int (1) twelve_bit;
    pub(crate) twelve_bit: u8,
    /// unsigned int (1) monochrome;
    pub(crate) monochrome: u8,
    /// unsigned int (1) chroma_subsampling_x;
    pub(crate) chroma_subsampling_x: u8,
    /// unsigned int (1) chroma_subsampling_y;
    pub(crate) chroma_subsampling_y: u8,
    /// unsigned int (2) chroma_sample_position;
    pub(crate) chroma_sample_position: u8,
    // unsigned int (3)reserved = 0;
    // unsigned int (1)initial_presentation_delay_present;
    // if (initial_presentation_delay_present) {
    //     unsigned int (4)initial_presentation_delay_minus_one;
    // } else {
    //     unsigned int (4)reserved = 0;
    // }
}

/// Translation of `avifSequenceHeader`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifSequenceHeader {
    pub(crate) reduced_still_picture_header: u8,
    pub(crate) max_width: u32,
    pub(crate) max_height: u32,
    pub(crate) bit_depth: u32,
    pub(crate) yuv_format: AvifPixelFormat,
    pub(crate) chroma_sample_position: AvifChromaSamplePosition,
    pub(crate) color_primaries: u16,
    pub(crate) transfer_characteristics: u16,
    pub(crate) matrix_coefficients: u16,
    pub(crate) range: AvifRange,
    /// TODO(yguyon): Rename or add av2C
    pub(crate) av1c: AvifCodecConfigurationBox,
}

// ---------------------------------------------------------------------------
// gain maps
// (not built in SDL_image's configuration)

pub(crate) const AVIF_INDEFINITE_DURATION64: u64 = u64::MAX;
pub(crate) const AVIF_INDEFINITE_DURATION32: u32 = u32::MAX;
