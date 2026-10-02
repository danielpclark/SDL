// Rust translation of src/video/SDL_yuv.c and src/video/yuv2rgb/ from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// The yuv2rgb kernels: Copyright 2016 Adrien Descamps, distributed under the
// BSD 3-Clause License (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Conversions between YUV (FOURCC) formats and RGB, and between YUV formats.
//!
//! Upstream has portable and SSE2 YUV to RGB kernels. The SSE2 kernels do
//! their arithmetic in 16-bit lanes (wrapping) and saturate when packing,
//! while the portable kernels use 32-bit arithmetic and a lookup table that
//! wraps out of range values, so the two disagree on saturated colors. Both
//! are translated (the SSE2 one in portable code, lane by lane) and chosen
//! like upstream, by whether the CPU has SSE2. The SSE2 versions of the
//! YUV to YUV conversions only move bytes, so they produce the same results
//! as the portable versions and only those are translated. The LSX kernels
//! are not translated.
//!
//! Upstream converts in place when the source and destination pointers are
//! the same; the source and destination here are always distinct buffers.

use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::video::blit::simd_support;
use crate::video::pixels::{Colorspace, PixelFormat};
use crate::video::surface::convert_pixels_and_colorspace;

/*
 * Calculate YUV size and pitch. Check for overflow.
 * Output 'pitch' that can be used with SDL_ConvertPixels()
 */
// SDL_CalculateYUVSize() is in surface/mod.rs (calculate_yuv_size).

fn is_planar_1x1_format(format: PixelFormat) -> bool {
    format == PixelFormat::I444 || format == PixelFormat::I4FL
}

fn is_planar_2x2_format(format: PixelFormat) -> bool {
    format == PixelFormat::YV12
        || format == PixelFormat::IYUV
        || format == PixelFormat::NV12
        || format == PixelFormat::NV21
        || format == PixelFormat::P010
        || format == PixelFormat::I0FL
}

fn is_packed4_format(format: PixelFormat) -> bool {
    format == PixelFormat::YUY2 || format == PixelFormat::UYVY || format == PixelFormat::YVYU
}

// ---------------------------------------------------------------------------
// yuv2rgb/yuv_rgb_internal.h

const PRECISION: i32 = 6;
const PRECISION_FACTOR: i32 = 1 << PRECISION;

/// `YCbCrType` (yuv_rgb_common.h): the matrix and range of a conversion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum YCbCrType {
    Bt601Full,
    Bt601Limited,
    Bt709Full,
    Bt709Limited,
    Bt2020NclFull,
}

/// `YUV2RGBParam`.
///
/// ```text
/// |R|                        |y_factor      0       v_r_factor|   |Y-y_shift|
/// |G| = 1/PRECISION_FACTOR * |y_factor  u_g_factor  v_g_factor| * |  U-128  |
/// |B|                        |y_factor  u_b_factor      0     |   |  V-128  |
/// ```
struct Yuv2RgbParam {
    y_shift: u8,
    y_factor: i16,
    v_r_factor: i16,
    u_g_factor: i16,
    v_g_factor: i16,
    u_b_factor: i16,
}

/// `V(value)`: a factor in fixed point.
const fn v(value: f64) -> i16 {
    ((value * PRECISION_FACTOR as f64) + 0.5) as i16
}

// for ITU-T T.871, values can be found in section 7
// for ITU-R BT.601-7 values are derived from equations in sections 2.5.1-2.5.3, assuming RGB is encoded using full range ([0-1]<->[0-255])
// for ITU-R BT.709-6 values are derived from equations in sections 3.2-3.4, assuming RGB is encoded using full range ([0-1]<->[0-255])
// for ITU-R BT.2020 values are assuming RGB is encoded using full 10-bit range ([0-1]<->[0-1023])
// all values are rounded to the fourth decimal

static YUV2RGB: [Yuv2RgbParam; 5] = [
    // ITU-T T.871 (JPEG)
    Yuv2RgbParam {
        y_shift: 0,
        y_factor: v(1.0),
        v_r_factor: v(1.402),
        u_g_factor: -v(0.3441),
        v_g_factor: -v(0.7141),
        u_b_factor: v(1.772),
    },
    // ITU-R BT.601-7
    Yuv2RgbParam {
        y_shift: 16,
        y_factor: v(1.1644),
        v_r_factor: v(1.596),
        u_g_factor: -v(0.3918),
        v_g_factor: -v(0.813),
        u_b_factor: v(2.0172),
    },
    // ITU-R BT.709-6 full range
    Yuv2RgbParam {
        y_shift: 0,
        y_factor: v(1.0),
        v_r_factor: v(1.581),
        u_g_factor: -v(0.1881),
        v_g_factor: -v(0.47),
        u_b_factor: v(1.8629),
    },
    // ITU-R BT.709-6
    Yuv2RgbParam {
        y_shift: 16,
        y_factor: v(1.1644),
        v_r_factor: v(1.7927),
        u_g_factor: -v(0.2132),
        v_g_factor: -v(0.5329),
        u_b_factor: v(2.1124),
    },
    // ITU-R BT.2020 10-bit full range
    Yuv2RgbParam {
        y_shift: 0,
        y_factor: v(1.0),
        v_r_factor: v(1.4760),
        u_g_factor: -v(0.1647),
        v_g_factor: -v(0.5719),
        u_b_factor: v(1.8832),
    },
];

// The integer RGB2YUV table and rgb24_yuv420_std() only serve a disabled
// ("Doesn't handle odd widths") RGB24 fast path upstream, so they are not
// translated.

/* The various layouts of YUV data we support */
const YUV_FORMAT_420: u8 = 1;
const YUV_FORMAT_422: u8 = 2;
const YUV_FORMAT_444: u8 = 3;
const YUV_FORMAT_NV12: u8 = 4;

/* The various formats of RGB pixel that we support */
const RGB_FORMAT_RGB565: u8 = 1;
const RGB_FORMAT_RGB24: u8 = 2;
const RGB_FORMAT_RGBA: u8 = 3;
const RGB_FORMAT_BGRA: u8 = 4;
const RGB_FORMAT_ARGB: u8 = 5;
const RGB_FORMAT_ABGR: u8 = 6;
const RGB_FORMAT_XBGR2101010: u8 = 7;
const RGB_FORMAT_RGB48: u8 = 8;

/// The pixel strides and chroma subsampling of a YUV layout:
/// `(y_pixel_stride, uv_pixel_stride, uv_x_sample_interval, uv_y_sample_interval)`.
const fn layout_strides(layout: u8) -> (usize, usize, usize, usize) {
    match layout {
        YUV_FORMAT_420 => (1, 1, 2, 2),
        YUV_FORMAT_422 => (2, 4, 2, 1),
        YUV_FORMAT_444 => (1, 1, 1, 1),
        _ => (1, 2, 2, 2), // YUV_FORMAT_NV12
    }
}

const fn rgb_pixel_stride(rgb: u8) -> usize {
    match rgb {
        RGB_FORMAT_RGB565 => 2,
        RGB_FORMAT_RGB24 => 3,
        RGB_FORMAT_RGB48 => 6,
        _ => 4,
    }
}

// ---------------------------------------------------------------------------
// yuv2rgb/yuv_rgb_std.c

// divide by PRECISION_FACTOR and clamp to [0:255] interval
// input must be in the [-128*PRECISION_FACTOR:384*PRECISION_FACTOR] range
static CLAMP_U8_LUT: [u8; 512] = {
    let mut lut = [0u8; 512];
    let mut i = 128;
    while i < 512 {
        lut[i] = if i < 384 { (i - 128) as u8 } else { 255 };
        i += 1;
    }
    lut
};

#[inline(always)]
fn clamp_u8(v: i32) -> u8 {
    CLAMP_U8_LUT[(((v + 128 * PRECISION_FACTOR) >> PRECISION) & 511) as usize]
}

#[inline(always)]
fn clamp10(v: i32) -> u16 {
    (v >> PRECISION).clamp(0, 1023) as u16
}

#[inline(always)]
fn clamp16(v: i32) -> u16 {
    (v >> PRECISION).clamp(0, 0xffff) as u16
}

/// Write one pixel of an 8-bit-per-channel RGB format.
#[inline(always)]
fn write_rgb8<const RGB: u8>(dst: &mut [u8], pos: usize, r: u8, g: u8, b: u8) {
    let (r, g, b) = (r as u32, g as u32, b as u32);
    match RGB {
        RGB_FORMAT_RGB565 => {
            let p = ((r << 8) & 0xF800) | ((g << 3) & 0x07E0) | (b >> 3);
            dst[pos..pos + 2].copy_from_slice(&(p as u16).to_ne_bytes());
        }
        RGB_FORMAT_RGB24 => {
            dst[pos] = r as u8;
            dst[pos + 1] = g as u8;
            dst[pos + 2] = b as u8;
        }
        _ => {
            let p = match RGB {
                RGB_FORMAT_RGBA => (r << 24) | (g << 16) | (b << 8) | 0x000000FF,
                RGB_FORMAT_BGRA => (b << 24) | (g << 16) | (r << 8) | 0x000000FF,
                RGB_FORMAT_ARGB => 0xFF000000 | (r << 16) | (g << 8) | b,
                _ => 0xFF000000 | (b << 16) | (g << 8) | r, // RGB_FORMAT_ABGR
            };
            dst[pos..pos + 4].copy_from_slice(&p.to_ne_bytes());
        }
    }
}

/// `PACK_PIXEL` of the portable kernels.
#[inline(always)]
fn pack_pixel<const RGB: u8>(
    dst: &mut [u8],
    pos: usize,
    y_tmp: i32,
    r_tmp: i32,
    g_tmp: i32,
    b_tmp: i32,
) {
    match RGB {
        RGB_FORMAT_XBGR2101010 => {
            let p = 0xC0000000
                | ((clamp10(y_tmp + b_tmp) as u32) << 20)
                | ((clamp10(y_tmp + g_tmp) as u32) << 10)
                | (clamp10(y_tmp + r_tmp) as u32);
            dst[pos..pos + 4].copy_from_slice(&p.to_ne_bytes());
        }
        RGB_FORMAT_RGB48 => {
            dst[pos..pos + 2].copy_from_slice(&clamp16(y_tmp + r_tmp).to_ne_bytes());
            dst[pos + 2..pos + 4].copy_from_slice(&clamp16(y_tmp + g_tmp).to_ne_bytes());
            dst[pos + 4..pos + 6].copy_from_slice(&clamp16(y_tmp + b_tmp).to_ne_bytes());
        }
        _ => write_rgb8::<RGB>(
            dst,
            pos,
            clamp_u8(y_tmp + r_tmp),
            clamp_u8(y_tmp + g_tmp),
            clamp_u8(y_tmp + b_tmp),
        ),
    }
}

