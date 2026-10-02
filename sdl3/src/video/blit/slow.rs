// Rust translation of src/video/SDL_blit_slow.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::video::pixels::{
    convert_color_primaries, pq_from_nits, pq_to_nits, srgb_from_linear, srgb_to_linear, ArrayOrder,
    Color, ColorPrimaries, PixelType, TransferCharacteristics,
};
use crate::video::surface::{get_hdr_headroom, get_sdr_white_point, PROP_SURFACE_HDR_HEADROOM_FLOAT, PROP_SURFACE_TONEMAP_OPERATOR_STRING};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SlowBlitPixelAccess {
    Index8,
    Rgb,
    Rgba,
    TenBit,
    Large,
}

fn get_pixel_access_method(format: PixelFormat) -> SlowBlitPixelAccess {
    if format.bytes_per_pixel() > 4 {
        SlowBlitPixelAccess::Large
    } else if format.is_10bit() {
        SlowBlitPixelAccess::TenBit
    } else if format == PixelFormat::INDEX8 {
        SlowBlitPixelAccess::Index8
    } else if format.has_alpha() {
        SlowBlitPixelAccess::Rgba
    } else {
        SlowBlitPixelAccess::Rgb
    }
}

/// A palette entry; upstream dereferences the palette unconditionally (the
/// blit is only chosen for INDEX8 with a palette, except through the float
/// path), so a missing palette or index reads as transparent black instead
/// of crashing.
fn pal_color(pal: Option<&Palette>, index: u32) -> Color {
    pal.and_then(|p| p.colors().get(index as usize).copied())
        .unwrap_or(Color::new(0, 0, 0, 0))
}

/// Translation of `SDL_LookupRGBAColor()`.
fn lookup_rgba_color(map: &mut PaletteMap, pixelvalue: u32, pal: Option<&Palette>) -> u8 {
    map.lookup(pixelvalue, pal)
}

fn read_8888_10bit(format: PixelFormat, pixel: u32) -> (u32, u32, u32, u32) {
    match format {
        PixelFormat::XRGB2101010 => {
            let (r, g, b, _) = rgba_from_argb2101010(pixel);
            (r, g, b, 0xFF)
        }
        PixelFormat::XBGR2101010 => {
            let (r, g, b, _) = rgba_from_abgr2101010(pixel);
            (r, g, b, 0xFF)
        }
        PixelFormat::ARGB2101010 => rgba_from_argb2101010(pixel),
        PixelFormat::ABGR2101010 => rgba_from_abgr2101010(pixel),
        _ => (0, 0, 0, 0),
    }
}

/* The ONE TRUE BLITTER
 * This puppy has to handle all the unoptimized cases - yes, it's slow.
 */
