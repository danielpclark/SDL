// Rust translation of src/video/SDL_blit_N.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Functions to blit from N-bit surfaces to other surfaces.
//!
//! On x86, upstream picks SSE4.1/AVX2 kernels for 8888 <-> 8888 swizzles and
//! RGB565 -> 32-bit; both produce the same bytes as the portable code below
//! (the RGB565 lookup tables, the SSE4.1 multipliers and the scalar
//! expansion all compute `(v << 3) | (v >> 2)` and `(v << 2) | (v >> 4)`), so
//! only the choice of kernel is kept. That choice matters for the 8888
//! swizzle, which has no portable twin: without SSE4.1/AVX2 such blits fall
//! through to the generic tables and fill an X channel with 0 instead of
//! 0xFF. The AltiVec, NEON and SVE2 kernels are not translated.

use super::copy::blit_copy;
use super::*;
use crate::video::pixels::{narrow_channel, PackedLayout};

#[cfg(target_endian = "little")]
const HI: usize = 1;
#[cfg(target_endian = "little")]
const LO: usize = 0;
#[cfg(target_endian = "big")]
const HI: usize = 0;
#[cfg(target_endian = "big")]
const LO: usize = 1;

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

/// Special optimized blit for RGB 8-8-8 --> RGB 5-5-5. Translation of `RGB888_RGB555()`.
fn rgb888_rgb555(src: u32) -> u32 {
    (((src & 0x00F80000) >> 9) | ((src & 0x0000F800) >> 6) | ((src & 0x000000F8) >> 3)) & 0xffff
}

/// Translation of `Blit_XRGB8888_RGB555()`.
fn blit_xrgb8888_rgb555(info: &mut BlitInfo<'_>) {
    walk(info, 4, 2, |src, s, dst, d| {
        wr16(dst, d, rgb888_rgb555(rd32(src, s)))
    });
}

/// Special optimized blit for RGB 8-8-8 --> RGB 5-6-5. Translation of `RGB888_RGB565()`.
fn rgb888_rgb565(src: u32) -> u32 {
    (((src & 0x00F80000) >> 8) | ((src & 0x0000FC00) >> 5) | ((src & 0x000000F8) >> 3)) & 0xffff
}

/// Translation of `Blit_XRGB8888_RGB565()`.
fn blit_xrgb8888_rgb565(info: &mut BlitInfo<'_>) {
    walk(info, 4, 2, |src, s, dst, d| {
        wr16(dst, d, rgb888_rgb565(rd32(src, s)))
    });
}

// This is the code used to generate the lookup tables below (GENERATE_SHIFTS):
// the tables are computed at compile time here.

const fn calculate_argb(
    v: u32,
    s: [u32; 3],
    s_shift: [u32; 3],
    s_bits: [u32; 3],
    d_shift: [u32; 3],
    d_amask: u32,
) -> u32 {
    let mut out = d_amask;
    let mut k = 0;
    while k < 3 {
        let c = (v & s[k]) >> s_shift[k];
        let e = match s_bits[k] {
            5 => (c << 3) | (c >> 2),
            6 => (c << 2) | (c >> 4),
            _ => c,
        };
        out |= e << d_shift[k];
        k += 1;
    }
    out
}

/// Translation of `GenerateLUT()`: entry `2i` maps the low byte of an RGB565
/// pixel, `2i + 1` the high byte.
const fn generate_lut(d_shift: [u32; 3], d_amask: u32) -> [u32; 512] {
    let s = [0xF800, 0x07E0, 0x001F];
    let s_shift = [11, 5, 0];
    let s_bits = [5, 6, 5];
    let mut lut = [0u32; 512];
    let mut i = 0;
    while i < 256 {
        lut[i * 2] = calculate_argb(i as u32, s, s_shift, s_bits, d_shift, d_amask);
        lut[i * 2 + 1] = calculate_argb((i as u32) << 8, s, s_shift, s_bits, d_shift, d_amask);
        i += 1;
    }
    lut
}

// Special optimized blit for RGB565 -> ARGB8888, ABGR8888, RGBA8888, BGRA8888
static RGB565_ARGB8888_LUT: [u32; 512] = generate_lut([16, 8, 0], 0xff000000);
static RGB565_ABGR8888_LUT: [u32; 512] = generate_lut([0, 8, 16], 0xff000000);
static RGB565_RGBA8888_LUT: [u32; 512] = generate_lut([24, 16, 8], 0x000000ff);
static RGB565_BGRA8888_LUT: [u32; 512] = generate_lut([8, 16, 24], 0x000000ff);

