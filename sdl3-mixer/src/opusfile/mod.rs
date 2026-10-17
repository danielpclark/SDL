// Rust translation of the declarations of include/opusfile.h and
// src/internal.h from opusfile 0.12 (https://opus-codec.org, at the revision
// SDL_mixer's external/opusfile pins).
// THE libopusfile SOURCE CODE IS (C) COPYRIGHT 1994-2020
// by the Xiph.Org Foundation and contributors https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A Rust translation of opusfile 0.12 as SDL_mixer builds it (float API,
//! `OP_DISABLE_HTTP`, no `OP_FIXED_POINT`): opening, seeking and decoding
//! Ogg Opus streams, chained ones included, from an
//! [`IoStream`](sdl3::io::IoStream) with SDL_mixer's callbacks
//! (`OPUS_IoCallbacks` in decoder_opus.c). Only what SDL_mixer's Opus
//! decoder calls is translated; not the int16 API (`op_read()`,
//! `op_read_stereo()` and their dithering and downmixing), the decode
//! callback (SDL_mixer sets none), picture tags, `opus_tags_add()` and the
//! like, the stdio/memory/HTTP streams of stream.c and http.c, or
//! `op_test()`.

// The translation keeps upstream's loops, declarations and arithmetic,
// which these lints would have rewritten.
#![allow(
    clippy::assign_op_pattern,
    clippy::field_reassign_with_default,
    clippy::int_plus_one,
    clippy::manual_range_contains,
    clippy::needless_late_init,
    clippy::needless_range_loop
)]

pub(crate) mod info;
pub(crate) mod internal;
#[allow(clippy::module_inception)]
pub(crate) mod opusfile;

/*A request did not succeed.*/
pub(crate) const OP_FALSE: i32 = -1;
/*Currently not used externally.*/
pub(crate) const OP_EOF: i32 = -2;
/*There was a hole in the page sequence numbers (e.g., a page was corrupt or
missing).*/
pub(crate) const OP_HOLE: i32 = -3;
/*An underlying read, seek, or tell operation failed when it should have
succeeded.*/
pub(crate) const OP_EREAD: i32 = -128;
/*A <code>NULL</code> pointer was passed where one was unexpected, or an
internal memory allocation failed, or an internal library error was
encountered.*/
pub(crate) const OP_EFAULT: i32 = -129;
/*The stream used a feature that is not implemented, such as an unsupported
channel family.*/
pub(crate) const OP_EIMPL: i32 = -130;
/*One or more parameters to a function were invalid.*/
pub(crate) const OP_EINVAL: i32 = -131;
/*A purported Ogg Opus stream did not begin with an Ogg page, a purported
header packet did not start with one of the required strings, "OpusHead" or
"OpusTags", or a link in a chained file was encountered that did not
contain any logical Opus streams.*/
pub(crate) const OP_ENOTFORMAT: i32 = -132;
/*A required header packet was not properly formatted, contained illegal
values, or was missing altogether.*/
pub(crate) const OP_EBADHEADER: i32 = -133;
/*The ID header contained an unrecognized version number.*/
pub(crate) const OP_EVERSION: i32 = -134;
/*Currently not used at all.*/
pub(crate) const OP_ENOTAUDIO: i32 = -135;
/*An audio packet failed to decode properly.
This is usually caused by a multistream Ogg packet where the durations of
the individual Opus packets contained in it are not all the same.*/
pub(crate) const OP_EBADPACKET: i32 = -136;
/*We failed to find data we had seen before, or the bitstream structure was
sufficiently malformed that seeking to the target destination was
impossible.*/
pub(crate) const OP_EBADLINK: i32 = -137;
/*An operation that requires seeking was requested on an unseekable stream.*/
pub(crate) const OP_ENOSEEK: i32 = -138;
/*The first or last granule position of a link failed basic validity checks.*/
pub(crate) const OP_EBADTIMESTAMP: i32 = -139;

/*The maximum number of channels in an Ogg Opus stream.*/
pub(crate) const OPUS_CHANNEL_COUNT_MAX: usize = 255;

/*Gain offset type that indicates that the provided offset is relative to the
header gain.
This is the default.*/
pub(crate) const OP_HEADER_GAIN: i32 = 0;
/*Gain offset type that indicates that the provided offset is relative to the
R128_ALBUM_GAIN value (if any), in addition to the header gain.*/
pub(crate) const OP_ALBUM_GAIN: i32 = 3007;
/*Gain offset type that indicates that the provided offset is relative to the
R128_TRACK_GAIN value (if any), in addition to the header gain.*/
pub(crate) const OP_TRACK_GAIN: i32 = 3008;
/*Gain offset type that indicates that the provided offset should be used as
the gain directly, without applying any the header or track gains.*/
pub(crate) const OP_ABSOLUTE_GAIN: i32 = 3009;

/// Translation of `struct OpusHead`: the Opus ID header.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpusHead {
    /*The Ogg Opus format version, in the range 0...255.*/
    pub version: i32,
    /*The number of channels, in the range 1...255.*/
    pub channel_count: i32,
    /*The number of samples that should be discarded from the beginning of the
    stream.*/
    pub pre_skip: u32,
    /*The sampling rate of the original input.*/
    pub input_sample_rate: u32,
    /*The gain to apply to the decoded output, in dB, as a Q8 value in the range
    -32768...32767.*/
    pub output_gain: i32,
    /*The channel mapping family, in the range 0...255.*/
    pub mapping_family: i32,
    /*The number of Opus streams in each Ogg packet, in the range 1...255.*/
    pub stream_count: i32,
    /*The number of coupled Opus streams in each Ogg packet, in the range
    0...127.*/
    pub coupled_count: i32,
    /*The mapping from coded stream channels to output channels.*/
    pub mapping: [u8; OPUS_CHANNEL_COUNT_MAX],
}

impl Default for OpusHead {
    fn default() -> Self {
        OpusHead {
            version: 0,
            channel_count: 0,
            pre_skip: 0,
            input_sample_rate: 0,
            output_gain: 0,
            mapping_family: 0,
            stream_count: 0,
            coupled_count: 0,
            mapping: [0; OPUS_CHANNEL_COUNT_MAX],
        }
    }
}

/// Translation of `struct OpusTags`: the comment header. C's
/// `user_comments`/`comment_lengths` pairs are `user_comments` (the
/// comments, as stored, without C's added NUL), and the binary suffix C
/// keeps after them is `binary_suffix`.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpusTags {
    /*The array of comment string vectors.*/
    pub user_comments: Vec<Vec<u8>>,
    /*(The binary metadata after the comments, if any.)*/
    pub binary_suffix: Vec<u8>,
    /*The total number of comment streams.*/
    pub comments: i32,
    /*The null-terminated vendor string.
    This identifies the software used to encode the stream.*/
    pub vendor: Vec<u8>,
}
