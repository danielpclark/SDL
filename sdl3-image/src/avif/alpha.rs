// Rust translation of src/alpha.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2020 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Alpha: filling and copying (with depth rescaling) an alpha channel
//! into RGB pixels, and premultiplying or unpremultiplying RGB pixels.
//! The destination samples above 8 bits are native-endian `u16`s in the
//! byte buffer, as upstream's `uint16_t` stores.

use super::avif::{avif_rgb_format_has_alpha, AvifResult, AvifRgbFormat, AvifRgbImage};
use super::internal::{AlphaSrc, AvifAlphaParams};
use super::utils::avif_roundf;

/// A `uint16_t` load from a byte buffer.
fn ld16(p: &[u8], i: usize) -> u16 {
    u16::from_ne_bytes([p[i], p[i + 1]])
}

/// A `uint16_t` store into a byte buffer.
fn st16(p: &mut [u8], i: usize, v: u16) {
    p[i..i + 2].copy_from_slice(&v.to_ne_bytes());
}

/// Translation of `avifFillAlpha()`.
pub(crate) fn avif_fill_alpha(params: &mut AvifAlphaParams<'_, '_>) {
    if params.dst_depth > 8 {
        let max_channel = ((1u32 << params.dst_depth) - 1) as u16;
        for j in 0..params.height as usize {
            let mut dst_row =
                params.dst_offset_bytes as usize + (j * params.dst_row_bytes as usize);
            for _ in 0..params.width {
                st16(params.dst_plane, dst_row, max_channel);
                dst_row += params.dst_pixel_bytes as usize;
            }
        }
    } else {
        // In this case, (1 << params->dstDepth) - 1 is always equal to 255.
        let max_channel: u8 = 255;
        for j in 0..params.height as usize {
            let mut dst_row =
                params.dst_offset_bytes as usize + (j * params.dst_row_bytes as usize);
            for _ in 0..params.width {
                params.dst_plane[dst_row] = max_channel;
                dst_row += params.dst_pixel_bytes as usize;
            }
        }
    }
}

