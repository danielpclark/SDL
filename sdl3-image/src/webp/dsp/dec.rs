// Rust translation of src/dsp/dec.c and src/dsp/dec_clip_tables.c from
// libwebp (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the plain-C functions.
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Speed-critical decoding functions: the inverse transforms, the intra
//! predictors and the in-loop filters.
//!
//! The blocks are addressed as a buffer and the offset of their top-left
//! sample (`dst` upstream), the predictors reading the samples above and
//! to the left of it. The clipping tables of dec_clip_tables.c
//! (`VP8ksclip1`, `VP8ksclip2`, `VP8kclip1`, `VP8kabs0`) are the clamps
//! they tabulate. The dithering combiner is not translated: dithering is a
//! decoding option SDL_image doesn't set.

use crate::webp::dec::{B_DC_PRED_NOLEFT, B_DC_PRED_NOTOP, B_DC_PRED_NOTOPLEFT};
use crate::webp::dsp::BPS;

/// `VP8ksclip1[v]`: clips [-1020, 1020] to [-128, 127].
fn sclip1(v: i32) -> i32 {
    v.clamp(-128, 127)
}

/// `VP8ksclip2[v]`: clips [-112, 112] to [-16, 15].
fn sclip2(v: i32) -> i32 {
    v.clamp(-16, 15)
}