/// Special optimized blit for RGB 5-6-5 --> 32-bit RGB surfaces.
/// Translation of `Blit_RGB565_32()`.
fn blit_rgb565_32(info: &mut BlitInfo<'_>, map: &[u32; 512]) {
    walk(info, 2, 4, |src, s, dst, d| {
        // RGB565_32(dst, src, map)
        let v = map[src[s + LO] as usize * 2] | map[src[s + HI] as usize * 2 + 1];
        wr32(dst, d, v);
    });
}

fn blit_rgb565_argb8888(info: &mut BlitInfo<'_>) {
    blit_rgb565_32(info, &RGB565_ARGB8888_LUT);
}
fn blit_rgb565_abgr8888(info: &mut BlitInfo<'_>) {
    blit_rgb565_32(info, &RGB565_ABGR8888_LUT);
}
fn blit_rgb565_rgba8888(info: &mut BlitInfo<'_>) {
    blit_rgb565_32(info, &RGB565_RGBA8888_LUT);
}
fn blit_rgb565_bgra8888(info: &mut BlitInfo<'_>) {
    blit_rgb565_32(info, &RGB565_BGRA8888_LUT);
}

/// Translation of `Blit_RGB565_32_SSE41()`, which works for any 8888
/// destination (the shuffle puts each channel at the destination's shift).
fn blit_rgb565_32_sse41(info: &mut BlitInfo<'_>) {
    let dstfmt = *info.dst_fmt;
    let (r_shift, g_shift, b_shift) = (
        dstfmt.Rshift as u32,
        dstfmt.Gshift as u32,
        dstfmt.Bshift as u32,
    );
    let (amask, _ashift) = dstfmt.alpha_mask_and_shift_8888();
    walk(info, 2, 4, |src, s, dst, d| {
        let (r, g, b) = rgb_from_rgb565(rd16(src, s));
        wr32(
            dst,
            d,
            (r << r_shift) | (g << g_shift) | (b << b_shift) | amask,
        );
    });
}

/// blits 16 bit RGB<->RGBA with both surfaces having the same R,G,B fields.
/// Translation of `Blit2to2MaskAlpha()`.
fn blit_2to2_mask_alpha(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    if dstfmt.Amask != 0 {
        // RGB->RGBA, SET_ALPHA
        let mask = ((narrow_channel(info.a as u32, dstfmt.Abits)) << dstfmt.Ashift) & 0xffff;
        walk(info, 2, 2, |src, s, dst, d| {
            wr16(dst, d, rd16(src, s) | mask)
        });
    } else {
        // RGBA->RGB, NO_ALPHA
        let mask = (srcfmt.Rmask | srcfmt.Gmask | srcfmt.Bmask) & 0xffff;
        walk(info, 2, 2, |src, s, dst, d| {
            wr16(dst, d, rd16(src, s) & mask)
        });
    }
}

/// blits 32 bit RGB<->RGBA with both surfaces having the same R,G,B fields.
/// Translation of `Blit4to4MaskAlpha()`.
fn blit_4to4_mask_alpha(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    if dstfmt.Amask != 0 {
        // RGB->RGBA, SET_ALPHA
        let mask = narrow_channel(info.a as u32, dstfmt.Abits) << dstfmt.Ashift;
        walk(info, 4, 4, |src, s, dst, d| {
            wr32(dst, d, rd32(src, s) | mask)
        });
    } else {
        // RGBA->RGB, NO_ALPHA
        let mask = srcfmt.Rmask | srcfmt.Gmask | srcfmt.Bmask;
        walk(info, 4, 4, |src, s, dst, d| {
            wr32(dst, d, rd32(src, s) & mask)
        });
    }
}

