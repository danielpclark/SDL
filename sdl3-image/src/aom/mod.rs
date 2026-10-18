// Rust translation of the parts of aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) SDL_image's libavif reaches to encode AVIF images.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! aom, the AV1 encoder SDL_image's libavif uses, as SDL_image builds it
//! (the encoder only, high bit depth on), for the configurations libavif
//! sets for SDL_image's saver (all intra, one frame) and animation encoder
//! (real-time usage at cpu-used 9, CBR rate control, key frames and inter
//! frames from the LAST, GOLDEN and ALTREF references, the non-RD mode
//! search, with or without row multi-threading).

// (the translation keeps upstream's loops over indices, argument lists,
// late initializations, explicit arithmetic and switch layouts as they are,
// and its model tables' literals as they are written)
#![allow(
    dead_code,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::eq_op,
    clippy::erasing_op,
    clippy::excessive_precision,
    clippy::explicit_counter_loop,
    clippy::identity_op,
    clippy::if_same_then_else,
    clippy::let_and_return,
    clippy::manual_is_multiple_of,
    clippy::manual_memcpy,
    clippy::manual_range_contains,
    clippy::manual_swap,
    clippy::module_inception,
    clippy::neg_multiply,
    clippy::needless_bool_assign,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::needless_return,
    clippy::precedence,
    clippy::too_many_arguments,
    clippy::unnecessary_cast
)]

pub(crate) mod aom_dsp;
pub(crate) mod aom_integer;
pub(crate) mod aom_scale;
pub(crate) mod av1;

#[cfg(test)]
mod tests;
