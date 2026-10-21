// Rust translation of src/video/SDL_blit_1.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Functions to blit from 8-bit surfaces to other surfaces.

use super::*;
use crate::video::pixels::Color;

/// Run `put(dst, dst_index, src_byte)` for every pixel of an 8-bit source.
fn walk_bytes(info: &mut BlitInfo<'_>, dst_step: usize, mut put: impl FnMut(&mut [u8], usize, u8)) {
    let width = info.dst_w as usize;
    let height = info.dst_h as usize;
    let srcskip = info.src_skip as isize;
    let dstskip = info.dst_skip as isize;
    let (mut s, mut d) = (0isize, 0isize);
    for _ in 0..height {
        for _ in 0..width {
            put(info.dst, d as usize, info.src[s as usize]);
            s += 1;
            d += dst_step as isize;
        }
        s += srcskip;
        d += dstskip;
    }
}

/// `map[*src]`; upstream always has a table here.
fn map1(map: &[u8], v: u8) -> u8 {
    map.get(v as usize).copied().unwrap_or(v)
}

/// Translation of `Blit1to1()`.
fn blit_1to1(info: &mut BlitInfo<'_>) {
    let map = info.table;
    walk_bytes(info, 1, |dst, d, s| dst[d] = map1(map, s));
}

/// Translation of `Blit1to2()`.
fn blit_1to2(info: &mut BlitInfo<'_>) {
    let map = info.table;
    walk_bytes(info, 2, |dst, d, s| wr16(dst, d, rd16(map, s as usize * 2)));
}

/// Translation of `Blit1to3()`.
fn blit_1to3(info: &mut BlitInfo<'_>) {
    let map = info.table;
    walk_bytes(info, 3, |dst, d, s| {
        let o = s as usize * 4;
        dst[d..d + 3].copy_from_slice(&map[o..o + 3]);
    });
}

/// Translation of `Blit1to4()`.
fn blit_1to4(info: &mut BlitInfo<'_>) {
    let map = info.table;
    walk_bytes(info, 4, |dst, d, s| wr32(dst, d, rd32(map, s as usize * 4)));
}

/// Translation of `Blit1to1Key()`.
fn blit_1to1_key(info: &mut BlitInfo<'_>) {
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bytes(info, 1, |dst, d, s| {
        if s as u32 != ckey {
            dst[d] = if palmap.is_empty() {
                s
            } else {
                palmap[s as usize]
            };
        }
    });
}

/// Translation of `Blit1to2Key()`.
fn blit_1to2_key(info: &mut BlitInfo<'_>) {
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bytes(info, 2, |dst, d, s| {
        if s as u32 != ckey {
            wr16(dst, d, rd16(palmap, s as usize * 2));
        }
    });
}

/// Translation of `Blit1to3Key()`.
fn blit_1to3_key(info: &mut BlitInfo<'_>) {
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bytes(info, 3, |dst, d, s| {
        if s as u32 != ckey {
            let o = s as usize * 4;
            dst[d..d + 3].copy_from_slice(&palmap[o..o + 3]);
        }
    });
}

/// Translation of `Blit1to4Key()`.
fn blit_1to4_key(info: &mut BlitInfo<'_>) {
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bytes(info, 4, |dst, d, s| {
        if s as u32 != ckey {
            wr32(dst, d, rd32(palmap, s as usize * 4));
        }
    });
}

/// Translation of `Blit1toNAlpha()` and `Blit1toNAlphaKey()`.
fn blit_1to_n_alpha_impl(info: &mut BlitInfo<'_>, keyed: bool) {
    let dstfmt = *info.dst_fmt;
    let srcpal = info.src_pal;
    let ckey = info.colorkey;
    let a = info.a as u32;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    walk_bytes(info, dstbpp, |dst, d, s| {
        if !keyed || s as u32 != ckey {
            let c = srcpal
                .and_then(|p| p.colors().get(s as usize).copied())
                .unwrap_or(Color::new(0, 0, 0, 0));
            let s_a = (c.a as u32 * a) / 255;
            let (_, dr, dg, db, da) = disemble_rgba(dst, d, dstbpp, &dstfmt);
            let (dr, dg, db, da) =
                alpha_blend_rgba((c.r as u32, c.g as u32, c.b as u32, s_a), (dr, dg, db, da));
            assemble_rgba(dst, d, dstbpp, &dstfmt, dr, dg, db, da);
        }
    });
}

fn blit_1to_n_alpha(info: &mut BlitInfo<'_>) {
    blit_1to_n_alpha_impl(info, false);
}

fn blit_1to_n_alpha_key(info: &mut BlitInfo<'_>) {
    blit_1to_n_alpha_impl(info, true);
}

const ONE_BLIT: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_1to1, "Blit1to1")),
    Some(named_blit!(blit_1to2, "Blit1to2")),
    Some(named_blit!(blit_1to3, "Blit1to3")),
    Some(named_blit!(blit_1to4, "Blit1to4")),
];

const ONE_BLITKEY: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_1to1_key, "Blit1to1Key")),
    Some(named_blit!(blit_1to2_key, "Blit1to2Key")),
    Some(named_blit!(blit_1to3_key, "Blit1to3Key")),
    Some(named_blit!(blit_1to4_key, "Blit1to4Key")),
];

/// Translation of `SDL_CalculateBlit1()`.
pub(crate) fn calculate_blit1(surface: &Surface<'_>, dst_format: PixelFormat) -> Option<NamedBlit> {
    let which = if dst_format.bits_per_pixel() < 8 {
        0
    } else {
        dst_format.bytes_per_pixel() as usize
    };

    let alpha = Some(named_blit!(blit_1to_n_alpha, "Blit1toNAlpha"));
    let alpha_key = Some(named_blit!(blit_1to_n_alpha_key, "Blit1toNAlphaKey"));
    match surface.map.flags & !COPY_RLE_MASK {
        0 => ONE_BLIT.get(which).copied().flatten(),
        COPY_COLORKEY => ONE_BLITKEY.get(which).copied().flatten(),
        // this is not super-robust but handles a specific case we found sdl12-compat.
        x if x == COPY_COLORKEY | COPY_BLEND => {
            if surface.map.a == 255 {
                ONE_BLITKEY.get(which).copied().flatten()
            } else if which >= 2 {
                alpha_key
            } else {
                None
            }
        }
        /* Supporting 8bpp->8bpp alpha is doable but requires lots of
        tables which consume space and takes time to precompute,
        so is better left to the user */
        x if x == COPY_BLEND || x == COPY_MODULATE_ALPHA | COPY_BLEND => {
            if which >= 2 {
                alpha
            } else {
                None
            }
        }
        x if x == COPY_COLORKEY | COPY_MODULATE_ALPHA | COPY_BLEND => {
            if which >= 2 {
                alpha_key
            } else {
                None
            }
        }
        _ => None,
    }
}