/// permutation for mapping srcfmt to dstfmt, overloading or not the alpha
/// channel. Translation of `get_permutation()`: `([p0, p1, p2, p3], alpha_channel)`.
fn get_permutation(
    srcfmt: &PixelFormatDetails,
    dstfmt: &PixelFormatDetails,
) -> ([usize; 4], usize) {
    let mut alpha_channel = 0;
    let mut pixel: u32 = if cfg!(target_endian = "little") {
        0x04030201 // identity permutation
    } else {
        0x01020304 // identity permutation
    };

    let (p0, p1, p2, p3) = if srcfmt.Amask != 0 {
        rgba_from_pixel(pixel, srcfmt)
    } else {
        let (r, g, b) = rgb_from_pixel(pixel, srcfmt);
        (r, g, b, 0)
    };

    if dstfmt.Amask != 0 {
        if srcfmt.Amask != 0 {
            pixel = pixel_from_rgba(dstfmt, p0, p1, p2, p3);
        } else {
            pixel = pixel_from_rgba(dstfmt, p0, p1, p2, 0);
        }
    } else {
        pixel = pixel_from_rgb(dstfmt, p0, p1, p2);
    }

    let mut p = if cfg!(target_endian = "little") {
        [
            pixel & 0xFF,
            (pixel >> 8) & 0xFF,
            (pixel >> 16) & 0xFF,
            (pixel >> 24) & 0xFF,
        ]
    } else {
        [
            (pixel >> 24) & 0xFF,
            (pixel >> 16) & 0xFF,
            (pixel >> 8) & 0xFF,
            pixel & 0xFF,
        ]
    };

    if p[0] == 0 {
        p[0] = 1;
        alpha_channel = 0;
    } else if p[1] == 0 {
        p[1] = 1;
        alpha_channel = 1;
    } else if p[2] == 0 {
        p[2] = 1;
        alpha_channel = 2;
    } else if p[3] == 0 {
        p[3] = 1;
        alpha_channel = 3;
    }

    if cfg!(target_endian = "big") {
        let srcbpp = srcfmt.bytes_per_pixel;
        let dstbpp = dstfmt.bytes_per_pixel;
        if srcbpp == 3 && dstbpp == 4 {
            for v in p.iter_mut() {
                if *v != 1 {
                    *v -= 1;
                }
            }
        } else if srcbpp == 4 && dstbpp == 3 {
            p = [p[1], p[2], p[3], p[3]];
        }
    }

    (
        [
            p[0] as usize - 1,
            p[1] as usize - 1,
            p[2] as usize - 1,
            p[3] as usize - 1,
        ],
        alpha_channel,
    )
}

/// Translation of `BlitNtoN()`.
fn blit_n_to_n(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstfmt = *info.dst_fmt;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    let alpha = if dstfmt.Amask != 0 { info.a as u32 } else { 0 };

    // Blit with permutation: 4->4
    if srcbpp == 4 && dstbpp == 4 && !srcfmt.format.is_10bit() && !dstfmt.format.is_10bit() {
        // Find the appropriate permutation
        let (p, alpha_channel) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 4, 4, |src, s, dst, d| {
            for k in 0..4 {
                dst[d + k] = src[s + p[k]];
            }
            dst[d + alpha_channel] = alpha as u8;
        });
        return;
    }

    // Blit with permutation: 4->3
    if srcbpp == 4 && dstbpp == 3 && !srcfmt.format.is_10bit() {
        // Find the appropriate permutation
        let (p, _) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 4, 3, |src, s, dst, d| {
            for k in 0..3 {
                dst[d + k] = src[s + p[k]];
            }
        });
        return;
    }

    // Blit with permutation: 3->4
    if srcbpp == 3 && dstbpp == 4 && !dstfmt.format.is_10bit() {
        // Find the appropriate permutation
        let (p, alpha_channel) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 3, 4, |src, s, dst, d| {
            for k in 0..4 {
                dst[d + k] = src[s + p[k]];
            }
            dst[d + alpha_channel] = alpha as u8;
        });
        return;
    }

    walk(info, srcbpp, dstbpp, |src, s, dst, d| {
        let (_, sr, sg, sb) = disemble_rgb(src, s, srcbpp, &srcfmt);
        assemble_rgba(dst, d, dstbpp, &dstfmt, sr, sg, sb, alpha);
    });
}

/// Translation of `BlitNtoNCopyAlpha()`.
fn blit_n_to_n_copy_alpha(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstfmt = *info.dst_fmt;
    let dstbpp = dstfmt.bytes_per_pixel as usize;

    // Blit with permutation: 4->4
    if srcbpp == 4 && dstbpp == 4 && !srcfmt.format.is_10bit() && !dstfmt.format.is_10bit() {
        // Find the appropriate permutation
        let (p, _) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 4, 4, |src, s, dst, d| {
            for k in 0..4 {
                dst[d + k] = src[s + p[k]];
            }
        });
        return;
    }

    walk(info, srcbpp, dstbpp, |src, s, dst, d| {
        let (_, sr, sg, sb, sa) = disemble_rgba(src, s, srcbpp, &srcfmt);
        assemble_rgba(dst, d, dstbpp, &dstfmt, sr, sg, sb, sa);
    });
}

/// Translation of `Blit2to2Key()`.
fn blit_2to2_key(info: &mut BlitInfo<'_>) {
    let rgbmask = !info.src_fmt.Amask;
    let ckey = info.colorkey & rgbmask;
    walk(info, 2, 2, |src, s, dst, d| {
        let v = rd16(src, s);
        if (v & rgbmask) != ckey {
            wr16(dst, d, v);
        }
    });
}

