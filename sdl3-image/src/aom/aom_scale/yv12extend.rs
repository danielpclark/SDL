// Rust translation of the parts of aom_scale/generic/yv12extend.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the encoder reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Border extension of frame buffers (`extend_plane()` and
//! `extend_plane_high()` are the same on 16-bit samples).

use super::yv12config::Yv12BufferConfig;

/// Translation of `extend_plane()` / `extend_plane_high()`; `src` is the
/// plane's offset in `buf`.
fn extend_plane(
    buf: &mut [u16],
    src: usize,
    src_stride: i32,
    width: i32,
    height: i32,
    extend_top: i32,
    extend_left: i32,
    extend_bottom: i32,
    extend_right: i32,
    v_start: i32,
    v_end: i32,
) {
    let linesize = (extend_left + extend_right + width) as usize;
    debug_assert!(linesize as i32 <= src_stride);
    let stride = src_stride as isize;
    let src = src as isize;
    let (el, er) = (extend_left as usize, extend_right as usize);

    /* copy the left and right most columns out */
    let mut src_ptr1 = src + v_start as isize * stride;
    let mut src_ptr2 = src + v_start as isize * stride + width as isize - 1;
    let mut dst_ptr1 = src + v_start as isize * stride - extend_left as isize;
    let mut dst_ptr2 = src_ptr2 + 1;

    for _i in v_start..v_end {
        let l = buf[src_ptr1 as usize];
        buf[dst_ptr1 as usize..dst_ptr1 as usize + el].fill(l);
        let r = buf[src_ptr2 as usize];
        buf[dst_ptr2 as usize..dst_ptr2 as usize + er].fill(r);
        src_ptr1 += stride;
        src_ptr2 += stride;
        dst_ptr1 += stride;
        dst_ptr2 += stride;
    }

    /* Now copy the top and bottom lines into each line of the respective
     * borders
     */
    let src_ptr1 = src - extend_left as isize;
    let mut dst_ptr1 = src_ptr1 + stride * -(extend_top as isize);

    for _i in 0..extend_top {
        buf.copy_within(
            src_ptr1 as usize..src_ptr1 as usize + linesize,
            dst_ptr1 as usize,
        );
        dst_ptr1 += stride;
    }

    let src_ptr2 = src_ptr1 + stride * (height as isize - 1);
    let mut dst_ptr2 = src_ptr2;

    for _i in 0..extend_bottom {
        dst_ptr2 += stride;
        buf.copy_within(
            src_ptr2 as usize..src_ptr2 as usize + linesize,
            dst_ptr2 as usize,
        );
    }
}

/// Translation of `aom_extend_frame_borders_plane_row_c()`.
pub(crate) fn aom_extend_frame_borders_plane_row(
    ybf: &mut Yv12BufferConfig,
    plane: usize,
    v_start: i32,
    v_end: i32,
) {
    let ext_size = ybf.border;
    let ss_x = ybf.subsampling_x;
    let ss_y = ybf.subsampling_y;

    debug_assert!(ybf.y_height() - ybf.y_crop_height() < 16);
    debug_assert!(ybf.y_width() - ybf.y_crop_width() < 16);
    debug_assert!(ybf.y_height() - ybf.y_crop_height() >= 0);
    debug_assert!(ybf.y_width() - ybf.y_crop_width() >= 0);

    let is_uv = (plane > 0) as usize;
    let top = ext_size >> (if is_uv != 0 { ss_y } else { 0 });
    let left = ext_size >> (if is_uv != 0 { ss_x } else { 0 });
    let bottom = top + ybf.heights[is_uv] - ybf.crop_heights[is_uv];
    let right = left + ybf.widths[is_uv] - ybf.crop_widths[is_uv];
    let extend_top_border = v_start == 0;
    let extend_bottom_border = v_end == ybf.crop_heights[is_uv];

    let (stride, cw, ch) = (
        ybf.strides[is_uv],
        ybf.crop_widths[is_uv],
        ybf.crop_heights[is_uv],
    );
    let off = ybf.buffers[plane];
    extend_plane(
        &mut ybf.buffer_alloc,
        off,
        stride,
        cw,
        ch,
        if extend_top_border { top } else { 0 },
        left,
        if extend_bottom_border { bottom } else { 0 },
        right,
        v_start,
        v_end,
    );
}
