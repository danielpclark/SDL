// Rust translation of the pixel macros of src/video/SDL_blit.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The macros of `SDL_blit.h` as inline functions. Pixels are read and
//! written in native byte order, like the C pointer casts.

// Each blitter uses its own subset of these.
#![allow(dead_code)]

use crate::video::pixels::{PixelFormatDetails, EXPAND_BYTE};

#[inline(always)]
pub(crate) fn rd16(b: &[u8], i: usize) -> u32 {
    u16::from_ne_bytes([b[i], b[i + 1]]) as u32
}
#[inline(always)]
pub(crate) fn rd32(b: &[u8], i: usize) -> u32 {
    u32::from_ne_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
#[inline(always)]
pub(crate) fn wr16(b: &mut [u8], i: usize, v: u32) {
    b[i..i + 2].copy_from_slice(&(v as u16).to_ne_bytes());
}
#[inline(always)]
pub(crate) fn wr32(b: &mut [u8], i: usize, v: u32) {
    b[i..i + 4].copy_from_slice(&v.to_ne_bytes());
}

/// Translation of `RGB_FROM_PIXEL()`.
#[inline(always)]
pub(crate) fn rgb_from_pixel(pixel: u32, fmt: &PixelFormatDetails) -> (u32, u32, u32) {
    (
        EXPAND_BYTE[fmt.Rbits as usize][((pixel & fmt.Rmask) >> fmt.Rshift) as usize] as u32,
        EXPAND_BYTE[fmt.Gbits as usize][((pixel & fmt.Gmask) >> fmt.Gshift) as usize] as u32,
        EXPAND_BYTE[fmt.Bbits as usize][((pixel & fmt.Bmask) >> fmt.Bshift) as usize] as u32,
    )
}

/// Translation of `RGBA_FROM_PIXEL()`.
// FIXME: Should we rescale alpha into 0..255 here?
#[inline(always)]
pub(crate) fn rgba_from_pixel(pixel: u32, fmt: &PixelFormatDetails) -> (u32, u32, u32, u32) {
    let (r, g, b) = rgb_from_pixel(pixel, fmt);
    (
        r,
        g,
        b,
        EXPAND_BYTE[fmt.Abits as usize][((pixel & fmt.Amask) >> fmt.Ashift) as usize] as u32,
    )
}

/// Translation of `GET_RGB24()`.
#[inline(always)]
pub(crate) fn get_rgb24(b: &[u8], i: usize) -> u32 {
    if cfg!(target_endian = "little") {
        ((b[i + 2] as u32) << 16) | ((b[i + 1] as u32) << 8) | b[i] as u32
    } else {
        ((b[i] as u32) << 16) | ((b[i + 1] as u32) << 8) | b[i + 2] as u32
    }
}

/// Translation of `RETRIEVE_RGB_PIXEL()`.
#[inline(always)]
pub(crate) fn retrieve_rgb_pixel(b: &[u8], i: usize, bpp: usize) -> u32 {
    match bpp {
        1 => b[i] as u32,
        2 => rd16(b, i),
        3 => get_rgb24(b, i),
        4 => rd32(b, i),
        _ => 0, // stop gcc complaints
    }
}

/// Byte offset of a 24-bit pixel's component. Translation of `GET_RGB24_COMPONENT()`.
#[inline(always)]
pub(crate) fn rgb24_component(shift: u8) -> usize {
    if cfg!(target_endian = "little") {
        shift as usize / 8
    } else {
        2 - shift as usize / 8
    }
}

/// Translation of `DISEMBLE_RGB()`: returns `(Pixel, r, g, b)`.
#[inline(always)]
pub(crate) fn disemble_rgb(
    b: &[u8],
    i: usize,
    bpp: usize,
    fmt: &PixelFormatDetails,
) -> (u32, u32, u32, u32) {
    match bpp {
        1 | 2 | 4 => {
            let pixel = match bpp {
                1 => b[i] as u32,
                2 => rd16(b, i),
                _ => rd32(b, i),
            };
            let (r, g, bl) = rgb_from_pixel(pixel, fmt);
            (pixel, r, g, bl)
        }
        3 => (
            0,
            b[i + rgb24_component(fmt.Rshift)] as u32,
            b[i + rgb24_component(fmt.Gshift)] as u32,
            b[i + rgb24_component(fmt.Bshift)] as u32,
        ),
        _ => (0, 0, 0, 0), // stop gcc complaints
    }
}

/// Translation of `DISEMBLE_RGBA()`: returns `(Pixel, r, g, b, a)`.
#[inline(always)]
pub(crate) fn disemble_rgba(
    b: &[u8],
    i: usize,
    bpp: usize,
    fmt: &PixelFormatDetails,
) -> (u32, u32, u32, u32, u32) {
    match bpp {
        1 | 2 | 4 => {
            let pixel = match bpp {
                1 => b[i] as u32,
                2 => rd16(b, i),
                _ => rd32(b, i),
            };
            let (r, g, bl, a) = rgba_from_pixel(pixel, fmt);
            (pixel, r, g, bl, a)
        }
        3 => (
            0,
            b[i + rgb24_component(fmt.Rshift)] as u32,
            b[i + rgb24_component(fmt.Gshift)] as u32,
            b[i + rgb24_component(fmt.Bshift)] as u32,
            0xFF,
        ),
        _ => (0, 0, 0, 0, 0), // stop gcc complaints
    }
}

use crate::video::pixels::narrow_channel as narrow;

/// Translation of `PIXEL_FROM_RGB()`.
#[inline(always)]
pub(crate) fn pixel_from_rgb(fmt: &PixelFormatDetails, r: u32, g: u32, b: u32) -> u32 {
    (narrow(r, fmt.Rbits) << fmt.Rshift)
        | (narrow(g, fmt.Gbits) << fmt.Gshift)
        | (narrow(b, fmt.Bbits) << fmt.Bshift)
        | fmt.Amask
}

/// Translation of `PIXEL_FROM_RGBA()`.
// FIXME: this isn't correct, especially for Alpha (maximum != 255)
#[inline(always)]
pub(crate) fn pixel_from_rgba(fmt: &PixelFormatDetails, r: u32, g: u32, b: u32, a: u32) -> u32 {
    (narrow(r, fmt.Rbits) << fmt.Rshift)
        | (narrow(g, fmt.Gbits) << fmt.Gshift)
        | (narrow(b, fmt.Bbits) << fmt.Bshift)
        | (narrow(a, fmt.Abits) << fmt.Ashift)
}

/// Store the low `bpp` bytes of a 1-, 2- or 4-byte pixel value.
#[inline(always)]
pub(crate) fn store_pixel(buf: &mut [u8], i: usize, bpp: usize, pixel: u32) {
    match bpp {
        1 => buf[i] = pixel as u8,
        2 => wr16(buf, i, pixel),
        4 => wr32(buf, i, pixel),
        _ => {}
    }
}

/// Translation of `ASSEMBLE_RGB()`.
#[inline(always)]
pub(crate) fn assemble_rgb(
    buf: &mut [u8],
    i: usize,
    bpp: usize,
    fmt: &PixelFormatDetails,
    r: u32,
    g: u32,
    b: u32,
) {
    if bpp == 3 {
        buf[i + rgb24_component(fmt.Rshift)] = r as u8;
        buf[i + rgb24_component(fmt.Gshift)] = g as u8;
        buf[i + rgb24_component(fmt.Bshift)] = b as u8;
    } else {
        store_pixel(buf, i, bpp, pixel_from_rgb(fmt, r, g, b));
    }
}

/// Translation of `ASSEMBLE_RGBA()`.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_rgba(
    buf: &mut [u8],
    i: usize,
    bpp: usize,
    fmt: &PixelFormatDetails,
    r: u32,
    g: u32,
    b: u32,
    a: u32,
) {
    if bpp == 3 {
        buf[i + rgb24_component(fmt.Rshift)] = r as u8;
        buf[i + rgb24_component(fmt.Gshift)] = g as u8;
        buf[i + rgb24_component(fmt.Bshift)] = b as u8;
    } else {
        store_pixel(buf, i, bpp, pixel_from_rgba(fmt, r, g, b, a));
    }
}

