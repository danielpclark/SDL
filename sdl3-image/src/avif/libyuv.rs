// Rust translation of the libyuv subset libavif bundles in
// third_party/libyuv (source/scale.c, source/scale_common.c,
// source/scale_any.c, source/row_common.c, source/planar_functions.c and
// their headers; libyuv as of def473f501acbd652cd4593fd2a90a067e8c9f1a,
// LIBYUV_VERSION 1880), at the libavif revision SDL_image's
// external/libavif pins.
// Copyright 2011 The LibYuv Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The plane scaler `avifImageScaleWithLimit()` uses (only `ScalePlane()`
//! and `ScalePlane_12()`, C row functions only).
//!
//! A pointer into a plane is a slice holding the whole plane and an index
//! into it; reads outside of the slice (which upstream would make past the
//! end of a row or of a buffer, only where the sample read is weighted by
//! zero) read 0, and writes outside of it are dropped.

/// Supported filtering. Translation of `enum FilterMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilterMode {
    /// Point sample; Fastest.
    None = 0,
    /// Filter horizontally only.
    Linear = 1,
    /// Faster than box, but lower quality scaling down.
    Bilinear = 2,
    /// Highest quality.
    Box = 3,
}

// ---------------------------------------------------------------------------
// Slice access (the translation's)

#[inline]
fn rd8(s: &[u8], i: isize) -> i32 {
    if i < 0 {
        return 0;
    }
    s.get(i as usize).copied().unwrap_or(0) as i32
}

#[inline]
fn rd16(s: &[u16], i: isize) -> i32 {
    if i < 0 {
        return 0;
    }
    s.get(i as usize).copied().unwrap_or(0) as i32
}

#[inline]
fn wr8(d: &mut [u8], i: isize, v: u8) {
    if i >= 0 {
        if let Some(p) = d.get_mut(i as usize) {
            *p = v;
        }
    }
}

#[inline]
fn wr16(d: &mut [u16], i: isize, v: u16) {
    if i >= 0 {
        if let Some(p) = d.get_mut(i as usize) {
            *p = v;
        }
    }
}

/// `align_buffer_64()`: a zeroed row buffer of `size` elements, `None`
/// when it can't be allocated.
fn align_buffer_64<T: Clone + Default>(size: usize) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(size).ok()?;
    v.resize(size, T::default());
    Some(v)
}

// ---------------------------------------------------------------------------
// planar_functions.c / row_common.c

/// `CopyRow_C()`
fn copy_row_c(src: &[u8], src_at: isize, dst: &mut [u8], dst_at: isize, count: i32) {
    for i in 0..count as isize {
        wr8(dst, dst_at + i, rd8(src, src_at + i) as u8);
    }
}

/// Copy a plane of data. Translation of `CopyPlane()`.
fn copy_plane(
    src_y: &[u8],
    mut src_stride_y: i32,
    dst_y: &mut [u8],
    mut dst_stride_y: i32,
    mut width: i32,
    mut height: i32,
) {
    let mut src_at: isize = 0;
    let mut dst_at: isize = 0;
    let copy_row: fn(&[u8], isize, &mut [u8], isize, i32) = copy_row_c;
    if width <= 0 || height == 0 {
        return;
    }
    // Negative height means invert the image.
    if height < 0 {
        height = -height;
        dst_at += (height - 1) as isize * dst_stride_y as isize;
        dst_stride_y = -dst_stride_y;
    }
    // Coalesce rows.
    if src_stride_y == width && dst_stride_y == width {
        width *= height;
        height = 1;
        src_stride_y = 0;
        dst_stride_y = 0;
    }
    // Nothing to do.
    // (the source and destination are never the same buffer here)

    // Copy plane
    for _y in 0..height {
        copy_row(src_y, src_at, dst_y, dst_at, width);
        src_at += src_stride_y as isize;
        dst_at += dst_stride_y as isize;
    }
}

/// Translation of `CopyPlane_16()` (which copies the bytes with
/// `CopyPlane()`).
fn copy_plane_16(
    src_y: &[u16],
    src_stride_y: i32,
    dst_y: &mut [u16],
    dst_stride_y: i32,
    width: i32,
    height: i32,
) {
    if width <= 0 || height == 0 {
        return;
    }
    // (as samples: rows are coalesced the same way)
    let (mut width, mut height, mut src_stride_y, mut dst_stride_y) =
        (width, height, src_stride_y, dst_stride_y);
    let mut dst_at: isize = 0;
    if height < 0 {
        height = -height;
        dst_at += (height - 1) as isize * dst_stride_y as isize;
        dst_stride_y = -dst_stride_y;
    }
    if src_stride_y == width && dst_stride_y == width {
        width *= height;
        height = 1;
        src_stride_y = 0;
        dst_stride_y = 0;
    }
    let mut src_at: isize = 0;
    for _y in 0..height {
        for i in 0..width as isize {
            wr16(dst_y, dst_at + i, rd16(src_y, src_at + i) as u16);
        }
        src_at += src_stride_y as isize;
        dst_at += dst_stride_y as isize;
    }
}

/// Blend 2 rows into 1. Translation of `HalfRow_C()`.
fn half_row_c(
    src_uv: &[u8],
    src_at: isize,
    src_uv_stride: isize,
    dst_uv: &mut [u8],
    dst_at: isize,
    width: i32,
) {
    for x in 0..width as isize {
        wr8(
            dst_uv,
            dst_at + x,
            ((rd8(src_uv, src_at + x) + rd8(src_uv, src_at + src_uv_stride + x) + 1) >> 1) as u8,
        );
    }
}

/// Translation of `HalfRow_16_C()`.
fn half_row_16_c(
    src_uv: &[u16],
    src_at: isize,
    src_uv_stride: isize,
    dst_uv: &mut [u16],
    dst_at: isize,
    width: i32,
) {
    for x in 0..width as isize {
        wr16(
            dst_uv,
            dst_at + x,
            ((rd16(src_uv, src_at + x) + rd16(src_uv, src_at + src_uv_stride + x) + 1) >> 1) as u16,
        );
    }
}

/// C version 2x2 -> 2x1. Translation of `InterpolateRow_C()`.
fn interpolate_row_c(
    dst_ptr: &mut [u8],
    dst_at: isize,
    src_ptr: &[u8],
    src_at: isize,
    src_stride: isize,
    width: i32,
    source_y_fraction: i32,
) {
    let y1_fraction = source_y_fraction;
    let y0_fraction = 256 - y1_fraction;
    let src_at1 = src_at + src_stride;

    if y1_fraction == 0 {
        copy_row_c(src_ptr, src_at, dst_ptr, dst_at, width);
        return;
    }
    if y1_fraction == 128 {
        half_row_c(src_ptr, src_at, src_stride, dst_ptr, dst_at, width);
        return;
    }
    for x in 0..width as isize {
        wr8(
            dst_ptr,
            dst_at + x,
            ((rd8(src_ptr, src_at + x) * y0_fraction
                + rd8(src_ptr, src_at1 + x) * y1_fraction
                + 128)
                >> 8) as u8,
        );
    }
}

/// C version 2x2 -> 2x1. Translation of `InterpolateRow_16_C()`.
fn interpolate_row_16_c(
    dst_ptr: &mut [u16],
    dst_at: isize,
    src_ptr: &[u16],
    src_at: isize,
    src_stride: isize,
    width: i32,
    source_y_fraction: i32,
) {
    let y1_fraction = source_y_fraction;
    let y0_fraction = 256 - y1_fraction;
    let src_at1 = src_at + src_stride;

    if y1_fraction == 0 {
        for x in 0..width as isize {
            wr16(dst_ptr, dst_at + x, rd16(src_ptr, src_at + x) as u16);
        }
        return;
    }
    if y1_fraction == 128 {
        half_row_16_c(src_ptr, src_at, src_stride, dst_ptr, dst_at, width);
        return;
    }
    for x in 0..width as isize {
        wr16(
            dst_ptr,
            dst_at + x,
            ((rd16(src_ptr, src_at + x) * y0_fraction
                + rd16(src_ptr, src_at1 + x) * y1_fraction
                + 128)
                >> 8) as u16,
        );
    }
}

// ---------------------------------------------------------------------------
// scale_common.c

// Sample position: (O is src sample position, X is dst sample position)
//
//      v dst_ptr at here           v stop at here
//  X O X   X O X   X O X   X O X   X O X
//    ^ src_ptr at here
/// Translation of `ScaleRowUp2_Linear_C()`.
fn scale_row_up2_linear_c(
    src_ptr: &[u8],
    src_at: isize,
    dst_ptr: &mut [u8],
    dst_at: isize,
    dst_width: i32,
) {
    let src_width = (dst_width >> 1) as isize;
    for x in 0..src_width {
        let s0 = rd8(src_ptr, src_at + x);
        let s1 = rd8(src_ptr, src_at + x + 1);
        wr8(dst_ptr, dst_at + 2 * x, ((s0 * 3 + s1 + 2) >> 2) as u8);
        wr8(dst_ptr, dst_at + 2 * x + 1, ((s0 + s1 * 3 + 2) >> 2) as u8);
    }
}

