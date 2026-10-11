// Rust translation of source/plutovg-blend.c from PlutoVG (0.0.12, as
// SDL_ttf's external/plutovg pins it).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! The compositing of spans with solid colors, gradients and textures,
//! with every operator.
//!
//! Translation notes: pixels are read and written as native-endian
//! 32-bit words of the surfaces' byte buffers; the SSE2 `memfill32` and
//! the plain one fill the same words.

use super::canvas::Canvas;
use super::matrix::*;
use super::paint::*;
use super::rasterize::SpanBuffer;
use super::surface::Surface;
use super::*;

const COLOR_TABLE_SIZE: i32 = 1024;

/// `gradient_data_t` (with its `values` union as an array)
struct GradientData {
    matrix: Matrix,
    spread: SpreadMethod,
    colortable: [u32; COLOR_TABLE_SIZE as usize],
    /// `linear`: x1, y1, x2, y2; `radial`: cx, cy, cr, fx, fy, fr
    values: [f32; 6],
}

/// `texture_data_t`
struct TextureData<'a> {
    matrix: Matrix,
    data: &'a [u8],
    width: i32,
    height: i32,
    stride: i32,
    const_alpha: i32,
}

/// `linear_gradient_values_t`
struct LinearGradientValues {
    dx: f32,
    dy: f32,
    l: f32,
    off: f32,
}

/// `radial_gradient_values_t`
struct RadialGradientValues {
    dx: f32,
    dy: f32,
    dr: f32,
    sqrfr: f32,
    a: f32,
    extended: bool,
}

/// The pixel at byte offset `off` of `data`.
#[inline]
fn rd(data: &[u8], off: usize) -> u32 {
    match data.get(off..off + 4) {
        Some(b) => u32::from_ne_bytes([b[0], b[1], b[2], b[3]]),
        None => 0,
    }
}

/// Stores the pixel `v` at byte offset `off` of `data`.
#[inline]
fn wr(data: &mut [u8], off: usize, v: u32) {
    if let Some(b) = data.get_mut(off..off + 4) {
        b.copy_from_slice(&v.to_ne_bytes());
    }
}

/// `premultiply_color_with_opacity`
#[inline]
fn premultiply_color_with_opacity(color: &Color, opacity: f32) -> u32 {
    let alpha = lroundf(color.a * opacity * 255.0) as u32;
    let pr = lroundf(color.r * alpha as f32) as u32;
    let pg = lroundf(color.g * alpha as f32) as u32;
    let pb = lroundf(color.b * alpha as f32) as u32;
    (alpha << 24) | (pr << 16) | (pg << 8) | pb
}

/// `INTERPOLATE_PIXEL`
#[inline]
fn interpolate_pixel(mut x: u32, a: u32, y: u32, b: u32) -> u32 {
    let mut t = (x & 0xff00ff)
        .wrapping_mul(a)
        .wrapping_add((y & 0xff00ff).wrapping_mul(b));
    t = (t.wrapping_add((t >> 8) & 0xff00ff).wrapping_add(0x800080)) >> 8;
    t &= 0xff00ff;
    x = ((x >> 8) & 0xff00ff)
        .wrapping_mul(a)
        .wrapping_add(((y >> 8) & 0xff00ff).wrapping_mul(b));
    x = x.wrapping_add((x >> 8) & 0xff00ff).wrapping_add(0x800080);
    x &= 0xff00ff00;
    x |= t;
    x
}

/// `BYTE_MUL`
#[inline]
fn byte_mul(mut x: u32, a: u32) -> u32 {
    let mut t = (x & 0xff00ff).wrapping_mul(a);
    t = (t.wrapping_add((t >> 8) & 0xff00ff).wrapping_add(0x800080)) >> 8;
    t &= 0xff00ff;
    x = ((x >> 8) & 0xff00ff).wrapping_mul(a);
    x = x.wrapping_add((x >> 8) & 0xff00ff).wrapping_add(0x800080);
    x &= 0xff00ff00;
    x |= t;
    x
}

/// `plutovg_memfill32`: fills `length` pixels of `data`, from pixel
/// `start` (in 32-bit words), with `value`.
pub fn plutovg_memfill32(data: &mut [u8], start: usize, length: i32, value: u32) {
    for i in 0..length.max(0) as usize {
        wr(data, (start + i) * 4, value);
    }
}

/// `gradient_clamp`
#[inline]
fn gradient_clamp(gradient: &GradientData, mut ipos: i32) -> i32 {
    if gradient.spread == SpreadMethod::Repeat {
        ipos %= COLOR_TABLE_SIZE;
        ipos = if ipos < 0 {
            COLOR_TABLE_SIZE + ipos
        } else {
            ipos
        };
    } else if gradient.spread == SpreadMethod::Reflect {
        let limit = COLOR_TABLE_SIZE * 2;
        ipos %= limit;
        ipos = if ipos < 0 { limit + ipos } else { ipos };
        ipos = if ipos >= COLOR_TABLE_SIZE {
            limit - 1 - ipos
        } else {
            ipos
        };
    } else {
        if ipos < 0 {
            ipos = 0;
        } else if ipos >= COLOR_TABLE_SIZE {
            ipos = COLOR_TABLE_SIZE - 1;
        }
    }

    ipos
}

