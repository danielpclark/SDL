// Rust translation of the blit-mapping functions of src/video/SDL_pixels.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use std::sync::Arc;

use super::{assemble_rgba, calculate_blit};
use crate::error::{Error, Result};
use crate::video::pixels::{Palette, PaletteMap, PixelFormatDetails};
use crate::video::surface::{read_palette, Surface};

/// Map from Palette to Palette. Translation of `Map1to1()`: `None` means the
/// palettes are identical.
fn map_1to1(src: &Palette, dst: &Palette) -> Option<Vec<u8>> {
    if src.is_prefix_of(dst) {
        return None;
    }

    let mut map = vec![0u8; 256];
    for (i, c) in src.colors().iter().enumerate() {
        map[i] = dst.find_color(*c);
    }
    Some(map)
}

/// Map from Palette to BitField. Translation of `Map1toN()`.
fn map_1ton(
    pal: Option<&Palette>,
    rmod: u8,
    gmod: u8,
    bmod: u8,
    amod: u8,
    dst: &PixelFormatDetails,
) -> Result<Vec<u8>> {
    let Some(pal) = pal else {
        return Err(Error::new("src does not have a palette set"));
    };

    let dst_bytes = dst.format.bytes_per_pixel() as usize;
    let bpp = if dst_bytes == 3 { 4 } else { dst_bytes };
    let mut map = vec![0u8; 256 * bpp];

    // We memory copy to the pixel map so the endianness is preserved
    for (i, c) in pal.colors().iter().enumerate() {
        let r = (c.r as u32 * rmod as u32) / 255;
        let g = (c.g as u32 * gmod as u32) / 255;
        let b = (c.b as u32 * bmod as u32) / 255;
        let a = (c.a as u32 * amod as u32) / 255;
        assemble_rgba(&mut map, i * bpp, dst_bytes, dst, r, g, b, a);
    }
    Ok(map)
}

fn same_arc<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// Check that `src`'s blit map is still valid for blitting to `dst`, and
/// rebuild it if not. Translation of `SDL_ValidateMap()`.
pub(crate) fn validate_map(src: &mut Surface<'_>, dst: &Surface<'_>) -> Result<()> {
    let map = &src.map;
    let dst_version = dst.palette.as_ref().map(|p| read_palette(p).version());
    let src_version = src.palette.as_ref().map(|p| read_palette(p).version());

    if map.dst_fmt != Some(dst.format)
        || !same_arc(&map.dst_pal, &dst.palette)
        || dst_version.is_some_and(|v| map.dst_palette_version != v)
        || src_version.is_some_and(|v| map.src_palette_version != v)
    {
        map_surface(src, dst)?;
    }
    Ok(())
}

/// Build the blit map from `src` to `dst`. Translation of `SDL_MapSurface()`.
pub(crate) fn map_surface(src: &mut Surface<'_>, dst: &Surface<'_>) -> Result<()> {
    // Clear out any previous mapping
    if src.is_rle_encoded() {
        crate::video::rle::un_rle_surface(src);
    }
    src.map.invalidate();

    // Figure out what kind of mapping we're doing
    src.map.identity = false;
    let srcfmt = src.fmt;
    let dstfmt = dst.fmt;
    {
        let srcpal = src.palette.as_ref().map(|p| read_palette(p));
        let same = same_arc(&src.palette, &dst.palette) && src.palette.is_some();
        let dstpal_guard = if same {
            None
        } else {
            dst.palette.as_ref().map(|p| read_palette(p))
        };
        let srcpal_ref: Option<&Palette> = srcpal.as_deref();
        let dstpal_ref: Option<&Palette> = if same {
            srcpal_ref
        } else {
            dstpal_guard.as_deref()
        };

        if srcfmt.format.is_indexed() {
            if dstfmt.format.is_indexed() {
                // Palette --> Palette
                match (srcpal_ref, dstpal_ref) {
                    (Some(s), Some(d)) => match map_1to1(s, d) {
                        None => src.map.identity = true,
                        Some(table) => src.map.table = table,
                    },
                    _ => src.map.identity = true,
                }
                if srcfmt.bits_per_pixel != dstfmt.bits_per_pixel {
                    src.map.identity = false;
                }
            } else {
                // Palette --> BitField
                let m = &src.map;
                src.map.table = map_1ton(srcpal_ref, m.r, m.g, m.b, m.a, &dstfmt)?;
            }
        } else if dstfmt.format.is_indexed() {
            // BitField --> Palette
            src.map.palette_map = Some(PaletteMap::new());
        } else {
            // BitField --> BitField
            if srcfmt.format == dstfmt.format {
                src.map.identity = true;
            }
        }

        src.map.dst_palette_version = dstpal_ref.map_or(0, |p| p.version());
        src.map.src_palette_version = srcpal_ref.map_or(0, |p| p.version());
    }

    // Choose your blitters wisely
    calculate_blit(src, dst)
}
