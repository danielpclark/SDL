// Rust translation of src/render/software/SDL_draw.h, SDL_drawpoint.c,
// SDL_drawline.c, SDL_blendpoint.c, SDL_blendline.c and SDL_blendfillrect.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Points, lines and filled rectangles drawn into surfaces, opaque or with
//! a blend mode.
//!
//! Upstream expands one macro per (pixel format, blend mode, primitive)
//! triple. Here a pixel [`Codec`] (RGB555, RGB565, XRGB8888, ARGB8888, or a
//! generic RGB/RGBA layout) is combined with a blend [`Op`]; the pixel
//! arithmetic is upstream's. The source color is premultiplied by alpha for
//! the blend and add modes before drawing, as upstream does.
//!
//! Anti-aliased lines (`AA_LINES`) are not enabled upstream, so lines are
//! drawn with Bresenham's algorithm.

use crate::error::{Error, Result};
use crate::video::blit::{
    argb8888_from_rgba, pixel_from_rgb, pixel_from_rgba, rd16, rd32, rgb555_from_rgb,
    rgb565_from_rgb, rgb_from_pixel, rgb_from_rgb555, rgb_from_rgb565, rgb_from_xrgb8888,
    rgba_from_argb8888, rgba_from_pixel, wr16, wr32, xrgb8888_from_rgb,
};
use crate::video::pixels::PixelFormatDetails;
use crate::video::rect::{Point, Rect};
use crate::video::{BlendMode, Surface};

/// `DRAW_MUL()`.
#[inline(always)]
pub(crate) fn draw_mul(a: u32, b: u32) -> u32 {
    (a * b) / 255
}

