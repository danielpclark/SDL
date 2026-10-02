// Rust translation of src/video/SDL_RLEaccel.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! RLE encoding for software colorkey and alpha-channel acceleration
//!
//! Original version by Sam Lantinga
//!
//! Mattias Engdegård (Yorick): Rewrite. New encoding format, encoder and
//! decoder. Added per-surface alpha blitter. Added per-pixel alpha
//! format, encoder and blitter.
//!
//! Many thanks to Xark and johns for hints, benchmarks and useful comments
//! leading to this code.
//!
//! (Upstream: "Welcome to Macro Mayhem." The macros are functions and an
//! enum of per-run blitters here.)
//!
//! The encoding translates the image data to a stream of segments of the form
//!
//! `<skip> <run> <data>`
//!
//! where `<skip>` is the number of transparent pixels to skip,
//! `<run>` is the number of opaque pixels to blit,
//! and `<data>` are the pixels themselves.
//!
//! This basic structure is used both for colorkeyed surfaces, used for simple
//! binary transparency and for per-surface alpha blending, and for surfaces
//! with per-pixel alpha. The details differ, however:
//!
//! Encoding of colorkeyed surfaces:
//!
//!   Encoded pixels always have the same format as the target surface.
//!   `<skip>` and `<run>` are unsigned 8 bit integers, except for 32 bit depth
//!   where they are 16 bit. This makes the pixel data aligned at all times.
//!   Segments never wrap around from one scan line to the next.
//!
//!   The end of the sequence is marked by a zero `<skip>`,`<run>` pair at the
//!   beginning of a line.
//!
//! Encoding of surfaces with per-pixel alpha:
//!
//!   The sequence begins with an `SDL_PixelFormat` value describing the target
//!   pixel format, to provide reliable un-encoding.
//!
//!   Each scan line is encoded twice: First all completely opaque pixels,
//!   encoded in the target format as described above, and then all
//!   partially transparent (translucent) pixels (where 1 <= alpha <= 254),
//!   in the following 32-bit format:
//!
//!   For 32-bit targets, each pixel has the target RGB format but with
//!   the alpha value occupying the highest 8 bits. The `<skip>` and `<run>`
//!   counts are 16 bit.
//!
//!   For 16-bit targets, each pixel has the target RGB format, but with
//!   the middle component (usually green) shifted 16 steps to the left,
//!   and the hole filled with the 5 most significant bits of the alpha value.
//!   i.e. if the target has the format         rrrrrggggggbbbbb,
//!   the encoded pixel will be 00000gggggg00000rrrrr0aaaaabbbbb.
//!   The `<skip>` and `<run>` counts are 8 bit for the opaque lines, 16 bit
//!   for the translucent lines. Two padding bytes may be inserted
//!   before each translucent line to keep them 32-bit aligned.
//!
//!   The end of the sequence is marked by a zero `<skip>`,`<run>` pair at the
//!   beginning of an opaque line.
//!
//! Upstream aligns the translucent lines by the *address* of the encoded
//! data (`(uintptr_t)dst & 2`), which comes from `SDL_malloc` and so is
//! aligned; the offset within the buffer is used here, which is the same.

use crate::video::blit::*;
use crate::video::pixels::{narrow_channel, PixelFormatDetails};
use crate::video::rect::Rect;
use crate::video::surface::{Pixels, Surface, SurfaceFlags, INTERNAL_SURFACE_RLEACCEL};

/// Which of the two RLE blitters a surface uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RleKind {
    /// `SDL_RLEBlit`
    Colorkey,
    /// `SDL_RLEAlphaBlit`
    Alpha,
}

/// The size of the `SDL_PixelFormat` header.
const HEADER: usize = 4;

/// The per-run blitters chosen by `CHOOSE_BLIT()`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RunBlit {
    /// `OPAQUE_BLIT`
    Opaque,
    /// `ALPHA_BLIT32_888`
    Alpha32_888,
    /// `ALPHA_BLIT16_565`
    Alpha16_565,
    /// `ALPHA_BLIT16_555`
    Alpha16_555,
    /// `ALPHA_BLIT_ANY`
    AlphaAny,
    /// `ALPHA_BLIT32_888_50`
    Alpha32_888_50,
    /// `ALPHA_BLIT16_565_50`
    Alpha16_565_50,
    /// `ALPHA_BLIT16_555_50`
    Alpha16_555_50,
}

/// Translation of `CHOOSE_BLIT()`: the run blitter, or `None` when nothing
/// is drawn (no 8bpp alpha blitting).
fn choose_blit(alpha: u32, fmt: &PixelFormatDetails) -> Option<RunBlit> {
    if alpha == 255 {
        return matches!(fmt.bytes_per_pixel, 1..=4).then_some(RunBlit::Opaque);
    }
    match fmt.bytes_per_pixel {
        1 => None, // No 8bpp alpha blitting
        2 => Some(match fmt.Rmask | fmt.Gmask | fmt.Bmask {
            0xffff if fmt.Gmask == 0x07e0 || fmt.Rmask == 0x07e0 || fmt.Bmask == 0x07e0 => {
                if alpha == 128 {
                    RunBlit::Alpha16_565_50
                } else {
                    RunBlit::Alpha16_565
                }
            }
            0x7fff if fmt.Gmask == 0x03e0 || fmt.Rmask == 0x03e0 || fmt.Bmask == 0x03e0 => {
                if alpha == 128 {
                    RunBlit::Alpha16_555_50
                } else {
                    RunBlit::Alpha16_555
                }
            }
            _ => RunBlit::AlphaAny, // general16
        }),
        3 => Some(RunBlit::AlphaAny),
        4 => Some(
            if (fmt.Rmask | fmt.Gmask | fmt.Bmask) == 0x00ffffff
                && (fmt.Gmask == 0xff00 || fmt.Rmask == 0xff00 || fmt.Bmask == 0xff00)
            {
                if alpha == 128 {
                    RunBlit::Alpha32_888_50
                } else {
                    RunBlit::Alpha32_888
                }
            } else {
                RunBlit::AlphaAny
            },
        ),
        _ => None,
    }
}