/// Translation of `avifReformatAlpha()`.
pub(crate) fn avif_reformat_alpha(params: &mut AvifAlphaParams<'_, '_>) {
    let src_max_channel = (1i32 << params.src_depth) - 1;
    let dst_max_channel = (1i32 << params.dst_depth) - 1;
    let src_max_channel_f = src_max_channel as f32;
    let dst_max_channel_f = dst_max_channel as f32;

    let src_row_bytes = params.src_row_bytes as usize;
    let src_offset = params.src_offset_bytes as usize;
    let src_pixel_bytes = params.src_pixel_bytes as usize;
    let dst_row_bytes = params.dst_row_bytes as usize;
    let dst_offset = params.dst_offset_bytes as usize;
    let dst_pixel_bytes = params.dst_pixel_bytes as usize;
    // (a 16-bit sample at byte offset k of the source)
    let src16 = |k: usize| -> i32 {
        match params.src_plane {
            AlphaSrc::U16(p) => p[k / 2] as i32,
            _ => 0,
        }
    };
    let src8 = |k: usize| -> i32 {
        match params.src_plane {
            AlphaSrc::U8(p) => p[k] as i32,
            _ => 0,
        }
    };
    let dst = &mut *params.dst_plane;

    if params.src_depth == params.dst_depth {
        // no depth rescale

        if params.src_depth > 8 {
            // no depth rescale, uint16_t -> uint16_t

            for j in 0..params.height as usize {
                let src_row = src_offset + (j * src_row_bytes);
                let dst_row = dst_offset + (j * dst_row_bytes);
                for i in 0..params.width as usize {
                    st16(
                        dst,
                        dst_row + i * dst_pixel_bytes,
                        src16(src_row + i * src_pixel_bytes) as u16,
                    );
                }
            }
        } else {
            // no depth rescale, uint8_t -> uint8_t

            for j in 0..params.height as usize {
                let src_row = src_offset + (j * src_row_bytes);
                let dst_row = dst_offset + (j * dst_row_bytes);
                for i in 0..params.width as usize {
                    dst[dst_row + i * dst_pixel_bytes] = src8(src_row + i * src_pixel_bytes) as u8;
                }
            }
        }
    } else {
        // depth rescale

        if params.src_depth > 8 {
            if params.dst_depth > 8 {
                // depth rescale, uint16_t -> uint16_t

                for j in 0..params.height as usize {
                    let src_row = src_offset + (j * src_row_bytes);
                    let dst_row = dst_offset + (j * dst_row_bytes);
                    for i in 0..params.width as usize {
                        let src_alpha = src16(src_row + i * src_pixel_bytes);
                        let alpha_f = src_alpha as f32 / src_max_channel_f;
                        let mut dst_alpha = (0.5f32 + (alpha_f * dst_max_channel_f)) as i32;
                        dst_alpha = dst_alpha.clamp(0, dst_max_channel);
                        st16(dst, dst_row + i * dst_pixel_bytes, dst_alpha as u16);
                    }
                }
            } else {
                // depth rescale, uint16_t -> uint8_t

                for j in 0..params.height as usize {
                    let src_row = src_offset + (j * src_row_bytes);
                    let dst_row = dst_offset + (j * dst_row_bytes);
                    for i in 0..params.width as usize {
                        let src_alpha = src16(src_row + i * src_pixel_bytes);
                        let alpha_f = src_alpha as f32 / src_max_channel_f;
                        let mut dst_alpha = (0.5f32 + (alpha_f * dst_max_channel_f)) as i32;
                        dst_alpha = dst_alpha.clamp(0, dst_max_channel);
                        dst[dst_row + i * dst_pixel_bytes] = dst_alpha as u8;
                    }
                }
            }
        } else {
            // If (srcDepth == 8), dstDepth must be >8 otherwise we'd be in the (params->srcDepth == params->dstDepth) block above.
            debug_assert!(params.dst_depth > 8);

            // depth rescale, uint8_t -> uint16_t
            for j in 0..params.height as usize {
                let src_row = src_offset + (j * src_row_bytes);
                let dst_row = dst_offset + (j * dst_row_bytes);
                for i in 0..params.width as usize {
                    let src_alpha = src8(src_row + i * src_pixel_bytes);
                    let alpha_f = src_alpha as f32 / src_max_channel_f;
                    let mut dst_alpha = (0.5f32 + (alpha_f * dst_max_channel_f)) as i32;
                    dst_alpha = dst_alpha.clamp(0, dst_max_channel);
                    st16(dst, dst_row + i * dst_pixel_bytes, dst_alpha as u16);
                }
            }
        }
    }
}

