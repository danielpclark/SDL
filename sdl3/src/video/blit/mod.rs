// Rust translation of src/video/SDL_blit.c and SDL_blit.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The software blitter: the blit map cached on each source surface, the
//! per-call [`BlitInfo`], the pixel packing helpers of `SDL_blit.h`, and the
//! dispatch that picks a blit function for a pair of surfaces.
//!
//! Upstream keeps `SDL_BlitInfo` inside the source surface's `SDL_BlitMap`
//! and rewrites its pointers on every blit. Here the persistent part (flags,
//! colorkey, modulation, lookup tables, the chosen function) is
//! [`BlitMap`], and each blit builds a short-lived [`BlitInfo`] that borrows
//! the two pixel buffers. Blit functions index into those slices where the
//! C code advances pointers.
//!
//! The `DUFFS_LOOP*` macros are plain loops here. (Upstream's unrolled
//! versions run the body 4 or 8 times for a width of 0; blits never reach
//! the inner loops with an empty rectangle.)

pub(crate) mod copy;
pub(crate) mod map;
pub(crate) mod slow;

use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::video::pixels::{Colorspace, Palette, PaletteMap, PixelFormat, PixelFormatDetails, EXPAND_BYTE};
use crate::video::rect::Rect;
use crate::video::surface::{SharedPalette, Surface};

// SDL blit copy flags
pub(crate) const COPY_MODULATE_COLOR: u32 = 0x00000001;
pub(crate) const COPY_MODULATE_ALPHA: u32 = 0x00000002;
pub(crate) const COPY_MODULATE_MASK: u32 = COPY_MODULATE_COLOR | COPY_MODULATE_ALPHA;
pub(crate) const COPY_BLEND: u32 = 0x00000010;
pub(crate) const COPY_BLEND_PREMULTIPLIED: u32 = 0x00000020;
pub(crate) const COPY_ADD: u32 = 0x00000040;
pub(crate) const COPY_ADD_PREMULTIPLIED: u32 = 0x00000080;
pub(crate) const COPY_MOD: u32 = 0x00000100;
pub(crate) const COPY_MUL: u32 = 0x00000200;
pub(crate) const COPY_BLEND_MASK: u32 = COPY_BLEND
    | COPY_BLEND_PREMULTIPLIED
    | COPY_ADD
    | COPY_ADD_PREMULTIPLIED
    | COPY_MOD
    | COPY_MUL;
pub(crate) const COPY_COLORKEY: u32 = 0x00000400;
pub(crate) const COPY_NEAREST: u32 = 0x00000800;
pub(crate) const COPY_RLE_DESIRED: u32 = 0x00001000;
pub(crate) const COPY_RLE_COLORKEY: u32 = 0x00002000;
pub(crate) const COPY_RLE_ALPHAKEY: u32 = 0x00004000;
#[allow(dead_code)]
pub(crate) const COPY_RLE_MASK: u32 = COPY_RLE_DESIRED | COPY_RLE_COLORKEY | COPY_RLE_ALPHAKEY;

/// The per-blit state handed to a blit function. Translation of `SDL_BlitInfo`
/// (minus the persistent fields, which live in [`BlitMap`]).
///
/// `src` starts at the first source pixel of the blit rectangle and `dst` at
/// the first destination pixel; both run to the end of their surface's
/// pixel buffer.
pub(crate) struct BlitInfo<'a> {
    pub src: &'a [u8],
    pub src_w: i32,
    pub src_h: i32,
    pub src_pitch: i32,
    pub src_skip: i32,
    pub leading_skip: i32,
    pub dst: &'a mut [u8],
    pub dst_w: i32,
    pub dst_h: i32,
    pub dst_pitch: i32,
    pub dst_skip: i32,
    pub src_fmt: &'a PixelFormatDetails,
    pub src_pal: Option<&'a Palette>,
    pub dst_fmt: &'a PixelFormatDetails,
    pub dst_pal: Option<&'a Palette>,
    pub table: &'a [u8],
    pub palette_map: &'a mut PaletteMap,
    pub flags: u32,
    pub colorkey: u32,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
    // What SDL_Blit_Slow_Float() reads through info->src_surface / dst_surface.
    pub src_colorspace: Colorspace,
    pub dst_colorspace: Colorspace,
    pub src_props: Option<&'a Properties>,
    pub dst_props: Option<&'a Properties>,
}

