// Rust translation of src/video/SDL_blit_A.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Functions to perform alpha blended blitting.
//!
//! Upstream's x86 kernels are selected the same way here (from the CPU
//! features, see [`simd_support`]) and computed in portable code:
//!
//! * `Blit565to565SurfaceAlphaMMX`/`Blit555to555SurfaceAlphaMMX` have their
//!   own per-channel arithmetic for groups of four pixels, emulated lane by
//!   lane;
//! * `Blit888to888SurfaceAlphaSSE2` blends groups of four pixels like
//!   `ALPHA_BLEND_CHANNEL` and finishes the row with a loop that stores only
//!   the first byte of each pixel (FIXME (upstream));
//! * the SSE4.1/AVX2 `Blit8888to8888PixelAlphaSwizzle` kernels produce the
//!   same bytes as the portable one (`(x * 257) >> 16` equals
//!   `(x + (x >> 8)) >> 8` for every value they see), so that one is used.
//!
//! The NEON, SVE2 and LSX kernels are not translated; other architectures
//! use the portable functions, as an upstream build without them would.

use super::*;
use crate::video::pixels::{Color, PackedLayout};

/// A palette entry (upstream dereferences `dst_pal->colors[*dst]`).
fn pal_rgb(pal: Option<&Palette>, index: u8) -> Color {
    pal.and_then(|p| p.colors().get(index as usize).copied())
        .unwrap_or(Color::new(0, 0, 0, 0))
}

/// Pack RGB into an 8-bit pixel through the palette map (or as RGB332).
fn pack_rgb332(palmap: &[u8], dr: u32, dg: u32, db: u32) -> u8 {
    let i = ((dr >> 5) << (3 + 2)) | ((dg >> 5) << 2) | (db >> 6);
    if palmap.is_empty() {
        i as u8
    } else {
        palmap[i as usize]
    }
}

/// Run `put(src, s, dst, d)` for every pixel, stepping `srcbpp`/`dstbpp`.
fn walk(
    info: &mut BlitInfo<'_>,
    srcbpp: usize,
    dstbpp: usize,
    mut put: impl FnMut(&[u8], usize, &mut [u8], usize),
) {
    let (width, height) = (info.dst_w as usize, info.dst_h as usize);
    let (srcskip, dstskip) = (info.src_skip as isize, info.dst_skip as isize);
    let (mut s, mut d) = (0isize, 0isize);
    for _ in 0..height {
        for _ in 0..width {
            put(info.src, s as usize, info.dst, d as usize);
            s += srcbpp as isize;
            d += dstbpp as isize;
        }
        s += srcskip;
        d += dstskip;
    }
}

// Functions to perform alpha blended blitting

/// N->1 blending with per-surface alpha. Translation of `BlitNto1SurfaceAlpha()`.
fn blit_n_to_1_surface_alpha(info: &mut BlitInfo<'_>) {
    let palmap = info.table;
    let srcfmt = *info.src_fmt;
    let dstpal = info.dst_pal;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let a = info.a as u32;
    walk(info, srcbpp, 1, |src, s, dst, d| {
        let (_, sr, sg, sb) = disemble_rgb(src, s, srcbpp, &srcfmt);
        let c = pal_rgb(dstpal, dst[d]);
        let (dr, dg, db) = alpha_blend_rgb(sr, sg, sb, a, (c.r as u32, c.g as u32, c.b as u32));
        // Pack RGB into 8bit pixel
        dst[d] = pack_rgb332(palmap, dr & 0xff, dg & 0xff, db & 0xff);
    });
}

/// N->1 blending with pixel alpha. Translation of `BlitNto1PixelAlpha()`.
fn blit_n_to_1_pixel_alpha(info: &mut BlitInfo<'_>) {
    let palmap = info.table;
    let srcfmt = *info.src_fmt;
    let dstpal = info.dst_pal;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    walk(info, srcbpp, 1, |src, s, dst, d| {
        let (_, sr, sg, sb, sa) = disemble_rgba(src, s, srcbpp, &srcfmt);
        let c = pal_rgb(dstpal, dst[d]);
        let (dr, dg, db) = alpha_blend_rgb(sr, sg, sb, sa, (c.r as u32, c.g as u32, c.b as u32));
        // Pack RGB into 8bit pixel
        dst[d] = pack_rgb332(palmap, dr & 0xff, dg & 0xff, db & 0xff);
    });
}

