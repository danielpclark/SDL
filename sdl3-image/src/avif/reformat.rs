// Rust translation of the YUV to RGB conversion of src/reformat.c from
// libavif (https://github.com/AOMediaCodec/libavif, at the revision
// SDL_image's external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! YUV to RGB: the built-in conversions (the "fast" paths and the slow one
//! that handles every combination, bilinear chroma upsampling and alpha
//! (un)multiplication), as libavif runs them built without libyuv, as
//! SDL_image builds it (`AVIF_LIBYUV` off: every `*LibYUV()` function is
//! `AVIF_RESULT_NOT_IMPLEMENTED`). RGB to YUV (`avifImageRGBToYUV()`) is
//! for encoding, which is not translated. The conversion jobs that
//! upstream may run on threads (`rgb->maxThreads`, 0 in SDL_image) run one
//! after the other here.

use super::alpha::{
    avif_fill_alpha, avif_reformat_alpha, avif_rgb_image_premultiply_alpha,
    avif_rgb_image_unpremultiply_alpha,
};
use super::avif::{
    avif_get_pixel_format_info, avif_image_set_view_rect, avif_rgb_format_channel_count,
    avif_rgb_format_has_alpha, avif_rgb_image_pixel_size, AvifChromaUpsampling, AvifCropRect,
    AvifImage, AvifPixelFormat, AvifRange, AvifResult, AvifRgbFormat, AvifRgbImage, AVIF_CHAN_U,
    AVIF_CHAN_V, AVIF_CHAN_Y, AVIF_MATRIX_COEFFICIENTS_BT2020_CL,
    AVIF_MATRIX_COEFFICIENTS_CHROMA_DERIVED_CL, AVIF_MATRIX_COEFFICIENTS_ICTCP,
    AVIF_MATRIX_COEFFICIENTS_IDENTITY, AVIF_MATRIX_COEFFICIENTS_LAST,
    AVIF_MATRIX_COEFFICIENTS_SMPTE2085, AVIF_MATRIX_COEFFICIENTS_YCGCO, AVIF_PIXEL_FORMAT_COUNT,
};
use super::colr::avif_calc_yuv_coefficients;
use super::internal::{
    avif_check, avif_checkerr, AlphaSrc, AvifAlphaMultiplyMode, AvifAlphaParams, AvifReformatMode,
    AvifReformatState, AvifRgbColorSpaceInfo, AvifYuvColorSpaceInfo,
};

/// A `uint16_t` store into a byte buffer.
fn st16(p: &mut [u8], i: usize, v: u16) {
    p[i..i + 2].copy_from_slice(&v.to_ne_bytes());
}

/// `AVIF_CLAMP(x, 0.0f, 1.0f)`
fn clamp01(x: f32) -> f32 {
    if x < 0.0f32 {
        0.0f32
    } else if 1.0f32 < x {
        1.0f32
    } else {
        x
    }
}

/// Translation of `avifGetRGBColorSpaceInfo()`.
pub(crate) fn avif_get_rgb_color_space_info(
    rgb: &AvifRgbImage<'_>,
    info: &mut AvifRgbColorSpaceInfo,
) -> bool {
    avif_check!(rgb.depth == 8 || rgb.depth == 10 || rgb.depth == 12 || rgb.depth == 16);
    if rgb.is_float {
        avif_check!(rgb.depth == 16);
    }
    if rgb.format == AvifRgbFormat::Rgb565 {
        avif_check!(rgb.depth == 8);
    }
    // Cast to silence "comparison of unsigned expression is always true" warning.
    // (the format is one of the enumeration's)

    info.channel_bytes = if rgb.depth > 8 { 2 } else { 1 };
    info.pixel_bytes = avif_rgb_image_pixel_size(rgb);

    match rgb.format {
        AvifRgbFormat::Rgb => {
            info.offset_bytes_r = info.channel_bytes * 0;
            info.offset_bytes_g = info.channel_bytes * 1;
            info.offset_bytes_b = info.channel_bytes * 2;
            info.offset_bytes_a = 0;
        }
        AvifRgbFormat::Rgba => {
            info.offset_bytes_r = info.channel_bytes * 0;
            info.offset_bytes_g = info.channel_bytes * 1;
            info.offset_bytes_b = info.channel_bytes * 2;
            info.offset_bytes_a = info.channel_bytes * 3;
        }
        AvifRgbFormat::Argb => {
            info.offset_bytes_a = info.channel_bytes * 0;
            info.offset_bytes_r = info.channel_bytes * 1;
            info.offset_bytes_g = info.channel_bytes * 2;
            info.offset_bytes_b = info.channel_bytes * 3;
        }
        AvifRgbFormat::Bgr => {
            info.offset_bytes_b = info.channel_bytes * 0;
            info.offset_bytes_g = info.channel_bytes * 1;
            info.offset_bytes_r = info.channel_bytes * 2;
            info.offset_bytes_a = 0;
        }
        AvifRgbFormat::Bgra => {
            info.offset_bytes_b = info.channel_bytes * 0;
            info.offset_bytes_g = info.channel_bytes * 1;
            info.offset_bytes_r = info.channel_bytes * 2;
            info.offset_bytes_a = info.channel_bytes * 3;
        }
        AvifRgbFormat::Abgr => {
            info.offset_bytes_a = info.channel_bytes * 0;
            info.offset_bytes_b = info.channel_bytes * 1;
            info.offset_bytes_g = info.channel_bytes * 2;
            info.offset_bytes_r = info.channel_bytes * 3;
        }
        AvifRgbFormat::Rgb565 => {
            // Since RGB_565 consists of two bytes per RGB pixel, we simply use
            // the pointer to the red channel to populate the entire pixel value
            // as a uint16_t. As a result only offsetBytesR is used and the
            // other offsets are unused.
            info.offset_bytes_r = 0;
            info.offset_bytes_g = 0;
            info.offset_bytes_b = 0;
            info.offset_bytes_a = 0;
        }
    }

    info.max_channel = (1i32 << rgb.depth) - 1;
    info.max_channel_f = info.max_channel as f32;

    true
}

