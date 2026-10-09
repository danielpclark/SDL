// Rust translation of src/avif.c and include/avif/avif.h from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! libavif's public types (results, pixel formats, CICP values, images
//! and RGB images, the decoder's settings) and the image helpers of
//! avif.c.
//!
//! An image's planes are [`AvifPlane`]s: buffers the image owns (8-bit
//! samples, or 16-bit ones for depths above 8) or the planes of a dav1d
//! picture the codec decoded (upstream's pointers into the decoder's
//! buffers, which are shared `Arc`s here), each with a byte offset into
//! the buffer (for views, `avifImageSetViewRect()`). Row strides stay in
//! bytes, as upstream's `yuvRowBytes` and `alphaRowBytes`. The encoder API
//! (`avifEncoder*()`, `avifImageRGBToYUV()`) is not translated: no AV1
//! encoder is.

use std::sync::Arc;

use crate::dav1d::PictureData;

use super::internal::{avif_checkerr, avif_checkres};
use super::rawdata::avif_rw_data_set;

// ---------------------------------------------------------------------------
// Constants

pub(crate) const AVIF_VERSION_MAJOR: u32 = 1;
pub(crate) const AVIF_VERSION_MINOR: u32 = 1;
pub(crate) const AVIF_VERSION_PATCH: u32 = 1;
pub(crate) const AVIF_VERSION_DEVEL: u32 = 0;
pub(crate) const AVIF_VERSION: u32 = (AVIF_VERSION_MAJOR * 1000000)
    + (AVIF_VERSION_MINOR * 10000)
    + (AVIF_VERSION_PATCH * 100)
    + AVIF_VERSION_DEVEL;

pub(crate) const AVIF_DIAGNOSTICS_ERROR_BUFFER_SIZE: usize = 256;

/// A reasonable default for maximum image size (in pixel count) to avoid out-of-memory errors or
/// integer overflow in (32-bit) int or unsigned int arithmetic operations.
pub(crate) const AVIF_DEFAULT_IMAGE_SIZE_LIMIT: u32 = 16384 * 16384;

/// A reasonable default for maximum image dimension (width or height).
pub(crate) const AVIF_DEFAULT_IMAGE_DIMENSION_LIMIT: u32 = 32768;

/// a 12 hour AVIF image sequence, running at 60 fps (a basic sanity check as this is quite ridiculous)
pub(crate) const AVIF_DEFAULT_IMAGE_COUNT_LIMIT: u32 = 12 * 3600 * 60;

pub(crate) const AVIF_PLANE_COUNT_YUV: usize = 3;

/// This value is used to indicate that an animated AVIF file has to be repeated infinitely.
pub(crate) const AVIF_REPETITION_COUNT_INFINITE: i32 = -1;
/// This value is used if an animated AVIF file does not have repetitions specified using an EditList box. Applications can choose
/// to handle this case however they want.
pub(crate) const AVIF_REPETITION_COUNT_UNKNOWN: i32 = -2;

/// The number of spatial layers in AV1, with spatial_id = 0..3.
pub(crate) const AVIF_MAX_AV1_LAYER_COUNT: u32 = 4;

// avifPlanesFlag
pub(crate) const AVIF_PLANES_YUV: u32 = 1 << 0;
pub(crate) const AVIF_PLANES_A: u32 = 1 << 1;
pub(crate) const AVIF_PLANES_ALL: u32 = 0xff;
/// Translation of `avifPlanesFlags`.
pub(crate) type AvifPlanesFlags = u32;

// avifChannelIndex
// These can be used as the index for the yuvPlanes and yuvRowBytes arrays in avifImage.
pub(crate) const AVIF_CHAN_Y: usize = 0;
pub(crate) const AVIF_CHAN_U: usize = 1;
pub(crate) const AVIF_CHAN_V: usize = 2;
// This may not be used in yuvPlanes and yuvRowBytes, but is available for use with avifImagePlane().
pub(crate) const AVIF_CHAN_A: usize = 3;

// ---------------------------------------------------------------------------
// avifResult

/// Translation of `avifResult`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AvifResult {
    Ok = 0,
    UnknownError = 1,
    InvalidFtyp = 2,
    NoContent = 3,
    NoYuvFormatSelected = 4,
    ReformatFailed = 5,
    UnsupportedDepth = 6,
    EncodeColorFailed = 7,
    EncodeAlphaFailed = 8,
    BmffParseFailed = 9,
    MissingImageItem = 10,
    DecodeColorFailed = 11,
    DecodeAlphaFailed = 12,
    ColorAlphaSizeMismatch = 13,
    IspeSizeMismatch = 14,
    NoCodecAvailable = 15,
    NoImagesRemaining = 16,
    InvalidExifPayload = 17,
    InvalidImageGrid = 18,
    InvalidCodecSpecificOption = 19,
    TruncatedData = 20,
    /// the avifIO field of avifDecoder is not set
    IoNotSet = 21,
    IoError = 22,
    /// similar to EAGAIN/EWOULDBLOCK, this means the avifIO doesn't have necessary data available yet
    WaitingOnIo = 23,
    /// an argument passed into this function is invalid
    InvalidArgument = 24,
    /// a requested code path is not (yet) implemented
    NotImplemented = 25,
    OutOfMemory = 26,
    /// a setting that can't change is changed during encoding
    CannotChangeSetting = 27,
    /// the image is incompatible with already encoded images
    IncompatibleImage = 28,
    /// some invariants have not been satisfied (likely a bug in libavif)
    InternalError = 29,
}

// ---------------------------------------------------------------------------
// avifROData/avifRWData: Generic raw memory storage
// (slices and vectors here: avifRWDataFree() is dropping or clearing one)

// ---------------------------------------------------------------------------
// avifPixelFormat
//
// Note to libavif maintainers: The lookup tables in avifImageYUVToRGBLibYUV
// rely on the ordering of this enum values for their correctness. So changing
// the values in this enum will require auditing avifImageYUVToRGBLibYUV for
// correctness.

/// Translation of `avifPixelFormat`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifPixelFormat {
    /// No YUV pixels are present. Alpha plane can still be present.
    #[default]
    None = 0,

    Yuv444,
    Yuv422,
    Yuv420,
    Yuv400,
}

/// `AVIF_PIXEL_FORMAT_COUNT`
pub(crate) const AVIF_PIXEL_FORMAT_COUNT: u32 = 5;

/// Translation of `avifPixelFormatInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifPixelFormatInfo {
    pub(crate) monochrome: bool,
    pub(crate) chroma_shift_x: i32,
    pub(crate) chroma_shift_y: i32,
}

// ---------------------------------------------------------------------------
// avifChromaSamplePosition

/// Translation of `avifChromaSamplePosition` (a value of the bitstream,
/// kept as it was read).
pub(crate) type AvifChromaSamplePosition = u32;
pub(crate) const AVIF_CHROMA_SAMPLE_POSITION_UNKNOWN: AvifChromaSamplePosition = 0;
pub(crate) const AVIF_CHROMA_SAMPLE_POSITION_VERTICAL: AvifChromaSamplePosition = 1;
pub(crate) const AVIF_CHROMA_SAMPLE_POSITION_COLOCATED: AvifChromaSamplePosition = 2;
pub(crate) const AVIF_CHROMA_SAMPLE_POSITION_RESERVED: AvifChromaSamplePosition = 3;

// ---------------------------------------------------------------------------
// avifRange

/// Translation of `avifRange`: only applicable to YUV planes. RGB and alpha
/// planes are always full range.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifRange {
    /// Y  [16..235],  UV  [16..240]  (bit depth 8)
    /// Y  [64..940],  UV  [64..960]  (bit depth 10)
    /// Y [256..3760], UV [256..3840] (bit depth 12)
    #[default]
    Limited = 0,
    /// [0..255]  (bit depth 8)
    /// [0..1023] (bit depth 10)
    /// [0..4095] (bit depth 12)
    Full = 1,
}

// ---------------------------------------------------------------------------
// CICP enums - https://www.itu.int/rec/T-REC-H.273-201612-S/en

// This is actually reserved, but libavif uses it as a sentinel value.
pub(crate) const AVIF_COLOR_PRIMARIES_UNKNOWN: u16 = 0;

pub(crate) const AVIF_COLOR_PRIMARIES_BT709: u16 = 1;
pub(crate) const AVIF_COLOR_PRIMARIES_SRGB: u16 = 1;
pub(crate) const AVIF_COLOR_PRIMARIES_IEC61966_2_4: u16 = 1;
pub(crate) const AVIF_COLOR_PRIMARIES_UNSPECIFIED: u16 = 2;
pub(crate) const AVIF_COLOR_PRIMARIES_BT470M: u16 = 4;
pub(crate) const AVIF_COLOR_PRIMARIES_BT470BG: u16 = 5;
pub(crate) const AVIF_COLOR_PRIMARIES_BT601: u16 = 6;
pub(crate) const AVIF_COLOR_PRIMARIES_SMPTE240: u16 = 7;
pub(crate) const AVIF_COLOR_PRIMARIES_GENERIC_FILM: u16 = 8;
pub(crate) const AVIF_COLOR_PRIMARIES_BT2020: u16 = 9;
pub(crate) const AVIF_COLOR_PRIMARIES_BT2100: u16 = 9;
pub(crate) const AVIF_COLOR_PRIMARIES_XYZ: u16 = 10;
pub(crate) const AVIF_COLOR_PRIMARIES_SMPTE431: u16 = 11;
pub(crate) const AVIF_COLOR_PRIMARIES_SMPTE432: u16 = 12;
pub(crate) const AVIF_COLOR_PRIMARIES_DCI_P3: u16 = 12;
pub(crate) const AVIF_COLOR_PRIMARIES_EBU3213: u16 = 22;
/// Translation of `avifColorPrimaries` (AVIF_COLOR_PRIMARIES_*).
pub(crate) type AvifColorPrimaries = u16;