// Sample position: (O is src sample position, X is dst sample position)
//
//    src_ptr at here
//  X v X   X   X   X   X   X   X   X   X
//    O       O       O       O       O
//  X   X   X   X   X   X   X   X   X   X
//      ^ dst_ptr at here           ^ stop at here
//  X   X   X   X   X   X   X   X   X   X
//    O       O       O       O       O
//  X   X   X   X   X   X   X   X   X   X
/// Translation of `ScaleRowUp2_Bilinear_C()`.
fn scale_row_up2_bilinear_c(
    src_ptr: &[u8],
    src_at: isize,
    src_stride: isize,
    dst_ptr: &mut [u8],
    dst_at: isize,
    dst_stride: isize,
    dst_width: i32,
) {
    let s = src_at;
    let t = src_at + src_stride;
    let d = dst_at;
    let e = dst_at + dst_stride;
    let src_width = (dst_width >> 1) as isize;
    for x in 0..src_width {
        let (s0, s1) = (rd8(src_ptr, s + x), rd8(src_ptr, s + x + 1));
        let (t0, t1) = (rd8(src_ptr, t + x), rd8(src_ptr, t + x + 1));
        wr8(
            dst_ptr,
            d + 2 * x,
            ((s0 * 9 + s1 * 3 + t0 * 3 + t1 + 8) >> 4) as u8,
        );
        wr8(
            dst_ptr,
            d + 2 * x + 1,
            ((s0 * 3 + s1 * 9 + t0 + t1 * 3 + 8) >> 4) as u8,
        );
        wr8(
            dst_ptr,
            e + 2 * x,
            ((s0 * 3 + s1 + t0 * 9 + t1 * 3 + 8) >> 4) as u8,
        );
        wr8(
            dst_ptr,
            e + 2 * x + 1,
            ((s0 + s1 * 3 + t0 * 3 + t1 * 9 + 8) >> 4) as u8,
        );
    }
}

/// Only suitable for at most 14 bit range. Translation of
/// `ScaleRowUp2_Linear_16_C()`.
fn scale_row_up2_linear_16_c(
    src_ptr: &[u16],
    src_at: isize,
    dst_ptr: &mut [u16],
    dst_at: isize,
    dst_width: i32,
) {
    let src_width = (dst_width >> 1) as isize;
    for x in 0..src_width {
        let s0 = rd16(src_ptr, src_at + x);
        let s1 = rd16(src_ptr, src_at + x + 1);
        wr16(dst_ptr, dst_at + 2 * x, ((s0 * 3 + s1 + 2) >> 2) as u16);
        wr16(dst_ptr, dst_at + 2 * x + 1, ((s0 + s1 * 3 + 2) >> 2) as u16);
    }
}

/// Only suitable for at most 12bit range. Translation of
/// `ScaleRowUp2_Bilinear_16_C()`.
fn scale_row_up2_bilinear_16_c(
    src_ptr: &[u16],
    src_at: isize,
    src_stride: isize,
    dst_ptr: &mut [u16],
    dst_at: isize,
    dst_stride: isize,
    dst_width: i32,
) {
    let s = src_at;
    let t = src_at + src_stride;
    let d = dst_at;
    let e = dst_at + dst_stride;
    let src_width = (dst_width >> 1) as isize;
    for x in 0..src_width {
        let (s0, s1) = (rd16(src_ptr, s + x), rd16(src_ptr, s + x + 1));
        let (t0, t1) = (rd16(src_ptr, t + x), rd16(src_ptr, t + x + 1));
        wr16(
            dst_ptr,
            d + 2 * x,
            ((s0 * 9 + s1 * 3 + t0 * 3 + t1 + 8) >> 4) as u16,
        );
        wr16(
            dst_ptr,
            d + 2 * x + 1,
            ((s0 * 3 + s1 * 9 + t0 + t1 * 3 + 8) >> 4) as u16,
        );
        wr16(
            dst_ptr,
            e + 2 * x,
            ((s0 * 3 + s1 + t0 * 9 + t1 * 3 + 8) >> 4) as u16,
        );
        wr16(
            dst_ptr,
            e + 2 * x + 1,
            ((s0 + s1 * 3 + t0 * 3 + t1 * 9 + 8) >> 4) as u16,
        );
    }
}

/// Scales a single row of pixels using point sampling. Translation of
/// `ScaleCols_C()`.
fn scale_cols_c(
    dst_ptr: &mut [u8],
    dst_at: isize,
    src_ptr: &[u8],
    src_at: isize,
    dst_width: i32,
    mut x: i32,
    dx: i32,
) {
    let mut d = dst_at;
    let mut j = 0;
    while j < dst_width - 1 {
        wr8(dst_ptr, d, rd8(src_ptr, src_at + (x >> 16) as isize) as u8);
        x = x.wrapping_add(dx);
        wr8(
            dst_ptr,
            d + 1,
            rd8(src_ptr, src_at + (x >> 16) as isize) as u8,
        );
        x = x.wrapping_add(dx);
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        wr8(dst_ptr, d, rd8(src_ptr, src_at + (x >> 16) as isize) as u8);
    }
}

/// Translation of `ScaleCols_16_C()`.
fn scale_cols_16_c(
    dst_ptr: &mut [u16],
    dst_at: isize,
    src_ptr: &[u16],
    src_at: isize,
    dst_width: i32,
    mut x: i32,
    dx: i32,
) {
    let mut d = dst_at;
    let mut j = 0;
    while j < dst_width - 1 {
        wr16(
            dst_ptr,
            d,
            rd16(src_ptr, src_at + (x >> 16) as isize) as u16,
        );
        x = x.wrapping_add(dx);
        wr16(
            dst_ptr,
            d + 1,
            rd16(src_ptr, src_at + (x >> 16) as isize) as u16,
        );
        x = x.wrapping_add(dx);
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        wr16(
            dst_ptr,
            d,
            rd16(src_ptr, src_at + (x >> 16) as isize) as u16,
        );
    }
}

/// Scales a single row of pixels up by 2x using point sampling.
/// Translation of `ScaleColsUp2_C()`.
fn scale_cols_up2_c(
    dst_ptr: &mut [u8],
    dst_at: isize,
    src_ptr: &[u8],
    src_at: isize,
    dst_width: i32,
    _x: i32,
    _dx: i32,
) {
    let mut d = dst_at;
    let mut s = src_at;
    let mut j = 0;
    while j < dst_width - 1 {
        let v = rd8(src_ptr, s) as u8;
        wr8(dst_ptr, d, v);
        wr8(dst_ptr, d + 1, v);
        s += 1;
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        wr8(dst_ptr, d, rd8(src_ptr, s) as u8);
    }
}

/// Translation of `ScaleColsUp2_16_C()`.
fn scale_cols_up2_16_c(
    dst_ptr: &mut [u16],
    dst_at: isize,
    src_ptr: &[u16],
    src_at: isize,
    dst_width: i32,
    _x: i32,
    _dx: i32,
) {
    let mut d = dst_at;
    let mut s = src_at;
    let mut j = 0;
    while j < dst_width - 1 {
        let v = rd16(src_ptr, s) as u16;
        wr16(dst_ptr, d, v);
        wr16(dst_ptr, d + 1, v);
        s += 1;
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        wr16(dst_ptr, d, rd16(src_ptr, s) as u16);
    }
}

// (1-f)a + fb can be replaced with a + f(b-a)
/// `BLENDER()` of the 8-bit functions (x86: 7 bit math with rounding; on
/// ARM upstream uses the 16-bit formula, see [`blender_16`]).
#[inline]
fn blender_8(a: i32, b: i32, f: i32) -> u8 {
    #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
    {
        (a + ((f * (b - a) + 0x8000) >> 16)) as u8
    }
    #[cfg(not(any(target_arch = "arm", target_arch = "aarch64")))]
    {
        // Intel uses 7 bit math with rounding.
        (a + (((f >> 9) * (b - a) + 0x40) >> 7)) as u8
    }
}

