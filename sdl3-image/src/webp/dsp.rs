// Rust translation of src/dsp/dsp.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the parts the decoder needs.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Speed-critical functions: the plain-C versions only. Upstream picks SSE2,
//! SSE4.1, NEON, MIPS or MSA versions at run time where the CPU has them;
//! they give the same results, and none is translated. The function
//! pointer tables (`VP8PredLuma4[]`, `WebPUnfilters[]`, ...) are `match`es
//! on the mode.

pub(crate) mod alpha_processing;
pub(crate) mod dec;
pub(crate) mod filters;
pub(crate) mod lossless;
pub(crate) mod upsampling;
pub(crate) mod yuv;

/// this is the common stride for enc/dec. Translation of `BPS`.
pub(crate) const BPS: usize = 32;

/// Filter types. Translation of `WEBP_FILTER_TYPE` (the meta-types
/// `WEBP_FILTER_BEST` and `WEBP_FILTER_FAST` are the encoder's).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum WebpFilterType {
    #[default]
    None = 0,
    Horizontal,
    Vertical,
    Gradient,
}

/// end marker. Translation of `WEBP_FILTER_LAST`.
#[allow(dead_code)]
pub(crate) const WEBP_FILTER_LAST: i32 = WebpFilterType::Gradient as i32 + 1;