/// `BLEND16_50()`
fn blend16_50(d: u32, s: u32, mask: u32) -> u32 {
    (((s & mask) + (d & mask)) >> 1) + (s & d & (!mask & 0xffff))
}

/// Blit `length` pixels from `src[from..]` to `dst[to..]` with one of the
/// per-run blitters (the `do_blit` argument of upstream's macros).
#[allow(clippy::too_many_arguments)]
fn do_blit(
    kind: RunBlit,
    dst: &mut [u8],
    to: usize,
    src: &[u8],
    from: usize,
    length: usize,
    bpp: usize,
    alpha: u32,
    fmt: &PixelFormatDetails,
) {
    match kind {
        RunBlit::Opaque => {
            // PIXEL_COPY
            dst[to..to + length * bpp].copy_from_slice(&src[from..from + length * bpp]);
        }
        RunBlit::Alpha32_888 => {
            /*
             * For 32bpp pixels on the form 0x00rrggbb:
             * If we treat the middle component separately, we can process the two
             * remaining in parallel. This is safe to do because of the gap to the left
             * of each component, so the bits from the multiplication don't collide.
             * This can be used for any RGB permutation of course.
             */
            for i in 0..length {
                let mut s = rd32(src, from + 4 * i);
                let mut d = rd32(dst, to + 4 * i);
                let s1 = s & 0xff00ff;
                let mut d1 = d & 0xff00ff;
                d1 = d1.wrapping_add(s1.wrapping_sub(d1).wrapping_mul(alpha) >> 8) & 0xff00ff;
                s &= 0xff00;
                d &= 0xff00;
                d = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(alpha) >> 8) & 0xff00;
                wr32(dst, to + 4 * i, d1 | d);
            }
        }
        RunBlit::Alpha16_565 | RunBlit::Alpha16_555 => {
            /*
             * For 16bpp pixels we can go a step further: put the middle component
             * in the high 16 bits of a 32 bit word, and process all three RGB
             * components at the same time. Since the smallest gap is here just
             * 5 bits, we have to scale alpha down to 5 bits as well.
             */
            let mask = if kind == RunBlit::Alpha16_565 {
                0x07e0f81f
            } else {
                0x03e07c1f
            };
            let alpha5 = alpha >> 3;
            for i in 0..length {
                let mut s = rd16(src, from + 2 * i);
                let mut d = rd16(dst, to + 2 * i);
                s = (s | s << 16) & mask;
                d = (d | d << 16) & mask;
                d = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(alpha5) >> 5);
                d &= mask;
                wr16(dst, to + 2 * i, d | d >> 16);
            }
        }
        RunBlit::AlphaAny => {
            // The general slow catch-all function, for remaining depths and formats
            for i in 0..length {
                let (si, di) = (from + i * bpp, to + i * bpp);
                let (s, d) = match bpp {
                    2 => (rd16(src, si), rd16(dst, di)),
                    3 => (get_rgb24(src, si), get_rgb24(dst, di)),
                    4 => (rd32(src, si), rd32(dst, di)),
                    _ => (0, 0),
                };
                let (rs, gs, bs) = rgb_from_pixel(s, fmt);
                let (mut rd, mut gd, mut bd) = rgb_from_pixel(d, fmt);
                rd = rd.wrapping_add(rs.wrapping_sub(rd).wrapping_mul(alpha) >> 8);
                gd = gd.wrapping_add(gs.wrapping_sub(gd).wrapping_mul(alpha) >> 8);
                bd = bd.wrapping_add(bs.wrapping_sub(bd).wrapping_mul(alpha) >> 8);
                let d = pixel_from_rgb(fmt, rd, gd, bd);
                match bpp {
                    2 => wr16(dst, di, d),
                    3 => {
                        // SET_RGB24
                        let b = if cfg!(target_endian = "big") {
                            [(d >> 16) as u8, (d >> 8) as u8, d as u8]
                        } else {
                            [d as u8, (d >> 8) as u8, (d >> 16) as u8]
                        };
                        dst[di..di + 3].copy_from_slice(&b);
                    }
                    4 => wr32(dst, di, d),
                    _ => {}
                }
            }
        }
        RunBlit::Alpha32_888_50 => {
            /*
             * Special case: 50% alpha (alpha=128)
             * This is treated specially because it can be optimized very well, and
             * since it is good for many cases of semi-translucency.
             * The theory is to do all three components at the same time:
             * First zero the lowest bit of each component, which gives us room to
             * add them. Then shift right and add the sum of the lowest bits.
             */
            for i in 0..length {
                let s = rd32(src, from + 4 * i);
                let d = rd32(dst, to + 4 * i);
                let v = (((s & 0x00fefefe) + (d & 0x00fefefe)) >> 1) + (s & d & 0x00010101);
                wr32(dst, to + 4 * i, v);
            }
        }
        RunBlit::Alpha16_565_50 | RunBlit::Alpha16_555_50 => {
            /*
             * For 16bpp, we can actually blend two pixels in parallel, if we take
             * care to shift before we add, not after. (Upstream picks the paired
             * form by address; with these masks it gives the same values as one
             * pixel at a time.)
             */
            let mask = if kind == RunBlit::Alpha16_565_50 {
                0xf7de
            } else {
                0xfbde
            };
            for i in 0..length {
                let s = rd16(src, from + 2 * i);
                let d = rd16(dst, to + 2 * i);
                wr16(dst, to + 2 * i, blend16_50(d, s, mask));
            }
        }
    }
}

