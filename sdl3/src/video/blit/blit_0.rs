// Rust translation of src/video/SDL_blit_0.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Functions to blit from bitmaps (1, 2 and 4 bits per pixel) to other surfaces.
//!
//! Upstream writes each of `BlitBto1` ... `BlitBto4Key` out in full, twice
//! (for the two bit orders); they share one bit-walking loop here, with the
//! per-pixel store passed in.

use super::*;
use crate::video::pixels::{BitmapOrder, Color, PixelType};

/// The bit-walking loop shared by all of upstream's `BlitBto*` functions:
/// calls `put(dst, dst_index, bit)` for every pixel of the blit rectangle.
///
/// Upstream computes the row skip from `width + leading_skip` pixels, so
/// when the rectangle starts inside a byte (`leading_skip > 0`) every row
/// starts `leading_skip` bytes further on than it should (and the last rows
/// read past the rectangle). Fixed here by taking `leading_skip` back off
/// the skip.
fn walk_bits(
    info: &mut BlitInfo<'_>,
    srcbpp: u32,
    dst_step: usize,
    mut put: impl FnMut(&mut [u8], usize, u32),
) {
    let mask: u32 = (1 << srcbpp) - 1;
    let align: i32 = (8 / srcbpp as i32) - 1;

    // Set up some basic variables
    let mut width = info.dst_w;
    let height = info.dst_h;
    let mut srcskip = info.src_skip as isize;
    let dstskip = info.dst_skip as isize;

    width += info.leading_skip;
    // (not upstream: the leading pixels are part of the bytes read below,
    // not of the `src_w` pixels `src_skip` was computed for)
    srcskip -= info.leading_skip as isize;

    if srcbpp == 4 {
        srcskip += (width - (width + 1) / 2) as isize;
    } else if srcbpp == 2 {
        srcskip += (width - (width + 3) / 4) as isize;
    } else if srcbpp == 1 {
        srcskip += (width - (width + 7) / 8) as isize;
    }

    let order_4321 = info.src_fmt.format.pixel_order() == BitmapOrder::Order4321 as u32;
    let src = info.src;
    let read = |i: isize| -> u8 {
        if i < 0 {
            0
        } else {
            src.get(i as usize).copied().unwrap_or(0)
        }
    };
    let mut s: isize = 0;
    let mut d: isize = 0;
    for _ in 0..height {
        let mut byte: u8 = 0;
        let mut c = 0;
        while c < info.leading_skip {
            if c & align == 0 {
                byte = read(s);
                s += 1;
            }
            byte = if order_4321 {
                byte >> srcbpp
            } else {
                byte << srcbpp
            };
            c += 1;
        }
        while c < width {
            if c & align == 0 {
                byte = read(s);
                s += 1;
            }
            let bit = if order_4321 {
                byte as u32 & mask
            } else {
                (byte as u32 >> (8 - srcbpp)) & mask
            };
            put(info.dst, d as usize, bit);
            d += dst_step as isize;
            byte = if order_4321 {
                byte >> srcbpp
            } else {
                byte << srcbpp
            };
            c += 1;
        }
        s += srcskip;
        d += dstskip;
    }
}

/// Translation of `BlitBto1()`.
fn blit_bto1(info: &mut BlitInfo<'_>, srcbpp: u32) {
    let map = info.table;
    walk_bits(info, srcbpp, 1, |dst, d, bit| {
        dst[d] = if map.is_empty() {
            bit as u8
        } else {
            map[bit as usize]
        };
    });
}

/// Translation of `BlitBto2()`.
fn blit_bto2(info: &mut BlitInfo<'_>, srcbpp: u32) {
    info.truncate_skips(1, 2);
    let map = info.table;
    walk_bits(info, srcbpp, 2, |dst, d, bit| {
        wr16(dst, d, rd16(map, bit as usize * 2));
    });
}

/// Translation of `BlitBto3()`.
fn blit_bto3(info: &mut BlitInfo<'_>, srcbpp: u32) {
    let map = info.table;
    walk_bits(info, srcbpp, 3, |dst, d, bit| {
        let o = bit as usize * 4;
        dst[d..d + 3].copy_from_slice(&map[o..o + 3]);
    });
}

/// Translation of `BlitBto4()`.
fn blit_bto4(info: &mut BlitInfo<'_>, srcbpp: u32) {
    info.truncate_skips(1, 4);
    let map = info.table;
    walk_bits(info, srcbpp, 4, |dst, d, bit| {
        wr32(dst, d, rd32(map, bit as usize * 4));
    });
}