/// How a pixel is read and written.
#[derive(Clone, Copy)]
enum Codec<'f> {
    Rgb555,
    Rgb565,
    Xrgb8888,
    Argb8888,
    /// `PIXEL_FROM_RGB()` / `RGB_FROM_PIXEL()` on 2- or 4-byte pixels.
    Rgb(&'f PixelFormatDetails, usize),
    /// `PIXEL_FROM_RGBA()` / `RGBA_FROM_PIXEL()` on 4-byte pixels.
    Rgba(&'f PixelFormatDetails),
}

impl<'f> Codec<'f> {
    /// The generic layouts expand channels through `SDL_expand_byte`, which
    /// only has entries for up to 8 bits.
    fn generic(fmt: &'f PixelFormatDetails, alpha: bool) -> Result<Codec<'f>> {
        let bpp = fmt.bytes_per_pixel as usize;
        // FIXME (upstream): wider channels (the 10-bit formats) index past
        // the end of SDL_expand_byte; they are reported as unsupported here.
        if fmt.Rbits > 8 || fmt.Gbits > 8 || fmt.Bbits > 8 || fmt.Abits > 8 {
            return Err(Error::unsupported());
        }
        match (alpha, bpp) {
            (false, 2 | 4) => Ok(Codec::Rgb(fmt, bpp)),
            (true, 4) => Ok(Codec::Rgba(fmt)),
            _ => Err(Error::unsupported()),
        }
    }

    fn bpp(&self) -> usize {
        match self {
            Codec::Rgb555 | Codec::Rgb565 => 2,
            Codec::Xrgb8888 | Codec::Argb8888 | Codec::Rgba(_) => 4,
            Codec::Rgb(_, bpp) => *bpp,
        }
    }

    /// The pixel's color; the alpha only for the layouts that have one.
    #[inline(always)]
    fn read(&self, px: &[u8], i: usize) -> (u32, u32, u32, Option<u32>) {
        match *self {
            Codec::Rgb555 => {
                let (r, g, b) = rgb_from_rgb555(rd16(px, i));
                (r, g, b, None)
            }
            Codec::Rgb565 => {
                let (r, g, b) = rgb_from_rgb565(rd16(px, i));
                (r, g, b, None)
            }
            Codec::Xrgb8888 => {
                let (r, g, b) = rgb_from_xrgb8888(rd32(px, i));
                (r, g, b, None)
            }
            Codec::Argb8888 => {
                let (r, g, b, a) = rgba_from_argb8888(rd32(px, i));
                (r, g, b, Some(a))
            }
            Codec::Rgb(fmt, bpp) => {
                let p = if bpp == 2 { rd16(px, i) } else { rd32(px, i) };
                let (r, g, b) = rgb_from_pixel(p, fmt);
                (r, g, b, None)
            }
            Codec::Rgba(fmt) => {
                let (r, g, b, a) = rgba_from_pixel(rd32(px, i), fmt);
                (r, g, b, Some(a))
            }
        }
    }

    /// Store a color, truncated to the pixel type like upstream.
    #[inline(always)]
    fn write(&self, px: &mut [u8], i: usize, r: u32, g: u32, b: u32, a: u32) {
        match *self {
            Codec::Rgb555 => wr16(px, i, rgb555_from_rgb(r, g, b)),
            Codec::Rgb565 => wr16(px, i, rgb565_from_rgb(r, g, b)),
            Codec::Xrgb8888 => wr32(px, i, xrgb8888_from_rgb(r, g, b)),
            Codec::Argb8888 => wr32(px, i, argb8888_from_rgba(r, g, b, a)),
            Codec::Rgb(fmt, bpp) => {
                let p = pixel_from_rgb(fmt, r, g, b);
                if bpp == 2 {
                    wr16(px, i, p);
                } else {
                    wr32(px, i, p);
                }
            }
            Codec::Rgba(fmt) => wr32(px, i, pixel_from_rgba(fmt, r, g, b, a)),
        }
    }
}

/// The blend arithmetic: `DRAW_SETPIXEL`, `DRAW_SETPIXEL_BLEND`,
/// `DRAW_SETPIXEL_BLEND_CLAMPED`, `DRAW_SETPIXEL_ADD`, `DRAW_SETPIXEL_MOD`
/// and `DRAW_SETPIXEL_MUL`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Set,
    Blend,
    BlendClamped,
    Add,
    Mod,
    Mul,
}

impl Op {
    fn for_mode(mode: BlendMode) -> Op {
        match mode {
            BlendMode::BLEND => Op::Blend,
            BlendMode::BLEND_PREMULTIPLIED => Op::BlendClamped,
            BlendMode::ADD | BlendMode::ADD_PREMULTIPLIED => Op::Add,
            BlendMode::MOD => Op::Mod,
            BlendMode::MUL => Op::Mul,
            _ => Op::Set,
        }
    }
}

/// The (premultiplied for blend and add) source color and `inva`.
#[derive(Clone, Copy)]
struct Src {
    r: u32,
    g: u32,
    b: u32,
    a: u32,
    inva: u32,
}

/// Apply `op` to the pixel at byte `i`.
#[inline(always)]
fn set_pixel(op: Op, codec: Codec<'_>, px: &mut [u8], i: usize, s: Src) {
    let Src { r, g, b, a, inva } = s;
    if op == Op::Set {
        codec.write(px, i, r, g, b, a);
        return;
    }
    let (mut sr, mut sg, mut sb, read_a) = codec.read(px, i);
    // The layouts without alpha leave `sa` at 0xFF (blend) or unused.
    let mut sa = read_a.unwrap_or(0xFF);
    match op {
        Op::Blend => {
            sr = draw_mul(inva, sr) + r;
            sg = draw_mul(inva, sg) + g;
            sb = draw_mul(inva, sb) + b;
            sa = draw_mul(inva, sa) + a;
        }
        Op::BlendClamped => {
            sr = (draw_mul(inva, sr) + r).min(0xff);
            sg = (draw_mul(inva, sg) + g).min(0xff);
            sb = (draw_mul(inva, sb) + b).min(0xff);
            sa = (draw_mul(inva, sa) + a).min(0xff);
        }
        Op::Add => {
            sr = (sr + r).min(0xff);
            sg = (sg + g).min(0xff);
            sb = (sb + b).min(0xff);
        }
        Op::Mod => {
            sr = draw_mul(sr, r);
            sg = draw_mul(sg, g);
            sb = draw_mul(sb, b);
        }
        Op::Mul => {
            sr = (draw_mul(sr, r) + draw_mul(inva, sr)).min(0xff);
            sg = (draw_mul(sg, g) + draw_mul(inva, sg)).min(0xff);
            sb = (draw_mul(sb, b) + draw_mul(inva, sb)).min(0xff);
        }
        Op::Set => unreachable!(),
    }
    codec.write(px, i, sr, sg, sb, sa);
}

/// The writable pixels of a drawing target.
fn target_pixels<'s>(dst: &'s mut Surface<'_>) -> Result<&'s mut [u8]> {
    dst.pixels
        .bytes_mut()
        .ok_or_else(|| Error::new("Surface pixels are not writable"))
}

/// The surface's clip rectangle as `(minx, maxx, miny, maxy)`.
fn clip_bounds(dst: &Surface<'_>) -> (i32, i32, i32, i32) {
    let c = dst.clip_rect;
    (c.x, c.x + c.w - 1, c.y, c.y + c.h - 1)
}

/// Whether `(x, y)` is inside the surface's clip rectangle.
fn in_clip(dst: &Surface<'_>, x: i32, y: i32) -> bool {
    let c = dst.clip_rect;
    !(x < c.x || y < c.y || x >= c.x + c.w || y >= c.y + c.h)
}

/// Store a 1-, 2- or 4-byte mapped color (`DRAW_FASTSETPIXEL`).
#[inline(always)]
fn fast_set(px: &mut [u8], i: usize, bpp: usize, color: u32) {
    match bpp {
        1 => px[i] = color as u8,
        2 => wr16(px, i, color),
        _ => wr32(px, i, color),
    }
}

/// The straight-line shapes `HLINE`, `VLINE` and `DLINE` walk in pixel
/// units, with `pitch / bytes_per_pixel` as the row step; `BLINE` (any
/// other slope) addresses pixels by `y * pitch + x * bpp`. `f` receives
/// byte offsets.
#[allow(clippy::too_many_arguments)]
fn walk_line(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    pitch: i32,
    bpp: i32,
    draw_end: bool,
    mut f: impl FnMut(usize),
) {
    let at = |index: i64| (index * bpp as i64) as usize;
    let pitch_px = (pitch / bpp) as i64;
    let (x1l, y1l, x2l, y2l) = (x1 as i64, y1 as i64, x2 as i64, y2 as i64);
    if y1 == y2 {
        // Horizontal line
        let (mut pixels, length) = if x1 <= x2 {
            (
                y1l * pitch_px + x1l,
                if draw_end { x2 - x1 + 1 } else { x2 - x1 },
            )
        } else {
            let start = y1l * pitch_px + x2l + i64::from(!draw_end);
            (start, if draw_end { x1 - x2 + 1 } else { x1 - x2 })
        };
        for _ in 0..length {
            f(at(pixels));
            pixels += 1;
        }
    } else if x1 == x2 {
        // Vertical line
        let (mut pixels, length) = if y1 <= y2 {
            (
                y1l * pitch_px + x1l,
                if draw_end { y2 - y1 + 1 } else { y2 - y1 },
            )
        } else {
            let mut start = y2l * pitch_px + x1l;
            if !draw_end {
                start += pitch_px;
            }
            (start, if draw_end { y1 - y2 + 1 } else { y1 - y2 })
        };
        for _ in 0..length {
            f(at(pixels));
            pixels += pitch_px;
        }
    } else if (x1 - x2).abs() == (y1 - y2).abs() {
        // Diagonal line
        let mut step = pitch_px;
        let (mut pixels, mut length) = if y1 <= y2 {
            step += if x1 <= x2 { 1 } else { -1 };
            (y1l * pitch_px + x1l, y2 - y1)
        } else {
            step += if x2 <= x1 { 1 } else { -1 };
            let mut start = y2l * pitch_px + x2l;
            if !draw_end {
                start += step;
            }
            (start, y1 - y2)
        };
        if draw_end {
            length += 1;
        }
        for _ in 0..length {
            f(at(pixels));
            pixels += step;
        }
    } else {
        bline(x1, y1, x2, y2, draw_end, |x, y| {
            f((y as i64 * pitch as i64 + x as i64 * bpp as i64) as usize)
        });
    }
}

/// Bresenham's line algorithm (`BLINE`).
fn bline(x1: i32, y1: i32, x2: i32, y2: i32, draw_end: bool, mut op: impl FnMut(i32, i32)) {
    let deltax = (x2 - x1).abs();
    let deltay = (y2 - y1).abs();

    let (mut numpixels, mut d, dinc1, dinc2, mut xinc1, mut xinc2, mut yinc1, mut yinc2);
    if deltax >= deltay {
        numpixels = deltax + 1;
        d = (2 * deltay) - deltax;
        dinc1 = deltay * 2;
        dinc2 = (deltay - deltax) * 2;
        xinc1 = 1;
        xinc2 = 1;
        yinc1 = 0;
        yinc2 = 1;
    } else {
        numpixels = deltay + 1;
        d = (2 * deltax) - deltay;
        dinc1 = deltax * 2;
        dinc2 = (deltax - deltay) * 2;
        xinc1 = 0;
        xinc2 = 1;
        yinc1 = 1;
        yinc2 = 1;
    }

    if x1 > x2 {
        xinc1 = -xinc1;
        xinc2 = -xinc2;
    }
    if y1 > y2 {
        yinc1 = -yinc1;
        yinc2 = -yinc2;
    }

    let mut x = x1;
    let mut y = y1;

    if !draw_end {
        numpixels -= 1;
    }
    for _ in 0..numpixels {
        op(x, y);
        if d < 0 {
            d += dinc1;
            x += xinc1;
            y += yinc1;
        } else {
            d += dinc2;
            x += xinc2;
            y += yinc2;
        }
    }
}

/// Draw a point in a mapped color. Translation of `SDL_DrawPoint()`.
pub(crate) fn draw_point(dst: &mut Surface<'_>, x: i32, y: i32, color: u32) -> Result<()> {
    // This function doesn't work on surfaces < 8 bpp
    if dst.fmt.bits_per_pixel < 8 {
        return Err(Error::new("SDL_DrawPoint(): Unsupported surface format"));
    }

    // Perform clipping
    if !in_clip(dst, x, y) {
        return Ok(());
    }

    let bpp = dst.fmt.bytes_per_pixel as usize;
    if bpp == 3 {
        return Err(Error::unsupported());
    }
    let i = y as usize * dst.pitch as usize + x as usize * bpp;
    fast_set(target_pixels(dst)?, i, bpp, color);
    Ok(())
}

/// Draw points in a mapped color. Translation of `SDL_DrawPoints()`.
pub(crate) fn draw_points(dst: &mut Surface<'_>, points: &[Point], color: u32) -> Result<()> {
    // This function doesn't work on surfaces < 8 bpp
    if dst.fmt.bits_per_pixel < 8 {
        return Err(Error::new("SDL_DrawPoints(): Unsupported surface format"));
    }

    let (minx, maxx, miny, maxy) = clip_bounds(dst);
    let bpp = dst.fmt.bytes_per_pixel as usize;
    let pitch = dst.pitch as usize;
    let px = target_pixels(dst)?;

    for p in points {
        let (x, y) = (p.x, p.y);
        if x < minx || x > maxx || y < miny || y > maxy {
            continue;
        }
        if bpp == 3 {
            return Err(Error::unsupported());
        }
        fast_set(px, y as usize * pitch + x as usize * bpp, bpp, color);
    }
    Ok(())
}

/// The opaque line drawer for a surface, if its format is supported
/// (`SDL_CalculateDrawLineFunc()`): 1-, 2- and 4-byte pixels.
fn draw_line_supported(fmt: &PixelFormatDetails) -> bool {
    match fmt.bytes_per_pixel {
        1 => fmt.bits_per_pixel >= 8,
        2 | 4 => true,
        _ => false,
    }
}

/// `SDL_DrawLine1()`, `SDL_DrawLine2()` and `SDL_DrawLine4()`.
fn draw_line_raw(
    dst: &mut Surface<'_>,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    draw_end: bool,
) -> Result<()> {
    let bpp = dst.fmt.bytes_per_pixel as i32;
    let pitch = dst.pitch;
    let px = target_pixels(dst)?;
    // The 2- and 4-byte versions look up the color as RGBA for anti-aliased
    // lines, which are disabled, so nothing else depends on the format.
    walk_line(x1, y1, x2, y2, pitch, bpp, draw_end, |i| {
        fast_set(px, i, bpp as usize, color)
    });
    Ok(())
}

/// Draw a line in a mapped color. Translation of `SDL_DrawLine()`.
pub(crate) fn draw_line(
    dst: &mut Surface<'_>,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
) -> Result<()> {
    if !draw_line_supported(&dst.fmt) {
        return Err(Error::new("SDL_DrawLine(): Unsupported surface format"));
    }

    // Perform clipping
    // FIXME: We don't actually want to clip, as it may change line slope
    let Some((a, b)) = dst
        .clip_rect
        .clip_line(Point { x: x1, y: y1 }, Point { x: x2, y: y2 })
    else {
        return Ok(());
    };

    draw_line_raw(dst, a.x, a.y, b.x, b.y, color, true)
}

/// Draw connected lines in a mapped color. Translation of `SDL_DrawLines()`.
pub(crate) fn draw_lines(dst: &mut Surface<'_>, points: &[Point], color: u32) -> Result<()> {
    if !draw_line_supported(&dst.fmt) {
        return Err(Error::new("SDL_DrawLines(): Unsupported surface format"));
    }

    for i in 1..points.len() {
        // Perform clipping
        // FIXME: We don't actually want to clip, as it may change line slope
        let Some((a, b)) = dst.clip_rect.clip_line(points[i - 1], points[i]) else {
            continue;
        };

        // Draw the end if the whole line is a single point or it was clipped
        let draw_end = (a.x == b.x && a.y == b.y) || (b.x != points[i].x || b.y != points[i].y);

        draw_line_raw(dst, a.x, a.y, b.x, b.y, color, draw_end)?;
    }
    // (upstream reads points[-1] for an empty list)
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        if first.x != last.x || first.y != last.y {
            draw_point(dst, last.x, last.y, color)?;
        }
    }
    Ok(())
}