/// A count stored as `Type` (u8 or u16) at `i`.
fn count(buf: &[u8], i: usize, wide: bool) -> usize {
    if wide {
        rd16(buf, i) as usize
    } else {
        buf[i] as usize
    }
}

/// Translation of `RLEClipBlit()` (the `RLECLIPBLIT` macro): clipped on
/// the left and/or right. Top clipping has already been taken care of.
#[allow(clippy::too_many_arguments)]
fn rle_clip_blit(
    w: i32,
    rle: &[u8],
    mut srcbuf: usize,
    dst: &mut [u8],
    dst_pitch: i32,
    mut dstbuf: isize,
    srcrect: &Rect,
    alpha: u32,
    fmt: &PixelFormatDetails,
) {
    let Some(kind) = choose_blit(alpha, fmt) else {
        return;
    };
    let bpp = fmt.bytes_per_pixel as usize;
    let wide = bpp == 4;
    let tsize = if wide { 2 } else { 1 };

    let mut linecount = srcrect.h;
    let mut ofs: i32 = 0;
    let left = srcrect.x;
    let right = left + srcrect.w;
    dstbuf -= (left as usize * bpp) as isize;
    loop {
        ofs += count(rle, srcbuf, wide) as i32;
        let run = count(rle, srcbuf + tsize, wide) as i32;
        srcbuf += 2 * tsize;
        if run != 0 {
            // clip to left and right borders
            if ofs < right {
                let mut start = 0;
                let mut len = run;
                let mut copy = true;
                if left - ofs > 0 {
                    start = left - ofs;
                    len -= start;
                    if len <= 0 {
                        copy = false;
                    }
                }
                if copy {
                    let startcol = ofs + start;
                    if len > right - startcol {
                        len = right - startcol;
                    }
                    let to = (dstbuf + (startcol as usize * bpp) as isize) as usize;
                    do_blit(
                        kind,
                        dst,
                        to,
                        rle,
                        srcbuf + start as usize * bpp,
                        len as usize,
                        bpp,
                        alpha,
                        fmt,
                    );
                }
            }
            // nocopy:
            srcbuf += run as usize * bpp;
            ofs += run;
        } else if ofs == 0 {
            break;
        }

        if ofs == w {
            ofs = 0;
            dstbuf += dst_pitch as isize;
            linecount -= 1;
            if linecount == 0 {
                break;
            }
        }
    }
}

/// blit a colorkeyed RLE surface. Translation of `SDL_RLEBlit()`.
fn sdl_rle_blit(
    surf_src: &Surface<'_>,
    srcrect: &Rect,
    surf_dst: &mut Surface<'_>,
    dstrect: &Rect,
) {
    let w = surf_src.w;
    let src_fmt = surf_src.fmt;
    let bpp = src_fmt.bytes_per_pixel as usize;
    let wide = bpp == 4;
    let tsize = if wide { 2 } else { 1 };
    let rle = &surf_src.map.rle;
    let dst_pitch = surf_dst.pitch;
    let dst_fmt = surf_dst.fmt;

    // Set up the source and destination pointers
    let (x, y) = (dstrect.x, dstrect.y);
    let mut dstbuf = y as isize * dst_pitch as isize + (x as usize * bpp) as isize;
    let mut srcbuf = HEADER;
    let Some(dst) = surf_dst.pixels.bytes_mut() else {
        return;
    };

    {
        // skip lines at the top if necessary
        let mut vskip = srcrect.y;
        let mut ofs = 0i32;
        if vskip != 0 {
            // RLESKIP(bpp, Type)
            loop {
                ofs += count(rle, srcbuf, wide) as i32;
                let run = count(rle, srcbuf + tsize, wide) as i32;
                srcbuf += 2 * tsize;
                if run != 0 {
                    srcbuf += run as usize * bpp;
                    ofs += run;
                } else if ofs == 0 {
                    return; // goto done
                }
                if ofs == w {
                    ofs = 0;
                    vskip -= 1;
                    if vskip == 0 {
                        break;
                    }
                }
            }
        }
    }

    let alpha = surf_src.map.a as u32;
    // if left or right edge clipping needed, call clip blit
    if srcrect.x != 0 || srcrect.w != surf_src.w {
        rle_clip_blit(
            w, rle, srcbuf, dst, dst_pitch, dstbuf, srcrect, alpha, &dst_fmt,
        );
    } else {
        let fmt = &src_fmt;
        let Some(kind) = choose_blit(alpha, fmt) else {
            return;
        };

        // RLEBLIT(bpp, Type, do_blit)
        let mut linecount = srcrect.h;
        let mut ofs = 0i32;
        loop {
            ofs += count(rle, srcbuf, wide) as i32;
            let run = count(rle, srcbuf + tsize, wide);
            srcbuf += 2 * tsize;
            if run != 0 {
                let to = (dstbuf + (ofs as usize * bpp) as isize) as usize;
                do_blit(kind, dst, to, rle, srcbuf, run, bpp, alpha, fmt);
                srcbuf += run * bpp;
                ofs += run as i32;
            } else if ofs == 0 {
                break;
            }
            if ofs == w {
                ofs = 0;
                dstbuf += dst_pitch as isize;
                linecount -= 1;
                if linecount == 0 {
                    break;
                }
            }
        }
    }
}

