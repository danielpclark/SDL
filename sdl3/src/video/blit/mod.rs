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

pub(crate) mod auto;
pub(crate) mod blit_0;
pub(crate) mod blit_1;
pub(crate) mod blit_a;
pub(crate) mod blit_n;
pub(crate) mod copy;
mod macros;
pub(crate) mod map;
pub(crate) mod slow;

use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::video::pixels::{Colorspace, Palette, PaletteMap, PixelFormat, PixelFormatDetails};
use crate::video::rect::Rect;
use crate::video::surface::{SharedPalette, Surface};

pub(crate) use macros::*;

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
pub(crate) const COPY_BLEND_MASK: u32 =
    COPY_BLEND | COPY_BLEND_PREMULTIPLIED | COPY_ADD | COPY_ADD_PREMULTIPLIED | COPY_MOD | COPY_MUL;
pub(crate) const COPY_COLORKEY: u32 = 0x00000400;
pub(crate) const COPY_NEAREST: u32 = 0x00000800;
pub(crate) const COPY_RLE_DESIRED: u32 = 0x00001000;
pub(crate) const COPY_RLE_COLORKEY: u32 = 0x00002000;
pub(crate) const COPY_RLE_ALPHAKEY: u32 = 0x00004000;
pub(crate) const COPY_RLE_MASK: u32 = COPY_RLE_DESIRED | COPY_RLE_COLORKEY | COPY_RLE_ALPHAKEY;

/// The per-blit state handed to a blit function. Translation of `SDL_BlitInfo`
/// (minus the persistent fields, which live in [`BlitMap`]).
///
/// `src` starts at the first source pixel of the blit rectangle and `dst` at
/// the first destination pixel; both run to the end of their surface's
/// pixel buffer.
#[allow(dead_code)] // some fields are only read by the specialized blitters
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

/// The x86 vector extensions upstream's blitter selection looks at
/// (`SDL_HasMMX()`, `SDL_HasSSE()`, `SDL_HasSSE2()`, `SDL_HasSSE41()`, `SDL_HasAVX2()`).
/// Their kernels are computed in portable code; only the choice depends on
/// the CPU, as upstream's does. Other architectures report none of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct SimdSupport {
    pub mmx: bool,
    pub sse: bool,
    pub sse2: bool,
    pub sse41: bool,
    pub avx2: bool,
}

#[cfg(test)]
thread_local! {
    /// Tests pin the selection to compare with upstream built either way.
    pub(crate) static SIMD_OVERRIDE: std::cell::Cell<Option<SimdSupport>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn simd_support() -> SimdSupport {
    #[cfg(test)]
    if let Some(s) = SIMD_OVERRIDE.with(|o| o.get()) {
        return s;
    }
    if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
        SimdSupport {
            mmx: crate::cpuinfo::has_mmx(),
            sse: crate::cpuinfo::has_sse(),
            sse2: crate::cpuinfo::has_sse2(),
            sse41: crate::cpuinfo::has_sse41(),
            avx2: crate::cpuinfo::has_avx2(),
        }
    } else {
        SimdSupport::default()
    }
}

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

fn run_soft_blit(
    src: &mut Surface<'_>,
    srcrect: &Rect,
    dst: &mut Surface<'_>,
    dstrect: &Rect,
    func: BlitFunc,
) {
    let src_fmt = src.fmt;
    let dst_fmt = dst.fmt;

    // Set up the blit information
    let (src_offset, leading_skip) = if src_fmt.bits_per_pixel >= 8 {
        (
            srcrect.y as isize * src.pitch as isize
                + srcrect.x as isize * src_fmt.bytes_per_pixel as isize,
            0,
        )
    } else {
        (
            srcrect.y as isize * src.pitch as isize
                + (srcrect.x as isize * src_fmt.bits_per_pixel as isize) / 8,
            ((srcrect.x * src_fmt.bits_per_pixel as i32) % 8) / src_fmt.bits_per_pixel as i32,
        )
    };
    let dst_offset = dstrect.y as isize * dst.pitch as isize
        + dstrect.x as isize * dst_fmt.bytes_per_pixel as isize;

    // Palettes are read-locked for the duration of the blit; a palette shared
    // by both surfaces is locked once.
    let src_pal_guard = src
        .palette
        .as_ref()
        .map(|p| crate::video::surface::read_palette(p));
    let same_palette = match (&src.palette, &dst.palette) {
        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
        _ => false,
    };
    let dst_pal_guard = if same_palette {
        None
    } else {
        dst.palette
            .as_ref()
            .map(|p| crate::video::surface::read_palette(p))
    };
    let src_pal: Option<&Palette> = src_pal_guard.as_deref();
    let dst_pal: Option<&Palette> = if same_palette {
        src_pal
    } else {
        dst_pal_guard.as_deref()
    };

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
        } else if surface.format.bits_per_pixel() < 8 && surface.format.is_indexed() {
            blit = blit_0::calculate_blit0(surface, dst.format);
        } else if surface.format.bytes_per_pixel() == 1 && surface.format.is_indexed() {
            blit = blit_1::calculate_blit1(surface, dst.format);
        } else if surface.map.flags & COPY_BLEND != 0 {
            blit = blit_a::calculate_blit_a(surface, dst);
        } else {
            blit = blit_n::calculate_blit_n(surface, dst);
        }
    }
    if blit.is_none() {
        let src_format = surface.format;
        let dst_format = dst.format;

        blit = choose_blit_func(
            src_format,
            dst_format,
            surface.map.flags,
            auto::GENERATED_BLIT_FUNC_TABLE,
        );
    }

    if blit.is_none() {
        let src_format = surface.format;
        let dst_format = dst.format;

        if (!src_format.is_indexed()
            || (src_format == PixelFormat::INDEX8 && surface.palette.is_some()))
            && !src_format.is_fourcc()
            && (!dst_format.is_indexed()
                || (dst_format == PixelFormat::INDEX8 && dst.palette.is_some()))
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