/// Translation of `RGBA_FROM_8888()`.
#[inline(always)]
pub(crate) fn rgba_from_8888(pixel: u32, fmt: &PixelFormatDetails) -> (u32, u32, u32, u32) {
    (
        (pixel & fmt.Rmask) >> fmt.Rshift,
        (pixel & fmt.Gmask) >> fmt.Gshift,
        (pixel & fmt.Bmask) >> fmt.Bshift,
        (pixel & fmt.Amask) >> fmt.Ashift,
    )
}

/// Translation of `RGBA_FROM_RGBA8888()`.
#[inline(always)]
pub(crate) fn rgba_from_rgba8888(p: u32) -> (u32, u32, u32, u32) {
    (p >> 24, (p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF)
}
/// Translation of `RGBA_FROM_ARGB8888()`.
#[inline(always)]
pub(crate) fn rgba_from_argb8888(p: u32) -> (u32, u32, u32, u32) {
    ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF, p >> 24)
}
/// Translation of `RGBA_FROM_ABGR8888()`.
#[inline(always)]
pub(crate) fn rgba_from_abgr8888(p: u32) -> (u32, u32, u32, u32) {
    (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF, p >> 24)
}
/// Translation of `RGBA_FROM_BGRA8888()`.
#[inline(always)]
pub(crate) fn rgba_from_bgra8888(p: u32) -> (u32, u32, u32, u32) {
    ((p >> 8) & 0xFF, (p >> 16) & 0xFF, p >> 24, p & 0xFF)
}
/// Translation of `RGBA_FROM_ARGB2101010()`.
#[inline(always)]
pub(crate) fn rgba_from_argb2101010(p: u32) -> (u32, u32, u32, u32) {
    (
        (p >> 22) & 0xFF,
        (p >> 12) & 0xFF,
        (p >> 2) & 0xFF,
        EXPAND_BYTE[2][(p >> 30) as usize] as u32,
    )
}
/// Translation of `RGBA_FROM_ABGR2101010()`.
#[inline(always)]
pub(crate) fn rgba_from_abgr2101010(p: u32) -> (u32, u32, u32, u32) {
    (
        (p >> 2) & 0xFF,
        (p >> 12) & 0xFF,
        (p >> 22) & 0xFF,
        EXPAND_BYTE[2][(p >> 30) as usize] as u32,
    )
}
/// Translation of `RGBAFLOAT_FROM_ARGB2101010()`.
#[inline(always)]
pub(crate) fn rgbafloat_from_argb2101010(p: u32) -> (f32, f32, f32, f32) {
    (
        ((p >> 20) & 0x3FF) as f32 / 1023.0,
        ((p >> 10) & 0x3FF) as f32 / 1023.0,
        (p & 0x3FF) as f32 / 1023.0,
        (p >> 30) as f32 / 3.0,
    )
}
/// Translation of `RGBAFLOAT_FROM_ABGR2101010()`.
#[inline(always)]
pub(crate) fn rgbafloat_from_abgr2101010(p: u32) -> (f32, f32, f32, f32) {
    (
        (p & 0x3FF) as f32 / 1023.0,
        ((p >> 10) & 0x3FF) as f32 / 1023.0,
        ((p >> 20) & 0x3FF) as f32 / 1023.0,
        (p >> 30) as f32 / 3.0,
    )
}