/// One YUV to RGB conversion: the source planes (byte offsets into `src`,
/// strides in bytes) and the destination (byte offset and stride).
#[derive(Clone, Copy)]
struct YuvToRgb<'a> {
    width: u32,
    height: u32,
    src: &'a [u8],
    y: usize,
    u: usize,
    v: usize,
    y_stride: usize,
    uv_stride: usize,
    rgb: usize,
    rgb_stride: usize,
    param: &'static Yuv2RgbParam,
}

type YuvToRgbKernel = fn(YuvToRgb<'_>, &mut [u8]);

/// The portable kernel template, `yuv_rgb_std_func.h`. `BITS` is the
/// sample size: 8, 10 (in the high bits of 16-bit samples) or 16.
fn yuv_rgb_std_kernel<const LAYOUT: u8, const RGB: u8, const BITS: u32>(
    job: YuvToRgb<'_>,
    rgb: &mut [u8],
) {
    let param = job.param;
    let (y_pixel_stride, uv_pixel_stride, uv_x_sample_interval, uv_y_sample_interval) =
        layout_strides(LAYOUT);
    let rgb_pixel_stride = rgb_pixel_stride(RGB);

    // The sample size; pointer steps below are in samples.
    let size = if BITS > 8 { 2 } else { 1 };
    let uv_offset: i32 = 1 << (BITS - 1);
    let src = job.src;
    let sample = |pos: usize| -> i32 {
        if BITS > 8 {
            u16::from_ne_bytes([src[pos], src[pos + 1]]) as i32
        } else {
            src[pos] as i32
        }
    };
    let get = |x: i32| -> i32 {
        if BITS == 10 {
            x >> 6
        } else {
            x
        }
    };
    let y_shift = param.y_shift as i32;
    let y_factor = param.y_factor as i32;

    let y_stride = job.y_stride / size;
    let uv_stride = job.uv_stride / size;

    // The chroma contributions, common to the pixels sharing U and V.
    let uv = |u_ptr: usize, v_ptr: usize| -> (i32, i32, i32) {
        let u_tmp = get(sample(u_ptr)) - uv_offset;
        let v_tmp = get(sample(v_ptr)) - uv_offset;
        (
            v_tmp * param.v_r_factor as i32,
            u_tmp * param.u_g_factor as i32 + v_tmp * param.v_g_factor as i32,
            u_tmp * param.u_b_factor as i32,
        )
    };

    // Upstream's unsigned loop bounds wrap for a zero width or height and
    // read out of bounds; nothing is converted here.
    let width = job.width as usize;
    let height = job.height as usize;

    let mut y = 0usize;
    while y + (uv_y_sample_interval - 1) < height {
        let mut y_ptr1 = job.y + y * y_stride * size;
        let mut u_ptr = job.u + (y / uv_y_sample_interval) * uv_stride * size;
        let mut v_ptr = job.v + (y / uv_y_sample_interval) * uv_stride * size;
        let mut y_ptr2 = job.y + (y + 1) * y_stride * size;

        let mut rgb_ptr1 = job.rgb + y * job.rgb_stride;
        let mut rgb_ptr2 = job.rgb + (y + 1) * job.rgb_stride;

        let mut x = 0usize;
        while x + (uv_x_sample_interval - 1) < width {
            // Compute U and V contributions, common to the four pixels
            let (r_tmp, g_tmp, b_tmp) = uv(u_ptr, v_ptr);

            // Compute the Y contribution for each pixel
            let y_tmp = get(sample(y_ptr1) - y_shift) * y_factor;
            pack_pixel::<RGB>(rgb, rgb_ptr1, y_tmp, r_tmp, g_tmp, b_tmp);
            rgb_ptr1 += rgb_pixel_stride;

            if uv_x_sample_interval > 1 {
                let y_tmp = get(sample(y_ptr1 + y_pixel_stride * size) - y_shift) * y_factor;
                pack_pixel::<RGB>(rgb, rgb_ptr1, y_tmp, r_tmp, g_tmp, b_tmp);
                rgb_ptr1 += rgb_pixel_stride;
            }

            if uv_y_sample_interval > 1 {
                let y_tmp = get(sample(y_ptr2) - y_shift) * y_factor;
                pack_pixel::<RGB>(rgb, rgb_ptr2, y_tmp, r_tmp, g_tmp, b_tmp);
                rgb_ptr2 += rgb_pixel_stride;

                let y_tmp = get(sample(y_ptr2 + y_pixel_stride * size) - y_shift) * y_factor;
                pack_pixel::<RGB>(rgb, rgb_ptr2, y_tmp, r_tmp, g_tmp, b_tmp);
                rgb_ptr2 += rgb_pixel_stride;
            }

            y_ptr1 += uv_x_sample_interval * y_pixel_stride * size;
            if uv_y_sample_interval > 1 {
                y_ptr2 += 2 * y_pixel_stride * size;
            }
            u_ptr += uv_pixel_stride * size;
            v_ptr += uv_pixel_stride * size;
            x += uv_x_sample_interval;
        }

        /* Catch the last pixel, if needed */
        if uv_x_sample_interval == 2 && x + 1 == width {
            // Compute U and V contributions, common to the four pixels
            let (r_tmp, g_tmp, b_tmp) = uv(u_ptr, v_ptr);

            // Compute the Y contribution for each pixel
            let y_tmp = get(sample(y_ptr1) - y_shift) * y_factor;
            pack_pixel::<RGB>(rgb, rgb_ptr1, y_tmp, r_tmp, g_tmp, b_tmp);

            if uv_y_sample_interval > 1 {
                let y_tmp = get(sample(y_ptr2) - y_shift) * y_factor;
                pack_pixel::<RGB>(rgb, rgb_ptr2, y_tmp, r_tmp, g_tmp, b_tmp);
            }
        }
        y += uv_y_sample_interval;
    }

    /* Catch the last line, if needed */
    if uv_y_sample_interval == 2 && y + 1 == height {
        let mut y_ptr1 = job.y + y * y_stride * size;
        let mut u_ptr = job.u + (y / uv_y_sample_interval) * uv_stride * size;
        let mut v_ptr = job.v + (y / uv_y_sample_interval) * uv_stride * size;

        let mut rgb_ptr1 = job.rgb + y * job.rgb_stride;

        let mut x = 0usize;
        while x + (uv_x_sample_interval - 1) < width {
            // Compute U and V contributions, common to the four pixels
            let (r_tmp, g_tmp, b_tmp) = uv(u_ptr, v_ptr);

            // Compute the Y contribution for each pixel
            let y_tmp = get(sample(y_ptr1) - y_shift) * y_factor;
            pack_pixel::<RGB>(rgb, rgb_ptr1, y_tmp, r_tmp, g_tmp, b_tmp);
            rgb_ptr1 += rgb_pixel_stride;

            let y_tmp = get(sample(y_ptr1 + y_pixel_stride * size) - y_shift) * y_factor;
            pack_pixel::<RGB>(rgb, rgb_ptr1, y_tmp, r_tmp, g_tmp, b_tmp);
            rgb_ptr1 += rgb_pixel_stride;

            y_ptr1 += 2 * y_pixel_stride * size;
            u_ptr += 2 * uv_pixel_stride / uv_x_sample_interval * size;
            v_ptr += 2 * uv_pixel_stride / uv_x_sample_interval * size;
            x += uv_x_sample_interval;
        }

        /* Catch the last pixel, if needed */
        if uv_x_sample_interval == 2 && x + 1 == width {
            // Compute U and V contributions, common to the four pixels
            let (r_tmp, g_tmp, b_tmp) = uv(u_ptr, v_ptr);

            // Compute the Y contribution for each pixel
            let y_tmp = get(sample(y_ptr1) - y_shift) * y_factor;
            pack_pixel::<RGB>(rgb, rgb_ptr1, y_tmp, r_tmp, g_tmp, b_tmp);
        }
    }
}

// ---------------------------------------------------------------------------
// yuv2rgb/yuv_rgb_sse_func.h, computed lane by lane

/// The SSE2 kernel template (the unaligned `_sseu` instantiations), for 8-bit
/// 4:2:0, 4:2:2 and NV12 sources. Blocks of 32 pixels (by two lines for the
/// vertically subsampled layouts) go through the 16-bit lane arithmetic;
/// the right column, the last line of 4:2:2 and an odd last line go through
/// the portable kernel.
fn yuv_rgb_sse_kernel<const LAYOUT: u8, const RGB: u8>(job: YuvToRgb<'_>, rgb: &mut [u8]) {
    let param = job.param;
    let (y_pixel_stride, uv_pixel_stride, uv_x_sample_interval, uv_y_sample_interval) =
        layout_strides(LAYOUT);
    let rgb_pixel_stride = rgb_pixel_stride(RGB);
    let width = job.width as usize;
    let height = job.height as usize;
    let src = job.src;

    if width == 0 || height == 0 {
        // Upstream's unsigned arithmetic wraps here and converts out of
        // bounds; there is nothing to convert.
        return;
    }

    /* For NV12 formats (where U/V are interleaved)
     * SSE READ_UV does an invalid read access at the very last pixel.
     * As a workaround. Make sure not to decode the last column using assembly but with STD fallback path.
     * see https://github.com/libsdl-org/SDL/issues/4841
     */
    let fix_read_nv12 = usize::from(LAYOUT == YUV_FORMAT_NV12 && (width & 31) == 0);

    /* Avoid invalid read on last line */
    let fix_read_422 = usize::from(LAYOUT == YUV_FORMAT_422);

    // UV2RGB_16, ADD_Y2RGB_16 and _mm_packus_epi16 for one pixel.
    let factors = (
        param.v_r_factor,
        param.u_g_factor,
        param.v_g_factor,
        param.u_b_factor,
        param.y_shift as i16,
        param.y_factor,
    );
    let convert = |y: u8, u: u8, v: u8| -> (u8, u8, u8) {
        let (v_r, u_g, v_g, u_b, y_shift, y_factor) = factors;
        let u = (u as i16).wrapping_add(-128);
        let v = (v as i16).wrapping_add(-128);
        let r_tmp = v.wrapping_mul(v_r);
        let g_tmp = u.wrapping_mul(u_g).wrapping_add(v.wrapping_mul(v_g));
        let b_tmp = u.wrapping_mul(u_b);
        let y = (y as i16).wrapping_sub(y_shift).wrapping_mul(y_factor);
        let pack = |c: i16| (c.wrapping_add(y) >> PRECISION).clamp(0, 255) as u8;
        (pack(r_tmp), pack(g_tmp), pack(b_tmp))
    };

    let std_kernel = yuv_rgb_std_kernel::<LAYOUT, RGB, 8>;

    let mut ypos = 0usize;
    if width >= 32 {
        while ypos + (uv_y_sample_interval - 1) + fix_read_422 < height {
            let mut y_ptr1 = job.y + ypos * job.y_stride;
            let mut y_ptr2 = job.y + (ypos + 1) * job.y_stride;
            let mut u_ptr = job.u + (ypos / uv_y_sample_interval) * job.uv_stride;
            let mut v_ptr = job.v + (ypos / uv_y_sample_interval) * job.uv_stride;

            let mut rgb_ptr1 = job.rgb + ypos * job.rgb_stride;
            let mut rgb_ptr2 = job.rgb + (ypos + 1) * job.rgb_stride;

            let mut xpos = 0usize;
            while xpos + 31 + fix_read_nv12 < width {
                // YUV2RGB_32, PACK_PIXEL, SAVE_LINE1 and SAVE_LINE2
                for i in 0..32 {
                    let c = i / 2;
                    let u = src[u_ptr + c * uv_pixel_stride];
                    let v = src[v_ptr + c * uv_pixel_stride];
                    let (r, g, b) = convert(src[y_ptr1 + i * y_pixel_stride], u, v);
                    write_rgb8::<RGB>(rgb, rgb_ptr1 + i * rgb_pixel_stride, r, g, b);
                    if uv_y_sample_interval > 1 {
                        let (r, g, b) = convert(src[y_ptr2 + i * y_pixel_stride], u, v);
                        write_rgb8::<RGB>(rgb, rgb_ptr2 + i * rgb_pixel_stride, r, g, b);
                    }
                }

                y_ptr1 += 32 * y_pixel_stride;
                y_ptr2 += 32 * y_pixel_stride;
                u_ptr += 32 * uv_pixel_stride / uv_x_sample_interval;
                v_ptr += 32 * uv_pixel_stride / uv_x_sample_interval;
                rgb_ptr1 += 32 * rgb_pixel_stride;
                rgb_ptr2 += 32 * rgb_pixel_stride;
                xpos += 32;
            }
            ypos += uv_y_sample_interval;
        }

        let line = |ypos: usize| YuvToRgb {
            height: 1,
            y: job.y + ypos * job.y_stride,
            u: job.u + (ypos / uv_y_sample_interval) * job.uv_stride,
            v: job.v + (ypos / uv_y_sample_interval) * job.uv_stride,
            rgb: job.rgb + ypos * job.rgb_stride,
            ..job
        };

        if fix_read_422 != 0 {
            std_kernel(line(ypos), rgb);
            ypos += uv_y_sample_interval;
        }

        /* Catch the last line, if needed */
        if uv_y_sample_interval == 2 && ypos + 1 == height {
            std_kernel(line(ypos), rgb);
        }
    }

    /* Catch the right column, if needed */
    {
        let mut converted = width & !31;
        if fix_read_nv12 != 0 {
            converted -= 32;
        }
        if converted != width {
            std_kernel(
                YuvToRgb {
                    width: (width - converted) as u32,
                    y: job.y + converted * y_pixel_stride,
                    u: job.u + converted * uv_pixel_stride / uv_x_sample_interval,
                    v: job.v + converted * uv_pixel_stride / uv_x_sample_interval,
                    rgb: job.rgb + converted * rgb_pixel_stride,
                    ..job
                },
                rgb,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// SDL_yuv.c

fn get_yuv_conversion_type(colorspace: Colorspace) -> Result<YCbCrType> {
    if colorspace.is_matrix_bt601() {
        if colorspace.is_limited_range() {
            return Ok(YCbCrType::Bt601Limited);
        } else {
            return Ok(YCbCrType::Bt601Full);
        }
    }

    if colorspace.is_matrix_bt709() {
        if colorspace.is_limited_range() {
            return Ok(YCbCrType::Bt709Limited);
        } else {
            return Ok(YCbCrType::Bt709Full);
        }
    }

    if colorspace.is_matrix_bt2020_ncl() && colorspace.is_full_range() {
        return Ok(YCbCrType::Bt2020NclFull);
    }

    Err(Error::new("Unsupported YUV colorspace"))
}

/// The planes of a YUV image: byte offsets of the first Y, U and V samples
/// and the strides, plus the byte size the planes span.
#[derive(Clone, Copy, Debug)]
struct YuvPlanes {
    y: usize,
    u: usize,
    v: usize,
    y_stride: usize,
    uv_stride: usize,
    extent: usize,
}

/// Translation of `GetYUVPlanes()`.
fn get_yuv_planes(
    width: usize,
    height: usize,
    format: PixelFormat,
    yuv_pitch: usize,
) -> Result<YuvPlanes> {
    let mut planes = [0usize; 3];
    let mut pitches = [0usize; 3];
    let extent;
    let bpp = format.bytes_per_pixel() as usize;

    match format {
        PixelFormat::YV12 | PixelFormat::IYUV | PixelFormat::I0FL => {
            pitches[0] = yuv_pitch;
            pitches[1] = ((pitches[0] / bpp).div_ceil(2)) * bpp;
            pitches[2] = pitches[1];
            planes[1] = planes[0] + pitches[0] * height;
            planes[2] = planes[1] + pitches[1] * height.div_ceil(2);
            extent = planes[2] + pitches[2] * height.div_ceil(2);
        }
        PixelFormat::YUY2 | PixelFormat::UYVY | PixelFormat::YVYU => {
            pitches[0] = yuv_pitch;
            extent = pitches[0] * height;
        }
        PixelFormat::NV12 | PixelFormat::NV21 => {
            pitches[0] = yuv_pitch;
            pitches[1] = 2 * pitches[0].div_ceil(2);
            planes[1] = planes[0] + pitches[0] * height;
            extent = planes[1] + pitches[1] * height.div_ceil(2);
        }
        PixelFormat::P010 => {
            pitches[0] = yuv_pitch;
            let uv_width = width.div_ceil(2) * 2;
            pitches[1] = pitches[0].max(uv_width * 2);
            planes[1] = planes[0] + pitches[0] * height;
            extent = planes[1] + pitches[1] * height.div_ceil(2);
        }
        PixelFormat::I444 | PixelFormat::I4FL => {
            pitches[0] = yuv_pitch;
            pitches[1] = pitches[0];
            pitches[2] = pitches[1];
            planes[1] = planes[0] + pitches[0] * height;
            planes[2] = planes[1] + pitches[1] * height;
            extent = planes[2] + pitches[2] * height;
        }
        _ => {
            return Err(Error::new(format!(
                "GetYUVPlanes(): Unsupported YUV format: {}",
                format.name()
            )));
        }
    }

    let (y, u, v, y_stride, uv_stride) = match format {
        PixelFormat::YV12 => (planes[0], planes[2], planes[1], pitches[0], pitches[1]),
        PixelFormat::IYUV | PixelFormat::I444 | PixelFormat::I0FL | PixelFormat::I4FL => {
            (planes[0], planes[1], planes[2], pitches[0], pitches[1])
        }
        PixelFormat::YUY2 => (
            planes[0],
            planes[0] + 1,
            planes[0] + 3,
            pitches[0],
            pitches[0],
        ),
        PixelFormat::UYVY => (
            planes[0] + 1,
            planes[0],
            planes[0] + 2,
            pitches[0],
            pitches[0],
        ),
        PixelFormat::YVYU => (
            planes[0],
            planes[0] + 3,
            planes[0] + 1,
            pitches[0],
            pitches[0],
        ),
        PixelFormat::NV12 => (planes[0], planes[1], planes[1] + 1, pitches[0], pitches[1]),
        PixelFormat::NV21 => (planes[0], planes[1] + 1, planes[1], pitches[0], pitches[1]),
        PixelFormat::P010 => (planes[0], planes[1], planes[1] + 2, pitches[0], pitches[1]),
        _ => {
            // Should have caught this above
            return Err(Error::new(format!(
                "GetYUVPlanes[2]: Unsupported YUV format: {}",
                format.name()
            )));
        }
    };
    Ok(YuvPlanes {
        y,
        u,
        v,
        y_stride,
        uv_stride,
        extent,
    })
}

/// The 8-bit RGB layout the kernels write for `format`.
fn rgb8_kernel_format(format: PixelFormat) -> Option<u8> {
    Some(match format {
        PixelFormat::RGB565 => RGB_FORMAT_RGB565,
        PixelFormat::RGB24 => RGB_FORMAT_RGB24,
        PixelFormat::RGBX8888 | PixelFormat::RGBA8888 => RGB_FORMAT_RGBA,
        PixelFormat::BGRX8888 | PixelFormat::BGRA8888 => RGB_FORMAT_BGRA,
        PixelFormat::XRGB8888 | PixelFormat::ARGB8888 => RGB_FORMAT_ARGB,
        PixelFormat::XBGR8888 | PixelFormat::ABGR8888 => RGB_FORMAT_ABGR,
        _ => return None,
    })
}

/// Instantiate `$kernel::<$layout, RGB $(, $bits)?>` for an 8-bit RGB layout.
macro_rules! by_rgb8 {
    ($rgb:expr, $kernel:ident, $layout:expr $(, $bits:expr)?) => {
        match $rgb {
            RGB_FORMAT_RGB565 => $kernel::<{ $layout }, RGB_FORMAT_RGB565 $(, $bits)?> as YuvToRgbKernel,
            RGB_FORMAT_RGB24 => $kernel::<{ $layout }, RGB_FORMAT_RGB24 $(, $bits)?>,
            RGB_FORMAT_RGBA => $kernel::<{ $layout }, RGB_FORMAT_RGBA $(, $bits)?>,
            RGB_FORMAT_BGRA => $kernel::<{ $layout }, RGB_FORMAT_BGRA $(, $bits)?>,
            RGB_FORMAT_ARGB => $kernel::<{ $layout }, RGB_FORMAT_ARGB $(, $bits)?>,
            _ => $kernel::<{ $layout }, RGB_FORMAT_ABGR $(, $bits)?>,
        }
    };
}

/// The SSE2 kernel for a conversion, if there is one (`yuv_rgb_sse()`).
fn yuv_rgb_sse(src_format: PixelFormat, dst_format: PixelFormat) -> Option<YuvToRgbKernel> {
    if !simd_support().sse2 {
        return None;
    }
    let rgb = rgb8_kernel_format(dst_format)?;

    if src_format == PixelFormat::YV12 || src_format == PixelFormat::IYUV {
        return Some(by_rgb8!(rgb, yuv_rgb_sse_kernel, YUV_FORMAT_420));
    }

    if src_format == PixelFormat::YUY2
        || src_format == PixelFormat::UYVY
        || src_format == PixelFormat::YVYU
    {
        return Some(by_rgb8!(rgb, yuv_rgb_sse_kernel, YUV_FORMAT_422));
    }

    if src_format == PixelFormat::NV12 || src_format == PixelFormat::NV21 {
        return Some(by_rgb8!(rgb, yuv_rgb_sse_kernel, YUV_FORMAT_NV12));
    }
    None
}

/// The portable kernel for a conversion, if there is one (`yuv_rgb_std()`).
fn yuv_rgb_std(src_format: PixelFormat, dst_format: PixelFormat) -> Option<YuvToRgbKernel> {
    let rgb = rgb8_kernel_format(dst_format);

    if src_format == PixelFormat::YV12 || src_format == PixelFormat::IYUV {
        if let Some(rgb) = rgb {
            return Some(by_rgb8!(rgb, yuv_rgb_std_kernel, YUV_FORMAT_420, 8));
        }
    }

    if src_format == PixelFormat::I444 {
        if let Some(rgb) = rgb.filter(|&rgb| rgb != RGB_FORMAT_RGB565 && rgb != RGB_FORMAT_RGB24) {
            return Some(by_rgb8!(rgb, yuv_rgb_std_kernel, YUV_FORMAT_444, 8));
        }
    }

    if src_format == PixelFormat::YUY2
        || src_format == PixelFormat::UYVY
        || src_format == PixelFormat::YVYU
    {
        if let Some(rgb) = rgb {
            return Some(by_rgb8!(rgb, yuv_rgb_std_kernel, YUV_FORMAT_422, 8));
        }
    }

    if src_format == PixelFormat::NV12 || src_format == PixelFormat::NV21 {
        if let Some(rgb) = rgb {
            return Some(by_rgb8!(rgb, yuv_rgb_std_kernel, YUV_FORMAT_NV12, 8));
        }
    }

    if src_format == PixelFormat::P010 && dst_format == PixelFormat::XBGR2101010 {
        return Some(yuv_rgb_std_kernel::<YUV_FORMAT_NV12, RGB_FORMAT_XBGR2101010, 10>);
    }

    if src_format == PixelFormat::I0FL && dst_format == PixelFormat::RGB48 {
        return Some(yuv_rgb_std_kernel::<YUV_FORMAT_420, RGB_FORMAT_RGB48, 16>);
    }

    if src_format == PixelFormat::I4FL && dst_format == PixelFormat::RGB48 {
        return Some(yuv_rgb_std_kernel::<YUV_FORMAT_444, RGB_FORMAT_RGB48, 16>);
    }
    None
}

/// Check that `buf` holds `needed` bytes. Upstream trusts its callers.
fn check_len(buf: usize, needed: usize, what: &'static str) -> Result<()> {
    if buf < needed {
        return Err(Error::new(format!(
            "The {what} buffer is too small, expected at least {needed} bytes"
        )));
    }
    Ok(())
}

/// The bytes spanned by `height` rows of `row` bytes, `pitch` apart.
fn rows_extent(height: usize, pitch: usize, row: usize) -> usize {
    if height == 0 {
        0
    } else {
        (height - 1) * pitch + row
    }
}

/// The bytes in a row of `width` pixels of `format`.
fn row_bytes(format: PixelFormat, width: usize) -> usize {
    if is_packed4_format(format) {
        4 * width.div_ceil(2)
    } else if format.is_fourcc() {
        width * format.bytes_per_pixel() as usize
    } else {
        (width * format.bits_per_pixel() as usize).div_ceil(8)
    }
}

/// Validate the dimensions and pitches of a conversion. Upstream trusts
/// them; a negative size or a pitch shorter than a row is an error here.
fn check_dims(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src_pitch: i32,
    dst_format: PixelFormat,
    dst_pitch: i32,
) -> Result<(usize, usize, usize, usize)> {
    if width < 0 {
        return Err(Error::invalid_param("width"));
    }
    if height < 0 {
        return Err(Error::invalid_param("height"));
    }
    let w = width as usize;
    for (format, pitch, what) in [
        (src_format, src_pitch, "Source"),
        (dst_format, dst_pitch, "Destination"),
    ] {
        let row = row_bytes(format, w);
        if pitch < 0 || (pitch as usize) < row {
            return Err(Error::new(format!(
                "{what} pitch is too small, expected at least {row}"
            )));
        }
    }
    Ok((w, height as usize, src_pitch as usize, dst_pitch as usize))
}

/// Translation of `SDL_ConvertPixels_YUV_to_RGB()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_pixels_yuv_to_rgb(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src_colorspace: Colorspace,
    src_properties: Option<&Properties>,
    src: &[u8],
    src_pitch: i32,
    dst_format: PixelFormat,
    dst_colorspace: Colorspace,
    dst_properties: Option<&Properties>,
    dst: &mut [u8],
    dst_pitch: i32,
) -> Result<()> {
    let (w, h, src_pitch_u, dst_pitch_u) =
        check_dims(width, height, src_format, src_pitch, dst_format, dst_pitch)?;

    let planes = get_yuv_planes(w, h, src_format, src_pitch_u)?;
    let yuv_type = get_yuv_conversion_type(src_colorspace)?;

    let kernel =
        yuv_rgb_sse(src_format, dst_format).or_else(|| yuv_rgb_std(src_format, dst_format));
    if let Some(kernel) = kernel {
        check_len(src.len(), planes.extent, "source")?;
        let row = w * dst_format.bytes_per_pixel() as usize;
        check_len(dst.len(), rows_extent(h, dst_pitch_u, row), "destination")?;
        kernel(
            YuvToRgb {
                width: w as u32,
                height: h as u32,
                src,
                y: planes.y,
                u: planes.u,
                v: planes.v,
                y_stride: planes.y_stride,
                uv_stride: planes.uv_stride,
                rgb: 0,
                rgb_stride: dst_pitch_u,
                param: &YUV2RGB[yuv_type as usize],
            },
            dst,
        );
        return Ok(());
    }

    // No fast path for the RGB format, instead convert using an intermediate buffer
    let via = |tmp_format: PixelFormat,
               tmp_pitch: usize,
               tmp_colorspace: Colorspace,
               tmp_properties: Option<&Properties>,
               dst: &mut [u8]|
     -> Result<()> {
        let mut tmp = vec![0u8; tmp_pitch * h];

        // convert src/src_format to tmp/tmp_format
        convert_pixels_yuv_to_rgb(
            width,
            height,
            src_format,
            src_colorspace,
            src_properties,
            src,
            src_pitch,
            tmp_format,
            tmp_colorspace,
            tmp_properties,
            &mut tmp,
            tmp_pitch as i32,
        )?;

        // convert tmp/tmp_format to dst/RGB
        convert_pixels_and_colorspace(
            width,
            height,
            tmp_format,
            tmp_colorspace,
            tmp_properties,
            &tmp,
            tmp_pitch as i32,
            dst_format,
            dst_colorspace,
            dst_properties,
            dst,
            dst_pitch,
        )
    };

    if src_format == PixelFormat::P010 && dst_format != PixelFormat::XBGR2101010 {
        // convert src/src_format to tmp/XBGR2101010
        return via(
            PixelFormat::XBGR2101010,
            w * 4,
            src_colorspace,
            src_properties,
            dst,
        );
    }

    if (src_format == PixelFormat::I0FL || src_format == PixelFormat::I4FL)
        && dst_format != PixelFormat::RGB48
    {
        // convert src/src_format to tmp/RGB48
        return via(
            PixelFormat::RGB48,
            w * 3 * 2,
            src_colorspace,
            src_properties,
            dst,
        );
    }

    if dst_format != PixelFormat::ARGB8888 {
        // convert src/src_format to tmp/ARGB8888
        return via(PixelFormat::ARGB8888, w * 4, Colorspace::SRGB, None, dst);
    }

    Err(Error::new("Unsupported YUV conversion"))
}

/// `struct RGB2YUVFactors`.
struct Rgb2YuvFactors {
    y_offset: i32,
    y: [f32; 3], // Rfactor, Gfactor, Bfactor
    u: [f32; 3], // Rfactor, Gfactor, Bfactor
    v: [f32; 3], // Rfactor, Gfactor, Bfactor
}

static RGB2YUV_FACTOR_TABLES: [Rgb2YuvFactors; 5] = [
    // ITU-T T.871 (JPEG)
    Rgb2YuvFactors {
        y_offset: 0,
        y: [0.2990, 0.5870, 0.1140],
        u: [-0.1687, -0.3313, 0.5000],
        v: [0.5000, -0.4187, -0.0813],
    },
    // ITU-R BT.601-7
    Rgb2YuvFactors {
        y_offset: 16,
        y: [0.2568, 0.5041, 0.0979],
        u: [-0.1482, -0.2910, 0.4392],
        v: [0.4392, -0.3678, -0.0714],
    },
    // ITU-R BT.709-6 full range
    Rgb2YuvFactors {
        y_offset: 0,
        y: [0.2126, 0.7152, 0.0722],
        u: [-0.1141, -0.3839, 0.498],
        v: [0.498, -0.4524, -0.0457],
    },
    // ITU-R BT.709-6
    Rgb2YuvFactors {
        y_offset: 16,
        y: [0.1826, 0.6142, 0.0620],
        u: [-0.1006, -0.3386, 0.4392],
        v: [0.4392, -0.3989, -0.0403],
    },
    // ITU-R BT.2020 10-bit full range
    Rgb2YuvFactors {
        y_offset: 0,
        y: [0.2627, 0.6780, 0.0593],
        u: [-0.1395, -0.3600, 0.4995],
        v: [0.4995, -0.4593, -0.0402],
    },
];

impl Rgb2YuvFactors {
    /// The factors applied to a color, rounded and truncated like upstream's
    /// `(int)(f[0] * r + f[1] * g + f[2] * b + 0.5f)`.
    #[inline(always)]
    fn apply(f: &[f32; 3], r: u32, g: u32, b: u32) -> i32 {
        (f[0] * r as f32 + f[1] * g as f32 + f[2] * b as f32 + 0.5) as i32
    }

    fn make_y(&self, r: u32, g: u32, b: u32) -> u8 {
        (Self::apply(&self.y, r, g, b) + self.y_offset).clamp(0, 255) as u8
    }

    fn make_u(&self, r: u32, g: u32, b: u32) -> u8 {
        (Self::apply(&self.u, r, g, b) + 128).clamp(0, 255) as u8
    }

    fn make_v(&self, r: u32, g: u32, b: u32) -> u8 {
        (Self::apply(&self.v, r, g, b) + 128).clamp(0, 255) as u8
    }

    // The 10-bit versions for P010 do not clamp.

    fn make_y10(&self, r: u32, g: u32, b: u32) -> u16 {
        ((Self::apply(&self.y, r, g, b) + self.y_offset) << 6) as u16
    }

    fn make_u10(&self, r: u32, g: u32, b: u32) -> u16 {
        ((Self::apply(&self.u, r, g, b) + 512) << 6) as u16
    }

    fn make_v10(&self, r: u32, g: u32, b: u32) -> u16 {
        ((Self::apply(&self.v, r, g, b) + 512) << 6) as u16
    }
}

#[inline(always)]
fn read_u32(buf: &[u8], pos: usize) -> u32 {
    u32::from_ne_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]])
}

/// The color of the `i`th pixel pair of a row of 32-bit pixels, averaged
/// over `rows` rows (1 or 2) and `cols` columns (1 or 2), with the channels
/// at `shifts` (red, green, blue) and `mask` wide. These are upstream's
/// `READ_2x2_PIXELS`, `READ_2x1_PIXELS`, `READ_1x2_PIXELS` and
/// `READ_1x1_PIXEL`.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn read_block(
    src: &[u8],
    curr_row: usize,
    next_row: usize,
    i: usize,
    rows: usize,
    cols: usize,
    shifts: [u32; 3],
    mask: u32,
) -> (u32, u32, u32) {
    let mut sum = [0u32; 3];
    for row in [curr_row, next_row].into_iter().take(rows) {
        for col in 0..cols {
            let p = read_u32(src, row + (2 * i + col) * 4);
            for (s, shift) in sum.iter_mut().zip(shifts) {
                *s += p & (mask << shift);
            }
        }
    }
    let n = (rows * cols).trailing_zeros();
    (
        sum[0] >> (shifts[0] + n),
        sum[1] >> (shifts[1] + n),
        sum[2] >> (shifts[2] + n),
    )
}

