// Rust translation of aom_dsp/sad.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Sums of absolute differences, over any block size here (upstream's
//! `aom_sad<W>x<H>_c()`, `aom_sad_skip_<W>x<H>_c()`, `..x4d_c()` and
//! `..x3d_c()` functions and their `aom_highbd_` versions). The low and
//! high bit depth versions compute the same sums: the samples are kept in
//! 16 bits either way.

/* Sum the difference between every corresponding element of the buffers. */
#[inline]
pub(crate) fn sad(
    a: &[u16],
    a_off: usize,
    a_stride: usize,
    b: &[u16],
    b_off: usize,
    b_stride: usize,
    width: i32,
    height: i32,
) -> u32 {
    let mut sad: u32 = 0;
    let (mut ao, mut bo) = (a_off, b_off);
    let w = width as usize;
    for _y in 0..height {
        let ar = &a[ao..ao + w];
        let br = &b[bo..bo + w];
        for x in 0..w {
            sad = sad.wrapping_add((ar[x] as i32 - br[x] as i32).unsigned_abs());
        }
        ao += a_stride;
        bo += b_stride;
    }
    sad
}

/// `aom_sad<W>x<H>_c()` (and `aom_highbd_sad<W>x<H>_c()`).
#[inline]
pub(crate) fn aom_sad(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    r: &[u16],
    ref_off: usize,
    ref_stride: usize,
    w: i32,
    h: i32,
) -> u32 {
    sad(src, src_off, src_stride, r, ref_off, ref_stride, w, h)
}

/// `aom_sad_skip_<W>x<H>_c()` (and `aom_highbd_sad_skip_<W>x<H>_c()`).
#[inline]
pub(crate) fn aom_sad_skip(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    r: &[u16],
    ref_off: usize,
    ref_stride: usize,
    w: i32,
    h: i32,
) -> u32 {
    2u32.wrapping_mul(sad(
        src,
        src_off,
        2 * src_stride,
        r,
        ref_off,
        2 * ref_stride,
        w,
        h / 2,
    ))
}

/// `aom_sad<W>x<H>x4d_c()` (and the `x3d` versions, which call it).
#[inline]
pub(crate) fn aom_sadx4d(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    r: &[u16],
    ref_offs: &[usize; 4],
    ref_stride: usize,
    w: i32,
    h: i32,
    sad_array: &mut [u32; 4],
) {
    for i in 0..4 {
        sad_array[i] = aom_sad(src, src_off, src_stride, r, ref_offs[i], ref_stride, w, h);
    }
}

/// `aom_sad_skip_<W>x<H>x4d_c()`.
#[inline]
pub(crate) fn aom_sad_skipx4d(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    r: &[u16],
    ref_offs: &[usize; 4],
    ref_stride: usize,
    w: i32,
    h: i32,
    sad_array: &mut [u32; 4],
) {
    for i in 0..4 {
        sad_array[i] = 2u32.wrapping_mul(sad(
            src,
            src_off,
            2 * src_stride,
            r,
            ref_offs[i],
            2 * ref_stride,
            w,
            h / 2,
        ));
    }
}