// This is actually reserved, but libavif uses it as a sentinel value.
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_UNKNOWN: u16 = 0;

pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_BT709: u16 = 1;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_UNSPECIFIED: u16 = 2;
/// 2.2 gamma
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_BT470M: u16 = 4;
/// 2.8 gamma
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_BT470BG: u16 = 5;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_BT601: u16 = 6;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_SMPTE240: u16 = 7;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_LINEAR: u16 = 8;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_LOG100: u16 = 9;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_LOG100_SQRT10: u16 = 10;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_IEC61966: u16 = 11;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_BT1361: u16 = 12;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_SRGB: u16 = 13;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_BT2020_10BIT: u16 = 14;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_BT2020_12BIT: u16 = 15;
/// Perceptual Quantizer (HDR); BT.2100 PQ
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_PQ: u16 = 16;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_SMPTE2084: u16 = 16;
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_SMPTE428: u16 = 17;
/// Hybrid Log-Gamma (HDR); ARIB STD-B67; BT.2100 HLG
pub(crate) const AVIF_TRANSFER_CHARACTERISTICS_HLG: u16 = 18;
/// Translation of `avifTransferCharacteristics` (AVIF_TRANSFER_CHARACTERISTICS_*).
pub(crate) type AvifTransferCharacteristics = u16;

pub(crate) const AVIF_MATRIX_COEFFICIENTS_IDENTITY: u16 = 0;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_BT709: u16 = 1;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_UNSPECIFIED: u16 = 2;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_FCC: u16 = 4;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_BT470BG: u16 = 5;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_BT601: u16 = 6;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_SMPTE240: u16 = 7;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_YCGCO: u16 = 8;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_BT2020_NCL: u16 = 9;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_BT2020_CL: u16 = 10;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_SMPTE2085: u16 = 11;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_CHROMA_DERIVED_NCL: u16 = 12;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_CHROMA_DERIVED_CL: u16 = 13;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_ICTCP: u16 = 14;
pub(crate) const AVIF_MATRIX_COEFFICIENTS_LAST: u16 = 15;
/// Translation of `avifMatrixCoefficients` (AVIF_MATRIX_COEFFICIENTS_*).
pub(crate) type AvifMatrixCoefficients = u16;

// ---------------------------------------------------------------------------
// avifDiagnostics
// (diag.rs)

// ---------------------------------------------------------------------------
// Fraction utility

/// Translation of `avifFraction`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifFraction {
    pub(crate) n: i32,
    pub(crate) d: i32,
}

// ---------------------------------------------------------------------------
// Optional transformation structs

// avifTransformFlag
pub(crate) const AVIF_TRANSFORM_NONE: u32 = 0;
pub(crate) const AVIF_TRANSFORM_PASP: u32 = 1 << 0;
pub(crate) const AVIF_TRANSFORM_CLAP: u32 = 1 << 1;
pub(crate) const AVIF_TRANSFORM_IROT: u32 = 1 << 2;
pub(crate) const AVIF_TRANSFORM_IMIR: u32 = 1 << 3;
/// Translation of `avifTransformFlags`.
pub(crate) type AvifTransformFlags = u32;

/// Translation of `avifPixelAspectRatioBox`: 'pasp' from ISO/IEC
/// 14496-12:2015 12.1.4.3.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifPixelAspectRatioBox {
    // define the relative width and height of a pixel
    pub(crate) h_spacing: u32,
    pub(crate) v_spacing: u32,
}

/// Translation of `avifCleanApertureBox`: 'clap' from ISO/IEC
/// 14496-12:2015 12.1.4.3.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifCleanApertureBox {
    // a fractional number which defines the exact clean aperture width, in counted pixels, of the video image
    pub(crate) width_n: u32,
    pub(crate) width_d: u32,

    // a fractional number which defines the exact clean aperture height, in counted pixels, of the video image
    pub(crate) height_n: u32,
    pub(crate) height_d: u32,

    // a fractional number which defines the horizontal offset of clean aperture centre minus (width-1)/2. Typically 0.
    pub(crate) horiz_off_n: u32,
    pub(crate) horiz_off_d: u32,

    // a fractional number which defines the vertical offset of clean aperture centre minus (height-1)/2. Typically 0.
    pub(crate) vert_off_n: u32,
    pub(crate) vert_off_d: u32,
}

/// Translation of `avifImageRotation`: 'irot' from ISO/IEC 23008-12:2017
/// 6.5.10.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifImageRotation {
    /// angle * 90 specifies the angle (in anti-clockwise direction) in units of degrees.
    /// legal values: [0-3]
    pub(crate) angle: u8,
}

/// Translation of `avifImageMirror`: 'imir' from ISO/IEC 23008-12:2022
/// 6.5.12.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifImageMirror {
    //     'axis' specifies how the mirroring is performed:
    //
    //     0 indicates that the top and bottom parts of the image are exchanged;
    //     1 specifies that the left and right parts are exchanged.
    //
    //     NOTE In Exif, orientation tag can be used to signal mirroring operations. Exif
    //     orientation tag 4 corresponds to axis = 0 of ImageMirror, and Exif orientation tag 2
    //     corresponds to axis = 1 accordingly.
    //
    // Legal values: [0, 1]
    pub(crate) axis: u8,
}

// ---------------------------------------------------------------------------
// avifCropRect - Helper struct/functions to work with avifCleanApertureBox

/// Translation of `avifCropRect`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifCropRect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

// ---------------------------------------------------------------------------
// avifContentLightLevelInformationBox

/// Translation of `avifContentLightLevelInformationBox`: 'clli' from
/// ISO/IEC 23000-22:2019 (MIAF) 7.4.4.2.2.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifContentLightLevelInformationBox {
    /// max_content_light_level, when not equal to 0, indicates an upper bound on the maximum light
    /// level among all individual samples in a 4:4:4 representation of red, green, and blue colour
    /// primary intensities (in the linear light domain) for the pictures of the CLVS, in units of
    /// candelas per square metre. When equal to 0, no such upper bound is indicated by
    /// max_content_light_level.
    pub(crate) max_cll: u16,

    /// max_pic_average_light_level, when not equal to 0, indicates an upper bound on the maximum
    /// average light level among the samples in a 4:4:4 representation of red, green, and blue
    /// colour primary intensities (in the linear light domain) for any individual picture of the
    /// CLVS, in units of candelas per square metre. When equal to 0, no such upper bound is
    /// indicated by max_pic_average_light_level.
    pub(crate) max_pall: u16,
}

// ---------------------------------------------------------------------------
// avifImage

/// The buffer of a plane: owned 8-bit or 16-bit samples, or a plane of a
/// decoded dav1d picture (`PictureData`, plane index).
#[derive(Clone, Debug)]
pub(crate) enum AvifPlaneData {
    U8(Arc<Vec<u8>>),
    U16(Arc<Vec<u16>>),
    Picture(Arc<PictureData>, usize),
}

/// A plane of an image: upstream's `uint8_t *` (a buffer and a byte offset
/// into it).
#[derive(Clone, Debug)]
pub(crate) struct AvifPlane {
    pub(crate) data: AvifPlaneData,
    /// Bytes from the start of the buffer.
    pub(crate) offset: usize,
}

impl AvifPlane {
    /// An owned plane of `size` bytes (`avifAlloc(size)`), of 16-bit samples
    /// if `u16_samples`; `None` when it can't be allocated.
    pub(crate) fn alloc(size: usize, u16_samples: bool) -> Option<AvifPlane> {
        let data = if u16_samples {
            let mut v = Vec::new();
            v.try_reserve_exact(size / 2).ok()?;
            v.resize(size / 2, 0u16);
            AvifPlaneData::U16(Arc::new(v))
        } else {
            let mut v = Vec::new();
            v.try_reserve_exact(size).ok()?;
            v.resize(size, 0u8);
            AvifPlaneData::U8(Arc::new(v))
        };
        Some(AvifPlane { data, offset: 0 })
    }

    /// The 8-bit samples from the plane's first byte.
    pub(crate) fn u8s(&self) -> &[u8] {
        let all: &[u8] = match &self.data {
            AvifPlaneData::U8(v) => v,
            AvifPlaneData::Picture(p, i) => p.plane_u8(*i).unwrap_or(&[]),
            AvifPlaneData::U16(_) => &[],
        };
        all.get(self.offset..).unwrap_or(&[])
    }

    /// The 16-bit samples from the plane's first byte.
    pub(crate) fn u16s(&self) -> &[u16] {
        let all: &[u16] = match &self.data {
            AvifPlaneData::U16(v) => v,
            AvifPlaneData::Picture(p, i) => p.plane_u16(*i).unwrap_or(&[]),
            AvifPlaneData::U8(_) => &[],
        };
        all.get(self.offset / 2..).unwrap_or(&[])
    }

    /// The 8-bit samples of an owned plane, writable.
    pub(crate) fn u8s_mut(&mut self) -> &mut [u8] {
        match &mut self.data {
            AvifPlaneData::U8(v) => match Arc::get_mut(v) {
                Some(v) => v.get_mut(self.offset..).unwrap_or(&mut []),
                None => &mut [],
            },
            _ => &mut [],
        }
    }