/// Translation of `SDL_Blit_Slow()`.
pub(crate) fn blit_slow(info: &mut BlitInfo<'_>) {
    let flags = info.flags;
    let modulate_r = info.r as u32;
    let modulate_g = info.g as u32;
    let modulate_b = info.b as u32;
    let modulate_a = info.a as u32;
    let mut srcpixel: u32 = 0;
    let (mut src_r, mut src_g, mut src_b, mut src_a) = (0u32, 0u32, 0u32, 0u32);
    let mut dstpixel: u32;
    let (mut dst_r, mut dst_g, mut dst_b, mut dst_a) = (0u32, 0u32, 0u32, 0u32);
    let src_fmt = info.src_fmt;
    let src_pal = info.src_pal;
    let dst_fmt = info.dst_fmt;
    let dst_pal = info.dst_pal;
    let srcbpp = src_fmt.bytes_per_pixel as usize;
    let dstbpp = dst_fmt.bytes_per_pixel as usize;
    let rgbmask = !src_fmt.Amask;
    let ckey = info.colorkey & rgbmask;
    let mut last_pixel: u32 = 0;
    let mut last_index: u8 = 0;

    let src_access = get_pixel_access_method(src_fmt.format);
    let dst_access = get_pixel_access_method(dst_fmt.format);
    if dst_access == SlowBlitPixelAccess::Index8 {
        last_index = lookup_rgba_color(info.palette_map, last_pixel, dst_pal);
    }

    let incy: u64 = if info.dst_h != 0 {
        ((info.src_h as u64) << 16) / info.dst_h as u64
    } else {
        0
    };
    let incx: u64 = if info.dst_w != 0 {
        ((info.src_w as u64) << 16) / info.dst_w as u64
    } else {
        0
    };
    let mut posy = incy / 2; // start at the middle of pixel

    let src_pitch = info.src_pitch as usize;
    let mut dst_row = 0usize;
    for _ in 0..info.dst_h {
        let mut dst = dst_row;
        let mut posx = incx / 2; // start at the middle of pixel
        let srcy = (posy >> 16) as usize;
        for _ in 0..info.dst_w {
            let srcx = (posx >> 16) as usize;
            let src = srcy * src_pitch + srcx * srcbpp;

            match src_access {
                SlowBlitPixelAccess::Index8 => {
                    srcpixel = info.src[src] as u32;
                    let c = pal_color(src_pal, srcpixel);
                    (src_r, src_g, src_b, src_a) = (c.r as u32, c.g as u32, c.b as u32, c.a as u32);
                }
                SlowBlitPixelAccess::Rgb => {
                    (srcpixel, src_r, src_g, src_b) = disemble_rgb(info.src, src, srcbpp, src_fmt);
                    src_a = 0xFF;
                }
                SlowBlitPixelAccess::Rgba => {
                    (srcpixel, src_r, src_g, src_b, src_a) = disemble_rgba(info.src, src, srcbpp, src_fmt);
                }
                SlowBlitPixelAccess::TenBit => {
                    srcpixel = rd32(info.src, src);
                    (src_r, src_g, src_b, src_a) = read_8888_10bit(src_fmt.format, srcpixel);
                }
                SlowBlitPixelAccess::Large => {
                    // Handled in SDL_Blit_Slow_Float()
                }
            }

            if flags & COPY_COLORKEY != 0 {
                // srcpixel isn't set for 24 bpp
                if srcbpp == 3 {
                    srcpixel = (src_r << src_fmt.Rshift) | (src_g << src_fmt.Gshift) | (src_b << src_fmt.Bshift);
                }
                if (srcpixel & rgbmask) == ckey {
                    posx += incx;
                    dst += dstbpp;
                    continue;
                }
            }
            if flags & COPY_BLEND_MASK != 0 {
                match dst_access {
                    SlowBlitPixelAccess::Index8 => {
                        dstpixel = info.dst[dst] as u32;
                        let c = pal_color(dst_pal, dstpixel);
                        (dst_r, dst_g, dst_b, dst_a) = (c.r as u32, c.g as u32, c.b as u32, c.a as u32);
                    }
                    SlowBlitPixelAccess::Rgb => {
                        (_, dst_r, dst_g, dst_b) = disemble_rgb(info.dst, dst, dstbpp, dst_fmt);
                        dst_a = 0xFF;
                    }
                    SlowBlitPixelAccess::Rgba => {
                        (_, dst_r, dst_g, dst_b, dst_a) = disemble_rgba(info.dst, dst, dstbpp, dst_fmt);
                    }
                    SlowBlitPixelAccess::TenBit => {
                        dstpixel = rd32(info.dst, dst);
                        (dst_r, dst_g, dst_b, dst_a) = read_8888_10bit(dst_fmt.format, dstpixel);
                    }
                    SlowBlitPixelAccess::Large => {
                        // Handled in SDL_Blit_Slow_Float()
                    }
                }
            } else {
                // don't care
            }

            if flags & COPY_MODULATE_COLOR != 0 {
                src_r = (src_r * modulate_r) / 255;
                src_g = (src_g * modulate_g) / 255;
                src_b = (src_b * modulate_b) / 255;
            }
            if flags & COPY_MODULATE_ALPHA != 0 {
                src_a = (src_a * modulate_a) / 255;
            }
            if flags & (COPY_BLEND | COPY_ADD) != 0 && src_a < 255 {
                src_r = (src_r * src_a) / 255;
                src_g = (src_g * src_a) / 255;
                src_b = (src_b * src_a) / 255;
            }
            match flags & COPY_BLEND_MASK {
                0 => {
                    dst_r = src_r;
                    dst_g = src_g;
                    dst_b = src_b;
                    dst_a = src_a;
                }
                COPY_BLEND => {
                    dst_r = src_r + ((255 - src_a) * dst_r) / 255;
                    dst_g = src_g + ((255 - src_a) * dst_g) / 255;
                    dst_b = src_b + ((255 - src_a) * dst_b) / 255;
                    dst_a = src_a + ((255 - src_a) * dst_a) / 255;
                }
                COPY_BLEND_PREMULTIPLIED => {
                    dst_r = (src_r + ((255 - src_a) * dst_r) / 255).min(255);
                    dst_g = (src_g + ((255 - src_a) * dst_g) / 255).min(255);
                    dst_b = (src_b + ((255 - src_a) * dst_b) / 255).min(255);
                    dst_a = (src_a + ((255 - src_a) * dst_a) / 255).min(255);
                }
                COPY_ADD | COPY_ADD_PREMULTIPLIED => {
                    dst_r = (src_r + dst_r).min(255);
                    dst_g = (src_g + dst_g).min(255);
                    dst_b = (src_b + dst_b).min(255);
                }
                COPY_MOD => {
                    dst_r = (src_r * dst_r) / 255;
                    dst_g = (src_g * dst_g) / 255;
                    dst_b = (src_b * dst_b) / 255;
                }
                COPY_MUL => {
                    dst_r = (((src_r * dst_r) + (dst_r * (255 - src_a))) / 255).min(255);
                    dst_g = (((src_g * dst_g) + (dst_g * (255 - src_a))) / 255).min(255);
                    dst_b = (((src_b * dst_b) + (dst_b * (255 - src_a))) / 255).min(255);
                }
                _ => {}
            }

            match dst_access {
                SlowBlitPixelAccess::Index8 => {
                    dstpixel = (dst_r << 24) | (dst_g << 16) | (dst_b << 8) | dst_a;
                    if dstpixel != last_pixel {
                        last_pixel = dstpixel;
                        last_index = lookup_rgba_color(info.palette_map, dstpixel, dst_pal);
                    }
                    info.dst[dst] = last_index;
                }
                SlowBlitPixelAccess::Rgb => {
                    assemble_rgb(info.dst, dst, dstbpp, dst_fmt, dst_r, dst_g, dst_b);
                }
                SlowBlitPixelAccess::Rgba => {
                    assemble_rgba(info.dst, dst, dstbpp, dst_fmt, dst_r, dst_g, dst_b, dst_a);
                }
                SlowBlitPixelAccess::TenBit => {
                    let pixelvalue = match dst_fmt.format {
                        PixelFormat::XRGB2101010 => {
                            dst_a = 0xFF;
                            argb2101010_from_rgba(dst_r, dst_g, dst_b, dst_a)
                        }
                        PixelFormat::ARGB2101010 => argb2101010_from_rgba(dst_r, dst_g, dst_b, dst_a),
                        PixelFormat::XBGR2101010 => {
                            dst_a = 0xFF;
                            abgr2101010_from_rgba(dst_r, dst_g, dst_b, dst_a)
                        }
                        PixelFormat::ABGR2101010 => abgr2101010_from_rgba(dst_r, dst_g, dst_b, dst_a),
                        _ => 0,
                    };
                    wr32(info.dst, dst, pixelvalue);
                }
                SlowBlitPixelAccess::Large => {
                    // Handled in SDL_Blit_Slow_Float()
                }
            }

            posx += incx;
            dst += dstbpp;
        }
        posy += incy;
        dst_row += info.dst_pitch as usize;
    }
}