/*
 * Per-pixel blitting macros for translucent pixels:
 * These use the same techniques as the per-surface blitting macros
 */

/// The `do_blend` macros of the per-pixel alpha blitters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transl {
    /// `BLIT_TRANSL_888`
    T888,
    /// `BLIT_TRANSL_565`
    T565,
    /// `BLIT_TRANSL_555`
    T555,
}

fn blit_transl(kind: Transl, src: u32, dst: u32) -> u32 {
    match kind {
        Transl::T888 => {
            /*
             * For 32bpp pixels, we have made sure the alpha is stored in the top
             * 8 bits, so proceed as usual
             */
            let mut s = src;
            let mut d = dst;
            let alpha = s >> 24;
            let s1 = s & 0xff00ff;
            let mut d1 = d & 0xff00ff;
            d1 = d1.wrapping_add(s1.wrapping_sub(d1).wrapping_mul(alpha) >> 8) & 0xff00ff;
            s &= 0xff00;
            d &= 0xff00;
            d = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(alpha) >> 8) & 0xff00;
            d1 | d | 0xff000000
        }
        Transl::T565 | Transl::T555 => {
            /*
             * For 16bpp pixels, we have stored the 5 most significant alpha bits in
             * bits 5-10. As before, we can process all 3 RGB components at the same time.
             */
            let mask = if kind == Transl::T565 {
                0x07e0f81f
            } else {
                0x03e07c1f
            };
            let mut s = src;
            let mut d = dst;
            let alpha = (s & 0x3e0) >> 5;
            s &= mask;
            d = (d | d << 16) & mask;
            d = d.wrapping_add(s.wrapping_sub(d).wrapping_mul(alpha) >> 5);
            d &= mask;
            (d | d >> 16) & 0xffff
        }
    }
}

fn transl_kind(df: &PixelFormatDetails) -> Option<(Transl, usize)> {
    match df.bytes_per_pixel {
        2 => Some((
            if df.Gmask == 0x07e0 || df.Rmask == 0x07e0 || df.Bmask == 0x07e0 {
                Transl::T565
            } else {
                Transl::T555
            },
            2,
        )),
        4 => Some((Transl::T888, 4)),
        _ => None,
    }
}

fn rd_px(buf: &[u8], i: usize, psize: usize) -> u32 {
    if psize == 2 {
        rd16(buf, i)
    } else {
        rd32(buf, i)
    }
}

fn wr_px(buf: &mut [u8], i: usize, psize: usize, v: u32) {
    if psize == 2 {
        wr16(buf, i, v)
    } else {
        wr32(buf, i, v)
    }
}

/// blit a pixel-alpha RLE surface clipped at the right and/or left edges.
/// Translation of `RLEAlphaClipBlit()` (`RLEALPHACLIPBLIT`).
#[allow(clippy::too_many_arguments)]
fn rle_alpha_clip_blit(
    w: i32,
    rle: &[u8],
    mut srcbuf: usize,
    dst: &mut [u8],
    dst_pitch: i32,
    mut dstbuf: isize,
    srcrect: &Rect,
    df: &PixelFormatDetails,
) {
    /*
     * clipped blitter: Ptype is the destination pixel type,
     * Ctype the translucent count type, and do_blend the macro
     * to blend one pixel.
     */
    let Some((kind, psize)) = transl_kind(df) else {
        return;
    };
    let wide = psize == 4; // Ctype
    let csize = if wide { 2 } else { 1 };

    let mut linecount = srcrect.h;
    let left = srcrect.x;
    let right = left + srcrect.w;
    dstbuf -= (left as usize * psize) as isize;
    loop {
        let mut ofs = 0i32;
        // blit opaque pixels on one line
        loop {
            ofs += count(rle, srcbuf, wide) as i32;
            let run = count(rle, srcbuf + csize, wide) as i32;
            srcbuf += 2 * csize;
            if run != 0 {
                // clip to left and right borders
                let mut cofs = ofs;
                let mut crun = run;
                if left - cofs > 0 {
                    crun -= left - cofs;
                    cofs = left;
                }
                if crun > right - cofs {
                    crun = right - cofs;
                }
                if crun > 0 {
                    let to = (dstbuf + (cofs as usize * psize) as isize) as usize;
                    let from = srcbuf + (cofs - ofs) as usize * psize;
                    let n = crun as usize * psize;
                    dst[to..to + n].copy_from_slice(&rle[from..from + n]);
                }
                srcbuf += run as usize * psize;
                ofs += run;
            } else if ofs == 0 {
                return;
            }
            if ofs >= w {
                break;
            }
        }
        // skip padding if necessary
        if psize == 2 {
            srcbuf += srcbuf & 2;
        }
        // blit translucent pixels on the same line
        ofs = 0;
        loop {
            ofs += rd16(rle, srcbuf) as i32;
            let run = rd16(rle, srcbuf + 2) as i32;
            srcbuf += 4;
            if run != 0 {
                // clip to left and right borders
                let mut cofs = ofs;
                let mut crun = run;
                if left - cofs > 0 {
                    crun -= left - cofs;
                    cofs = left;
                }
                if crun > right - cofs {
                    crun = right - cofs;
                }
                if crun > 0 {
                    let d0 = (dstbuf + (cofs as usize * psize) as isize) as usize;
                    let s0 = srcbuf + 4 * (cofs - ofs) as usize;
                    for i in 0..crun as usize {
                        let v = blit_transl(
                            kind,
                            rd32(rle, s0 + 4 * i),
                            rd_px(dst, d0 + psize * i, psize),
                        );
                        wr_px(dst, d0 + psize * i, psize, v);
                    }
                }
                srcbuf += run as usize * 4;
                ofs += run;
            }
            if ofs >= w {
                break;
            }
        }
        dstbuf += dst_pitch as isize;
        linecount -= 1;
        if linecount == 0 {
            break;
        }
    }
}