/// Translation of `avifGetYUVColorSpaceInfo()`.
pub(crate) fn avif_get_yuv_color_space_info(
    image: &AvifImage,
    info: &mut AvifYuvColorSpaceInfo,
) -> bool {
    avif_check!(image.depth == 8 || image.depth == 10 || image.depth == 12 || image.depth == 16);
    avif_check!(
        image.yuv_format as u32 >= AvifPixelFormat::Yuv444 as u32
            && (image.yuv_format as u32) < AVIF_PIXEL_FORMAT_COUNT
    );
    avif_check!(image.yuv_range == AvifRange::Limited || image.yuv_range == AvifRange::Full);

    // These matrix coefficients values are currently unsupported. Revise this list as more support is added.
    //
    // YCgCo performs limited-full range adjustment on R,G,B but the current implementation performs range adjustment
    // on Y,U,V. So YCgCo with limited range is unsupported.
    if (image.matrix_coefficients == 3/* CICP reserved */)
        || ((image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_YCGCO)
            && (image.yuv_range == AvifRange::Limited))
        || (image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_BT2020_CL)
        || (image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_SMPTE2085)
        || (image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_CHROMA_DERIVED_CL)
        || (image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_ICTCP)
        || (image.matrix_coefficients >= AVIF_MATRIX_COEFFICIENTS_LAST)
    {
        return false;
    }

    if (image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_IDENTITY)
        && (image.yuv_format != AvifPixelFormat::Yuv444)
        && (image.yuv_format != AvifPixelFormat::Yuv400)
    {
        return false;
    }

    info.format_info = avif_get_pixel_format_info(image.yuv_format);
    (info.kr, info.kg, info.kb) = avif_calc_yuv_coefficients(image);

    info.channel_bytes = if image.depth > 8 { 2 } else { 1 };

    info.depth = image.depth;
    info.range = image.yuv_range;
    info.max_channel = (1i32 << image.depth) - 1;
    info.bias_y = if info.range == AvifRange::Limited {
        (16i32 << (info.depth - 8)) as f32
    } else {
        0.0f32
    };
    info.bias_uv = (1i32 << (info.depth - 1)) as f32;
    info.range_y = (if info.range == AvifRange::Limited {
        219i32 << (info.depth - 8)
    } else {
        info.max_channel
    }) as f32;
    info.range_uv = (if info.range == AvifRange::Limited {
        224i32 << (info.depth - 8)
    } else {
        info.max_channel
    }) as f32;

    true
}

/// Translation of `avifPrepareReformatState()`.
fn avif_prepare_reformat_state(
    image: &AvifImage,
    rgb: &AvifRgbImage<'_>,
    state: &mut AvifReformatState,
) -> bool {
    avif_check!(avif_get_rgb_color_space_info(rgb, &mut state.rgb));
    avif_check!(avif_get_yuv_color_space_info(image, &mut state.yuv));

    state.yuv.mode = AvifReformatMode::YuvCoefficients;

    if image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_IDENTITY {
        state.yuv.mode = AvifReformatMode::Identity;
    } else if image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_YCGCO {
        state.yuv.mode = AvifReformatMode::Ycgco;
    }

    if state.yuv.mode != AvifReformatMode::YuvCoefficients {
        state.yuv.kr = 0.0f32;
        state.yuv.kg = 0.0f32;
        state.yuv.kb = 0.0f32;
    }

    true
}

// (avifYUVColorSpaceInfoYToUNorm(), avifYUVColorSpaceInfoUVToUNorm() and
// avifImageRGBToYUV() are for encoding: not translated.)

/// Allocates and fills look-up tables for going from YUV limited/full unorm -> full range RGB FP32.
/// Review this when implementing YCgCo limited range support.
///
/// Translation of `avifCreateYUVToRGBLookUpTables()` (the UV table is
/// `None` when not asked for; for the identity mode it is a copy of the Y
/// table, which upstream shares).
fn avif_create_yuv_to_rgb_look_up_tables(
    with_uv: bool,
    depth: u32,
    state: &AvifReformatState,
) -> Option<(Vec<f32>, Vec<f32>)> {
    let cp_count = 1usize << depth;

    let mut unorm_float_table_y = Vec::new();
    unorm_float_table_y.try_reserve_exact(cp_count).ok()?;
    for cp in 0..cp_count as u32 {
        unorm_float_table_y.push((cp as f32 - state.yuv.bias_y) / state.yuv.range_y);
    }

    let mut unorm_float_table_uv = Vec::new();
    if with_uv {
        if state.yuv.mode == AvifReformatMode::Identity {
            // Just reuse the luma table since the chroma values are the same.
            unorm_float_table_uv = unorm_float_table_y.clone();
        } else {
            unorm_float_table_uv.try_reserve_exact(cp_count).ok()?;
            for cp in 0..cp_count as u32 {
                unorm_float_table_uv.push((cp as f32 - state.yuv.bias_uv) / state.yuv.range_uv);
            }
        }
    }
    Some((unorm_float_table_y, unorm_float_table_uv))
}

// (avifFreeYUVToRGBLookUpTables() is dropping the tables)

/// `RGB565()`
fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((b as u16) >> 3) | (((g as u16) >> 2) << 5) | (((r as u16) >> 3) << 11)
}

/// Translation of `avifStoreRGB8Pixel()`: the pixel's channels are at
/// byte offsets `ptr_r`, `ptr_g` and `ptr_b`.
#[allow(clippy::too_many_arguments)]
fn avif_store_rgb8_pixel(
    format: AvifRgbFormat,
    r: u8,
    g: u8,
    b: u8,
    pixels: &mut [u8],
    ptr_r: usize,
    ptr_g: usize,
    ptr_b: usize,
) {
    if format == AvifRgbFormat::Rgb565 {
        // References for RGB565 color conversion:
        // * https://docs.microsoft.com/en-us/windows/win32/directshow/working-with-16-bit-rgb
        // * https://chromium.googlesource.com/libyuv/libyuv/+/9892d70c965678381d2a70a1c9002d1cf136ee78/source/row_common.cc#2362
        st16(pixels, ptr_r, rgb565(r, g, b));
        return;
    }
    pixels[ptr_r] = r;
    pixels[ptr_g] = g;
    pixels[ptr_b] = b;
}

// (avifGetRGB565() is for encoding: not translated.)

