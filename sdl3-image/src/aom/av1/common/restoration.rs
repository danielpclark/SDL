// Rust translation of the parts of av1/common/restoration.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loop restoration: the saver never enables it, so only the query of the
//! restoration units a superblock covers (none) is here.

use super::av1_common_int::Av1Common;
use super::enums::*;

/// Translation of `av1_loop_restoration_corners_in_sb()`, which returns 0
/// for every frame the saver codes (no plane uses loop restoration).
pub(crate) fn av1_loop_restoration_corners_in_sb(
    cm: &Av1Common,
    plane: usize,
    bsize: BlockSize,
) -> i32 {
    if bsize != cm.seq_params.sb_size {
        return 0;
    }
    if cm.rst_info[plane].frame_restoration_type == RESTORE_NONE {
        return 0;
    }
    debug_assert!(false, "loop restoration is never enabled");
    0
}
