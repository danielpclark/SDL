// Rust translation of src/render/software/SDL_triangle.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Filled and textured triangles for the software renderer, rasterized with
//! barycentric coordinates in fixed point and the top-left rule.

use crate::error::{Error, Result};
use crate::render::TextureAddressMode;
use crate::video::blit::{
    argb2101010_from_rgba, assemble_rgb, assemble_rgba, disemble_rgb, disemble_rgba, rd32,
    rgba_from_argb2101010, wr32,
};
use crate::video::blit::{
    COPY_ADD, COPY_BLEND, COPY_COLORKEY, COPY_MOD, COPY_MODULATE_ALPHA, COPY_MODULATE_COLOR,
    COPY_MUL,
};
use crate::video::pixels::{Color, PixelFormat, PixelFormatDetails};
use crate::video::rect::{Point, Rect};
use crate::video::surface::read_palette;
use crate::video::{BlendMode, Surface};

/* fixed points bits precision
 * Set to 1, so that it can start rendering with middle of a pixel precision.
 * It doesn't need to be increased.
 * But, if increased too much, it overflows (srcx, srcy) coordinates used for filling with texture.
 * (which could be turned to int64).
 */
const FP_BITS: i32 = 1;

// The disabled SDL_BlitTriangle() and SDL_FillTriangle() wrappers are not
// translated.

/// cross product AB x AC
fn cross_product(a: &Point, b: &Point, c_x: i32, c_y: i32) -> i64 {
    (b.x - a.x) as i64 * (c_y - a.y) as i64 - (b.y - a.y) as i64 * (c_x - a.x) as i64
}

/// check for top left rules
fn is_top_left(a: &Point, b: &Point, is_clockwise: bool) -> bool {
    if is_clockwise {
        if a.y == b.y && a.x < b.x {
            return true;
        }
        if b.y < a.y {
            return true;
        }
    } else {
        if a.y == b.y && b.x < a.x {
            return true;
        }
        if a.y < b.y {
            return true;
        }
    }
    false
}

/// `PRECOMP()`: `y << FP_BITS`, avoiding the left shift of a negative value.
fn precomp(val: i32) -> i32 {
    if val >= 0 {
        val << FP_BITS
    } else {
        -((-val) << FP_BITS)
    }
}

/// Convert a triangle point to fixed point. Translation of `trianglepoint_2_fixedpoint()`.
pub(crate) fn trianglepoint_2_fixedpoint(a: &mut Point) {
    a.x = precomp(a.x);
    a.y = precomp(a.y);
}

/// bounding rect of three points (in fixed point)
fn bounding_rect_fixedpoint(a: &Point, b: &Point, c: &Point) -> Rect {
    let min_x = a.x.min(b.x.min(c.x));
    let max_x = a.x.max(b.x.max(c.x));
    let min_y = a.y.min(b.y.min(c.y));
    let max_y = a.y.max(b.y.max(c.y));
    // points are in fixed point, shift back
    Rect::new(
        min_x >> FP_BITS,
        min_y >> FP_BITS,
        (max_x - min_x) >> FP_BITS,
        (max_y - min_y) >> FP_BITS,
    )
}

/// `SDL_GetRectIntersection(&a, b, &a)`.
fn intersect_in_place(a: &mut Rect, b: &Rect) {
    let copy = *a;
    copy.intersect_into(b, a);
}

/// The rasterizer state shared by the triangle loops
/// (`TRIANGLE_BEGIN_LOOP` / `TRIANGLE_END_LOOP`).
#[derive(Clone, Copy)]
struct Raster {
    dstrect: Rect,
    bias: [i64; 3],
    /// x steps of (w0, w1, w2)
    dx: [i64; 3],
    /// y steps of (w0, w1, w2)
    dy: [i64; 3],
    /// (w0, w1, w2) at the start of the first row
    row: [i64; 3],
    area: i64,
}