/// Premultiply handling functions. Translation of
/// `avifRGBImagePremultiplyAlpha()` (`avifRGBImagePremultiplyAlphaLibYUV()`
/// is `AVIF_RESULT_NOT_IMPLEMENTED` without libyuv).
pub(crate) fn avif_rgb_image_premultiply_alpha(rgb: &mut AvifRgbImage<'_>) -> AvifResult {
    // no data
    if rgb.pixels.is_none() || rgb.row_bytes == 0 {
        return AvifResult::ReformatFailed;
    }

    // no alpha.
    if !avif_rgb_format_has_alpha(rgb.format) {
        return AvifResult::InvalidArgument;
    }

    debug_assert!(rgb.depth >= 8 && rgb.depth <= 16);

    let max: u32 = (1u32 << rgb.depth) - 1;
    let max_f = max as f32;
    let (width, height, row_bytes, depth, format) = (
        rgb.width as usize,
        rgb.height as usize,
        rgb.row_bytes as usize,
        rgb.depth,
        rgb.format,
    );
    let Some(pixels) = rgb.pixels.as_deref_mut() else {
        return AvifResult::ReformatFailed;
    };

    if depth > 8 {
        if format == AvifRgbFormat::Rgba || format == AvifRgbFormat::Bgra {
            for j in 0..height {
                let row = j * row_bytes;
                for i in 0..width {
                    let pixel = row + i * 8;
                    let a = ld16(pixels, pixel + 6);
                    if a as u32 >= max {
                        // opaque is no-op
                        continue;
                    } else if a == 0 {
                        // result must be zero
                        st16(pixels, pixel, 0);
                        st16(pixels, pixel + 2, 0);
                        st16(pixels, pixel + 4, 0);
                    } else {
                        // a < maxF is always true now, so we don't need clamp here
                        for c in 0..3 {
                            let v = ld16(pixels, pixel + 2 * c);
                            st16(
                                pixels,
                                pixel + 2 * c,
                                avif_roundf(v as f32 * a as f32 / max_f) as u16,
                            );
                        }
                    }
                }
            }
        } else {
            for j in 0..height {
                let row = j * row_bytes;
                for i in 0..width {
                    let pixel = row + i * 8;
                    let a = ld16(pixels, pixel);
                    if a as u32 >= max {
                        continue;
                    } else if a == 0 {
                        st16(pixels, pixel + 2, 0);
                        st16(pixels, pixel + 4, 0);
                        st16(pixels, pixel + 6, 0);
                    } else {
                        for c in 1..4 {
                            let v = ld16(pixels, pixel + 2 * c);
                            st16(
                                pixels,
                                pixel + 2 * c,
                                avif_roundf(v as f32 * a as f32 / max_f) as u16,
                            );
                        }
                    }
                }
            }
        }
    } else if format == AvifRgbFormat::Rgba || format == AvifRgbFormat::Bgra {
        for j in 0..height {
            let row = j * row_bytes;
            for i in 0..width {
                let pixel = &mut pixels[row + i * 4..row + i * 4 + 4];
                let a = pixel[3];
                // uint8_t can't exceed 255
                if a as u32 == max {
                    continue;
                } else if a == 0 {
                    pixel[0] = 0;
                    pixel[1] = 0;
                    pixel[2] = 0;
                } else {
                    pixel[0] = avif_roundf(pixel[0] as f32 * a as f32 / max_f) as u8;
                    pixel[1] = avif_roundf(pixel[1] as f32 * a as f32 / max_f) as u8;
                    pixel[2] = avif_roundf(pixel[2] as f32 * a as f32 / max_f) as u8;
                }
            }
        }
    } else {
        for j in 0..height {
            let row = j * row_bytes;
            for i in 0..width {
                let pixel = &mut pixels[row + i * 4..row + i * 4 + 4];
                let a = pixel[0];
                if a as u32 == max {
                    continue;
                } else if a == 0 {
                    pixel[1] = 0;
                    pixel[2] = 0;
                    pixel[3] = 0;
                } else {
                    pixel[1] = avif_roundf(pixel[1] as f32 * a as f32 / max_f) as u8;
                    pixel[2] = avif_roundf(pixel[2] as f32 * a as f32 / max_f) as u8;
                    pixel[3] = avif_roundf(pixel[3] as f32 * a as f32 / max_f) as u8;
                }
            }
        }
    }

    AvifResult::Ok
}