/// Translation of `BlitBto1Key()`.
fn blit_bto1_key(info: &mut BlitInfo<'_>, srcbpp: u32) {
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bits(info, srcbpp, 1, |dst, d, bit| {
        if bit != ckey {
            dst[d] = if palmap.is_empty() {
                bit as u8
            } else {
                palmap[bit as usize]
            };
        }
    });
}

/// Translation of `BlitBto2Key()`.
fn blit_bto2_key(info: &mut BlitInfo<'_>, srcbpp: u32) {
    info.truncate_skips(1, 2);
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bits(info, srcbpp, 2, |dst, d, bit| {
        if bit != ckey {
            wr16(dst, d, rd16(palmap, bit as usize * 2));
        }
    });
}

/// Translation of `BlitBto3Key()`.
fn blit_bto3_key(info: &mut BlitInfo<'_>, srcbpp: u32) {
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bits(info, srcbpp, 3, |dst, d, bit| {
        if bit != ckey {
            let o = bit as usize * 4;
            dst[d..d + 3].copy_from_slice(&palmap[o..o + 3]);
        }
    });
}

/// Translation of `BlitBto4Key()`.
fn blit_bto4_key(info: &mut BlitInfo<'_>, srcbpp: u32) {
    info.truncate_skips(1, 4);
    let (palmap, ckey) = (info.table, info.colorkey);
    walk_bits(info, srcbpp, 4, |dst, d, bit| {
        if bit != ckey {
            wr32(dst, d, rd32(palmap, bit as usize * 4));
        }
    });
}

fn palette_rgb(pal: Option<&Palette>, index: u32) -> Color {
    pal.and_then(|p| p.colors().get(index as usize).copied())
        .unwrap_or(Color::new(0, 0, 0, 0))
}

/// Translation of `BlitBtoNAlpha()` and `BlitBtoNAlphaKey()`.
///
/// FIXME (upstream): these take the source's *bytes* per pixel (1) as its
/// bits per pixel, so 2- and 4-bit sources are read as if they were 1-bit.
fn blit_bto_n_alpha_impl(info: &mut BlitInfo<'_>, keyed: bool) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    let srcpal = info.src_pal;
    let a = info.a as u32;
    let ckey = info.colorkey;
    let srcbpp = srcfmt.bytes_per_pixel as u32;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    walk_bits(info, srcbpp, dstbpp, |dst, d, bit| {
        if !keyed || bit != ckey {
            let c = palette_rgb(srcpal, bit);
            let (_, dr, dg, db, da) = disemble_rgba(dst, d, dstbpp, &dstfmt);
            let (dr, dg, db, da) =
                alpha_blend_rgba((c.r as u32, c.g as u32, c.b as u32, a), (dr, dg, db, da));
            assemble_rgba(dst, d, dstbpp, &dstfmt, dr, dg, db, da);
        }
    });
}

fn blit_bto_n_alpha(info: &mut BlitInfo<'_>) {
    blit_bto_n_alpha_impl(info, false);
}

fn blit_bto_n_alpha_key(info: &mut BlitInfo<'_>) {
    blit_bto_n_alpha_impl(info, true);
}

