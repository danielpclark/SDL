// Rust translation of src/dsp/upsampling.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the fancy upsampler.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! YUV to RGB upsampling functions.
//!
//! The `UPSAMPLE_FUNC()` instances are one function taking the sample
//! converter and the bytes per pixel; `WebPUpsamplers[mode]` picks them.
//! The 16-bit (4444 and 565) colorspaces, the YUV444 converters and the
//! point-sampling "dual line" samplers are not translated: SDL_image's
//! loader decodes to RGB and RGBA only, with fancy upsampling.

use crate::webp::decode::WebpCspMode;
use crate::webp::dsp::yuv::{
    vp8_yuv_to_argb, vp8_yuv_to_bgr, vp8_yuv_to_bgra, vp8_yuv_to_rgb, vp8_yuv_to_rgba,
};

/// A sample converter: Y, U and V to one output pixel.
type YuvToPixel = fn(i32, i32, i32, &mut [u8]);

//------------------------------------------------------------------------------
// Fancy upsampler

// Given samples laid out in a square as:
//  [a b]
//  [c d]
// we interpolate u/v as:
//  ([9*a + 3*b + 3*c +   d    3*a + 9*b + 3*c +   d] + [8 8]) / 16
//  ([3*a +   b + 9*c + 3*d      a + 3*b + 3*c + 9*d]   [8 8]) / 16

/// We process u and v together stashed into 32bit (16bit each).
fn load_uv(u: u8, v: u8) -> u32 {
    u as u32 | ((v as u32) << 16)
}

/// The rows of a line pair: the luma of the top and (unless it is the
/// last odd row) bottom rows, the chroma rows above and below them, and
/// where in `dst` the output rows start.
pub(crate) struct LinePair<'a> {
    pub(crate) top_y: &'a [u8],
    pub(crate) bottom_y: Option<&'a [u8]>,
    pub(crate) top_u: &'a [u8],
    pub(crate) top_v: &'a [u8],
    pub(crate) cur_u: &'a [u8],
    pub(crate) cur_v: &'a [u8],
    pub(crate) top_dst: usize,
    pub(crate) bottom_dst: Option<usize>,
}

/// Translation of the `UPSAMPLE_FUNC()` macro, with `FUNC` and `XSTEP`.
fn upsample_func(func: YuvToPixel, xstep: usize, l: &LinePair<'_>, dst: &mut [u8], len: i32) {
    let len = len as usize;
    let last_pixel_pair = (len - 1) >> 1;
    let mut tl_uv = load_uv(l.top_u[0], l.top_v[0]); // top-left sample
    let mut l_uv = load_uv(l.cur_u[0], l.cur_v[0]); // left-sample
    let top_y = l.top_y;
    let top_dst = l.top_dst;
    {
        let uv0 = (3 * tl_uv + l_uv + 0x00020002) >> 2;
        func(
            top_y[0] as i32,
            (uv0 & 0xff) as i32,
            (uv0 >> 16) as i32,
            &mut dst[top_dst..],
        );
    }
    if let (Some(bottom_y), Some(bottom_dst)) = (l.bottom_y, l.bottom_dst) {
        let uv0 = (3 * l_uv + tl_uv + 0x00020002) >> 2;
        func(
            bottom_y[0] as i32,
            (uv0 & 0xff) as i32,
            (uv0 >> 16) as i32,
            &mut dst[bottom_dst..],
        );
    }
    for x in 1..=last_pixel_pair {
        let t_uv = load_uv(l.top_u[x], l.top_v[x]); // top sample
        let uv = load_uv(l.cur_u[x], l.cur_v[x]); // sample
                                                  // precompute invariant values associated with first and second diagonals
        let avg = tl_uv + t_uv + l_uv + uv + 0x00080008;
        let diag_12 = (avg + 2 * (t_uv + l_uv)) >> 3;
        let diag_03 = (avg + 2 * (tl_uv + uv)) >> 3;
        {
            let uv0 = (diag_12 + tl_uv) >> 1;
            let uv1 = (diag_03 + t_uv) >> 1;
            func(
                top_y[2 * x - 1] as i32,
                (uv0 & 0xff) as i32,
                (uv0 >> 16) as i32,
                &mut dst[top_dst + (2 * x - 1) * xstep..],
            );
            func(
                top_y[2 * x] as i32,
                (uv1 & 0xff) as i32,
                (uv1 >> 16) as i32,
                &mut dst[top_dst + (2 * x) * xstep..],
            );
        }
        if let (Some(bottom_y), Some(bottom_dst)) = (l.bottom_y, l.bottom_dst) {
            let uv0 = (diag_03 + l_uv) >> 1;
            let uv1 = (diag_12 + uv) >> 1;
            func(
                bottom_y[2 * x - 1] as i32,
                (uv0 & 0xff) as i32,
                (uv0 >> 16) as i32,
                &mut dst[bottom_dst + (2 * x - 1) * xstep..],
            );
            func(
                bottom_y[2 * x] as i32,
                (uv1 & 0xff) as i32,
                (uv1 >> 16) as i32,
                &mut dst[bottom_dst + (2 * x) * xstep..],
            );
        }
        tl_uv = t_uv;
        l_uv = uv;
    }
    if len & 1 == 0 {
        {
            let uv0 = (3 * tl_uv + l_uv + 0x00020002) >> 2;
            func(
                top_y[len - 1] as i32,
                (uv0 & 0xff) as i32,
                (uv0 >> 16) as i32,
                &mut dst[top_dst + (len - 1) * xstep..],
            );
        }
        if let (Some(bottom_y), Some(bottom_dst)) = (l.bottom_y, l.bottom_dst) {
            let uv0 = (3 * l_uv + tl_uv + 0x00020002) >> 2;
            func(
                bottom_y[len - 1] as i32,
                (uv0 & 0xff) as i32,
                (uv0 >> 16) as i32,
                &mut dst[bottom_dst + (len - 1) * xstep..],
            );
        }
    }
}

/// Fancy upsampling functions to convert YUV to RGB. Translation of
/// `WebPUpsamplers[mode](...)`.
pub(crate) fn webp_upsample(mode: WebpCspMode, l: &LinePair<'_>, dst: &mut [u8], len: i32) {
    let (func, xstep): (YuvToPixel, usize) = match mode {
        WebpCspMode::Rgba | WebpCspMode::RgbAPremultiplied => (vp8_yuv_to_rgba, 4),
        WebpCspMode::Bgra | WebpCspMode::BgrAPremultiplied => (vp8_yuv_to_bgra, 4),
        WebpCspMode::Argb | WebpCspMode::ArgbPremultiplied => (vp8_yuv_to_argb, 4),
        WebpCspMode::Rgb => (vp8_yuv_to_rgb, 3),
        WebpCspMode::Bgr => (vp8_yuv_to_bgr, 3),
        _ => unreachable!("the 16-bit upsamplers are not translated"),
    };
    upsample_func(func, xstep, l, dst, len);
}