const FIXPT_BITS: i32 = 8;
const FIXPT_SIZE: i32 = 1 << FIXPT_BITS;

/// `gradient_pixel_fixed`
#[inline]
fn gradient_pixel_fixed(gradient: &GradientData, fixed_pos: i32) -> u32 {
    let ipos = fixed_pos.wrapping_add(FIXPT_SIZE / 2) >> FIXPT_BITS;
    gradient.colortable[gradient_clamp(gradient, ipos) as usize]
}

/// `gradient_pixel`
#[inline]
fn gradient_pixel(gradient: &GradientData, pos: f32) -> u32 {
    let ipos = cvt_f32_i32(pos * (COLOR_TABLE_SIZE - 1) as f32 + 0.5);
    gradient.colortable[gradient_clamp(gradient, ipos) as usize]
}

/// `fetch_linear_gradient`
fn fetch_linear_gradient(
    buffer: &mut [u32],
    v: &LinearGradientValues,
    gradient: &GradientData,
    y: i32,
    x: i32,
) {
    let length = buffer.len() as i32;
    let mut t;
    let mut inc;
    let rx;
    let ry;

    if v.l == 0.0 {
        t = 0.0;
        inc = 0.0;
    } else {
        rx = gradient.matrix.c * (y as f32 + 0.5)
            + gradient.matrix.a * (x as f32 + 0.5)
            + gradient.matrix.e;
        ry = gradient.matrix.d * (y as f32 + 0.5)
            + gradient.matrix.b * (x as f32 + 0.5)
            + gradient.matrix.f;
        t = v.dx * rx + v.dy * ry + v.off;
        inc = v.dx * gradient.matrix.a + v.dy * gradient.matrix.b;
        t *= (COLOR_TABLE_SIZE - 1) as f32;
        inc *= (COLOR_TABLE_SIZE - 1) as f32;
    }

    if inc > -1e-5 && inc < 1e-5 {
        let p = gradient_pixel_fixed(gradient, cvt_f32_i32(t * FIXPT_SIZE as f32));
        buffer.fill(p);
    } else {
        if t + inc * (length as f32) < (i32::MAX >> (FIXPT_BITS + 1)) as f32
            && t + inc * (length as f32) > (i32::MIN >> (FIXPT_BITS + 1)) as f32
        {
            let mut t_fixed = cvt_f32_i32(t * FIXPT_SIZE as f32);
            let inc_fixed = cvt_f32_i32(inc * FIXPT_SIZE as f32);
            for b in buffer.iter_mut() {
                *b = gradient_pixel_fixed(gradient, t_fixed);
                t_fixed = t_fixed.wrapping_add(inc_fixed);
            }
        } else {
            for b in buffer.iter_mut() {
                *b = gradient_pixel(gradient, t / COLOR_TABLE_SIZE as f32);
                t += inc;
            }
        }
    }
}

/// `fetch_radial_gradient`
fn fetch_radial_gradient(
    buffer: &mut [u32],
    v: &RadialGradientValues,
    gradient: &GradientData,
    y: i32,
    x: i32,
) {
    if v.a == 0.0 {
        buffer.fill(0);
        return;
    }

    let (fx, fy, fr) = (gradient.values[3], gradient.values[4], gradient.values[5]);

    let mut rx = gradient.matrix.c * (y as f32 + 0.5)
        + gradient.matrix.e
        + gradient.matrix.a * (x as f32 + 0.5);
    let mut ry = gradient.matrix.d * (y as f32 + 0.5)
        + gradient.matrix.f
        + gradient.matrix.b * (x as f32 + 0.5);

    rx -= fx;
    ry -= fy;

    let mut inv_a = 1.0 / (2.0 * v.a);
    let delta_rx = gradient.matrix.a;
    let delta_ry = gradient.matrix.b;

    let mut b = 2.0 * (v.dr * fr + rx * v.dx + ry * v.dy);
    let mut delta_b = 2.0 * (delta_rx * v.dx + delta_ry * v.dy);
    let b_delta_b = 2.0 * b * delta_b;
    let delta_b_delta_b = 2.0 * delta_b * delta_b;

    let bb = b * b;
    let delta_bb = delta_b * delta_b;

    b *= inv_a;
    delta_b *= inv_a;

    let rxrxryry = rx * rx + ry * ry;
    let delta_rxrxryry = delta_rx * delta_rx + delta_ry * delta_ry;
    let rx_plus_ry = 2.0 * (rx * delta_rx + ry * delta_ry);
    let delta_rx_plus_ry = 2.0 * delta_rxrxryry;

    inv_a *= inv_a;

    let mut det = (bb - 4.0 * v.a * (v.sqrfr - rxrxryry)) * inv_a;
    let mut delta_det = (b_delta_b + delta_bb + 4.0 * v.a * (rx_plus_ry + delta_rxrxryry)) * inv_a;
    let delta_delta_det = (delta_b_delta_b + 4.0 * v.a * delta_rx_plus_ry) * inv_a;

    if v.extended {
        for p in buffer.iter_mut() {
            let mut result = 0;
            if det >= 0.0 {
                let w = det.sqrt() - b;
                if fr + v.dr * w >= 0.0 {
                    result = gradient_pixel(gradient, w);
                }
            }

            *p = result;
            det += delta_det;
            delta_det += delta_delta_det;
            b += delta_b;
        }
    } else {
        for p in buffer.iter_mut() {
            *p = gradient_pixel(gradient, det.sqrt() - b);
            det += delta_det;
            delta_det += delta_delta_det;
            b += delta_b;
        }
    }
}

