// Rust translation of aom_dsp/subtract.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The residual of high bit depth blocks.

/// Translation of `aom_highbd_subtract_block_c()`.
pub(crate) fn aom_highbd_subtract_block(
    rows: i32,
    cols: i32,
    diff: &mut [i16],
    diff_off: usize,
    diff_stride: usize,
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    pred: &[u16],
    pred_off: usize,
    pred_stride: usize,
) {
    let (mut d, mut s, mut p) = (diff_off, src_off, pred_off);
    for _r in 0..rows {
        for c in 0..cols as usize {
            diff[d + c] = (src[s + c] as i32 - pred[p + c] as i32) as i16;
        }

        d += diff_stride;
        p += pred_stride;
        s += src_stride;
    }
}