/// Translation of `ScaleFilterCols_C()`.
fn scale_filter_cols_c(
    dst_ptr: &mut [u8],
    dst_at: isize,
    src_ptr: &[u8],
    src_at: isize,
    dst_width: i32,
    mut x: i32,
    dx: i32,
) {
    let mut d = dst_at;
    let mut j = 0;
    while j < dst_width - 1 {
        let mut xi = (x >> 16) as isize;
        let mut a = rd8(src_ptr, src_at + xi);
        let mut b = rd8(src_ptr, src_at + xi + 1);
        wr8(dst_ptr, d, blender_8(a, b, x & 0xffff));
        x = x.wrapping_add(dx);
        xi = (x >> 16) as isize;
        a = rd8(src_ptr, src_at + xi);
        b = rd8(src_ptr, src_at + xi + 1);
        wr8(dst_ptr, d + 1, blender_8(a, b, x & 0xffff));
        x = x.wrapping_add(dx);
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        let xi = (x >> 16) as isize;
        let a = rd8(src_ptr, src_at + xi);
        let b = rd8(src_ptr, src_at + xi + 1);
        wr8(dst_ptr, d, blender_8(a, b, x & 0xffff));
    }
}

/// Translation of `ScaleFilterCols64_C()`.
fn scale_filter_cols64_c(
    dst_ptr: &mut [u8],
    dst_at: isize,
    src_ptr: &[u8],
    src_at: isize,
    dst_width: i32,
    x32: i32,
    dx: i32,
) {
    let mut x = x32 as i64;
    let mut d = dst_at;
    let mut j = 0;
    while j < dst_width - 1 {
        let mut xi = (x >> 16) as isize;
        let mut a = rd8(src_ptr, src_at + xi);
        let mut b = rd8(src_ptr, src_at + xi + 1);
        wr8(dst_ptr, d, blender_8(a, b, (x & 0xffff) as i32));
        x += dx as i64;
        xi = (x >> 16) as isize;
        a = rd8(src_ptr, src_at + xi);
        b = rd8(src_ptr, src_at + xi + 1);
        wr8(dst_ptr, d + 1, blender_8(a, b, (x & 0xffff) as i32));
        x += dx as i64;
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        let xi = (x >> 16) as isize;
        let a = rd8(src_ptr, src_at + xi);
        let b = rd8(src_ptr, src_at + xi + 1);
        wr8(dst_ptr, d, blender_8(a, b, (x & 0xffff) as i32));
    }
}

// Same as 8 bit arm blender but return is cast to uint16_t
/// `BLENDER()` of the 16-bit functions.
#[inline]
fn blender_16(a: i32, b: i32, f: i64) -> u16 {
    (a + (((f * (b as i64 - a as i64)) + 0x8000) >> 16) as i32) as u16
}

/// Translation of `ScaleFilterCols_16_C()`.
fn scale_filter_cols_16_c(
    dst_ptr: &mut [u16],
    dst_at: isize,
    src_ptr: &[u16],
    src_at: isize,
    dst_width: i32,
    mut x: i32,
    dx: i32,
) {
    let mut d = dst_at;
    let mut j = 0;
    while j < dst_width - 1 {
        let mut xi = (x >> 16) as isize;
        let mut a = rd16(src_ptr, src_at + xi);
        let mut b = rd16(src_ptr, src_at + xi + 1);
        wr16(dst_ptr, d, blender_16(a, b, (x & 0xffff) as i64));
        x = x.wrapping_add(dx);
        xi = (x >> 16) as isize;
        a = rd16(src_ptr, src_at + xi);
        b = rd16(src_ptr, src_at + xi + 1);
        wr16(dst_ptr, d + 1, blender_16(a, b, (x & 0xffff) as i64));
        x = x.wrapping_add(dx);
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        let xi = (x >> 16) as isize;
        let a = rd16(src_ptr, src_at + xi);
        let b = rd16(src_ptr, src_at + xi + 1);
        wr16(dst_ptr, d, blender_16(a, b, (x & 0xffff) as i64));
    }
}

/// Translation of `ScaleFilterCols64_16_C()`.
fn scale_filter_cols64_16_c(
    dst_ptr: &mut [u16],
    dst_at: isize,
    src_ptr: &[u16],
    src_at: isize,
    dst_width: i32,
    x32: i32,
    dx: i32,
) {
    let mut x = x32 as i64;
    let mut d = dst_at;
    let mut j = 0;
    while j < dst_width - 1 {
        let mut xi = (x >> 16) as isize;
        let mut a = rd16(src_ptr, src_at + xi);
        let mut b = rd16(src_ptr, src_at + xi + 1);
        wr16(dst_ptr, d, blender_16(a, b, x & 0xffff));
        x += dx as i64;
        xi = (x >> 16) as isize;
        a = rd16(src_ptr, src_at + xi);
        b = rd16(src_ptr, src_at + xi + 1);
        wr16(dst_ptr, d + 1, blender_16(a, b, x & 0xffff));
        x += dx as i64;
        d += 2;
        j += 2;
    }
    if dst_width & 1 != 0 {
        let xi = (x >> 16) as isize;
        let a = rd16(src_ptr, src_at + xi);
        let b = rd16(src_ptr, src_at + xi + 1);
        wr16(dst_ptr, d, blender_16(a, b, x & 0xffff));
    }
}

/// Translation of `ScaleAddRow_C()`.
fn scale_add_row_c(src_ptr: &[u8], src_at: isize, dst_ptr: &mut [u16], src_width: i32) {
    for x in 0..src_width.max(0) as usize {
        if let Some(d) = dst_ptr.get_mut(x) {
            *d = d.wrapping_add(rd8(src_ptr, src_at + x as isize) as u16);
        }
    }
}

/// Translation of `ScaleAddRow_16_C()`.
fn scale_add_row_16_c(src_ptr: &[u16], src_at: isize, dst_ptr: &mut [u32], src_width: i32) {
    for x in 0..src_width.max(0) as usize {
        if let Some(d) = dst_ptr.get_mut(x) {
            *d = d.wrapping_add(rd16(src_ptr, src_at + x as isize) as u32);
        }
    }
}

/// Scale plane vertically with bilinear interpolation. Translation of
/// `ScalePlaneVertical()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_vertical(
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_argb: &[u8],
    dst_argb: &mut [u8],
    x: i32,
    mut y: i32,
    dy: i32,
    bpp: i32, // bytes per pixel. 4 for ARGB.
    filtering: FilterMode,
) {
    // TODO(fbarchard): Allow higher bpp.
    let dst_width_bytes = dst_width * bpp;
    let interpolate_row: fn(&mut [u8], isize, &[u8], isize, isize, i32, i32) = interpolate_row_c;
    let max_y = if src_height > 1 {
        ((src_height - 1) << 16) - 1
    } else {
        0
    };
    let src_at = (x >> 16) as isize * bpp as isize;
    let mut dst_at: isize = 0;

    for _j in 0..dst_height {
        if y > max_y {
            y = max_y;
        }
        let yi = y >> 16;
        let yf = if filtering != FilterMode::None {
            (y >> 8) & 255
        } else {
            0
        };
        interpolate_row(
            dst_argb,
            dst_at,
            src_argb,
            src_at + yi as isize * src_stride as isize,
            src_stride as isize,
            dst_width_bytes,
            yf,
        );
        dst_at += dst_stride as isize;
        y = y.wrapping_add(dy);
    }
}

/// Translation of `ScalePlaneVertical_16()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_vertical_16(
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_argb: &[u16],
    dst_argb: &mut [u16],
    x: i32,
    mut y: i32,
    dy: i32,
    wpp: i32, /* words per pixel. normally 1 */
    filtering: FilterMode,
) {
    // TODO(fbarchard): Allow higher wpp.
    let dst_width_words = dst_width * wpp;
    let interpolate_row: fn(&mut [u16], isize, &[u16], isize, isize, i32, i32) =
        interpolate_row_16_c;
    let max_y = if src_height > 1 {
        ((src_height - 1) << 16) - 1
    } else {
        0
    };
    let src_at = (x >> 16) as isize * wpp as isize;
    let mut dst_at: isize = 0;
    for _j in 0..dst_height {
        if y > max_y {
            y = max_y;
        }
        let yi = y >> 16;
        let yf = if filtering != FilterMode::None {
            (y >> 8) & 255
        } else {
            0
        };
        interpolate_row(
            dst_argb,
            dst_at,
            src_argb,
            src_at + yi as isize * src_stride as isize,
            src_stride as isize,
            dst_width_words,
            yf,
        );
        dst_at += dst_stride as isize;
        y = y.wrapping_add(dy);
    }
}