/// `VP8kclip1[v]`: clips [-255,511] to [0,255].
fn clip1(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// `VP8kabs0[v]`: abs(x) for x in [-255,255].
fn abs0(v: i32) -> i32 {
    v.abs()
}

/// Translation of `clip_8b()`.
fn clip_8b(v: i32) -> u8 {
    if v & !0xff == 0 {
        v as u8
    } else if v < 0 {
        0
    } else {
        255
    }
}

/// The index `k` samples away from `off`.
fn at(off: usize, k: isize) -> usize {
    (off as isize + k) as usize
}

//------------------------------------------------------------------------------
// Transforms (Paragraph 14.4)

/// Translation of the `STORE()` macro.
fn store(dst: &mut [u8], off: usize, x: usize, y: usize, v: i32) {
    let i = off + x + y * BPS;
    dst[i] = clip_8b(dst[i] as i32 + (v >> 3));
}

/// Translation of the `STORE2()` macro.
fn store2(dst: &mut [u8], off: usize, y: usize, dc: i32, d: i32, c: i32) {
    store(dst, off, 0, y, dc + d);
    store(dst, off, 1, y, dc + c);
    store(dst, off, 2, y, dc - c);
    store(dst, off, 3, y, dc - d);
}

fn mul1(a: i32) -> i32 {
    ((a * 20091) >> 16) + a
}

fn mul2(a: i32) -> i32 {
    (a * 35468) >> 16
}

/// Translation of `TransformOne_C()`.
fn transform_one(input: &[i16], dst: &mut [u8], mut off: usize) {
    let mut c = [0i32; 4 * 4];
    for i in 0..4 {
        // vertical pass
        let inp = |k: usize| input[i + k] as i32;
        let a = inp(0) + inp(8); // [-4096, 4094]
        let b = inp(0) - inp(8); // [-4095, 4095]
        let cc = mul2(inp(4)) - mul1(inp(12)); // [-3783, 3783]
        let d = mul1(inp(4)) + mul2(inp(12)); // [-3785, 3781]
        c[4 * i] = a + d; // [-7881, 7875]
        c[4 * i + 1] = b + cc; // [-7878, 7878]
        c[4 * i + 2] = b - cc; // [-7878, 7878]
        c[4 * i + 3] = a - d; // [-7877, 7879]
    }
    // Each pass is expanding the dynamic range by ~3.85 (upper bound).
    // The exact value is (2. + (20091 + 35468) / 65536).
    // After the second pass, maximum interval is [-3794, 3794], assuming
    // an input in [-2048, 2047] interval. We then need to add a dst value
    // in the [0, 255] range.
    // In the worst case scenario, the input to clip_8b() can be as large as
    // [-60713, 60968].
    for i in 0..4 {
        // horizontal pass
        let tmp = |k: usize| c[i + k];
        let dc = tmp(0) + 4;
        let a = dc + tmp(8);
        let b = dc - tmp(8);
        let cc = mul2(tmp(4)) - mul1(tmp(12));
        let d = mul1(tmp(4)) + mul2(tmp(12));
        store(dst, off, 0, 0, a + d);
        store(dst, off, 1, 0, b + cc);
        store(dst, off, 2, 0, b - cc);
        store(dst, off, 3, 0, a - d);
        off += BPS;
    }
}

/// Simplified transform when only in[0], in[1] and in[4] are non-zero.
/// Translation of `TransformAC3_C()` (`VP8TransformAC3`).
pub(crate) fn transform_ac3(input: &[i16], dst: &mut [u8], off: usize) {
    let a = input[0] as i32 + 4;
    let c4 = mul2(input[4] as i32);
    let d4 = mul1(input[4] as i32);
    let c1 = mul2(input[1] as i32);
    let d1 = mul1(input[1] as i32);
    store2(dst, off, 0, a + d4, d1, c1);
    store2(dst, off, 1, a + c4, d1, c1);
    store2(dst, off, 2, a - c4, d1, c1);
    store2(dst, off, 3, a - d4, d1, c1);
}

/// Translation of `TransformTwo_C()` (`VP8Transform`).
pub(crate) fn transform(input: &[i16], dst: &mut [u8], off: usize, do_two: bool) {
    transform_one(input, dst, off);
    if do_two {
        transform_one(&input[16..], dst, off + 4);
    }
}

/// Translation of `TransformUV_C()` (`VP8TransformUV`).
pub(crate) fn transform_uv(input: &[i16], dst: &mut [u8], off: usize) {
    transform(input, dst, off, true);
    transform(&input[2 * 16..], dst, off + 4 * BPS, true);
}

/// Translation of `TransformDC_C()` (`VP8TransformDC`).
pub(crate) fn transform_dc(input: &[i16], dst: &mut [u8], off: usize) {
    let dc = input[0] as i32 + 4;
    for j in 0..4 {
        for i in 0..4 {
            store(dst, off, i, j, dc);
        }
    }
}

/// Translation of `TransformDCUV_C()` (`VP8TransformDCUV`).
pub(crate) fn transform_dcuv(input: &[i16], dst: &mut [u8], off: usize) {
    if input[0] != 0 {
        transform_dc(input, dst, off);
    }
    if input[16] != 0 {
        transform_dc(&input[16..], dst, off + 4);
    }
    if input[2 * 16] != 0 {
        transform_dc(&input[2 * 16..], dst, off + 4 * BPS);
    }
    if input[3 * 16] != 0 {
        transform_dc(&input[3 * 16..], dst, off + 4 * BPS + 4);
    }
}

//------------------------------------------------------------------------------
// Paragraph 14.3

/// Translation of `TransformWHT_C()` (`VP8TransformWHT`): the 16 DC
/// coefficients, written every 16 entries of `out`.
pub(crate) fn transform_wht(input: &[i16], out: &mut [i16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let inp = |k: usize| input[k + i] as i32;
        let a0 = inp(0) + inp(12);
        let a1 = inp(4) + inp(8);
        let a2 = inp(4) - inp(8);
        let a3 = inp(0) - inp(12);
        tmp[i] = a0 + a1;
        tmp[8 + i] = a0 - a1;
        tmp[4 + i] = a3 + a2;
        tmp[12 + i] = a3 - a2;
    }
    for i in 0..4 {
        let dc = tmp[i * 4] + 3; // w/ rounder
        let a0 = dc + tmp[3 + i * 4];
        let a1 = tmp[1 + i * 4] + tmp[2 + i * 4];
        let a2 = tmp[1 + i * 4] - tmp[2 + i * 4];
        let a3 = dc - tmp[3 + i * 4];
        let o = 64 * i;
        out[o] = ((a0 + a1) >> 3) as i16;
        out[o + 16] = ((a3 + a2) >> 3) as i16;
        out[o + 32] = ((a0 - a1) >> 3) as i16;
        out[o + 48] = ((a3 - a2) >> 3) as i16;
    }
}

//------------------------------------------------------------------------------
// Intra predictions

/// Translation of `TrueMotion()`.
fn true_motion(dst: &mut [u8], mut off: usize, size: usize) {
    let top = off - BPS;
    let top_left = dst[top - 1] as i32;
    for _ in 0..size {
        let left = dst[off - 1] as i32;
        for x in 0..size {
            dst[off + x] = clip1(dst[top + x] as i32 + left - top_left);
        }
        off += BPS;
    }
}

//------------------------------------------------------------------------------
// 16x16

/// Translation of `VE16_C()`: vertical.
fn ve16(dst: &mut [u8], off: usize) {
    for j in 0..16 {
        dst.copy_within(off - BPS..off - BPS + 16, off + j * BPS);
    }
}

/// Translation of `HE16_C()`: horizontal.
fn he16(dst: &mut [u8], mut off: usize) {
    for _ in 0..16 {
        let v = dst[off - 1];
        dst[off..off + 16].fill(v);
        off += BPS;
    }
}

/// Translation of `Put16()`.
fn put16(v: i32, dst: &mut [u8], off: usize) {
    for j in 0..16 {
        dst[off + j * BPS..off + j * BPS + 16].fill(v as u8);
    }
}

/// Translation of `DC16_C()`: DC.
fn dc16(dst: &mut [u8], off: usize) {
    let mut dc = 16;
    for j in 0..16 {
        dc += dst[off - 1 + j * BPS] as i32 + dst[off + j - BPS] as i32;
    }
    put16(dc >> 5, dst, off);
}

/// DC with top samples not available. Translation of `DC16NoTop_C()`.
fn dc16_no_top(dst: &mut [u8], off: usize) {
    let mut dc = 8;
    for j in 0..16 {
        dc += dst[off - 1 + j * BPS] as i32;
    }
    put16(dc >> 4, dst, off);
}

/// DC with left samples not available. Translation of `DC16NoLeft_C()`.
fn dc16_no_left(dst: &mut [u8], off: usize) {
    let mut dc = 8;
    for i in 0..16 {
        dc += dst[off + i - BPS] as i32;
    }
    put16(dc >> 4, dst, off);
}

/// DC with no top and left samples. Translation of `DC16NoTopLeft_C()`.
fn dc16_no_top_left(dst: &mut [u8], off: usize) {
    put16(0x80, dst, off);
}

/// The 16x16 luma predictors. Translation of `VP8PredLuma16[mode](dst)`.
pub(crate) fn pred_luma16(mode: i32, dst: &mut [u8], off: usize) {
    match mode {
        0 => dc16(dst, off),
        1 => true_motion(dst, off, 16),
        2 => ve16(dst, off),
        3 => he16(dst, off),
        4 => dc16_no_top(dst, off),
        5 => dc16_no_left(dst, off),
        _ => dc16_no_top_left(dst, off),
    }
}

//------------------------------------------------------------------------------
// 4x4

fn avg3(a: i32, b: i32, c: i32) -> u8 {
    ((a + 2 * b + c + 2) >> 2) as u8
}

fn avg2(a: i32, b: i32) -> u8 {
    ((a + b + 1) >> 1) as u8
}

/// The sample at (x, y) of the block. Translation of the `DST()` macro.
fn dst_at(off: usize, x: usize, y: usize) -> usize {
    off + x + y * BPS
}

/// Translation of `VE4_C()`: vertical.
fn ve4(dst: &mut [u8], off: usize) {
    let top = off - BPS;
    let t = |k: isize| dst[at(top, k)] as i32;
    let vals = [
        avg3(t(-1), t(0), t(1)),
        avg3(t(0), t(1), t(2)),
        avg3(t(1), t(2), t(3)),
        avg3(t(2), t(3), t(4)),
    ];
    for i in 0..4 {
        dst[off + i * BPS..off + i * BPS + 4].copy_from_slice(&vals);
    }
}

/// Translation of `HE4_C()`: horizontal.
fn he4(dst: &mut [u8], off: usize) {
    let a = dst[off - 1 - BPS] as i32;
    let b = dst[off - 1] as i32;
    let c = dst[off - 1 + BPS] as i32;
    let d = dst[off - 1 + 2 * BPS] as i32;
    let e = dst[off - 1 + 3 * BPS] as i32;
    dst[off..off + 4].fill(avg3(a, b, c));
    dst[off + BPS..off + BPS + 4].fill(avg3(b, c, d));
    dst[off + 2 * BPS..off + 2 * BPS + 4].fill(avg3(c, d, e));
    dst[off + 3 * BPS..off + 3 * BPS + 4].fill(avg3(d, e, e));
}

/// Translation of `DC4_C()`: DC.
fn dc4(dst: &mut [u8], off: usize) {
    let mut dc: u32 = 4;
    for i in 0..4 {
        dc += dst[off + i - BPS] as u32 + dst[off - 1 + i * BPS] as u32;
    }
    dc >>= 3;
    for i in 0..4 {
        dst[off + i * BPS..off + i * BPS + 4].fill(dc as u8);
    }
}

/// The left column (I, J, K, L), the top-left sample (X) and the top row
/// (A to H) of a 4x4 block.
struct Edges {
    i: i32,
    j: i32,
    k: i32,
    l: i32,
    x: i32,
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    e: i32,
    f: i32,
    g: i32,
    h: i32,
}

fn edges(dst: &[u8], off: usize) -> Edges {
    let top = |k: usize| dst[off + k - BPS] as i32;
    Edges {
        i: dst[off - 1] as i32,
        j: dst[off - 1 + BPS] as i32,
        k: dst[off - 1 + 2 * BPS] as i32,
        l: dst[off - 1 + 3 * BPS] as i32,
        x: dst[off - 1 - BPS] as i32,
        a: top(0),
        b: top(1),
        c: top(2),
        d: top(3),
        e: top(4),
        f: top(5),
        g: top(6),
        h: top(7),
    }
}

/// Translation of `RD4_C()`: Down-right.
fn rd4(dst: &mut [u8], off: usize) {
    let Edges {
        i, j, k, l, x, a, b, c, d, ..
    } = edges(dst, off);
    let mut put = |xy: &[(usize, usize)], v: u8| {
        for &(px, py) in xy {
            dst[dst_at(off, px, py)] = v;
        }
    };
    put(&[(0, 3)], avg3(j, k, l));
    put(&[(1, 3), (0, 2)], avg3(i, j, k));
    put(&[(2, 3), (1, 2), (0, 1)], avg3(x, i, j));
    put(&[(3, 3), (2, 2), (1, 1), (0, 0)], avg3(a, x, i));
    put(&[(3, 2), (2, 1), (1, 0)], avg3(b, a, x));
    put(&[(3, 1), (2, 0)], avg3(c, b, a));
    put(&[(3, 0)], avg3(d, c, b));
}

/// Translation of `LD4_C()`: Down-Left.
fn ld4(dst: &mut [u8], off: usize) {
    let Edges {
        a, b, c, d, e, f, g, h, ..
    } = edges(dst, off);
    let mut put = |xy: &[(usize, usize)], v: u8| {
        for &(px, py) in xy {
            dst[dst_at(off, px, py)] = v;
        }
    };
    put(&[(0, 0)], avg3(a, b, c));
    put(&[(1, 0), (0, 1)], avg3(b, c, d));
    put(&[(2, 0), (1, 1), (0, 2)], avg3(c, d, e));
    put(&[(3, 0), (2, 1), (1, 2), (0, 3)], avg3(d, e, f));
    put(&[(3, 1), (2, 2), (1, 3)], avg3(e, f, g));
    put(&[(3, 2), (2, 3)], avg3(f, g, h));
    put(&[(3, 3)], avg3(g, h, h));
}

/// Translation of `VR4_C()`: Vertical-Right.
fn vr4(dst: &mut [u8], off: usize) {
    let Edges {
        i, j, k, x, a, b, c, d, ..
    } = edges(dst, off);
    let mut put = |xy: &[(usize, usize)], v: u8| {
        for &(px, py) in xy {
            dst[dst_at(off, px, py)] = v;
        }
    };
    put(&[(0, 0), (1, 2)], avg2(x, a));
    put(&[(1, 0), (2, 2)], avg2(a, b));
    put(&[(2, 0), (3, 2)], avg2(b, c));
    put(&[(3, 0)], avg2(c, d));

    put(&[(0, 3)], avg3(k, j, i));
    put(&[(0, 2)], avg3(j, i, x));
    put(&[(0, 1), (1, 3)], avg3(i, x, a));
    put(&[(1, 1), (2, 3)], avg3(x, a, b));
    put(&[(2, 1), (3, 3)], avg3(a, b, c));
    put(&[(3, 1)], avg3(b, c, d));
}

/// Translation of `VL4_C()`: Vertical-Left.
fn vl4(dst: &mut [u8], off: usize) {
    let Edges {
        a, b, c, d, e, f, g, h, ..
    } = edges(dst, off);
    let mut put = |xy: &[(usize, usize)], v: u8| {
        for &(px, py) in xy {
            dst[dst_at(off, px, py)] = v;
        }
    };
    put(&[(0, 0)], avg2(a, b));
    put(&[(1, 0), (0, 2)], avg2(b, c));
    put(&[(2, 0), (1, 2)], avg2(c, d));
    put(&[(3, 0), (2, 2)], avg2(d, e));

    put(&[(0, 1)], avg3(a, b, c));
    put(&[(1, 1), (0, 3)], avg3(b, c, d));
    put(&[(2, 1), (1, 3)], avg3(c, d, e));
    put(&[(3, 1), (2, 3)], avg3(d, e, f));
    put(&[(3, 2)], avg3(e, f, g));
    put(&[(3, 3)], avg3(f, g, h));
}

/// Translation of `HU4_C()`: Horizontal-Up.
fn hu4(dst: &mut [u8], off: usize) {
    let Edges { i, j, k, l, .. } = edges(dst, off);
    let mut put = |xy: &[(usize, usize)], v: u8| {
        for &(px, py) in xy {
            dst[dst_at(off, px, py)] = v;
        }
    };
    put(&[(0, 0)], avg2(i, j));
    put(&[(2, 0), (0, 1)], avg2(j, k));
    put(&[(2, 1), (0, 2)], avg2(k, l));
    put(&[(1, 0)], avg3(i, j, k));
    put(&[(3, 0), (1, 1)], avg3(j, k, l));
    put(&[(3, 1), (1, 2)], avg3(k, l, l));
    put(&[(3, 2), (2, 2), (0, 3), (1, 3), (2, 3), (3, 3)], l as u8);
}

/// Translation of `HD4_C()`: Horizontal-Down.
fn hd4(dst: &mut [u8], off: usize) {
    let Edges {
        i, j, k, l, x, a, b, c, ..
    } = edges(dst, off);
    let mut put = |xy: &[(usize, usize)], v: u8| {
        for &(px, py) in xy {
            dst[dst_at(off, px, py)] = v;
        }
    };
    put(&[(0, 0), (2, 1)], avg2(i, x));
    put(&[(0, 1), (2, 2)], avg2(j, i));
    put(&[(0, 2), (2, 3)], avg2(k, j));
    put(&[(0, 3)], avg2(l, k));

    put(&[(3, 0)], avg3(a, b, c));
    put(&[(2, 0)], avg3(x, a, b));
    put(&[(1, 0), (3, 1)], avg3(i, x, a));
    put(&[(1, 1), (3, 2)], avg3(j, i, x));
    put(&[(1, 2), (3, 3)], avg3(k, j, i));
    put(&[(1, 3)], avg3(l, k, j));
}

/// The 4x4 luma predictors. Translation of `VP8PredLuma4[mode](dst)`.
pub(crate) fn pred_luma4(mode: u8, dst: &mut [u8], off: usize) {
    match mode {
        0 => dc4(dst, off),
        1 => true_motion(dst, off, 4),
        2 => ve4(dst, off),
        3 => he4(dst, off),
        4 => rd4(dst, off),
        5 => vr4(dst, off),
        6 => ld4(dst, off),
        7 => vl4(dst, off),
        8 => hd4(dst, off),
        _ => hu4(dst, off),
    }
}

//------------------------------------------------------------------------------
// Chroma

/// Translation of `VE8uv_C()`: vertical.
fn ve8uv(dst: &mut [u8], off: usize) {
    for j in 0..8 {
        dst.copy_within(off - BPS..off - BPS + 8, off + j * BPS);
    }
}

/// Translation of `HE8uv_C()`: horizontal.
fn he8uv(dst: &mut [u8], mut off: usize) {
    for _ in 0..8 {
        let v = dst[off - 1];
        dst[off..off + 8].fill(v);
        off += BPS;
    }
}

/// helper for chroma-DC predictions. Translation of `Put8x8uv()`.
fn put8x8uv(value: u8, dst: &mut [u8], off: usize) {
    for j in 0..8 {
        dst[off + j * BPS..off + j * BPS + 8].fill(value);
    }
}

/// Translation of `DC8uv_C()`: DC.
fn dc8uv(dst: &mut [u8], off: usize) {
    let mut dc0 = 8;
    for i in 0..8 {
        dc0 += dst[off + i - BPS] as i32 + dst[off - 1 + i * BPS] as i32;
    }
    put8x8uv((dc0 >> 4) as u8, dst, off);
}

/// DC with no left samples. Translation of `DC8uvNoLeft_C()`.
fn dc8uv_no_left(dst: &mut [u8], off: usize) {
    let mut dc0 = 4;
    for i in 0..8 {
        dc0 += dst[off + i - BPS] as i32;
    }
    put8x8uv((dc0 >> 3) as u8, dst, off);
}

/// DC with no top samples. Translation of `DC8uvNoTop_C()`.
fn dc8uv_no_top(dst: &mut [u8], off: usize) {
    let mut dc0 = 4;
    for i in 0..8 {
        dc0 += dst[off - 1 + i * BPS] as i32;
    }
    put8x8uv((dc0 >> 3) as u8, dst, off);
}

/// DC with nothing. Translation of `DC8uvNoTopLeft_C()`.
fn dc8uv_no_top_left(dst: &mut [u8], off: usize) {
    put8x8uv(0x80, dst, off);
}

/// The 8x8 chroma predictors. Translation of `VP8PredChroma8[mode](dst)`.
pub(crate) fn pred_chroma8(mode: i32, dst: &mut [u8], off: usize) {
    match mode {
        0 => dc8uv(dst, off),
        1 => true_motion(dst, off, 8),
        2 => ve8uv(dst, off),
        3 => he8uv(dst, off),
        B_DC_PRED_NOTOP => dc8uv_no_top(dst, off),
        B_DC_PRED_NOLEFT => dc8uv_no_left(dst, off),
        B_DC_PRED_NOTOPLEFT => dc8uv_no_top_left(dst, off),
        _ => dc8uv_no_top_left(dst, off),
    }
}

//------------------------------------------------------------------------------
// Edge filtering functions

/// 4 pixels in, 2 pixels out. Translation of `DoFilter2_C()`.
fn do_filter2(p: &mut [u8], off: usize, step: isize) {
    let p1 = p[at(off, -2 * step)] as i32;
    let p0 = p[at(off, -step)] as i32;
    let q0 = p[off] as i32;
    let q1 = p[at(off, step)] as i32;
    let a = 3 * (q0 - p0) + sclip1(p1 - q1); // in [-893,892]
    let a1 = sclip2((a + 4) >> 3); // in [-16,15]
    let a2 = sclip2((a + 3) >> 3);
    p[at(off, -step)] = clip1(p0 + a2);
    p[off] = clip1(q0 - a1);
}

/// 4 pixels in, 4 pixels out. Translation of `DoFilter4_C()`.
fn do_filter4(p: &mut [u8], off: usize, step: isize) {
    let p1 = p[at(off, -2 * step)] as i32;
    let p0 = p[at(off, -step)] as i32;
    let q0 = p[off] as i32;
    let q1 = p[at(off, step)] as i32;
    let a = 3 * (q0 - p0);
    let a1 = sclip2((a + 4) >> 3);
    let a2 = sclip2((a + 3) >> 3);
    let a3 = (a1 + 1) >> 1;
    p[at(off, -2 * step)] = clip1(p1 + a3);
    p[at(off, -step)] = clip1(p0 + a2);
    p[off] = clip1(q0 - a1);
    p[at(off, step)] = clip1(q1 - a3);
}

/// 6 pixels in, 6 pixels out. Translation of `DoFilter6_C()`.
fn do_filter6(p: &mut [u8], off: usize, step: isize) {
    let p2 = p[at(off, -3 * step)] as i32;
    let p1 = p[at(off, -2 * step)] as i32;
    let p0 = p[at(off, -step)] as i32;
    let q0 = p[off] as i32;
    let q1 = p[at(off, step)] as i32;
    let q2 = p[at(off, 2 * step)] as i32;
    let a = sclip1(3 * (q0 - p0) + sclip1(p1 - q1));
    // a is in [-128,127], a1 in [-27,27], a2 in [-18,18] and a3 in [-9,9]
    let a1 = (27 * a + 63) >> 7; // eq. to ((3 * a + 7) * 9) >> 7
    let a2 = (18 * a + 63) >> 7; // eq. to ((2 * a + 7) * 9) >> 7
    let a3 = (9 * a + 63) >> 7; // eq. to ((1 * a + 7) * 9) >> 7
    p[at(off, -3 * step)] = clip1(p2 + a3);
    p[at(off, -2 * step)] = clip1(p1 + a2);
    p[at(off, -step)] = clip1(p0 + a1);
    p[off] = clip1(q0 - a1);
    p[at(off, step)] = clip1(q1 - a2);
    p[at(off, 2 * step)] = clip1(q2 - a3);
}

/// Translation of `Hev()`.
fn hev(p: &[u8], off: usize, step: isize, thresh: i32) -> bool {
    let p1 = p[at(off, -2 * step)] as i32;
    let p0 = p[at(off, -step)] as i32;
    let q0 = p[off] as i32;
    let q1 = p[at(off, step)] as i32;
    (abs0(p1 - p0) > thresh) || (abs0(q1 - q0) > thresh)
}

/// Translation of `NeedsFilter_C()`.
fn needs_filter(p: &[u8], off: usize, step: isize, t: i32) -> bool {
    let p1 = p[at(off, -2 * step)] as i32;
    let p0 = p[at(off, -step)] as i32;
    let q0 = p[off] as i32;
    let q1 = p[at(off, step)] as i32;
    (4 * abs0(p0 - q0) + abs0(p1 - q1)) <= t
}

/// Translation of `NeedsFilter2_C()`.
fn needs_filter2(p: &[u8], off: usize, step: isize, t: i32, it: i32) -> bool {
    let s = |k: isize| p[at(off, k * step)] as i32;
    let (p3, p2, p1, p0) = (s(-4), s(-3), s(-2), s(-1));
    let (q0, q1, q2, q3) = (s(0), s(1), s(2), s(3));
    if (4 * abs0(p0 - q0) + abs0(p1 - q1)) > t {
        return false;
    }
    abs0(p3 - p2) <= it
        && abs0(p2 - p1) <= it
        && abs0(p1 - p0) <= it
        && abs0(q3 - q2) <= it
        && abs0(q2 - q1) <= it
        && abs0(q1 - q0) <= it
}

//------------------------------------------------------------------------------
// Simple In-loop filtering (Paragraph 15.2)

/// Translation of `SimpleVFilter16_C()` (`VP8SimpleVFilter16`).
pub(crate) fn simple_v_filter16(p: &mut [u8], off: usize, stride: usize, thresh: i32) {
    let thresh2 = 2 * thresh + 1;
    for i in 0..16 {
        if needs_filter(p, off + i, stride as isize, thresh2) {
            do_filter2(p, off + i, stride as isize);
        }
    }
}

/// Translation of `SimpleHFilter16_C()` (`VP8SimpleHFilter16`).
pub(crate) fn simple_h_filter16(p: &mut [u8], off: usize, stride: usize, thresh: i32) {
    let thresh2 = 2 * thresh + 1;
    for i in 0..16 {
        if needs_filter(p, off + i * stride, 1, thresh2) {
            do_filter2(p, off + i * stride, 1);
        }
    }
}

/// Translation of `SimpleVFilter16i_C()` (`VP8SimpleVFilter16i`).
pub(crate) fn simple_v_filter16i(p: &mut [u8], mut off: usize, stride: usize, thresh: i32) {
    for _ in 0..3 {
        off += 4 * stride;
        simple_v_filter16(p, off, stride, thresh);
    }
}

/// Translation of `SimpleHFilter16i_C()` (`VP8SimpleHFilter16i`).
pub(crate) fn simple_h_filter16i(p: &mut [u8], mut off: usize, stride: usize, thresh: i32) {
    for _ in 0..3 {
        off += 4;
        simple_h_filter16(p, off, stride, thresh);
    }
}

//------------------------------------------------------------------------------
// Complex In-loop filtering (Paragraph 15.3)

/// Translation of `FilterLoop26_C()`.
#[allow(clippy::too_many_arguments)]
fn filter_loop26(
    p: &mut [u8],
    mut off: usize,
    hstride: isize,
    vstride: usize,
    size: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    let thresh2 = 2 * thresh + 1;
    for _ in 0..size {
        if needs_filter2(p, off, hstride, thresh2, ithresh) {
            if hev(p, off, hstride, hev_thresh) {
                do_filter2(p, off, hstride);
            } else {
                do_filter6(p, off, hstride);
            }
        }
        off += vstride;
    }
}

/// Translation of `FilterLoop24_C()`.
#[allow(clippy::too_many_arguments)]
fn filter_loop24(
    p: &mut [u8],
    mut off: usize,
    hstride: isize,
    vstride: usize,
    size: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    let thresh2 = 2 * thresh + 1;
    for _ in 0..size {
        if needs_filter2(p, off, hstride, thresh2, ithresh) {
            if hev(p, off, hstride, hev_thresh) {
                do_filter2(p, off, hstride);
            } else {
                do_filter4(p, off, hstride);
            }
        }
        off += vstride;
    }
}

// on macroblock edges

/// Translation of `VFilter16_C()` (`VP8VFilter16`).
pub(crate) fn v_filter16(
    p: &mut [u8],
    off: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    filter_loop26(p, off, stride as isize, 1, 16, thresh, ithresh, hev_thresh);
}

/// Translation of `HFilter16_C()` (`VP8HFilter16`).
pub(crate) fn h_filter16(
    p: &mut [u8],
    off: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    filter_loop26(p, off, 1, stride, 16, thresh, ithresh, hev_thresh);
}

// on three inner edges

/// Translation of `VFilter16i_C()` (`VP8VFilter16i`).
pub(crate) fn v_filter16i(
    p: &mut [u8],
    mut off: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    for _ in 0..3 {
        off += 4 * stride;
        filter_loop24(p, off, stride as isize, 1, 16, thresh, ithresh, hev_thresh);
    }
}

/// Translation of `HFilter16i_C()` (`VP8HFilter16i`).
pub(crate) fn h_filter16i(
    p: &mut [u8],
    mut off: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    for _ in 0..3 {
        off += 4;
        filter_loop24(p, off, 1, stride, 16, thresh, ithresh, hev_thresh);
    }
}

// 8-pixels wide variant, for chroma filtering

/// Translation of `VFilter8_C()` (`VP8VFilter8`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn v_filter8(
    p: &mut [u8],
    u: usize,
    v: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    filter_loop26(p, u, stride as isize, 1, 8, thresh, ithresh, hev_thresh);
    filter_loop26(p, v, stride as isize, 1, 8, thresh, ithresh, hev_thresh);
}

