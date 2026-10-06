// Rust translation of src/webp/decode.h, src/webp/format_constants.h and
// src/webp/mux_types.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Main decoding functions for WebP images: the types and constants of the
//! public headers (the WebP container constants, the VP8X feature flags,
//! colorspaces, status codes, bitstream features and output buffers).
//! The decoding functions are in `dec/webp_dec.rs`.

#![allow(dead_code)] // (the headers' whole constant sets)

//------------------------------------------------------------------------------
// format_constants.h

/// Create fourcc of the chunk from the chunk tag characters.
/// Translation of `MKFOURCC()`.
pub(crate) const fn mkfourcc(tag: &[u8; 4]) -> u32 {
    tag[0] as u32 | (tag[1] as u32) << 8 | (tag[2] as u32) << 16 | (tag[3] as u32) << 24
}

// VP8 related constants.
/// Signature in VP8 data.
pub(crate) const VP8_SIGNATURE: u32 = 0x9d012a;
/// max size of mode partition
pub(crate) const VP8_MAX_PARTITION0_SIZE: u32 = 1 << 19;
/// max size for token partition
pub(crate) const VP8_MAX_PARTITION_SIZE: u32 = 1 << 24;
/// Size of the frame header within VP8 data.
pub(crate) const VP8_FRAME_HEADER_SIZE: usize = 10;

// VP8L related constants.
/// VP8L signature size.
pub(crate) const VP8L_SIGNATURE_SIZE: usize = 1;
/// VP8L signature byte.
pub(crate) const VP8L_MAGIC_BYTE: u32 = 0x2f;
/// Number of bits used to store width and height.
pub(crate) const VP8L_IMAGE_SIZE_BITS: i32 = 14;
/// 3 bits reserved for version.
pub(crate) const VP8L_VERSION_BITS: i32 = 3;
/// version 0
pub(crate) const VP8L_VERSION: u32 = 0;
/// Size of the VP8L frame header.
pub(crate) const VP8L_FRAME_HEADER_SIZE: usize = 5;

pub(crate) const MAX_PALETTE_SIZE: usize = 256;
pub(crate) const MAX_CACHE_BITS: i32 = 11;
pub(crate) const HUFFMAN_CODES_PER_META_CODE: usize = 5;
pub(crate) const ARGB_BLACK: u32 = 0xff000000;

pub(crate) const DEFAULT_CODE_LENGTH: i32 = 8;
pub(crate) const MAX_ALLOWED_CODE_LENGTH: i32 = 15;

pub(crate) const NUM_LITERAL_CODES: i32 = 256;
pub(crate) const NUM_LENGTH_CODES: i32 = 24;
pub(crate) const NUM_DISTANCE_CODES: i32 = 40;
pub(crate) const CODE_LENGTH_CODES: i32 = 19;

/// min number of Huffman bits
pub(crate) const MIN_HUFFMAN_BITS: i32 = 2;
/// max number of Huffman bits
pub(crate) const MAX_HUFFMAN_BITS: i32 = 9;

/// The bit to be written when next data to be read is a transform.
pub(crate) const TRANSFORM_PRESENT: u32 = 1;
/// Maximum number of allowed transform in a bitstream.
pub(crate) const NUM_TRANSFORMS: usize = 4;

/// Translation of `VP8LImageTransformType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum VP8LImageTransformType {
    #[default]
    PredictorTransform = 0,
    CrossColorTransform = 1,
    SubtractGreenTransform = 2,
    ColorIndexingTransform = 3,
}

// Alpha related constants.
pub(crate) const ALPHA_HEADER_LEN: usize = 1;
pub(crate) const ALPHA_NO_COMPRESSION: i32 = 0;
pub(crate) const ALPHA_LOSSLESS_COMPRESSION: i32 = 1;
pub(crate) const ALPHA_PREPROCESSED_LEVELS: i32 = 1;

// Mux related constants.
/// Size of a chunk tag (e.g. "VP8L").
pub(crate) const TAG_SIZE: usize = 4;
/// Size needed to store chunk's size.
pub(crate) const CHUNK_SIZE_BYTES: usize = 4;
/// Size of a chunk header.
pub(crate) const CHUNK_HEADER_SIZE: usize = 8;
/// Size of the RIFF header ("RIFFnnnnWEBP").
pub(crate) const RIFF_HEADER_SIZE: usize = 12;
/// Size of an ANMF chunk.
pub(crate) const ANMF_CHUNK_SIZE: u32 = 16;
/// Size of an ANIM chunk.
pub(crate) const ANIM_CHUNK_SIZE: u32 = 6;
/// Size of a VP8X chunk.
pub(crate) const VP8X_CHUNK_SIZE: u32 = 10;

/// 24-bit max for VP8X width/height.
pub(crate) const MAX_CANVAS_SIZE: u32 = 1 << 24;
/// 32-bit max for width x height.
pub(crate) const MAX_IMAGE_AREA: u64 = 1 << 32;
/// maximum value for loop-count
pub(crate) const MAX_LOOP_COUNT: u32 = 1 << 16;
/// maximum duration
pub(crate) const MAX_DURATION: u32 = 1 << 24;
/// maximum frame x/y offset
pub(crate) const MAX_POSITION_OFFSET: u32 = 1 << 24;

/// Maximum chunk payload is such that adding the header and padding won't
/// overflow a uint32_t.
pub(crate) const MAX_CHUNK_PAYLOAD: u32 = !0u32 - CHUNK_HEADER_SIZE as u32 - 1;

//------------------------------------------------------------------------------
// mux_types.h