/* Convert from F16 to float
 * Public domain implementation from https://gist.github.com/rygorous/2144712
 */
/// Translation of `half_to_float()`.
pub(crate) fn half_to_float(un_value: u16) -> f32 {
    let magic = f32::from_bits((254 - 15) << 23);
    let was_infnan = f32::from_bits((127 + 16) << 23);

    let h = un_value as u32;
    let mut o = f32::from_bits((h & 0x7fff) << 13); // exponent/mantissa bits
    o *= magic; // exponent adjust
    let mut bits = o.to_bits();
    if o >= was_infnan {
        // make sure Inf/NaN survive
        bits |= 255 << 23;
    }
    bits |= (h & 0x8000) << 16; // sign bit
    f32::from_bits(bits)
}

/* Convert from float to F16
 * Public domain implementation from https://stackoverflow.com/questions/76799117/how-to-convert-a-float-to-a-half-type-and-the-other-way-around-in-c
 */
/// Translation of `float_to_half()`.
pub(crate) fn float_to_half(a: f32) -> u16 {
    let mut ia = a.to_bits();
    let mut ir: u16 = ((ia >> 16) & 0x8000) as u16;
    if (ia & 0x7f800000) == 0x7f800000 {
        if (ia & 0x7fffffff) == 0x7f800000 {
            ir |= 0x7c00; // infinity
        } else {
            ir |= 0x7e00 | ((ia >> (24 - 11)) & 0x1ff) as u16; // NaN, quietened
        }
    } else if (ia & 0x7f800000) >= 0x33000000 {
        let shift = ((ia >> 23) & 0xff) as i32 - 127;
        if shift > 15 {
            ir |= 0x7c00; // infinity
        } else {
            ia = (ia & 0x007fffff) | 0x00800000; // extract mantissa
            if shift < -14 {
                // denormal
                ir |= (ia >> (-1 - shift)) as u16;
                ia <<= 32 - (-1 - shift);
            } else {
                // normal
                ir |= (ia >> (24 - 11)) as u16;
                ia <<= 32 - (24 - 11);
                ir = ir.wrapping_add(((14 + shift) << 10) as u16);
            }
            // IEEE-754 round to nearest of even
            if (ia > 0x80000000) || ((ia == 0x80000000) && (ir & 1) != 0) {
                ir = ir.wrapping_add(1);
            }
        }
    }
    ir
}

