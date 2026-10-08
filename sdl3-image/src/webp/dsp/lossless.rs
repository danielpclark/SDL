// Rust translation of src/dsp/lossless.c, lossless.h and
// lossless_common.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the decoder's functions.
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Image transforms and color space conversion methods for lossless
//! decoder.
//!
//! The inverse transforms work in place here: the rows are in `buf` at
//! `out` (the decoder copies the transform's input there first), with the
//! row above them at `out - width` (upstream's `out - width` too: the
//! argb cache's top-prediction row). Upstream's in-place color indexing
//! (moving the packed pixels to the end of the unpacked region first) is
//! then the only path; it gives what reading another buffer gives. The
//! premultiplied and 16-bit colorspace conversions are not translated.

use crate::webp::dec::vp8l_dec::VP8LTransform;
use crate::webp::decode::{VP8LImageTransformType, WebpCspMode, ARGB_BLACK};

//------------------------------------------------------------------------------
// lossless_common.h

// color mapping related functions.

/// Translation of `VP8GetARGBIndex()`.
fn vp8_get_argb_index(idx: u32) -> u32 {
    (idx >> 8) & 0xff
}

/// Translation of `VP8GetAlphaValue()`.
fn vp8_get_alpha_value(val: u32) -> u8 {
    ((val >> 8) & 0xff) as u8
}

/// Computes sampled size of 'size' when sampling using 'sampling bits'.
/// Translation of `VP8LSubSampleSize()`.
pub(crate) fn vp8l_sub_sample_size(size: u32, sampling_bits: u32) -> u32 {
    (size + (1 << sampling_bits) - 1) >> sampling_bits
}

/// Sum of each component, mod 256. Translation of `VP8LAddPixels()`.
pub(crate) fn vp8l_add_pixels(a: u32, b: u32) -> u32 {
    let alpha_and_green = (a & 0xff00ff00).wrapping_add(b & 0xff00ff00);
    let red_and_blue = (a & 0x00ff00ff).wrapping_add(b & 0x00ff00ff);
    (alpha_and_green & 0xff00ff00) | (red_and_blue & 0x00ff00ff)
}

//------------------------------------------------------------------------------
// Image transforms.

/// Translation of `Average2()`.
fn average2(a0: u32, a1: u32) -> u32 {
    (((a0 ^ a1) & 0xfefefefe) >> 1) + (a0 & a1)
}

/// Translation of `Average3()`.
fn average3(a0: u32, a1: u32, a2: u32) -> u32 {
    average2(average2(a0, a2), a1)
}

/// Translation of `Average4()`.
fn average4(a0: u32, a1: u32, a2: u32, a3: u32) -> u32 {
    average2(average2(a0, a1), average2(a2, a3))
}

/// Translation of `Clip255()`.
fn clip255(a: u32) -> u32 {
    if a < 256 {
        return a;
    }
    // return 0, when a is a negative integer.
    // return 255, when a is positive.
    !a >> 24
}

/// Translation of `AddSubtractComponentFull()`.
fn add_subtract_component_full(a: i32, b: i32, c: i32) -> i32 {
    clip255((a + b - c) as u32) as i32
}