/// blit a pixel-alpha RLE surface. Translation of `SDL_RLEAlphaBlit()`.
fn sdl_rle_alpha_blit(
    surf_src: &Surface<'_>,
    srcrect: &Rect,
    surf_dst: &mut Surface<'_>,
    dstrect: &Rect,
) {
    let w = surf_src.w;
    let df = surf_dst.fmt;
    let rle = &surf_src.map.rle;
    let dst_pitch = surf_dst.pitch;

    let (x, y) = (dstrect.x, dstrect.y);
    let mut dstbuf =
        y as isize * dst_pitch as isize + (x as usize * df.bytes_per_pixel as usize) as isize;
    let mut srcbuf = HEADER;
    let Some(dst) = surf_dst.pixels.bytes_mut() else {
        return;
    };

    {
        // skip lines at the top if necessary
        let mut vskip = srcrect.y;
        if vskip != 0 {
            if df.bytes_per_pixel == 2 {
                // the 16/32 interleaved format
                loop {
                    // skip opaque line
                    let mut ofs = 0i32;
                    loop {
                        ofs += rle[srcbuf] as i32;
                        let run = rle[srcbuf + 1] as i32;
                        srcbuf += 2;
                        if run != 0 {
                            srcbuf += 2 * run as usize;
                            ofs += run;
                        } else if ofs == 0 {
                            return; // goto done
                        }
                        if ofs >= w {
                            break;
                        }
                    }

                    // skip padding
                    srcbuf += srcbuf & 2;

                    // skip translucent line
                    ofs = 0;
                    loop {
                        ofs += rd16(rle, srcbuf) as i32;
                        let run = rd16(rle, srcbuf + 2) as i32;
                        srcbuf += 4 * (run as usize + 1);
                        ofs += run;
                        if ofs >= w {
                            break;
                        }
                    }
                    vskip -= 1;
                    if vskip == 0 {
                        break;
                    }
                }
            } else {
                // the 32/32 interleaved format
                vskip <<= 1; // opaque and translucent have same format
                loop {
                    let mut ofs = 0i32;
                    loop {
                        ofs += rd16(rle, srcbuf) as i32;
                        let run = rd16(rle, srcbuf + 2) as i32;
                        srcbuf += 4;
                        if run != 0 {
                            srcbuf += 4 * run as usize;
                            ofs += run;
                        } else if ofs == 0 {
                            return; // goto done
                        }
                        if ofs >= w {
                            break;
                        }
                    }
                    vskip -= 1;
                    if vskip == 0 {
                        break;
                    }
                }
            }
        }
    }

    // if left or right edge clipping needed, call clip blit
    if srcrect.x != 0 || srcrect.w != surf_src.w {
        rle_alpha_clip_blit(w, rle, srcbuf, dst, dst_pitch, dstbuf, srcrect, &df);
        return;
    }

    /*
     * non-clipped blitter. Ptype is the destination pixel type,
     * Ctype the translucent count type, and do_blend the
     * macro to blend one pixel.
     */
    let Some((kind, psize)) = transl_kind(&df) else {
        return;
    };
    let wide = psize == 4;
    let csize = if wide { 2 } else { 1 };
    let mut linecount = srcrect.h;
    loop {
        let mut ofs = 0i32;
        // blit opaque pixels on one line
        loop {
            ofs += count(rle, srcbuf, wide) as i32;
            let run = count(rle, srcbuf + csize, wide);
            srcbuf += 2 * csize;
            if run != 0 {
                let to = (dstbuf + (ofs as usize * psize) as isize) as usize;
                let n = run * psize;
                dst[to..to + n].copy_from_slice(&rle[srcbuf..srcbuf + n]);
                srcbuf += n;
                ofs += run as i32;
            } else if ofs == 0 {
                return; // goto done
            }
            if ofs >= w {
                break;
            }
        }
        // skip padding if necessary
        if psize == 2 {
            srcbuf += srcbuf & 2;
        }
        // blit translucent pixels on the same line
        ofs = 0;
        loop {
            ofs += rd16(rle, srcbuf) as i32;
            let run = rd16(rle, srcbuf + 2) as usize;
            srcbuf += 4;
            if run != 0 {
                let mut d = (dstbuf + (ofs as usize * psize) as isize) as usize;
                for _ in 0..run {
                    let src = rd32(rle, srcbuf);
                    let v = blit_transl(kind, src, rd_px(dst, d, psize));
                    wr_px(dst, d, psize, v);
                    srcbuf += 4;
                    d += psize;
                }
                ofs += run as i32;
            }
            if ofs >= w {
                break;
            }
        }
        dstbuf += dst_pitch as isize;
        linecount -= 1;
        if linecount == 0 {
            break;
        }
    }
}