/// Simplify the filtering based on scale factors. Translation of
/// `ScaleFilterReduce()`.
fn scale_filter_reduce(
    mut src_width: i32,
    mut src_height: i32,
    dst_width: i32,
    dst_height: i32,
    mut filtering: FilterMode,
) -> FilterMode {
    if src_width < 0 {
        src_width = -src_width;
    }
    if src_height < 0 {
        src_height = -src_height;
    }
    if filtering == FilterMode::Box {
        // If scaling either axis to 0.5 or larger, switch from Box to Bilinear.
        if dst_width * 2 >= src_width || dst_height * 2 >= src_height {
            filtering = FilterMode::Bilinear;
        }
    }
    if filtering == FilterMode::Bilinear {
        if src_height == 1 {
            filtering = FilterMode::Linear;
        }
        // TODO(fbarchard): Detect any odd scale factor and reduce to Linear.
        if dst_height == src_height || dst_height * 3 == src_height {
            filtering = FilterMode::Linear;
        }
        // TODO(fbarchard): Remove 1 pixel wide filter restriction, which is to
        // avoid reading 2 pixels horizontally that causes memory exception.
        if src_width == 1 {
            filtering = FilterMode::None;
        }
    }
    if filtering == FilterMode::Linear {
        if src_width == 1 {
            filtering = FilterMode::None;
        }
        // TODO(fbarchard): Detect any odd scale factor and reduce to None.
        if dst_width == src_width || dst_width * 3 == src_width {
            filtering = FilterMode::None;
        }
    }
    filtering
}

/// Divide num by div and return as 16.16 fixed point result. Translation
/// of `FixedDiv_C()` (`FixedDiv`).
fn fixed_div(num: i32, div: i32) -> i32 {
    if div == 0 {
        // (never: the callers check the dimensions first)
        return 0;
    }
    (((num as i64) << 16) / div as i64) as i32
}

/// Divide num - 1 by div - 1 and return as 16.16 fixed point result.
/// Translation of `FixedDiv1_C()` (`FixedDiv1`).
fn fixed_div1(num: i32, div: i32) -> i32 {
    if div == 1 {
        // (never: the callers check the dimensions first)
        return 0;
    }
    ((((num as i64) << 16) - 0x00010001) / (div as i64 - 1)) as i32
}

/// `CENTERSTART()`
#[inline]
fn centerstart(dx: i32, s: i32) -> i32 {
    if dx < 0 {
        -((-dx >> 1) + s)
    } else {
        (dx >> 1) + s
    }
}

/// Translation of `Abs()`.
#[inline]
fn abs(v: i32) -> i32 {
    if v >= 0 {
        v
    } else {
        -v
    }
}

/// Compute slope values for stepping. Translation of `ScaleSlope()`;
/// returns `(x, y, dx, dy)` updated from the given values.
#[allow(clippy::too_many_arguments)]
fn scale_slope(
    src_width: i32,
    src_height: i32,
    mut dst_width: i32,
    mut dst_height: i32,
    filtering: FilterMode,
    x: &mut i32,
    y: &mut i32,
    dx: &mut i32,
    dy: &mut i32,
) {
    // Check for 1 pixel and avoid FixedDiv overflow.
    if dst_width == 1 && src_width >= 32768 {
        dst_width = src_width;
    }
    if dst_height == 1 && src_height >= 32768 {
        dst_height = src_height;
    }
    if filtering == FilterMode::Box {
        // Scale step for point sampling duplicates all pixels equally.
        *dx = fixed_div(abs(src_width), dst_width);
        *dy = fixed_div(src_height, dst_height);
        *x = 0;
        *y = 0;
    } else if filtering == FilterMode::Bilinear {
        // Scale step for bilinear sampling renders last pixel once for upsample.
        if dst_width <= abs(src_width) {
            *dx = fixed_div(abs(src_width), dst_width);
            *x = centerstart(*dx, -32768); // Subtract 0.5 (32768) to center filter.
        } else if src_width > 1 && dst_width > 1 {
            *dx = fixed_div1(abs(src_width), dst_width);
            *x = 0;
        }
        if dst_height <= src_height {
            *dy = fixed_div(src_height, dst_height);
            *y = centerstart(*dy, -32768); // Subtract 0.5 (32768) to center filter.
        } else if src_height > 1 && dst_height > 1 {
            *dy = fixed_div1(src_height, dst_height);
            *y = 0;
        }
    } else if filtering == FilterMode::Linear {
        // Scale step for bilinear sampling renders last pixel once for upsample.
        if dst_width <= abs(src_width) {
            *dx = fixed_div(abs(src_width), dst_width);
            *x = centerstart(*dx, -32768); // Subtract 0.5 (32768) to center filter.
        } else if src_width > 1 && dst_width > 1 {
            *dx = fixed_div1(abs(src_width), dst_width);
            *x = 0;
        }
        *dy = fixed_div(src_height, dst_height);
        *y = *dy >> 1;
    } else {
        // Scale step for point sampling duplicates all pixels equally.
        *dx = fixed_div(abs(src_width), dst_width);
        *dy = fixed_div(src_height, dst_height);
        *x = centerstart(*dx, 0);
        *y = centerstart(*dy, 0);
    }
    // Negative src_width means horizontally mirror.
    if src_width < 0 {
        *x = x.wrapping_add((dst_width - 1).wrapping_mul(*dx));
        *dx = -*dx;
        // src_width = -src_width;   // Caller must do this.
    }
}

// ---------------------------------------------------------------------------
// scale_any.c

// Scale up horizontally 2 times using linear filter.
// (SUH2LANY() with the C function as the "SIMD" one and a MASK of 0)
// Even the C versions need to be wrapped, because boundary pixels have to
// be handled differently

/// Translation of `ScaleRowUp2_Linear_Any_C()`.
fn scale_row_up2_linear_any_c(
    src_ptr: &[u8],
    src_at: isize,
    dst_ptr: &mut [u8],
    dst_at: isize,
    dst_width: i32,
) {
    let work_width = (dst_width - 1) & !1;
    let r = 0; // work_width & MASK
    let n = work_width; // work_width & ~MASK
    wr8(dst_ptr, dst_at, rd8(src_ptr, src_at) as u8);
    if work_width > 0 {
        if n != 0 {
            scale_row_up2_linear_c(src_ptr, src_at, dst_ptr, dst_at + 1, n);
        }
        scale_row_up2_linear_c(
            src_ptr,
            src_at + (n / 2) as isize,
            dst_ptr,
            dst_at + n as isize + 1,
            r,
        );
    }
    wr8(
        dst_ptr,
        dst_at + (dst_width - 1) as isize,
        rd8(src_ptr, src_at + ((dst_width - 1) / 2) as isize) as u8,
    );
}

/// Translation of `ScaleRowUp2_Linear_16_Any_C()`.
fn scale_row_up2_linear_16_any_c(
    src_ptr: &[u16],
    src_at: isize,
    dst_ptr: &mut [u16],
    dst_at: isize,
    dst_width: i32,
) {
    let work_width = (dst_width - 1) & !1;
    let r = 0; // work_width & MASK
    let n = work_width; // work_width & ~MASK
    wr16(dst_ptr, dst_at, rd16(src_ptr, src_at) as u16);
    if work_width > 0 {
        if n != 0 {
            scale_row_up2_linear_16_c(src_ptr, src_at, dst_ptr, dst_at + 1, n);
        }
        scale_row_up2_linear_16_c(
            src_ptr,
            src_at + (n / 2) as isize,
            dst_ptr,
            dst_at + n as isize + 1,
            r,
        );
    }
    wr16(
        dst_ptr,
        dst_at + (dst_width - 1) as isize,
        rd16(src_ptr, src_at + ((dst_width - 1) / 2) as isize) as u16,
    );
}

// Scale up 2 times using bilinear filter.
// This function produces 2 rows at a time.
// (SU2BLANY() with the C function as the "SIMD" one and a MASK of 0)

/// Translation of `ScaleRowUp2_Bilinear_Any_C()`.
fn scale_row_up2_bilinear_any_c(
    src_ptr: &[u8],
    src_at: isize,
    src_stride: isize,
    dst_ptr: &mut [u8],
    dst_at: isize,
    dst_stride: isize,
    dst_width: i32,
) {
    let work_width = (dst_width - 1) & !1;
    let r = 0; // work_width & MASK
    let n = work_width; // work_width & ~MASK
    let sa = src_at;
    let sb = src_at + src_stride;
    let da = dst_at;
    let db = dst_at + dst_stride;
    wr8(
        dst_ptr,
        da,
        ((3 * rd8(src_ptr, sa) + rd8(src_ptr, sb) + 2) >> 2) as u8,
    );
    wr8(
        dst_ptr,
        db,
        ((rd8(src_ptr, sa) + 3 * rd8(src_ptr, sb) + 2) >> 2) as u8,
    );
    if work_width > 0 {
        if n != 0 {
            scale_row_up2_bilinear_c(src_ptr, sa, sb - sa, dst_ptr, da + 1, db - da, n);
        }
        scale_row_up2_bilinear_c(
            src_ptr,
            sa + (n / 2) as isize,
            sb - sa,
            dst_ptr,
            da + n as isize + 1,
            db - da,
            r,
        );
    }
    let last = (dst_width - 1) as isize;
    let half = ((dst_width - 1) / 2) as isize;
    wr8(
        dst_ptr,
        da + last,
        ((3 * rd8(src_ptr, sa + half) + rd8(src_ptr, sb + half) + 2) >> 2) as u8,
    );
    wr8(
        dst_ptr,
        db + last,
        ((rd8(src_ptr, sa + half) + 3 * rd8(src_ptr, sb + half) + 2) >> 2) as u8,
    );
}