/// colorkeyed N->1 blending with per-surface alpha. Translation of `BlitNto1SurfaceAlphaKey()`.
fn blit_n_to_1_surface_alpha_key(info: &mut BlitInfo<'_>) {
    let palmap = info.table;
    let srcfmt = *info.src_fmt;
    let dstpal = info.dst_pal;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let ckey = info.colorkey;
    let a = info.a as u32;
    walk(info, srcbpp, 1, |src, s, dst, d| {
        let (pixel, sr, sg, sb) = disemble_rgb(src, s, srcbpp, &srcfmt);
        if pixel != ckey {
            let c = pal_rgb(dstpal, dst[d]);
            let (dr, dg, db) = alpha_blend_rgb(sr, sg, sb, a, (c.r as u32, c.g as u32, c.b as u32));
            // Pack RGB into 8bit pixel
            dst[d] = pack_rgb332(palmap, dr & 0xff, dg & 0xff, db & 0xff);
        }
    });
}

/// Translation of `Blit888to888SurfaceAlphaSSE2()`, computed lane by lane.
fn blit_888_to_888_surface_alpha_sse2(info: &mut BlitInfo<'_>) {
    let width = info.dst_w as usize;
    let height = info.dst_h as usize;
    let srcskip = info.src_skip as isize;
    let dstskip = info.dst_skip as isize;
    let alpha = info.a as u32;
    let (mut s, mut d) = (0isize, 0isize);
    for _ in 0..height {
        let mut i = 0;
        while i + 4 <= width {
            // dst = ((src - dst) * srcA) + ((dst << 8) - dst), += 1, (dst + (dst >> 8)) >> 8
            for k in 0..16 {
                let (sb, db) = (
                    info.src[s as usize + k] as u32,
                    info.dst[d as usize + k] as u32,
                );
                info.dst[d as usize + k] = alpha_blend_channel(sb, db, alpha) as u8;
            }
            // Set the alpha channels of dst to 255
            for p in 0..4 {
                let v = rd32(info.dst, d as usize + 4 * p) | 0xff000000;
                wr32(info.dst, d as usize + 4 * p, v);
            }
            s += 16;
            d += 16;
            i += 4;
        }
        while i < width {
            let src32 = rd32(info.src, s as usize);
            let dst32 = rd32(info.dst, d as usize);
            let dst32 = factor_blend_8888(src32, dst32, alpha);
            // FIXME (upstream): `*dst` is a Uint8 pointer here, so only the
            // first byte of the pixel is stored.
            info.dst[d as usize] = (dst32 | 0xff000000) as u8;
            s += 4;
            d += 4;
            i += 1;
        }
        s += srcskip;
        d += dstskip;
    }
}

/// fast RGB888->(A)RGB888 blending with surface alpha=128 special case.
/// Translation of `BlitRGBtoRGBSurfaceAlpha128()`.
fn blit_rgb_to_rgb_surface_alpha128(info: &mut BlitInfo<'_>) {
    info.truncate_skips(4, 4);
    walk(info, 4, 4, |src, si, dst, di| {
        let s = rd32(src, si);
        let d = rd32(dst, di);
        let v = ((((s & 0x00fefefe) + (d & 0x00fefefe)) >> 1) + (s & d & 0x00010101)) | 0xff000000;
        wr32(dst, di, v);
    });
}

/// fast RGB888->(A)RGB888 blending with surface alpha. Translation of `BlitRGBtoRGBSurfaceAlpha()`.
fn blit_rgb_to_rgb_surface_alpha(info: &mut BlitInfo<'_>) {
    info.truncate_skips(4, 4);
    let alpha = info.a as u32;
    if alpha == 128 {
        blit_rgb_to_rgb_surface_alpha128(info);
    } else {
        walk(info, 4, 4, |src, si, dst, di| {
            let s = rd32(src, si);
            let d = rd32(dst, di);
            let d = factor_blend_8888(s, d, alpha);
            wr32(dst, di, d | 0xff000000);
        });
    }
}

// 16bpp special case for per-surface alpha=50%: blend 2 pixels in parallel