/// The 8-bit samples of plane `c` of `image` (empty without the plane).
fn plane8(image: &AvifImage, c: usize) -> &[u8] {
    image.yuv_planes[c].as_ref().map_or(&[], |p| p.u8s())
}

/// The 16-bit samples of plane `c` of `image` (empty without the plane).
fn plane16(image: &AvifImage, c: usize) -> &[u16] {
    image.yuv_planes[c].as_ref().map_or(&[], |p| p.u16s())
}

/// Note: This function handles alpha (un)multiply. Translation of
/// `avifImageYUVAnyToRGBAnySlow()`.
fn avif_image_yuv_any_to_rgb_any_slow(
    image: &AvifImage,
    rgb: &mut AvifRgbImage<'_>,
    state: &AvifReformatState,
    alpha_multiply_mode: AvifAlphaMultiplyMode,
) -> AvifResult {
    // Aliases for some state
    let kr = state.yuv.kr;
    let kg = state.yuv.kg;
    let kb = state.yuv.kb;
    let tables = avif_create_yuv_to_rgb_look_up_tables(true, image.depth, state);
    avif_checkerr!(tables.is_some(), AvifResult::OutOfMemory);
    let Some((unorm_float_table_y, unorm_float_table_uv)) = tables else {
        return AvifResult::OutOfMemory;
    };
    let yuv_channel_bytes = state.yuv.channel_bytes as isize;
    let rgb_pixel_bytes = state.rgb.pixel_bytes as usize;

    // Aliases for plane data
    let y_plane8 = plane8(image, AVIF_CHAN_Y);
    let u_plane8 = plane8(image, AVIF_CHAN_U);
    let v_plane8 = plane8(image, AVIF_CHAN_V);
    let y_plane16 = plane16(image, AVIF_CHAN_Y);
    let u_plane16 = plane16(image, AVIF_CHAN_U);
    let v_plane16 = plane16(image, AVIF_CHAN_V);
    let a_plane = image.alpha_plane.as_ref();
    let y_row_bytes = image.yuv_row_bytes[AVIF_CHAN_Y] as usize;
    let u_row_bytes = image.yuv_row_bytes[AVIF_CHAN_U] as usize;
    let v_row_bytes = image.yuv_row_bytes[AVIF_CHAN_V] as usize;
    let a_row_bytes = image.alpha_row_bytes as usize;

    // Various observations and limits
    let has_color = image.yuv_planes[AVIF_CHAN_U].is_some()
        && image.yuv_planes[AVIF_CHAN_V].is_some()
        && (image.yuv_format != AvifPixelFormat::Yuv400);
    let yuv_max_channel = state.yuv.max_channel as u16;
    let rgb_max_channel_f = state.rgb.max_channel_f;

    // If toRGBAlphaMode is active (not no-op), assert that the alpha plane is present. The end of
    // the avifPrepareReformatState() function should ensure this, but this assert makes it clear
    // to clang's analyzer.
    debug_assert!((alpha_multiply_mode == AvifAlphaMultiplyMode::NoOp) || a_plane.is_some());

    let (rgb_row_bytes, rgb_format, rgb_depth, chroma_upsampling) = (
        rgb.row_bytes as usize,
        rgb.format,
        rgb.depth,
        rgb.chroma_upsampling,
    );
    let Some(pixels) = rgb.pixels.as_deref_mut() else {
        return AvifResult::ReformatFailed;
    };

    // (a 16-bit sample at byte offset k of a plane)
    let rd16 = |p: &[u16], k: isize| -> u16 { p[(k >> 1) as usize] };

    for j in 0..image.height {
        // uvJ is used only when hasColor is true.
        let uv_j = if has_color {
            j >> state.yuv.format_info.chroma_shift_y
        } else {
            0
        } as usize;
        let j_us = j as usize;
        let ptr_a = j_us * a_row_bytes;

        let mut ptr_r = state.rgb.offset_bytes_r as usize + (j_us * rgb_row_bytes);
        let mut ptr_g = state.rgb.offset_bytes_g as usize + (j_us * rgb_row_bytes);
        let mut ptr_b = state.rgb.offset_bytes_b as usize + (j_us * rgb_row_bytes);

        for i in 0..image.width {
            let i_us = i as usize;
            let y;
            let mut cb = 0.5f32;
            let mut cr = 0.5f32;

            // Calculate Y
            let unorm_y: u16 = if image.depth == 8 {
                y_plane8[j_us * y_row_bytes + i_us] as u16
            } else {
                // clamp incoming data to protect against bad LUT lookups
                y_plane16[(j_us * y_row_bytes) / 2 + i_us].min(yuv_max_channel)
            };
            y = unorm_float_table_y[unorm_y as usize];

            // Calculate Cb and Cr
            if has_color {
                let uv_i = (i >> state.yuv.format_info.chroma_shift_x) as usize;
                if image.yuv_format == AvifPixelFormat::Yuv444 {
                    let unorm_u: u16;
                    let unorm_v: u16;

                    if image.depth == 8 {
                        unorm_u = u_plane8[uv_j * u_row_bytes + uv_i] as u16;
                        unorm_v = v_plane8[uv_j * v_row_bytes + uv_i] as u16;
                    } else {
                        // clamp incoming data to protect against bad LUT lookups
                        unorm_u = u_plane16[(uv_j * u_row_bytes) / 2 + uv_i].min(yuv_max_channel);
                        unorm_v = v_plane16[(uv_j * v_row_bytes) / 2 + uv_i].min(yuv_max_channel);
                    }

                    cb = unorm_float_table_uv[unorm_u as usize];
                    cr = unorm_float_table_uv[unorm_v as usize];
                } else {
                    // Upsample to 444:
                    //
                    // *   *   *   *
                    //   A       B
                    // *   1   2   *
                    //
                    // *   3   4   *
                    //   C       D
                    // *   *   *   *
                    //
                    // When converting from YUV420 to RGB, for any given "high-resolution" RGB
                    // coordinate (1,2,3,4,*), there are up to four "low-resolution" UV samples
                    // (A,B,C,D) that are "nearest" to the pixel. For RGB pixel #1, A is the closest
                    // UV sample, B and C are "adjacent" to it on the same row and column, and D is
                    // the diagonal. For RGB pixel 3, C is the closest UV sample, A and D are
                    // adjacent, and B is the diagonal. Sometimes the adjacent pixel on the same row
                    // is to the left or right, and sometimes the adjacent pixel on the same column
                    // is up or down. For any edge or corner, there might only be only one or two
                    // samples nearby, so they'll be duplicated.
                    //
                    // The following code attempts to find all four nearest UV samples and put them
                    // in the following unormU and unormV grid as follows:
                    //
                    // unorm[0][0] = closest         ( weights: bilinear: 9/16, nearest: 1 )
                    // unorm[1][0] = adjacent col    ( weights: bilinear: 3/16, nearest: 0 )
                    // unorm[0][1] = adjacent row    ( weights: bilinear: 3/16, nearest: 0 )
                    // unorm[1][1] = diagonal        ( weights: bilinear: 1/16, nearest: 0 )
                    //
                    // It then weights them according to the requested upsampling set in avifRGBImage.

                    let mut unorm_u = [[0u16; 2]; 2];
                    let mut unorm_v = [[0u16; 2]; 2];

                    // How many bytes to add to a uint8_t pointer index to get to the adjacent (lesser) sample in a given direction
                    let u_adj_col: isize;
                    let v_adj_col: isize;
                    let u_adj_row: isize;
                    let v_adj_row: isize;
                    if (i == 0) || ((i == (image.width - 1)) && ((i % 2) != 0)) {
                        u_adj_col = 0;
                        v_adj_col = 0;
                    } else if (i % 2) != 0 {
                        u_adj_col = yuv_channel_bytes;
                        v_adj_col = yuv_channel_bytes;
                    } else {
                        u_adj_col = -yuv_channel_bytes;
                        v_adj_col = -yuv_channel_bytes;
                    }

                    // For YUV422, uvJ will always be a fresh value (always corresponds to j), so
                    // we'll simply duplicate the sample as if we were on the top or bottom row and
                    // it'll behave as plain old linear (1D) upsampling, which is all we want.
                    if (j == 0)
                        || ((j == (image.height - 1)) && ((j % 2) != 0))
                        || (image.yuv_format == AvifPixelFormat::Yuv422)
                    {
                        u_adj_row = 0;
                        v_adj_row = 0;
                    } else if (j % 2) != 0 {
                        u_adj_row = u_row_bytes as isize;
                        v_adj_row = v_row_bytes as isize;
                    } else {
                        u_adj_row = -(u_row_bytes as isize);
                        v_adj_row = -(v_row_bytes as isize);
                    }

                    let u_base = (uv_j * u_row_bytes) as isize + uv_i as isize * yuv_channel_bytes;
                    let v_base = (uv_j * v_row_bytes) as isize + uv_i as isize * yuv_channel_bytes;
                    if image.depth == 8 {
                        let rd8 = |p: &[u8], k: isize| -> u16 { p[k as usize] as u16 };
                        unorm_u[0][0] = rd8(u_plane8, u_base);
                        unorm_v[0][0] = rd8(v_plane8, v_base);
                        unorm_u[1][0] = rd8(u_plane8, u_base + u_adj_col);
                        unorm_v[1][0] = rd8(v_plane8, v_base + v_adj_col);
                        unorm_u[0][1] = rd8(u_plane8, u_base + u_adj_row);
                        unorm_v[0][1] = rd8(v_plane8, v_base + v_adj_row);
                        unorm_u[1][1] = rd8(u_plane8, u_base + u_adj_col + u_adj_row);
                        unorm_v[1][1] = rd8(v_plane8, v_base + v_adj_col + v_adj_row);
                    } else {
                        unorm_u[0][0] = rd16(u_plane16, u_base);
                        unorm_v[0][0] = rd16(v_plane16, v_base);
                        unorm_u[1][0] = rd16(u_plane16, u_base + u_adj_col);
                        unorm_v[1][0] = rd16(v_plane16, v_base + v_adj_col);
                        unorm_u[0][1] = rd16(u_plane16, u_base + u_adj_row);
                        unorm_v[0][1] = rd16(v_plane16, v_base + v_adj_row);
                        unorm_u[1][1] = rd16(u_plane16, u_base + u_adj_col + u_adj_row);
                        unorm_v[1][1] = rd16(v_plane16, v_base + v_adj_col + v_adj_row);

                        // clamp incoming data to protect against bad LUT lookups
                        for b_j in 0..2 {
                            for b_i in 0..2 {
                                unorm_u[b_i][b_j] = unorm_u[b_i][b_j].min(yuv_max_channel);
                                unorm_v[b_i][b_j] = unorm_v[b_i][b_j].min(yuv_max_channel);
                            }
                        }
                    }

                    if (chroma_upsampling == AvifChromaUpsampling::Fastest)
                        || (chroma_upsampling == AvifChromaUpsampling::Nearest)
                    {
                        // Nearest neighbor; ignore all UVs but the closest one
                        cb = unorm_float_table_uv[unorm_u[0][0] as usize];
                        cr = unorm_float_table_uv[unorm_v[0][0] as usize];
                    } else {
                        // Bilinear filtering with weights
                        let t = &unorm_float_table_uv;
                        cb = (t[unorm_u[0][0] as usize] * (9.0f32 / 16.0f32))
                            + (t[unorm_u[1][0] as usize] * (3.0f32 / 16.0f32))
                            + (t[unorm_u[0][1] as usize] * (3.0f32 / 16.0f32))
                            + (t[unorm_u[1][1] as usize] * (1.0f32 / 16.0f32));
                        cr = (t[unorm_v[0][0] as usize] * (9.0f32 / 16.0f32))
                            + (t[unorm_v[1][0] as usize] * (3.0f32 / 16.0f32))
                            + (t[unorm_v[0][1] as usize] * (3.0f32 / 16.0f32))
                            + (t[unorm_v[1][1] as usize] * (1.0f32 / 16.0f32));
                    }
                }
            }

            let r;
            let g;
            let b;
            if has_color {
                if state.yuv.mode == AvifReformatMode::Identity {
                    // Identity (GBR): Formulas 41,42,43 from https://www.itu.int/rec/T-REC-H.273-201612-I/en
                    g = y;
                    b = cb;
                    r = cr;
                } else if state.yuv.mode == AvifReformatMode::Ycgco {
                    // YCgCo: Formulas 47,48,49,50 from https://www.itu.int/rec/T-REC-H.273-201612-I/en
                    let t = y - cb;
                    g = y + cb;
                    b = t - cr;
                    r = t + cr;
                } else {
                    // Normal YUV
                    r = y + (2.0f32 * (1.0f32 - kr)) * cr;
                    b = y + (2.0f32 * (1.0f32 - kb)) * cb;
                    g = y
                        - ((2.0f32 * ((kr * (1.0f32 - kr) * cr) + (kb * (1.0f32 - kb) * cb))) / kg);
                }
            } else {
                // Monochrome: just populate all channels with luma (state->yuv.mode is irrelevant)
                r = y;
                g = y;
                b = y;
            }

            let mut rc = clamp01(r);
            let mut gc = clamp01(g);
            let mut bc = clamp01(b);

            if alpha_multiply_mode != AvifAlphaMultiplyMode::NoOp {
                // Calculate A
                let unorm_a: u16 = match a_plane {
                    Some(a) if image.depth == 8 => a.u8s()[ptr_a + i_us] as u16,
                    Some(a) => a.u16s()[ptr_a / 2 + i_us].min(yuv_max_channel),
                    None => 0,
                };
                let a = unorm_a as f32 / (state.yuv.max_channel as f32);
                let ac = clamp01(a);

                if alpha_multiply_mode == AvifAlphaMultiplyMode::Multiply {
                    if ac == 0.0f32 {
                        rc = 0.0f32;
                        gc = 0.0f32;
                        bc = 0.0f32;
                    } else if ac < 1.0f32 {
                        rc *= ac;
                        gc *= ac;
                        bc *= ac;
                    }
                } else {
                    // alphaMultiplyMode == AVIF_ALPHA_MULTIPLY_MODE_UNMULTIPLY
                    if ac == 0.0f32 {
                        rc = 0.0f32;
                        gc = 0.0f32;
                        bc = 0.0f32;
                    } else if ac < 1.0f32 {
                        rc /= ac;
                        gc /= ac;
                        bc /= ac;
                        rc = rc.min(1.0f32);
                        gc = gc.min(1.0f32);
                        bc = bc.min(1.0f32);
                    }
                }
            }

            if rgb_depth == 8 {
                avif_store_rgb8_pixel(
                    rgb_format,
                    (0.5f32 + (rc * rgb_max_channel_f)) as u8,
                    (0.5f32 + (gc * rgb_max_channel_f)) as u8,
                    (0.5f32 + (bc * rgb_max_channel_f)) as u8,
                    pixels,
                    ptr_r,
                    ptr_g,
                    ptr_b,
                );
            } else {
                st16(pixels, ptr_r, (0.5f32 + (rc * rgb_max_channel_f)) as u16);
                st16(pixels, ptr_g, (0.5f32 + (gc * rgb_max_channel_f)) as u16);
                st16(pixels, ptr_b, (0.5f32 + (bc * rgb_max_channel_f)) as u16);
            }
            ptr_r += rgb_pixel_bytes;
            ptr_g += rgb_pixel_bytes;
            ptr_b += rgb_pixel_bytes;
        }
    }
    AvifResult::Ok
}