/// The colorkey bytes of a 24-bit pixel in memory order.
fn key_bytes(ckey: u32) -> (u8, u8, u8) {
    if cfg!(target_endian = "little") {
        (ckey as u8, (ckey >> 8) as u8, (ckey >> 16) as u8)
    } else {
        ((ckey >> 16) as u8, (ckey >> 8) as u8, ckey as u8)
    }
}

/// Translation of `BlitNtoNKey()`.
fn blit_n_to_n_key(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    let alpha = if dstfmt.Amask != 0 { info.a as u32 } else { 0 };
    let rgbmask = !srcfmt.Amask;
    let sfmt = srcfmt.format;
    let dfmt = dstfmt.format;

    // Set up some basic variables
    let ckey = info.colorkey & rgbmask;

    // BPP 4, same rgb
    if srcbpp == 4
        && dstbpp == 4
        && srcfmt.Rmask == dstfmt.Rmask
        && srcfmt.Gmask == dstfmt.Gmask
        && srcfmt.Bmask == dstfmt.Bmask
    {
        if dstfmt.Amask != 0 {
            // RGB->RGBA, SET_ALPHA
            let mask = (info.a as u32) << dstfmt.Ashift;
            walk(info, 4, 4, |src, s, dst, d| {
                let v = rd32(src, s);
                if (v & rgbmask) != ckey {
                    wr32(dst, d, v | mask);
                }
            });
        } else {
            // RGBA->RGB, NO_ALPHA
            let mask = srcfmt.Rmask | srcfmt.Gmask | srcfmt.Bmask;
            walk(info, 4, 4, |src, s, dst, d| {
                let v = rd32(src, s);
                if (v & rgbmask) != ckey {
                    wr32(dst, d, v & mask);
                }
            });
        }
        return;
    }

    // Blit with permutation: 4->4
    if srcbpp == 4 && dstbpp == 4 && !srcfmt.format.is_10bit() && !dstfmt.format.is_10bit() {
        // Find the appropriate permutation
        let (p, alpha_channel) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 4, 4, |src, s, dst, d| {
            if (rd32(src, s) & rgbmask) != ckey {
                for k in 0..4 {
                    dst[d + k] = src[s + p[k]];
                }
                dst[d + alpha_channel] = alpha as u8;
            }
        });
        return;
    }

    // BPP 3, same rgb triplet
    if (sfmt == PixelFormat::RGB24 && dfmt == PixelFormat::RGB24)
        || (sfmt == PixelFormat::BGR24 && dfmt == PixelFormat::BGR24)
    {
        let (k0, k1, k2) = key_bytes(ckey);
        walk(info, 3, 3, |src, s, dst, d| {
            let (s0, s1, s2) = (src[s], src[s + 1], src[s + 2]);
            if k0 != s0 || k1 != s1 || k2 != s2 {
                dst[d] = s0;
                dst[d + 1] = s1;
                dst[d + 2] = s2;
            }
        });
        return;
    }

    // BPP 3, inversed rgb triplet
    if (sfmt == PixelFormat::RGB24 && dfmt == PixelFormat::BGR24)
        || (sfmt == PixelFormat::BGR24 && dfmt == PixelFormat::RGB24)
    {
        let (k0, k1, k2) = key_bytes(ckey);
        walk(info, 3, 3, |src, s, dst, d| {
            let (s0, s1, s2) = (src[s], src[s + 1], src[s + 2]);
            if k0 != s0 || k1 != s1 || k2 != s2 {
                // Inversed RGB
                dst[d] = s2;
                dst[d + 1] = s1;
                dst[d + 2] = s0;
            }
        });
        return;
    }

    // Blit with permutation: 4->3
    if srcbpp == 4 && dstbpp == 3 && !srcfmt.format.is_10bit() {
        // Find the appropriate permutation
        let (p, _) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 4, 3, |src, s, dst, d| {
            if (rd32(src, s) & rgbmask) != ckey {
                for k in 0..3 {
                    dst[d + k] = src[s + p[k]];
                }
            }
        });
        return;
    }

    // Blit with permutation: 3->4
    if srcbpp == 3 && dstbpp == 4 && !dstfmt.format.is_10bit() {
        let (k0, k1, k2) = key_bytes(ckey);

        // Find the appropriate permutation
        let (p, alpha_channel) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 3, 4, |src, s, dst, d| {
            let (s0, s1, s2) = (src[s], src[s + 1], src[s + 2]);
            if k0 != s0 || k1 != s1 || k2 != s2 {
                for k in 0..4 {
                    dst[d + k] = src[s + p[k]];
                }
                dst[d + alpha_channel] = alpha as u8;
            }
        });
        return;
    }

    walk(info, srcbpp, dstbpp, |src, s, dst, d| {
        let pixel = retrieve_rgb_pixel(src, s, srcbpp);
        if (pixel & rgbmask) != ckey {
            let (sr, sg, sb) = rgb_from_pixel(pixel, &srcfmt);
            assemble_rgba(dst, d, dstbpp, &dstfmt, sr, sg, sb, alpha);
        }
    });
}