    /// The 16-bit samples of an owned plane, writable.
    pub(crate) fn u16s_mut(&mut self) -> &mut [u16] {
        match &mut self.data {
            AvifPlaneData::U16(v) => match Arc::get_mut(v) {
                Some(v) => v.get_mut(self.offset / 2..).unwrap_or(&mut []),
                None => &mut [],
            },
            _ => &mut [],
        }
    }

    /// The same buffer, `bytes` further.
    pub(crate) fn offset_by(&self, bytes: usize) -> AvifPlane {
        AvifPlane {
            data: self.data.clone(),
            offset: self.offset + bytes,
        }
    }
}

/// Translation of `avifImage`. `avifImageCreate()` and
/// `avifImageCreateEmpty()` are [`avif_image_create`] and `Default`,
/// `avifImageDestroy()` dropping it.
#[derive(Clone, Debug, Default)]
pub(crate) struct AvifImage {
    // Image information
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// all planes must share this depth; if depth>8, all planes are uint16_t internally
    pub(crate) depth: u32,

    pub(crate) yuv_format: AvifPixelFormat,
    pub(crate) yuv_range: AvifRange,
    pub(crate) yuv_chroma_sample_position: AvifChromaSamplePosition,
    pub(crate) yuv_planes: [Option<AvifPlane>; AVIF_PLANE_COUNT_YUV],
    pub(crate) yuv_row_bytes: [u32; AVIF_PLANE_COUNT_YUV],
    pub(crate) image_owns_yuv_planes: bool,

    pub(crate) alpha_plane: Option<AvifPlane>,
    pub(crate) alpha_row_bytes: u32,
    pub(crate) image_owns_alpha_plane: bool,
    pub(crate) alpha_premultiplied: bool,

    /// ICC Profile
    pub(crate) icc: Vec<u8>,

    // CICP information:
    // These are stored in the AV1 payload and used to signal YUV conversion. Additionally, if an
    // ICC profile is not specified, these will be stored in the AVIF container's `colr` box with
    // a type of `nclx`. If your system supports ICC profiles, be sure to check for the existence
    // of one (avifImage.icc) before relying on the values listed here!
    pub(crate) color_primaries: AvifColorPrimaries,
    pub(crate) transfer_characteristics: AvifTransferCharacteristics,
    pub(crate) matrix_coefficients: AvifMatrixCoefficients,

    // CLLI information:
    // Content Light Level Information. Used to represent maximum and average light level of an
    // image. Useful for tone mapping HDR images, especially when using transfer characteristics
    // SMPTE2084 (PQ). The default value of (0, 0) means the content light level information is
    // unknown or unavailable, and will cause libavif to avoid writing a clli box for it.
    pub(crate) clli: AvifContentLightLevelInformationBox,

    // Transformations - These metadata values are encoded/decoded when transformFlags are set
    // appropriately, but do not impact/adjust the actual pixel buffers used (images won't be
    // pre-cropped or mirrored upon decode). Basic explanations from the standards are offered in
    // comments above, but for detailed explanations, please refer to the HEIF standard (ISO/IEC
    // 23008-12:2017) and the BMFF standard (ISO/IEC 14496-12:2015).
    //
    // To encode any of these boxes, set the values in the associated box, then enable the flag in
    // transformFlags. On decode, only honor the values in boxes with the associated transform flag set.
    // These also apply to gainMap->image, if any.
    pub(crate) transform_flags: AvifTransformFlags,
    pub(crate) pasp: AvifPixelAspectRatioBox,
    pub(crate) clap: AvifCleanApertureBox,
    pub(crate) irot: AvifImageRotation,
    pub(crate) imir: AvifImageMirror,

    // Metadata - set with avifImageSetMetadata*() before write, check .size>0 for existence after read
    /// exif_payload chunk from the ExifDataBlock specified in ISO/IEC 23008-12:2022 Section A.2.1.
    /// The value of the 4-byte exif_tiff_header_offset field, which is not part of this avifRWData
    /// byte sequence, can be retrieved by calling avifGetExifTiffHeaderOffset(avifImage.exif).
    pub(crate) exif: Vec<u8>,
    pub(crate) xmp: Vec<u8>,
    // Version 1.0.0 ends here. Add any new members after this line.
}

// ---------------------------------------------------------------------------
// Optional YUV<->RGB support

/// Translation of `avifRGBFormat`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifRgbFormat {
    Rgb = 0,
    /// This is the default format set in avifRGBImageSetDefaults().
    #[default]
    Rgba,
    Argb,
    Bgr,
    Bgra,
    Abgr,
    // RGB_565 format uses five bits for the red and blue components and six
    // bits for the green component. Each RGB pixel is 16 bits (2 bytes), which
    // is packed as follows:
    //   uint16_t: [r4 r3 r2 r1 r0 g5 g4 g3 g2 g1 g0 b4 b3 b2 b1 b0]
    //   r4 and r0 are the MSB and LSB of the red component respectively.
    //   g5 and g0 are the MSB and LSB of the green component respectively.
    //   b4 and b0 are the MSB and LSB of the blue component respectively.
    // This format is only supported for YUV -> RGB conversion and when
    // avifRGBImage.depth is set to 8.
    Rgb565,
}

/// Translation of `avifChromaUpsampling`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifChromaUpsampling {
    /// Chooses best trade off of speed/quality (uses BILINEAR libyuv if available,
    /// or falls back to NEAREST libyuv if available, or falls back to BILINEAR built-in)
    #[default]
    Automatic = 0,
    /// Chooses speed over quality (same as NEAREST)
    Fastest = 1,
    /// Chooses the best quality upsampling, given settings (same as BILINEAR)
    BestQuality = 2,
    /// Uses nearest-neighbor filter
    Nearest = 3,
    /// Uses bilinear filter
    Bilinear = 4,
}

/// Translation of `avifChromaDownsampling`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifChromaDownsampling {
    /// Chooses best trade off of speed/quality (same as AVERAGE)
    #[default]
    Automatic = 0,
    /// Chooses speed over quality (same as AVERAGE)
    Fastest = 1,
    /// Chooses the best quality upsampling (same as AVERAGE)
    BestQuality = 2,
    /// Uses averaging filter
    Average = 3,
    /// Uses sharp yuv filter (libsharpyuv), available for 4:2:0 only, ignored for 4:2:2
    SharpYuv = 4,
}

/// Translation of `avifRGBImage`. It must be initialized with
/// avifRGBImageSetDefaults() (preferred) or `Default` (upstream's memset())
/// before use. The pixels are a byte slice (`pixels`); samples above 8
/// bits are native-endian `u16`s in it, as upstream's.
#[derive(Debug, Default)]
pub(crate) struct AvifRgbImage<'p> {
    /// must match associated avifImage
    pub(crate) width: u32,
    /// must match associated avifImage
    pub(crate) height: u32,
    /// legal depths [8, 10, 12, 16]. if depth>8, pixels must be uint16_t internally
    pub(crate) depth: u32,
    /// all channels are always full range
    pub(crate) format: AvifRgbFormat,
    /// How to upsample from 4:2:0 or 4:2:2 UV when converting to RGB (ignored for 4:4:4 and 4:0:0).
    /// Ignored when converting to YUV. Defaults to AVIF_CHROMA_UPSAMPLING_AUTOMATIC.
    pub(crate) chroma_upsampling: AvifChromaUpsampling,
    /// How to downsample to 4:2:0 or 4:2:2 UV when converting from RGB (ignored for 4:4:4 and 4:0:0).
    /// Ignored when converting to RGB. Defaults to AVIF_CHROMA_DOWNSAMPLING_AUTOMATIC.
    pub(crate) chroma_downsampling: AvifChromaDownsampling,
    /// If AVIF_FALSE and libyuv conversion between RGB and YUV (including upsampling or downsampling if any)
    /// is available for the avifImage/avifRGBImage combination, then libyuv is used. Default is AVIF_FALSE.
    pub(crate) avoid_lib_yuv: bool,
    /// Used for XRGB formats, treats formats containing alpha (such as ARGB) as if they were RGB, treating
    /// the alpha bits as if they were all 1.
    pub(crate) ignore_alpha: bool,
    /// indicates if RGB value is pre-multiplied by alpha. Default: false
    pub(crate) alpha_premultiplied: bool,
    /// indicates if RGBA values are in half float (f16) format. Valid only when depth == 16. Default: false
    pub(crate) is_float: bool,
    /// Number of threads to be used for the YUV to RGB conversion. Note that this value is ignored for RGB to YUV
    /// conversion. Setting this to zero has the same effect as setting it to one. Negative values are invalid.
    /// Default: 1.
    pub(crate) max_threads: i32,

    pub(crate) pixels: Option<&'p mut [u8]>,
    pub(crate) row_bytes: u32,
}

// ---------------------------------------------------------------------------
// Codec selection

/// Translation of `avifCodecChoice`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifCodecChoice {
    #[default]
    Auto = 0,
    Aom,
    /// Decode only
    Dav1d,
    /// Decode only
    Libgav1,
    /// Encode only
    Rav1e,
    /// Encode only
    Svt,
    /// Experimental (AV2)
    Avm,
}

// avifCodecFlag
pub(crate) const AVIF_CODEC_FLAG_CAN_DECODE: u32 = 1 << 0;
pub(crate) const AVIF_CODEC_FLAG_CAN_ENCODE: u32 = 1 << 1;
/// Translation of `avifCodecFlags`.
pub(crate) type AvifCodecFlags = u32;

// ---------------------------------------------------------------------------
// avifDecoder