/// The YUV to RGB conversion of the "fast" paths: the source is 8-bit or
/// 16-bit (`Y8`/`Y16`), the destination 8-bit or 16-bit, color or
/// monochrome. One function for upstream's eight
/// `avifImageYUV{8,16}ToRGB{8,16}{Color,Mono}()`, whose loops are the same
/// but for the sample types (the 8-bit sources don't clamp: "the full
/// uint8_t range is a legal lookup").
fn avif_image_yuv_to_rgb_fast(
    image: &AvifImage,
    rgb: &mut AvifRgbImage<'_>,
    state: &AvifReformatState,
    yuv16: bool,
    color: bool,
) -> AvifResult {
    let kr = state.yuv.kr;
    let kg = state.yuv.kg;
    let kb = state.yuv.kb;
    let rgb_pixel_bytes = state.rgb.pixel_bytes as usize;
    let tables = avif_create_yuv_to_rgb_look_up_tables(color, image.depth, state);
    avif_checkerr!(tables.is_some(), AvifResult::OutOfMemory);
    let Some((unorm_float_table_y, unorm_float_table_uv)) = tables else {
        return AvifResult::OutOfMemory;
    };

    let yuv_max_channel = state.yuv.max_channel as u16;
    let rgb_max_channel_f = state.rgb.max_channel_f;
    let (rgb_row_bytes, rgb_format, rgb_depth) = (rgb.row_bytes as usize, rgb.format, rgb.depth);
    let Some(pixels) = rgb.pixels.as_deref_mut() else {
        return AvifResult::ReformatFailed;
    };
    let rows = [
        image.yuv_row_bytes[AVIF_CHAN_Y] as usize,
        image.yuv_row_bytes[AVIF_CHAN_U] as usize,
        image.yuv_row_bytes[AVIF_CHAN_V] as usize,
    ];
    let (y8, u8p, v8) = (
        plane8(image, AVIF_CHAN_Y),
        plane8(image, AVIF_CHAN_U),
        plane8(image, AVIF_CHAN_V),
    );
    let (y16, u16p, v16) = (
        plane16(image, AVIF_CHAN_Y),
        plane16(image, AVIF_CHAN_U),
        plane16(image, AVIF_CHAN_V),
    );
    for j in 0..image.height {
        let uv_j = (j >> state.yuv.format_info.chroma_shift_y) as usize;
        let j_us = j as usize;
        let mut ptr_r = state.rgb.offset_bytes_r as usize + (j_us * rgb_row_bytes);
        let mut ptr_g = state.rgb.offset_bytes_g as usize + (j_us * rgb_row_bytes);
        let mut ptr_b = state.rgb.offset_bytes_b as usize + (j_us * rgb_row_bytes);

        for i in 0..image.width {
            let uv_i = (i >> state.yuv.format_info.chroma_shift_x) as usize;
            let i_us = i as usize;

            let (y, cb, cr);
            if yuv16 {
                // clamp incoming data to protect against bad LUT lookups
                let unorm_y = y16[(j_us * rows[0]) / 2 + i_us].min(yuv_max_channel);
                // Convert unorm to float
                y = unorm_float_table_y[unorm_y as usize];
                if color {
                    let unorm_u = u16p[(uv_j * rows[1]) / 2 + uv_i].min(yuv_max_channel);
                    let unorm_v = v16[(uv_j * rows[2]) / 2 + uv_i].min(yuv_max_channel);
                    cb = unorm_float_table_uv[unorm_u as usize];
                    cr = unorm_float_table_uv[unorm_v as usize];
                } else {
                    cb = 0.0f32;
                    cr = 0.0f32;
                }
            } else {
                // Convert unorm to float (no clamp necessary, the full uint8_t range is a legal lookup)
                y = unorm_float_table_y[y8[j_us * rows[0] + i_us] as usize];
                if color {
                    cb = unorm_float_table_uv[u8p[uv_j * rows[1] + uv_i] as usize];
                    cr = unorm_float_table_uv[v8[uv_j * rows[2] + uv_i] as usize];
                } else {
                    cb = 0.0f32;
                    cr = 0.0f32;
                }
            }

            let r = y + (2.0f32 * (1.0f32 - kr)) * cr;
            let b = y + (2.0f32 * (1.0f32 - kb)) * cb;
            let g = y - ((2.0f32 * ((kr * (1.0f32 - kr) * cr) + (kb * (1.0f32 - kb) * cb))) / kg);
            let rc = clamp01(r);
            let gc = clamp01(g);
            let bc = clamp01(b);

            if rgb_depth > 8 {
                st16(pixels, ptr_r, (0.5f32 + (rc * rgb_max_channel_f)) as u16);
                st16(pixels, ptr_g, (0.5f32 + (gc * rgb_max_channel_f)) as u16);
                st16(pixels, ptr_b, (0.5f32 + (bc * rgb_max_channel_f)) as u16);
            } else {
                avif_store_rgb8_pixel(
                    rgb_format,
                    (0.5f32 + (rc * rgb_max_channel_f)) as u8,
                    (0.5f32 + (gc * rgb_max_channel_f)) as u8,
                    (0.5f32 + (bc * rgb_max_channel_f)) as u8,
                    pixels,
                    ptr_r,
                    ptr_g,
                    ptr_b,
                );
            }

            ptr_r += rgb_pixel_bytes;
            ptr_g += rgb_pixel_bytes;
            ptr_b += rgb_pixel_bytes;
        }
    }
    AvifResult::Ok
}