/// Translation of `BlitNtoNKeyCopyAlpha()`.
fn blit_n_to_n_key_copy_alpha(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    let rgbmask = !srcfmt.Amask;

    // Set up some basic variables
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    let ckey = info.colorkey & rgbmask;

    // Fastpath: same source/destination format, with Amask, bpp 32, loop is vectorized. ~10x faster
    if srcfmt.format == dstfmt.format {
        if matches!(
            srcfmt.format,
            PixelFormat::ARGB8888
                | PixelFormat::ABGR8888
                | PixelFormat::BGRA8888
                | PixelFormat::RGBA8888
        ) {
            walk(info, 4, 4, |src, s, dst, d| {
                let v = rd32(src, s);
                if (v & rgbmask) != ckey {
                    wr32(dst, d, v);
                }
            });
        }
        // FIXME (upstream): other formats that are the same on both sides
        // return without blitting anything.
        return;
    }

    // Blit with permutation: 4->4
    if srcbpp == 4 && dstbpp == 4 && !srcfmt.format.is_10bit() && !dstfmt.format.is_10bit() {
        // Find the appropriate permutation
        let (p, _) = get_permutation(&srcfmt, &dstfmt);
        walk(info, 4, 4, |src, s, dst, d| {
            if (rd32(src, s) & rgbmask) != ckey {
                for k in 0..4 {
                    dst[d + k] = src[s + p[k]];
                }
            }
        });
        return;
    }

    walk(info, srcbpp, dstbpp, |src, s, dst, d| {
        let (pixel, sr, sg, sb, sa) = disemble_rgba(src, s, srcbpp, &srcfmt);
        if (pixel & rgbmask) != ckey {
            assemble_rgba(dst, d, dstbpp, &dstfmt, sr, sg, sb, sa);
        }
    });
}

/// Translation of `Blit8888to8888PixelSwizzleSSE41()`/`...AVX2()` (their
/// portable remainder loop, `SWIZZLE_8888_SRC_ALPHA`/`SWIZZLE_8888_DST_ALPHA`,
/// gives the same bytes as the vector part).
fn blit_8888_to_8888_pixel_swizzle(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let dstfmt = *info.dst_fmt;
    let fill_alpha = srcfmt.Amask == 0 || dstfmt.Amask == 0;
    let (_src_amask, _src_ashift) = srcfmt.alpha_mask_and_shift_8888();
    let (dst_amask, _dst_ashift) = dstfmt.alpha_mask_and_shift_8888();
    walk(info, 4, 4, |src, s, dst, d| {
        let src32 = rd32(src, s);
        let rgb = (((src32 >> srcfmt.Rshift) & 0xFF) << dstfmt.Rshift)
            | (((src32 >> srcfmt.Gshift) & 0xFF) << dstfmt.Gshift)
            | (((src32 >> srcfmt.Bshift) & 0xFF) << dstfmt.Bshift);
        let dst32 = if fill_alpha {
            rgb | dst_amask
        } else {
            rgb | (((src32 >> srcfmt.Ashift) & 0xFF) << dstfmt.Ashift)
        };
        wr32(dst, d, dst32);
    });
}

/// Translation of `Blit_3or4_to_3or4__same_rgb()`.
fn blit_3or4_to_3or4_same_rgb(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstfmt = *info.dst_fmt;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    let le = cfg!(target_endian = "little");

    if dstfmt.Amask != 0 {
        // SET_ALPHA
        let mask = (info.a as u32) << dstfmt.Ashift;
        let (i0, i1, i2) = if le {
            (0, 1, 2)
        } else {
            (srcbpp - 1, srcbpp - 2, srcbpp - 3)
        };
        walk(info, srcbpp, 4, |src, s, dst, d| {
            let (s0, s1, s2) = (src[s + i0] as u32, src[s + i1] as u32, src[s + i2] as u32);
            wr32(dst, d, s0 | (s1 << 8) | (s2 << 16) | mask);
        });
    } else {
        // NO_ALPHA
        let (i0, i1, i2) = if le {
            (0, 1, 2)
        } else {
            (srcbpp - 1, srcbpp - 2, srcbpp - 3)
        };
        let (j0, j1, j2) = if le {
            (0, 1, 2)
        } else {
            (dstbpp - 1, dstbpp - 2, dstbpp - 3)
        };
        walk(info, srcbpp, dstbpp, |src, s, dst, d| {
            let (s0, s1, s2) = (src[s + i0], src[s + i1], src[s + i2]);
            dst[d + j0] = s0;
            dst[d + j1] = s1;
            dst[d + j2] = s2;
        });
    }
}