// Some encoders (including very old versions of avifenc) do not implement the AVIF standard
// perfectly, and thus create invalid files. However, these files are likely still recoverable /
// decodable, if it wasn't for the strict requirements imposed by libavif's decoder. These flags
// allow a user of avifDecoder to decide what level of strictness they want in their project.
// avifStrictFlag
/// Disables all strict checks.
pub(crate) const AVIF_STRICT_DISABLED: u32 = 0;
/// Requires the PixelInformationProperty ('pixi') be present in AV1 image items. libheif v1.11.0
/// or older does not add the 'pixi' item property to AV1 image items. If you need to decode AVIF
/// images encoded by libheif v1.11.0 or older, be sure to disable this bit. (This issue has been
/// corrected in libheif v1.12.0.)
pub(crate) const AVIF_STRICT_PIXI_REQUIRED: u32 = 1 << 0;
/// This demands that the values surfaced in the clap box are valid, determined by attempting to
/// convert the clap box to a crop rect using avifCropRectConvertCleanApertureBox(). If this
/// function returns AVIF_FALSE and this strict flag is set, the decode will fail.
pub(crate) const AVIF_STRICT_CLAP_VALID: u32 = 1 << 1;
/// Requires the ImageSpatialExtentsProperty ('ispe') be present in alpha auxiliary image items.
/// avif-serialize 0.7.3 or older does not add the 'ispe' item property to alpha auxiliary image
/// items. If you need to decode AVIF images encoded by the cavif encoder with avif-serialize
/// 0.7.3 or older, be sure to disable this bit. (This issue has been corrected in avif-serialize
/// 0.7.4.) See https://github.com/kornelski/avif-serialize/issues/3 and
/// https://crbug.com/1246678.
pub(crate) const AVIF_STRICT_ALPHA_ISPE_REQUIRED: u32 = 1 << 2;
/// Maximum strictness; enables all bits above. This is avifDecoder's default.
pub(crate) const AVIF_STRICT_ENABLED: u32 =
    AVIF_STRICT_PIXI_REQUIRED | AVIF_STRICT_CLAP_VALID | AVIF_STRICT_ALPHA_ISPE_REQUIRED;
/// Translation of `avifStrictFlags`.
pub(crate) type AvifStrictFlags = u32;

/// Useful stats related to a read/write. Translation of `avifIOStats`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifIoStats {
    /// Size in bytes of the AV1 image item or track data containing color samples.
    pub(crate) color_obu_size: usize,
    /// Size in bytes of the AV1 image item or track data containing alpha samples.
    pub(crate) alpha_obu_size: usize,
}

/// Translation of `avifDecoderSource`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifDecoderSource {
    /// Honor the major brand signaled in the beginning of the file to pick between an AVIF sequence
    /// ('avis', tracks-based) or a single image ('avif', item-based). If the major brand is neither
    /// of these, prefer the AVIF sequence ('avis', tracks-based), if present.
    #[default]
    Auto = 0,

    /// Use the primary item and the aux (alpha) item in the avif(s).
    /// This is where single-image avifs store their image.
    PrimaryItem,

    /// Use the chunks inside primary/aux tracks in the moov block.
    /// This is where avifs image sequences store their images.
    Tracks,
    // Decode the thumbnail item. Currently unimplemented.
    // AVIF_DECODER_SOURCE_THUMBNAIL_ITEM
}

/// Information about the timing of a single image in an image sequence.
/// Translation of `avifImageTiming`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifImageTiming {
    /// timescale of the media (Hz)
    pub(crate) timescale: u64,
    /// presentation timestamp in seconds (ptsInTimescales / timescale)
    pub(crate) pts: f64,
    /// presentation timestamp in "timescales"
    pub(crate) pts_in_timescales: u64,
    /// in seconds (durationInTimescales / timescale)
    pub(crate) duration: f64,
    /// duration in "timescales"
    pub(crate) duration_in_timescales: u64,
}

/// Translation of `avifProgressiveState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum AvifProgressiveState {
    /// The current AVIF/Source does not offer a progressive image. This will always be the state
    /// for an image sequence.
    #[default]
    Unavailable = 0,

    /// The current AVIF/Source offers a progressive image, but avifDecoder.allowProgressive is not
    /// enabled, so it will behave as if the image was not progressive and will simply decode the
    /// best version of this item.
    Available,

    /// The current AVIF/Source offers a progressive image, and avifDecoder.allowProgressive is true.
    /// In this state, avifDecoder.imageCount will be the count of all of the available progressive
    /// layers, and any specific layer can be decoded using avifDecoderNthImage() as if it was an
    /// image sequence, or simply using repeated calls to avifDecoderNextImage() to decode better and
    /// better versions of this image.
    Active,
}

/// Translation of `avifExtent`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AvifExtent {
    pub(crate) offset: u64,
    pub(crate) size: usize,
}

// ---------------------------------------------------------------------------
// avif.c

/// Translation of `avifVersion()`.
pub(crate) fn avif_version() -> &'static str {
    "1.1.1"
}

/// Translation of `avifPixelFormatToString()`.
pub(crate) fn avif_pixel_format_to_string(format: AvifPixelFormat) -> &'static str {
    match format {
        AvifPixelFormat::Yuv444 => "YUV444",
        AvifPixelFormat::Yuv420 => "YUV420",
        AvifPixelFormat::Yuv422 => "YUV422",
        AvifPixelFormat::Yuv400 => "YUV400",
        AvifPixelFormat::None => "Unknown",
    }
}

/// Returns the avifPixelFormatInfo depending on the avifPixelFormat.
/// When monochrome is AVIF_TRUE, chromaShiftX and chromaShiftY are set to 1 according to the AV1 specification but they should be ignored.
///
/// Note: This function implements the second table on page 119 of the AV1 specification version 1.0.0 with Errata 1.
/// For monochrome 4:0:0, subsampling_x and subsampling are specified as 1 to allow
/// an AV1 implementation that only supports profile 0 to hardcode subsampling_x and subsampling_y to 1.
///
/// Translation of `avifGetPixelFormatInfo()`.
pub(crate) fn avif_get_pixel_format_info(format: AvifPixelFormat) -> AvifPixelFormatInfo {
    let mut info = AvifPixelFormatInfo::default();

    match format {
        AvifPixelFormat::Yuv444 => {
            info.chroma_shift_x = 0;
            info.chroma_shift_y = 0;
        }

        AvifPixelFormat::Yuv422 => {
            info.chroma_shift_x = 1;
            info.chroma_shift_y = 0;
        }

        AvifPixelFormat::Yuv420 => {
            info.chroma_shift_x = 1;
            info.chroma_shift_y = 1;
        }

        AvifPixelFormat::Yuv400 => {
            info.monochrome = true;
            // The nonexistent chroma is considered as subsampled in each dimension
            // according to the AV1 specification. See sections 5.5.2 and 6.4.2.
            info.chroma_shift_x = 1;
            info.chroma_shift_y = 1;
        }

        AvifPixelFormat::None => {}
    }
    info
}

/// Translation of `avifResultToString()`.
pub(crate) fn avif_result_to_string(result: AvifResult) -> &'static str {
    match result {
        AvifResult::Ok => "OK",
        AvifResult::InvalidFtyp => "Invalid ftyp",
        AvifResult::NoContent => "No content",
        AvifResult::NoYuvFormatSelected => "No YUV format selected",
        AvifResult::ReformatFailed => "Reformat failed",
        AvifResult::UnsupportedDepth => "Unsupported depth",
        AvifResult::EncodeColorFailed => "Encoding of color planes failed",
        AvifResult::EncodeAlphaFailed => "Encoding of alpha plane failed",
        AvifResult::BmffParseFailed => "BMFF parsing failed",
        AvifResult::MissingImageItem => "Missing or empty image item",
        AvifResult::DecodeColorFailed => "Decoding of color planes failed",
        AvifResult::DecodeAlphaFailed => "Decoding of alpha plane failed",
        AvifResult::ColorAlphaSizeMismatch => "Color and alpha planes size mismatch",
        AvifResult::IspeSizeMismatch => "Plane sizes don't match ispe values",
        AvifResult::NoCodecAvailable => "No codec available",
        AvifResult::NoImagesRemaining => "No images remaining",
        AvifResult::InvalidExifPayload => "Invalid Exif payload",
        AvifResult::InvalidImageGrid => "Invalid image grid",
        AvifResult::InvalidCodecSpecificOption => "Invalid codec-specific option",
        AvifResult::TruncatedData => "Truncated data",
        AvifResult::IoNotSet => "IO not set",
        AvifResult::IoError => "IO Error",
        AvifResult::WaitingOnIo => "Waiting on IO",
        AvifResult::InvalidArgument => "Invalid argument",
        AvifResult::NotImplemented => "Not implemented",
        AvifResult::OutOfMemory => "Out of memory",
        AvifResult::CannotChangeSetting => "Cannot change some setting during encoding",
        AvifResult::IncompatibleImage => "The image is incompatible with already encoded images",
        AvifResult::InternalError => "Internal error",
        AvifResult::UnknownError => "Unknown Error",
    }
}

/// Translation of `avifProgressiveStateToString()`.
pub(crate) fn avif_progressive_state_to_string(
    progressive_state: AvifProgressiveState,
) -> &'static str {
    match progressive_state {
        AvifProgressiveState::Unavailable => "Unavailable",
        AvifProgressiveState::Available => "Available",
        AvifProgressiveState::Active => "Active",
    }
}

/// Translation of `avifImageSetDefaults()`.
pub(crate) fn avif_image_set_defaults(image: &mut AvifImage) {
    *image = AvifImage::default();
    image.yuv_range = AvifRange::Full;
    image.color_primaries = AVIF_COLOR_PRIMARIES_UNSPECIFIED;
    image.transfer_characteristics = AVIF_TRANSFER_CHARACTERISTICS_UNSPECIFIED;
    image.matrix_coefficients = AVIF_MATRIX_COEFFICIENTS_UNSPECIFIED;
}