/// Translation of `avifImageIdentity8ToRGB8ColorFullRange()`.
fn avif_image_identity8_to_rgb8_color_full_range(
    image: &AvifImage,
    rgb: &mut AvifRgbImage<'_>,
    state: &AvifReformatState,
) -> AvifResult {
    let rgb_pixel_bytes = state.rgb.pixel_bytes as usize;
    let (rgb_row_bytes, rgb_format) = (rgb.row_bytes as usize, rgb.format);
    let Some(pixels) = rgb.pixels.as_deref_mut() else {
        return AvifResult::ReformatFailed;
    };
    let (y8, u8p, v8) = (
        plane8(image, AVIF_CHAN_Y),
        plane8(image, AVIF_CHAN_U),
        plane8(image, AVIF_CHAN_V),
    );
    for j in 0..image.height as usize {
        let ptr_y = &y8[(j * image.yuv_row_bytes[AVIF_CHAN_Y] as usize)..];
        let ptr_u = &u8p[(j * image.yuv_row_bytes[AVIF_CHAN_U] as usize)..];
        let ptr_v = &v8[(j * image.yuv_row_bytes[AVIF_CHAN_V] as usize)..];
        let mut ptr_r = state.rgb.offset_bytes_r as usize + (j * rgb_row_bytes);
        let mut ptr_g = state.rgb.offset_bytes_g as usize + (j * rgb_row_bytes);
        let mut ptr_b = state.rgb.offset_bytes_b as usize + (j * rgb_row_bytes);

        // This is intentionally a per-row conditional instead of a per-pixel
        // conditional. This makes the "else" path (much more common than the
        // "if" path) much faster than having a per-pixel branch.
        if rgb_format == AvifRgbFormat::Rgb565 {
            for i in 0..image.width as usize {
                st16(pixels, ptr_r, rgb565(ptr_v[i], ptr_y[i], ptr_u[i]));
                ptr_r += rgb_pixel_bytes;
            }
        } else {
            for i in 0..image.width as usize {
                pixels[ptr_r] = ptr_v[i];
                pixels[ptr_g] = ptr_y[i];
                pixels[ptr_b] = ptr_u[i];
                ptr_r += rgb_pixel_bytes;
                ptr_g += rgb_pixel_bytes;
                ptr_b += rgb_pixel_bytes;
            }
        }
    }
    AvifResult::Ok
}