/* The composition functions take the destination pixels as the bytes of
`length` pixels (`dest`). */

/// `composition_solid_clear`
fn composition_solid_clear(dest: &mut [u8], length: usize, _color: u32, const_alpha: u32) {
    if const_alpha == 255 {
        plutovg_memfill32(dest, 0, length as i32, 0);
    } else {
        let ialpha = 255 - const_alpha;
        for i in 0..length {
            wr(dest, i * 4, byte_mul(rd(dest, i * 4), ialpha));
        }
    }
}

/// `composition_solid_source`
fn composition_solid_source(dest: &mut [u8], length: usize, mut color: u32, const_alpha: u32) {
    if const_alpha == 255 {
        plutovg_memfill32(dest, 0, length as i32, color);
    } else {
        let ialpha = 255 - const_alpha;
        color = byte_mul(color, const_alpha);
        for i in 0..length {
            wr(
                dest,
                i * 4,
                color.wrapping_add(byte_mul(rd(dest, i * 4), ialpha)),
            );
        }
    }
}

/// `composition_solid_destination`
fn composition_solid_destination(_dest: &mut [u8], _length: usize, _color: u32, _const_alpha: u32) {
}

/// `composition_solid_source_over`
fn composition_solid_source_over(dest: &mut [u8], length: usize, mut color: u32, const_alpha: u32) {
    if const_alpha != 255 {
        color = byte_mul(color, const_alpha);
    }
    let ialpha = 255 - plutovg_alpha(color);
    for i in 0..length {
        wr(
            dest,
            i * 4,
            color.wrapping_add(byte_mul(rd(dest, i * 4), ialpha)),
        );
    }
}

/// `composition_solid_destination_over`
fn composition_solid_destination_over(
    dest: &mut [u8],
    length: usize,
    mut color: u32,
    const_alpha: u32,
) {
    if const_alpha != 255 {
        color = byte_mul(color, const_alpha);
    }
    for i in 0..length {
        let d = rd(dest, i * 4);
        wr(
            dest,
            i * 4,
            d.wrapping_add(byte_mul(color, plutovg_alpha(!d))),
        );
    }
}

/// `composition_solid_source_in`
fn composition_solid_source_in(dest: &mut [u8], length: usize, mut color: u32, const_alpha: u32) {
    if const_alpha == 255 {
        for i in 0..length {
            wr(dest, i * 4, byte_mul(color, plutovg_alpha(rd(dest, i * 4))));
        }
    } else {
        color = byte_mul(color, const_alpha);
        let cia = 255 - const_alpha;
        for i in 0..length {
            let d = rd(dest, i * 4);
            wr(
                dest,
                i * 4,
                interpolate_pixel(color, plutovg_alpha(d), d, cia),
            );
        }
    }
}

/// `composition_solid_destination_in`
fn composition_solid_destination_in(dest: &mut [u8], length: usize, color: u32, const_alpha: u32) {
    let mut a = plutovg_alpha(color);
    if const_alpha != 255 {
        a = byte_mul(a, const_alpha).wrapping_add(255 - const_alpha);
    }
    for i in 0..length {
        wr(dest, i * 4, byte_mul(rd(dest, i * 4), a));
    }
}

/// `composition_solid_source_out`
fn composition_solid_source_out(dest: &mut [u8], length: usize, mut color: u32, const_alpha: u32) {
    if const_alpha == 255 {
        for i in 0..length {
            wr(
                dest,
                i * 4,
                byte_mul(color, plutovg_alpha(!rd(dest, i * 4))),
            );
        }
    } else {
        color = byte_mul(color, const_alpha);
        let cia = 255 - const_alpha;
        for i in 0..length {
            let d = rd(dest, i * 4);
            wr(
                dest,
                i * 4,
                interpolate_pixel(color, plutovg_alpha(!d), d, cia),
            );
        }
    }
}

/// `composition_solid_destination_out`
fn composition_solid_destination_out(dest: &mut [u8], length: usize, color: u32, const_alpha: u32) {
    let mut a = plutovg_alpha(!color);
    if const_alpha != 255 {
        a = byte_mul(a, const_alpha).wrapping_add(255 - const_alpha);
    }
    for i in 0..length {
        wr(dest, i * 4, byte_mul(rd(dest, i * 4), a));
    }
}