/// The top-level RLE blit (`map->blit` set to `SDL_RLEBlit` or
/// `SDL_RLEAlphaBlit`), with the destination lock both do.
pub(crate) fn rle_blit(
    surf_src: &mut Surface<'_>,
    srcrect: &Rect,
    surf_dst: &mut Surface<'_>,
    dstrect: &Rect,
    kind: RleKind,
) -> crate::Result<()> {
    // Lock the destination if necessary
    let must_lock = surf_dst.must_lock();
    if must_lock {
        surf_dst.lock_raw()?;
    }

    match kind {
        RleKind::Colorkey => sdl_rle_blit(surf_src, srcrect, surf_dst, dstrect),
        RleKind::Alpha => sdl_rle_alpha_blit(surf_src, srcrect, surf_dst, dstrect),
    }

    // Unlock the destination if necessary
    if must_lock {
        surf_dst.unlock_raw();
    }
    Ok(())
}

/*
 * Auxiliary functions:
 * The encoding functions take 32bpp rgb + a, and
 * return the number of bytes copied to the destination.
 * The decoding functions copy to 32bpp rgb + a, and
 * return the number of bytes copied from the source.
 * These are only used in the encoder and un-RLE code and are therefore not
 * highly optimised.
 */

/// The per-pixel encoders of `RLEAlphaSurface()`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Copier {
    /// `copy_opaque_16`: encode 32bpp rgb + a into 16bpp rgb, losing alpha
    Opaque16,
    /// `copy_transl_565`: encode 32bpp rgb + a into 32bpp G0RAB format for blitting into 565
    Transl565,
    /// `copy_transl_555`: encode 32bpp rgb + a into 32bpp G0RAB format for blitting into 555
    Transl555,
    /// `copy_32`: encode 32bpp rgba into 32bpp rgba, keeping alpha (dual purpose)
    Copy32,
}

/// Set a pixel value using the given format, except that the alpha value is
/// placed in the top byte. This is the format used for RLE with alpha.
/// Translation of `RLEPIXEL_FROM_RGBA()`.
fn rlepixel_from_rgba(fmt: &PixelFormatDetails, r: u32, g: u32, b: u32, a: u32) -> u32 {
    (narrow_channel(r, fmt.Rbits) << fmt.Rshift)
        | (narrow_channel(g, fmt.Gbits) << fmt.Gshift)
        | (narrow_channel(b, fmt.Bbits) << fmt.Bshift)
        | (a << 24)
}

/// Encode the 32-bit pixels of `src` (`sfmt`) onto the end of `out`.
fn copy_pixels(
    kind: Copier,
    out: &mut Vec<u8>,
    src: &[u32],
    sfmt: &PixelFormatDetails,
    dfmt: &PixelFormatDetails,
) {
    for &p in src {
        match kind {
            Copier::Opaque16 => {
                let (r, g, b) = rgb_from_pixel(p, sfmt);
                let d = pixel_from_rgb(dfmt, r, g, b) as u16;
                out.extend_from_slice(&d.to_ne_bytes());
            }
            Copier::Transl565 | Copier::Transl555 => {
                let (r, g, b, a) = rgba_from_8888(p, sfmt);
                let pix = pixel_from_rgb(dfmt, r, g, b) as u16 as u32;
                let d = if kind == Copier::Transl565 {
                    ((pix & 0x7e0) << 16) | (pix & 0xf81f) | ((a << 2) & 0x7e0)
                } else {
                    ((pix & 0x3e0) << 16) | (pix & 0xfc1f) | ((a << 2) & 0x3e0)
                };
                out.extend_from_slice(&d.to_ne_bytes());
            }
            Copier::Copy32 => {
                let (r, g, b, a) = rgba_from_8888(p, sfmt);
                out.extend_from_slice(&rlepixel_from_rgba(dfmt, r, g, b, a).to_ne_bytes());
            }
        }
    }
}

/// `ISOPAQUE()`
fn is_opaque(pixel: u32, fmt: &PixelFormatDetails) -> bool {
    ((pixel & fmt.Amask) >> fmt.Ashift) == 255
}

/// `ISTRANSL()`
fn is_transl(pixel: u32, fmt: &PixelFormatDetails) -> bool {
    (((pixel & fmt.Amask) >> fmt.Ashift).wrapping_sub(1)) < 254
}

fn add_counts(out: &mut Vec<u8>, n: usize, m: usize, wide: bool) {
    if wide {
        out.extend_from_slice(&(n as u16).to_ne_bytes());
        out.extend_from_slice(&(m as u16).to_ne_bytes());
    } else {
        out.push(n as u8);
        out.push(m as u8);
    }
}