/// The codec the point and rectangle blenders pick for a surface
/// (`SDL_BlendPoint()`'s and `SDL_BlendFillRect()`'s dispatch, on the bits
/// per pixel and masks).
fn fill_codec(fmt: &PixelFormatDetails) -> Result<Codec<'_>> {
    match (fmt.bits_per_pixel, fmt.Rmask) {
        (15, 0x7C00) => return Ok(Codec::Rgb555),
        (16, 0xF800) => return Ok(Codec::Rgb565),
        (32, 0x00FF0000) => {
            return Ok(if fmt.Amask == 0 {
                Codec::Xrgb8888
            } else {
                Codec::Argb8888
            });
        }
        _ => {}
    }
    Codec::generic(fmt, fmt.Amask != 0)
}

/// The color for the blenders, premultiplied for blend and add.
fn blend_src(mode: BlendMode, r: u8, g: u8, b: u8, a: u8) -> Src {
    let (mut r, mut g, mut b, a) = (r as u32, g as u32, b as u32, a as u32);
    if mode == BlendMode::BLEND || mode == BlendMode::ADD {
        r = draw_mul(r, a);
        g = draw_mul(g, a);
        b = draw_mul(b, a);
    }
    Src {
        r,
        g,
        b,
        a,
        inva: 0xff - a,
    }
}