/// `composition_solid_source_atop`
fn composition_solid_source_atop(dest: &mut [u8], length: usize, mut color: u32, const_alpha: u32) {
    if const_alpha != 255 {
        color = byte_mul(color, const_alpha);
    }
    let sia = plutovg_alpha(!color);
    for i in 0..length {
        let d = rd(dest, i * 4);
        wr(
            dest,
            i * 4,
            interpolate_pixel(color, plutovg_alpha(d), d, sia),
        );
    }
}

/// `composition_solid_destination_atop`
fn composition_solid_destination_atop(
    dest: &mut [u8],
    length: usize,
    mut color: u32,
    const_alpha: u32,
) {
    let mut a = plutovg_alpha(color);
    if const_alpha != 255 {
        color = byte_mul(color, const_alpha);
        a = plutovg_alpha(color) + 255 - const_alpha;
    }

    for i in 0..length {
        let d = rd(dest, i * 4);
        wr(
            dest,
            i * 4,
            interpolate_pixel(d, a, color, plutovg_alpha(!d)),
        );
    }
}

/// `composition_solid_xor`
fn composition_solid_xor(dest: &mut [u8], length: usize, mut color: u32, const_alpha: u32) {
    if const_alpha != 255 {
        color = byte_mul(color, const_alpha);
    }
    let sia = plutovg_alpha(!color);
    for i in 0..length {
        let d = rd(dest, i * 4);
        wr(
            dest,
            i * 4,
            interpolate_pixel(color, plutovg_alpha(!d), d, sia),
        );
    }
}

/// `composition_solid_function_t`
type CompositionSolidFunction = fn(&mut [u8], usize, u32, u32);

static COMPOSITION_SOLID_TABLE: [CompositionSolidFunction; 12] = [
    composition_solid_clear,
    composition_solid_source,
    composition_solid_destination,
    composition_solid_source_over,
    composition_solid_destination_over,
    composition_solid_source_in,
    composition_solid_destination_in,
    composition_solid_source_out,
    composition_solid_destination_out,
    composition_solid_source_atop,
    composition_solid_destination_atop,
    composition_solid_xor,
];

/// `composition_clear`
fn composition_clear(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    let length = src.len();
    if const_alpha == 255 {
        plutovg_memfill32(dest, 0, length as i32, 0);
    } else {
        let ialpha = 255 - const_alpha;
        for i in 0..length {
            wr(dest, i * 4, byte_mul(rd(dest, i * 4), ialpha));
        }
    }
}

/// `composition_source`
fn composition_source(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, s) in src.iter().enumerate() {
            wr(dest, i * 4, *s);
        }
    } else {
        let ialpha = 255 - const_alpha;
        for (i, s) in src.iter().enumerate() {
            wr(
                dest,
                i * 4,
                interpolate_pixel(*s, const_alpha, rd(dest, i * 4), ialpha),
            );
        }
    }
}

/// `composition_destination`
fn composition_destination(_dest: &mut [u8], _src: &[u32], _const_alpha: u32) {}

/// `composition_source_over`
fn composition_source_over(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            if s >= 0xff000000 {
                wr(dest, i * 4, s);
            } else if s != 0 {
                wr(
                    dest,
                    i * 4,
                    s.wrapping_add(byte_mul(rd(dest, i * 4), plutovg_alpha(!s))),
                );
            }
        }
    } else {
        for (i, &s) in src.iter().enumerate() {
            let s = byte_mul(s, const_alpha);
            wr(
                dest,
                i * 4,
                s.wrapping_add(byte_mul(rd(dest, i * 4), plutovg_alpha(!s))),
            );
        }
    }
}

/// `composition_destination_over`
fn composition_destination_over(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            let d = rd(dest, i * 4);
            wr(dest, i * 4, d.wrapping_add(byte_mul(s, plutovg_alpha(!d))));
        }
    } else {
        for (i, &s) in src.iter().enumerate() {
            let d = rd(dest, i * 4);
            let s = byte_mul(s, const_alpha);
            wr(dest, i * 4, d.wrapping_add(byte_mul(s, plutovg_alpha(!d))));
        }
    }
}

/// `composition_source_in`
fn composition_source_in(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            wr(dest, i * 4, byte_mul(s, plutovg_alpha(rd(dest, i * 4))));
        }
    } else {
        let cia = 255 - const_alpha;
        for (i, &s) in src.iter().enumerate() {
            let d = rd(dest, i * 4);
            let s = byte_mul(s, const_alpha);
            wr(dest, i * 4, interpolate_pixel(s, plutovg_alpha(d), d, cia));
        }
    }
}

/// `composition_destination_in`
fn composition_destination_in(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            wr(dest, i * 4, byte_mul(rd(dest, i * 4), plutovg_alpha(s)));
        }
    } else {
        let cia = 255 - const_alpha;
        for (i, &s) in src.iter().enumerate() {
            let a = byte_mul(plutovg_alpha(s), const_alpha).wrapping_add(cia);
            wr(dest, i * 4, byte_mul(rd(dest, i * 4), a));
        }
    }
}