/// Translation of `ScaleRowUp2_Bilinear_16_Any_C()`.
fn scale_row_up2_bilinear_16_any_c(
    src_ptr: &[u16],
    src_at: isize,
    src_stride: isize,
    dst_ptr: &mut [u16],
    dst_at: isize,
    dst_stride: isize,
    dst_width: i32,
) {
    let work_width = (dst_width - 1) & !1;
    let r = 0; // work_width & MASK
    let n = work_width; // work_width & ~MASK
    let sa = src_at;
    let sb = src_at + src_stride;
    let da = dst_at;
    let db = dst_at + dst_stride;
    wr16(
        dst_ptr,
        da,
        ((3 * rd16(src_ptr, sa) + rd16(src_ptr, sb) + 2) >> 2) as u16,
    );
    wr16(
        dst_ptr,
        db,
        ((rd16(src_ptr, sa) + 3 * rd16(src_ptr, sb) + 2) >> 2) as u16,
    );
    if work_width > 0 {
        if n != 0 {
            scale_row_up2_bilinear_16_c(src_ptr, sa, sb - sa, dst_ptr, da + 1, db - da, n);
        }
        scale_row_up2_bilinear_16_c(
            src_ptr,
            sa + (n / 2) as isize,
            sb - sa,
            dst_ptr,
            da + n as isize + 1,
            db - da,
            r,
        );
    }
    let last = (dst_width - 1) as isize;
    let half = ((dst_width - 1) / 2) as isize;
    wr16(
        dst_ptr,
        da + last,
        ((3 * rd16(src_ptr, sa + half) + rd16(src_ptr, sb + half) + 2) >> 2) as u16,
    );
    wr16(
        dst_ptr,
        db + last,
        ((rd16(src_ptr, sa + half) + 3 * rd16(src_ptr, sb + half) + 2) >> 2) as u16,
    );
}

// ---------------------------------------------------------------------------
// scale.c

/// `MIN1()`
#[inline]
fn min1(x: i32) -> i32 {
    if x < 1 {
        1
    } else {
        x
    }
}

/// Translation of `SumPixels()`.
#[inline]
fn sum_pixels(iboxwidth: i32, src_ptr: &[u16], at: isize) -> u32 {
    let mut sum = 0u32;
    for x in 0..iboxwidth as isize {
        let i = at + x;
        let v = if i < 0 {
            0
        } else {
            src_ptr.get(i as usize).copied().unwrap_or(0)
        };
        sum = sum.wrapping_add(v as u32);
    }
    sum
}

/// Translation of `SumPixels_16()`.
#[inline]
fn sum_pixels_16(iboxwidth: i32, src_ptr: &[u32], at: isize) -> u32 {
    let mut sum = 0u32;
    for x in 0..iboxwidth as isize {
        let i = at + x;
        let v = if i < 0 {
            0
        } else {
            src_ptr.get(i as usize).copied().unwrap_or(0)
        };
        sum = sum.wrapping_add(v);
    }
    sum
}

/// Translation of `ScaleAddCols2_C()`.
fn scale_add_cols2_c(
    dst_width: i32,
    boxheight: i32,
    mut x: i32,
    dx: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u8],
    dst_at: isize,
) {
    let minboxwidth = dx >> 16;
    let scaletbl = [
        65536 / (min1(minboxwidth) * boxheight),
        65536 / (min1(minboxwidth + 1) * boxheight),
    ];
    let mut d = dst_at;
    for _i in 0..dst_width {
        let ix = x >> 16;
        x = x.wrapping_add(dx);
        let boxwidth = min1((x >> 16) - ix);
        let scaletbl_index = boxwidth - minboxwidth;
        // (assert((scaletbl_index == 0) || (scaletbl_index == 1)), compiled
        // out)
        let scale = match scaletbl_index {
            0 => scaletbl[0],
            1 => scaletbl[1],
            _ => 0,
        };
        wr8(
            dst_ptr,
            d,
            (sum_pixels(boxwidth, src_ptr, ix as isize).wrapping_mul(scale as u32) >> 16) as u8,
        );
        d += 1;
    }
}

/// Translation of `ScaleAddCols2_16_C()`.
fn scale_add_cols2_16_c(
    dst_width: i32,
    boxheight: i32,
    mut x: i32,
    dx: i32,
    src_ptr: &[u32],
    dst_ptr: &mut [u16],
    dst_at: isize,
) {
    let minboxwidth = dx >> 16;
    let scaletbl = [
        65536 / (min1(minboxwidth) * boxheight),
        65536 / (min1(minboxwidth + 1) * boxheight),
    ];
    let mut d = dst_at;
    for _i in 0..dst_width {
        let ix = x >> 16;
        x = x.wrapping_add(dx);
        let boxwidth = min1((x >> 16) - ix);
        let scaletbl_index = boxwidth - minboxwidth;
        let scale = match scaletbl_index {
            0 => scaletbl[0],
            1 => scaletbl[1],
            _ => 0,
        };
        wr16(
            dst_ptr,
            d,
            (sum_pixels_16(boxwidth, src_ptr, ix as isize).wrapping_mul(scale as u32) >> 16) as u16,
        );
        d += 1;
    }
}

/// Translation of `ScaleAddCols0_C()`.
fn scale_add_cols0_c(
    dst_width: i32,
    boxheight: i32,
    x: i32,
    _dx: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u8],
    dst_at: isize,
) {
    let scaleval = 65536 / boxheight;
    let src_at = (x >> 16) as isize;
    for i in 0..dst_width as isize {
        let j = src_at + i;
        let v = if j < 0 {
            0
        } else {
            src_ptr.get(j as usize).copied().unwrap_or(0)
        };
        wr8(
            dst_ptr,
            dst_at + i,
            ((v as i32).wrapping_mul(scaleval) >> 16) as u8,
        );
    }
}

/// Translation of `ScaleAddCols1_C()`.
fn scale_add_cols1_c(
    dst_width: i32,
    boxheight: i32,
    mut x: i32,
    dx: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u8],
    dst_at: isize,
) {
    let boxwidth = min1(dx >> 16);
    let scaleval = 65536 / (boxwidth * boxheight);
    x >>= 16;
    let mut d = dst_at;
    for _i in 0..dst_width {
        wr8(
            dst_ptr,
            d,
            (sum_pixels(boxwidth, src_ptr, x as isize).wrapping_mul(scaleval as u32) >> 16) as u8,
        );
        d += 1;
        x += boxwidth;
    }
}

/// Translation of `ScaleAddCols1_16_C()` (which, unlike the 8-bit one,
/// doesn't shift `x`; it is 0 in the box filter).
fn scale_add_cols1_16_c(
    dst_width: i32,
    boxheight: i32,
    mut x: i32,
    dx: i32,
    src_ptr: &[u32],
    dst_ptr: &mut [u16],
    dst_at: isize,
) {
    let boxwidth = min1(dx >> 16);
    let scaleval = 65536 / (boxwidth * boxheight);
    let mut d = dst_at;
    for _i in 0..dst_width {
        wr16(
            dst_ptr,
            d,
            (sum_pixels_16(boxwidth, src_ptr, x as isize).wrapping_mul(scaleval as u32) >> 16)
                as u16,
        );
        d += 1;
        x = x.wrapping_add(boxwidth);
    }
}

type ScaleAddColsFn = fn(i32, i32, i32, i32, &[u16], &mut [u8], isize);
type ScaleAddCols16Fn = fn(i32, i32, i32, i32, &[u32], &mut [u16], isize);