/// convert surface to be quickly alpha-blittable onto dest, if possible.
/// Translation of `RLEAlphaSurface()`.
fn rle_alpha_surface(surface: &mut Surface<'_>, dest: &Surface<'_>) -> bool {
    let df = dest.fmt;
    let max_transl_run = 65535;
    if surface.fmt.bits_per_pixel != 32 {
        return false; // only 32bpp source supported
    }

    /* find out whether the destination is one we support,
    and determine the max size of the encoded result */
    let masksum = df.Rmask | df.Gmask | df.Bmask;
    let (copy_opaque, copy_transl, max_opaque_run) = match df.bytes_per_pixel {
        2 => {
            // 16bpp: only support 565 and 555 formats
            match masksum {
                0xffff if df.Gmask == 0x07e0 || df.Rmask == 0x07e0 || df.Bmask == 0x07e0 => {
                    (Copier::Opaque16, Copier::Transl565, 255) // runs stored as bytes
                }
                0x7fff if df.Gmask == 0x03e0 || df.Rmask == 0x03e0 || df.Bmask == 0x03e0 => {
                    (Copier::Opaque16, Copier::Transl555, 255)
                }
                _ => return false,
            }
        }
        4 => {
            if masksum != 0x00ffffff {
                return false; // requires unused high byte
            }
            (Copier::Copy32, Copier::Copy32, 255) // runs stored as short ints
        }
        _ => return false, // anything else unsupported right now
    };
    let wide = df.bytes_per_pixel == 4;

    let Some(pixels) = surface.pixels.bytes() else {
        return false;
    };
    let mut out: Vec<u8> = Vec::new();
    // save the destination format so we can undo the encoding later
    out.extend_from_slice(&dest.format.0.to_ne_bytes());

    // Do the actual encoding
    let (h, w) = (surface.h as usize, surface.w as usize);
    let sf = surface.fmt;
    let pitch = surface.pitch as usize;
    let mut lastline = out.len(); // end of last non-blank line
    let mut row: Vec<u32> = vec![0; w];

    for y in 0..h {
        for (x, v) in row.iter_mut().enumerate() {
            *v = rd32(pixels, y * (pitch >> 2) * 4 + 4 * x);
        }
        let src = &row;
        let mut blankline = false;
        // First encode all opaque pixels of a scan line
        let mut x = 0;
        loop {
            let skipstart = x;
            while x < w && !is_opaque(src[x], &sf) {
                x += 1;
            }
            let mut runstart = x;
            while x < w && is_opaque(src[x], &sf) {
                x += 1;
            }
            let mut skip = runstart - skipstart;
            if skip == w {
                blankline = true;
            }
            let mut run = x - runstart;
            while skip > max_opaque_run {
                add_counts(&mut out, max_opaque_run, 0, wide);
                skip -= max_opaque_run;
            }
            let mut len = run.min(max_opaque_run);
            add_counts(&mut out, skip, len, wide);
            copy_pixels(
                copy_opaque,
                &mut out,
                &src[runstart..runstart + len],
                &sf,
                &df,
            );
            runstart += len;
            run -= len;
            while run != 0 {
                len = run.min(max_opaque_run);
                add_counts(&mut out, 0, len, wide);
                copy_pixels(
                    copy_opaque,
                    &mut out,
                    &src[runstart..runstart + len],
                    &sf,
                    &df,
                );
                runstart += len;
                run -= len;
            }
            if x >= w {
                break;
            }
        }

        // Make sure the next output address is 32-bit aligned
        let pad = out.len() & 2;
        out.resize(out.len() + pad, 0);

        // Next, encode all translucent pixels of the same scan line
        x = 0;
        loop {
            let skipstart = x;
            while x < w && !is_transl(src[x], &sf) {
                x += 1;
            }
            let mut runstart = x;
            while x < w && is_transl(src[x], &sf) {
                x += 1;
            }
            let mut skip = runstart - skipstart;
            blankline &= skip == w;
            let mut run = x - runstart;
            while skip > max_transl_run {
                add_counts(&mut out, max_transl_run, 0, true);
                skip -= max_transl_run;
            }
            let mut len = run.min(max_transl_run);
            add_counts(&mut out, skip, len, true);
            copy_pixels(
                copy_transl,
                &mut out,
                &src[runstart..runstart + len],
                &sf,
                &df,
            );
            runstart += len;
            run -= len;
            while run != 0 {
                len = run.min(max_transl_run);
                add_counts(&mut out, 0, len, true);
                copy_pixels(
                    copy_transl,
                    &mut out,
                    &src[runstart..runstart + len],
                    &sf,
                    &df,
                );
                runstart += len;
                run -= len;
            }
            if !blankline {
                lastline = out.len();
            }
            if x >= w {
                break;
            }
        }
    }
    out.truncate(lastline); // back up past trailing blank lines
    add_counts(&mut out, 0, 0, wide);

    // reallocate the buffer to release unused memory
    out.shrink_to_fit();
    surface.map.rle = out;

    true
}

/// `getpix_8()` ... `getpix_32()`
fn getpix(buf: &[u8], i: usize, bpp: usize) -> u32 {
    match bpp {
        1 => buf[i] as u32,
        2 => rd16(buf, i),
        3 => {
            if cfg!(target_endian = "little") {
                buf[i] as u32 + ((buf[i + 1] as u32) << 8) + ((buf[i + 2] as u32) << 16)
            } else {
                ((buf[i] as u32) << 16) + ((buf[i + 1] as u32) << 8) + buf[i + 2] as u32
            }
        }
        _ => rd32(buf, i),
    }
}