/// Translation of `avifImageCreate()`: `None` if arguments are invalid.
pub(crate) fn avif_image_create(
    width: u32,
    height: u32,
    depth: u32,
    yuv_format: AvifPixelFormat,
) -> Option<Box<AvifImage>> {
    // width and height are checked when actually used, for example by avifImageAllocatePlanes().
    avif_checkerr!(depth <= 16, None); // avifImage only supports up to 16 bits per sample. See avifImageUsesU16().
                                       // Cast to silence "comparison of unsigned expression is always true" warning.
    avif_checkerr!((yuv_format as u32) < AVIF_PIXEL_FORMAT_COUNT, None);

    let mut image = Box::<AvifImage>::default();
    avif_image_set_defaults(&mut image);
    image.width = width;
    image.height = height;
    image.depth = depth;
    image.yuv_format = yuv_format;
    Some(image)
}

/// helper for making an image to decode into. Translation of
/// `avifImageCreateEmpty()`.
pub(crate) fn avif_image_create_empty() -> Option<Box<AvifImage>> {
    avif_image_create(0, 0, 0, AvifPixelFormat::None)
}

/// Copies all fields that do not need to be freed/allocated from srcImage
/// to dstImage. Translation of `avifImageCopyNoAlloc()`.
pub(crate) fn avif_image_copy_no_alloc(dst_image: &mut AvifImage, src_image: &AvifImage) {
    dst_image.width = src_image.width;
    dst_image.height = src_image.height;
    dst_image.depth = src_image.depth;
    dst_image.yuv_format = src_image.yuv_format;
    dst_image.yuv_range = src_image.yuv_range;
    dst_image.yuv_chroma_sample_position = src_image.yuv_chroma_sample_position;
    dst_image.alpha_premultiplied = src_image.alpha_premultiplied;

    dst_image.color_primaries = src_image.color_primaries;
    dst_image.transfer_characteristics = src_image.transfer_characteristics;
    dst_image.matrix_coefficients = src_image.matrix_coefficients;
    dst_image.clli = src_image.clli;

    dst_image.transform_flags = src_image.transform_flags;
    dst_image.pasp = src_image.pasp;
    dst_image.clap = src_image.clap;
    dst_image.irot = src_image.irot;
    dst_image.imir = src_image.imir;
}

/// Copies the samples from srcImage to dstImage. dstImage must be allocated.
/// srcImage and dstImage must have the same width, height, and depth.
/// If the AVIF_PLANES_YUV bit is set in planes, then srcImage and dstImage must have the same yuvFormat.
/// Ignores the gainMap field (which exists only if AVIF_ENABLE_EXPERIMENTAL_GAIN_MAP is defined).
///
/// Translation of `avifImageCopySamples()` (upstream's asserts are debug
/// assertions).
pub(crate) fn avif_image_copy_samples(
    dst_image: &mut AvifImage,
    src_image: &AvifImage,
    planes: AvifPlanesFlags,
) {
    debug_assert!(src_image.depth == dst_image.depth);
    if planes & AVIF_PLANES_YUV != 0 {
        debug_assert!(src_image.yuv_format == dst_image.yuv_format);
        // Note that there may be a mismatch between srcImage->yuvRange and dstImage->yuvRange
        // because libavif allows for 'colr' and AV1 OBU video range values to differ.
    }
    let bytes_per_pixel: usize = if avif_image_uses_u16(src_image) { 2 } else { 1 };

    let skip_color = planes & AVIF_PLANES_YUV == 0;
    let skip_alpha = planes & AVIF_PLANES_A == 0;
    for c in AVIF_CHAN_Y..=AVIF_CHAN_A {
        let alpha = c == AVIF_CHAN_A;
        if (skip_color && !alpha) || (skip_alpha && alpha) {
            continue;
        }

        let plane_width = avif_image_plane_width(src_image, c);
        let plane_height = avif_image_plane_height(src_image, c);
        let src_row_bytes = avif_image_plane_row_bytes(src_image, c) as usize;
        let dst_row_bytes = avif_image_plane_row_bytes(dst_image, c) as usize;
        let Some(src_row) = avif_image_plane(src_image, c) else {
            continue;
        };
        debug_assert!(plane_width == avif_image_plane_width(dst_image, c));
        debug_assert!(plane_height == avif_image_plane_height(dst_image, c));
        let Some(dst_row) = avif_image_plane_mut(dst_image, c) else {
            continue;
        };

        let plane_width_bytes = plane_width as usize * bytes_per_pixel;
        if bytes_per_pixel == 2 {
            let src = src_row.u16s();
            let dst = dst_row.u16s_mut();
            let (w, srs, drs) = (plane_width_bytes / 2, src_row_bytes / 2, dst_row_bytes / 2);
            for y in 0..plane_height as usize {
                dst[y * drs..y * drs + w].copy_from_slice(&src[y * srs..y * srs + w]);
            }
        } else {
            let src = src_row.u8s();
            let dst = dst_row.u8s_mut();
            let w = plane_width_bytes;
            for y in 0..plane_height as usize {
                dst[y * dst_row_bytes..y * dst_row_bytes + w]
                    .copy_from_slice(&src[y * src_row_bytes..y * src_row_bytes + w]);
            }
        }
    }
}

/// Performs a deep copy of an image, including all metadata and planes.
/// Translation of `avifImageCopy()`.
pub(crate) fn avif_image_copy(
    dst_image: &mut AvifImage,
    src_image: &AvifImage,
    planes: AvifPlanesFlags,
) -> AvifResult {
    avif_image_free_planes(dst_image, AVIF_PLANES_ALL);
    avif_image_copy_no_alloc(dst_image, src_image);

    avif_checkres!(avif_image_set_profile_icc(dst_image, &src_image.icc));

    avif_checkres!(avif_rw_data_set(&mut dst_image.exif, &src_image.exif));
    avif_checkres!(avif_image_set_metadata_xmp(dst_image, &src_image.xmp));

    if (planes & AVIF_PLANES_YUV) != 0 && src_image.yuv_planes[AVIF_CHAN_Y].is_some() {
        if (src_image.yuv_format != AvifPixelFormat::Yuv400)
            && (src_image.yuv_planes[AVIF_CHAN_U].is_none()
                || src_image.yuv_planes[AVIF_CHAN_V].is_none())
        {
            return AvifResult::InvalidArgument;
        }
        let allocation_result = avif_image_allocate_planes(dst_image, AVIF_PLANES_YUV);
        if allocation_result != AvifResult::Ok {
            return allocation_result;
        }
    }
    if (planes & AVIF_PLANES_A) != 0 && src_image.alpha_plane.is_some() {
        let allocation_result = avif_image_allocate_planes(dst_image, AVIF_PLANES_A);
        if allocation_result != AvifResult::Ok {
            return allocation_result;
        }
    }
    avif_image_copy_samples(dst_image, src_image, planes);

    AvifResult::Ok
}

/// Performs a shallow copy of a rectangular area of an image. 'dstImage'
/// does not own the planes. Translation of `avifImageSetViewRect()`.
pub(crate) fn avif_image_set_view_rect(
    dst_image: &mut AvifImage,
    src_image: &AvifImage,
    rect: &AvifCropRect,
) -> AvifResult {
    let format_info = avif_get_pixel_format_info(src_image.yuv_format);
    if (rect.width > src_image.width)
        || (rect.height > src_image.height)
        || (rect.x > (src_image.width - rect.width))
        || (rect.y > (src_image.height - rect.height))
    {
        return AvifResult::InvalidArgument;
    }
    if !format_info.monochrome
        && ((rect.x & format_info.chroma_shift_x as u32) != 0
            || (rect.y & format_info.chroma_shift_y as u32) != 0)
    {
        return AvifResult::InvalidArgument;
    }
    avif_image_free_planes(dst_image, AVIF_PLANES_ALL); // dstImage->imageOwnsYUVPlanes and dstImage->imageOwnsAlphaPlane set to AVIF_FALSE.
    avif_image_copy_no_alloc(dst_image, src_image);
    dst_image.width = rect.width;
    dst_image.height = rect.height;
    let pixel_bytes: usize = if src_image.depth > 8 { 2 } else { 1 };
    if src_image.yuv_planes[AVIF_CHAN_Y].is_some() {
        for yuv_plane in AVIF_CHAN_Y..=AVIF_CHAN_V {
            if src_image.yuv_row_bytes[yuv_plane] != 0 {
                let plane_x = if yuv_plane == AVIF_CHAN_Y {
                    rect.x as usize
                } else {
                    (rect.x >> format_info.chroma_shift_x) as usize
                };
                let plane_y = if yuv_plane == AVIF_CHAN_Y {
                    rect.y as usize
                } else {
                    (rect.y >> format_info.chroma_shift_y) as usize
                };
                dst_image.yuv_planes[yuv_plane] =
                    src_image.yuv_planes[yuv_plane].as_ref().map(|p| {
                        p.offset_by(
                            plane_y * src_image.yuv_row_bytes[yuv_plane] as usize
                                + plane_x * pixel_bytes,
                        )
                    });
                dst_image.yuv_row_bytes[yuv_plane] = src_image.yuv_row_bytes[yuv_plane];
            }
        }
    }
    if let Some(alpha) = &src_image.alpha_plane {
        dst_image.alpha_plane = Some(alpha.offset_by(
            rect.y as usize * src_image.alpha_row_bytes as usize + rect.x as usize * pixel_bytes,
        ));
        dst_image.alpha_row_bytes = src_image.alpha_row_bytes;
    }
    AvifResult::Ok
}