/// Translation of `ARGB8888_FROM_RGBA()`.
#[inline(always)]
pub(crate) fn argb8888_from_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    (a << 24) | (r << 16) | (g << 8) | b
}
/// Translation of `RGBA8888_FROM_RGBA()`.
#[inline(always)]
pub(crate) fn rgba8888_from_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    (r << 24) | (g << 16) | (b << 8) | a
}
/// Translation of `ABGR8888_FROM_RGBA()`.
#[inline(always)]
pub(crate) fn abgr8888_from_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    (a << 24) | (b << 16) | (g << 8) | r
}
/// Translation of `BGRA8888_FROM_RGBA()`.
#[inline(always)]
pub(crate) fn bgra8888_from_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    (b << 24) | (g << 16) | (r << 8) | a
}
/// Translation of `XRGB8888_FROM_RGB()`.
#[inline(always)]
pub(crate) fn xrgb8888_from_rgb(r: u32, g: u32, b: u32) -> u32 {
    (r << 16) | (g << 8) | b
}
/// Translation of `RGB332_FROM_RGB()`.
#[inline(always)]
pub(crate) fn rgb332_from_rgb(r: u32, g: u32, b: u32) -> u32 {
    (((r >> 5) << 5) | ((g >> 5) << 2) | (b >> 6)) as u8 as u32
}
/// Translation of `RGB565_FROM_RGB()`.
#[inline(always)]
pub(crate) fn rgb565_from_rgb(r: u32, g: u32, b: u32) -> u32 {
    (((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)) as u16 as u32
}
/// Translation of `RGB555_FROM_RGB()`.
#[inline(always)]
pub(crate) fn rgb555_from_rgb(r: u32, g: u32, b: u32) -> u32 {
    (((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3)) as u16 as u32
}
/// Translation of `RGB_FROM_RGB565()`.
#[inline(always)]
pub(crate) fn rgb_from_rgb565(p: u32) -> (u32, u32, u32) {
    (
        EXPAND_BYTE[5][((p & 0xF800) >> 11) as usize] as u32,
        EXPAND_BYTE[6][((p & 0x07E0) >> 5) as usize] as u32,
        EXPAND_BYTE[5][(p & 0x001F) as usize] as u32,
    )
}
/// Translation of `RGB_FROM_RGB555()`.
#[inline(always)]
pub(crate) fn rgb_from_rgb555(p: u32) -> (u32, u32, u32) {
    (
        EXPAND_BYTE[5][((p & 0x7C00) >> 10) as usize] as u32,
        EXPAND_BYTE[5][((p & 0x03E0) >> 5) as usize] as u32,
        EXPAND_BYTE[5][(p & 0x001F) as usize] as u32,
    )
}
/// Translation of `RGB_FROM_XRGB8888()`.
#[inline(always)]
pub(crate) fn rgb_from_xrgb8888(p: u32) -> (u32, u32, u32) {
    ((p & 0xFF0000) >> 16, (p & 0xFF00) >> 8, p & 0xFF)
}

/// Translation of `ARGB2101010_FROM_RGBA()`.
#[inline(always)]
pub(crate) fn argb2101010_from_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    let r = if r != 0 { (r << 2) | 0x3 } else { 0 };
    let g = if g != 0 { (g << 2) | 0x3 } else { 0 };
    let b = if b != 0 { (b << 2) | 0x3 } else { 0 };
    let a = (a * 3) / 255;
    (a << 30) | (r << 20) | (g << 10) | b
}
/// Translation of `ABGR2101010_FROM_RGBA()`.
#[inline(always)]
pub(crate) fn abgr2101010_from_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    let r = if r != 0 { (r << 2) | 0x3 } else { 0 };
    let g = if g != 0 { (g << 2) | 0x3 } else { 0 };
    let b = if b != 0 { (b << 2) | 0x3 } else { 0 };
    let a = (a * 3) / 255;
    (a << 30) | (b << 20) | (g << 10) | r
}
/// Translation of `ARGB2101010_FROM_RGBAFLOAT()`.
#[inline(always)]
pub(crate) fn argb2101010_from_rgbafloat(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let r = r.clamp(0.0, 1.0) * 1023.0;
    let g = g.clamp(0.0, 1.0) * 1023.0;
    let b = b.clamp(0.0, 1.0) * 1023.0;
    let a = a.clamp(0.0, 1.0) * 3.0;
    ((a.round() as u32) << 30)
        | ((r.round() as u32) << 20)
        | ((g.round() as u32) << 10)
        | b.round() as u32
}
/// Translation of `ABGR2101010_FROM_RGBAFLOAT()`.
#[inline(always)]
pub(crate) fn abgr2101010_from_rgbafloat(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let r = r.clamp(0.0, 1.0) * 1023.0;
    let g = g.clamp(0.0, 1.0) * 1023.0;
    let b = b.clamp(0.0, 1.0) * 1023.0;
    let a = a.clamp(0.0, 1.0) * 3.0;
    ((a.round() as u32) << 30)
        | ((b.round() as u32) << 20)
        | ((g.round() as u32) << 10)
        | r.round() as u32
}