/// Translation of `Blit_3or4_to_3or4__inversed_rgb()`.
fn blit_3or4_to_3or4_inversed_rgb(info: &mut BlitInfo<'_>) {
    let srcfmt = *info.src_fmt;
    let srcbpp = srcfmt.bytes_per_pixel as usize;
    let dstfmt = *info.dst_fmt;
    let dstbpp = dstfmt.bytes_per_pixel as usize;
    let le = cfg!(target_endian = "little");

    if dstfmt.Amask != 0 {
        if srcfmt.Amask != 0 {
            // COPY_ALPHA
            // Only to switch ABGR8888 <-> ARGB8888
            let (i0, i1, i2, i3) = if le { (0, 1, 2, 3) } else { (3, 2, 1, 0) };
            let ashift = dstfmt.Ashift;
            walk(info, 4, 4, |src, s, dst, d| {
                let (s0, s1, s2) = (src[s + i0] as u32, src[s + i1] as u32, src[s + i2] as u32);
                let alphashift = (src[s + i3] as u32) << ashift;
                // inversed, compared to Blit_3or4_to_3or4__same_rgb
                wr32(dst, d, (s0 << 16) | (s1 << 8) | s2 | alphashift);
            });
        } else {
            // SET_ALPHA
            let mask = (info.a as u32) << dstfmt.Ashift;
            let (i0, i1, i2) = if le {
                (0, 1, 2)
            } else {
                (srcbpp - 1, srcbpp - 2, srcbpp - 3)
            };
            walk(info, srcbpp, 4, |src, s, dst, d| {
                let (s0, s1, s2) = (src[s + i0] as u32, src[s + i1] as u32, src[s + i2] as u32);
                // inversed, compared to Blit_3or4_to_3or4__same_rgb
                wr32(dst, d, (s0 << 16) | (s1 << 8) | s2 | mask);
            });
        }
    } else {
        // NO_ALPHA
        let (i0, i1, i2) = if le {
            (0, 1, 2)
        } else {
            (srcbpp - 1, srcbpp - 2, srcbpp - 3)
        };
        let (j0, j1, j2) = if le {
            (2, 1, 0)
        } else {
            (dstbpp - 3, dstbpp - 2, dstbpp - 1)
        };
        walk(info, srcbpp, dstbpp, |src, s, dst, d| {
            let (s0, s1, s2) = (src[s + i0], src[s + i1], src[s + i2]);
            // inversed, compared to Blit_3or4_to_3or4__same_rgb
            dst[d + j0] = s0;
            dst[d + j1] = s1;
            dst[d + j2] = s2;
        });
    }
}

// Normal N to N optimized blitters
const NO_ALPHA: u32 = 1;
const SET_ALPHA: u32 = 2;
const COPY_ALPHA: u32 = 4;

const BLIT_FEATURE_HAS_SSE41: u32 = 1;

struct BlitTable {
    src_r: u32,
    src_g: u32,
    src_b: u32,
    dstbpp: u8,
    dst_r: u32,
    dst_g: u32,
    dst_b: u32,
    blit_features: u32,
    blitfunc: NamedBlit,
    alpha: u32, // bitwise NO_ALPHA, SET_ALPHA, COPY_ALPHA
}

macro_rules! entry {
    ($sr:expr, $sg:expr, $sb:expr, $dbpp:expr, $dr:expr, $dg:expr, $db:expr, $feat:expr, $f:path, $name:literal, $alpha:expr) => {
        BlitTable {
            src_r: $sr,
            src_g: $sg,
            src_b: $sb,
            dstbpp: $dbpp,
            dst_r: $dr,
            dst_g: $dg,
            dst_b: $db,
            blit_features: $feat,
            blitfunc: named_blit!($f, $name),
            alpha: $alpha,
        }
    };
}

const NORMAL_BLIT_1: &[BlitTable] = &[
    // Default for 8-bit RGB source, never optimized
    entry!(0, 0, 0, 0, 0, 0, 0, 0, blit_n_to_n, "BlitNtoN", 0),
];