/// Translation of `avifImageSetProfileICC()`.
pub(crate) fn avif_image_set_profile_icc(image: &mut AvifImage, icc: &[u8]) -> AvifResult {
    avif_rw_data_set(&mut image.icc, icc)
}

/// Sets XMP metadata. Translation of `avifImageSetMetadataXMP()`.
pub(crate) fn avif_image_set_metadata_xmp(image: &mut AvifImage, xmp: &[u8]) -> AvifResult {
    avif_rw_data_set(&mut image.xmp, xmp)
}

/// Ignores any pre-existing planes. Translation of
/// `avifImageAllocatePlanes()`.
pub(crate) fn avif_image_allocate_planes(
    image: &mut AvifImage,
    planes: AvifPlanesFlags,
) -> AvifResult {
    if image.width == 0 || image.height == 0 {
        return AvifResult::InvalidArgument;
    }
    let channel_size: usize = if avif_image_uses_u16(image) { 2 } else { 1 };
    if image.width as usize > usize::MAX / channel_size {
        return AvifResult::InvalidArgument;
    }
    let full_row_bytes = channel_size * image.width as usize;
    if (full_row_bytes > u32::MAX as usize) || (image.height as usize > usize::MAX / full_row_bytes)
    {
        return AvifResult::InvalidArgument;
    }
    let full_size = full_row_bytes * image.height as usize;
    let u16_samples = channel_size == 2;

    if (planes & AVIF_PLANES_YUV) != 0 && (image.yuv_format != AvifPixelFormat::None) {
        let info = avif_get_pixel_format_info(image.yuv_format);

        image.image_owns_yuv_planes = true;
        if image.yuv_planes[AVIF_CHAN_Y].is_none() {
            image.yuv_row_bytes[AVIF_CHAN_Y] = full_row_bytes as u32;
            image.yuv_planes[AVIF_CHAN_Y] = AvifPlane::alloc(full_size, u16_samples);
            if image.yuv_planes[AVIF_CHAN_Y].is_none() {
                return AvifResult::OutOfMemory;
            }
        }

        if !info.monochrome {
            // Intermediary computation as 64 bits in case width or height is exactly UINT32_MAX.
            let shifted_w =
                ((image.width as u64 + info.chroma_shift_x as u64) >> info.chroma_shift_x) as u32;
            let shifted_h =
                ((image.height as u64 + info.chroma_shift_y as u64) >> info.chroma_shift_y) as u32;

            // These are less than or equal to fullRowBytes/fullSize. No need to check overflows.
            let uv_row_bytes = channel_size * shifted_w as usize;
            let uv_size = uv_row_bytes * shifted_h as usize;

            for uv_plane in AVIF_CHAN_U..=AVIF_CHAN_V {
                if image.yuv_planes[uv_plane].is_none() {
                    image.yuv_row_bytes[uv_plane] = uv_row_bytes as u32;
                    image.yuv_planes[uv_plane] = AvifPlane::alloc(uv_size, u16_samples);
                    if image.yuv_planes[uv_plane].is_none() {
                        return AvifResult::OutOfMemory;
                    }
                }
            }
        }
    }
    if planes & AVIF_PLANES_A != 0 {
        image.image_owns_alpha_plane = true;
        if image.alpha_plane.is_none() {
            image.alpha_row_bytes = full_row_bytes as u32;
            image.alpha_plane = AvifPlane::alloc(full_size, u16_samples);
            if image.alpha_plane.is_none() {
                return AvifResult::OutOfMemory;
            }
        }
    }
    AvifResult::Ok
}

/// Ignores already-freed planes. Translation of `avifImageFreePlanes()`
/// (a plane the image doesn't own is the codec's or another image's,
/// whose reference is dropped).
pub(crate) fn avif_image_free_planes(image: &mut AvifImage, planes: AvifPlanesFlags) {
    if (planes & AVIF_PLANES_YUV) != 0 && (image.yuv_format != AvifPixelFormat::None) {
        image.yuv_planes[AVIF_CHAN_Y] = None;
        image.yuv_row_bytes[AVIF_CHAN_Y] = 0;
        image.yuv_planes[AVIF_CHAN_U] = None;
        image.yuv_row_bytes[AVIF_CHAN_U] = 0;
        image.yuv_planes[AVIF_CHAN_V] = None;
        image.yuv_row_bytes[AVIF_CHAN_V] = 0;
        image.image_owns_yuv_planes = false;
    }
    if planes & AVIF_PLANES_A != 0 {
        image.alpha_plane = None;
        image.alpha_row_bytes = 0;
        image.image_owns_alpha_plane = false;
    }
}

/// Translation of `avifImageStealPlanes()`.
pub(crate) fn avif_image_steal_planes(
    dst_image: &mut AvifImage,
    src_image: &mut AvifImage,
    planes: AvifPlanesFlags,
) {
    avif_image_free_planes(dst_image, planes);

    if planes & AVIF_PLANES_YUV != 0 {
        dst_image.yuv_planes[AVIF_CHAN_Y] = src_image.yuv_planes[AVIF_CHAN_Y].take();
        dst_image.yuv_row_bytes[AVIF_CHAN_Y] = src_image.yuv_row_bytes[AVIF_CHAN_Y];
        dst_image.yuv_planes[AVIF_CHAN_U] = src_image.yuv_planes[AVIF_CHAN_U].take();
        dst_image.yuv_row_bytes[AVIF_CHAN_U] = src_image.yuv_row_bytes[AVIF_CHAN_U];
        dst_image.yuv_planes[AVIF_CHAN_V] = src_image.yuv_planes[AVIF_CHAN_V].take();
        dst_image.yuv_row_bytes[AVIF_CHAN_V] = src_image.yuv_row_bytes[AVIF_CHAN_V];

        src_image.yuv_row_bytes[AVIF_CHAN_Y] = 0;
        src_image.yuv_row_bytes[AVIF_CHAN_U] = 0;
        src_image.yuv_row_bytes[AVIF_CHAN_V] = 0;

        dst_image.yuv_format = src_image.yuv_format;
        dst_image.image_owns_yuv_planes = src_image.image_owns_yuv_planes;
        src_image.image_owns_yuv_planes = false;
    }
    if planes & AVIF_PLANES_A != 0 {
        dst_image.alpha_plane = src_image.alpha_plane.take();
        dst_image.alpha_row_bytes = src_image.alpha_row_bytes;

        src_image.alpha_row_bytes = 0;

        dst_image.image_owns_alpha_plane = src_image.image_owns_alpha_plane;
        src_image.image_owns_alpha_plane = false;
    }
}

/// Translation of `avifImageUsesU16()`.
pub(crate) fn avif_image_uses_u16(image: &AvifImage) -> bool {
    image.depth > 8
}

/// Translation of `avifImageIsOpaque()`.
pub(crate) fn avif_image_is_opaque(image: &AvifImage) -> bool {
    let Some(alpha) = &image.alpha_plane else {
        return true;
    };

    let opaque_value = (1u32 << image.depth).wrapping_sub(1);
    let row_bytes = image.alpha_row_bytes as usize;
    for y in 0..image.height as usize {
        if avif_image_uses_u16(image) {
            let row16 = &alpha.u16s()[y * row_bytes / 2..];
            for x in 0..image.width as usize {
                if row16[x] as u32 != opaque_value {
                    return false;
                }
            }
        } else {
            let row = &alpha.u8s()[y * row_bytes..];
            for x in 0..image.width as usize {
                if row[x] as u32 != opaque_value {
                    return false;
                }
            }
        }
    }
    true
}

/// channel can be an avifChannelIndex. Translation of `avifImagePlane()`.
pub(crate) fn avif_image_plane(image: &AvifImage, channel: usize) -> Option<&AvifPlane> {
    if (channel == AVIF_CHAN_Y) || (channel == AVIF_CHAN_U) || (channel == AVIF_CHAN_V) {
        return image.yuv_planes[channel].as_ref();
    }
    if channel == AVIF_CHAN_A {
        return image.alpha_plane.as_ref();
    }
    None
}

/// [`avif_image_plane`], writable.
pub(crate) fn avif_image_plane_mut(
    image: &mut AvifImage,
    channel: usize,
) -> Option<&mut AvifPlane> {
    if (channel == AVIF_CHAN_Y) || (channel == AVIF_CHAN_U) || (channel == AVIF_CHAN_V) {
        return image.yuv_planes[channel].as_mut();
    }
    if channel == AVIF_CHAN_A {
        return image.alpha_plane.as_mut();
    }
    None
}

/// Translation of `avifImagePlaneRowBytes()`.
pub(crate) fn avif_image_plane_row_bytes(image: &AvifImage, channel: usize) -> u32 {
    if (channel == AVIF_CHAN_Y) || (channel == AVIF_CHAN_U) || (channel == AVIF_CHAN_V) {
        return image.yuv_row_bytes[channel];
    }
    if channel == AVIF_CHAN_A {
        return image.alpha_row_bytes;
    }
    0
}

/// Translation of `avifImagePlaneWidth()`.
pub(crate) fn avif_image_plane_width(image: &AvifImage, channel: usize) -> u32 {
    if channel == AVIF_CHAN_Y {
        return image.width;
    }
    if (channel == AVIF_CHAN_U) || (channel == AVIF_CHAN_V) {
        let format_info = avif_get_pixel_format_info(image.yuv_format);
        if format_info.monochrome {
            return 0;
        }
        return (image.width + format_info.chroma_shift_x as u32) >> format_info.chroma_shift_x;
    }
    if (channel == AVIF_CHAN_A) && image.alpha_plane.is_some() {
        return image.width;
    }
    0
}