fn read_large(pixels: &[u8], i: usize, fmt: &PixelFormatDetails) -> [f32; 4] {
    let u16_at = |k: usize| u16::from_ne_bytes([pixels[i + 2 * k], pixels[i + 2 * k + 1]]);
    let f32_at = |k: usize| f32::from_bits(rd32(pixels, i + 4 * k));
    match fmt.format.pixel_type() {
        PixelType::ArrayU16 => [
            u16_at(0) as f32 / u16::MAX as f32,
            u16_at(1) as f32 / u16::MAX as f32,
            u16_at(2) as f32 / u16::MAX as f32,
            if fmt.bytes_per_pixel == 8 {
                u16_at(3) as f32 / u16::MAX as f32
            } else {
                1.0
            },
        ],
        PixelType::ArrayF16 => [
            half_to_float(u16_at(0)),
            half_to_float(u16_at(1)),
            half_to_float(u16_at(2)),
            if fmt.bytes_per_pixel == 8 {
                half_to_float(u16_at(3))
            } else {
                1.0
            },
        ],
        PixelType::ArrayF32 => [
            f32_at(0),
            f32_at(1),
            f32_at(2),
            if fmt.bytes_per_pixel == 16 { f32_at(3) } else { 1.0 },
        ],
        // Unknown array type
        _ => [0.0; 4],
    }
}

fn array_order(fmt: &PixelFormatDetails) -> Option<ArrayOrder> {
    let order = fmt.format.pixel_order();
    [
        ArrayOrder::Rgb,
        ArrayOrder::Rgba,
        ArrayOrder::Argb,
        ArrayOrder::Bgr,
        ArrayOrder::Bgra,
        ArrayOrder::Abgr,
    ]
    .into_iter()
    .find(|o| *o as u32 == order)
}