/// Translation of `HFilter8_C()` (`VP8HFilter8`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn h_filter8(
    p: &mut [u8],
    u: usize,
    v: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    filter_loop26(p, u, 1, stride, 8, thresh, ithresh, hev_thresh);
    filter_loop26(p, v, 1, stride, 8, thresh, ithresh, hev_thresh);
}

/// Translation of `VFilter8i_C()` (`VP8VFilter8i`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn v_filter8i(
    p: &mut [u8],
    u: usize,
    v: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    filter_loop24(p, u + 4 * stride, stride as isize, 1, 8, thresh, ithresh, hev_thresh);
    filter_loop24(p, v + 4 * stride, stride as isize, 1, 8, thresh, ithresh, hev_thresh);
}

/// Translation of `HFilter8i_C()` (`VP8HFilter8i`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn h_filter8i(
    p: &mut [u8],
    u: usize,
    v: usize,
    stride: usize,
    thresh: i32,
    ithresh: i32,
    hev_thresh: i32,
) {
    filter_loop24(p, u + 4, 1, stride, 8, thresh, ithresh, hev_thresh);
    filter_loop24(p, v + 4, 1, stride, 8, thresh, ithresh, hev_thresh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_match_hand_computed_values() {
        // a DC-only block adds (dc + 4) >> 3 to every sample
        let mut dst = vec![100u8; BPS * 4];
        let mut coeffs = [0i16; 16];
        coeffs[0] = 80;
        transform_dc(&coeffs, &mut dst, 0);
        assert!(dst[..4].iter().all(|&v| v == 110));
        // the full transform agrees with it on a DC-only input
        let mut dst2 = vec![100u8; BPS * 4];
        transform(&coeffs, &mut dst2, 0, false);
        assert_eq!(dst[..4], dst2[..4]);
        assert_eq!(dst[3 * BPS..3 * BPS + 4], dst2[3 * BPS..3 * BPS + 4]);
        // and the simplified AC3 one too
        let mut dst3 = vec![100u8; BPS * 4];
        transform_ac3(&coeffs, &mut dst3, 0);
        assert_eq!(dst[..4], dst3[..4]);
        // clipping
        assert_eq!(clip_8b(-3), 0);
        assert_eq!(clip_8b(300), 255);
        assert_eq!(clip_8b(77), 77);
    }

    #[test]
    fn wht_of_a_dc_is_flat() {
        let mut input = [0i16; 16];
        input[0] = 16 * 8;
        let mut out = [0i16; 256];
        transform_wht(&input, &mut out);
        for i in 0..16 {
            assert_eq!(out[16 * i], (16 * 8 + 3) >> 3);
        }
    }
}