/// Blend a point. Translation of `SDL_BlendPoint()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_point(
    dst: &mut Surface<'_>,
    x: i32,
    y: i32,
    mode: BlendMode,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Result<()> {
    // This function doesn't work on surfaces < 8 bpp
    if dst.format.bits_per_pixel() < 8 {
        return Err(Error::new("SDL_BlendPoint(): Unsupported surface format"));
    }

    // Perform clipping
    if !in_clip(dst, x, y) {
        return Ok(());
    }

    let src = blend_src(mode, r, g, b, a);
    let fmt = dst.fmt;
    let codec = fill_codec(&fmt)?;
    let i = y as usize * dst.pitch as usize + x as usize * codec.bpp();
    set_pixel(Op::for_mode(mode), codec, target_pixels(dst)?, i, src);
    Ok(())
}

/// Blend points. Translation of `SDL_BlendPoints()`.
pub(crate) fn blend_points(
    dst: &mut Surface<'_>,
    points: &[Point],
    mode: BlendMode,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Result<()> {
    // This function doesn't work on surfaces < 8 bpp
    if dst.fmt.bits_per_pixel < 8 {
        return Err(Error::new("SDL_BlendPoints(): Unsupported surface format"));
    }

    let src = blend_src(mode, r, g, b, a);
    let fmt = dst.fmt;
    let codec = fill_codec(&fmt);
    let (minx, maxx, miny, maxy) = clip_bounds(dst);
    let pitch = dst.pitch as usize;
    let op = Op::for_mode(mode);
    let px = target_pixels(dst)?;

    let mut result = Ok(());
    for p in points {
        let (x, y) = (p.x, p.y);
        if x < minx || x > maxx || y < miny || y > maxy {
            continue;
        }
        result = match codec {
            Ok(codec) => {
                set_pixel(
                    op,
                    codec,
                    px,
                    y as usize * pitch + x as usize * codec.bpp(),
                    src,
                );
                Ok(())
            }
            Err(ref e) => Err(e.clone()),
        };
    }
    result
}

/// Blend into a clipped rectangle (`FILLRECT`).
fn fill_rect_raw(px: &mut [u8], pitch: i32, codec: Codec<'_>, rect: &Rect, op: Op, src: Src) {
    let bpp = codec.bpp() as i64;
    let pitch_px = pitch as i64 / bpp;
    for row in 0..rect.h as i64 {
        let start = (rect.y as i64 + row) * pitch_px + rect.x as i64;
        for col in 0..rect.w as i64 {
            set_pixel(op, codec, px, ((start + col) * bpp) as usize, src);
        }
    }
}

/// Blend a rectangle (or the clip rectangle). Translation of
/// `SDL_BlendFillRect()`.
pub(crate) fn blend_fill_rect(
    dst: &mut Surface<'_>,
    rect: Option<&Rect>,
    mode: BlendMode,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Result<()> {
    // This function doesn't work on surfaces < 8 bpp
    if dst.format.bits_per_pixel() < 8 {
        return Err(Error::new(
            "SDL_BlendFillRect(): Unsupported surface format",
        ));
    }

    // If 'rect' == NULL, then fill the whole surface
    let rect = match rect {
        Some(rect) => {
            // Perform clipping
            let mut clipped = Rect::default();
            if !rect.intersect_into(&dst.clip_rect, &mut clipped) {
                return Ok(());
            }
            clipped
        }
        None => dst.clip_rect,
    };

    let src = blend_src(mode, r, g, b, a);
    let fmt = dst.fmt;
    let codec = fill_codec(&fmt)?;
    let pitch = dst.pitch;
    fill_rect_raw(
        target_pixels(dst)?,
        pitch,
        codec,
        &rect,
        Op::for_mode(mode),
        src,
    );
    Ok(())
}

/// Blend rectangles. Translation of `SDL_BlendFillRects()`.
pub(crate) fn blend_fill_rects(
    dst: &mut Surface<'_>,
    rects: &[Rect],
    mode: BlendMode,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Result<()> {
    // This function doesn't work on surfaces < 8 bpp
    if dst.fmt.bits_per_pixel < 8 {
        return Err(Error::new(
            "SDL_BlendFillRects(): Unsupported surface format",
        ));
    }

    let src = blend_src(mode, r, g, b, a);
    let fmt = dst.fmt;
    let codec = fill_codec(&fmt);
    let clip = dst.clip_rect;
    let pitch = dst.pitch;
    let op = Op::for_mode(mode);
    let px = target_pixels(dst)?;

    let mut result = Ok(());
    for r in rects {
        // Perform clipping
        let mut rect = Rect::default();
        if !r.intersect_into(&clip, &mut rect) {
            continue;
        }
        result = match codec {
            Ok(codec) => {
                fill_rect_raw(px, pitch, codec, &rect, op, src);
                Ok(())
            }
            Err(ref e) => Err(e.clone()),
        };
    }
    result
}

/// The codec the line blender picks (`SDL_CalculateBlendLineFunc()`, on the
/// bytes per pixel and masks).
fn line_codec(fmt: &PixelFormatDetails) -> Option<Result<Codec<'_>>> {
    match fmt.bytes_per_pixel {
        2 => Some(match fmt.Rmask {
            0x7C00 => Ok(Codec::Rgb555),
            0xF800 => Ok(Codec::Rgb565),
            _ => Codec::generic(fmt, false),
        }),
        4 => Some(if fmt.Rmask == 0x00FF0000 {
            Ok(if fmt.Amask != 0 {
                Codec::Argb8888
            } else {
                Codec::Xrgb8888
            })
        } else {
            Codec::generic(fmt, fmt.Amask != 0)
        }),
        _ => None,
    }
}

/// `SDL_BlendLine_RGB2()` and friends: one blended line, already clipped.
#[allow(clippy::too_many_arguments)]
fn blend_line_raw(
    px: &mut [u8],
    pitch: i32,
    codec: Codec<'_>,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    mode: BlendMode,
    src: Src,
    draw_end: bool,
) {
    let op = Op::for_mode(mode);
    // (`inva` is `a ^ 0xff` here, the same as `0xff - a`)
    walk_line(x1, y1, x2, y2, pitch, codec.bpp() as i32, draw_end, |i| {
        set_pixel(op, codec, px, i, src)
    });
}

/// Blend a line. Translation of `SDL_BlendLine()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_line(
    dst: &mut Surface<'_>,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    mode: BlendMode,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Result<()> {
    let fmt = dst.fmt;
    let Some(codec) = line_codec(&fmt) else {
        return Err(Error::new("SDL_BlendLine(): Unsupported surface format"));
    };

    // Perform clipping
    // FIXME: We don't actually want to clip, as it may change line slope
    let Some((p1, p2)) = dst
        .clip_rect
        .clip_line(Point { x: x1, y: y1 }, Point { x: x2, y: y2 })
    else {
        return Ok(());
    };

    let codec = codec?;
    let pitch = dst.pitch;
    let src = blend_src(mode, r, g, b, a);
    blend_line_raw(
        target_pixels(dst)?,
        pitch,
        codec,
        p1.x,
        p1.y,
        p2.x,
        p2.y,
        mode,
        src,
        true,
    );
    Ok(())
}

/// Blend connected lines. Translation of `SDL_BlendLines()`.
pub(crate) fn blend_lines(
    dst: &mut Surface<'_>,
    points: &[Point],
    mode: BlendMode,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> Result<()> {
    let fmt = dst.fmt;
    let Some(codec) = line_codec(&fmt) else {
        return Err(Error::new("SDL_BlendLines(): Unsupported surface format"));
    };

    let clip = dst.clip_rect;
    let pitch = dst.pitch;
    let src = blend_src(mode, r, g, b, a);
    for i in 1..points.len() {
        // Perform clipping
        // FIXME: We don't actually want to clip, as it may change line slope
        let Some((p1, p2)) = clip.clip_line(points[i - 1], points[i]) else {
            continue;
        };

        // Draw the end if it was clipped
        let draw_end = p2.x != points[i].x || p2.y != points[i].y;

        let codec = codec.clone()?;
        blend_line_raw(
            target_pixels(dst)?,
            pitch,
            codec,
            p1.x,
            p1.y,
            p2.x,
            p2.y,
            mode,
            src,
            draw_end,
        );
    }
    // (upstream reads points[-1] for an empty list)
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        if first.x != last.x || first.y != last.y {
            // (upstream ignores this result)
            let _ = blend_point(dst, last.x, last.y, mode, r, g, b, a);
        }
    }
    Ok(())
}