/// Translation of `avifRGBImageUnpremultiplyAlpha()`
/// (`avifRGBImageUnpremultiplyAlphaLibYUV()` is
/// `AVIF_RESULT_NOT_IMPLEMENTED` without libyuv).
pub(crate) fn avif_rgb_image_unpremultiply_alpha(rgb: &mut AvifRgbImage<'_>) -> AvifResult {
    // no data
    if rgb.pixels.is_none() || rgb.row_bytes == 0 {
        return AvifResult::ReformatFailed;
    }

    // no alpha.
    if !avif_rgb_format_has_alpha(rgb.format) {
        return AvifResult::ReformatFailed;
    }

    debug_assert!(rgb.depth >= 8 && rgb.depth <= 16);

    let max: u32 = (1u32 << rgb.depth) - 1;
    let max_f = max as f32;
    let (width, height, row_bytes, depth, format) = (
        rgb.width as usize,
        rgb.height as usize,
        rgb.row_bytes as usize,
        rgb.depth,
        rgb.format,
    );
    let Some(pixels) = rgb.pixels.as_deref_mut() else {
        return AvifResult::ReformatFailed;
    };

    if depth > 8 {
        if format == AvifRgbFormat::Rgba || format == AvifRgbFormat::Bgra {
            for j in 0..height {
                let row = j * row_bytes;
                for i in 0..width {
                    let pixel = row + i * 8;
                    let a = ld16(pixels, pixel + 6);
                    if a as u32 >= max {
                        // opaque is no-op
                        continue;
                    } else if a == 0 {
                        // prevent division by zero
                        st16(pixels, pixel, 0);
                        st16(pixels, pixel + 2, 0);
                        st16(pixels, pixel + 4, 0);
                    } else {
                        let c1 = avif_roundf(ld16(pixels, pixel) as f32 * max_f / a as f32);
                        let c2 = avif_roundf(ld16(pixels, pixel + 2) as f32 * max_f / a as f32);
                        let c3 = avif_roundf(ld16(pixels, pixel + 4) as f32 * max_f / a as f32);
                        st16(pixels, pixel, min_f(c1, max_f) as u16);
                        st16(pixels, pixel + 2, min_f(c2, max_f) as u16);
                        st16(pixels, pixel + 4, min_f(c3, max_f) as u16);
                    }
                }
            }
        } else {
            for j in 0..height {
                let row = j * row_bytes;
                for i in 0..width {
                    let pixel = row + i * 8;
                    let a = ld16(pixels, pixel);
                    if a as u32 >= max {
                        continue;
                    } else if a == 0 {
                        st16(pixels, pixel + 2, 0);
                        st16(pixels, pixel + 4, 0);
                        st16(pixels, pixel + 6, 0);
                    } else {
                        let c1 = avif_roundf(ld16(pixels, pixel + 2) as f32 * max_f / a as f32);
                        let c2 = avif_roundf(ld16(pixels, pixel + 4) as f32 * max_f / a as f32);
                        let c3 = avif_roundf(ld16(pixels, pixel + 6) as f32 * max_f / a as f32);
                        st16(pixels, pixel + 2, min_f(c1, max_f) as u16);
                        st16(pixels, pixel + 4, min_f(c2, max_f) as u16);
                        st16(pixels, pixel + 6, min_f(c3, max_f) as u16);
                    }
                }
            }
        }
    } else if format == AvifRgbFormat::Rgba || format == AvifRgbFormat::Bgra {
        for j in 0..height {
            let row = j * row_bytes;
            for i in 0..width {
                let pixel = &mut pixels[row + i * 4..row + i * 4 + 4];
                let a = pixel[3];
                if a as u32 == max {
                    continue;
                } else if a == 0 {
                    pixel[0] = 0;
                    pixel[1] = 0;
                    pixel[2] = 0;
                } else {
                    let c1 = avif_roundf(pixel[0] as f32 * max_f / a as f32);
                    let c2 = avif_roundf(pixel[1] as f32 * max_f / a as f32);
                    let c3 = avif_roundf(pixel[2] as f32 * max_f / a as f32);
                    pixel[0] = min_f(c1, max_f) as u8;
                    pixel[1] = min_f(c2, max_f) as u8;
                    pixel[2] = min_f(c3, max_f) as u8;
                }
            }
        }
    } else {
        for j in 0..height {
            let row = j * row_bytes;
            for i in 0..width {
                let pixel = &mut pixels[row + i * 4..row + i * 4 + 4];
                let a = pixel[0];
                if a as u32 == max {
                    continue;
                } else if a == 0 {
                    pixel[1] = 0;
                    pixel[2] = 0;
                    pixel[3] = 0;
                } else {
                    let c1 = avif_roundf(pixel[1] as f32 * max_f / a as f32);
                    let c2 = avif_roundf(pixel[2] as f32 * max_f / a as f32);
                    let c3 = avif_roundf(pixel[3] as f32 * max_f / a as f32);
                    pixel[1] = min_f(c1, max_f) as u8;
                    pixel[2] = min_f(c2, max_f) as u8;
                    pixel[3] = min_f(c3, max_f) as u8;
                }
            }
        }
    }

    AvifResult::Ok
}

/// `AVIF_MIN()` of two floats.
fn min_f(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