// Scale plane down to any dimensions, with interpolation.
// (boxfilter).
//
// Same method as SimpleScale, which is fixed point, outputting
// one pixel of destination using fixed point (16.16) to step
// through source, sampling a box of pixel with simple
// averaging.
/// Translation of `ScalePlaneBox()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_box(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u8],
    dst_ptr: &mut [u8],
) -> i32 {
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    let max_y = src_height << 16;
    scale_slope(
        src_width,
        src_height,
        dst_width,
        dst_height,
        FilterMode::Box,
        &mut x,
        &mut y,
        &mut dx,
        &mut dy,
    );
    src_width = abs(src_width);
    {
        // Allocate a row buffer of uint16_t.
        let Some(mut row16) = align_buffer_64::<u16>(src_width as usize) else {
            return 1;
        };
        let scale_add_cols: ScaleAddColsFn = if dx & 0xffff != 0 {
            scale_add_cols2_c
        } else if dx != 0x10000 {
            scale_add_cols1_c
        } else {
            scale_add_cols0_c
        };
        let scale_add_row: fn(&[u8], isize, &mut [u16], i32) = scale_add_row_c;

        let mut dst_at: isize = 0;
        for _j in 0..dst_height {
            let iy = y >> 16;
            let mut src = iy as isize * src_stride as isize;
            y = y.wrapping_add(dy);
            if y > max_y {
                y = max_y;
            }
            let boxheight = min1((y >> 16) - iy);
            row16.fill(0);
            for _k in 0..boxheight {
                scale_add_row(src_ptr, src, &mut row16, src_width);
                src += src_stride as isize;
            }
            scale_add_cols(dst_width, boxheight, x, dx, &row16, dst_ptr, dst_at);
            dst_at += dst_stride as isize;
        }
    }
    0
}

/// Translation of `ScalePlaneBox_16()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_box_16(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u16],
) -> i32 {
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    let max_y = src_height << 16;
    scale_slope(
        src_width,
        src_height,
        dst_width,
        dst_height,
        FilterMode::Box,
        &mut x,
        &mut y,
        &mut dx,
        &mut dy,
    );
    src_width = abs(src_width);
    {
        // Allocate a row buffer of uint32_t.
        let Some(mut row32) = align_buffer_64::<u32>(src_width as usize) else {
            return 1;
        };
        let scale_add_cols: ScaleAddCols16Fn = if dx & 0xffff != 0 {
            scale_add_cols2_16_c
        } else {
            scale_add_cols1_16_c
        };
        let scale_add_row: fn(&[u16], isize, &mut [u32], i32) = scale_add_row_16_c;

        let mut dst_at: isize = 0;
        for _j in 0..dst_height {
            let iy = y >> 16;
            let mut src = iy as isize * src_stride as isize;
            y = y.wrapping_add(dy);
            if y > max_y {
                y = max_y;
            }
            let boxheight = min1((y >> 16) - iy);
            row32.fill(0);
            for _k in 0..boxheight {
                scale_add_row(src_ptr, src, &mut row32, src_width);
                src += src_stride as isize;
            }
            scale_add_cols(dst_width, boxheight, x, dx, &row32, dst_ptr, dst_at);
            dst_at += dst_stride as isize;
        }
    }
    0
}

type ScaleFilterColsFn = fn(&mut [u8], isize, &[u8], isize, i32, i32, i32);
type ScaleFilterCols16Fn = fn(&mut [u16], isize, &[u16], isize, i32, i32, i32);

/// Scale plane down with bilinear interpolation. Translation of
/// `ScalePlaneBilinearDown()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_bilinear_down(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u8],
    dst_ptr: &mut [u8],
    filtering: FilterMode,
) -> i32 {
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    // TODO(fbarchard): Consider not allocating row buffer for kFilterLinear.
    // Allocate a row buffer.
    let Some(mut row) = align_buffer_64::<u8>(src_width.max(0) as usize) else {
        return 1;
    };

    let max_y = (src_height - 1) << 16;
    let scale_filter_cols: ScaleFilterColsFn = if src_width >= 32768 {
        scale_filter_cols64_c
    } else {
        scale_filter_cols_c
    };
    let interpolate_row: fn(&mut [u8], isize, &[u8], isize, isize, i32, i32) = interpolate_row_c;
    scale_slope(
        src_width, src_height, dst_width, dst_height, filtering, &mut x, &mut y, &mut dx, &mut dy,
    );
    src_width = abs(src_width);

    if y > max_y {
        y = max_y;
    }

    let mut dst_at: isize = 0;
    for _j in 0..dst_height {
        let yi = y >> 16;
        let src = yi as isize * src_stride as isize;
        if filtering == FilterMode::Linear {
            scale_filter_cols(dst_ptr, dst_at, src_ptr, src, dst_width, x, dx);
        } else {
            let yf = (y >> 8) & 255;
            interpolate_row(
                &mut row,
                0,
                src_ptr,
                src,
                src_stride as isize,
                src_width,
                yf,
            );
            scale_filter_cols(dst_ptr, dst_at, &row, 0, dst_width, x, dx);
        }
        dst_at += dst_stride as isize;
        y = y.wrapping_add(dy);
        if y > max_y {
            y = max_y;
        }
    }
    0
}

/// Translation of `ScalePlaneBilinearDown_16()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_bilinear_down_16(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u16],
    filtering: FilterMode,
) -> i32 {
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    // TODO(fbarchard): Consider not allocating row buffer for kFilterLinear.
    // Allocate a row buffer.
    let Some(mut row) = align_buffer_64::<u16>(src_width.max(0) as usize) else {
        return 1;
    };

    let max_y = (src_height - 1) << 16;
    let scale_filter_cols: ScaleFilterCols16Fn = if src_width >= 32768 {
        scale_filter_cols64_16_c
    } else {
        scale_filter_cols_16_c
    };
    let interpolate_row: fn(&mut [u16], isize, &[u16], isize, isize, i32, i32) =
        interpolate_row_16_c;
    scale_slope(
        src_width, src_height, dst_width, dst_height, filtering, &mut x, &mut y, &mut dx, &mut dy,
    );
    src_width = abs(src_width);

    if y > max_y {
        y = max_y;
    }

    let mut dst_at: isize = 0;
    for _j in 0..dst_height {
        let yi = y >> 16;
        let src = yi as isize * src_stride as isize;
        if filtering == FilterMode::Linear {
            scale_filter_cols(dst_ptr, dst_at, src_ptr, src, dst_width, x, dx);
        } else {
            let yf = (y >> 8) & 255;
            interpolate_row(
                &mut row,
                0,
                src_ptr,
                src,
                src_stride as isize,
                src_width,
                yf,
            );
            scale_filter_cols(dst_ptr, dst_at, &row, 0, dst_width, x, dx);
        }
        dst_at += dst_stride as isize;
        y = y.wrapping_add(dy);
        if y > max_y {
            y = max_y;
        }
    }
    0
}

/// Scale up down with bilinear interpolation. Translation of
/// `ScalePlaneBilinearUp()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_bilinear_up(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u8],
    dst_ptr: &mut [u8],
    filtering: FilterMode,
) -> i32 {
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    let max_y = (src_height - 1) << 16;
    let interpolate_row: fn(&mut [u8], isize, &[u8], isize, isize, i32, i32) = interpolate_row_c;
    let mut scale_filter_cols: ScaleFilterColsFn = if filtering != FilterMode::None {
        scale_filter_cols_c
    } else {
        scale_cols_c
    };
    scale_slope(
        src_width, src_height, dst_width, dst_height, filtering, &mut x, &mut y, &mut dx, &mut dy,
    );
    src_width = abs(src_width);

    if filtering != FilterMode::None && src_width >= 32768 {
        scale_filter_cols = scale_filter_cols64_c;
    }
    if filtering == FilterMode::None && src_width * 2 == dst_width && x < 0x8000 {
        scale_filter_cols = scale_cols_up2_c;
    }

    if y > max_y {
        y = max_y;
    }
    {
        let mut yi = y >> 16;
        let mut src = yi as isize * src_stride as isize;

        // Allocate 2 row buffers.
        let row_size = (dst_width + 31) & !31;
        let Some(mut row) = align_buffer_64::<u8>(row_size as usize * 2) else {
            return 1;
        };

        let mut rowptr: isize = 0;
        let mut rowstride = row_size as isize;
        let mut lasty = yi;

        scale_filter_cols(&mut row, rowptr, src_ptr, src, dst_width, x, dx);
        if src_height > 1 {
            src += src_stride as isize;
        }
        scale_filter_cols(&mut row, rowptr + rowstride, src_ptr, src, dst_width, x, dx);
        if src_height > 2 {
            src += src_stride as isize;
        }

        let mut dst_at: isize = 0;
        for _j in 0..dst_height {
            yi = y >> 16;
            if yi != lasty {
                if y > max_y {
                    y = max_y;
                    yi = y >> 16;
                    src = yi as isize * src_stride as isize;
                }
                if yi != lasty {
                    scale_filter_cols(&mut row, rowptr, src_ptr, src, dst_width, x, dx);
                    rowptr += rowstride;
                    rowstride = -rowstride;
                    lasty = yi;
                    if y.wrapping_add(65536) < max_y {
                        src += src_stride as isize;
                    }
                }
            }
            if filtering == FilterMode::Linear {
                interpolate_row(dst_ptr, dst_at, &row, rowptr, 0, dst_width, 0);
            } else {
                let yf = (y >> 8) & 255;
                interpolate_row(dst_ptr, dst_at, &row, rowptr, rowstride, dst_width, yf);
            }
            dst_at += dst_stride as isize;
            y = y.wrapping_add(dy);
        }
    }
    0
}

