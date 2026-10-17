// Rust translation of the declarations of include/FLAC/format.h,
// include/FLAC/stream_decoder.h, src/libFLAC/include/private/*.h and
// src/libFLAC/include/protected/stream_decoder.h from libFLAC 1.3.4
// (https://xiph.org/flac/, at the revision SDL_mixer's external/flac pins).
// Copyright (C) 2000-2009  Josh Coalson
// Copyright (C) 2011-2016  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A Rust translation of the decoding half of libFLAC 1.3.4, as SDL_mixer's
//! CMake builds the copy it vendors (`SDLMIXER_FLAC_LIBFLAC`, on by
//! default, with `WITH_OGG` off, so `FLAC__HAS_OGG` is 0): the stream
//! decoder (stream_decoder.c: metadata parsing, frame sync, frame and
//! subframe decoding, seeking), the bit reader (bitreader.c), the CRCs
//! (crc.c), the fixed and LPC predictors' signal restoration (fixed.c,
//! lpc.c), the bit math (bitmath.c, private/bitmath.h) and the parts of
//! format.c the decoder uses.
//!
//! Only the plain C paths are translated, which are what upstream's build
//! runs on x86-64 too: `FLAC__lpc_restore_signal()` and
//! `FLAC__lpc_restore_signal_wide()` (stream_decoder.c installs no SIMD
//! versions on x86-64; "No useful SSE optimizations yet"), in their
//! `FLAC__OVERFLOW_DETECT` form, which SDL_mixer's default RelWithDebInfo
//! build defines (the wide one stops restoring a block at the first sample
//! that doesn't fit in 32 bits; the overflow messages it prints to stderr
//! aren't), and `FLAC__fixed_restore_signal()`; the bit reader with 32-bit
//! words (`ENABLE_64_BIT_WORDS` is 0). Signed overflow in the predictors,
//! undefined in C, wraps, as GCC's x86-64 code does.
//!
//! Not translated: the encoder (bitwriter.c, stream_encoder*.c, window.c,
//! float.c and the analysis halves of fixed.c and lpc.c), the metadata
//! editing interfaces (metadata_iterators.c, metadata_object.c), the Ogg
//! FLAC layer (ogg_*.c, which the build leaves out:
//! `FLAC__stream_decoder_init_ogg_stream()` returns
//! `FLAC__STREAM_DECODER_INIT_STATUS_UNSUPPORTED_CONTAINER`), the SIMD and
//! assembly versions (cpu.c, *_intrin_*.c, ia32/), md5.c (MD5 checking is
//! off unless asked for, and SDL_mixer never asks), the stdio interfaces
//! (`FLAC__stream_decoder_init_file()` and the like), and the
//! stream_decoder.c functions SDL_mixer doesn't call (the CUESHEET and
//! PICTURE readers, which only run for blocks the client asks for and
//! SDL_mixer asks for none, `FLAC__stream_decoder_process_until_end_of_stream()`,
//! `FLAC__stream_decoder_skip_single_frame()`, the getters it doesn't use).
//! The callbacks and `client_data` are a [`StreamDecoderClient`] passed to
//! each call that may run them.

// The translation keeps upstream's loops, declarations and arithmetic,
// which these lints would have rewritten.
#![allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    clippy::assign_op_pattern,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::field_reassign_with_default,
    clippy::manual_range_contains,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

pub(crate) mod bitmath;
pub(crate) mod bitreader;
pub(crate) mod crc;
pub(crate) mod fixed;
pub(crate) mod format;
pub(crate) mod lpc;
pub(crate) mod stream_decoder;

pub(crate) use stream_decoder::*;

// ---- include/FLAC/format.h ----

/// The largest legal metadata type code.
pub(crate) const FLAC__MAX_METADATA_TYPE_CODE: u32 = 126;

/// The maximum number of channels permitted by the format.
pub(crate) const FLAC__MAX_CHANNELS: usize = 8;

/// The maximum LPC order permitted by the format.
pub(crate) const FLAC__MAX_LPC_ORDER: usize = 32;

/// The maximum order of the fixed predictors permitted by the format.
pub(crate) const FLAC__MAX_FIXED_ORDER: usize = 4;

/// `FLAC__EntropyCodingMethodType`: Residual is coded by partitioning into
/// contexts, each with it's own 4-bit Rice parameter.
pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE: u32 = 0;
/// Residual is coded by partitioning into contexts, each with it's own
/// 5-bit Rice parameter.
pub(crate) const FLAC__ENTROPY_CODING_METHOD_PARTITIONED_RICE2: u32 = 1;