/// Translation of `ReadFloatPixel()`: returns `(R, G, B, A)`.
fn read_float_pixel(
    pixels: &[u8],
    i: usize,
    access: SlowBlitPixelAccess,
    fmt: &PixelFormatDetails,
    pal: Option<&Palette>,
    colorspace: Colorspace,
    sdr_white_point: f32,
) -> (f32, f32, f32, f32) {
    let (mut f_r, mut f_g, mut f_b, f_a);

    match access {
        SlowBlitPixelAccess::Index8 => {
            let c = pal_color(pal, pixels[i] as u32);
            f_r = c.r as f32 / 255.0;
            f_g = c.g as f32 / 255.0;
            f_b = c.b as f32 / 255.0;
            f_a = c.a as f32 / 255.0;
        }
        SlowBlitPixelAccess::Rgb => {
            let (_, r, g, b) = disemble_rgb(pixels, i, fmt.bytes_per_pixel as usize, fmt);
            f_r = r as f32 / 255.0;
            f_g = g as f32 / 255.0;
            f_b = b as f32 / 255.0;
            f_a = 1.0;
        }
        SlowBlitPixelAccess::Rgba => {
            let (_, r, g, b, a) = disemble_rgba(pixels, i, fmt.bytes_per_pixel as usize, fmt);
            f_r = r as f32 / 255.0;
            f_g = g as f32 / 255.0;
            f_b = b as f32 / 255.0;
            f_a = a as f32 / 255.0;
        }
        SlowBlitPixelAccess::TenBit => {
            let pixelvalue = rd32(pixels, i);
            (f_r, f_g, f_b, f_a) = match fmt.format {
                PixelFormat::XRGB2101010 => {
                    let (r, g, b, _) = rgbafloat_from_argb2101010(pixelvalue);
                    (r, g, b, 1.0)
                }
                PixelFormat::XBGR2101010 => {
                    let (r, g, b, _) = rgbafloat_from_abgr2101010(pixelvalue);
                    (r, g, b, 1.0)
                }
                PixelFormat::ARGB2101010 => rgbafloat_from_argb2101010(pixelvalue),
                PixelFormat::ABGR2101010 => rgbafloat_from_abgr2101010(pixelvalue),
                _ => (0.0, 0.0, 0.0, 0.0),
            };
        }
        SlowBlitPixelAccess::Large => {
            let v = read_large(pixels, i, fmt);
            (f_r, f_g, f_b, f_a) = match array_order(fmt) {
                Some(ArrayOrder::Rgb) => (v[0], v[1], v[2], 1.0),
                Some(ArrayOrder::Rgba) => (v[0], v[1], v[2], v[3]),
                Some(ArrayOrder::Argb) => (v[1], v[2], v[3], v[0]),
                Some(ArrayOrder::Bgr) => (v[2], v[1], v[0], 1.0),
                Some(ArrayOrder::Bgra) => (v[2], v[1], v[0], v[3]),
                Some(ArrayOrder::Abgr) => (v[3], v[2], v[1], v[0]),
                // Unknown array order
                _ => (0.0, 0.0, 0.0, 0.0),
            };
        }
    }

    // Convert to nits so src and dst are guaranteed to be linear and in the same units
    match colorspace.transfer() {
        TransferCharacteristics::Srgb => {
            f_r = srgb_to_linear(f_r);
            f_g = srgb_to_linear(f_g);
            f_b = srgb_to_linear(f_b);
        }
        TransferCharacteristics::Pq => {
            f_r = pq_to_nits(f_r) / sdr_white_point;
            f_g = pq_to_nits(f_g) / sdr_white_point;
            f_b = pq_to_nits(f_b) / sdr_white_point;
        }
        TransferCharacteristics::Linear => {
            f_r /= sdr_white_point;
            f_g /= sdr_white_point;
            f_b /= sdr_white_point;
        }
        _ => {
            // Unknown, leave it alone
        }
    }

    (f_r, f_g, f_b, f_a)
}

fn to_u8(v: f32) -> u32 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8 as u32
}