// Scale plane, horizontally up by 2 times.
// Uses linear filter horizontally, nearest vertically.
// This is an optimized version for scaling up a plane to 2 times of
// its original width, using linear interpolation.
// This is used to scale U and V planes of I422 to I444.
/// Translation of `ScalePlaneUp2_Linear()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_up2_linear(
    _src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u8],
    dst_ptr: &mut [u8],
) {
    let scale_row_up: fn(&[u8], isize, &mut [u8], isize, i32) = scale_row_up2_linear_any_c;

    // This function can only scale up by 2 times horizontally.
    // (assert(src_width == ((dst_width + 1) / 2)), compiled out)

    if dst_height == 1 {
        scale_row_up(
            src_ptr,
            ((src_height - 1) / 2) as isize * src_stride as isize,
            dst_ptr,
            0,
            dst_width,
        );
    } else {
        let dy = fixed_div(src_height - 1, dst_height - 1);
        let mut y: i32 = (1 << 15) - 1;
        let mut dst_at: isize = 0;
        for _i in 0..dst_height {
            scale_row_up(
                src_ptr,
                (y >> 16) as isize * src_stride as isize,
                dst_ptr,
                dst_at,
                dst_width,
            );
            dst_at += dst_stride as isize;
            y = y.wrapping_add(dy);
        }
    }
}

// Scale plane, up by 2 times.
// This is an optimized version for scaling up a plane to 2 times of
// its original size, using bilinear interpolation.
// This is used to scale U and V planes of I420 to I444.
/// Translation of `ScalePlaneUp2_Bilinear()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_up2_bilinear(
    _src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u8],
    dst_ptr: &mut [u8],
) {
    let scale2_row_up: fn(&[u8], isize, isize, &mut [u8], isize, isize, i32) =
        scale_row_up2_bilinear_any_c;

    // This function can only scale up by 2 times.
    // (asserts compiled out)

    let mut src_at: isize = 0;
    let mut dst_at: isize = 0;
    scale2_row_up(src_ptr, src_at, 0, dst_ptr, dst_at, 0, dst_width);
    dst_at += dst_stride as isize;
    for _x in 0..src_height - 1 {
        scale2_row_up(
            src_ptr,
            src_at,
            src_stride as isize,
            dst_ptr,
            dst_at,
            dst_stride as isize,
            dst_width,
        );
        src_at += src_stride as isize;
        // TODO(fbarchard): Test performance of writing one row of destination at a
        // time.
        dst_at += 2 * dst_stride as isize;
    }
    if dst_height & 1 == 0 {
        scale2_row_up(src_ptr, src_at, 0, dst_ptr, dst_at, 0, dst_width);
    }
}

// Scale at most 14 bit plane, horizontally up by 2 times.
// This is an optimized version for scaling up a plane to 2 times of
// its original width, using linear interpolation.
// stride is in count of uint16_t.
// This is used to scale U and V planes of I210 to I410 and I212 to I412.
/// Translation of `ScalePlaneUp2_12_Linear()` (and of
/// `ScalePlaneUp2_16_Linear()`, which is the same).
#[allow(clippy::too_many_arguments)]
fn scale_plane_up2_12_linear(
    _src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u16],
) {
    let scale_row_up: fn(&[u16], isize, &mut [u16], isize, i32) = scale_row_up2_linear_16_any_c;

    // This function can only scale up by 2 times horizontally.
    // (assert compiled out)

    if dst_height == 1 {
        scale_row_up(
            src_ptr,
            ((src_height - 1) / 2) as isize * src_stride as isize,
            dst_ptr,
            0,
            dst_width,
        );
    } else {
        let dy = fixed_div(src_height - 1, dst_height - 1);
        let mut y: i32 = (1 << 15) - 1;
        let mut dst_at: isize = 0;
        for _i in 0..dst_height {
            scale_row_up(
                src_ptr,
                (y >> 16) as isize * src_stride as isize,
                dst_ptr,
                dst_at,
                dst_width,
            );
            dst_at += dst_stride as isize;
            y = y.wrapping_add(dy);
        }
    }
}

// Scale at most 12 bit plane, up by 2 times.
// This is an optimized version for scaling up a plane to 2 times of
// its original size, using bilinear interpolation.
// stride is in count of uint16_t.
// This is used to scale U and V planes of I010 to I410 and I012 to I412.
/// Translation of `ScalePlaneUp2_12_Bilinear()` (and of
/// `ScalePlaneUp2_16_Bilinear()`, which is the same).
#[allow(clippy::too_many_arguments)]
fn scale_plane_up2_12_bilinear(
    _src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u16],
) {
    let scale2_row_up: fn(&[u16], isize, isize, &mut [u16], isize, isize, i32) =
        scale_row_up2_bilinear_16_any_c;

    // This function can only scale up by 2 times.
    // (asserts compiled out)

    let mut src_at: isize = 0;
    let mut dst_at: isize = 0;
    scale2_row_up(src_ptr, src_at, 0, dst_ptr, dst_at, 0, dst_width);
    dst_at += dst_stride as isize;
    for _x in 0..src_height - 1 {
        scale2_row_up(
            src_ptr,
            src_at,
            src_stride as isize,
            dst_ptr,
            dst_at,
            dst_stride as isize,
            dst_width,
        );
        src_at += src_stride as isize;
        dst_at += 2 * dst_stride as isize;
    }
    if dst_height & 1 == 0 {
        scale2_row_up(src_ptr, src_at, 0, dst_ptr, dst_at, 0, dst_width);
    }
}

/// Translation of `ScalePlaneBilinearUp_16()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_bilinear_up_16(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u16],
    filtering: FilterMode,
) -> i32 {
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    let max_y = (src_height - 1) << 16;
    let interpolate_row: fn(&mut [u16], isize, &[u16], isize, isize, i32, i32) =
        interpolate_row_16_c;
    let mut scale_filter_cols: ScaleFilterCols16Fn = if filtering != FilterMode::None {
        scale_filter_cols_16_c
    } else {
        scale_cols_16_c
    };
    scale_slope(
        src_width, src_height, dst_width, dst_height, filtering, &mut x, &mut y, &mut dx, &mut dy,
    );
    src_width = abs(src_width);

    if filtering != FilterMode::None && src_width >= 32768 {
        scale_filter_cols = scale_filter_cols64_16_c;
    }
    if filtering == FilterMode::None && src_width * 2 == dst_width && x < 0x8000 {
        scale_filter_cols = scale_cols_up2_16_c;
    }
    if y > max_y {
        y = max_y;
    }
    {
        let mut yi = y >> 16;
        let mut src = yi as isize * src_stride as isize;

        // Allocate 2 row buffers.
        let row_size = (dst_width + 31) & !31;
        // (row_size * 4 bytes: two rows of uint16_t)
        let Some(mut row) = align_buffer_64::<u16>(row_size as usize * 2) else {
            return 1;
        };
        let mut rowstride = row_size as isize;
        let mut lasty = yi;
        let mut rowptr: isize = 0;

        scale_filter_cols(&mut row, rowptr, src_ptr, src, dst_width, x, dx);
        if src_height > 1 {
            src += src_stride as isize;
        }
        scale_filter_cols(&mut row, rowptr + rowstride, src_ptr, src, dst_width, x, dx);
        if src_height > 2 {
            src += src_stride as isize;
        }

        let mut dst_at: isize = 0;
        for _j in 0..dst_height {
            yi = y >> 16;
            if yi != lasty {
                if y > max_y {
                    y = max_y;
                    yi = y >> 16;
                    src = yi as isize * src_stride as isize;
                }
                if yi != lasty {
                    scale_filter_cols(&mut row, rowptr, src_ptr, src, dst_width, x, dx);
                    rowptr += rowstride;
                    rowstride = -rowstride;
                    lasty = yi;
                    if y.wrapping_add(65536) < max_y {
                        src += src_stride as isize;
                    }
                }
            }
            if filtering == FilterMode::Linear {
                interpolate_row(dst_ptr, dst_at, &row, rowptr, 0, dst_width, 0);
            } else {
                let yf = (y >> 8) & 255;
                interpolate_row(dst_ptr, dst_at, &row, rowptr, rowstride, dst_width, yf);
            }
            dst_at += dst_stride as isize;
            y = y.wrapping_add(dy);
        }
    }
    0
}