/// blend a single 16 bit pixel at 50%. Translation of `BLEND16_50()`.
fn blend16_50(d: u32, s: u32, mask: u32) -> u32 {
    (((s & mask) + (d & mask)) >> 1) + (s & d & (!mask & 0xffff))
}

/// Translation of `Blit16to16SurfaceAlpha128()`.
///
/// Upstream blends two pixels at a time with `BLEND2x16_50`, choosing
/// between an aligned and a pipelined loop by the addresses of the rows.
/// With masks that clear the low bit of every channel (0xf7de, 0xfbde) the
/// paired form gives the same value for each pixel as `BLEND16_50`, so
/// every pixel is blended singly here.
fn blit_16_to_16_surface_alpha128(info: &mut BlitInfo<'_>, mask: u16) {
    info.truncate_skips(2, 2);
    let mask = mask as u32;
    walk(info, 2, 2, |src, si, dst, di| {
        let s = rd16(src, si);
        let d = rd16(dst, di);
        wr16(dst, di, blend16_50(d, s, mask));
    });
}

/// `d += (s - d) * alpha >> 5` on the G0RAB packing of a 16-bit pixel.
fn blend_packed16(s: u32, d: u32, alpha: u32, packmask: u32) -> u32 {
    /*
     * shift out the middle component (green) to
     * the high 16 bits, and process all three RGB
     * components at the same time.
     */
    let s = (s | s << 16) & packmask;
    let mut d = (d | d << 16) & packmask;
    d = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(alpha) >> 5);
    d &= packmask;
    (d | d >> 16) & 0xffff
}

/// fast RGB565->RGB565 blending with surface alpha. Translation of `Blit565to565SurfaceAlpha()`.
fn blit_565_to_565_surface_alpha(info: &mut BlitInfo<'_>) {
    info.truncate_skips(2, 2);
    let alpha = info.a as u32;
    if alpha == 128 {
        blit_16_to_16_surface_alpha128(info, 0xf7de);
    } else {
        let alpha = alpha >> 3; // downscale alpha to 5 bits
        walk(info, 2, 2, |src, si, dst, di| {
            let v = blend_packed16(rd16(src, si), rd16(dst, di), alpha, 0x07e0f81f);
            wr16(dst, di, v);
        });
    }
}

/// fast RGB555->RGB555 blending with surface alpha. Translation of `Blit555to555SurfaceAlpha()`.
fn blit_555_to_555_surface_alpha(info: &mut BlitInfo<'_>) {
    info.truncate_skips(2, 2);
    let alpha = info.a as u32; // downscale alpha to 5 bits
    if alpha == 128 {
        blit_16_to_16_surface_alpha128(info, 0xfbde);
    } else {
        let alpha = alpha >> 3; // downscale alpha to 5 bits
        walk(info, 2, 2, |src, si, dst, di| {
            let v = blend_packed16(rd16(src, si), rd16(dst, di), alpha, 0x03e07c1f);
            wr16(dst, di, v);
        });
    }
}

/// One 16-bit lane of `_mm_mullo_pi16`.
fn mullo16(a: u16, b: u16) -> u16 {
    a.wrapping_mul(b)
}
/// One 16-bit lane of `_mm_mulhi_pi16` (signed).
fn mulhi16(a: u16, b: u16) -> u16 {
    (((a as i16 as i32) * (b as i16 as i32)) >> 16) as u16
}

/// The four-pixel MMX kernel of `Blit565to565SurfaceAlphaMMX()`, one lane.
fn mmx_565_lane(s: u16, d: u16, mm_alpha: u16) -> u16 {
    // red
    let src2 = s >> 11;
    let dst2 = d >> 11;
    let src2 = src2.wrapping_sub(dst2); // src - dst -> src2
    let src2 = mullo16(src2, mm_alpha); /* src2 * alpha -> src2 */
    let src2 = src2 >> 11; // src2 >> 11 -> src2
    let dst2 = src2.wrapping_add(dst2); // src2 + dst2 -> dst2
    let mut mm_res = dst2 << 11; // RED -> mm_res
                                 // green -- process the bits in place
    let src2 = s & 0x07E0;
    let dst2 = d & 0x07E0;
    let src2 = src2.wrapping_sub(dst2); // src - dst -> src2
    let src2 = mulhi16(src2, mm_alpha); /* src2 * alpha -> src2 */
    let src2 = src2 << 5; // src2 << 5 -> src2
    let dst2 = src2.wrapping_add(dst2); // src2 + dst2 -> dst2
    mm_res |= dst2; // RED | GREEN -> mm_res
                    // blue
    let src2 = s & 0x001F;
    let dst2 = d & 0x001F;
    let src2 = src2.wrapping_sub(dst2); // src - dst -> src2
    let src2 = mullo16(src2, mm_alpha); /* src2 * alpha -> src2 */
    let src2 = src2 >> 11; // src2 >> 11 -> src2
    let dst2 = src2.wrapping_add(dst2) & 0x001F; // src2 + dst2 -> dst2, & MASKBLUE
    mm_res | dst2 // RED | GREEN | BLUE -> mm_res
}