/// Translation of `RLEColorkeySurface()`.
fn rle_colorkey_surface(surface: &mut Surface<'_>, dest: &Surface<'_>) -> bool {
    let bpp = surface.fmt.bytes_per_pixel as usize;
    if !(1..=4).contains(&bpp) {
        return false;
    }

    let Some(srcbuf) = surface.pixels.bytes() else {
        return false;
    };
    let mut out: Vec<u8> = Vec::new();
    // save the destination format so we can undo the encoding later
    out.extend_from_slice(&dest.format.0.to_ne_bytes());

    // Set up the conversion
    let maxn = if bpp == 4 { 65535 } else { 255 };
    let wide = bpp == 4;
    let rgbmask = !surface.fmt.Amask;
    let ckey = surface.map.colorkey & rgbmask;
    let mut lastline = out.len();
    let (w, h) = (surface.w as usize, surface.h as usize);
    let pitch = surface.pitch as usize;

    for y in 0..h {
        let row = y * pitch;
        let mut x = 0;
        let mut blankline = false;
        loop {
            let skipstart = x;

            // find run of transparent, then opaque pixels
            while x < w && (getpix(srcbuf, row + x * bpp, bpp) & rgbmask) == ckey {
                x += 1;
            }
            let mut runstart = x;
            while x < w && (getpix(srcbuf, row + x * bpp, bpp) & rgbmask) != ckey {
                x += 1;
            }
            let mut skip = runstart - skipstart;
            if skip == w {
                blankline = true;
            }
            let mut run = x - runstart;

            // encode segment
            while skip > maxn {
                add_counts(&mut out, maxn, 0, wide);
                skip -= maxn;
            }
            let mut len = run.min(maxn);
            add_counts(&mut out, skip, len, wide);
            out.extend_from_slice(&srcbuf[row + runstart * bpp..row + (runstart + len) * bpp]);
            run -= len;
            runstart += len;
            while run != 0 {
                len = run.min(maxn);
                add_counts(&mut out, 0, len, wide);
                out.extend_from_slice(&srcbuf[row + runstart * bpp..row + (runstart + len) * bpp]);
                runstart += len;
                run -= len;
            }
            if !blankline {
                lastline = out.len();
            }
            if x >= w {
                break;
            }
        }
    }
    out.truncate(lastline); // back up bast trailing blank lines
    add_counts(&mut out, 0, 0, wide);

    // reallocate the buffer to release unused memory
    out.shrink_to_fit();
    surface.map.rle = out;

    true
}

/// RLE-encode `surface` for blitting to `dest`, if its blit settings allow
/// it. Translation of `SDL_RLESurface()`.
pub(crate) fn rle_surface(surface: &mut Surface<'_>, dest: &Surface<'_>) -> bool {
    // Clear any previous RLE conversion
    if surface.internal_flags & INTERNAL_SURFACE_RLEACCEL != 0 {
        un_rle_surface(surface);
    }

    // We don't support RLE encoding of bitmaps
    if surface.format.bits_per_pixel() < 8 {
        return false;
    }

    // Make sure the pixels are available
    if !surface.pixels.is_some() {
        return false;
    }

    let flags = surface.map.flags;
    let colorkey_ok = flags & COPY_COLORKEY != 0;
    let blend_ok = (flags & COPY_BLEND) != 0 && surface.format.has_alpha();
    if !colorkey_ok && !blend_ok {
        // If we don't have colorkey or blending, nothing to do...
        return false;
    }

    // Pass on combinations not supported
    if (flags & COPY_MODULATE_COLOR) != 0
        || ((flags & COPY_MODULATE_ALPHA) != 0 && surface.format.has_alpha())
        || (flags
            & (COPY_BLEND_PREMULTIPLIED | COPY_ADD | COPY_ADD_PREMULTIPLIED | COPY_MOD | COPY_MUL))
            != 0
        || (flags & COPY_NEAREST) != 0
    {
        return false;
    }

    // Encode and set up the blit
    if !surface.format.has_alpha() || (flags & COPY_BLEND) == 0 {
        if !surface.map.identity {
            return false;
        }
        if !rle_colorkey_surface(surface, dest) {
            return false;
        }
        surface.map.blit = MapBlit::Rle(RleKind::Colorkey);
        surface.map.flags |= COPY_RLE_COLORKEY;
    } else {
        if !rle_alpha_surface(surface, dest) {
            return false;
        }
        surface.map.blit = MapBlit::Rle(RleKind::Alpha);
        surface.map.flags |= COPY_RLE_ALPHAKEY;
    }

    if !surface.flags.contains(SurfaceFlags::PREALLOCATED) {
        surface.saved_pixels = std::mem::replace(&mut surface.pixels, Pixels::None);
    }

    // The surface is now accelerated
    surface.internal_flags |= INTERNAL_SURFACE_RLEACCEL;
    surface.update_lock_flag();

    true
}

/// Translation of `SDL_UnRLESurface()`.
pub(crate) fn un_rle_surface(surface: &mut Surface<'_>) {
    if surface.internal_flags & INTERNAL_SURFACE_RLEACCEL != 0 {
        surface.internal_flags &= !INTERNAL_SURFACE_RLEACCEL;

        surface.map.flags &= !(COPY_RLE_COLORKEY | COPY_RLE_ALPHAKEY);

        if !surface.flags.contains(SurfaceFlags::PREALLOCATED) {
            surface.pixels = std::mem::replace(&mut surface.saved_pixels, Pixels::None);
        }

        surface.map.rle = Vec::new();

        surface.map.invalidate();

        surface.update_lock_flag();
    }
}
