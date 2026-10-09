// Rust translation of lib/jxl/dec_group.h and lib/jxl/dec_group.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding of the VarDCT groups.

use super::base::{Status, StatusCode};
use super::dec_bit_reader::BitReader;
use super::dec_cache::{GroupDecCache, PassesDecoderState};
use super::render_pipeline::RenderPipelineInput;

/// Translation of `DecodeGroup()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_group(
    _readers: &mut [&mut BitReader<'_>],
    _num_passes: usize,
    _group_idx: usize,
    _dec_state: &mut PassesDecoderState,
    _group_dec_cache: &mut GroupDecCache,
    _thread: usize,
    _render_pipeline_input: &RenderPipelineInput,
    _first_pass: usize,
    _force_draw: bool,
    _dc_only: bool,
    _should_run_pipeline: Option<&mut bool>,
) -> Status {
    Err(StatusCode::GenericError)
}