/// Contents of a Rice partitioned residual
/// (`FLAC__EntropyCodingMethod_PartitionedRiceContents`).
#[derive(Default, Debug)]
pub(crate) struct PartitionedRiceContents {
    /// The Rice parameters for each context.
    pub parameters: Vec<u32>,
    /// Widths for escape-coded partitions.  Will be non-zero for escaped
    /// partitions and zero for unescaped partitions.
    pub raw_bits: Vec<u32>,
    /// The capacity of the `parameters` and `raw_bits` arrays specified as
    /// an order, i.e. the number of array elements allocated is
    /// 2 ^ `capacity_by_order`.
    pub capacity_by_order: u32,
}

/// `FLAC__EntropyCodingMethod` (with the partitioned Rice method's order;
/// its contents are the decoder's per-channel
/// [`PartitionedRiceContents`]).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct EntropyCodingMethod {
    pub type_: u32,
    /// The partition order, i.e. # of contexts = 2 ^ `order`.
    pub order: u32,
}

/// `FLAC__SubframeType`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum SubframeType {
    /// constant signal
    #[default]
    Constant,
    /// uncompressed signal
    Verbatim,
    /// fixed polynomial prediction
    Fixed,
    /// linear prediction
    Lpc,
}

/// `FLAC__Subframe_Fixed` (the residual is the decoder's).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SubframeFixed {
    /// The residual coding method.
    pub entropy_coding_method: EntropyCodingMethod,
    /// The polynomial order.
    pub order: u32,
    /// Warmup samples to prime the predictor, length == order.
    pub warmup: [i32; FLAC__MAX_FIXED_ORDER],
}

/// `FLAC__Subframe_LPC` (the residual is the decoder's).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SubframeLpc {
    /// The residual coding method.
    pub entropy_coding_method: EntropyCodingMethod,
    /// The FIR order.
    pub order: u32,
    /// Quantized FIR filter coefficient precision in bits.
    pub qlp_coeff_precision: u32,
    /// The qlp coeff shift needed.
    pub quantization_level: i32,
    /// FIR filter coefficients.
    pub qlp_coeff: [i32; FLAC__MAX_LPC_ORDER],
    /// Warmup samples to prime the predictor, length == order.
    pub warmup: [i32; FLAC__MAX_LPC_ORDER],
}

/// `FLAC__Subframe` (C's union is a field per kind; the verbatim signal is
/// the decoder's residual).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Subframe {
    pub type_: SubframeType,
    /// `data.constant.value`: the constant signal value.
    pub constant_value: i32,
    pub fixed: SubframeFixed,
    pub lpc: SubframeLpc,
    pub wasted_bits: u32,
}

/// `FLAC__ChannelAssignment`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum ChannelAssignment {
    /// independent channels
    #[default]
    Independent = 0,
    /// left+side stereo
    LeftSide = 1,
    /// right+side stereo
    RightSide = 2,
    /// mid+side stereo
    MidSide = 3,
}

/// `FLAC__FrameNumberType`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum FrameNumberType {
    /// number contains the frame number
    #[default]
    FrameNumber,
    /// number contains the sample number of first sample in frame
    SampleNumber,
}

/// `FLAC__FrameHeader`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct FrameHeader {
    /// The number of samples per subframe.
    pub blocksize: u32,
    /// The sample rate in Hz.
    pub sample_rate: u32,
    /// The number of channels (== number of subframes).
    pub channels: u32,
    /// The channel assignment for the frame.
    pub channel_assignment: ChannelAssignment,
    /// The sample resolution.
    pub bits_per_sample: u32,
    /// The numbering scheme used for the frame.  As a convenience, the
    /// decoder will always convert a frame number to a sample number because
    /// the rules are complex.
    pub number_type: FrameNumberType,
    /// The frame number or sample number of first sample in frame;
    /// use the `number_type` value to determine which to use. (C's union
    /// of a 32-bit `frame_number` and a 64-bit `sample_number`: a frame
    /// number is the low 32 bits.)
    pub number: u64,
    // (`crc`, the header's CRC-8, isn't kept: the decoder only checks it.)
}

impl FrameHeader {
    /// `number.sample_number`.
    pub(crate) fn sample_number(&self) -> u64 {
        self.number
    }

    /// `number.frame_number`.
    pub(crate) fn frame_number(&self) -> u32 {
        self.number as u32
    }

    /// `number.frame_number = x` (the union's other 32 bits stay).
    pub(crate) fn set_frame_number(&mut self, x: u32) {
        self.number = (self.number & !0xffff_ffff) | x as u64;
    }
}