// This constant comes from libyuv. For details, see here:
// https://chromium.googlesource.com/libyuv/libyuv/+/2f87e9a7/source/row_common.cc#3537
const F16_MULTIPLIER: f32 = 1.9259299444e-34f32;

/// Translation of `avifRGBImageToF16()` (`avifRGBImageToF16LibYUV()` is
/// `AVIF_RESULT_NOT_IMPLEMENTED` without libyuv).
fn avif_rgb_image_to_f16(rgb: &mut AvifRgbImage<'_>) -> AvifResult {
    let channel_count = avif_rgb_format_channel_count(rgb.format) as usize;
    let scale = 1.0f32 / (((1i32 << rgb.depth) - 1) as f32);
    let multiplier = F16_MULTIPLIER * scale;
    let stride = (rgb.row_bytes >> 1) as usize;
    let (width, height) = (rgb.width as usize, rgb.height as usize);
    let Some(pixels) = rgb.pixels.as_deref_mut() else {
        return AvifResult::Ok;
    };
    for j in 0..height {
        for i in 0..width * channel_count {
            let k = (j * stride + i) * 2;
            let v = u16::from_ne_bytes([pixels[k], pixels[k + 1]]);
            let f = v as f32 * multiplier;
            st16(pixels, k, (f.to_bits() >> 13) as u16);
        }
    }
    AvifResult::Ok
}