impl Raster {
    /// The setup common to filling and blitting.
    fn new(d0: &Point, d1: &Point, d2: &Point, dstrect: Rect, area: i64) -> Raster {
        let is_clockwise = area > 0;
        let area = area.abs();

        let mut dx = [
            precomp(d1.y - d2.y),
            precomp(d2.y - d0.y),
            precomp(d0.y - d1.y),
        ];
        let mut dy = [
            precomp(d2.x - d1.x),
            precomp(d0.x - d2.x),
            precomp(d1.x - d0.x),
        ];

        // Starting point for rendering, at the middle of a pixel
        let mut p = Point {
            x: dstrect.x,
            y: dstrect.y,
        };
        trianglepoint_2_fixedpoint(&mut p);
        p.x += (1 << FP_BITS) / 2;
        p.y += (1 << FP_BITS) / 2;
        let mut row = [
            cross_product(d1, d2, p.x, p.y),
            cross_product(d2, d0, p.x, p.y),
            cross_product(d0, d1, p.x, p.y),
        ];

        // Handle anti-clockwise triangles
        if !is_clockwise {
            for v in dx.iter_mut().chain(dy.iter_mut()) {
                *v *= -1;
            }
            for v in row.iter_mut() {
                *v *= -1;
            }
        }

        // Add a bias to respect top-left rasterization rule
        let bias = [
            if is_top_left(d1, d2, is_clockwise) {
                0
            } else {
                -1
            },
            if is_top_left(d2, d0, is_clockwise) {
                0
            } else {
                -1
            },
            if is_top_left(d0, d1, is_clockwise) {
                0
            } else {
                -1
            },
        ];

        Raster {
            dstrect,
            bias,
            dx: dx.map(i64::from),
            dy: dy.map(i64::from),
            row,
            area,
        }
    }

    /// Visit the pixels inside the triangle as `(x, y, w)`.
    ///
    /// Upstream's loop body is a macro pair whose x step comes after the
    /// body, so a `continue` in the body (the color key test of
    /// `SDL_SW_BlitTriangle()`) skips the step and the rest of the row is
    /// drawn from stale coordinates: after the first color keyed texel,
    /// the row stays on that texel and nothing more is drawn. Fixed here:
    /// the coordinates step after every pixel.
    fn for_each(&self, mut f: impl FnMut(i32, i32, [i64; 3])) {
        let mut row = self.row;
        for y in 0..self.dstrect.h {
            // y start
            let mut w = row;
            for x in 0..self.dstrect.w {
                // In triangle
                if w[0] + self.bias[0] >= 0 && w[1] + self.bias[1] >= 0 && w[2] + self.bias[2] >= 0
                {
                    f(x, y, w);
                }
                // x += 1
                for (wk, dk) in w.iter_mut().zip(self.dx) {
                    *wk += dk;
                }
            }
            // y += 1
            for (rk, dk) in row.iter_mut().zip(self.dy) {
                *rk += dk;
            }
        }
    }

    /// `TRIANGLE_GET_COLOR` (and `TRIANGLE_GET_MAPPED_COLOR` before the
    /// truncation to bytes).
    fn color(&self, w: [i64; 3], c0: Color, c1: Color, c2: Color) -> [i32; 4] {
        let mix = |a: u8, b: u8, c: u8| {
            ((w[0] * a as i64 + w[1] * b as i64 + w[2] * c as i64) / self.area) as i32
        };
        [
            mix(c0.r, c1.r, c2.r),
            mix(c0.g, c1.g, c2.g),
            mix(c0.b, c1.b, c2.b),
            mix(c0.a, c1.a, c2.a),
        ]
    }
}

/// Store the low `bpp` bytes of a mapped color.
fn store_color(px: &mut [u8], i: usize, bpp: usize, color: u32) {
    match bpp {
        4 => wr32(px, i, color),
        3 => px[i..i + 3].copy_from_slice(&color.to_ne_bytes()[..3]),
        2 => px[i..i + 2].copy_from_slice(&(color as u16).to_ne_bytes()),
        1 => px[i] = color as u8,
        _ => {}
    }
}