const XRGB8888_SHIFTS: [u32; 3] = [16, 8, 0];
const XBGR2101010_SHIFTS: [u32; 3] = [0, 10, 20];

/// Translation of `SDL_ConvertPixels_XRGB8888_to_YUV()`.
#[allow(clippy::too_many_arguments)]
fn convert_pixels_xrgb8888_to_yuv(
    width: usize,
    height: usize,
    src: &[u8],
    src_pitch: usize,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: usize,
    yuv_type: YCbCrType,
) -> Result<()> {
    let src_pitch_x_2 = src_pitch * 2;
    let height_half = height / 2;
    let height_remainder = (height & 0x1) != 0;
    let width_half = width / 2;
    let width_remainder = (width & 0x1) != 0;
    let cvt = &RGB2YUV_FACTOR_TABLES[yuv_type as usize];

    check_len(
        src.len(),
        rows_extent(height, src_pitch, width * 4),
        "source",
    )?;

    let read_one = |row: usize, i: usize| {
        let p = read_u32(src, row + i * 4);
        (
            (p & 0x00ff0000) >> 16,
            (p & 0x0000ff00) >> 8,
            p & 0x000000ff,
        )
    };
    let read = |curr_row: usize, next_row: usize, i: usize, rows: usize, cols: usize| {
        read_block(
            src,
            curr_row,
            next_row,
            i,
            rows,
            cols,
            XRGB8888_SHIFTS,
            0xff,
        )
    };

    match dst_format {
        PixelFormat::YV12
        | PixelFormat::IYUV
        | PixelFormat::I444
        | PixelFormat::NV12
        | PixelFormat::NV21 => {
            let planes = get_yuv_planes(width, height, dst_format, dst_pitch)?;
            check_len(dst.len(), planes.extent, "destination")?;
            let (y_stride, uv_stride) = (planes.y_stride, planes.uv_stride);
            let plane_interleaved_uv = planes.y + height * y_stride;

            // Write Y plane
            for j in 0..height {
                for i in 0..width {
                    let (r, g, b) = read_one(j * src_pitch, i);
                    dst[planes.y + j * y_stride + i] = cvt.make_y(r, g, b);
                }
            }

            if dst_format == PixelFormat::YV12 || dst_format == PixelFormat::IYUV {
                // Write UV planes, not interleaved
                let mut put = |j: usize, i: usize, (r, g, b): (u32, u32, u32)| {
                    dst[planes.u + j * uv_stride + i] = cvt.make_u(r, g, b);
                    dst[planes.v + j * uv_stride + i] = cvt.make_v(r, g, b);
                };
                uv_2x2_blocks(
                    width_half,
                    width_remainder,
                    height_half,
                    height_remainder,
                    src_pitch,
                    src_pitch_x_2,
                    &read,
                    &mut put,
                );
            } else if dst_format == PixelFormat::I444 {
                // Write UV planes, not interleaved
                for j in 0..height {
                    for i in 0..width {
                        let (r, g, b) = read_one(j * src_pitch, i);
                        dst[planes.u + j * uv_stride + i] = cvt.make_u(r, g, b);
                        dst[planes.v + j * uv_stride + i] = cvt.make_v(r, g, b);
                    }
                }
            } else {
                let nv21 = dst_format == PixelFormat::NV21;
                let mut put = |j: usize, i: usize, (r, g, b): (u32, u32, u32)| {
                    let pos = plane_interleaved_uv + j * uv_stride + 2 * i;
                    let (u, v) = (cvt.make_u(r, g, b), cvt.make_v(r, g, b));
                    if nv21 {
                        dst[pos] = v;
                        dst[pos + 1] = u;
                    } else {
                        dst[pos] = u;
                        dst[pos + 1] = v;
                    }
                };
                uv_2x2_blocks(
                    width_half,
                    width_remainder,
                    height_half,
                    height_remainder,
                    src_pitch,
                    src_pitch_x_2,
                    &read,
                    &mut put,
                );
            }
        }
        PixelFormat::YUY2 | PixelFormat::UYVY | PixelFormat::YVYU => {
            let row_size = 4 * width.div_ceil(2);

            if dst_pitch < row_size {
                return Err(Error::new(format!(
                    "Destination pitch is too small, expected at least {row_size}"
                )));
            }
            check_len(
                dst.len(),
                rows_extent(height, dst_pitch, row_size),
                "destination",
            )?;

            // The byte order of a macropixel: positions of Y0, U, Y1 and V.
            let (y0, u, y1, v) = match dst_format {
                PixelFormat::YUY2 => (0, 1, 2, 3), // Y U Y1 V
                PixelFormat::UYVY => (1, 0, 3, 2), // U Y V Y1
                _ => (0, 3, 2, 1),                 // Y V Y1 U
            };

            // Write YUV plane, packed
            for j in 0..height {
                let curr_row = j * src_pitch;
                let mut plane = j * dst_pitch;
                for i in 0..width_half {
                    // READ_TWO_RGB_PIXELS
                    let (r, g, b) = read_one(curr_row, 2 * i);
                    let (r1, g1, b1) = read_one(curr_row, 2 * i + 1);
                    let (rr, gg, bb) = ((r + r1) / 2, (g + g1) / 2, (b + b1) / 2);
                    dst[plane + y0] = cvt.make_y(r, g, b);
                    dst[plane + u] = cvt.make_u(rr, gg, bb);
                    dst[plane + y1] = cvt.make_y(r1, g1, b1);
                    dst[plane + v] = cvt.make_v(rr, gg, bb);
                    plane += 4;
                }
                if width_remainder {
                    // READ_ONE_RGB_PIXEL
                    let (r, g, b) = read_one(curr_row, 2 * width_half);
                    dst[plane + y0] = cvt.make_y(r, g, b);
                    dst[plane + u] = cvt.make_u(r, g, b);
                    dst[plane + y1] = cvt.make_y(r, g, b);
                    dst[plane + v] = cvt.make_v(r, g, b);
                }
            }
        }
        _ => {
            return Err(Error::new(format!(
                "Unsupported YUV destination format: {}",
                dst_format.name()
            )));
        }
    }
    Ok(())
}

