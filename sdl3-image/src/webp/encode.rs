// Rust translation of src/webp/encode.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebP encoder: main interface: the encoding configuration, the picture
//! and the memory writer. The encoding functions are in `enc/`.
//!
//! A `WebPPicture`'s planes are `Vec`s (empty where upstream's pointers
//! are NULL), each its own allocation where upstream carves the YUVA
//! planes out of one (`memory_`); views of other pictures
//! (`WebPPictureView()`) are not translated, their users copying the
//! pixels in and out instead. The writer is always the memory writer (or
//! none: upstream's default `DummyWriter()`), and the error code is a
//! `Cell`, as upstream sets it through `const` pointers. The statistics,
//! the extra info and the progress hook are not translated: SDL_image
//! doesn't use them.

#![allow(dead_code)] // (the header's whole constant sets)

use std::cell::{Cell, RefCell};

/// MAJOR(8b) + MINOR(8b). Translation of `WEBP_ENCODER_ABI_VERSION`.
pub(crate) const WEBP_ENCODER_ABI_VERSION: i32 = 0x020f;

//------------------------------------------------------------------------------
// Coding parameters

/// Image characteristics hint for the underlying encoder. Translation of
/// `WebPImageHint`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum WebPImageHint {
    /// default preset.
    #[default]
    Default = 0,
    /// digital picture, like portrait, inner shot
    Picture,
    /// outdoor photograph, with natural lighting
    Photo,
    /// Discrete tone image (graph, map-tile etc).
    Graph,
}

/// Compression parameters. Translation of `WebPConfig`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WebPConfig {
    /// Lossless encoding (0=lossy(default), 1=lossless).
    pub(crate) lossless: i32,
    /// between 0 and 100. For lossy, 0 gives the smallest
    /// size and 100 the largest. For lossless, this
    /// parameter is the amount of effort put into the
    /// compression: 0 is the fastest but gives larger
    /// files compared to the slowest, but best, 100.
    pub(crate) quality: f32,
    /// quality/speed trade-off (0=fast, 6=slower-better)
    pub(crate) method: i32,

    /// Hint for image type (lossless only for now).
    pub(crate) image_hint: WebPImageHint,

    /// if non-zero, set the desired target size in bytes.
    /// Takes precedence over the 'compression' parameter.
    pub(crate) target_size: i32,
    /// if non-zero, specifies the minimal distortion to
    /// try to achieve. Takes precedence over target_size.
    pub(crate) target_psnr: f32,
    /// maximum number of segments to use, in [1..4]
    pub(crate) segments: i32,
    /// Spatial Noise Shaping. 0=off, 100=maximum.
    pub(crate) sns_strength: i32,
    /// range: [0 = off .. 100 = strongest]
    pub(crate) filter_strength: i32,
    /// range: [0 = off .. 7 = least sharp]
    pub(crate) filter_sharpness: i32,
    /// filtering type: 0 = simple, 1 = strong (only used
    /// if filter_strength > 0 or autofilter > 0)
    pub(crate) filter_type: i32,
    /// Auto adjust filter's strength [0 = off, 1 = on]
    pub(crate) autofilter: i32,
    /// Algorithm for encoding the alpha plane (0 = none,
    /// 1 = compressed with WebP lossless). Default is 1.
    pub(crate) alpha_compression: i32,
    /// Predictive filtering method for alpha plane.
    ///  0: none, 1: fast, 2: best. Default if 1.
    pub(crate) alpha_filtering: i32,
    /// Between 0 (smallest size) and 100 (lossless).
    /// Default is 100.
    pub(crate) alpha_quality: i32,
    /// number of entropy-analysis passes (in [1..10]).
    pub(crate) pass: i32,

    /// if true, export the compressed picture back.
    /// In-loop filtering is not applied.
    pub(crate) show_compressed: i32,
    /// preprocessing filter:
    /// 0=none, 1=segment-smooth, 2=pseudo-random dithering
    pub(crate) preprocessing: i32,
    /// log2(number of token partitions) in [0..3]. Default
    /// is set to 0 for easier progressive decoding.
    pub(crate) partitions: i32,
    /// quality degradation allowed to fit the 512k limit
    /// on prediction modes coding (0: no degradation,
    /// 100: maximum possible degradation).
    pub(crate) partition_limit: i32,
    /// If true, compression parameters will be remapped
    /// to better match the expected output size from
    /// JPEG compression. Generally, the output size will
    /// be similar but the degradation will be lower.
    pub(crate) emulate_jpeg_size: i32,
    /// If non-zero, try and use multi-threaded encoding.
    pub(crate) thread_level: i32,
    /// If set, reduce memory usage (but increase CPU use).
    pub(crate) low_memory: i32,

    /// Near lossless encoding [0 = max loss .. 100 = off
    /// (default)].
    pub(crate) near_lossless: i32,
    /// if non-zero, preserve the exact RGB values under
    /// transparent area. Otherwise, discard this invisible
    /// RGB information for better compression. The default
    /// value is 0.
    pub(crate) exact: i32,

    /// reserved for future lossless feature
    pub(crate) use_delta_palette: i32,
    /// if needed, use sharp (and slow) RGB->YUV conversion
    pub(crate) use_sharp_yuv: i32,

    /// minimum permissible quality factor
    pub(crate) qmin: i32,
    /// maximum permissible quality factor
    pub(crate) qmax: i32,
}