// Scale Plane to/from any dimensions, without interpolation.
// Fixed point math is used for performance: The upper 16 bits
// of x and dx is the integer part of the source position and
// the lower 16 bits are the fixed decimal part.

/// Translation of `ScalePlaneSimple()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_simple(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u8],
    dst_ptr: &mut [u8],
) {
    let mut scale_cols: ScaleFilterColsFn = scale_cols_c;
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    scale_slope(
        src_width,
        src_height,
        dst_width,
        dst_height,
        FilterMode::None,
        &mut x,
        &mut y,
        &mut dx,
        &mut dy,
    );
    src_width = abs(src_width);

    if src_width * 2 == dst_width && x < 0x8000 {
        scale_cols = scale_cols_up2_c;
    }

    let mut dst_at: isize = 0;
    for _i in 0..dst_height {
        scale_cols(
            dst_ptr,
            dst_at,
            src_ptr,
            (y >> 16) as isize * src_stride as isize,
            dst_width,
            x,
            dx,
        );
        dst_at += dst_stride as isize;
        y = y.wrapping_add(dy);
    }
}

/// Translation of `ScalePlaneSimple_16()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_simple_16(
    mut src_width: i32,
    src_height: i32,
    dst_width: i32,
    dst_height: i32,
    src_stride: i32,
    dst_stride: i32,
    src_ptr: &[u16],
    dst_ptr: &mut [u16],
) {
    let mut scale_cols: ScaleFilterCols16Fn = scale_cols_16_c;
    // Initial source x/y coordinate and step values as 16.16 fixed point.
    let mut x = 0;
    let mut y = 0;
    let mut dx = 0;
    let mut dy = 0;
    scale_slope(
        src_width,
        src_height,
        dst_width,
        dst_height,
        FilterMode::None,
        &mut x,
        &mut y,
        &mut dx,
        &mut dy,
    );
    src_width = abs(src_width);

    if src_width * 2 == dst_width && x < 0x8000 {
        scale_cols = scale_cols_up2_16_c;
    }

    let mut dst_at: isize = 0;
    for _i in 0..dst_height {
        scale_cols(
            dst_ptr,
            dst_at,
            src_ptr,
            (y >> 16) as isize * src_stride as isize,
            dst_width,
            x,
            dx,
        );
        dst_at += dst_stride as isize;
        y = y.wrapping_add(dy);
    }
}

/// Scale a plane. This function dispatches to a specialized scaler based
/// on scale factor. Translation of `ScalePlane()` (the source height is
/// never negative here: libavif doesn't invert images).
#[allow(clippy::too_many_arguments)]
pub(crate) fn scale_plane(
    src: &[u8],
    src_stride: i32,
    src_width: i32,
    src_height: i32,
    dst: &mut [u8],
    dst_stride: i32,
    dst_width: i32,
    dst_height: i32,
    mut filtering: FilterMode,
) -> i32 {
    // Simplify filtering when possible.
    filtering = scale_filter_reduce(src_width, src_height, dst_width, dst_height, filtering);

    // Use specialized scales to improve performance for common resolutions.
    // For example, all the 1/2 scalings will use ScalePlaneDown2()
    if dst_width == src_width && dst_height == src_height {
        // Straight copy.
        copy_plane(src, src_stride, dst, dst_stride, dst_width, dst_height);
        return 0;
    }
    if dst_width == src_width && filtering != FilterMode::Box {
        let mut dy = 0;
        let mut y = 0;
        // When scaling down, use the center 2 rows to filter.
        // When scaling up, last row of destination uses the last 2 source rows.
        if dst_height <= src_height {
            dy = fixed_div(src_height, dst_height);
            y = centerstart(dy, -32768); // Subtract 0.5 (32768) to center filter.
        } else if src_height > 1 && dst_height > 1 {
            dy = fixed_div1(src_height, dst_height);
        }
        // Arbitrary scale vertically, but unscaled horizontally.
        scale_plane_vertical(
            src_height, dst_width, dst_height, src_stride, dst_stride, src, dst, 0, y, dy,
            /*bpp=*/ 1, filtering,
        );
        return 0;
    }
    if filtering == FilterMode::Box && dst_height * 2 < src_height {
        return scale_plane_box(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
    }
    if (dst_width + 1) / 2 == src_width && filtering == FilterMode::Linear {
        scale_plane_up2_linear(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
        return 0;
    }
    if (dst_height + 1) / 2 == src_height
        && (dst_width + 1) / 2 == src_width
        && (filtering == FilterMode::Bilinear || filtering == FilterMode::Box)
    {
        scale_plane_up2_bilinear(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
        return 0;
    }
    if filtering != FilterMode::None && dst_height > src_height {
        return scale_plane_bilinear_up(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
            filtering,
        );
    }
    if filtering != FilterMode::None {
        return scale_plane_bilinear_down(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
            filtering,
        );
    }
    scale_plane_simple(
        src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
    );
    0
}

/// Translation of `ScalePlane_16()`.
#[allow(clippy::too_many_arguments)]
fn scale_plane_16(
    src: &[u16],
    src_stride: i32,
    src_width: i32,
    src_height: i32,
    dst: &mut [u16],
    dst_stride: i32,
    dst_width: i32,
    dst_height: i32,
    mut filtering: FilterMode,
) -> i32 {
    // Simplify filtering when possible.
    filtering = scale_filter_reduce(src_width, src_height, dst_width, dst_height, filtering);

    // Use specialized scales to improve performance for common resolutions.
    // For example, all the 1/2 scalings will use ScalePlaneDown2()
    if dst_width == src_width && dst_height == src_height {
        // Straight copy.
        copy_plane_16(src, src_stride, dst, dst_stride, dst_width, dst_height);
        return 0;
    }
    if dst_width == src_width && filtering != FilterMode::Box {
        let mut dy = 0;
        let mut y = 0;
        // When scaling down, use the center 2 rows to filter.
        // When scaling up, last row of destination uses the last 2 source rows.
        if dst_height <= src_height {
            dy = fixed_div(src_height, dst_height);
            y = centerstart(dy, -32768); // Subtract 0.5 (32768) to center filter.
                                         // When scaling up, ensure the last row of destination uses the last
                                         // source. Avoid divide by zero for dst_height but will do no scaling
                                         // later.
        } else if src_height > 1 && dst_height > 1 {
            dy = fixed_div1(src_height, dst_height);
        }
        // Arbitrary scale vertically, but unscaled horizontally.
        scale_plane_vertical_16(
            src_height, dst_width, dst_height, src_stride, dst_stride, src, dst, 0, y, dy,
            /*bpp=*/ 1, filtering,
        );
        return 0;
    }
    if filtering == FilterMode::Box && dst_height * 2 < src_height {
        return scale_plane_box_16(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
    }
    if (dst_width + 1) / 2 == src_width && filtering == FilterMode::Linear {
        scale_plane_up2_12_linear(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
        return 0;
    }
    if (dst_height + 1) / 2 == src_height
        && (dst_width + 1) / 2 == src_width
        && (filtering == FilterMode::Bilinear || filtering == FilterMode::Box)
    {
        scale_plane_up2_12_bilinear(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
        return 0;
    }
    if filtering != FilterMode::None && dst_height > src_height {
        return scale_plane_bilinear_up_16(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
            filtering,
        );
    }
    if filtering != FilterMode::None {
        return scale_plane_bilinear_down_16(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
            filtering,
        );
    }
    scale_plane_simple_16(
        src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
    );
    0
}

/// Translation of `ScalePlane_12()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scale_plane_12(
    src: &[u16],
    src_stride: i32,
    src_width: i32,
    src_height: i32,
    dst: &mut [u16],
    dst_stride: i32,
    dst_width: i32,
    dst_height: i32,
    mut filtering: FilterMode,
) -> i32 {
    // Simplify filtering when possible.
    filtering = scale_filter_reduce(src_width, src_height, dst_width, dst_height, filtering);

    if (dst_width + 1) / 2 == src_width && filtering == FilterMode::Linear {
        scale_plane_up2_12_linear(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
        return 0;
    }
    if (dst_height + 1) / 2 == src_height
        && (dst_width + 1) / 2 == src_width
        && (filtering == FilterMode::Bilinear || filtering == FilterMode::Box)
    {
        scale_plane_up2_12_bilinear(
            src_width, src_height, dst_width, dst_height, src_stride, dst_stride, src, dst,
        );
        return 0;
    }

    scale_plane_16(
        src, src_stride, src_width, src_height, dst, dst_stride, dst_width, dst_height, filtering,
    )
}