/// The four-pixel MMX kernel of `Blit555to555SurfaceAlphaMMX()`, one lane.
fn mmx_555_lane(s: u16, d: u16, mm_alpha: u16) -> u16 {
    // red -- process the bits in place
    let src2 = s & 0x7C00;
    let dst2 = d & 0x7C00;
    let src2 = src2.wrapping_sub(dst2); // src - dst -> src2
    let src2 = mulhi16(src2, mm_alpha); /* src2 * alpha -> src2 */
    let src2 = src2 << 5; // src2 << 5 -> src2
    let dst2 = src2.wrapping_add(dst2) & 0x7C00; // src2 + dst2 -> dst2, & MASKRED
    let mut mm_res = dst2; // RED -> mm_res
                           // green -- process the bits in place
    let src2 = s & 0x03E0;
    let dst2 = d & 0x03E0;
    let src2 = src2.wrapping_sub(dst2); // src - dst -> src2
    let src2 = mulhi16(src2, mm_alpha); /* src2 * alpha -> src2 */
    let src2 = src2 << 5; // src2 << 5 -> src2
    let dst2 = src2.wrapping_add(dst2); // src2 + dst2 -> dst2
    mm_res |= dst2; // RED | GREEN -> mm_res
                    // blue
    let src2 = s & 0x001F;
    let dst2 = d & 0x001F;
    let src2 = src2.wrapping_sub(dst2); // src - dst -> src2
    let src2 = mullo16(src2, mm_alpha); /* src2 * alpha -> src2 */
    let src2 = src2 >> 11; // src2 >> 11 -> src2
    let dst2 = src2.wrapping_add(dst2) & 0x001F; // src2 + dst2 -> dst2, & MASKBLUE
    mm_res | dst2 // RED | GREEN | BLUE -> mm_res
}

/// Translation of `Blit565to565SurfaceAlphaMMX()` and
/// `Blit555to555SurfaceAlphaMMX()`: `DUFFS_LOOP_124` blends the first
/// `width & 1` and `width & 2` pixels of each row with the scalar formula
/// and the rest in groups of four with the MMX kernel.
fn blit_16_to_16_surface_alpha_mmx(info: &mut BlitInfo<'_>, is_565: bool) {
    info.truncate_skips(2, 2);
    let alpha = info.a as u32;
    if alpha == 128 {
        blit_16_to_16_surface_alpha128(info, if is_565 { 0xf7de } else { 0xfbde });
        return;
    }
    let packmask = if is_565 { 0x07e0f81f } else { 0x03e07c1f };
    let alpha = alpha & !(1 + 2 + 4); // cut alpha to get the exact same behaviour
    let mm_alpha = (alpha << 3) as u16;
    let alpha = alpha >> 3; // downscale alpha to 5 bits

    let width = info.dst_w as usize;
    let height = info.dst_h as usize;
    let srcskip = info.src_skip as isize;
    let dstskip = info.dst_skip as isize;
    let scalar_pixels = (width & 1) + (width & 2);
    let (mut s, mut d) = (0isize, 0isize);
    for _ in 0..height {
        for x in 0..width {
            let sv = rd16(info.src, s as usize);
            let dv = rd16(info.dst, d as usize);
            let v = if x < scalar_pixels {
                blend_packed16(sv, dv, alpha, packmask)
            } else if is_565 {
                mmx_565_lane(sv as u16, dv as u16, mm_alpha) as u32
            } else {
                mmx_555_lane(sv as u16, dv as u16, mm_alpha) as u32
            };
            wr16(info.dst, d as usize, v);
            s += 2;
            d += 2;
        }
        s += srcskip;
        d += dstskip;
    }
}