/// `composition_source_out`
fn composition_source_out(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            wr(dest, i * 4, byte_mul(s, plutovg_alpha(!rd(dest, i * 4))));
        }
    } else {
        let cia = 255 - const_alpha;
        for (i, &s) in src.iter().enumerate() {
            let s = byte_mul(s, const_alpha);
            let d = rd(dest, i * 4);
            wr(dest, i * 4, interpolate_pixel(s, plutovg_alpha(!d), d, cia));
        }
    }
}

/// `composition_destination_out`
fn composition_destination_out(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            wr(dest, i * 4, byte_mul(rd(dest, i * 4), plutovg_alpha(!s)));
        }
    } else {
        let cia = 255 - const_alpha;
        for (i, &s) in src.iter().enumerate() {
            let sia = byte_mul(plutovg_alpha(!s), const_alpha).wrapping_add(cia);
            wr(dest, i * 4, byte_mul(rd(dest, i * 4), sia));
        }
    }
}

/// `composition_source_atop`
fn composition_source_atop(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            let d = rd(dest, i * 4);
            wr(
                dest,
                i * 4,
                interpolate_pixel(s, plutovg_alpha(d), d, plutovg_alpha(!s)),
            );
        }
    } else {
        for (i, &s) in src.iter().enumerate() {
            let s = byte_mul(s, const_alpha);
            let d = rd(dest, i * 4);
            wr(
                dest,
                i * 4,
                interpolate_pixel(s, plutovg_alpha(d), d, plutovg_alpha(!s)),
            );
        }
    }
}

/// `composition_destination_atop`
fn composition_destination_atop(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            let d = rd(dest, i * 4);
            wr(
                dest,
                i * 4,
                interpolate_pixel(d, plutovg_alpha(s), s, plutovg_alpha(!d)),
            );
        }
    } else {
        let cia = 255 - const_alpha;
        for (i, &s) in src.iter().enumerate() {
            let s = byte_mul(s, const_alpha);
            let d = rd(dest, i * 4);
            let a = plutovg_alpha(s) + cia;
            wr(dest, i * 4, interpolate_pixel(d, a, s, plutovg_alpha(!d)));
        }
    }
}

/// `composition_xor`
fn composition_xor(dest: &mut [u8], src: &[u32], const_alpha: u32) {
    if const_alpha == 255 {
        for (i, &s) in src.iter().enumerate() {
            let d = rd(dest, i * 4);
            wr(
                dest,
                i * 4,
                interpolate_pixel(s, plutovg_alpha(!d), d, plutovg_alpha(!s)),
            );
        }
    } else {
        for (i, &s) in src.iter().enumerate() {
            let d = rd(dest, i * 4);
            let s = byte_mul(s, const_alpha);
            wr(
                dest,
                i * 4,
                interpolate_pixel(s, plutovg_alpha(!d), d, plutovg_alpha(!s)),
            );
        }
    }
}

/// `composition_function_t`
type CompositionFunction = fn(&mut [u8], &[u32], u32);

static COMPOSITION_TABLE: [CompositionFunction; 12] = [
    composition_clear,
    composition_source,
    composition_destination,
    composition_source_over,
    composition_destination_over,
    composition_source_in,
    composition_destination_in,
    composition_source_out,
    composition_destination_out,
    composition_source_atop,
    composition_destination_atop,
    composition_xor,
];

/// The destination bytes of `len` pixels at (`x`, `y`) of `surface`
/// (C's `(uint32_t*)(surface->data + y * surface->stride) + x`), or
/// `None` if outside the buffer (never: spans are clipped to the
/// surface).
#[inline]
fn target(surface: &mut Surface, x: i32, y: i32, len: usize) -> Option<&mut [u8]> {
    let off = (y as i64) * surface.stride as i64 + 4 * x as i64;
    if off < 0 {
        return None;
    }
    let off = off as usize;
    surface
        .data
        .get_mut(off..off.checked_add(len.checked_mul(4)?)?)
}

/// `blend_solid`
fn blend_solid(surface: &mut Surface, op: Operator, solid: u32, span_buffer: &SpanBuffer) {
    let func = COMPOSITION_SOLID_TABLE[op as usize];
    for spans in &span_buffer.spans {
        let len = spans.len.max(0) as usize;
        if let Some(t) = target(surface, spans.x, spans.y, len) {
            func(t, len, solid, spans.coverage as u32);
        }
    }
}

const BUFFER_SIZE: i32 = 1024;

/// `blend_linear_gradient`
fn blend_linear_gradient(
    surface: &mut Surface,
    op: Operator,
    gradient: &GradientData,
    span_buffer: &SpanBuffer,
) {
    let func = COMPOSITION_TABLE[op as usize];
    let mut buffer = [0u32; BUFFER_SIZE as usize];

    let (x1, y1, x2, y2) = (
        gradient.values[0],
        gradient.values[1],
        gradient.values[2],
        gradient.values[3],
    );
    let mut v = LinearGradientValues {
        dx: x2 - x1,
        dy: y2 - y1,
        l: 0.0,
        off: 0.0,
    };
    v.l = v.dx * v.dx + v.dy * v.dy;
    v.off = 0.0;
    if v.l != 0.0 {
        v.dx /= v.l;
        v.dy /= v.l;
        v.off = -v.dx * x1 - v.dy * y1;
    }

    for spans in &span_buffer.spans {
        let mut length = spans.len;
        let mut x = spans.x;
        while length > 0 {
            let l = plutovg_min(length, BUFFER_SIZE);
            fetch_linear_gradient(&mut buffer[..l as usize], &v, gradient, spans.y, x);
            if let Some(t) = target(surface, x, spans.y, l as usize) {
                func(t, &buffer[..l as usize], spans.coverage as u32);
            }
            x += l;
            length -= l;
        }
    }
}

