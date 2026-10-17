// Rust translation of the parts of av1/common/reconinter.c and
// av1/common/reconinter.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Plane buffer setup (the saver never predicts between frames, so only the
//! unscaled buffer offsets are here).

use super::blockd::{Buf2d, MacroblockdPlane, MAX_MB_PLANE};
use super::enums::*;
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;

/// `scaled_buffer_offset()` with no scale factors.
#[inline]
fn scaled_buffer_offset(x_offset: i32, y_offset: i32, stride: i32) -> i64 {
    let x = x_offset;
    let y = y_offset;
    y as i64 * stride as i64 + x as i64
}

/// `setup_pred_plane()` with no scale factors; `src` is the plane's offset
/// in its frame buffer.
#[inline]
pub(crate) fn setup_pred_plane(
    dst: &mut Buf2d,
    bsize: BlockSize,
    src: usize,
    width: i32,
    height: i32,
    stride: i32,
    mut mi_row: i32,
    mut mi_col: i32,
    subsampling_x: i32,
    subsampling_y: i32,
) {
    // Offset the buffer pointer
    if subsampling_y != 0 && (mi_row & 0x01) != 0 && (MI_SIZE_HIGH[bsize as usize] == 1) {
        mi_row -= 1;
    }
    if subsampling_x != 0 && (mi_col & 0x01) != 0 && (MI_SIZE_WIDE[bsize as usize] == 1) {
        mi_col -= 1;
    }
    let x = (MI_SIZE * mi_col) >> subsampling_x;
    let y = (MI_SIZE * mi_row) >> subsampling_y;
    dst.buf = (src as i64 + scaled_buffer_offset(x, y, stride)) as usize;
    dst.buf0 = src;
    dst.width = width;
    dst.height = height;
    dst.stride = stride;
}

pub(crate) fn av1_setup_dst_planes(
    planes: &mut [MacroblockdPlane; MAX_MB_PLANE],
    bsize: BlockSize,
    src: &Yv12BufferConfig,
    mi_row: i32,
    mi_col: i32,
    plane_start: usize,
    plane_end: usize,
) {
    // We use AOMMIN(num_planes, MAX_MB_PLANE) instead of num_planes to quiet
    // the static analysis warnings.
    for i in plane_start..plane_end.min(MAX_MB_PLANE) {
        let pd = &mut planes[i];
        let is_uv = (i > 0) as usize;
        setup_pred_plane(
            &mut pd.dst,
            bsize,
            src.buffers[i],
            src.crop_widths[is_uv],
            src.crop_heights[is_uv],
            src.strides[is_uv],
            mi_row,
            mi_col,
            pd.subsampling_x,
            pd.subsampling_y,
        );
    }
}
