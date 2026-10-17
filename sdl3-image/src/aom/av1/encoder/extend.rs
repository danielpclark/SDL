// Rust translation of the parts of av1/encoder/extend.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the high bit depth copy of a source frame with extended borders.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Source frame copies with replicated borders (the saver's frames are
//! always high bit depth).

use crate::aom::aom_dsp::aom_dsp_common::align_power_of_two;
use crate::aom::aom_scale::yv12config::*;

/// Translation of `highbd_copy_and_extend_plane()`: `dst` is the whole
/// frame buffer and `dst_off` the plane's offset in it.
#[allow(clippy::too_many_arguments)]
fn highbd_copy_and_extend_plane(
    src: &[u16],
    src_pitch: i32,
    dst: &mut [u16],
    dst_off: usize,
    dst_pitch: i32,
    w: i32,
    h: i32,
    extend_top: i32,
    extend_left: i32,
    extend_bottom: i32,
    extend_right: i32,
) {
    let src_pitch = src_pitch as isize;
    let dst_pitch = dst_pitch as isize;
    let dst_off = dst_off as isize;
    let (w_, el, er) = (w as usize, extend_left as usize, extend_right as usize);

    // copy the left and right most columns out
    let mut src_ptr1: isize = 0;
    let mut src_ptr2: isize = w as isize - 1;
    let mut dst_ptr1: isize = dst_off - extend_left as isize;
    let mut dst_ptr2: isize = dst_off + w as isize;

    for _ in 0..h {
        let s1 = src_ptr1 as usize;
        let d1 = dst_ptr1 as usize;
        let d2 = dst_ptr2 as usize;
        let left = src[s1];
        dst[d1..d1 + el].fill(left);
        dst[d1 + el..d1 + el + w_].copy_from_slice(&src[s1..s1 + w_]);
        let right = src[src_ptr2 as usize];
        dst[d2..d2 + er].fill(right);
        src_ptr1 += src_pitch;
        src_ptr2 += src_pitch;
        dst_ptr1 += dst_pitch;
        dst_ptr2 += dst_pitch;
    }

    // Now copy the top and bottom lines into each line of the respective
    // borders
    let src_ptr1 = dst_off - extend_left as isize;
    let src_ptr2 = dst_off + dst_pitch * (h as isize - 1) - extend_left as isize;
    let mut dst_ptr1 = dst_off + dst_pitch * (-extend_top as isize) - extend_left as isize;
    let mut dst_ptr2 = dst_off + dst_pitch * h as isize - extend_left as isize;
    let linesize = (extend_left + extend_right + w) as usize;
    debug_assert!(linesize as isize <= dst_pitch);

    for _ in 0..extend_top {
        dst.copy_within(
            src_ptr1 as usize..src_ptr1 as usize + linesize,
            dst_ptr1 as usize,
        );
        dst_ptr1 += dst_pitch;
    }

    for _ in 0..extend_bottom {
        dst.copy_within(
            src_ptr2 as usize..src_ptr2 as usize + linesize,
            dst_ptr2 as usize,
        );
        dst_ptr2 += dst_pitch;
    }
}

/// Translation of `av1_copy_and_extend_frame()` (high bit depth): `src`
/// describes the source frame and `src_planes` holds its sample planes,
/// each starting at the plane's first sample.
pub(crate) fn av1_copy_and_extend_frame(
    src: &Yv12BufferConfig,
    src_planes: [&[u16]; 3],
    dst: &mut Yv12BufferConfig,
) {
    // Extend src frame in buffer
    let et_y = dst.border;
    let el_y = dst.border;
    let er_y =
        (src.y_width() + dst.border).max(align_power_of_two(src.y_width(), 6)) - src.y_crop_width();
    let eb_y = (src.y_height() + dst.border).max(align_power_of_two(src.y_height(), 6))
        - src.y_crop_height();
    let uv_width_subsampling = src.subsampling_x;
    let uv_height_subsampling = src.subsampling_y;
    let et_uv = et_y >> uv_height_subsampling;
    let el_uv = el_y >> uv_width_subsampling;
    let eb_uv = eb_y >> uv_height_subsampling;
    let er_uv = er_y >> uv_width_subsampling;

    // (copy_and_extend_plane() and highbd_copy_and_extend_plane() are the
    // same on 16-bit samples; libavif's planes are never NV12)
    let (y_off, u_off, v_off) = (dst.buffers[0], dst.buffers[1], dst.buffers[2]);
    let (y_stride, uv_stride) = (dst.y_stride(), dst.uv_stride());
    highbd_copy_and_extend_plane(
        src_planes[0],
        src.y_stride(),
        &mut dst.buffer_alloc,
        y_off,
        y_stride,
        src.y_crop_width(),
        src.y_crop_height(),
        et_y,
        el_y,
        eb_y,
        er_y,
    );
    if src.monochrome == 0 {
        highbd_copy_and_extend_plane(
            src_planes[1],
            src.uv_stride(),
            &mut dst.buffer_alloc,
            u_off,
            uv_stride,
            src.uv_crop_width(),
            src.uv_crop_height(),
            et_uv,
            el_uv,
            eb_uv,
            er_uv,
        );
        highbd_copy_and_extend_plane(
            src_planes[2],
            src.uv_stride(),
            &mut dst.buffer_alloc,
            v_off,
            uv_stride,
            src.uv_crop_width(),
            src.uv_crop_height(),
            et_uv,
            el_uv,
            eb_uv,
            er_uv,
        );
    }
}