/// Translation of `avifImageYUVToRGBImpl()`.
fn avif_image_yuv_to_rgb_impl(
    image: &AvifImage,
    rgb: &mut AvifRgbImage<'_>,
    state: &AvifReformatState,
    mut alpha_multiply_mode: AvifAlphaMultiplyMode,
) -> AvifResult {
    let converted_with_lib_yuv = false;
    // Reformat alpha, if user asks for it, or (un)multiply processing needs it.
    let reformat_alpha = avif_rgb_format_has_alpha(rgb.format)
        && (!rgb.ignore_alpha || (alpha_multiply_mode != AvifAlphaMultiplyMode::NoOp));
    // This value is used only when reformatAlpha is true.
    // (avifImageYUVToRGBLibYUV() is AVIF_RESULT_NOT_IMPLEMENTED without libyuv)
    let alpha_reformatted_with_lib_yuv = false;

    if reformat_alpha && !alpha_reformatted_with_lib_yuv {
        let (width, height, depth, row_bytes) = (rgb.width, rgb.height, rgb.depth, rgb.row_bytes);
        if let Some(pixels) = rgb.pixels.as_deref_mut() {
            let mut params = AvifAlphaParams {
                width,
                height,
                dst_depth: depth,
                dst_plane: pixels,
                dst_row_bytes: row_bytes,
                dst_offset_bytes: state.rgb.offset_bytes_a,
                dst_pixel_bytes: state.rgb.pixel_bytes,
                src_depth: 0,
                src_plane: AlphaSrc::None,
                src_row_bytes: 0,
                src_offset_bytes: 0,
                src_pixel_bytes: 0,
            };

            match &image.alpha_plane {
                Some(alpha) if image.alpha_row_bytes != 0 => {
                    params.src_depth = image.depth;
                    params.src_plane = if image.depth > 8 {
                        AlphaSrc::U16(alpha.u16s())
                    } else {
                        AlphaSrc::U8(alpha.u8s())
                    };
                    params.src_row_bytes = image.alpha_row_bytes;
                    params.src_offset_bytes = 0;
                    params.src_pixel_bytes = state.yuv.channel_bytes;

                    avif_reformat_alpha(&mut params);
                }
                _ => avif_fill_alpha(&mut params),
            }
        }
    }

    if !converted_with_lib_yuv {
        // libyuv is either unavailable or unable to perform the specific conversion required here.
        // Look over the available built-in "fast" routines for YUV->RGB conversion and see if one
        // fits the current combination, or as a last resort, call avifImageYUVAnyToRGBAnySlow(),
        // which handles every possibly YUV->RGB combination, but very slowly (in comparison).

        let mut convert_result = AvifResult::NotImplemented;

        let has_color = image.yuv_row_bytes[AVIF_CHAN_U] != 0
            && image.yuv_row_bytes[AVIF_CHAN_V] != 0
            && (image.yuv_format != AvifPixelFormat::Yuv400);

        if (!has_color
            || (image.yuv_format == AvifPixelFormat::Yuv444)
            || ((rgb.chroma_upsampling == AvifChromaUpsampling::Fastest)
                || (rgb.chroma_upsampling == AvifChromaUpsampling::Nearest)))
            && (alpha_multiply_mode == AvifAlphaMultiplyMode::NoOp
                || avif_rgb_format_has_alpha(rgb.format))
        {
            // Explanations on the above conditional:
            // * None of these fast paths currently support bilinear upsampling, so avoid all of them
            //   unless the YUV data isn't subsampled or they explicitly requested AVIF_CHROMA_UPSAMPLING_NEAREST.
            // * None of these fast paths currently handle alpha (un)multiply, so avoid all of them
            //   if we can't do alpha (un)multiply as a separated post step (destination format doesn't have alpha).

            if state.yuv.mode == AvifReformatMode::Identity {
                if (image.depth == 8)
                    && (rgb.depth == 8)
                    && (image.yuv_format == AvifPixelFormat::Yuv444)
                    && (image.yuv_range == AvifRange::Full)
                {
                    convert_result =
                        avif_image_identity8_to_rgb8_color_full_range(image, rgb, state);
                }

                // TODO: Add more fast paths for identity
            } else if state.yuv.mode == AvifReformatMode::YuvCoefficients {
                // yuv:u16 or yuv:u8, rgb:u16 or rgb:u8, color or mono
                // (avifImageYUV{16,8}ToRGB{16,8}{Color,Mono}())
                convert_result =
                    avif_image_yuv_to_rgb_fast(image, rgb, state, image.depth > 8, has_color);
            }
        }

        if convert_result == AvifResult::NotImplemented {
            // If we get here, there is no fast path for this combination. Time to be slow!
            convert_result =
                avif_image_yuv_any_to_rgb_any_slow(image, rgb, state, alpha_multiply_mode);

            // The slow path also handles alpha (un)multiply, so forget the operation here.
            alpha_multiply_mode = AvifAlphaMultiplyMode::NoOp;
        }

        if convert_result != AvifResult::Ok {
            return convert_result;
        }
    }

    // Process alpha premultiplication, if necessary
    if alpha_multiply_mode == AvifAlphaMultiplyMode::Multiply {
        let result = avif_rgb_image_premultiply_alpha(rgb);
        if result != AvifResult::Ok {
            return result;
        }
    } else if alpha_multiply_mode == AvifAlphaMultiplyMode::Unmultiply {
        let result = avif_rgb_image_unpremultiply_alpha(rgb);
        if result != AvifResult::Ok {
            return result;
        }
    }

    // Convert pixels to half floats (F16), if necessary.
    if rgb.is_float {
        return avif_rgb_image_to_f16(rgb);
    }

    AvifResult::Ok
}