/// Translation of `WriteFloatPixel()`.
#[allow(clippy::too_many_arguments)]
fn write_float_pixel(
    pixels: &mut [u8],
    i: usize,
    access: SlowBlitPixelAccess,
    fmt: &PixelFormatDetails,
    colorspace: Colorspace,
    sdr_white_point: f32,
    mut f_r: f32,
    mut f_g: f32,
    mut f_b: f32,
    mut f_a: f32,
) {
    // We converted to nits so src and dst are guaranteed to be linear and in the same units
    match colorspace.transfer() {
        TransferCharacteristics::Srgb => {
            f_r = srgb_from_linear(f_r);
            f_g = srgb_from_linear(f_g);
            f_b = srgb_from_linear(f_b);
        }
        TransferCharacteristics::Pq => {
            f_r = pq_from_nits(f_r * sdr_white_point);
            f_g = pq_from_nits(f_g * sdr_white_point);
            f_b = pq_from_nits(f_b * sdr_white_point);
        }
        TransferCharacteristics::Linear => {
            f_r *= sdr_white_point;
            f_g *= sdr_white_point;
            f_b *= sdr_white_point;
        }
        _ => {
            // Unknown, leave it alone
        }
    }

    let bpp = fmt.bytes_per_pixel as usize;
    match access {
        SlowBlitPixelAccess::Index8 => {
            // This should never happen, checked before this call
            crate::sdl_assert!(false);
        }
        SlowBlitPixelAccess::Rgb => {
            assemble_rgb(pixels, i, bpp, fmt, to_u8(f_r), to_u8(f_g), to_u8(f_b));
        }
        SlowBlitPixelAccess::Rgba => {
            assemble_rgba(pixels, i, bpp, fmt, to_u8(f_r), to_u8(f_g), to_u8(f_b), to_u8(f_a));
        }
        SlowBlitPixelAccess::TenBit => {
            let pixelvalue = match fmt.format {
                PixelFormat::XRGB2101010 => {
                    f_a = 1.0;
                    argb2101010_from_rgbafloat(f_r, f_g, f_b, f_a)
                }
                PixelFormat::ARGB2101010 => argb2101010_from_rgbafloat(f_r, f_g, f_b, f_a),
                PixelFormat::XBGR2101010 => {
                    f_a = 1.0;
                    abgr2101010_from_rgbafloat(f_r, f_g, f_b, f_a)
                }
                PixelFormat::ABGR2101010 => abgr2101010_from_rgbafloat(f_r, f_g, f_b, f_a),
                _ => 0,
            };
            wr32(pixels, i, pixelvalue);
        }
        SlowBlitPixelAccess::Large => {
            let v = match array_order(fmt) {
                Some(ArrayOrder::Rgb) => [f_r, f_g, f_b, 1.0],
                Some(ArrayOrder::Rgba) => [f_r, f_g, f_b, f_a],
                Some(ArrayOrder::Argb) => [f_a, f_r, f_g, f_b],
                Some(ArrayOrder::Bgr) => [f_b, f_g, f_r, 1.0],
                Some(ArrayOrder::Bgra) => [f_b, f_g, f_r, f_a],
                Some(ArrayOrder::Abgr) => [f_a, f_b, f_g, f_r],
                // Unknown array order
                _ => [0.0; 4],
            };
            let put16 = |p: &mut [u8], k: usize, x: u16| p[i + 2 * k..i + 2 * k + 2].copy_from_slice(&x.to_ne_bytes());
            let to_u16 = |x: f32| (x.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16;
            match fmt.format.pixel_type() {
                PixelType::ArrayU16 => {
                    put16(pixels, 0, to_u16(v[0]));
                    put16(pixels, 1, to_u16(v[1]));
                    put16(pixels, 2, to_u16(v[2]));
                    if fmt.bytes_per_pixel == 8 {
                        put16(pixels, 3, to_u16(v[3]));
                    }
                }
                PixelType::ArrayF16 => {
                    put16(pixels, 0, float_to_half(v[0]));
                    put16(pixels, 1, float_to_half(v[1]));
                    put16(pixels, 2, float_to_half(v[2]));
                    if fmt.bytes_per_pixel == 8 {
                        put16(pixels, 3, float_to_half(v[3]));
                    }
                }
                PixelType::ArrayF32 => {
                    wr32(pixels, i, v[0].to_bits());
                    wr32(pixels, i + 4, v[1].to_bits());
                    wr32(pixels, i + 8, v[2].to_bits());
                    if fmt.bytes_per_pixel == 16 {
                        wr32(pixels, i + 12, v[3].to_bits());
                    }
                }
                _ => {
                    // Unknown array type
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum TonemapOperator {
    None,
    Linear { scale: f32 },
    Chrome { a: f32, b: f32, color_primaries_matrix: Option<&'static crate::video::pixels::PrimariesMatrix> },
}

fn tonemap_linear(r: &mut f32, g: &mut f32, b: &mut f32, scale: f32) {
    *r *= scale;
    *g *= scale;
    *b *= scale;
}

/* This uses the same tonemapping algorithm developed by Google for Chrome:
 * https://colab.research.google.com/drive/1hI10nq6L6ru_UFvz7-f7xQaQp0qarz_K
 *
 * Essentially, you use the source headroom and the destination headroom
 * to calculate scaling factors:
 *  tonemap_a = (dst_headroom / (src_headroom * src_headroom));
 *  tonemap_b = (1.0f / dst_headroom);
 *
 * Then you normalize your source color by the HDR whitepoint,
 * and calculate a final scaling factor in BT.2020 colorspace.
 */
fn tonemap_chrome(r: &mut f32, g: &mut f32, b: &mut f32, tonemap_a: f32, tonemap_b: f32) {
    let v1 = *r;
    let v2 = *g;
    let v3 = *b;
    let vmax = c_max(v1, c_max(v2, v3));

    if vmax > 0.0 {
        let scale = (1.0 + tonemap_a * vmax) / (1.0 + tonemap_b * vmax);
        tonemap_linear(r, g, b, scale);
    }
}

/// `SDL_max()`: `(x > y) ? x : y`, which differs from `f32::max` for NaN.
fn c_max(x: f32, y: f32) -> f32 {
    if x > y {
        x
    } else {
        y
    }
}

fn apply_tonemap(op: &TonemapOperator, r: &mut f32, g: &mut f32, b: &mut f32) {
    match *op {
        TonemapOperator::Linear { scale } => tonemap_linear(r, g, b, scale),
        TonemapOperator::Chrome { a, b: tb, color_primaries_matrix } => {
            if let Some(m) = color_primaries_matrix {
                [*r, *g, *b] = convert_color_primaries([*r, *g, *b], m);
            }
            tonemap_chrome(r, g, b, a, tb);
        }
        TonemapOperator::None => {}
    }
}

/* The SECOND TRUE BLITTER
 * This one is even slower than the first, but also handles large pixel formats and colorspace conversion
 */
/// Translation of `SDL_Blit_Slow_Float()`.
pub(crate) fn blit_slow_float(info: &mut BlitInfo<'_>) {
    let flags = info.flags;
    let modulate_r = info.r as f32;
    let modulate_g = info.g as f32;
    let modulate_b = info.b as f32;
    let modulate_a = info.a as f32;
    let src_fmt = info.src_fmt;
    let src_pal = info.src_pal;
    let dst_fmt = info.dst_fmt;
    let dst_pal = info.dst_pal;
    let srcbpp = src_fmt.bytes_per_pixel as usize;
    let dstbpp = dst_fmt.bytes_per_pixel as usize;
    let mut color_primaries_matrix = None;
    let mut last_pixel: u32 = 0;
    let mut last_index: u8 = 0;

    let src_colorspace = info.src_colorspace;
    let dst_colorspace = info.dst_colorspace;
    let mut src_primaries = src_colorspace.primaries();
    let dst_primaries = dst_colorspace.primaries();

    let src_white_point = get_sdr_white_point(info.src_props, src_colorspace);
    let dst_white_point = get_sdr_white_point(info.dst_props, dst_colorspace);
    let src_headroom = get_hdr_headroom(info.src_props, src_colorspace);
    let mut dst_headroom = get_hdr_headroom(info.dst_props, dst_colorspace);
    if dst_headroom == 0.0 {
        // The destination will have the same headroom as the source
        dst_headroom = src_headroom;
        if let Some(props) = info.dst_props {
            let _ = props.set(PROP_SURFACE_HDR_HEADROOM_FLOAT, dst_headroom);
        }
    }

    let mut tonemap = TonemapOperator::None;

    if src_headroom > dst_headroom {
        let tonemap_operator = info.src_props.and_then(|p| p.get_string(PROP_SURFACE_TONEMAP_OPERATOR_STRING));
        let mut chrome = false;
        if let Some(op) = tonemap_operator {
            if op.as_bytes().starts_with(b"*=") {
                tonemap = TonemapOperator::Linear {
                    scale: crate::stdlib::string::strtod(&op[2..]).0 as f32,
                };
            } else if crate::stdlib::string::strcasecmp(&op, "chrome").is_eq() {
                chrome = true;
            } else if crate::stdlib::string::strcasecmp(&op, "none").is_eq() {
                tonemap = TonemapOperator::None;
            }
        } else {
            chrome = true;
        }
        if chrome {
            // We'll convert to BT.2020 primaries for the tonemap operation
            let m = src_primaries.conversion_matrix_to(ColorPrimaries::Bt2020);
            if m.is_some() {
                src_primaries = ColorPrimaries::Bt2020;
            }
            tonemap = TonemapOperator::Chrome {
                a: dst_headroom / (src_headroom * src_headroom),
                b: 1.0 / dst_headroom,
                color_primaries_matrix: m,
            };
        }
    }

    if src_primaries != dst_primaries {
        color_primaries_matrix = src_primaries.conversion_matrix_to(dst_primaries);
    }

    let src_access = get_pixel_access_method(src_fmt.format);
    let dst_access = get_pixel_access_method(dst_fmt.format);
    if dst_access == SlowBlitPixelAccess::Index8 {
        last_index = lookup_rgba_color(info.palette_map, last_pixel, dst_pal);
    }

    let incy: u64 = ((info.src_h as u64) << 16) / info.dst_h as u64;
    let incx: u64 = ((info.src_w as u64) << 16) / info.dst_w as u64;
    let mut posy = incy / 2; // start at the middle of pixel

    let src_pitch = info.src_pitch as usize;
    let mut dst_row = 0usize;
    for _ in 0..info.dst_h {
        let mut dst = dst_row;
        let mut posx = incx / 2; // start at the middle of pixel
        let srcy = (posy >> 16) as usize;
        for _ in 0..info.dst_w {
            let srcx = (posx >> 16) as usize;
            let src = srcy * src_pitch + srcx * srcbpp;

            let (mut src_r, mut src_g, mut src_b, mut src_a) =
                read_float_pixel(info.src, src, src_access, src_fmt, src_pal, src_colorspace, src_white_point);

            if tonemap != TonemapOperator::None {
                apply_tonemap(&tonemap, &mut src_r, &mut src_g, &mut src_b);
            }

            if let Some(m) = color_primaries_matrix {
                [src_r, src_g, src_b] = convert_color_primaries([src_r, src_g, src_b], m);
            }

            if flags & COPY_COLORKEY != 0 {
                // colorkey isn't supported
            }
            let (mut dst_r, mut dst_g, mut dst_b, mut dst_a) =
                if flags & (COPY_BLEND | COPY_ADD | COPY_MOD | COPY_MUL) != 0 {
                    read_float_pixel(info.dst, dst, dst_access, dst_fmt, dst_pal, dst_colorspace, dst_white_point)
                } else {
                    // don't care
                    (0.0, 0.0, 0.0, 0.0)
                };

            if flags & COPY_MODULATE_COLOR != 0 {
                src_r = (src_r * modulate_r) / 255.0;
                src_g = (src_g * modulate_g) / 255.0;
                src_b = (src_b * modulate_b) / 255.0;
            }
            if flags & COPY_MODULATE_ALPHA != 0 {
                src_a = (src_a * modulate_a) / 255.0;
            }
            if flags & (COPY_BLEND | COPY_ADD) != 0 && src_a < 1.0 {
                src_r *= src_a;
                src_g *= src_a;
                src_b *= src_a;
            }
            match flags & (COPY_BLEND | COPY_ADD | COPY_MOD | COPY_MUL) {
                0 => {
                    dst_r = src_r;
                    dst_g = src_g;
                    dst_b = src_b;
                    dst_a = src_a;
                }
                COPY_BLEND => {
                    dst_r = src_r + ((1.0 - src_a) * dst_r);
                    dst_g = src_g + ((1.0 - src_a) * dst_g);
                    dst_b = src_b + ((1.0 - src_a) * dst_b);
                    dst_a = src_a + ((1.0 - src_a) * dst_a);
                }
                COPY_ADD => {
                    dst_r += src_r;
                    dst_g += src_g;
                    dst_b += src_b;
                }
                COPY_MOD => {
                    dst_r *= src_r;
                    dst_g *= src_g;
                    dst_b *= src_b;
                }
                COPY_MUL => {
                    dst_r = (src_r * dst_r) + (dst_r * (1.0 - src_a));
                    dst_g = (src_g * dst_g) + (dst_g * (1.0 - src_a));
                    dst_b = (src_b * dst_b) + (dst_b * (1.0 - src_a));
                }
                _ => {}
            }

            if dst_access == SlowBlitPixelAccess::Index8 {
                let r = to_u8(srgb_from_linear(dst_r));
                let g = to_u8(srgb_from_linear(dst_g));
                let b = to_u8(srgb_from_linear(dst_b));
                let a = to_u8(dst_a);
                let dstpixel = (r << 24) | (g << 16) | (b << 8) | a;
                if dstpixel != last_pixel {
                    last_pixel = dstpixel;
                    last_index = lookup_rgba_color(info.palette_map, dstpixel, dst_pal);
                }
                info.dst[dst] = last_index;
            } else {
                write_float_pixel(
                    info.dst,
                    dst,
                    dst_access,
                    dst_fmt,
                    dst_colorspace,
                    dst_white_point,
                    dst_r,
                    dst_g,
                    dst_b,
                    dst_a,
                );
            }

            posx += incx;
            dst += dstbpp;
        }
        posy += incy;
        dst_row += info.dst_pitch as usize;
    }
}