/// Translation of `avifImagePlaneHeight()`.
pub(crate) fn avif_image_plane_height(image: &AvifImage, channel: usize) -> u32 {
    if channel == AVIF_CHAN_Y {
        return image.height;
    }
    if (channel == AVIF_CHAN_U) || (channel == AVIF_CHAN_V) {
        let format_info = avif_get_pixel_format_info(image.yuv_format);
        if format_info.monochrome {
            return 0;
        }
        return (image.height + format_info.chroma_shift_y as u32) >> format_info.chroma_shift_y;
    }
    if (channel == AVIF_CHAN_A) && image.alpha_plane.is_some() {
        return image.height;
    }
    0
}

/// Translation of `avifDimensionsTooLarge()`.
pub(crate) fn avif_dimensions_too_large(
    width: u32,
    height: u32,
    image_size_limit: u32,
    image_dimension_limit: u32,
) -> bool {
    // FIXME (upstream): divides by zero for a height of 0 (the callers
    // check for it first).
    if width > (image_size_limit / height) {
        return true;
    }
    if (image_dimension_limit != 0)
        && ((width > image_dimension_limit) || (height > image_dimension_limit))
    {
        return true;
    }
    false
}

// avifCodecCreate*() functions are in their respective codec_*.c files
// (avifCodecDestroy() is dropping the codec)

// ---------------------------------------------------------------------------
// avifRGBImage

/// Translation of `avifRGBFormatHasAlpha()`.
pub(crate) fn avif_rgb_format_has_alpha(format: AvifRgbFormat) -> bool {
    (format != AvifRgbFormat::Rgb)
        && (format != AvifRgbFormat::Bgr)
        && (format != AvifRgbFormat::Rgb565)
}

/// Translation of `avifRGBFormatChannelCount()`.
pub(crate) fn avif_rgb_format_channel_count(format: AvifRgbFormat) -> u32 {
    if avif_rgb_format_has_alpha(format) {
        4
    } else {
        3
    }
}

/// Translation of `avifRGBImagePixelSize()`.
pub(crate) fn avif_rgb_image_pixel_size(rgb: &AvifRgbImage<'_>) -> u32 {
    if rgb.format == AvifRgbFormat::Rgb565 {
        return 2;
    }
    avif_rgb_format_channel_count(rgb.format) * if rgb.depth > 8 { 2 } else { 1 }
}

/// Sets rgb->width, rgb->height, and rgb->depth to image->width, image->height, and image->depth.
/// Sets rgb->pixels to NULL and rgb->rowBytes to 0. Sets the other fields of 'rgb' to default
/// values.
///
/// Translation of `avifRGBImageSetDefaults()`.
pub(crate) fn avif_rgb_image_set_defaults(rgb: &mut AvifRgbImage<'_>, image: &AvifImage) {
    rgb.width = image.width;
    rgb.height = image.height;
    rgb.depth = image.depth;
    rgb.format = AvifRgbFormat::Rgba;
    rgb.chroma_upsampling = AvifChromaUpsampling::Automatic;
    rgb.chroma_downsampling = AvifChromaDownsampling::Automatic;
    rgb.avoid_lib_yuv = false;
    rgb.ignore_alpha = false;
    rgb.pixels = None;
    rgb.row_bytes = 0;
    rgb.alpha_premultiplied = false; // Most expect RGBA output to *not* be premultiplied. Those that do can opt-in by
                                     // setting this to match image->alphaPremultiplied or forcing this to true
                                     // after calling avifRGBImageSetDefaults(),
    rgb.is_float = false;
    rgb.max_threads = 1;
}

// (avifRGBImageAllocatePixels() and avifRGBImageFreePixels() are not
// translated: the caller supplies the pixels.)

// ---------------------------------------------------------------------------
// avifCropRect

/// Translation of `calcCenter()`.
fn calc_center(dim: i32) -> AvifFraction {
    let mut f = AvifFraction { n: dim >> 1, d: 1 };
    if (dim % 2) != 0 {
        f.n = dim;
        f.d = 2;
    }
    f
}

/// Translation of `avifCropRectIsValid()`.
fn avif_crop_rect_is_valid(
    crop_rect: &AvifCropRect,
    image_w: u32,
    image_h: u32,
    yuv_format: AvifPixelFormat,
    diag: Option<&super::diag::AvifDiagnostics>,
) -> bool {
    // ISO/IEC 23000-22:2019/Amd. 2:2021, Section 7.3.6.7:
    //   The clean aperture property is restricted according to the chroma
    //   sampling format of the input image (4:4:4, 4:2:2:, 4:2:0, or 4:0:0) as
    //   follows:
    //   ...
    //   - If chroma is subsampled horizontally (i.e., 4:2:2 and 4:2:0), the
    //     leftmost pixel of the clean aperture shall be even numbers;
    //   - If chroma is subsampled vertically (i.e., 4:2:0), the topmost line
    //     of the clean aperture shall be even numbers.

    if (crop_rect.width == 0) || (crop_rect.height == 0) {
        super::diag::printf(
            diag,
            format_args!("[Strict] crop rect width and height must be nonzero"),
        );
        return false;
    }
    if (crop_rect.x > (u32::MAX - crop_rect.width))
        || ((crop_rect.x + crop_rect.width) > image_w)
        || (crop_rect.y > (u32::MAX - crop_rect.height))
        || ((crop_rect.y + crop_rect.height) > image_h)
    {
        super::diag::printf(
            diag,
            format_args!("[Strict] crop rect is out of the image's bounds"),
        );
        return false;
    }

    if (yuv_format == AvifPixelFormat::Yuv420) || (yuv_format == AvifPixelFormat::Yuv422) {
        if (crop_rect.x % 2) != 0 {
            super::diag::printf(
                diag,
                format_args!(
                    "[Strict] crop rect X offset must be even due to this image's YUV subsampling"
                ),
            );
            return false;
        }
    }
    if yuv_format == AvifPixelFormat::Yuv420 {
        if (crop_rect.y % 2) != 0 {
            super::diag::printf(
                diag,
                format_args!(
                    "[Strict] crop rect Y offset must be even due to this image's YUV subsampling"
                ),
            );
            return false;
        }
    }
    true
}

/// These will return AVIF_FALSE if the resultant values violate any standards, and if so, the output
/// values are not guaranteed to be complete or correct and should not be used.
///
/// Translation of `avifCropRectConvertCleanApertureBox()`.
pub(crate) fn avif_crop_rect_convert_clean_aperture_box(
    crop_rect: &mut AvifCropRect,
    clap: &AvifCleanApertureBox,
    image_w: u32,
    image_h: u32,
    yuv_format: AvifPixelFormat,
    diag: Option<&super::diag::AvifDiagnostics>,
) -> bool {
    super::diag::clear_error(diag);
    use super::utils::{avif_fraction_add, avif_fraction_sub};
    let printf = |args: std::fmt::Arguments<'_>| super::diag::printf(diag, args);

    // ISO/IEC 14496-12:2020, Section 12.1.4.1:
    //   For horizOff and vertOff, D shall be strictly positive and N may be
    //   positive or negative. For cleanApertureWidth and cleanApertureHeight,
    //   N shall be positive and D shall be strictly positive.

    let width_n = clap.width_n as i32;
    let width_d = clap.width_d as i32;
    let height_n = clap.height_n as i32;
    let height_d = clap.height_d as i32;
    let horiz_off_n = clap.horiz_off_n as i32;
    let horiz_off_d = clap.horiz_off_d as i32;
    let vert_off_n = clap.vert_off_n as i32;
    let vert_off_d = clap.vert_off_d as i32;
    if (width_d <= 0) || (height_d <= 0) || (horiz_off_d <= 0) || (vert_off_d <= 0) {
        printf(format_args!(
            "[Strict] clap contains a denominator that is not strictly positive"
        ));
        return false;
    }
    if (width_n < 0) || (height_n < 0) {
        printf(format_args!("[Strict] clap width or height is negative"));
        return false;
    }

    if (width_n % width_d) != 0 {
        printf(format_args!(
            "[Strict] clap width {width_n}/{width_d} is not an integer"
        ));
        return false;
    }
    if (height_n % height_d) != 0 {
        printf(format_args!(
            "[Strict] clap height {height_n}/{height_d} is not an integer"
        ));
        return false;
    }
    let clap_w = width_n / width_d;
    let clap_h = height_n / height_d;

    if (image_w > i32::MAX as u32) || (image_h > i32::MAX as u32) {
        printf(format_args!(
            "[Strict] image width {image_w} or height {image_h} is greater than INT32_MAX"
        ));
        return false;
    }
    let uncropped_center_x = calc_center(image_w as i32);
    let uncropped_center_y = calc_center(image_h as i32);

    let horiz_off = AvifFraction {
        n: horiz_off_n,
        d: horiz_off_d,
    };
    let mut cropped_center_x = AvifFraction::default();
    if !avif_fraction_add(uncropped_center_x, horiz_off, &mut cropped_center_x) {
        printf(format_args!("[Strict] croppedCenterX overflowed"));
        return false;
    }

    let vert_off = AvifFraction {
        n: vert_off_n,
        d: vert_off_d,
    };
    let mut cropped_center_y = AvifFraction::default();
    if !avif_fraction_add(uncropped_center_y, vert_off, &mut cropped_center_y) {
        printf(format_args!("[Strict] croppedCenterY overflowed"));
        return false;
    }

    let half_w = AvifFraction { n: clap_w, d: 2 };
    let mut crop_x = AvifFraction::default();
    if !avif_fraction_sub(cropped_center_x, half_w, &mut crop_x) {
        printf(format_args!("[Strict] cropX overflowed"));
        return false;
    }
    if (crop_x.n % crop_x.d) != 0 {
        printf(format_args!(
            "[Strict] calculated crop X offset {}/{} is not an integer",
            crop_x.n, crop_x.d
        ));
        return false;
    }

    let half_h = AvifFraction { n: clap_h, d: 2 };
    let mut crop_y = AvifFraction::default();
    if !avif_fraction_sub(cropped_center_y, half_h, &mut crop_y) {
        printf(format_args!("[Strict] cropY overflowed"));
        return false;
    }
    if (crop_y.n % crop_y.d) != 0 {
        printf(format_args!(
            "[Strict] calculated crop Y offset {}/{} is not an integer",
            crop_y.n, crop_y.d
        ));
        return false;
    }

    if (crop_x.n < 0) || (crop_y.n < 0) {
        printf(format_args!(
            "[Strict] at least one crop offset is not positive"
        ));
        return false;
    }

    crop_rect.x = (crop_x.n / crop_x.d) as u32;
    crop_rect.y = (crop_y.n / crop_y.d) as u32;
    crop_rect.width = clap_w as u32;
    crop_rect.height = clap_h as u32;
    avif_crop_rect_is_valid(crop_rect, image_w, image_h, yuv_format, diag)
}