/// Walk the chroma blocks of a 2x2 subsampled image: 2x2 pixel blocks,
/// then the odd right column (2x1) and the odd bottom row (1x2 and 1x1).
/// `read(curr_row, next_row, i, rows, cols)` reads the averaged color of
/// block `i`; `put(j, i, color)` writes the chroma of row `j`, block `i`.
#[allow(clippy::too_many_arguments)]
fn uv_2x2_blocks(
    width_half: usize,
    width_remainder: bool,
    height_half: usize,
    height_remainder: bool,
    src_pitch: usize,
    src_pitch_x_2: usize,
    read: &dyn Fn(usize, usize, usize, usize, usize) -> (u32, u32, u32),
    put: &mut dyn FnMut(usize, usize, (u32, u32, u32)),
) {
    let mut curr_row = 0;
    let mut next_row = src_pitch;
    for j in 0..height_half {
        for i in 0..width_half {
            put(j, i, read(curr_row, next_row, i, 2, 2));
        }
        if width_remainder {
            put(j, width_half, read(curr_row, next_row, width_half, 2, 1));
        }
        curr_row += src_pitch_x_2;
        next_row += src_pitch_x_2;
    }
    if height_remainder {
        for i in 0..width_half {
            put(height_half, i, read(curr_row, next_row, i, 1, 2));
        }
        if width_remainder {
            put(
                height_half,
                width_half,
                read(curr_row, next_row, width_half, 1, 1),
            );
        }
    }
}