macro_rules! bitmap_blits {
    ($($name:ident = $f:ident($bits:literal), $cname:literal;)*) => {
        $( fn $name(info: &mut BlitInfo<'_>) { $f(info, $bits) } )*
    };
}

bitmap_blits! {
    blit_1bto1 = blit_bto1(1), "Blit1bto1";
    blit_1bto2 = blit_bto2(1), "Blit1bto2";
    blit_1bto3 = blit_bto3(1), "Blit1bto3";
    blit_1bto4 = blit_bto4(1), "Blit1bto4";
    blit_1bto1_key = blit_bto1_key(1), "Blit1bto1Key";
    blit_1bto2_key = blit_bto2_key(1), "Blit1bto2Key";
    blit_1bto3_key = blit_bto3_key(1), "Blit1bto3Key";
    blit_1bto4_key = blit_bto4_key(1), "Blit1bto4Key";
    blit_2bto1 = blit_bto1(2), "Blit2bto1";
    blit_2bto2 = blit_bto2(2), "Blit2bto2";
    blit_2bto3 = blit_bto3(2), "Blit2bto3";
    blit_2bto4 = blit_bto4(2), "Blit2bto4";
    blit_2bto1_key = blit_bto1_key(2), "Blit2bto1Key";
    blit_2bto2_key = blit_bto2_key(2), "Blit2bto2Key";
    blit_2bto3_key = blit_bto3_key(2), "Blit2bto3Key";
    blit_2bto4_key = blit_bto4_key(2), "Blit2bto4Key";
    blit_4bto1 = blit_bto1(4), "Blit4bto1";
    blit_4bto2 = blit_bto2(4), "Blit4bto2";
    blit_4bto3 = blit_bto3(4), "Blit4bto3";
    blit_4bto4 = blit_bto4(4), "Blit4bto4";
    blit_4bto1_key = blit_bto1_key(4), "Blit4bto1Key";
    blit_4bto2_key = blit_bto2_key(4), "Blit4bto2Key";
    blit_4bto3_key = blit_bto3_key(4), "Blit4bto3Key";
    blit_4bto4_key = blit_bto4_key(4), "Blit4bto4Key";
}

const BITMAP_BLIT_1B: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_1bto1, "Blit1bto1")),
    Some(named_blit!(blit_1bto2, "Blit1bto2")),
    Some(named_blit!(blit_1bto3, "Blit1bto3")),
    Some(named_blit!(blit_1bto4, "Blit1bto4")),
];
const COLORKEY_BLIT_1B: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_1bto1_key, "Blit1bto1Key")),
    Some(named_blit!(blit_1bto2_key, "Blit1bto2Key")),
    Some(named_blit!(blit_1bto3_key, "Blit1bto3Key")),
    Some(named_blit!(blit_1bto4_key, "Blit1bto4Key")),
];
const BITMAP_BLIT_2B: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_2bto1, "Blit2bto1")),
    Some(named_blit!(blit_2bto2, "Blit2bto2")),
    Some(named_blit!(blit_2bto3, "Blit2bto3")),
    Some(named_blit!(blit_2bto4, "Blit2bto4")),
];
const COLORKEY_BLIT_2B: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_2bto1_key, "Blit2bto1Key")),
    Some(named_blit!(blit_2bto2_key, "Blit2bto2Key")),
    Some(named_blit!(blit_2bto3_key, "Blit2bto3Key")),
    Some(named_blit!(blit_2bto4_key, "Blit2bto4Key")),
];
const BITMAP_BLIT_4B: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_4bto1, "Blit4bto1")),
    Some(named_blit!(blit_4bto2, "Blit4bto2")),
    Some(named_blit!(blit_4bto3, "Blit4bto3")),
    Some(named_blit!(blit_4bto4, "Blit4bto4")),
];
const COLORKEY_BLIT_4B: [Option<NamedBlit>; 5] = [
    None,
    Some(named_blit!(blit_4bto1_key, "Blit4bto1Key")),
    Some(named_blit!(blit_4bto2_key, "Blit4bto2Key")),
    Some(named_blit!(blit_4bto3_key, "Blit4bto3Key")),
    Some(named_blit!(blit_4bto4_key, "Blit4bto4Key")),
];

/// Translation of `SDL_CalculateBlit0()`.
pub(crate) fn calculate_blit0(surface: &Surface<'_>, dst_format: PixelFormat) -> Option<NamedBlit> {
    let which = if dst_format.bits_per_pixel() < 8 {
        0
    } else {
        dst_format.bytes_per_pixel() as usize
    };

    let (bitmap, colorkey) = match surface.format.pixel_type() {
        PixelType::Index1 => (&BITMAP_BLIT_1B, &COLORKEY_BLIT_1B),
        PixelType::Index2 => (&BITMAP_BLIT_2B, &COLORKEY_BLIT_2B),
        PixelType::Index4 => (&BITMAP_BLIT_4B, &COLORKEY_BLIT_4B),
        _ => return None,
    };
    match surface.map.flags & !COPY_RLE_MASK {
        0 => bitmap.get(which).copied().flatten(),
        COPY_COLORKEY => colorkey.get(which).copied().flatten(),
        x if x == COPY_MODULATE_ALPHA | COPY_BLEND => {
            (which >= 2).then_some(named_blit!(blit_bto_n_alpha, "BlitBtoNAlpha"))
        }
        x if x == COPY_COLORKEY | COPY_MODULATE_ALPHA | COPY_BLEND => {
            (which >= 2).then_some(named_blit!(blit_bto_n_alpha_key, "BlitBtoNAlphaKey"))
        }
        _ => None,
    }
}