/// `blend_radial_gradient`
fn blend_radial_gradient(
    surface: &mut Surface,
    op: Operator,
    gradient: &GradientData,
    span_buffer: &SpanBuffer,
) {
    let func = COMPOSITION_TABLE[op as usize];
    let mut buffer = [0u32; BUFFER_SIZE as usize];

    let (cx, cy, cr, fx, fy, fr) = (
        gradient.values[0],
        gradient.values[1],
        gradient.values[2],
        gradient.values[3],
        gradient.values[4],
        gradient.values[5],
    );
    let mut v = RadialGradientValues {
        dx: cx - fx,
        dy: cy - fy,
        dr: cr - fr,
        sqrfr: fr * fr,
        a: 0.0,
        extended: false,
    };
    v.a = v.dr * v.dr - v.dx * v.dx - v.dy * v.dy;
    v.extended = fr != 0.0 || v.a <= 0.0;

    for spans in &span_buffer.spans {
        let mut length = spans.len;
        let mut x = spans.x;
        while length > 0 {
            let l = plutovg_min(length, BUFFER_SIZE);
            fetch_radial_gradient(&mut buffer[..l as usize], &v, gradient, spans.y, x);
            if let Some(t) = target(surface, x, spans.y, l as usize) {
                func(t, &buffer[..l as usize], spans.coverage as u32);
            }
            x += l;
            length -= l;
        }
    }
}

/// `blend_untransformed_argb`
fn blend_untransformed_argb(
    surface: &mut Surface,
    op: Operator,
    texture: &TextureData<'_>,
    span_buffer: &SpanBuffer,
) {
    let func = COMPOSITION_TABLE[op as usize];
    let mut buffer = [0u32; BUFFER_SIZE as usize];

    let image_width = texture.width;
    let image_height = texture.height;

    let xoff = cvt_f32_i32(texture.matrix.e);
    let yoff = cvt_f32_i32(texture.matrix.f);

    for spans in &span_buffer.spans {
        let mut x = spans.x;
        let mut length = spans.len;
        let mut sx = xoff.wrapping_add(x);
        let sy = yoff.wrapping_add(spans.y);
        if sy >= 0 && sy < image_height && sx < image_width {
            if sx < 0 {
                x = x.wrapping_sub(sx);
                length = length.wrapping_add(sx);
                sx = 0;
            }

            if sx.wrapping_add(length) > image_width {
                length = image_width - sx;
            }
            if length > 0 {
                let coverage = ((spans.coverage as i32 * texture.const_alpha) >> 8) as u32;
                let src_off = sy as usize * texture.stride as usize + 4 * sx as usize;
                /* (in chunks of BUFFER_SIZE: the composition is per pixel) */
                let mut done = 0;
                while done < length {
                    let l = plutovg_min(length - done, BUFFER_SIZE);
                    for (i, b) in buffer[..l as usize].iter_mut().enumerate() {
                        *b = rd(texture.data, src_off + 4 * (done as usize + i));
                    }
                    if let Some(t) = target(surface, x + done, spans.y, l as usize) {
                        func(t, &buffer[..l as usize], coverage);
                    }
                    done += l;
                }
            }
        }
    }
}

const FIXED_SCALE: i32 = 1 << 16;

/// `blend_transformed_argb`
fn blend_transformed_argb(
    surface: &mut Surface,
    op: Operator,
    texture: &TextureData<'_>,
    span_buffer: &SpanBuffer,
) {
    let func = COMPOSITION_TABLE[op as usize];
    let mut buffer = [0u32; BUFFER_SIZE as usize];

    let image_width = texture.width;
    let image_height = texture.height;

    let fdx = cvt_f32_i32(texture.matrix.a * FIXED_SCALE as f32);
    let fdy = cvt_f32_i32(texture.matrix.b * FIXED_SCALE as f32);

    for spans in &span_buffer.spans {
        let mut tx = spans.x;

        let cx = spans.x as f32 + 0.5;
        let cy = spans.y as f32 + 0.5;

        let mut x = cvt_f32_i32(
            (texture.matrix.c * cy + texture.matrix.a * cx + texture.matrix.e) * FIXED_SCALE as f32,
        );
        let mut y = cvt_f32_i32(
            (texture.matrix.d * cy + texture.matrix.b * cx + texture.matrix.f) * FIXED_SCALE as f32,
        );

        let mut length = spans.len;
        let coverage = ((spans.coverage as i32 * texture.const_alpha) >> 8) as u32;
        while length > 0 {
            let l = plutovg_min(length, BUFFER_SIZE);
            for b in buffer[..l as usize].iter_mut() {
                let px = x >> 16;
                let py = y >> 16;
                if (px < 0) || (px >= image_width) || (py < 0) || (py >= image_height) {
                    *b = 0x00000000;
                } else {
                    *b = rd(
                        texture.data,
                        py as usize * texture.stride as usize + 4 * px as usize,
                    );
                }

                x = x.wrapping_add(fdx);
                y = y.wrapping_add(fdy);
            }

            if let Some(t) = target(surface, tx, spans.y, l as usize) {
                func(t, &buffer[..l as usize], coverage);
            }
            tx += l;
            length -= l;
        }
    }
}

