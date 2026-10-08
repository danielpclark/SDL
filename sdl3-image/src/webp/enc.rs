// The encoder of libwebp (https://chromium.googlesource.com/webm/libwebp,
// as SDL_image's external/libwebp pins it): the module of its src/enc/
// files.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encoder: the lossless (VP8L) encoder with its backward references,
//! histograms, transforms and Huffman coding, and the pictures and their
//! colorspace conversions.

pub(crate) mod backward_references_cost_enc;
pub(crate) mod backward_references_enc;
pub(crate) mod config_enc;
pub(crate) mod histogram_enc;
pub(crate) mod picture_csp_enc;
pub(crate) mod picture_enc;
pub(crate) mod picture_tools_enc;
pub(crate) mod predictor_enc;
pub(crate) mod vp8l_enc;
pub(crate) mod webp_enc;

use crate::webp::enc::picture_enc::webp_encoding_set_error;
use crate::webp::encode::{WebPConfig, WebPEncodingError, WebPPicture};

/// The lossy encoder (not translated yet).
pub(crate) fn vp8_encode_lossy(_config: &WebPConfig, pic: &mut WebPPicture) -> bool {
    webp_encoding_set_error(pic, WebPEncodingError::InvalidConfiguration)
}