// VP8X Feature Flags. Translation of `WebPFeatureFlags`.
pub(crate) const ANIMATION_FLAG: u32 = 0x00000002;
pub(crate) const XMP_FLAG: u32 = 0x00000004;
pub(crate) const EXIF_FLAG: u32 = 0x00000008;
pub(crate) const ALPHA_FLAG: u32 = 0x00000010;
pub(crate) const ICCP_FLAG: u32 = 0x00000020;
pub(crate) const ALL_VALID_FLAGS: u32 = 0x0000003e;

/// Dispose method (animation only). Indicates how the area used by the
/// current frame is to be treated before rendering the next frame on the
/// canvas. Translation of `WebPMuxAnimDispose`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum WebPMuxAnimDispose {
    /// Do not dispose.
    #[default]
    None,
    /// Dispose to background color.
    Background,
}

/// Blend operation (animation only). Indicates how transparent pixels of
/// the current frame are blended with those of the previous canvas.
/// Translation of `WebPMuxAnimBlend`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum WebPMuxAnimBlend {
    /// Blend.
    #[default]
    Blend,
    /// Do not blend.
    NoBlend,
}

//------------------------------------------------------------------------------
// decode.h

/// MAJOR(8b) + MINOR(8b). Translation of `WEBP_DECODER_ABI_VERSION`.
pub(crate) const WEBP_DECODER_ABI_VERSION: i32 = 0x0209;

/// Colorspaces. Note: the naming describes the byte-ordering of packed
/// samples in memory. For instance, MODE_BGRA relates to samples ordered
/// as B,G,R,A,B,G,R,A,... Non-capital names (e.g.:MODE_Argb) relates to
/// pre-multiplied RGB channels. Translation of `WEBP_CSP_MODE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WebpCspMode {
    Rgb = 0,
    Rgba = 1,
    Bgr = 2,
    Bgra = 3,
    Argb = 4,
    Rgba4444 = 5,
    Rgb565 = 6,
    // RGB-premultiplied transparent modes (alpha value is preserved)
    RgbAPremultiplied = 7,
    BgrAPremultiplied = 8,
    ArgbPremultiplied = 9,
    RgbA4444Premultiplied = 10,
    // YUV modes must come after RGB ones.
    /// yuv 4:2:0
    Yuv = 11,
    Yuva = 12,
}

/// The number of colorspaces. Translation of `MODE_LAST`.
pub(crate) const MODE_LAST: usize = 13;

/// Translation of `WebPIsPremultipliedMode()`.
pub(crate) fn webp_is_premultiplied_mode(mode: WebpCspMode) -> bool {
    matches!(
        mode,
        WebpCspMode::RgbAPremultiplied
            | WebpCspMode::BgrAPremultiplied
            | WebpCspMode::ArgbPremultiplied
            | WebpCspMode::RgbA4444Premultiplied
    )
}

/// Translation of `WebPIsAlphaMode()`.
pub(crate) fn webp_is_alpha_mode(mode: WebpCspMode) -> bool {
    matches!(
        mode,
        WebpCspMode::Rgba
            | WebpCspMode::Bgra
            | WebpCspMode::Argb
            | WebpCspMode::Rgba4444
            | WebpCspMode::Yuva
    ) || webp_is_premultiplied_mode(mode)
}

/// Translation of `WebPIsRGBMode()`.
pub(crate) fn webp_is_rgb_mode(mode: WebpCspMode) -> bool {
    (mode as i32) < WebpCspMode::Yuv as i32
}

//------------------------------------------------------------------------------
// WebPDecBuffer: Generic structure for describing the output sample buffer.

/// view as RGBA. Translation of `WebPRGBABuffer`.
#[derive(Debug)]
pub(crate) struct WebPRGBABuffer<'o> {
    /// RGBA samples (`size` is their length)
    pub(crate) rgba: &'o mut [u8],
    /// stride in bytes from one scanline to the next.
    pub(crate) stride: i32,
}

/// Output buffer. Translation of `WebPDecBuffer`, for the RGB(A)
/// colorspaces into external memory (what `WebPDecodeRGBInto()` and
/// friends decode into); the YUV view and the internally allocated
/// memory, which SDL_image's loader doesn't use, are not translated.
#[derive(Debug)]
pub(crate) struct WebPDecBuffer<'o> {
    /// Colorspace.
    pub(crate) colorspace: WebpCspMode,
    /// Dimensions.
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// If non-zero, 'internal_memory' pointer is not used. If value is '2'
    /// or more, the external memory is considered 'slow' and multiple
    /// read/write will be avoided.
    pub(crate) is_external_memory: i32,
    /// Buffer parameters (the union's RGBA view).
    pub(crate) rgba: WebPRGBABuffer<'o>,
}

/// Enumeration of the status codes. Translation of `VP8StatusCode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VP8StatusCode {
    Ok = 0,
    OutOfMemory,
    InvalidParam,
    BitstreamError,
    UnsupportedFeature,
    Suspended,
    UserAbort,
    NotEnoughData,
}

/// Features gathered from the bitstream. Translation of
/// `WebPBitstreamFeatures`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct WebPBitstreamFeatures {
    /// Width in pixels, as read from the bitstream.
    pub(crate) width: i32,
    /// Height in pixels, as read from the bitstream.
    pub(crate) height: i32,
    /// True if the bitstream contains an alpha channel.
    pub(crate) has_alpha: bool,
    /// True if the bitstream is an animation.
    pub(crate) has_animation: bool,
    /// 0 = undefined (/mixed), 1 = lossy, 2 = lossless
    pub(crate) format: i32,
}