/// Convert any 32-bit 4-bpp pixel to ARGB format. Translation of `PIXEL_TO_ARGB_PIXEL()`.
#[inline(always)]
pub(crate) fn pixel_to_argb_pixel(src: u32, srcfmt: &PixelFormatDetails) -> u32 {
    let (r, g, b, a) = rgba_from_pixel(src, srcfmt);
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Blend a single color channel or alpha value. Translation of `ALPHA_BLEND_CHANNEL()`.
/* dC = ((sC * sA) + (dC * (255 - sA))) / 255 */
#[inline(always)]
pub(crate) fn alpha_blend_channel(sc: u32, dc: u32, sa: u32) -> u32 {
    let mut x =
        (sc.wrapping_sub(dc).wrapping_mul(sa)).wrapping_add((dc << 8).wrapping_sub(dc)) as u16;
    x = x.wrapping_add(0x1);
    x = x.wrapping_add(x >> 8);
    (x >> 8) as u32
}

/// Perform a division by 255 after a multiplication of two 8-bit color
/// channels. Translation of `MULT_DIV_255()`.
/* out = (sC * dC) / 255 */
#[inline(always)]
pub(crate) fn mult_div_255(sc: u32, dc: u32) -> u32 {
    let mut x = sc.wrapping_mul(dc) as u16;
    x = x.wrapping_add(0x1);
    x = x.wrapping_add(x >> 8);
    (x >> 8) as u32
}

/// Blend the RGB values of two pixels with an alpha value. Translation of `ALPHA_BLEND_RGB()`.
#[inline(always)]
pub(crate) fn alpha_blend_rgb(
    sr: u32,
    sg: u32,
    sb: u32,
    a: u32,
    d: (u32, u32, u32),
) -> (u32, u32, u32) {
    (
        alpha_blend_channel(sr, d.0, a),
        alpha_blend_channel(sg, d.1, a),
        alpha_blend_channel(sb, d.2, a),
    )
}

/// Blend the RGBA values of two pixels. Translation of `ALPHA_BLEND_RGBA()`.
#[inline(always)]
pub(crate) fn alpha_blend_rgba(
    s: (u32, u32, u32, u32),
    d: (u32, u32, u32, u32),
) -> (u32, u32, u32, u32) {
    (
        alpha_blend_channel(s.0, d.0, s.3),
        alpha_blend_channel(s.1, d.1, s.3),
        alpha_blend_channel(s.2, d.2, s.3),
        alpha_blend_channel(255, d.3, s.3),
    )
}

/// Blend two 8888 pixels with the same format.
/// Calculates dst = ((src * factor) + (dst * (255 - factor))) / 255.
/// Translation of the 64-bit `FACTOR_BLEND_8888()`.
#[inline(always)]
pub(crate) fn factor_blend_8888(src: u32, dst: u32, factor: u32) -> u32 {
    let mut src64 = src as u64;
    src64 = (src64 | (src64 << 24)) & 0x00FF00FF00FF00FF;

    let mut dst64 = dst as u64;
    dst64 = (dst64 | (dst64 << 24)) & 0x00FF00FF00FF00FF;

    dst64 = (src64.wrapping_sub(dst64).wrapping_mul(factor as u64))
        .wrapping_add(dst64 << 8)
        .wrapping_sub(dst64);
    dst64 = dst64.wrapping_add(0x0001000100010001);
    dst64 = dst64.wrapping_add((dst64 >> 8) & 0x00FF00FF00FF00FF);
    dst64 &= 0xFF00FF00FF00FF00;

    ((dst64 >> 8) | (dst64 >> 32)) as u32
}

/// Alpha blend two 8888 pixels with the same formats. Translation of `ALPHA_BLEND_8888()`.
#[inline(always)]
pub(crate) fn alpha_blend_8888(src: u32, dst: u32, fmt: &PixelFormatDetails) -> u32 {
    let src_a = (src >> fmt.Ashift) & 0xFF;
    let tmp = src | fmt.Amask;
    factor_blend_8888(tmp, dst, src_a)
}

/// Alpha blend two 8888 pixels with differing formats. Translation of `ALPHA_BLEND_SWIZZLE_8888()`.
#[inline(always)]
pub(crate) fn alpha_blend_swizzle_8888(
    src: u32,
    dst: u32,
    srcfmt: &PixelFormatDetails,
    dstfmt: &PixelFormatDetails,
) -> u32 {
    let src_a = (src >> srcfmt.Ashift) & 0xFF;
    let tmp = (((src >> srcfmt.Rshift) & 0xFF) << dstfmt.Rshift)
        | (((src >> srcfmt.Gshift) & 0xFF) << dstfmt.Gshift)
        | (((src >> srcfmt.Bshift) & 0xFF) << dstfmt.Bshift)
        | dstfmt.Amask;
    factor_blend_8888(tmp, dst, src_a)
}
