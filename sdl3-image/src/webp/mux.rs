// Rust translation of src/webp/mux.h and src/mux/muxi.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), and the module of its src/mux/ files.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! RIFF container manipulation and encoding for WebP images: the muxer
//! and the animation encoder.
//!
//! The chunk lists and the image list are vectors, and every chunk owns
//! its data (upstream's `copy_data` choice only decides whether the data
//! is borrowed or copied, which makes no difference to the bytes). The
//! APIs SDL_image doesn't use (`WebPMuxGetChunk()`, `WebPMuxDeleteChunk()`,
//! `WebPMuxDeleteFrame()`, `WebPAnimEncoderRefineRect()`, the encoder
//! options' sanitizer for caller-given options) are not translated.

pub(crate) mod anim_encode;
pub(crate) mod muxedit;
pub(crate) mod muxinternal;
pub(crate) mod muxread;

use crate::webp::decode::{WebPMuxAnimBlend, WebPMuxAnimDispose};

//------------------------------------------------------------------------------
// mux.h

/// Error codes. Translation of `WebPMuxError`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WebPMuxError {
    Ok = 1,
    NotFound = 0,
    InvalidArgument = -1,
    BadData = -2,
    MemoryError = -3,
    NotEnoughData = -4,
}

/// IDs for different types of chunks. Translation of `WebPChunkId`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum WebPChunkId {
    /// VP8X
    Vp8x,
    /// ICCP
    Iccp,
    /// ANIM
    Anim,
    /// ANMF
    Anmf,
    /// (deprecated from FRGM)
    #[allow(dead_code)]
    Deprecated,
    /// ALPH
    Alpha,
    /// VP8/VP8L
    Image,
    /// EXIF
    Exif,
    /// XMP
    Xmp,
    /// Other chunks.
    Unknown,
    #[default]
    Nil,
}

/// Encapsulates data about a single frame. Translation of
/// `WebPMuxFrameInfo`.
#[derive(Clone, Default, Debug)]
pub(crate) struct WebPMuxFrameInfo {
    /// image data: can be a raw VP8/VP8L bitstream
    /// or a single-image WebP file.
    pub(crate) bitstream: Vec<u8>,
    /// x-offset of the frame.
    pub(crate) x_offset: i32,
    /// y-offset of the frame.
    pub(crate) y_offset: i32,
    /// duration of the frame (in milliseconds).
    pub(crate) duration: i32,

    /// frame type: should be one of WEBP_CHUNK_ANMF
    /// or WEBP_CHUNK_IMAGE
    pub(crate) id: WebPChunkId,
    /// Disposal method for the frame.
    pub(crate) dispose_method: WebPMuxAnimDispose,
    /// Blend operation for the frame.
    pub(crate) blend_method: WebPMuxAnimBlend,
}

/// Animation parameters. Translation of `WebPMuxAnimParams`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct WebPMuxAnimParams {
    /// Background color of the canvas stored (in MSB order) as:
    /// Bits 00 to 07: Alpha.
    /// Bits 08 to 15: Red.
    /// Bits 16 to 23: Green.
    /// Bits 24 to 31: Blue.
    pub(crate) bgcolor: u32,
    /// Number of times to repeat the animation [0 = infinite].
    pub(crate) loop_count: i32,
}

/// Global options. Translation of `WebPAnimEncoderOptions`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct WebPAnimEncoderOptions {
    /// Animation parameters.
    pub(crate) anim_params: WebPMuxAnimParams,
    /// If true, minimize the output size (slow). Implicitly
    /// disables key-frame insertion.
    pub(crate) minimize_size: bool,
    /// Minimum and maximum distance between consecutive key
    /// frames in the output. The library may insert some key
    /// frames as needed to satisfy this criteria.
    /// Note that these conditions should hold: kmax > kmin
    /// and kmin >= kmax / 2 + 1. Also, if kmax <= 0, then
    /// key-frame insertion is disabled; and if kmax == 1,
    /// then all frames will be key-frames (kmin value does
    /// not matter for these special cases).
    pub(crate) kmin: i32,
    pub(crate) kmax: i32,
    /// If true, use mixed compression mode; may choose
    /// either lossy and lossless for each frame.
    pub(crate) allow_mixed: bool,
    /// If true, print info and warning messages to stderr.
    pub(crate) verbose: bool,
}

//------------------------------------------------------------------------------
// muxi.h

/// Chunk object. Translation of `WebPChunk` (in its list, without
/// `next_`; the chunk always owns its data).
#[derive(Clone, Default, Debug)]
pub(crate) struct WebPChunk {
    pub(crate) tag: u32,
    pub(crate) data: Vec<u8>,
}

/// MuxImage object. Store a full WebP image (including ANMF chunk, ALPH
/// chunk and VP8/VP8L chunk). Translation of `WebPMuxImage`.
#[derive(Clone, Default, Debug)]
pub(crate) struct WebPMuxImage {
    /// Corresponds to WEBP_CHUNK_ANMF.
    pub(crate) header: Option<WebPChunk>,
    /// Corresponds to WEBP_CHUNK_ALPHA.
    pub(crate) alpha: Option<WebPChunk>,
    /// Corresponds to WEBP_CHUNK_IMAGE.
    pub(crate) img: Option<WebPChunk>,
    /// Corresponds to WEBP_CHUNK_UNKNOWN.
    pub(crate) unknown: Vec<WebPChunk>,
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// Through ALPH chunk or as part of VP8L.
    pub(crate) has_alpha: bool,
    /// True if only some of the chunks are filled.
    pub(crate) is_partial: bool,
}

/// Main mux object. Stores data chunks. Translation of `WebPMux`.
#[derive(Clone, Default, Debug)]
pub(crate) struct WebPMux {
    pub(crate) images: Vec<WebPMuxImage>,
    pub(crate) iccp: Vec<WebPChunk>,
    pub(crate) exif: Vec<WebPChunk>,
    pub(crate) xmp: Vec<WebPChunk>,
    pub(crate) anim: Vec<WebPChunk>,
    pub(crate) vp8x: Vec<WebPChunk>,

    pub(crate) unknown: Vec<WebPChunk>,
    pub(crate) canvas_width: i32,
    pub(crate) canvas_height: i32,
}

/// CHUNK_INDEX enum: used for indexing within 'kChunks' (defined below) only.
/// Note: the reason for having two enums ('WebPChunkId' and 'CHUNK_INDEX') is to
/// allow two different chunks to have the same id (e.g. WebPChunkId
/// 'WEBP_CHUNK_IMAGE' can correspond to CHUNK_INDEX 'IDX_VP8' or 'IDX_VP8L').
/// Translation of `CHUNK_INDEX`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ChunkIndex {
    Vp8x = 0,
    Iccp,
    Anim,
    Anmf,
    Alpha,
    Vp8,
    Vp8l,
    Exif,
    Xmp,
    Unknown,

    Nil,
}

/// To signal void chunk.
pub(crate) const NIL_TAG: u32 = 0x00000000;

/// Translation of `ChunkInfo`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChunkInfo {
    pub(crate) tag: u32,
    pub(crate) id: WebPChunkId,
    pub(crate) size: u32,
}

/// Check if given ID corresponds to an image related chunk.
/// Translation of `IsWPI()`.
pub(crate) fn is_wpi(id: WebPChunkId) -> bool {
    matches!(
        id,
        WebPChunkId::Anmf | WebPChunkId::Alpha | WebPChunkId::Image
    )
}