/// Enumerate some predefined settings for WebPConfig, depending on the
/// type of source picture. These presets are used when calling
/// WebPConfigPreset(). Translation of `WebPPreset`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WebPPreset {
    /// default preset.
    Default = 0,
    /// digital picture, like portrait, inner shot
    Picture,
    /// outdoor photograph, with natural lighting
    Photo,
    /// hand or line drawing, with high-contrast details
    Drawing,
    /// small-sized colorful images
    Icon,
    /// text-like
    Text,
}

//------------------------------------------------------------------------------
// Output

/// WebPMemoryWriter: utility to write the encoded bytes to memory.
/// Translation of `WebPMemoryWriter` (`mem` holds `size` bytes; the
/// capacity is the `Vec`'s).
#[derive(Clone, Default, Debug)]
pub(crate) struct WebPMemoryWriter {
    /// final buffer
    pub(crate) mem: Vec<u8>,
}

/// Color spaces. Translation of `WebPEncCSP`.
/// 4:2:0
pub(crate) const WEBP_YUV420: i32 = 0;
/// alpha channel variant
pub(crate) const WEBP_YUV420A: i32 = 4;
/// bit-mask to get the UV sampling factors
pub(crate) const WEBP_CSP_UV_MASK: i32 = 3;
/// bit that is set if alpha is present
pub(crate) const WEBP_CSP_ALPHA_BIT: i32 = 4;

/// Encoding error conditions. Translation of `WebPEncodingError`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum WebPEncodingError {
    #[default]
    Ok = 0,
    /// memory error allocating objects
    OutOfMemory,
    /// memory error while flushing bits
    BitstreamOutOfMemory,
    /// a pointer parameter is NULL
    NullParameter,
    /// configuration is invalid
    InvalidConfiguration,
    /// picture has invalid width/height
    BadDimension,
    /// partition is bigger than 512k
    Partition0Overflow,
    /// partition is bigger than 16M
    PartitionOverflow,
    /// error while flushing bytes
    BadWrite,
    /// file is bigger than 4G
    FileTooBig,
    /// abort request by user
    UserAbort,
}

/// maximum width/height allowed (inclusive), in pixels. Translation of
/// `WEBP_MAX_DIMENSION`.
pub(crate) const WEBP_MAX_DIMENSION: i32 = 16383;

/// Main exchange structure (input samples, output bytes, statistics).
/// Translation of `WebPPicture`.
#[derive(Clone, Debug, Default)]
pub(crate) struct WebPPicture {
    //   INPUT
    //////////////
    /// Main flag for encoder selecting between ARGB or YUV input.
    /// It is recommended to use ARGB input (*argb, argb_stride) for
    /// lossless compression, and YUV input (*y, *u, *v, etc.) for lossy
    /// compression since these are the respective native colorspace for
    /// these formats.
    pub(crate) use_argb: bool,

    // YUV input (mostly used for input to lossy compression)
    /// colorspace: should be YUV420 for now (=Y'CbCr).
    pub(crate) colorspace: i32,
    /// dimensions (less or equal to WEBP_MAX_DIMENSION)
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// luma plane.
    pub(crate) y: Vec<u8>,
    /// chroma planes.
    pub(crate) u: Vec<u8>,
    pub(crate) v: Vec<u8>,
    /// luma/chroma strides.
    pub(crate) y_stride: i32,
    pub(crate) uv_stride: i32,
    /// the alpha plane
    pub(crate) a: Vec<u8>,
    /// stride of the alpha plane
    pub(crate) a_stride: i32,

    // ARGB input (mostly used for input to lossless compression)
    /// argb (32 bit) plane.
    pub(crate) argb: Vec<u32>,
    /// This is stride in pixels units, not bytes.
    pub(crate) argb_stride: i32,

    //   OUTPUT
    ///////////////
    /// Byte-emission hook, to store compressed bytes as they are ready:
    /// the memory writer (`WebPMemoryWrite()` with `custom_ptr`), or none
    /// (`DummyWriter()`).
    pub(crate) writer: RefCell<Option<WebPMemoryWriter>>,

    /// Error code for the latest error encountered during encoding
    pub(crate) error_code: Cell<WebPEncodingError>,
}