fn blit_565_to_565_surface_alpha_mmx(info: &mut BlitInfo<'_>) {
    blit_16_to_16_surface_alpha_mmx(info, true);
}

fn blit_555_to_555_surface_alpha_mmx(info: &mut BlitInfo<'_>) {
    blit_16_to_16_surface_alpha_mmx(info, false);
}

/// fast ARGB8888->RGB565 blending with pixel alpha. Translation of `BlitARGBto565PixelAlpha()`.
fn blit_argb_to_565_pixel_alpha(info: &mut BlitInfo<'_>) {
    info.truncate_skips(4, 2);
    walk(info, 4, 2, |src, si, dst, di| {
        let mut s = rd32(src, si);
        let alpha = s >> 27; // downscale alpha to 5 bits
                             /* Here we special-case opaque alpha since the
                             compositioning used (>>8 instead of /255) doesn't handle
                             it correctly. */
        if alpha != 0 {
            if alpha == (255 >> 3) {
                wr16(
                    dst,
                    di,
                    (s >> 8 & 0xf800) + (s >> 5 & 0x7e0) + (s >> 3 & 0x1f),
                );
            } else {
                let mut d = rd16(dst, di);
                /*
                 * convert source and destination to G0RAB65565
                 * and blend all components at the same time
                 */
                s = ((s & 0xfc00) << 11) + (s >> 8 & 0xf800) + (s >> 3 & 0x1f);
                d = (d | d << 16) & 0x07e0f81f;
                d = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(alpha) >> 5);
                d &= 0x07e0f81f;
                wr16(dst, di, d | d >> 16);
            }
        }
    });
}

/// fast ARGB8888->RGB555 blending with pixel alpha. Translation of `BlitARGBto555PixelAlpha()`.
fn blit_argb_to_555_pixel_alpha(info: &mut BlitInfo<'_>) {
    info.truncate_skips(4, 2);
    walk(info, 4, 2, |src, si, dst, di| {
        let mut s = rd32(src, si);
        let alpha = s >> 27; // downscale alpha to 5 bits
                             /* Here we special-case opaque alpha since the
                             compositioning used (>>8 instead of /255) doesn't handle
                             it correctly. */
        if alpha != 0 {
            if alpha == (255 >> 3) {
                wr16(
                    dst,
                    di,
                    (s >> 9 & 0x7c00) + (s >> 6 & 0x3e0) + (s >> 3 & 0x1f),
                );
            } else {
                let mut d = rd16(dst, di);
                /*
                 * convert source and destination to G0RAB55555
                 * and blend all components at the same time
                 */
                s = ((s & 0xf800) << 10) + (s >> 9 & 0x7c00) + (s >> 3 & 0x1f);
                d = (d | d << 16) & 0x03e07c1f;
                d = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(alpha) >> 5);
                d &= 0x03e07c1f;
                wr16(dst, di, d | d >> 16);
            }
        }
    });
}

/// General (slow) N->N blending with per-surface alpha. Translation of `BlitNtoNSurfaceAlpha()`.
fn blit_n_to_n_surface_alpha(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    let s_a = info.a as u32;
    if s_a != 0 {
        walk(info, srcbpp, dstbpp, |src, s, dst, d| {
            let (_, sr, sg, sb) = disemble_rgb(src, s, srcbpp, &srcfmt);
            let (_, dr, dg, db, da) = disemble_rgba(dst, d, dstbpp, &dstfmt);
            let (dr, dg, db, da) = alpha_blend_rgba((sr, sg, sb, s_a), (dr, dg, db, da));
            assemble_rgba(dst, d, dstbpp, &dstfmt, dr, dg, db, da);
        });
    }
}