/// Translation of `ClampedAddSubtractFull()`.
fn clamped_add_subtract_full(c0: u32, c1: u32, c2: u32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as i32;
    let a = add_subtract_component_full(ch(c0, 24), ch(c1, 24), ch(c2, 24));
    let r = add_subtract_component_full(ch(c0, 16), ch(c1, 16), ch(c2, 16));
    let g = add_subtract_component_full(ch(c0, 8), ch(c1, 8), ch(c2, 8));
    let b = add_subtract_component_full(ch(c0, 0), ch(c1, 0), ch(c2, 0));
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// Translation of `AddSubtractComponentHalf()`.
fn add_subtract_component_half(a: i32, b: i32) -> i32 {
    clip255((a + (a - b) / 2) as u32) as i32
}

/// Translation of `ClampedAddSubtractHalf()`.
fn clamped_add_subtract_half(c0: u32, c1: u32, c2: u32) -> u32 {
    let ave = average2(c0, c1);
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as i32;
    let a = add_subtract_component_half(ch(ave, 24), ch(c2, 24));
    let r = add_subtract_component_half(ch(ave, 16), ch(c2, 16));
    let g = add_subtract_component_half(ch(ave, 8), ch(c2, 8));
    let b = add_subtract_component_half(ch(ave, 0), ch(c2, 0));
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// Translation of `Sub3()`.
fn sub3(a: i32, b: i32, c: i32) -> i32 {
    let pb = b - c;
    let pa = a - c;
    pb.abs() - pa.abs()
}

/// Translation of `Select()`.
fn select(a: u32, b: u32, c: u32) -> u32 {
    let ch = |v: u32, s: u32| ((v >> s) & 0xff) as i32;
    let pa_minus_pb = sub3(ch(a, 24), ch(b, 24), ch(c, 24))
        + sub3(ch(a, 16), ch(b, 16), ch(c, 16))
        + sub3(ch(a, 8), ch(b, 8), ch(c, 8))
        + sub3(ch(a, 0), ch(b, 0), ch(c, 0));
    if pa_minus_pb <= 0 {
        a
    } else {
        b
    }
}

//------------------------------------------------------------------------------
// Predictors

/// The predictors `VP8LPredictor0_C()` to `VP8LPredictor13_C()` (14 and 15
/// are padding security sentinels, as 0): the prediction from the pixel
/// to the left and the top-left, top and top-right ones.
pub(crate) fn vp8l_predictor(mode: u32, left: u32, tl: u32, t: u32, tr: u32) -> u32 {
    match mode {
        1 => left,
        2 => t,
        3 => tr,
        4 => tl,
        5 => average3(left, t, tr),
        6 => average2(left, tl),
        7 => average2(left, t),
        8 => average2(tl, t),
        9 => average2(t, tr),
        10 => average4(left, tl, t, tr),
        11 => select(t, left, tl),
        12 => clamped_add_subtract_full(left, t, tl),
        13 => clamped_add_subtract_half(left, t, tl),
        _ => ARGB_BLACK,
    }
}

/// The batch predictors `PredictorAdd0_C()` to `PredictorAdd13_C()`
/// (`VP8LPredictorsAdd[mode]`), in place: `num_pixels` residuals at `out`
/// in `buf` become pixels, predicted from the row at `upper`.
fn predictor_add(mode: u32, buf: &mut [u32], out: usize, upper: usize, num_pixels: usize) {
    for x in 0..num_pixels {
        let left = buf[out + x - 1];
        let pred = if mode == 0 || mode >= 14 {
            ARGB_BLACK
        } else {
            let u = upper + x;
            vp8l_predictor(mode, left, buf[u - 1], buf[u], buf[u + 1])
        };
        buf[out + x] = vp8l_add_pixels(buf[out + x], pred);
    }
}

//------------------------------------------------------------------------------

/// Inverse prediction. Translation of `PredictorInverseTransform_C()`.
fn predictor_inverse_transform(
    transform: &VP8LTransform,
    mut y_start: i32,
    y_end: i32,
    buf: &mut [u32],
    mut out: usize,
) {
    let width = transform.xsize as usize;
    if y_start == 0 {
        // First Row follows the L (mode=1) mode.
        buf[out] = vp8l_add_pixels(buf[out], ARGB_BLACK);
        let mut left = buf[out];
        for i in 1..width {
            left = vp8l_add_pixels(buf[out + i], left);
            buf[out + i] = left;
        }
        out += width;
        y_start += 1;
    }

    {
        let mut y = y_start;
        let tile_width = 1usize << transform.bits;
        let mask = tile_width as i32 - 1;
        let tiles_per_row = vp8l_sub_sample_size(width as u32, transform.bits as u32) as usize;
        let mut pred_mode_base = (y >> transform.bits) as usize * tiles_per_row;

        while y < y_end {
            let mut pred_mode_src = pred_mode_base;
            let mut x = 1;
            // First pixel follows the T (mode=2) mode.
            buf[out] = vp8l_add_pixels(buf[out], buf[out - width]);
            // .. the rest:
            while x < width {
                let mode = (transform.data[pred_mode_src] >> 8) & 0xf;
                pred_mode_src += 1;
                let mut x_end = (x & !(mask as usize)) + tile_width;
                if x_end > width {
                    x_end = width;
                }
                predictor_add(mode, buf, out + x, out + x - width, x_end - x);
                x = x_end;
            }
            out += width;
            y += 1;
            if (y & mask) == 0 {
                // Use the same mask, since tiles are squares.
                pred_mode_base += tiles_per_row;
            }
        }
    }
}

/// Add green to blue and red channels (i.e. perform the inverse transform
/// of 'subtract green'). Translation of `VP8LAddGreenToBlueAndRed_C()`.
fn vp8l_add_green_to_blue_and_red(buf: &mut [u32]) {
    for argb in buf {
        let green = (*argb >> 8) & 0xff;
        let mut red_blue = *argb & 0x00ff00ff;
        red_blue = red_blue.wrapping_add((green << 16) | green);
        red_blue &= 0x00ff00ff;
        *argb = (*argb & 0xff00ff00) | red_blue;
    }
}

/// Translation of `ColorTransformDelta()`.
fn color_transform_delta(color_pred: i8, color: i8) -> i32 {
    (color_pred as i32 * color as i32) >> 5
}

/// Translation of `VP8LMultipliers`.
#[derive(Clone, Copy, Default)]
struct VP8LMultipliers {
    // Note: the members are uint8_t, so that any negative values are
    // automatically converted to "mod 256" values.
    green_to_red: u8,
    green_to_blue: u8,
    red_to_blue: u8,
}

/// Translation of `ColorCodeToMultipliers()`.
fn color_code_to_multipliers(color_code: u32) -> VP8LMultipliers {
    VP8LMultipliers {
        green_to_red: (color_code & 0xff) as u8,
        green_to_blue: ((color_code >> 8) & 0xff) as u8,
        red_to_blue: ((color_code >> 16) & 0xff) as u8,
    }
}

/// Translation of `VP8LTransformColorInverse_C()`.
fn vp8l_transform_color_inverse(m: &VP8LMultipliers, buf: &mut [u32]) {
    for argb in buf {
        let green = (*argb >> 8) as i8;
        let red = *argb >> 16;
        let mut new_red = (red & 0xff) as i32;
        let mut new_blue = (*argb & 0xff) as i32;
        new_red += color_transform_delta(m.green_to_red as i8, green);
        new_red &= 0xff;
        new_blue += color_transform_delta(m.green_to_blue as i8, green);
        new_blue += color_transform_delta(m.red_to_blue as i8, new_red as i8);
        new_blue &= 0xff;
        *argb = (*argb & 0xff00ff00) | ((new_red as u32) << 16) | new_blue as u32;
    }
}

/// Color space inverse transform. Translation of
/// `ColorSpaceInverseTransform_C()`.
fn color_space_inverse_transform(
    transform: &VP8LTransform,
    y_start: i32,
    y_end: i32,
    buf: &mut [u32],
    mut src: usize,
) {
    let width = transform.xsize as usize;
    let tile_width = 1usize << transform.bits;
    let mask = tile_width - 1;
    let safe_width = width & !mask;
    let remaining_width = width - safe_width;
    let tiles_per_row = vp8l_sub_sample_size(width as u32, transform.bits as u32) as usize;
    let mut y = y_start;
    let mut pred_row = (y >> transform.bits) as usize * tiles_per_row;

    while y < y_end {
        let mut pred = pred_row;
        let src_safe_end = src + safe_width;
        let src_end = src + width;
        while src < src_safe_end {
            let m = color_code_to_multipliers(transform.data[pred]);
            pred += 1;
            vp8l_transform_color_inverse(&m, &mut buf[src..src + tile_width]);
            src += tile_width;
        }
        if src < src_end {
            // Left-overs using C-version.
            let m = color_code_to_multipliers(transform.data[pred]);
            vp8l_transform_color_inverse(&m, &mut buf[src..src + remaining_width]);
            src += remaining_width;
        }
        y += 1;
        if (y as usize & mask) == 0 {
            pred_row += tiles_per_row;
        }
    }
}

/// Separate out pixels packed together using pixel-bundling: the ARGB
/// instance of the `COLOR_INDEX_INVERSE()` macro
/// (`ColorIndexInverseTransform_C()` and `MapARGB_C()`), reading the
/// indices at `src` and writing the pixels at `dst`, both in `buf`.
fn color_index_inverse_transform(
    transform: &VP8LTransform,
    y_start: i32,
    y_end: i32,
    buf: &mut [u32],
    mut src: usize,
    mut dst: usize,
) {
    let bits_per_pixel = 8 >> transform.bits;
    let width = transform.xsize as usize;
    let color_map = &transform.data;
    if bits_per_pixel < 8 {
        let pixels_per_byte = 1usize << transform.bits;
        let count_mask = pixels_per_byte - 1;
        let bit_mask: u32 = (1 << bits_per_pixel) - 1;
        for _ in y_start..y_end {
            let mut packed_pixels: u32 = 0;
            for x in 0..width {
                // We need to load fresh 'packed_pixels' once every
                // 'pixels_per_byte' increments of x. Fortunately, pixels_per_byte
                // is a power of 2, so can just use a mask for that, instead of
                // decrementing a counter.
                if (x & count_mask) == 0 {
                    packed_pixels = vp8_get_argb_index(buf[src]);
                    src += 1;
                }
                buf[dst] = color_map[(packed_pixels & bit_mask) as usize];
                dst += 1;
                packed_pixels >>= bits_per_pixel;
            }
        }
    } else {
        // VP8LMapColor32b: MapARGB_C()
        for _ in y_start..y_end {
            for _ in 0..width {
                buf[dst] = color_map[vp8_get_argb_index(buf[src]) as usize];
                dst += 1;
                src += 1;
            }
        }
    }
}

/// The alpha instance of the `COLOR_INDEX_INVERSE()` macro:
/// `VP8LColorIndexInverseTransformAlpha()` (and `MapAlpha_C()`).
pub(crate) fn vp8l_color_index_inverse_transform_alpha(
    transform: &VP8LTransform,
    y_start: i32,
    y_end: i32,
    src: &[u8],
    dst: &mut [u8],
) {
    let bits_per_pixel = 8 >> transform.bits;
    let width = transform.xsize as usize;
    let color_map = &transform.data;
    let mut s = 0;
    let mut d = 0;
    if bits_per_pixel < 8 {
        let pixels_per_byte = 1usize << transform.bits;
        let count_mask = pixels_per_byte - 1;
        let bit_mask: u32 = (1 << bits_per_pixel) - 1;
        for _ in y_start..y_end {
            let mut packed_pixels: u32 = 0;
            for x in 0..width {
                if (x & count_mask) == 0 {
                    packed_pixels = src[s] as u32;
                    s += 1;
                }
                dst[d] = vp8_get_alpha_value(color_map[(packed_pixels & bit_mask) as usize]);
                d += 1;
                packed_pixels >>= bits_per_pixel;
            }
        }
    } else {
        // VP8LMapColor8b: MapAlpha_C()
        for _ in y_start..y_end {
            for _ in 0..width {
                dst[d] = vp8_get_alpha_value(color_map[src[s] as usize]);
                d += 1;
                s += 1;
            }
        }
    }
}

/// Performs inverse transform of data given transform information, start
/// and end rows. Transform will be applied to rows [row_start, row_end[.
/// The rows are at `out` in `buf`, transformed in place, the row before
/// them being the predictor's top row. Translation of
/// `VP8LInverseTransform()`.
pub(crate) fn vp8l_inverse_transform(
    transform: &VP8LTransform,
    row_start: i32,
    row_end: i32,
    buf: &mut [u32],
    out: usize,
) {
    let width = transform.xsize as usize;
    debug_assert!(row_start < row_end);
    debug_assert!(row_end <= transform.ysize);
    let num_rows = (row_end - row_start) as usize;
    match transform.type_ {
        VP8LImageTransformType::SubtractGreenTransform => {
            vp8l_add_green_to_blue_and_red(&mut buf[out..out + num_rows * width]);
        }
        VP8LImageTransformType::PredictorTransform => {
            predictor_inverse_transform(transform, row_start, row_end, buf, out);
            if row_end != transform.ysize {
                // The last predicted row in this iteration will be the top-pred row
                // for the first row in next iteration.
                let last = out + (num_rows - 1) * width;
                buf.copy_within(last..last + width, out - width);
            }
        }
        VP8LImageTransformType::CrossColorTransform => {
            color_space_inverse_transform(transform, row_start, row_end, buf, out);
        }
        VP8LImageTransformType::ColorIndexingTransform => {
            if transform.bits > 0 {
                // Move packed pixels to the end of unpacked region, so that unpacking
                // can occur seamlessly.
                // Also, note that this is the only transform that applies on
                // the effective width of VP8LSubSampleSize(xsize_, bits_). All other
                // transforms work on effective width of xsize_.
                let out_stride = num_rows * width;
                let in_stride = num_rows
                    * vp8l_sub_sample_size(transform.xsize as u32, transform.bits as u32) as usize;
                let src = out + out_stride - in_stride;
                buf.copy_within(out..out + in_stride, src);
                color_index_inverse_transform(transform, row_start, row_end, buf, src, out);
            } else {
                color_index_inverse_transform(transform, row_start, row_end, buf, out, out);
            }
        }
    }
}

//------------------------------------------------------------------------------
// Color space conversion.

/// Translation of `VP8LConvertFromBGRA()`: `num_pixels` BGRA pixels (ARGB
/// words) to `rgba` in the output colorspace.
pub(crate) fn vp8l_convert_from_bgra(
    in_data: &[u32],
    num_pixels: usize,
    out_colorspace: WebpCspMode,
    rgba: &mut [u8],
) {
    let src = &in_data[..num_pixels];
    match out_colorspace {
        WebpCspMode::Rgb => {
            // VP8LConvertBGRAToRGB_C()
            for (argb, dst) in src.iter().zip(rgba.chunks_exact_mut(3)) {
                dst[0] = (argb >> 16) as u8;
                dst[1] = (argb >> 8) as u8;
                dst[2] = *argb as u8;
            }
        }
        WebpCspMode::Rgba => {
            // VP8LConvertBGRAToRGBA_C()
            for (argb, dst) in src.iter().zip(rgba.chunks_exact_mut(4)) {
                dst[0] = (argb >> 16) as u8;
                dst[1] = (argb >> 8) as u8;
                dst[2] = *argb as u8;
                dst[3] = (argb >> 24) as u8;
            }
        }
        WebpCspMode::Bgr => {
            // VP8LConvertBGRAToBGR_C()
            for (argb, dst) in src.iter().zip(rgba.chunks_exact_mut(3)) {
                dst[0] = *argb as u8;
                dst[1] = (argb >> 8) as u8;
                dst[2] = (argb >> 16) as u8;
            }
        }
        WebpCspMode::Bgra => {
            // CopyOrSwap(in_data, num_pixels, rgba, 1): the words' little-endian bytes
            for (argb, dst) in src.iter().zip(rgba.chunks_exact_mut(4)) {
                dst.copy_from_slice(&argb.to_le_bytes());
            }
        }
        WebpCspMode::Argb => {
            // CopyOrSwap(in_data, num_pixels, rgba, 0): the words' big-endian bytes
            for (argb, dst) in src.iter().zip(rgba.chunks_exact_mut(4)) {
                dst.copy_from_slice(&argb.to_be_bytes());
            }
        }
        _ => unreachable!("the premultiplied and 16-bit conversions are not translated"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_arithmetic_is_per_channel() {
        assert_eq!(vp8l_add_pixels(0xff01_02ff, 0x0101_0101), 0x0002_0300);
        assert_eq!(average2(0x0000_0002, 0x0000_0004), 3);
        assert_eq!(clip255(300), 255);
        assert_eq!(clip255(-5i32 as u32), 0);
        assert_eq!(select(1, 2, 1), 2);
        assert_eq!(vp8l_sub_sample_size(23, 2), 6);
    }
}