/// Translation of `SDL_BlitFunc`.
pub(crate) type BlitFunc = fn(&mut BlitInfo<'_>);

/// A blit function and the upstream name it translates (used to report
/// which routine was chosen, and to recognize the float blitter, which
/// needs the destination's property group).
#[derive(Clone, Copy)]
pub(crate) struct NamedBlit {
    pub func: BlitFunc,
    pub name: &'static str,
}

impl std::fmt::Debug for NamedBlit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

macro_rules! named_blit {
    ($f:path, $name:literal) => {
        $crate::video::blit::NamedBlit {
            func: $f,
            name: $name,
        }
    };
}
pub(crate) use named_blit;

/// Which top-level blit `SDL_BlitMap::blit` points at.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum MapBlit {
    /// No valid mapping.
    #[default]
    None,
    /// `SDL_SoftBlit` with the given row function (`map->data`).
    Soft(NamedBlit),
}

/// Blit mapping definition. Translation of `SDL_BlitMap` together with the
/// persistent fields of its `SDL_BlitInfo` (flags, colorkey, modulation,
/// lookup tables, source and destination formats and palettes).
#[derive(Debug)]
pub(crate) struct BlitMap {
    pub identity: bool,
    pub blit: MapBlit,
    pub flags: u32,
    pub colorkey: u32,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
    /// `info.table`: palette → palette (`Uint8[256]`) or palette → pixel
    /// (`256 * bpp` bytes) translation; empty when there is none.
    pub table: Vec<u8>,
    /// `info.palette_map`: memoized RGBA → index lookups (pixels → palette).
    pub palette_map: Option<PaletteMap>,
    /// `info.dst_fmt` (`None` = invalidated).
    pub dst_fmt: Option<PixelFormat>,
    /// `info.dst_pal`; a strong reference so the identity check can't be
    /// fooled by a freed and reallocated palette.
    pub dst_pal: Option<SharedPalette>,
    /// The version count matches the destination; mismatch indicates an
    /// invalid mapping.
    pub dst_palette_version: u32,
    pub src_palette_version: u32,
}

impl Default for BlitMap {
    fn default() -> BlitMap {
        BlitMap {
            identity: false,
            blit: MapBlit::None,
            flags: 0,
            colorkey: 0,
            // Allocate an empty mapping
            r: 0xFF,
            g: 0xFF,
            b: 0xFF,
            a: 0xFF,
            table: Vec::new(),
            palette_map: None,
            dst_fmt: None,
            dst_pal: None,
            dst_palette_version: 0,
            src_palette_version: 0,
        }
    }
}

impl BlitMap {
    /// Translation of `SDL_InvalidateMap()`.
    pub fn invalidate(&mut self) {
        self.dst_fmt = None;
        self.dst_pal = None;
        self.src_palette_version = 0;
        self.dst_palette_version = 0;
        self.table = Vec::new();
        self.palette_map = None;
    }
}