/// General (slow) colorkeyed N->N blending with per-surface alpha.
/// Translation of `BlitNtoNSurfaceAlphaKey()`.
fn blit_n_to_n_surface_alpha_key(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    let ckey = info.colorkey;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    let s_a = info.a as u32;
    walk(info, srcbpp, dstbpp, |src, s, dst, d| {
        let pixel = retrieve_rgb_pixel(src, s, srcbpp);
        if s_a != 0 && pixel != ckey {
            let (sr, sg, sb) = rgb_from_pixel(pixel, &srcfmt);
            let (_, dr, dg, db, da) = disemble_rgba(dst, d, dstbpp, &dstfmt);
            let (dr, dg, db, da) = alpha_blend_rgba((sr, sg, sb, s_a), (dr, dg, db, da));
            assemble_rgba(dst, d, dstbpp, &dstfmt, dr, dg, db, da);
        }
    });
}

/// Fast 32-bit RGBA->RGBA blending with pixel alpha. Translation of `Blit8888to8888PixelAlpha()`.
fn blit_8888_to_8888_pixel_alpha(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    walk(info, 4, 4, |src, s, dst, d| {
        let v = alpha_blend_8888(rd32(src, s), rd32(dst, d), &srcfmt);
        wr32(dst, d, v);
    });
}

/// Fast 32-bit RGBA->RGB(A) blending with pixel alpha and src swizzling.
/// Translation of `Blit8888to8888PixelAlphaSwizzle()` (and of the SSE4.1
/// and AVX2 versions, which compute the same bytes).
fn blit_8888_to_8888_pixel_alpha_swizzle(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    let fill_alpha = dstfmt.Amask == 0;
    let (dst_amask, _dst_ashift) = dstfmt.alpha_mask_and_shift_8888();
    walk(info, 4, 4, |src, s, dst, d| {
        let mut dst32 = alpha_blend_swizzle_8888(rd32(src, s), rd32(dst, d), &srcfmt, &dstfmt);
        if fill_alpha {
            dst32 |= dst_amask;
        }
        wr32(dst, d, dst32);
    });
}

/// Translation of `BlitNtoNPixelAlpha()`.
fn blit_n_to_n_pixel_alpha(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    // Set up some basic variables
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    walk(info, srcbpp, dstbpp, |src, s, dst, d| {
        let (_, sr, sg, sb, sa) = disemble_rgba(src, s, srcbpp, &srcfmt);
        if sa != 0 {
            let (_, dr, dg, db, da) = disemble_rgba(dst, d, dstbpp, &dstfmt);
            let (dr, dg, db, da) = alpha_blend_rgba((sr, sg, sb, sa), (dr, dg, db, da));
            assemble_rgba(dst, d, dstbpp, &dstfmt, dr, dg, db, da);
        }
    });
}

