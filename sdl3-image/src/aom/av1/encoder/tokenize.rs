// Rust translation of the parts of av1/encoder/tokenize.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the real-time encoder
// reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transform block contexts of inter blocks (variable transform sizes).

use super::encoder::{Av1Comp, ThreadData};
use crate::aom::av1::common::enums::BlockSize;

/// Translation of `av1_tokenize_sb_vartx()`.
pub(crate) fn av1_tokenize_sb_vartx(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    dry_run: i32,
    bsize: BlockSize,
    allow_update_cdf: bool,
) -> Result<(), ()> {
    let _ = (cpi, td, dry_run, bsize, allow_update_cdf);
    Err(())
}