const NORMAL_BLIT_2: &[BlitTable] = &[
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        BLIT_FEATURE_HAS_SSE41,
        blit_rgb565_32_sse41,
        "Blit_RGB565_32_SSE41",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        BLIT_FEATURE_HAS_SSE41,
        blit_rgb565_32_sse41,
        "Blit_RGB565_32_SSE41",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0xFF000000,
        0x00FF0000,
        0x0000FF00,
        BLIT_FEATURE_HAS_SSE41,
        blit_rgb565_32_sse41,
        "Blit_RGB565_32_SSE41",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0x0000FF00,
        0x00FF0000,
        0xFF000000,
        BLIT_FEATURE_HAS_SSE41,
        blit_rgb565_32_sse41,
        "Blit_RGB565_32_SSE41",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        0,
        blit_rgb565_argb8888,
        "Blit_RGB565_ARGB8888",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        0,
        blit_rgb565_abgr8888,
        "Blit_RGB565_ABGR8888",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0xFF000000,
        0x00FF0000,
        0x0000FF00,
        0,
        blit_rgb565_rgba8888,
        "Blit_RGB565_RGBA8888",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    entry!(
        0x0000F800,
        0x000007E0,
        0x0000001F,
        4,
        0x0000FF00,
        0x00FF0000,
        0xFF000000,
        0,
        blit_rgb565_bgra8888,
        "Blit_RGB565_BGRA8888",
        NO_ALPHA | COPY_ALPHA | SET_ALPHA
    ),
    // Default for 16-bit RGB source, used if no other blitter matches
    entry!(0, 0, 0, 0, 0, 0, 0, 0, blit_n_to_n, "BlitNtoN", 0),
];

const NORMAL_BLIT_3: &[BlitTable] = &[
    // 3->4 with same rgb triplet
    entry!(
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        4,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        0,
        blit_3or4_to_3or4_same_rgb,
        "Blit_3or4_to_3or4__same_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        4,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        0,
        blit_3or4_to_3or4_same_rgb,
        "Blit_3or4_to_3or4__same_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    // 3->4 with inversed rgb triplet
    entry!(
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        4,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        4,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    // 3->3 to switch RGB 24 <-> BGR 24
    entry!(
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        3,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA
    ),
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        3,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA
    ),
    // Default for 24-bit RGB source, never optimized
    entry!(0, 0, 0, 0, 0, 0, 0, 0, blit_n_to_n, "BlitNtoN", 0),
];

const NORMAL_BLIT_4: &[BlitTable] = &[
    // 4->3 with same rgb triplet
    entry!(
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        3,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        0,
        blit_3or4_to_3or4_same_rgb,
        "Blit_3or4_to_3or4__same_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        3,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        0,
        blit_3or4_to_3or4_same_rgb,
        "Blit_3or4_to_3or4__same_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    // 4->3 with inversed rgb triplet
    entry!(
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        3,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        3,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA | SET_ALPHA
    ),
    // 4->4 with inversed rgb triplet, and COPY_ALPHA to switch ABGR8888 <-> ARGB8888
    entry!(
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        4,
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA | SET_ALPHA | COPY_ALPHA
    ),
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        4,
        0x000000FF,
        0x0000FF00,
        0x00FF0000,
        0,
        blit_3or4_to_3or4_inversed_rgb,
        "Blit_3or4_to_3or4__inversed_rgb",
        NO_ALPHA | SET_ALPHA | COPY_ALPHA
    ),
    // RGB 888 and RGB 565
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        2,
        0x0000F800,
        0x000007E0,
        0x0000001F,
        0,
        blit_xrgb8888_rgb565,
        "Blit_XRGB8888_RGB565",
        NO_ALPHA
    ),
    entry!(
        0x00FF0000,
        0x0000FF00,
        0x000000FF,
        2,
        0x00007C00,
        0x000003E0,
        0x0000001F,
        0,
        blit_xrgb8888_rgb555,
        "Blit_XRGB8888_RGB555",
        NO_ALPHA
    ),
    // Default for 32-bit RGB source, used if no other blitter matches
    entry!(0, 0, 0, 0, 0, 0, 0, 0, blit_n_to_n, "BlitNtoN", 0),
];

const NORMAL_BLIT: [&[BlitTable]; 4] = [NORMAL_BLIT_1, NORMAL_BLIT_2, NORMAL_BLIT_3, NORMAL_BLIT_4];

/// Mask matches table, or table entry is zero. Translation of `MASKOK()`.
fn maskok(x: u32, y: u32) -> bool {
    x == y || y == 0x00000000
}

