// The encoder of libwebp (https://chromium.googlesource.com/webm/libwebp,
// as SDL_image's external/libwebp pins it): the module of its src/enc/
// files.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encoder: the lossy (VP8) encoder with its analysis, mode decision,
//! quantization, token coding and alpha compression, the lossless (VP8L)
//! encoder with its backward references, histograms, transforms and
//! Huffman coding, and the pictures and their colorspace conversions.

pub(crate) mod alpha_enc;
pub(crate) mod analysis_enc;
pub(crate) mod backward_references_cost_enc;
pub(crate) mod backward_references_enc;
pub(crate) mod config_enc;
pub(crate) mod cost_enc;
pub(crate) mod filter_enc;
pub(crate) mod frame_enc;
pub(crate) mod histogram_enc;
pub(crate) mod iterator_enc;
pub(crate) mod picture_csp_enc;
pub(crate) mod picture_enc;
pub(crate) mod picture_tools_enc;
pub(crate) mod predictor_enc;
pub(crate) mod quant_enc;
pub(crate) mod syntax_enc;
pub(crate) mod token_enc;
pub(crate) mod tree_enc;
pub(crate) mod vp8i_enc;
pub(crate) mod vp8l_enc;
pub(crate) mod webp_enc;