/// Translation of `SDL_ConvertPixels_XBGR2101010_to_P010()`.
#[allow(clippy::too_many_arguments)]
fn convert_pixels_xbgr2101010_to_p010(
    width: usize,
    height: usize,
    src: &[u8],
    src_pitch: usize,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: usize,
    yuv_type: YCbCrType,
) -> Result<()> {
    let src_pitch_x_2 = src_pitch * 2;
    let height_half = height / 2;
    let height_remainder = (height & 0x1) != 0;
    let width_half = width / 2;
    let width_remainder = (width & 0x1) != 0;
    let cvt = &RGB2YUV_FACTOR_TABLES[yuv_type as usize];

    check_len(
        src.len(),
        rows_extent(height, src_pitch, width * 4),
        "source",
    )?;
    let planes = get_yuv_planes(width, height, dst_format, dst_pitch)?;
    check_len(dst.len(), planes.extent, "destination")?;

    // The strides in samples
    let y_stride = planes.y_stride / 2;
    let uv_stride = planes.uv_stride / 2;
    let plane_interleaved_uv = planes.y + 2 * height * y_stride;
    let mut put16 = |pos: usize, v: u16| dst[pos..pos + 2].copy_from_slice(&v.to_ne_bytes());

    // Write Y plane
    for j in 0..height {
        for i in 0..width {
            let p1 = read_u32(src, j * src_pitch + i * 4);
            let r = p1 & 0x03ff;
            let g = (p1 >> 10) & 0x03ff;
            let b = (p1 >> 20) & 0x03ff;
            put16(planes.y + 2 * (j * y_stride + i), cvt.make_y10(r, g, b));
        }
    }

    let read = |curr_row: usize, next_row: usize, i: usize, rows: usize, cols: usize| {
        read_block(
            src,
            curr_row,
            next_row,
            i,
            rows,
            cols,
            XBGR2101010_SHIFTS,
            0x3ff,
        )
    };
    let mut put = |j: usize, i: usize, (r, g, b): (u32, u32, u32)| {
        let pos = plane_interleaved_uv + 2 * (j * uv_stride + 2 * i);
        put16(pos, cvt.make_u10(r, g, b));
        put16(pos + 2, cvt.make_v10(r, g, b));
    };
    uv_2x2_blocks(
        width_half,
        width_remainder,
        height_half,
        height_remainder,
        src_pitch,
        src_pitch_x_2,
        &read,
        &mut put,
    );
    Ok(())
}