// (avifCleanApertureBoxConvertCropRect() is for encoding: not translated.)

// ---------------------------------------------------------------------------

/// Returns false if the tiles in a grid image violate any standards.
/// The image contains imageW*imageH pixels. The tiles are of tileW*tileH pixels each.
///
/// Translation of `avifAreGridDimensionsValid()`.
pub(crate) fn avif_are_grid_dimensions_valid(
    yuv_format: AvifPixelFormat,
    image_w: u32,
    image_h: u32,
    tile_w: u32,
    tile_h: u32,
    diag: Option<&super::diag::AvifDiagnostics>,
) -> bool {
    // ISO/IEC 23000-22:2019, Section 7.3.11.4.2:
    //   - the tile_width shall be greater than or equal to 64, and should be a multiple of 64
    //   - the tile_height shall be greater than or equal to 64, and should be a multiple of 64
    // The "should" part is ignored here.
    if (tile_w < 64) || (tile_h < 64) {
        super::diag::printf(
            diag,
            format_args!(
                "Grid image tile width ({tile_w}) or height ({tile_h}) cannot be smaller than 64. \
                 See MIAF (ISO/IEC 23000-22:2019), Section 7.3.11.4.2"
            ),
        );
        return false;
    }

    // ISO/IEC 23000-22:2019, Section 7.3.11.4.2:
    //   - when the images are in the 4:2:2 chroma sampling format the horizontal tile offsets and widths,
    //     and the output width, shall be even numbers;
    //   - when the images are in the 4:2:0 chroma sampling format both the horizontal and vertical tile
    //     offsets and widths, and the output width and height, shall be even numbers.
    // If the rules above were not respected, the following problematic situation may happen:
    //   Some 4:2:0 image is 650 pixels wide and has 10 cell columns, each being 65 pixels wide.
    //   The chroma plane of the whole image is 325 pixels wide. The chroma plane of each cell is 33 pixels wide.
    //   33*10 - 325 gives 5 extra pixels with no specified destination in the reconstructed image.

    // Tile offsets are not enforced since they depend on tile size (ISO/IEC 23008-12:2017, Section 6.6.2.3.1):
    //   The reconstructed image is formed by tiling the input images into a grid [...] without gap or overlap
    if (((yuv_format == AvifPixelFormat::Yuv420) || (yuv_format == AvifPixelFormat::Yuv422))
        && (((image_w % 2) != 0) || ((tile_w % 2) != 0)))
        || ((yuv_format == AvifPixelFormat::Yuv420)
            && (((image_h % 2) != 0) || ((tile_h % 2) != 0)))
    {
        super::diag::printf(
            diag,
            format_args!(
                "Grid image width ({image_w}) or height ({image_h}) or tile width ({tile_w}) or height ({tile_h}) \
                 shall be even if chroma is subsampled in that dimension. \
                 See MIAF (ISO/IEC 23000-22:2019), Section 7.3.11.4.2"
            ),
        );
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// avifCodecSpecificOption
// (encoder options: not translated)

// ---------------------------------------------------------------------------
// Codec availability and versions

/// Translation of `struct AvailableCodec` (`create` is
/// [`avif_codec_create_dav1d`](super::codec_dav1d::avif_codec_create_dav1d)
/// for the only codec that can decode).
struct AvailableCodec {
    choice: AvifCodecChoice,
    type_: super::internal::AvifCodecType,
    name: &'static str,
    version: fn() -> &'static str,
    flags: u32,
}

/// The libaom version SDL_image builds (`avifCodecVersionAOM()`, for
/// avifCodecVersions(): its encoder isn't translated).
fn avif_codec_version_aom() -> &'static str {
    "v3.6.1"
}

// This is the main codec table; it determines all usage/availability in libavif.

static AVAILABLE_CODECS: &[AvailableCodec] = &[
    // Ordered by preference (for AUTO)
    AvailableCodec {
        choice: AvifCodecChoice::Dav1d,
        type_: super::internal::AvifCodecType::Av1,
        name: "dav1d",
        version: super::codec_dav1d::avif_codec_version_dav1d,
        flags: AVIF_CODEC_FLAG_CAN_DECODE,
    },
    AvailableCodec {
        choice: AvifCodecChoice::Aom,
        type_: super::internal::AvifCodecType::Av1,
        name: "aom",
        version: avif_codec_version_aom,
        flags: AVIF_CODEC_FLAG_CAN_ENCODE,
    },
];

/// Translation of `findAvailableCodec()`.
fn find_available_codec(
    choice: AvifCodecChoice,
    required_flags: AvifCodecFlags,
) -> Option<&'static AvailableCodec> {
    for codec in AVAILABLE_CODECS {
        if (choice != AvifCodecChoice::Auto) && (codec.choice != choice) {
            continue;
        }
        if required_flags != 0 && ((codec.flags & required_flags) != required_flags) {
            continue;
        }
        if (choice == AvifCodecChoice::Auto) && (codec.choice == AvifCodecChoice::Avm) {
            // AV2 is experimental and cannot be the default, it must be explicitly selected.
            continue;
        }
        return Some(codec);
    }
    None
}

/// If this returns NULL, the codec choice/flag combination is unavailable.
/// Translation of `avifCodecName()`.
pub(crate) fn avif_codec_name(
    choice: AvifCodecChoice,
    required_flags: AvifCodecFlags,
) -> Option<&'static str> {
    find_available_codec(choice, required_flags).map(|c| c.name)
}

/// Returns AVIF_CODEC_TYPE_UNKNOWN unless the chosen codec is available
/// with the requiredFlags. Translation of `avifCodecTypeFromChoice()`.
pub(crate) fn avif_codec_type_from_choice(
    choice: AvifCodecChoice,
    required_flags: AvifCodecFlags,
) -> super::internal::AvifCodecType {
    match find_available_codec(choice, required_flags) {
        Some(c) => c.type_,
        None => super::internal::AvifCodecType::Unknown,
    }
}

/// Translation of `avifCodecChoiceFromName()`.
pub(crate) fn avif_codec_choice_from_name(name: &str) -> AvifCodecChoice {
    for codec in AVAILABLE_CODECS {
        if codec.name == name {
            return codec.choice;
        }
    }
    AvifCodecChoice::Auto
}

/// Translation of `avifCodecCreate()`: the codec that can decode is
/// dav1d's.
pub(crate) fn avif_codec_create(
    choice: AvifCodecChoice,
    required_flags: AvifCodecFlags,
    codec: &mut Option<Box<super::codec_dav1d::AvifCodec>>,
) -> AvifResult {
    *codec = None;
    let available_codec = find_available_codec(choice, required_flags);
    avif_checkerr!(available_codec.is_some(), AvifResult::NoCodecAvailable);
    let Some(available_codec) = available_codec else {
        return AvifResult::NoCodecAvailable;
    };
    // (only dav1d is in the table with the decode flag: the AOM encoder
    // can't be created here)
    if available_codec.choice != AvifCodecChoice::Dav1d {
        return AvifResult::NoCodecAvailable;
    }
    *codec = Some(super::codec_dav1d::avif_codec_create_dav1d());
    AvifResult::Ok
}

/// Translation of `avifCodecVersions()`.
pub(crate) fn avif_codec_versions() -> String {
    let mut out = String::new();
    for (i, codec) in AVAILABLE_CODECS.iter().enumerate() {
        if i > 0 {
            out += ", ";
        }
        out += codec.name;
        if (codec.flags & (AVIF_CODEC_FLAG_CAN_ENCODE | AVIF_CODEC_FLAG_CAN_DECODE))
            == (AVIF_CODEC_FLAG_CAN_ENCODE | AVIF_CODEC_FLAG_CAN_DECODE)
        {
            out += " [enc/dec]";
        } else if codec.flags & AVIF_CODEC_FLAG_CAN_ENCODE != 0 {
            out += " [enc]";
        } else if codec.flags & AVIF_CODEC_FLAG_CAN_DECODE != 0 {
            out += " [dec]";
        }
        out += ":";
        out += (codec.version)();
    }
    out.truncate(255);
    out
}