/// Translation of `SDL_CalculateBlitA()`.
pub(crate) fn calculate_blit_a(surface: &Surface<'_>, dst: &Surface<'_>) -> Option<NamedBlit> {
    let sf = &surface.fmt;
    let df = &dst.fmt;
    let has_dst_pal = surface.map.dst_pal.is_some();
    let simd = simd_support();

    match surface.map.flags & !COPY_RLE_MASK {
        COPY_BLEND => {
            // Per-pixel alpha blits
            match df.bytes_per_pixel {
                1 => Some(if has_dst_pal {
                    named_blit!(blit_n_to_1_pixel_alpha, "BlitNto1PixelAlpha")
                } else {
                    // RGB332 has no palette !
                    named_blit!(blit_n_to_n_pixel_alpha, "BlitNtoNPixelAlpha")
                }),
                2 => {
                    if sf.bytes_per_pixel == 4
                        && sf.Amask == 0xff000000
                        && sf.Gmask == 0xff00
                        && ((sf.Rmask == 0xff && df.Rmask == 0x1f)
                            || (sf.Bmask == 0xff && df.Bmask == 0x1f))
                    {
                        if df.Gmask == 0x7e0 {
                            return Some(named_blit!(
                                blit_argb_to_565_pixel_alpha,
                                "BlitARGBto565PixelAlpha"
                            ));
                        } else if df.Gmask == 0x3e0 && df.Amask == 0 {
                            return Some(named_blit!(
                                blit_argb_to_555_pixel_alpha,
                                "BlitARGBto555PixelAlpha"
                            ));
                        }
                    }
                    Some(named_blit!(blit_n_to_n_pixel_alpha, "BlitNtoNPixelAlpha"))
                }
                4 => {
                    if sf.format.pixel_layout() == PackedLayout::L8888
                        && sf.Amask != 0
                        && df.format.pixel_layout() == PackedLayout::L8888
                    {
                        if simd.avx2 {
                            return Some(named_blit!(
                                blit_8888_to_8888_pixel_alpha_swizzle,
                                "Blit8888to8888PixelAlphaSwizzleAVX2"
                            ));
                        }
                        if simd.sse41 {
                            return Some(named_blit!(
                                blit_8888_to_8888_pixel_alpha_swizzle,
                                "Blit8888to8888PixelAlphaSwizzleSSE41"
                            ));
                        }
                        return Some(if sf.format == df.format {
                            named_blit!(blit_8888_to_8888_pixel_alpha, "Blit8888to8888PixelAlpha")
                        } else {
                            named_blit!(
                                blit_8888_to_8888_pixel_alpha_swizzle,
                                "Blit8888to8888PixelAlphaSwizzle"
                            )
                        });
                    }
                    Some(named_blit!(blit_n_to_n_pixel_alpha, "BlitNtoNPixelAlpha"))
                }
                _ => Some(named_blit!(blit_n_to_n_pixel_alpha, "BlitNtoNPixelAlpha")),
            }
        }
        x if x == COPY_MODULATE_ALPHA | COPY_BLEND => {
            if sf.Amask != 0 {
                return None;
            }
            // Per-surface alpha blits
            Some(match df.bytes_per_pixel {
                1 => {
                    if has_dst_pal {
                        named_blit!(blit_n_to_1_surface_alpha, "BlitNto1SurfaceAlpha")
                    } else {
                        // RGB332 has no palette !
                        named_blit!(blit_n_to_n_surface_alpha, "BlitNtoNSurfaceAlpha")
                    }
                }
                2 => {
                    if surface.map.identity {
                        if df.Gmask == 0x7e0 {
                            return Some(if simd.mmx {
                                named_blit!(
                                    blit_565_to_565_surface_alpha_mmx,
                                    "Blit565to565SurfaceAlphaMMX"
                                )
                            } else {
                                named_blit!(
                                    blit_565_to_565_surface_alpha,
                                    "Blit565to565SurfaceAlpha"
                                )
                            });
                        } else if df.Gmask == 0x3e0 {
                            return Some(if simd.mmx {
                                named_blit!(
                                    blit_555_to_555_surface_alpha_mmx,
                                    "Blit555to555SurfaceAlphaMMX"
                                )
                            } else {
                                named_blit!(
                                    blit_555_to_555_surface_alpha,
                                    "Blit555to555SurfaceAlpha"
                                )
                            });
                        }
                    }
                    named_blit!(blit_n_to_n_surface_alpha, "BlitNtoNSurfaceAlpha")
                }
                4 => {
                    if sf.Rmask == df.Rmask
                        && sf.Gmask == df.Gmask
                        && sf.Bmask == df.Bmask
                        && sf.bytes_per_pixel == 4
                    {
                        if sf.Rshift.is_multiple_of(8)
                            && sf.Gshift.is_multiple_of(8)
                            && sf.Bshift.is_multiple_of(8)
                            && simd.sse2
                        {
                            return Some(named_blit!(
                                blit_888_to_888_surface_alpha_sse2,
                                "Blit888to888SurfaceAlphaSSE2"
                            ));
                        }
                        if (sf.Rmask | sf.Gmask | sf.Bmask) == 0xffffff {
                            return Some(named_blit!(
                                blit_rgb_to_rgb_surface_alpha,
                                "BlitRGBtoRGBSurfaceAlpha"
                            ));
                        }
                    }
                    named_blit!(blit_n_to_n_surface_alpha, "BlitNtoNSurfaceAlpha")
                }
                _ => named_blit!(blit_n_to_n_surface_alpha, "BlitNtoNSurfaceAlpha"),
            })
        }
        x if x == COPY_COLORKEY | COPY_MODULATE_ALPHA | COPY_BLEND => {
            if sf.Amask != 0 {
                return None;
            }
            Some(if df.bytes_per_pixel == 1 && has_dst_pal {
                named_blit!(blit_n_to_1_surface_alpha_key, "BlitNto1SurfaceAlphaKey")
            } else {
                // RGB332 has no palette !
                named_blit!(blit_n_to_n_surface_alpha_key, "BlitNtoNSurfaceAlphaKey")
            })
        }
        _ => None,
    }
}