/// Translation of `SDL_ConvertPixels_RGB_to_YUV()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_pixels_rgb_to_yuv(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src_colorspace: Colorspace,
    src_properties: Option<&Properties>,
    src: &[u8],
    src_pitch: i32,
    dst_format: PixelFormat,
    dst_colorspace: Colorspace,
    dst_properties: Option<&Properties>,
    dst: &mut [u8],
    dst_pitch: i32,
) -> Result<()> {
    let yuv_type = get_yuv_conversion_type(dst_colorspace)?;
    let (w, h, src_pitch_u, dst_pitch_u) =
        check_dims(width, height, src_format, src_pitch, dst_format, dst_pitch)?;

    // ARGB8888 to FOURCC
    if src_format == PixelFormat::ARGB8888 || src_format == PixelFormat::XRGB8888 {
        // This comes before the P010 case, so ARGB8888 to P010 is
        // "Unsupported YUV destination format" upstream too.
        return convert_pixels_xrgb8888_to_yuv(
            w,
            h,
            src,
            src_pitch_u,
            dst_format,
            dst,
            dst_pitch_u,
            yuv_type,
        );
    }

    if dst_format == PixelFormat::P010 {
        if src_format == PixelFormat::XBGR2101010 {
            return convert_pixels_xbgr2101010_to_p010(
                w,
                h,
                src,
                src_pitch_u,
                dst_format,
                dst,
                dst_pitch_u,
                yuv_type,
            );
        }

        // We currently only support converting from XBGR2101010 to P010
        let tmp_pitch = w * 4;
        let mut tmp = vec![0u8; tmp_pitch * h];

        // convert src/src_format to tmp/XBGR2101010
        convert_pixels_and_colorspace(
            width,
            height,
            src_format,
            src_colorspace,
            src_properties,
            src,
            src_pitch,
            PixelFormat::XBGR2101010,
            dst_colorspace,
            dst_properties,
            &mut tmp,
            tmp_pitch as i32,
        )?;

        // convert tmp/XBGR2101010 to dst/P010
        return convert_pixels_xbgr2101010_to_p010(
            w,
            h,
            &tmp,
            tmp_pitch,
            dst_format,
            dst,
            dst_pitch_u,
            yuv_type,
        );
    }

    // not ARGB8888 to FOURCC : need an intermediate conversion
    let tmp_pitch = w * 4;
    let mut tmp = vec![0u8; tmp_pitch * h];

    // convert src/src_format to tmp/XRGB8888
    convert_pixels_and_colorspace(
        width,
        height,
        src_format,
        src_colorspace,
        src_properties,
        src,
        src_pitch,
        PixelFormat::XRGB8888,
        Colorspace::SRGB,
        None,
        &mut tmp,
        tmp_pitch as i32,
    )?;

    // convert tmp/XRGB8888 to dst/FOURCC
    convert_pixels_xrgb8888_to_yuv(
        w,
        h,
        &tmp,
        tmp_pitch,
        dst_format,
        dst,
        dst_pitch_u,
        yuv_type,
    )
}

/// Copy `rows` rows of `length` bytes, `src_pitch` and `dst_pitch` apart,
/// advancing the positions past them.
#[allow(clippy::too_many_arguments)]
fn copy_rows(
    src: &[u8],
    src_pos: &mut usize,
    src_pitch: usize,
    dst: &mut [u8],
    dst_pos: &mut usize,
    dst_pitch: usize,
    rows: usize,
    length: usize,
) {
    for _ in 0..rows {
        dst[*dst_pos..*dst_pos + length].copy_from_slice(&src[*src_pos..*src_pos + length]);
        *src_pos += src_pitch;
        *dst_pos += dst_pitch;
    }
}

/// Translation of `SDL_ConvertPixels_YUV_to_YUV_Copy()`.
fn convert_pixels_yuv_to_yuv_copy(
    mut width: usize,
    mut height: usize,
    format: PixelFormat,
    src: &[u8],
    mut src_pitch: usize,
    dst: &mut [u8],
    mut dst_pitch: usize,
) -> Result<()> {
    let (mut s, mut d) = (0usize, 0usize);

    if is_planar_1x1_format(format) {
        // YUV planes
        let length = width * format.bytes_per_pixel() as usize;
        copy_rows(
            src,
            &mut s,
            src_pitch,
            dst,
            &mut d,
            dst_pitch,
            height * 3,
            length,
        );
        return Ok(());
    }

    if is_planar_2x2_format(format) {
        let bpp = format.bytes_per_pixel() as usize;

        // Y plane
        let length = width * bpp;
        copy_rows(
            src, &mut s, src_pitch, dst, &mut d, dst_pitch, height, length,
        );

        if format == PixelFormat::YV12 || format == PixelFormat::IYUV || format == PixelFormat::I0FL
        {
            // U and V planes are a quarter the size of the Y plane, rounded up
            width = width.div_ceil(2) * bpp;
            height = height.div_ceil(2);
            src_pitch = (src_pitch / bpp).div_ceil(2) * bpp;
            dst_pitch = (dst_pitch / bpp).div_ceil(2) * bpp;
            copy_rows(
                src,
                &mut s,
                src_pitch,
                dst,
                &mut d,
                dst_pitch,
                height * 2,
                width,
            );
        } else if format == PixelFormat::NV12
            || format == PixelFormat::NV21
            || format == PixelFormat::P010
        {
            // U/V plane is half the height of the Y plane, rounded up, with packed CrCb
            width = width.div_ceil(2) * 2 * bpp;
            height = height.div_ceil(2);
            src_pitch = (src_pitch / bpp).div_ceil(2) * 2 * bpp;
            dst_pitch = (dst_pitch / bpp).div_ceil(2) * 2 * bpp;
            copy_rows(
                src, &mut s, src_pitch, dst, &mut d, dst_pitch, height, width,
            );
        }
        return Ok(());
    }

    if is_packed4_format(format) {
        // Packed planes
        width = 4 * width.div_ceil(2);
        copy_rows(
            src, &mut s, src_pitch, dst, &mut d, dst_pitch, height, width,
        );
        return Ok(());
    }

    Err(Error::new(format!(
        "SDL_ConvertPixels_YUV_to_YUV_Copy: Unsupported YUV format: {}",
        format.name()
    )))
}

/// Translation of `SDL_ConvertPixels_SwapUVPlanes()`.
fn convert_pixels_swap_uv_planes(
    width: usize,
    height: usize,
    src: &[u8],
    src_pitch: usize,
    dst: &mut [u8],
    dst_pitch: usize,
) {
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);

    // Skip the Y plane
    let src_base = height * src_pitch;
    let dst_base = height * dst_pitch;

    let src_uv_pitch = src_pitch.div_ceil(2);
    let dst_uv_pitch = dst_pitch.div_ceil(2);

    // Copy the first plane
    let mut src_uv = src_base;
    let mut dst_uv = dst_base + uv_height * dst_uv_pitch;
    copy_rows(
        src,
        &mut src_uv,
        src_uv_pitch,
        dst,
        &mut dst_uv,
        dst_uv_pitch,
        uv_height,
        uv_width,
    );

    // Copy the second plane
    let mut dst_uv = dst_base;
    copy_rows(
        src,
        &mut src_uv,
        src_uv_pitch,
        dst,
        &mut dst_uv,
        dst_uv_pitch,
        uv_height,
        uv_width,
    );
}

/// Translation of `SDL_ConvertPixels_PackUVPlanes_to_NV()`.
fn convert_pixels_pack_uv_planes_to_nv(
    width: usize,
    height: usize,
    src: &[u8],
    src_pitch: usize,
    dst: &mut [u8],
    dst_pitch: usize,
    reverse_uv: bool,
) {
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let src_uv_pitch = src_pitch.div_ceil(2);
    let dst_uv_pitch = dst_pitch.div_ceil(2) * 2;

    // Skip the Y plane
    let src_base = height * src_pitch;
    let dst_base = height * dst_pitch;

    let (src1, src2) = if reverse_uv {
        (src_base + uv_height * src_uv_pitch, src_base)
    } else {
        (src_base, src_base + uv_height * src_uv_pitch)
    };

    for y in 0..uv_height {
        let (s1, s2, d) = (
            src1 + y * src_uv_pitch,
            src2 + y * src_uv_pitch,
            dst_base + y * dst_uv_pitch,
        );
        for x in 0..uv_width {
            dst[d + 2 * x] = src[s1 + x];
            dst[d + 2 * x + 1] = src[s2 + x];
        }
    }
}