/// `blend_untransformed_tiled_argb`
fn blend_untransformed_tiled_argb(
    surface: &mut Surface,
    op: Operator,
    texture: &TextureData<'_>,
    span_buffer: &SpanBuffer,
) {
    let func = COMPOSITION_TABLE[op as usize];
    let mut buffer = [0u32; BUFFER_SIZE as usize];

    let image_width = texture.width;
    let image_height = texture.height;
    if image_width <= 0 || image_height <= 0 {
        return;
    }

    let mut xoff = cvt_f32_i32(texture.matrix.e).wrapping_rem(image_width);
    let mut yoff = cvt_f32_i32(texture.matrix.f).wrapping_rem(image_height);

    if xoff < 0 {
        xoff += image_width;
    }
    if yoff < 0 {
        yoff += image_height;
    }

    for spans in &span_buffer.spans {
        let mut x = spans.x;
        let mut length = spans.len;
        let mut sx = xoff.wrapping_add(spans.x).wrapping_rem(image_width);
        let mut sy = spans.y.wrapping_add(yoff).wrapping_rem(image_height);
        if sx < 0 {
            sx += image_width;
        }
        if sy < 0 {
            sy += image_height;
        }

        let coverage = ((spans.coverage as i32 * texture.const_alpha) >> 8) as u32;
        while length > 0 {
            let mut l = plutovg_min(image_width - sx, length);
            if BUFFER_SIZE < l {
                l = BUFFER_SIZE;
            }
            let src_off = sy as usize * texture.stride as usize + 4 * sx as usize;
            for (i, b) in buffer[..l as usize].iter_mut().enumerate() {
                *b = rd(texture.data, src_off + 4 * i);
            }
            if let Some(t) = target(surface, x, spans.y, l as usize) {
                func(t, &buffer[..l as usize], coverage);
            }
            x += l;
            sx += l;
            length -= l;
            if sx >= image_width {
                sx = 0;
            }
        }
    }
}

/// `blend_transformed_tiled_argb`
fn blend_transformed_tiled_argb(
    surface: &mut Surface,
    op: Operator,
    texture: &TextureData<'_>,
    span_buffer: &SpanBuffer,
) {
    let func = COMPOSITION_TABLE[op as usize];
    let mut buffer = [0u32; BUFFER_SIZE as usize];

    let image_width = texture.width;
    let image_height = texture.height;
    if image_width <= 0 || image_height <= 0 {
        return;
    }
    let scanline_offset = texture.stride / 4;

    let fdx = cvt_f32_i32(texture.matrix.a * FIXED_SCALE as f32);
    let fdy = cvt_f32_i32(texture.matrix.b * FIXED_SCALE as f32);

    for spans in &span_buffer.spans {
        let mut tx = spans.x;

        let cx = spans.x as f32 + 0.5;
        let cy = spans.y as f32 + 0.5;

        let mut x = cvt_f32_i32(
            (texture.matrix.c * cy + texture.matrix.a * cx + texture.matrix.e) * FIXED_SCALE as f32,
        );
        let mut y = cvt_f32_i32(
            (texture.matrix.d * cy + texture.matrix.b * cx + texture.matrix.f) * FIXED_SCALE as f32,
        );

        let coverage = ((spans.coverage as i32 * texture.const_alpha) >> 8) as u32;
        let mut length = spans.len;
        while length > 0 {
            let l = plutovg_min(length, BUFFER_SIZE);
            for b in buffer[..l as usize].iter_mut() {
                let mut px = x >> 16;
                let mut py = y >> 16;
                px %= image_width;
                py %= image_height;
                if px < 0 {
                    px += image_width;
                }
                if py < 0 {
                    py += image_height;
                }
                let y_offset = py * scanline_offset;

                *b = rd(texture.data, 4 * (y_offset as usize + px as usize));
                x = x.wrapping_add(fdx);
                y = y.wrapping_add(fdy);
            }

            if let Some(t) = target(surface, tx, spans.y, l as usize) {
                func(t, &buffer[..l as usize], coverage);
            }
            tx += l;
            length -= l;
        }
    }
}

/// `plutovg_blend_color`
fn plutovg_blend_color(canvas: &mut Canvas, color: &Color, span_buffer: &SpanBuffer) {
    let state = canvas.state();
    let solid = premultiply_color_with_opacity(color, state.opacity);
    let alpha = plutovg_alpha(solid);
    let op = state.op;

    if alpha == 255 && op == Operator::SrcOver {
        blend_solid(&mut canvas.surface, Operator::Src, solid, span_buffer);
    } else {
        blend_solid(&mut canvas.surface, op, solid, span_buffer);
    }
}