/// Fill a triangle given in fixed point, with per-vertex colors and a
/// blend mode. Translation of `SDL_SW_FillTriangle()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sw_fill_triangle(
    dst: &mut Surface<'_>,
    d0: &Point,
    d1: &Point,
    d2: &Point,
    blend: BlendMode,
    c0: Color,
    c1: Color,
    c2: Color,
) -> Result<()> {
    let area = cross_product(d0, d1, d2.x, d2.y);

    let is_uniform = c0 == c1 && c1 == c2;

    // Flat triangle
    if area == 0 {
        return Ok(());
    }

    // Lock the destination, if needed
    let dst_locked = dst.must_lock();
    if dst_locked {
        dst.lock_raw()?;
    }
    let result = fill_triangle_locked(dst, d0, d1, d2, blend, c0, c1, c2, area, is_uniform);
    if dst_locked {
        dst.unlock_raw();
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn fill_triangle_locked(
    dst: &mut Surface<'_>,
    d0: &Point,
    d1: &Point,
    d2: &Point,
    blend: BlendMode,
    c0: Color,
    c1: Color,
    c2: Color,
    area: i64,
    is_uniform: bool,
) -> Result<()> {
    let mut dstrect = bounding_rect_fixedpoint(d0, d1, d2);

    // Clip triangle rect with surface rect
    intersect_in_place(&mut dstrect, &Rect::new(0, 0, dst.w, dst.h));

    // Clip triangle with surface clip rect
    let clip = dst.clip_rect();
    intersect_in_place(&mut dstrect, &clip);

    let raster = Raster::new(d0, d1, d2, dstrect, area);

    if blend != BlendMode::NONE {
        let mut format = dst.format;

        // need an alpha format
        if !format.has_alpha() {
            format = PixelFormat::ARGB8888;
        }

        // Use an intermediate surface
        let mut tmp = Surface::new(dstrect.w, dstrect.h, format)?;

        if blend == BlendMode::MOD {
            let c = tmp.map_rgba(255, 255, 255, 255);
            let _ = tmp.fill_rect(None, c); // (upstream ignores the result)
        }

        // (upstream ignores the result: an unknown mode leaves the surface
        // without blending)
        let _ = tmp.set_blend_mode(blend);

        let pitch = tmp.pitch as usize;
        fill_triangle_pixels(&mut tmp, 0, pitch, &raster, c0, c1, c2, is_uniform);

        // (upstream ignores the result of the blit)
        let _ = tmp.blit(None, dst, Some(&dstrect));
    } else {
        // Write directly to destination surface
        let bpp = dst.fmt.bytes_per_pixel as isize;
        let pitch = dst.pitch as usize;
        let base = dstrect.x as isize * bpp + dstrect.y as isize * pitch as isize;
        fill_triangle_pixels(dst, base, pitch, &raster, c0, c1, c2, is_uniform);
    }
    Ok(())
}

/// The fill loops of `SDL_SW_FillTriangle()`, writing to `target` from
/// byte `base` with rows `pitch` apart.
#[allow(clippy::too_many_arguments)]
fn fill_triangle_pixels(
    target: &mut Surface<'_>,
    base: isize,
    pitch: usize,
    raster: &Raster,
    c0: Color,
    c1: Color,
    c2: Color,
    is_uniform: bool,
) {
    let fmt = target.fmt;
    let bpp = fmt.bytes_per_pixel as usize;
    let palette = target.palette.clone();
    let uniform_color = target.map_rgba(c0.r, c0.g, c0.b, c0.a);
    let Some(px) = target.pixels.bytes_mut() else {
        return;
    };
    let palette = palette.as_ref().map(|p| read_palette(p));
    let at =
        |x: i32, y: i32| (base + y as isize * pitch as isize + x as isize * bpp as isize) as usize;

    if is_uniform {
        raster.for_each(|x, y, _| {
            store_color(px, at(x, y), bpp, uniform_color);
        });
    } else {
        raster.for_each(|x, y, w| {
            let [r, g, b, a] = raster.color(w, c0, c1, c2);
            let color = fmt
                .map_rgba(
                    palette.as_deref(),
                    Color::new(r as u8, g as u8, b as u8, a as u8),
                )
                .unwrap_or(0);
            store_color(px, at(x, y), bpp, color);
        });
    }
}

/// Blit a textured triangle: source points in texels, destination points
/// in fixed point, per-vertex modulation colors. Translation of
/// `SDL_SW_BlitTriangle()`. The address modes must not be `Auto` (the
/// renderer resolves them).
#[allow(clippy::too_many_arguments)]
pub(crate) fn sw_blit_triangle(
    src: &mut Surface<'_>,
    s0: &Point,
    s1: &Point,
    s2: &Point,
    dst: &mut Surface<'_>,
    d0: &Point,
    d1: &Point,
    d2: &Point,
    c0: Color,
    c1: Color,
    c2: Color,
    texture_address_mode_u: TextureAddressMode,
    texture_address_mode_v: TextureAddressMode,
) -> Result<()> {
    let area = cross_product(d0, d1, d2.x, d2.y);

    // Flat triangle
    if area == 0 {
        return Ok(());
    }

    // Lock the destination, if needed
    let dst_locked = dst.must_lock();
    if dst_locked {
        dst.lock_raw()?;
    }

    // Lock the source, if needed
    let src_locked = src.must_lock();
    let result = if src_locked { src.lock_raw() } else { Ok(()) }.and_then(|()| {
        blit_triangle_locked(
            src,
            [s0, s1, s2],
            dst,
            [d0, d1, d2],
            [c0, c1, c2],
            area,
            texture_address_mode_u,
            texture_address_mode_v,
        )
    });

    if dst_locked {
        dst.unlock_raw();
    }
    if src_locked {
        src.unlock_raw();
    }
    result
}

/// `TRIANGLE_GET_TEXTCOORD`: the texel for barycentric coordinates `w`.
#[derive(Clone, Copy)]
struct TexCoords {
    s2_x_area: (i64, i64),
    s2s0: (i64, i64),
    s2s1: (i64, i64),
    area: i64,
    w: i32,
    h: i32,
    mode_u: TextureAddressMode,
    mode_v: TextureAddressMode,
}

impl TexCoords {
    fn get(&self, w: [i64; 3]) -> (i32, i32) {
        // Use 64 bits precision to prevent overflow when interpolating color / texture with wide triangles
        let mut srcx =
            ((w[0] * self.s2s0.0 + w[1] * self.s2s1.0 + self.s2_x_area.0) / self.area) as i32;
        let mut srcy =
            ((w[0] * self.s2s0.1 + w[1] * self.s2s1.1 + self.s2_x_area.1) / self.area) as i32;
        let address = |v: &mut i32, mode: TextureAddressMode, size: i32| match mode {
            TextureAddressMode::Clamp => {
                if *v < 0 {
                    *v = 0;
                } else if *v >= size {
                    *v = size - 1;
                }
            }
            TextureAddressMode::Wrap => {
                *v %= size;
                if *v < 0 {
                    // (upstream adds size - 1, which puts negative
                    // coordinates one texel off, -1 on size - 2; fixed here)
                    *v += size;
                }
            }
            _ => {}
        };
        address(&mut srcx, self.mode_u, self.w);
        address(&mut srcy, self.mode_v, self.h);
        (srcx, srcy)
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_triangle_locked(
    src: &mut Surface<'_>,
    [s0, s1, s2]: [&Point; 3],
    dst: &mut Surface<'_>,
    [d0, d1, d2]: [&Point; 3],
    [c0, c1, c2]: [Color; 3],
    area: i64,
    texture_address_mode_u: TextureAddressMode,
    texture_address_mode_v: TextureAddressMode,
) -> Result<()> {
    let is_uniform = c0 == c1 && c1 == c2;

    let mut dstrect = bounding_rect_fixedpoint(d0, d1, d2);

    let blend = src.blend_mode();

    let has_modulation = if is_uniform {
        // SDL_GetSurfaceColorMod(src, &r, &g, &b);
        c0.r != 255 || c0.g != 255 || c0.b != 255 || c0.a != 255
    } else {
        true
    };

    // Clip triangle with surface clip rect
    let clip = dst.clip_rect();
    intersect_in_place(&mut dstrect, &clip);

    let raster = Raster::new(d0, d1, d2, dstrect, area);
    let area = raster.area;

    let s2s0 = ((s0.x - s2.x) as i64, (s0.y - s2.y) as i64);
    let s2s1 = ((s1.x - s2.x) as i64, (s1.y - s2.y) as i64);

    /* precompute constant 's2->x * area' used in TRIANGLE_GET_TEXTCOORD */
    let tmp64 = s2.x as i64 * area;
    if tmp64 < i32::MIN as i64 || tmp64 > i32::MAX as i64 {
        return Err(Error::new("triangle area overflow"));
    }
    let s2_x = tmp64;
    let tmp64 = s2.y as i64 * area;
    if tmp64 < i32::MIN as i64 || tmp64 > i32::MAX as i64 {
        return Err(Error::new("triangle area overflow"));
    }
    let s2_y = tmp64;

    let tex = TexCoords {
        s2_x_area: (s2_x, s2_y),
        s2s0,
        s2s1,
        area,
        w: src.w,
        h: src.h,
        mode_u: texture_address_mode_u,
        mode_v: texture_address_mode_v,
    };

    let dstbpp = dst.fmt.bytes_per_pixel as isize;
    let dst_pitch = dst.pitch as isize;
    let dst_base = dstrect.x as isize * dstbpp + dstrect.y as isize * dst_pitch;

    if blend != BlendMode::NONE || src.format != dst.format || has_modulation || !is_uniform {
        // Use SDL_BlitTriangle_Slow

        let mut flags = src.map.flags;
        flags &= !(COPY_MODULATE_COLOR | COPY_MODULATE_ALPHA);

        if c0.r != 255
            || c1.r != 255
            || c2.r != 255
            || c0.g != 255
            || c1.g != 255
            || c2.g != 255
            || c0.b != 255
            || c1.b != 255
            || c2.b != 255
        {
            flags |= COPY_MODULATE_COLOR;
        }

        if c0.a != 255 || c1.a != 255 || c2.a != 255 {
            flags |= COPY_MODULATE_ALPHA;
        }

        let check_int_range = |name: &str, x: i64| {
            if x < i32::MIN as i64 || x > i32::MAX as i64 {
                Err(Error::new(format!("integer overflow ({name} = {x})")))
            } else {
                Ok(())
            }
        };
        check_int_range("area", area)?;
        check_int_range("w0_row", raster.row[0])?;
        check_int_range("w1_row", raster.row[1])?;
        check_int_range("w2_row", raster.row[2])?;

        let info = SlowInfo {
            flags,
            modulate: [c0.r as u32, c0.g as u32, c0.b as u32, c0.a as u32],
            colorkey: src.map.colorkey,
            colors: [c0, c1, c2],
            is_uniform,
        };
        blit_triangle_slow(src, dst, dst_base, &raster, &tex, &info);
        return Ok(());
    }

    let src_pitch = src.pitch as usize;
    let Some(sp) = src.pixels.bytes() else {
        return Ok(());
    };
    let Some(dp) = dst.pixels.bytes_mut() else {
        return Ok(());
    };
    let bpp = dstbpp as usize;
    raster.for_each(|x, y, w| {
        let (srcx, srcy) = tex.get(w);
        let d = (dst_base + y as isize * dst_pitch + x as isize * dstbpp) as usize;
        let s = srcy as usize * src_pitch + srcx as usize * bpp;
        dp[d..d + bpp].copy_from_slice(&sp[s..s + bpp]);
    });
    Ok(())
}

/// The modulation and colorkey of a slow triangle blit (the parts of the
/// `SDL_BlitInfo` it uses).
struct SlowInfo {
    flags: u32,
    modulate: [u32; 4],
    colorkey: u32,
    colors: [Color; 3],
    is_uniform: bool,
}

/// `detect_format()`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TriFormat {
    Alpha,
    NoAlpha,
    Index8,
    Argb2101010,
}

fn detect_format(pf: &PixelFormatDetails) -> TriFormat {
    if pf.format.is_indexed() {
        TriFormat::Index8
    } else if pf.format == PixelFormat::ARGB2101010 {
        TriFormat::Argb2101010
    } else if pf.Amask != 0 {
        TriFormat::Alpha
    } else {
        TriFormat::NoAlpha
    }
}

/// Translation of `SDL_BlitTriangle_Slow()`.
fn blit_triangle_slow(
    src: &Surface<'_>,
    dst: &mut Surface<'_>,
    dst_base: isize,
    raster: &Raster,
    tex: &TexCoords,
    info: &SlowInfo,
) {
    let flags = info.flags;
    let [mut modulate_r, mut modulate_g, mut modulate_b, mut modulate_a] = info.modulate;
    let src_fmt = src.fmt;
    let dst_fmt = dst.fmt;
    let srcbpp = src_fmt.bytes_per_pixel as usize;
    let dstbpp = dst_fmt.bytes_per_pixel as usize;
    let rgbmask = !src_fmt.Amask;
    let ckey = info.colorkey & rgbmask;
    let [c0, c1, c2] = info.colors;

    let srcfmt_val = detect_format(&src_fmt);
    let dstfmt_val = detect_format(&dst_fmt);

    let palette = src.palette.as_ref().map(|p| read_palette(p));
    let src_pitch = src.pitch as isize;
    let dst_pitch = dst.pitch as isize;
    let Some(sp) = src.pixels.bytes() else {
        return;
    };
    let Some(dp) = dst.pixels.bytes_mut() else {
        return;
    };

    raster.for_each(|x, y, w| {
        let d = (dst_base + y as isize * dst_pitch + x as isize * dstbpp as isize) as usize;
        let (srcx, srcy) = tex.get(w);
        let s = (srcy as isize * src_pitch + srcx as isize * srcbpp as isize) as usize;

        let (mut srcpixel, mut src_r, mut src_g, mut src_b, mut src_a);
        match srcfmt_val {
            TriFormat::Index8 => {
                srcpixel = sp[s] as u32;
                // (upstream reads past a short palette)
                let color = palette
                    .as_ref()
                    .and_then(|p| p.colors().get(srcpixel as usize).copied())
                    .unwrap_or_default();
                (src_r, src_g, src_b, src_a) = (
                    color.r as u32,
                    color.g as u32,
                    color.b as u32,
                    color.a as u32,
                );
            }
            TriFormat::Alpha => {
                (srcpixel, src_r, src_g, src_b, src_a) = disemble_rgba(sp, s, srcbpp, &src_fmt);
            }
            TriFormat::NoAlpha => {
                (srcpixel, src_r, src_g, src_b) = disemble_rgb(sp, s, srcbpp, &src_fmt);
                src_a = 0xFF;
            }
            TriFormat::Argb2101010 => {
                srcpixel = rd32(sp, s);
                (src_r, src_g, src_b, src_a) = rgba_from_argb2101010(srcpixel);
            }
        }
        if flags & COPY_COLORKEY != 0 {
            // srcpixel isn't set for 24 bpp
            if srcbpp == 3 {
                srcpixel = (src_r << src_fmt.Rshift)
                    | (src_g << src_fmt.Gshift)
                    | (src_b << src_fmt.Bshift);
            }
            if (srcpixel & rgbmask) == ckey {
                return;
            }
        }
        let (mut dst_r, mut dst_g, mut dst_b, mut dst_a);
        if flags & (COPY_BLEND | COPY_ADD | COPY_MOD | COPY_MUL) != 0 {
            match dstfmt_val {
                TriFormat::Alpha => {
                    (_, dst_r, dst_g, dst_b, dst_a) = disemble_rgba(dp, d, dstbpp, &dst_fmt);
                }
                TriFormat::Argb2101010 => {
                    let dstpixel = rd32(dp, d);
                    (dst_r, dst_g, dst_b, dst_a) = rgba_from_argb2101010(dstpixel);
                }
                _ => {
                    // FORMAT_HAS_NO_ALPHA (indexed formats too)
                    (_, dst_r, dst_g, dst_b) = disemble_rgb(dp, d, dstbpp, &dst_fmt);
                    dst_a = 0xFF;
                }
            }
        } else {
            // don't care
            (dst_r, dst_g, dst_b, dst_a) = (0, 0, 0, 0);
        }

        if !info.is_uniform {
            let [r, g, b, a] = raster.color(w, c0, c1, c2);
            modulate_r = r as u32;
            modulate_g = g as u32;
            modulate_b = b as u32;
            modulate_a = a as u32;
        }

        if flags & COPY_MODULATE_COLOR != 0 {
            src_r = src_r.wrapping_mul(modulate_r) / 255;
            src_g = src_g.wrapping_mul(modulate_g) / 255;
            src_b = src_b.wrapping_mul(modulate_b) / 255;
        }
        if flags & COPY_MODULATE_ALPHA != 0 {
            src_a = src_a.wrapping_mul(modulate_a) / 255;
        }
        if flags & (COPY_BLEND | COPY_ADD) != 0 {
            // This goes away if we ever use premultiplied alpha
            if src_a < 255 {
                src_r = src_r.wrapping_mul(src_a) / 255;
                src_g = src_g.wrapping_mul(src_a) / 255;
                src_b = src_b.wrapping_mul(src_a) / 255;
            }
        }
        match flags & (COPY_BLEND | COPY_ADD | COPY_MOD | COPY_MUL) {
            0 => {
                dst_r = src_r;
                dst_g = src_g;
                dst_b = src_b;
                dst_a = src_a;
            }
            COPY_BLEND => {
                dst_r = src_r.wrapping_add(255u32.wrapping_sub(src_a).wrapping_mul(dst_r) / 255);
                dst_g = src_g.wrapping_add(255u32.wrapping_sub(src_a).wrapping_mul(dst_g) / 255);
                dst_b = src_b.wrapping_add(255u32.wrapping_sub(src_a).wrapping_mul(dst_b) / 255);
                dst_a = src_a.wrapping_add(255u32.wrapping_sub(src_a).wrapping_mul(dst_a) / 255);
            }
            COPY_ADD => {
                dst_r = src_r.wrapping_add(dst_r).min(255);
                dst_g = src_g.wrapping_add(dst_g).min(255);
                dst_b = src_b.wrapping_add(dst_b).min(255);
            }
            COPY_MOD => {
                dst_r = src_r.wrapping_mul(dst_r) / 255;
                dst_g = src_g.wrapping_mul(dst_g) / 255;
                dst_b = src_b.wrapping_mul(dst_b) / 255;
            }
            COPY_MUL => {
                let mul = |s: u32, d: u32| {
                    (s.wrapping_mul(d)
                        .wrapping_add(d.wrapping_mul(255u32.wrapping_sub(src_a)))
                        / 255)
                        .min(255)
                };
                dst_r = mul(src_r, dst_r);
                dst_g = mul(src_g, dst_g);
                dst_b = mul(src_b, dst_b);
            }
            _ => {}
        }
        match dstfmt_val {
            TriFormat::Alpha => assemble_rgba(dp, d, dstbpp, &dst_fmt, dst_r, dst_g, dst_b, dst_a),
            TriFormat::Argb2101010 => {
                wr32(dp, d, argb2101010_from_rgba(dst_r, dst_g, dst_b, dst_a))
            }
            _ => assemble_rgb(dp, d, dstbpp, &dst_fmt, dst_r, dst_g, dst_b),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // A color keyed texel doesn't stop the rest of the row.
    #[test]
    fn blit_triangle_after_color_key() {
        let mut src = Surface::new(2, 1, PixelFormat::XRGB8888).unwrap();
        let white = src.map_rgb(255, 255, 255);
        src.fill_rect(Some(&Rect::new(1, 0, 1, 1)), white).unwrap();
        let black = src.map_rgb(0, 0, 0);
        src.set_color_key(Some(black)).unwrap();
        src.set_blend_mode(BlendMode::NONE).unwrap();
        // (another format than the source, for the general blitter)
        let mut dst = Surface::new(16, 16, PixelFormat::ARGB8888).unwrap();
        let red = dst.map_rgba(255, 0, 0, 255);
        dst.fill_rect(None, red).unwrap();

        let p = |x, y| Point { x, y };
        let c = Color::new(255, 255, 255, 255);
        // (the destination in fixed point, as the renderer passes it)
        let mut d = [p(0, 0), p(16, 0), p(0, 16)];
        d.iter_mut().for_each(trianglepoint_2_fixedpoint);
        sw_blit_triangle(
            &mut src,
            &p(0, 0),
            &p(2, 0),
            &p(0, 1),
            &mut dst,
            &d[0],
            &d[1],
            &d[2],
            c,
            c,
            c,
            TextureAddressMode::Clamp,
            TextureAddressMode::Clamp,
        )
        .unwrap();
        // the keyed left half is left alone, the right half is drawn
        assert_eq!(dst.read_pixel(2, 1).unwrap(), Color::new(255, 0, 0, 255));
        assert_eq!(
            dst.read_pixel(12, 1).unwrap(),
            Color::new(255, 255, 255, 255)
        );
    }

    // Wrapped texture coordinates repeat the texture to the left and above.
    #[test]
    fn wrap_negative_coordinates() {
        let at = |x: i64, y: i64| {
            TexCoords {
                s2_x_area: (x, y),
                s2s0: (0, 0),
                s2s1: (0, 0),
                area: 1,
                w: 4,
                h: 3,
                mode_u: TextureAddressMode::Wrap,
                mode_v: TextureAddressMode::Wrap,
            }
            .get([0, 0, 0])
        };
        assert_eq!(at(-1, -1), (3, 2));
        assert_eq!(at(-3, -2), (1, 1));
        assert_eq!(at(-4, -3), (0, 0));
        assert_eq!(at(-5, -7), (3, 2));
        assert_eq!(at(5, 4), (1, 1));
    }
}
