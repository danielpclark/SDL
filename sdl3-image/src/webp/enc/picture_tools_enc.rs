// Rust translation of src/enc/picture_tools_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2014 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebPPicture tools: alpha handling, etc. (`WebPBlendAlpha()` is not
//! translated: SDL_image doesn't call it.)

use crate::webp::dsp::alpha_processing::webp_alpha_replace;
use crate::webp::encode::WebPPicture;

//------------------------------------------------------------------------------
// Helper: clean up fully transparent area to help compressibility.

const SIZE: usize = 8;
const SIZE2: usize = SIZE / 2;

/// Translation of `IsTransparentARGBArea()`.
fn is_transparent_argb_area(argb: &[u32], mut ptr: usize, stride: usize, size: usize) -> bool {
    for _ in 0..size {
        for x in 0..size {
            if argb[ptr + x] & 0xff000000 != 0 {
                return false;
            }
        }
        ptr += stride;
    }
    true
}

/// Translation of `Flatten()`.
fn flatten(plane: &mut [u8], mut ptr: usize, v: i32, stride: usize, size: usize) {
    for _ in 0..size {
        plane[ptr..ptr + size].fill(v as u8);
        ptr += stride;
    }
}

/// Translation of `FlattenARGB()`.
fn flatten_argb(argb: &mut [u32], mut ptr: usize, v: u32, stride: usize, size: usize) {
    for _ in 0..size {
        argb[ptr..ptr + size].fill(v);
        ptr += stride;
    }
}

/// Smoothen the luma components of transparent pixels. Return true if the
/// whole block is transparent. Translation of `SmoothenBlock()` (the
/// blocks start at `a_ptr` in `a` and `y_ptr` in `y`).
#[allow(clippy::too_many_arguments)]
fn smoothen_block(
    a: &[u8],
    a_ptr: usize,
    a_stride: usize,
    y: &mut [u8],
    y_ptr: usize,
    y_stride: usize,
    width: usize,
    height: usize,
) -> bool {
    let mut sum = 0i32;
    let mut count = 0i32;
    let mut alpha_ptr = a_ptr;
    let mut luma_ptr = y_ptr;
    for _ in 0..height {
        for x in 0..width {
            if a[alpha_ptr + x] != 0 {
                count += 1;
                sum += y[luma_ptr + x] as i32;
            }
        }
        alpha_ptr += a_stride;
        luma_ptr += y_stride;
    }
    if count > 0 && count < (width * height) as i32 {
        let avg_u8 = (sum / count) as u8;
        alpha_ptr = a_ptr;
        luma_ptr = y_ptr;
        for _ in 0..height {
            for x in 0..width {
                if a[alpha_ptr + x] == 0 {
                    y[luma_ptr + x] = avg_u8;
                }
            }
            alpha_ptr += a_stride;
            luma_ptr += y_stride;
        }
    }
    count == 0
}

/// Replace transparent pixels (alpha 0) of the ARGB picture by 'color'.
/// Translation of `WebPReplaceTransparentPixels()`.
pub(crate) fn webp_replace_transparent_pixels(pic: &mut WebPPicture, mut color: u32) {
    if pic.use_argb {
        let width = pic.width as usize;
        let stride = pic.argb_stride as usize;
        color &= 0xffffff; // force alpha=0
        for y in 0..pic.height as usize {
            webp_alpha_replace(&mut pic.argb[y * stride..y * stride + width], color);
        }
    }
}

/// Remove the transparent area of the picture to help compressibility.
/// Translation of `WebPCleanupTransparentArea()`.
pub(crate) fn webp_cleanup_transparent_area(pic: &mut WebPPicture) {
    let w = pic.width as usize / SIZE;
    let h = pic.height as usize / SIZE;

    // note: we ignore the left-overs on right/bottom, except for SmoothenBlock().
    if pic.use_argb {
        let stride = pic.argb_stride as usize;
        let mut argb_value = 0u32;
        for y in 0..h {
            let mut need_reset = true;
            for x in 0..w {
                let off = (y * stride + x) * SIZE;
                if is_transparent_argb_area(&pic.argb, off, stride, SIZE) {
                    if need_reset {
                        argb_value = pic.argb[off];
                        need_reset = false;
                    }
                    flatten_argb(&mut pic.argb, off, argb_value, stride, SIZE);
                } else {
                    need_reset = true;
                }
            }
        }
    } else {
        let width = pic.width as usize;
        let height = pic.height as usize;
        let y_stride = pic.y_stride as usize;
        let uv_stride = pic.uv_stride as usize;
        let a_stride = pic.a_stride as usize;
        let mut y_ptr = 0usize;
        let mut u_ptr = 0usize;
        let mut v_ptr = 0usize;
        let mut a_ptr = 0usize;
        let mut values = [0i32; 3];
        if pic.a.is_empty() || pic.y.is_empty() || pic.u.is_empty() || pic.v.is_empty() {
            return;
        }
        let mut y = 0;
        while y + SIZE <= height {
            let mut need_reset = true;
            let mut x = 0;
            while x + SIZE <= width {
                if smoothen_block(
                    &pic.a,
                    a_ptr + x,
                    a_stride,
                    &mut pic.y,
                    y_ptr + x,
                    y_stride,
                    SIZE,
                    SIZE,
                ) {
                    if need_reset {
                        values[0] = pic.y[y_ptr + x] as i32;
                        values[1] = pic.u[u_ptr + (x >> 1)] as i32;
                        values[2] = pic.v[v_ptr + (x >> 1)] as i32;
                        need_reset = false;
                    }
                    flatten(&mut pic.y, y_ptr + x, values[0], y_stride, SIZE);
                    flatten(&mut pic.u, u_ptr + (x >> 1), values[1], uv_stride, SIZE2);
                    flatten(&mut pic.v, v_ptr + (x >> 1), values[2], uv_stride, SIZE2);
                } else {
                    need_reset = true;
                }
                x += SIZE;
            }
            if x < width {
                smoothen_block(
                    &pic.a,
                    a_ptr + x,
                    a_stride,
                    &mut pic.y,
                    y_ptr + x,
                    y_stride,
                    width - x,
                    SIZE,
                );
            }
            a_ptr += SIZE * a_stride;
            y_ptr += SIZE * y_stride;
            u_ptr += SIZE2 * uv_stride;
            v_ptr += SIZE2 * uv_stride;
            y += SIZE;
        }
        if y < height {
            let sub_height = height - y;
            let mut x = 0;
            while x + SIZE <= width {
                smoothen_block(
                    &pic.a,
                    a_ptr + x,
                    a_stride,
                    &mut pic.y,
                    y_ptr + x,
                    y_stride,
                    SIZE,
                    sub_height,
                );
                x += SIZE;
            }
            if x < width {
                smoothen_block(
                    &pic.a,
                    a_ptr + x,
                    a_stride,
                    &mut pic.y,
                    y_ptr + x,
                    y_stride,
                    width - x,
                    sub_height,
                );
            }
        }
    }
}