/// Translation of `SDL_CalculateBlitN()`.
pub(crate) fn calculate_blit_n(surface: &Surface<'_>, dst: &Surface<'_>) -> Option<NamedBlit> {
    // Set up data for choosing the blit
    let srcfmt = &surface.fmt;
    let dstfmt = &dst.fmt;
    let simd = simd_support();

    // We don't support destinations less than 8-bits
    if dstfmt.bits_per_pixel < 8 {
        return None;
    }

    match surface.map.flags & !COPY_RLE_MASK {
        0 => {
            if srcfmt.format.pixel_layout() == PackedLayout::L8888
                && dstfmt.format.pixel_layout() == PackedLayout::L8888
            {
                if simd.avx2 {
                    return Some(named_blit!(
                        blit_8888_to_8888_pixel_swizzle,
                        "Blit8888to8888PixelSwizzleAVX2"
                    ));
                }
                if simd.sse41 {
                    return Some(named_blit!(
                        blit_8888_to_8888_pixel_swizzle,
                        "Blit8888to8888PixelSwizzleSSE41"
                    ));
                }
            }

            let mut blitfun = None;
            if dstfmt.bits_per_pixel > 8 {
                let mut a_need = NO_ALPHA;
                if dstfmt.Amask != 0 {
                    a_need = if srcfmt.Amask != 0 {
                        COPY_ALPHA
                    } else {
                        SET_ALPHA
                    };
                }
                let blit_features = if simd.sse41 {
                    BLIT_FEATURE_HAS_SSE41
                } else {
                    0
                };
                if srcfmt.bytes_per_pixel > 0
                    && srcfmt.bytes_per_pixel as usize <= NORMAL_BLIT.len()
                {
                    let table = NORMAL_BLIT[srcfmt.bytes_per_pixel as usize - 1];
                    let mut which = 0;
                    while table[which].dstbpp != 0 {
                        let t = &table[which];
                        if maskok(srcfmt.Rmask, t.src_r)
                            && maskok(srcfmt.Gmask, t.src_g)
                            && maskok(srcfmt.Bmask, t.src_b)
                            && maskok(dstfmt.Rmask, t.dst_r)
                            && maskok(dstfmt.Gmask, t.dst_g)
                            && maskok(dstfmt.Bmask, t.dst_b)
                            && dstfmt.bytes_per_pixel == t.dstbpp
                            && (a_need & t.alpha) == a_need
                            && (t.blit_features & blit_features) == t.blit_features
                        {
                            break;
                        }
                        which += 1;
                    }
                    blitfun = Some(table[which].blitfunc);
                }
                if blitfun.is_some_and(|b| b.name == "BlitNtoN") {
                    // default C fallback catch-all. Slow!
                    if srcfmt.bytes_per_pixel == dstfmt.bytes_per_pixel
                        && srcfmt.Rmask == dstfmt.Rmask
                        && srcfmt.Gmask == dstfmt.Gmask
                        && srcfmt.Bmask == dstfmt.Bmask
                    {
                        if a_need == COPY_ALPHA {
                            if srcfmt.Amask == dstfmt.Amask {
                                // Fastpath C fallback: RGBA<->RGBA blit with matching RGBA
                                blitfun = Some(named_blit!(blit_copy, "SDL_BlitCopy"));
                            } else {
                                blitfun =
                                    Some(named_blit!(blit_n_to_n_copy_alpha, "BlitNtoNCopyAlpha"));
                            }
                        } else if srcfmt.bytes_per_pixel == 4 {
                            // Fastpath C fallback: 32bit RGB<->RGBA blit with matching RGB
                            blitfun = Some(named_blit!(blit_4to4_mask_alpha, "Blit4to4MaskAlpha"));
                        } else if srcfmt.bytes_per_pixel == 2 {
                            // Fastpath C fallback: 16bit RGB<->RGBA blit with matching RGB
                            blitfun = Some(named_blit!(blit_2to2_mask_alpha, "Blit2to2MaskAlpha"));
                        }
                    } else if a_need == COPY_ALPHA {
                        blitfun = Some(named_blit!(blit_n_to_n_copy_alpha, "BlitNtoNCopyAlpha"));
                    }
                }
            }
            blitfun
        }
        COPY_COLORKEY => {
            /* colorkey blit: Here we don't have too many options, mostly
            because RLE is the preferred fast way to deal with this.
            If a particular case turns out to be useful we'll add it. */
            if srcfmt.bytes_per_pixel == 2 && surface.map.identity {
                Some(named_blit!(blit_2to2_key, "Blit2to2Key"))
            } else if srcfmt.Amask != 0 && dstfmt.Amask != 0 {
                Some(named_blit!(
                    blit_n_to_n_key_copy_alpha,
                    "BlitNtoNKeyCopyAlpha"
                ))
            } else {
                Some(named_blit!(blit_n_to_n_key, "BlitNtoNKey"))
            }
        }
        _ => None,
    }
}