/// Translation of `SDL_ConvertPixels_SplitNV_to_UVPlanes()`.
fn convert_pixels_split_nv_to_uv_planes(
    width: usize,
    height: usize,
    src: &[u8],
    src_pitch: usize,
    dst: &mut [u8],
    dst_pitch: usize,
    reverse_uv: bool,
) {
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let src_uv_pitch = src_pitch.div_ceil(2) * 2;
    let dst_uv_pitch = dst_pitch.div_ceil(2);

    // Skip the Y plane
    let src_base = height * src_pitch;
    let dst_base = height * dst_pitch;

    let (dst1, dst2) = if reverse_uv {
        (dst_base + uv_height * dst_uv_pitch, dst_base)
    } else {
        (dst_base, dst_base + uv_height * dst_uv_pitch)
    };

    for y in 0..uv_height {
        let (s, d1, d2) = (
            src_base + y * src_uv_pitch,
            dst1 + y * dst_uv_pitch,
            dst2 + y * dst_uv_pitch,
        );
        for x in 0..uv_width {
            dst[d1 + x] = src[s + 2 * x];
            dst[d2 + x] = src[s + 2 * x + 1];
        }
    }
}

/// Translation of `SDL_ConvertPixels_SwapNV()`.
fn convert_pixels_swap_nv(
    width: usize,
    height: usize,
    src: &[u8],
    src_pitch: usize,
    dst: &mut [u8],
    dst_pitch: usize,
) {
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let src_uv_pitch = src_pitch.div_ceil(2) * 2;
    let dst_uv_pitch = dst_pitch.div_ceil(2) * 2;

    // Skip the Y plane
    let src_base = height * src_pitch;
    let dst_base = height * dst_pitch;

    for y in 0..uv_height {
        let (s, d) = (src_base + y * src_uv_pitch, dst_base + y * dst_uv_pitch);
        for x in 0..uv_width {
            let u = src[s + 2 * x];
            let v = src[s + 2 * x + 1];
            dst[d + 2 * x] = v;
            dst[d + 2 * x + 1] = u;
        }
    }
}

/// Translation of `SDL_ConvertPixels_Planar2x2_to_Planar2x2()`.
#[allow(clippy::too_many_arguments)]
fn convert_pixels_planar2x2_to_planar2x2(
    width: usize,
    height: usize,
    src_format: PixelFormat,
    src: &[u8],
    src_pitch: usize,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: usize,
) -> Result<()> {
    use PixelFormat as F;

    let swap_uv_planes = convert_pixels_swap_uv_planes;
    let pack = convert_pixels_pack_uv_planes_to_nv;
    let split = convert_pixels_split_nv_to_uv_planes;
    let swap_nv = convert_pixels_swap_nv;

    // Copy Y plane
    let (mut s, mut d) = (0, 0);
    copy_rows(
        src, &mut s, src_pitch, dst, &mut d, dst_pitch, height, width,
    );

    let (w, h, sp, dp) = (width, height, src_pitch, dst_pitch);
    match (src_format, dst_format) {
        (F::YV12, F::IYUV) | (F::IYUV, F::YV12) => swap_uv_planes(w, h, src, sp, dst, dp),
        (F::YV12, F::NV12) => pack(w, h, src, sp, dst, dp, true),
        (F::YV12, F::NV21) => pack(w, h, src, sp, dst, dp, false),
        (F::IYUV, F::NV12) => pack(w, h, src, sp, dst, dp, false),
        (F::IYUV, F::NV21) => pack(w, h, src, sp, dst, dp, true),
        (F::NV12, F::YV12) => split(w, h, src, sp, dst, dp, true),
        (F::NV12, F::IYUV) => split(w, h, src, sp, dst, dp, false),
        (F::NV21, F::YV12) => split(w, h, src, sp, dst, dp, false),
        (F::NV21, F::IYUV) => split(w, h, src, sp, dst, dp, true),
        (F::NV12, F::NV21) | (F::NV21, F::NV12) => swap_nv(w, h, src, sp, dst, dp),
        _ => {
            return Err(Error::new(format!(
                "SDL_ConvertPixels_Planar2x2_to_Planar2x2: Unsupported YUV conversion: {} -> {}",
                src_format.name(),
                dst_format.name()
            )));
        }
    }
    Ok(())
}

/// Translation of `SDL_ConvertPixels_Packed4_to_Packed4()`: the
/// `SDL_ConvertPixels_YUY2_to_UYVY()` family as byte permutations.
#[allow(clippy::too_many_arguments)]
fn convert_pixels_packed4_to_packed4(
    width: usize,
    height: usize,
    src_format: PixelFormat,
    src: &[u8],
    src_pitch: usize,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: usize,
) -> Result<()> {
    use PixelFormat as F;

    // dst[k] = src[perm[k]] for each macropixel
    let perm: [usize; 4] = match (src_format, dst_format) {
        (F::YUY2, F::UYVY) => [1, 0, 3, 2],
        (F::YUY2, F::YVYU) => [0, 3, 2, 1],
        (F::UYVY, F::YUY2) => [1, 0, 3, 2],
        (F::UYVY, F::YVYU) => [1, 2, 3, 0],
        (F::YVYU, F::YUY2) => [0, 3, 2, 1],
        (F::YVYU, F::UYVY) => [3, 0, 1, 2],
        _ => {
            return Err(Error::new(format!(
                "SDL_ConvertPixels_Packed4_to_Packed4: Unsupported YUV conversion: {} -> {}",
                src_format.name(),
                dst_format.name()
            )));
        }
    };

    let yuv_width = width.div_ceil(2);
    for y in 0..height {
        let (s, d) = (y * src_pitch, y * dst_pitch);
        for x in 0..yuv_width {
            let px = &src[s + 4 * x..s + 4 * x + 4];
            let out = &mut dst[d + 4 * x..d + 4 * x + 4];
            for k in 0..4 {
                out[k] = px[perm[k]];
            }
        }
    }
    Ok(())
}

/// Translation of `SDL_ConvertPixels_Planar2x2_to_Packed4()`.
#[allow(clippy::too_many_arguments)]
fn convert_pixels_planar2x2_to_packed4(
    width: usize,
    height: usize,
    src_format: PixelFormat,
    src: &[u8],
    src_pitch: usize,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: usize,
) -> Result<()> {
    let sp = get_yuv_planes(width, height, src_format, src_pitch)?;
    check_len(src.len(), sp.extent, "source")?;
    let (mut src_y1, mut src_u, mut src_v) = (sp.y, sp.u, sp.v);
    let (src_y_pitch, src_uv_pitch) = (sp.y_stride, sp.uv_stride);
    let mut src_y2 = src_y1 + src_y_pitch;
    // The pitch "lefts" can be negative for a short pitch; the position
    // arithmetic wraps like upstream's pointers.
    let src_y_pitch_left = src_y_pitch.wrapping_sub(width);
    let (src_uv_pixel_stride, src_uv_pitch_left) =
        if src_format == PixelFormat::NV12 || src_format == PixelFormat::NV21 {
            (2, src_uv_pitch.wrapping_sub(2 * width.div_ceil(2)))
        } else {
            (1, src_uv_pitch.wrapping_sub(width.div_ceil(2)))
        };

    let dp = get_yuv_planes(width, height, dst_format, dst_pitch)?;
    check_len(dst.len(), dp.extent, "destination")?;
    let (mut dst_y1, mut dst_u1, mut dst_v1) = (dp.y, dp.u, dp.v);
    let (dst_y_pitch, dst_uv_pitch) = (dp.y_stride, dp.uv_stride);
    let mut dst_y2 = dst_y1 + dst_y_pitch;
    let mut dst_u2 = dst_u1 + dst_uv_pitch;
    let mut dst_v2 = dst_v1 + dst_uv_pitch;
    let dst_pitch_left = dst_y_pitch.wrapping_sub(4 * width.div_ceil(2));

    let (width, height) = (width as isize, height as isize);

    // Copy 2x2 blocks of pixels at a time
    let mut y = 0isize;
    while y < height - 1 {
        let mut x = 0isize;
        while x < width - 1 {
            // Row 1
            dst[dst_y1] = src[src_y1];
            src_y1 += 1;
            dst_y1 += 2;
            dst[dst_y1] = src[src_y1];
            src_y1 += 1;
            dst_y1 += 2;
            dst[dst_u1] = src[src_u];
            dst[dst_v1] = src[src_v];

            // Row 2
            dst[dst_y2] = src[src_y2];
            src_y2 += 1;
            dst_y2 += 2;
            dst[dst_y2] = src[src_y2];
            src_y2 += 1;
            dst_y2 += 2;
            dst[dst_u2] = src[src_u];
            dst[dst_v2] = src[src_v];

            src_u += src_uv_pixel_stride;
            src_v += src_uv_pixel_stride;
            dst_u1 += 4;
            dst_u2 += 4;
            dst_v1 += 4;
            dst_v2 += 4;
            x += 2;
        }

        // Last column
        if x == width - 1 {
            // Row 1
            dst[dst_y1] = src[src_y1];
            dst_y1 += 2;
            dst[dst_y1] = src[src_y1];
            src_y1 += 1;
            dst_y1 += 2;
            dst[dst_u1] = src[src_u];
            dst[dst_v1] = src[src_v];

            // Row 2
            dst[dst_y2] = src[src_y2];
            dst_y2 += 2;
            dst[dst_y2] = src[src_y2];
            src_y2 += 1;
            dst_y2 += 2;
            dst[dst_u2] = src[src_u];
            dst[dst_v2] = src[src_v];

            src_u += src_uv_pixel_stride;
            src_v += src_uv_pixel_stride;
            dst_u1 += 4;
            dst_u2 += 4;
            dst_v1 += 4;
            dst_v2 += 4;
        }

        src_y1 = src_y1
            .wrapping_add(src_y_pitch_left)
            .wrapping_add(src_y_pitch);
        src_y2 = src_y2
            .wrapping_add(src_y_pitch_left)
            .wrapping_add(src_y_pitch);
        src_u = src_u.wrapping_add(src_uv_pitch_left);
        src_v = src_v.wrapping_add(src_uv_pitch_left);
        dst_y1 = dst_y1
            .wrapping_add(dst_pitch_left)
            .wrapping_add(dst_y_pitch);
        dst_y2 = dst_y2
            .wrapping_add(dst_pitch_left)
            .wrapping_add(dst_y_pitch);
        dst_u1 = dst_u1
            .wrapping_add(dst_pitch_left)
            .wrapping_add(dst_uv_pitch);
        dst_u2 = dst_u2
            .wrapping_add(dst_pitch_left)
            .wrapping_add(dst_uv_pitch);
        dst_v1 = dst_v1
            .wrapping_add(dst_pitch_left)
            .wrapping_add(dst_uv_pitch);
        dst_v2 = dst_v2
            .wrapping_add(dst_pitch_left)
            .wrapping_add(dst_uv_pitch);
        y += 2;
    }

    // Last row
    if y == height - 1 {
        let mut x = 0isize;
        while x < width - 1 {
            // Row 1
            dst[dst_y1] = src[src_y1];
            src_y1 += 1;
            dst_y1 += 2;
            dst[dst_y1] = src[src_y1];
            src_y1 += 1;
            dst_y1 += 2;
            dst[dst_u1] = src[src_u];
            dst[dst_v1] = src[src_v];

            src_u += src_uv_pixel_stride;
            src_v += src_uv_pixel_stride;
            dst_u1 += 4;
            dst_v1 += 4;
            x += 2;
        }

        // Last column
        if x == width - 1 {
            // Row 1
            dst[dst_y1] = src[src_y1];
            dst_y1 += 2;
            dst[dst_y1] = src[src_y1];
            dst[dst_u1] = src[src_u];
            dst[dst_v1] = src[src_v];
        }
    }
    Ok(())
}

