// Rust translation of the error codes and constants of include/opus_defines.h
// and src/opus_private.h from libopus 1.4 (https://opus-codec.org, at the
// revision SDL_mixer's external/opus pins).
// Copyright (c) 2010-2011 Xiph.Org Foundation, Skype Limited
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A Rust translation of libopus 1.4's decoder, as SDL_mixer builds it
//! (float, `OPUS_FIXED_POINT` off, `OPUS_FLOAT_APPROX` off, no custom
//! modes, hardening on: `celt_assert()` becomes `debug_assert!()`). The
//! plain C code paths are translated, as with `OPUS_DISABLE_INTRINSICS`; none
//! of the SSE/NEON/assembly variants. Only what opusfile's float API reaches
//! is here: the multistream decoder, the single stream decoder (SILK, CELT
//! and hybrid, with their packet loss concealment, FEC and mode transitions)
//! and packet parsing. The encoders, the projection (ambisonics) decoder,
//! the repacketizer and `opus_pcm_soft_clip()` (used by the int16 API only)
//! are not translated.

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

pub(crate) mod celt;
pub(crate) mod multistream;
pub(crate) mod opus_decoder;
pub(crate) mod packet;
pub(crate) mod silk;

/* No error @hideinitializer*/
pub(crate) const OPUS_OK: i32 = 0;
/* One or more invalid/out of range arguments @hideinitializer*/
pub(crate) const OPUS_BAD_ARG: i32 = -1;
/* Not enough bytes allocated in the buffer @hideinitializer*/
pub(crate) const OPUS_BUFFER_TOO_SMALL: i32 = -2;
/* An internal error was detected @hideinitializer*/
pub(crate) const OPUS_INTERNAL_ERROR: i32 = -3;
/* The compressed data passed is corrupted @hideinitializer*/
pub(crate) const OPUS_INVALID_PACKET: i32 = -4;
/* Invalid/unsupported request number @hideinitializer*/
#[allow(dead_code)]
pub(crate) const OPUS_UNIMPLEMENTED: i32 = -5;
/* An encoder or decoder structure is invalid or already freed @hideinitializer*/
#[allow(dead_code)]
pub(crate) const OPUS_INVALID_STATE: i32 = -6;
/* Memory allocation has failed @hideinitializer*/
pub(crate) const OPUS_ALLOC_FAIL: i32 = -7;

pub(crate) const OPUS_BANDWIDTH_NARROWBAND: i32 = 1101; /* 4 kHz bandpass @hideinitializer*/
pub(crate) const OPUS_BANDWIDTH_MEDIUMBAND: i32 = 1102; /* 6 kHz bandpass @hideinitializer*/
pub(crate) const OPUS_BANDWIDTH_WIDEBAND: i32 = 1103; /* 8 kHz bandpass @hideinitializer*/
pub(crate) const OPUS_BANDWIDTH_SUPERWIDEBAND: i32 = 1104; /*12 kHz bandpass @hideinitializer*/
pub(crate) const OPUS_BANDWIDTH_FULLBAND: i32 = 1105; /*20 kHz bandpass @hideinitializer*/

pub(crate) const MODE_SILK_ONLY: i32 = 1000;
pub(crate) const MODE_HYBRID: i32 = 1001;
pub(crate) const MODE_CELT_ONLY: i32 = 1002;