/// The main conversion function. Translation of `avifImageYUVToRGB()`.
pub(crate) fn avif_image_yuv_to_rgb(image: &AvifImage, rgb: &mut AvifRgbImage<'_>) -> AvifResult {
    // It is okay for rgb->maxThreads to be equal to zero in order to allow clients to zero initialize the avifRGBImage struct
    // with memset.
    if image.yuv_planes[AVIF_CHAN_Y].is_none() || rgb.max_threads < 0 {
        return AvifResult::ReformatFailed;
    }

    let mut state = AvifReformatState::default();
    if !avif_prepare_reformat_state(image, rgb, &mut state) {
        return AvifResult::ReformatFailed;
    }

    let mut alpha_multiply_mode = AvifAlphaMultiplyMode::NoOp;
    if image.alpha_plane.is_some() {
        if !avif_rgb_format_has_alpha(rgb.format) || rgb.ignore_alpha {
            // if we are converting some image with alpha into a format without alpha, we should do 'premultiply alpha' before
            // discarding alpha plane. This has the same effect of rendering this image on a black background, which makes sense.
            if !image.alpha_premultiplied {
                alpha_multiply_mode = AvifAlphaMultiplyMode::Multiply;
            }
        } else if !image.alpha_premultiplied && rgb.alpha_premultiplied {
            alpha_multiply_mode = AvifAlphaMultiplyMode::Multiply;
        } else if image.alpha_premultiplied && !rgb.alpha_premultiplied {
            alpha_multiply_mode = AvifAlphaMultiplyMode::Unmultiply;
        }
    }

    // In practice, we rarely need more than 8 threads for YUV to RGB conversion.
    let mut jobs = rgb.max_threads.clamp(1, 8) as u32;

    // When yuv format is 420 and chromaUpsampling could be BILINEAR, there is a dependency across the horizontal borders of each
    // job. So we disallow multithreading in that case.
    if image.yuv_format == AvifPixelFormat::Yuv420
        && (rgb.chroma_upsampling == AvifChromaUpsampling::Automatic
            || rgb.chroma_upsampling == AvifChromaUpsampling::BestQuality
            || rgb.chroma_upsampling == AvifChromaUpsampling::Bilinear)
    {
        jobs = 1;
    }

    // Each thread worker needs at least 2 Y rows (to account for potential U/V subsampling).
    if jobs == 1 || (image.height / 2) < jobs {
        return avif_image_yuv_to_rgb_impl(image, rgb, &state, alpha_multiply_mode);
    }

    // (the jobs, which upstream runs on threads, one after the other)
    let mut rows_per_job = image.height / jobs;
    if rows_per_job % 2 != 0 {
        rows_per_job += 1;
        jobs = (image.height + rows_per_job - 1) / rows_per_job; // ceil
    }
    let rows_for_last_job = image.height - rows_per_job * (jobs - 1);
    let mut start_row = 0u32;
    let mut result = AvifResult::Ok;
    let row_bytes = rgb.row_bytes as usize;
    let Some(pixels) = rgb.pixels.as_deref_mut() else {
        return AvifResult::ReformatFailed;
    };
    for i in 0..jobs {
        let rect = AvifCropRect {
            x: 0,
            y: start_row,
            width: image.width,
            height: if i == jobs - 1 {
                rows_for_last_job
            } else {
                rows_per_job
            },
        };
        let mut tdata_image = AvifImage::default();
        if avif_image_set_view_rect(&mut tdata_image, image, &rect) != AvifResult::Ok {
            result = AvifResult::ReformatFailed;
            break;
        }

        let mut tdata_rgb = AvifRgbImage {
            width: rgb.width,
            height: tdata_image.height,
            depth: rgb.depth,
            format: rgb.format,
            chroma_upsampling: rgb.chroma_upsampling,
            chroma_downsampling: rgb.chroma_downsampling,
            avoid_lib_yuv: rgb.avoid_lib_yuv,
            ignore_alpha: rgb.ignore_alpha,
            alpha_premultiplied: rgb.alpha_premultiplied,
            is_float: rgb.is_float,
            max_threads: rgb.max_threads,
            pixels: pixels.get_mut(start_row as usize * row_bytes..),
            row_bytes: rgb.row_bytes,
        };
        let job_result =
            avif_image_yuv_to_rgb_impl(&tdata_image, &mut tdata_rgb, &state, alpha_multiply_mode);
        if job_result != AvifResult::Ok {
            result = job_result;
        }
        start_row += rows_per_job;
    }
    result
}

// Limited -> Full
// Plan: subtract limited offset, then multiply by ratio of FULLSIZE/LIMITEDSIZE (rounding), then clamp.
// RATIO = (FULLY - 0) / (MAXLIMITEDY - MINLIMITEDY)
// -----------------------------------------
// ( ( (v - MINLIMITEDY)                    | subtract limited offset
//     * FULLY                              | multiply numerator of ratio
//   ) + ((MAXLIMITEDY - MINLIMITEDY) / 2)  | add 0.5 (half of denominator) to round
// ) / (MAXLIMITEDY - MINLIMITEDY)          | divide by denominator of ratio
// AVIF_CLAMP(v, 0, FULLY)                  | clamp to full range
// -----------------------------------------
/// `LIMITED_TO_FULL()`
fn limited_to_full(v: i32, min_limited_y: i32, max_limited_y: i32, full_y: i32) -> i32 {
    let v = (((v - min_limited_y) * full_y) + ((max_limited_y - min_limited_y) / 2))
        / (max_limited_y - min_limited_y);
    v.clamp(0, full_y)
}

/// Translation of `avifLimitedToFullY()`.
pub(crate) fn avif_limited_to_full_y(depth: u32, v: i32) -> i32 {
    match depth {
        8 => limited_to_full(v, 16, 235, 255),
        10 => limited_to_full(v, 64, 940, 1023),
        12 => limited_to_full(v, 256, 3760, 4095),
        _ => v,
    }
}

/// Translation of `avifLimitedToFullUV()`.
pub(crate) fn avif_limited_to_full_uv(depth: u32, v: i32) -> i32 {
    match depth {
        8 => limited_to_full(v, 16, 240, 255),
        10 => limited_to_full(v, 64, 960, 1023),
        12 => limited_to_full(v, 256, 3840, 4095),
        _ => v,
    }
}

// (FULL_TO_LIMITED(), avifFullToLimitedY(), avifFullToLimitedUV(),
// avifGetRGBAPixel() and avifSetRGBAPixel() are for encoding: not
// translated.)