/// Translation of `SDL_ConvertPixels_Packed4_to_Planar2x2()`.
#[allow(clippy::too_many_arguments)]
fn convert_pixels_packed4_to_planar2x2(
    width: usize,
    height: usize,
    src_format: PixelFormat,
    src: &[u8],
    src_pitch: usize,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: usize,
) -> Result<()> {
    let sp = get_yuv_planes(width, height, src_format, src_pitch)?;
    check_len(src.len(), sp.extent, "source")?;
    let (mut src_y1, mut src_u1, mut src_v1) = (sp.y, sp.u, sp.v);
    let (src_y_pitch, src_uv_pitch) = (sp.y_stride, sp.uv_stride);
    let mut src_y2 = src_y1 + src_y_pitch;
    let mut src_u2 = src_u1 + src_uv_pitch;
    let mut src_v2 = src_v1 + src_uv_pitch;
    // The pitch "lefts" can be negative for a short pitch; the position
    // arithmetic wraps like upstream's pointers.
    let src_pitch_left = src_y_pitch.wrapping_sub(4 * width.div_ceil(2));

    let dp = get_yuv_planes(width, height, dst_format, dst_pitch)?;
    check_len(dst.len(), dp.extent, "destination")?;
    let (mut dst_y1, mut dst_u, mut dst_v) = (dp.y, dp.u, dp.v);
    let (dst_y_pitch, dst_uv_pitch) = (dp.y_stride, dp.uv_stride);
    let mut dst_y2 = dst_y1 + dst_y_pitch;
    let dst_y_pitch_left = dst_y_pitch.wrapping_sub(width);
    let (dst_uv_pixel_stride, dst_uv_pitch_left) =
        if dst_format == PixelFormat::NV12 || dst_format == PixelFormat::NV21 {
            (2, dst_uv_pitch.wrapping_sub(2 * width.div_ceil(2)))
        } else {
            (1, dst_uv_pitch.wrapping_sub(width.div_ceil(2)))
        };

    let (width, height) = (width as isize, height as isize);
    let avg = |a: u8, b: u8| ((a as u32 + b as u32) / 2) as u8;

    // Copy 2x2 blocks of pixels at a time
    let mut y = 0isize;
    while y < height - 1 {
        let mut x = 0isize;
        while x < width - 1 {
            // Row 1
            dst[dst_y1] = src[src_y1];
            dst_y1 += 1;
            src_y1 += 2;
            dst[dst_y1] = src[src_y1];
            dst_y1 += 1;
            src_y1 += 2;

            // Row 2
            dst[dst_y2] = src[src_y2];
            dst_y2 += 1;
            src_y2 += 2;
            dst[dst_y2] = src[src_y2];
            dst_y2 += 1;
            src_y2 += 2;

            dst[dst_u] = avg(src[src_u1], src[src_u2]);
            dst[dst_v] = avg(src[src_v1], src[src_v2]);

            src_u1 += 4;
            src_u2 += 4;
            src_v1 += 4;
            src_v2 += 4;
            dst_u += dst_uv_pixel_stride;
            dst_v += dst_uv_pixel_stride;
            x += 2;
        }

        // Last column
        if x == width - 1 {
            // Row 1 (the second Y of the macropixel overwrites the first)
            dst[dst_y1] = src[src_y1];
            src_y1 += 2;
            dst[dst_y1] = src[src_y1];
            dst_y1 += 1;
            src_y1 += 2;

            // Row 2
            dst[dst_y2] = src[src_y2];
            src_y2 += 2;
            dst[dst_y2] = src[src_y2];
            dst_y2 += 1;
            src_y2 += 2;

            dst[dst_u] = avg(src[src_u1], src[src_u2]);
            dst[dst_v] = avg(src[src_v1], src[src_v2]);

            src_u1 += 4;
            src_u2 += 4;
            src_v1 += 4;
            src_v2 += 4;
            dst_u += dst_uv_pixel_stride;
            dst_v += dst_uv_pixel_stride;
        }

        src_y1 = src_y1
            .wrapping_add(src_pitch_left)
            .wrapping_add(src_y_pitch);
        src_y2 = src_y2
            .wrapping_add(src_pitch_left)
            .wrapping_add(src_y_pitch);
        src_u1 = src_u1
            .wrapping_add(src_pitch_left)
            .wrapping_add(src_uv_pitch);
        src_u2 = src_u2
            .wrapping_add(src_pitch_left)
            .wrapping_add(src_uv_pitch);
        src_v1 = src_v1
            .wrapping_add(src_pitch_left)
            .wrapping_add(src_uv_pitch);
        src_v2 = src_v2
            .wrapping_add(src_pitch_left)
            .wrapping_add(src_uv_pitch);
        dst_y1 = dst_y1
            .wrapping_add(dst_y_pitch_left)
            .wrapping_add(dst_y_pitch);
        dst_y2 = dst_y2
            .wrapping_add(dst_y_pitch_left)
            .wrapping_add(dst_y_pitch);
        dst_u = dst_u.wrapping_add(dst_uv_pitch_left);
        dst_v = dst_v.wrapping_add(dst_uv_pitch_left);
        y += 2;
    }

    // Last row
    if y == height - 1 {
        let mut x = 0isize;
        while x < width - 1 {
            dst[dst_y1] = src[src_y1];
            dst_y1 += 1;
            src_y1 += 2;
            dst[dst_y1] = src[src_y1];
            dst_y1 += 1;
            src_y1 += 2;

            dst[dst_u] = src[src_u1];
            dst[dst_v] = src[src_v1];

            src_u1 += 4;
            src_v1 += 4;
            dst_u += dst_uv_pixel_stride;
            dst_v += dst_uv_pixel_stride;
            x += 2;
        }

        // Last column
        if x == width - 1 {
            dst[dst_y1] = src[src_y1];
            dst[dst_u] = src[src_u1];
            dst[dst_v] = src[src_v1];
        }
    }
    Ok(())
}

/// Translation of `SDL_ConvertPixels_YUV_to_YUV()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_pixels_yuv_to_yuv(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src_colorspace: Colorspace,
    _src_properties: Option<&Properties>,
    src: &[u8],
    src_pitch: i32,
    dst_format: PixelFormat,
    dst_colorspace: Colorspace,
    _dst_properties: Option<&Properties>,
    dst: &mut [u8],
    dst_pitch: i32,
) -> Result<()> {
    if src_colorspace != dst_colorspace {
        return Err(Error::new(
            "SDL_ConvertPixels_YUV_to_YUV: colorspace conversion not supported",
        ));
    }

    let (w, h, sp, dp) = check_dims(width, height, src_format, src_pitch, dst_format, dst_pitch)?;

    // Every conversion stays within the planes of its formats (the packed
    // and planar walkers check their own buffers).
    let check = |format: PixelFormat, len: usize, pitch: usize, what| match get_yuv_planes(
        w, h, format, pitch,
    ) {
        Ok(planes) => check_len(len, planes.extent, what),
        Err(_) => Ok(()),
    };

    if src_format == dst_format {
        check(src_format, src.len(), sp, "source")?;
        check(dst_format, dst.len(), dp, "destination")?;
        return convert_pixels_yuv_to_yuv_copy(w, h, src_format, src, sp, dst, dp);
    }

    if is_planar_2x2_format(src_format) && is_planar_2x2_format(dst_format) {
        check(src_format, src.len(), sp, "source")?;
        check(dst_format, dst.len(), dp, "destination")?;
        convert_pixels_planar2x2_to_planar2x2(w, h, src_format, src, sp, dst_format, dst, dp)
    } else if is_packed4_format(src_format) && is_packed4_format(dst_format) {
        check(src_format, src.len(), sp, "source")?;
        check(dst_format, dst.len(), dp, "destination")?;
        convert_pixels_packed4_to_packed4(w, h, src_format, src, sp, dst_format, dst, dp)
    } else if is_planar_2x2_format(src_format) && is_packed4_format(dst_format) {
        convert_pixels_planar2x2_to_packed4(w, h, src_format, src, sp, dst_format, dst, dp)
    } else if is_packed4_format(src_format) && is_planar_2x2_format(dst_format) {
        convert_pixels_packed4_to_planar2x2(w, h, src_format, src, sp, dst_format, dst, dp)
    } else {
        Err(Error::new(format!(
            "SDL_ConvertPixels_YUV_to_YUV: Unsupported YUV conversion: {} -> {}",
            src_format.name(),
            dst_format.name()
        )))
    }
}
