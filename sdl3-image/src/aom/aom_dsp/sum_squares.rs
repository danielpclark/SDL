// Rust translation of the parts of aom_dsp/sum_squares.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Sums of squares of residuals.

/// Translation of `aom_sum_squares_2d_i16_c()`.
pub(crate) fn aom_sum_squares_2d_i16(
    src: &[i16],
    src_off: usize,
    src_stride: usize,
    width: i32,
    height: i32,
) -> u64 {
    let mut ss: u64 = 0;
    let mut s = src_off;

    for _r in 0..height {
        for c in 0..width as usize {
            let v = src[s + c] as i32;
            ss = ss.wrapping_add((v * v) as u64);
        }
        s += src_stride;
    }

    ss
}