/// `plutovg_blend_gradient`
fn plutovg_blend_gradient(canvas: &mut Canvas, gradient: &GradientPaint, span_buffer: &SpanBuffer) {
    if gradient.stops.is_empty() {
        return;
    }
    let state = canvas.state();
    let mut data = GradientData {
        matrix: gradient.matrix,
        spread: gradient.spread,
        colortable: [0; COLOR_TABLE_SIZE as usize],
        values: [0.0; 6],
    };
    let left = data.matrix;
    plutovg_matrix_multiply(&mut data.matrix, &left, &state.matrix);
    let m = data.matrix;
    if !plutovg_matrix_invert(&m, Some(&mut data.matrix)) {
        return;
    }
    let nstops = gradient.stops.len();
    let opacity = state.opacity;
    let op = state.op;
    let mut pos: usize = 0;

    let start = &gradient.stops;
    let mut curr = &start[0];
    let mut curr_color = premultiply_color_with_opacity(&curr.color, opacity);

    data.colortable[pos] = curr_color;
    pos += 1;
    let incr = 1.0f32 / COLOR_TABLE_SIZE as f32;
    let mut fpos = 1.5 * incr;

    while fpos <= curr.offset {
        /* (C writes past the table for offsets near 1: fpos never exceeds
        the last entry's position before reaching 1) */
        if pos >= COLOR_TABLE_SIZE as usize {
            break;
        }
        data.colortable[pos] = data.colortable[pos - 1];
        pos += 1;
        fpos += incr;
    }

    for i in 0..nstops - 1 {
        curr = &start[i];
        let next = &start[i + 1];
        if curr.offset == next.offset {
            continue;
        }
        let delta = 1.0 / (next.offset - curr.offset);
        let next_color = premultiply_color_with_opacity(&next.color, opacity);
        while fpos < next.offset && pos < COLOR_TABLE_SIZE as usize {
            let t = (fpos - curr.offset) * delta;
            let dist = cvt_f32_u32(255.0 * t);
            let idist = 255u32.wrapping_sub(dist);
            data.colortable[pos] = interpolate_pixel(curr_color, idist, next_color, dist);
            pos += 1;
            fpos += incr;
        }

        curr_color = next_color;
    }

    let last = &start[nstops - 1];
    let last_color = premultiply_color_with_opacity(&last.color, opacity);
    while pos < COLOR_TABLE_SIZE as usize {
        data.colortable[pos] = last_color;
        pos += 1;
    }

    data.values = gradient.values;
    if gradient.type_ == GradientType::Linear {
        blend_linear_gradient(&mut canvas.surface, op, &data, span_buffer);
    } else {
        blend_radial_gradient(&mut canvas.surface, op, &data, span_buffer);
    }
}

/// `plutovg_blend_texture`
fn plutovg_blend_texture(canvas: &mut Canvas, texture: &TexturePaint, span_buffer: &SpanBuffer) {
    let Some(tsurface) = texture.surface.as_ref() else {
        return;
    };
    let state = canvas.state();
    let mut data = TextureData {
        matrix: texture.matrix,
        data: &tsurface.data,
        width: tsurface.width,
        height: tsurface.height,
        stride: tsurface.stride,
        const_alpha: lroundf(state.opacity * texture.opacity * 256.0) as i32,
    };
    let op = state.op;

    let left = data.matrix;
    plutovg_matrix_multiply(&mut data.matrix, &left, &state.matrix);
    let m = data.matrix;
    if !plutovg_matrix_invert(&m, Some(&mut data.matrix)) {
        return;
    }
    let matrix = &data.matrix;
    if matrix.a == 1.0 && matrix.b == 0.0 && matrix.c == 0.0 && matrix.d == 1.0 {
        if texture.type_ == TextureType::Plain {
            blend_untransformed_argb(&mut canvas.surface, op, &data, span_buffer);
        } else {
            blend_untransformed_tiled_argb(&mut canvas.surface, op, &data, span_buffer);
        }
    } else {
        if texture.type_ == TextureType::Plain {
            blend_transformed_argb(&mut canvas.surface, op, &data, span_buffer);
        } else {
            blend_transformed_tiled_argb(&mut canvas.surface, op, &data, span_buffer);
        }
    }
}

/// `plutovg_blend`
pub fn plutovg_blend(canvas: &mut Canvas, span_buffer: &SpanBuffer) {
    if span_buffer.spans.is_empty() {
        return;
    }
    let Some(paint) = canvas.state().paint.clone() else {
        let color = canvas.state().color;
        plutovg_blend_color(canvas, &color, span_buffer);
        return;
    };

    match &*paint {
        Paint::Color(color) => plutovg_blend_color(canvas, color, span_buffer),
        Paint::Gradient(gradient) => plutovg_blend_gradient(canvas, gradient, span_buffer),
        Paint::Texture(texture) => plutovg_blend_texture(canvas, texture, span_buffer),
    }
}