/// `FLAC__Frame` (without the `footer`, whose CRC-16 the decoder only
/// checks).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Frame {
    pub header: FrameHeader,
    pub subframes: [Subframe; FLAC__MAX_CHANNELS],
}

/// `FLAC__MetadataType`: STREAMINFO block
pub(crate) const FLAC__METADATA_TYPE_STREAMINFO: u32 = 0;
/// PADDING block
pub(crate) const FLAC__METADATA_TYPE_PADDING: u32 = 1;
/// APPLICATION block
pub(crate) const FLAC__METADATA_TYPE_APPLICATION: u32 = 2;
/// SEEKTABLE block
pub(crate) const FLAC__METADATA_TYPE_SEEKTABLE: u32 = 3;
/// VORBISCOMMENT block (a.k.a. FLAC tags)
pub(crate) const FLAC__METADATA_TYPE_VORBIS_COMMENT: u32 = 4;
/// CUESHEET block
pub(crate) const FLAC__METADATA_TYPE_CUESHEET: u32 = 5;
/// PICTURE block
pub(crate) const FLAC__METADATA_TYPE_PICTURE: u32 = 6;

/// FLAC STREAMINFO structure (`FLAC__StreamMetadata_StreamInfo`).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct StreamInfo {
    pub min_blocksize: u32,
    pub max_blocksize: u32,
    pub min_framesize: u32,
    pub max_framesize: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub bits_per_sample: u32,
    pub total_samples: u64,
    pub md5sum: [u8; 16],
}

/// SeekPoint structure used in SEEKTABLE blocks
/// (`FLAC__StreamMetadata_SeekPoint`).
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct SeekPoint {
    /// The sample number of the target frame.
    pub sample_number: u64,
    /// The offset, in bytes, of the target frame with respect to
    /// beginning of the first frame.
    pub stream_offset: u64,
    /// The number of samples in the target frame.
    pub frame_samples: u32,
}

/// The total stream length of a seek point in bytes.
pub(crate) const FLAC__STREAM_METADATA_SEEKPOINT_LENGTH: u32 = 18;

/// FLAC SEEKTABLE structure (`FLAC__StreamMetadata_SeekTable`). `points`
/// is C's pointer: `None` when it is NULL.
#[derive(Default, Debug)]
pub(crate) struct SeekTable {
    pub num_points: u32,
    pub points: Option<Vec<SeekPoint>>,
}

/// Vorbis comment entry structure used in VORBIS_COMMENT blocks
/// (`FLAC__StreamMetadata_VorbisComment_Entry`). `entry` (`None` for C's
/// NULL) has a NUL after its `length` bytes.
#[derive(Clone, Default, Debug)]
pub(crate) struct VorbisCommentEntry {
    pub length: u32,
    pub entry: Option<Vec<u8>>,
}

/// FLAC VORBIS_COMMENT structure (`FLAC__StreamMetadata_VorbisComment`).
#[derive(Clone, Default, Debug)]
pub(crate) struct VorbisComment {
    pub vendor_string: VorbisCommentEntry,
    pub num_comments: u32,
    pub comments: Option<Vec<VorbisCommentEntry>>,
}

/// The `data` union of `FLAC__StreamMetadata`, for the kinds of block
/// the decoder hands to its client (borrowed from the decoder, as C's
/// metadata callback gets a pointer).
#[derive(Debug)]
#[allow(dead_code)] // (SDL_mixer's client only looks at STREAMINFO and VORBIS_COMMENT blocks)
pub(crate) enum StreamMetadataData<'a> {
    StreamInfo(&'a StreamInfo),
    SeekTable(&'a SeekTable),
    Padding,
    Application { id: [u8; 4], data: Option<&'a [u8]> },
    VorbisComment(&'a VorbisComment),
    Unknown { data: Option<&'a [u8]> },
}

/// FLAC metadata block structure (`FLAC__StreamMetadata`).
#[derive(Debug)]
#[allow(dead_code)] // (likewise)
pub(crate) struct StreamMetadata<'a> {
    /// The type of the metadata block; used determine which member of the
    /// `data` union to dereference.
    pub type_: u32,
    /// `true` if this metadata block is the last, else `false`
    pub is_last: bool,
    /// Length, in bytes, of the block data as it appears in the stream.
    pub length: u32,
    /// Polymorphic block data; use the `type_` value to determine which
    /// to use.
    pub data: StreamMetadataData<'a>,
}