// ---------------------------------------------------------------------------
// Useful macros for blitting routines (SDL_blit.h), as inline functions.
// Pixels are read and written in native byte order, like the C pointer casts.
// ---------------------------------------------------------------------------

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
pub(crate) fn disemble_rgb(b: &[u8], i: usize, bpp: usize, fmt: &PixelFormatDetails) -> (u32, u32, u32, u32) {
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

/// `x >> (8 - bits)` for a channel of `bits` width; C relies on the value
/// being below 256 when `bits` is 0.
#[inline(always)]
fn narrow(v: u32, bits: u8) -> u32 {
    v >> (8 - bits as u32)
}

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
pub(crate) fn assemble_rgb(buf: &mut [u8], i: usize, bpp: usize, fmt: &PixelFormatDetails, r: u32, g: u32, b: u32) {
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
    ((a.round() as u32) << 30) | ((r.round() as u32) << 20) | ((g.round() as u32) << 10) | b.round() as u32
}
/// Translation of `ABGR2101010_FROM_RGBAFLOAT()`.
#[inline(always)]
pub(crate) fn abgr2101010_from_rgbafloat(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let r = r.clamp(0.0, 1.0) * 1023.0;
    let g = g.clamp(0.0, 1.0) * 1023.0;
    let b = b.clamp(0.0, 1.0) * 1023.0;
    let a = a.clamp(0.0, 1.0) * 3.0;
    ((a.round() as u32) << 30) | ((b.round() as u32) << 20) | ((g.round() as u32) << 10) | r.round() as u32
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
    let mut x = (sc.wrapping_sub(dc).wrapping_mul(sa)).wrapping_add((dc << 8).wrapping_sub(dc)) as u16;
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
pub(crate) fn alpha_blend_rgb(sr: u32, sg: u32, sb: u32, a: u32, d: (u32, u32, u32)) -> (u32, u32, u32) {
    (
        alpha_blend_channel(sr, d.0, a),
        alpha_blend_channel(sg, d.1, a),
        alpha_blend_channel(sb, d.2, a),
    )
}

/// Blend the RGBA values of two pixels. Translation of `ALPHA_BLEND_RGBA()`.
#[inline(always)]
pub(crate) fn alpha_blend_rgba(s: (u32, u32, u32, u32), d: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
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

// ---------------------------------------------------------------------------
// SDL_blit.c
// ---------------------------------------------------------------------------

/// The general purpose software blit routine. Translation of `SDL_SoftBlit()`.
pub(crate) fn soft_blit(
    src: &mut Surface<'_>,
    srcrect: &Rect,
    dst: &mut Surface<'_>,
    dstrect: &Rect,
    run_blit: NamedBlit,
) -> Result<()> {
    // Everything is okay at the beginning...
    let mut okay: Result<()> = Ok(());

    // Lock the destination if it's in hardware
    let mut dst_locked = false;
    if dst.must_lock() {
        match dst.lock_raw() {
            Ok(()) => dst_locked = true,
            Err(e) => okay = Err(e),
        }
    }
    // Lock the source if it's in hardware
    let mut src_locked = false;
    if src.must_lock() {
        match src.lock_raw() {
            Ok(()) => src_locked = true,
            Err(e) => okay = Err(e),
        }
    }

    // Set up source and destination buffer pointers, and BLIT!
    if okay.is_ok() {
        if run_blit.name == "SDL_Blit_Slow_Float" {
            // SDL_GetSurfaceProperties(info->dst_surface) creates the group
            // when the float blitter stores the headroom.
            dst.properties();
        }
        run_soft_blit(src, srcrect, dst, dstrect, run_blit.func);
    }

    // We need to unlock the surfaces if they're locked
    if dst_locked {
        dst.unlock_raw();
    }
    if src_locked {
        src.unlock_raw();
    }
    // Blit is done!
    okay
}

fn run_soft_blit(src: &mut Surface<'_>, srcrect: &Rect, dst: &mut Surface<'_>, dstrect: &Rect, func: BlitFunc) {
    let src_fmt = src.fmt;
    let dst_fmt = dst.fmt;

    // Set up the blit information
    let (src_offset, leading_skip) = if src_fmt.bits_per_pixel >= 8 {
        (
            srcrect.y as isize * src.pitch as isize + srcrect.x as isize * src_fmt.bytes_per_pixel as isize,
            0,
        )
    } else {
        (
            srcrect.y as isize * src.pitch as isize
                + (srcrect.x as isize * src_fmt.bits_per_pixel as isize) / 8,
            ((srcrect.x * src_fmt.bits_per_pixel as i32) % 8) / src_fmt.bits_per_pixel as i32,
        )
    };
    let dst_offset =
        dstrect.y as isize * dst.pitch as isize + dstrect.x as isize * dst_fmt.bytes_per_pixel as isize;

    // Palettes are read-locked for the duration of the blit; a palette shared
    // by both surfaces is locked once.
    let src_pal_guard = src.palette.as_ref().map(|p| crate::video::surface::read_palette(p));
    let same_palette = match (&src.palette, &dst.palette) {
        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
        _ => false,
    };
    let dst_pal_guard = if same_palette {
        None
    } else {
        dst.palette.as_ref().map(|p| crate::video::surface::read_palette(p))
    };
    let src_pal: Option<&Palette> = src_pal_guard.as_deref();
    let dst_pal: Option<&Palette> = if same_palette { src_pal } else { dst_pal_guard.as_deref() };

    let map = &mut src.map;
    let palette_map = map.palette_map.get_or_insert_with(PaletteMap::new);
    let src_pixels = src.pixels.bytes().unwrap_or(&[]);
    let dst_pixels = dst.pixels.bytes_mut().unwrap_or(&mut []);
    let mut info = BlitInfo {
        src: &src_pixels[src_offset as usize..],
        src_w: srcrect.w,
        src_h: srcrect.h,
        src_pitch: src.pitch,
        src_skip: src.pitch - srcrect.w * src_fmt.bytes_per_pixel as i32,
        leading_skip,
        dst: &mut dst_pixels[dst_offset as usize..],
        dst_w: dstrect.w,
        dst_h: dstrect.h,
        dst_pitch: dst.pitch,
        dst_skip: dst.pitch - dstrect.w * dst_fmt.bytes_per_pixel as i32,
        src_fmt: &src_fmt,
        src_pal,
        dst_fmt: &dst_fmt,
        dst_pal,
        table: &map.table,
        palette_map,
        flags: map.flags,
        colorkey: map.colorkey,
        r: map.r,
        g: map.g,
        b: map.b,
        a: map.a,
        src_colorspace: src.colorspace,
        dst_colorspace: dst.colorspace,
        src_props: src.props.as_ref(),
        dst_props: dst.props.as_ref(),
    };

    // Run the actual software blit
    func(&mut info);
}

/// Translation of `SDL_ChooseBlitFunc()`, over a generated table.
#[allow(dead_code)]
pub(crate) fn choose_blit_func(
    src_format: PixelFormat,
    dst_format: PixelFormat,
    flags: u32,
    entries: &[BlitFuncEntry],
) -> Option<NamedBlit> {
    let flagcheck = flags & (COPY_MODULATE_MASK | COPY_BLEND_MASK | COPY_COLORKEY | COPY_NEAREST);

    for entry in entries {
        // Check for matching pixel formats
        if src_format != entry.src_format {
            continue;
        }
        if dst_format != entry.dst_format {
            continue;
        }

        // Check flags
        if (flagcheck & entry.flags) != flagcheck {
            continue;
        }

        // We found the best one!
        return Some(entry.func);
    }
    None
}

/// Translation of `SDL_BlitFuncEntry`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlitFuncEntry {
    pub src_format: PixelFormat,
    pub dst_format: PixelFormat,
    pub flags: u32,
    pub func: NamedBlit,
}

/// Figure out which of many blit routines to set up on a surface.
/// Translation of `SDL_CalculateBlit()`.
pub(crate) fn calculate_blit(surface: &mut Surface<'_>, dst: &Surface<'_>) -> Result<()> {
    let mut blit: Option<NamedBlit> = None;
    let src_colorspace = surface.colorspace;
    let dst_colorspace = dst.colorspace;

    // We don't currently support blitting to < 8 bpp surfaces
    if dst.format.bits_per_pixel() < 8 {
        surface.map.invalidate();
        return Err(Error::new("Blit combination not supported"));
    }

    // We should have cleared out RLE at this point
    crate::sdl_assert!(!surface.is_rle_encoded());

    surface.map.blit = MapBlit::None;
    surface.map.dst_fmt = Some(dst.format);
    surface.map.dst_pal = dst.palette.clone();

    // Choose a standard blit function
    if blit.is_none()
        && (src_colorspace != dst_colorspace
            || surface.format.bytes_per_pixel() > 4
            || dst.format.bytes_per_pixel() > 4)
    {
        blit = Some(named_blit!(slow::blit_slow_float, "SDL_Blit_Slow_Float"));
    }
    if blit.is_none() {
        if surface.map.identity && (surface.map.flags & !COPY_RLE_DESIRED) == 0 {
            blit = Some(named_blit!(copy::blit_copy, "SDL_BlitCopy"));
        } else if surface.format.is_10bit() || dst.format.is_10bit() {
            blit = Some(named_blit!(slow::blit_slow, "SDL_Blit_Slow"));
        }
    }

    if blit.is_none() {
        let src_format = surface.format;
        let dst_format = dst.format;

        if (!src_format.is_indexed() || (src_format == PixelFormat::INDEX8 && surface.palette.is_some()))
            && !src_format.is_fourcc()
            && (!dst_format.is_indexed() || (dst_format == PixelFormat::INDEX8 && dst.palette.is_some()))
            && !dst_format.is_fourcc()
        {
            blit = Some(named_blit!(slow::blit_slow, "SDL_Blit_Slow"));
        }
    }

    // Make sure we have a blit function
    match blit {
        Some(b) => {
            surface.map.blit = MapBlit::Soft(b);
            Ok(())
        }
        None => {
            surface.map.invalidate();
            Err(Error::new("Blit combination not supported"))
        }
    }
}
