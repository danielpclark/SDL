// Rust translation of src/dsp/enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the plain-C functions.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Speed-critical encoding functions: the forward and inverse transforms,
//! the intra predictions, the distortion metrics, the quantization and the
//! block copies.
//!
//! The blocks are slices starting at their top-left sample, with rows
//! `BPS` apart. A `left` column starts at its top-left corner sample
//! (`left[-1]` upstream) and is `None` where upstream passes NULL; the
//! intra4 predictions read their boundary around `top`, an index into it.
//! The `clip1[]` run-time table is the clamp it tabulates.

use crate::webp::dsp::BPS;
use crate::webp::enc::vp8i_enc::{
    quantdiv, VP8Matrix, C8DC8, C8HE8, C8TM8, C8VE8, I16DC16, I16HE16, I16TM16, I16VE16, I4DC4,
    I4HD4, I4HE4, I4HU4, I4LD4, I4RD4, I4TM4, I4VE4, I4VL4, I4VR4, MAX_LEVEL,
};

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

/// Translation of `clip_max()`.
fn clip_max(v: i32, max: i32) -> i32 {
    if v > max {
        max
    } else {
        v
    }
}

//------------------------------------------------------------------------------
// Compute susceptibility based on DCT-coeff histograms:
// the higher, the "easier" the macroblock is to compress.

/// size of histogram used by CollectHistogram. Translation of
/// `MAX_COEFF_THRESH`.
pub(crate) const MAX_COEFF_THRESH: usize = 31;

/// Translation of `VP8Histogram`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8Histogram {
    // We only need to store max_value and last_non_zero, not the distribution.
    pub(crate) max_value: i32,
    pub(crate) last_non_zero: i32,
}

/// Translation of `VP8DspScan`.
pub(crate) const VP8_DSP_SCAN: [usize; 16 + 4 + 4] = [
    // Luma
    0,
    4,
    8,
    12,
    4 * BPS,
    4 + 4 * BPS,
    8 + 4 * BPS,
    12 + 4 * BPS,
    8 * BPS,
    4 + 8 * BPS,
    8 + 8 * BPS,
    12 + 8 * BPS,
    12 * BPS,
    4 + 12 * BPS,
    8 + 12 * BPS,
    12 + 12 * BPS,
    // U
    0,
    4,
    4 * BPS,
    4 + 4 * BPS,
    // V
    8,
    12,
    8 + 4 * BPS,
    12 + 4 * BPS,
];

/// general-purpose util function. Translation of `VP8SetHistogramData()`.
pub(crate) fn vp8_set_histogram_data(
    distribution: &[i32; MAX_COEFF_THRESH + 1],
    histo: &mut VP8Histogram,
) {
    let mut max_value = 0;
    let mut last_non_zero = 1;
    for (k, &value) in distribution.iter().enumerate() {
        if value > 0 {
            if value > max_value {
                max_value = value;
            }
            last_non_zero = k as i32;
        }
    }
    histo.max_value = max_value;
    histo.last_non_zero = last_non_zero;
}

/// Translation of `CollectHistogram_C()` (`VP8CollectHistogram`).
pub(crate) fn vp8_collect_histogram(
    ref_: &[u8],
    pred: &[u8],
    start_block: usize,
    end_block: usize,
    histo: &mut VP8Histogram,
) {
    let mut distribution = [0i32; MAX_COEFF_THRESH + 1];
    for j in start_block..end_block {
        let mut out = [0i16; 16];

        vp8_ftransform(&ref_[VP8_DSP_SCAN[j]..], &pred[VP8_DSP_SCAN[j]..], &mut out);

        // Convert coefficients to bin.
        for &o in &out {
            let v = (o as i32).abs() >> 3;
            let clipped_value = clip_max(v, MAX_COEFF_THRESH as i32);
            distribution[clipped_value as usize] += 1;
        }
    }
    vp8_set_histogram_data(&distribution, histo);
}

//------------------------------------------------------------------------------
// run-time tables (~4k)

/// clips [-255,510] to [0,255]. Translation of `clip1[255 + v]`.
fn clip1(v: i32) -> u8 {
    clip_8b(v)
}

//------------------------------------------------------------------------------
// Transforms (Paragraph 14.4)

/// Translation of the `STORE()` macro.
fn store(dst: &mut [u8], ref_: &[u8], x: usize, y: usize, v: i32) {
    dst[x + y * BPS] = clip_8b(ref_[x + y * BPS] as i32 + (v >> 3));
}

const K_C1: i32 = 20091 + (1 << 16);
const K_C2: i32 = 35468;

/// Translation of the `MUL()` macro.
fn mul(a: i32, b: i32) -> i32 {
    (a * b) >> 16
}

/// Translation of `ITransformOne()`.
fn itransform_one(ref_: &[u8], input: &[i16], dst: &mut [u8]) {
    let mut c = [0i32; 4 * 4];
    for i in 0..4 {
        // vertical pass
        let inp = |k: usize| input[i + k] as i32;
        let a = inp(0) + inp(8);
        let b = inp(0) - inp(8);
        let cc = mul(inp(4), K_C2) - mul(inp(12), K_C1);
        let d = mul(inp(4), K_C1) + mul(inp(12), K_C2);
        c[4 * i] = a + d;
        c[4 * i + 1] = b + cc;
        c[4 * i + 2] = b - cc;
        c[4 * i + 3] = a - d;
    }

    for i in 0..4 {
        // horizontal pass
        let tmp = |k: usize| c[i + k];
        let dc = tmp(0) + 4;
        let a = dc + tmp(8);
        let b = dc - tmp(8);
        let cc = mul(tmp(4), K_C2) - mul(tmp(12), K_C1);
        let d = mul(tmp(4), K_C1) + mul(tmp(12), K_C2);
        store(dst, ref_, 0, i, a + d);
        store(dst, ref_, 1, i, b + cc);
        store(dst, ref_, 2, i, b - cc);
        store(dst, ref_, 3, i, a - d);
    }
}

/// Translation of `ITransform_C()` (`VP8ITransform`).
pub(crate) fn vp8_itransform(ref_: &[u8], input: &[i16], dst: &mut [u8], do_two: bool) {
    itransform_one(ref_, input, dst);
    if do_two {
        itransform_one(&ref_[4..], &input[16..], &mut dst[4..]);
    }
}

/// Translation of `FTransform_C()` (`VP8FTransform`).
pub(crate) fn vp8_ftransform(src: &[u8], ref_: &[u8], out: &mut [i16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let s = &src[i * BPS..];
        let r = &ref_[i * BPS..];
        let d0 = s[0] as i32 - r[0] as i32; // 9bit dynamic range ([-255,255])
        let d1 = s[1] as i32 - r[1] as i32;
        let d2 = s[2] as i32 - r[2] as i32;
        let d3 = s[3] as i32 - r[3] as i32;
        let a0 = d0 + d3; // 10b                      [-510,510]
        let a1 = d1 + d2;
        let a2 = d1 - d2;
        let a3 = d0 - d3;
        tmp[i * 4] = (a0 + a1) * 8; // 14b                      [-8160,8160]
        tmp[1 + i * 4] = (a2 * 2217 + a3 * 5352 + 1812) >> 9; // [-7536,7542]
        tmp[2 + i * 4] = (a0 - a1) * 8;
        tmp[3 + i * 4] = (a3 * 2217 - a2 * 5352 + 937) >> 9;
    }
    for i in 0..4 {
        let a0 = tmp[i] + tmp[12 + i]; // 15b
        let a1 = tmp[4 + i] + tmp[8 + i];
        let a2 = tmp[4 + i] - tmp[8 + i];
        let a3 = tmp[i] - tmp[12 + i];
        out[i] = ((a0 + a1 + 7) >> 4) as i16; // 12b
        out[4 + i] = (((a2 * 2217 + a3 * 5352 + 12000) >> 16) + (a3 != 0) as i32) as i16;
        out[8 + i] = ((a0 - a1 + 7) >> 4) as i16;
        out[12 + i] = ((a3 * 2217 - a2 * 5352 + 51000) >> 16) as i16;
    }
}

/// Translation of `FTransform2_C()` (`VP8FTransform2`).
pub(crate) fn vp8_ftransform2(src: &[u8], ref_: &[u8], out: &mut [i16]) {
    vp8_ftransform(src, ref_, out);
    vp8_ftransform(&src[4..], &ref_[4..], &mut out[16..]);
}

/// Translation of `FTransformWHT_C()` (`VP8FTransformWHT`): the 16 DC
/// coefficients, read every 16 entries of `input`.
pub(crate) fn vp8_ftransform_wht(input: &[i16], out: &mut [i16]) {
    // input is 12b signed
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let inp = |k: usize| input[i * 64 + k * 16] as i32;
        let a0 = inp(0) + inp(2); // 13b
        let a1 = inp(1) + inp(3);
        let a2 = inp(1) - inp(3);
        let a3 = inp(0) - inp(2);
        tmp[i * 4] = a0 + a1; // 14b
        tmp[1 + i * 4] = a3 + a2;
        tmp[2 + i * 4] = a3 - a2;
        tmp[3 + i * 4] = a0 - a1;
    }
    for i in 0..4 {
        let a0 = tmp[i] + tmp[8 + i]; // 15b
        let a1 = tmp[4 + i] + tmp[12 + i];
        let a2 = tmp[4 + i] - tmp[12 + i];
        let a3 = tmp[i] - tmp[8 + i];
        let b0 = a0 + a1; // 16b
        let b1 = a3 + a2;
        let b2 = a3 - a2;
        let b3 = a0 - a1;
        out[i] = (b0 >> 1) as i16; // 15b
        out[4 + i] = (b1 >> 1) as i16;
        out[8 + i] = (b2 >> 1) as i16;
        out[12 + i] = (b3 >> 1) as i16;
    }
}

//------------------------------------------------------------------------------
// Intra predictions

/// Translation of `Fill()`.
fn fill(dst: &mut [u8], value: i32, size: usize) {
    for j in 0..size {
        dst[j * BPS..j * BPS + size].fill(value as u8);
    }
}

/// Translation of `VerticalPred()`.
fn vertical_pred(dst: &mut [u8], top: Option<&[u8]>, size: usize) {
    if let Some(top) = top {
        for j in 0..size {
            dst[j * BPS..j * BPS + size].copy_from_slice(&top[..size]);
        }
    } else {
        fill(dst, 127, size);
    }
}

/// Translation of `HorizontalPred()`.
fn horizontal_pred(dst: &mut [u8], left: Option<&[u8]>, size: usize) {
    if let Some(left) = left {
        for j in 0..size {
            dst[j * BPS..j * BPS + size].fill(left[1 + j]);
        }
    } else {
        fill(dst, 129, size);
    }
}

/// Translation of `TrueMotion()`.
fn true_motion(dst: &mut [u8], left: Option<&[u8]>, top: Option<&[u8]>, size: usize) {
    if let Some(left) = left {
        if let Some(top) = top {
            let clip = 255 - left[0] as i32;
            for y in 0..size {
                let clip_table = clip + left[1 + y] as i32;
                for x in 0..size {
                    dst[y * BPS + x] = clip1(clip_table + top[x] as i32 - 255);
                }
            }
        } else {
            horizontal_pred(dst, Some(left), size);
        }
    } else {
        // true motion without left samples (hence: with default 129 value)
        // is equivalent to VE prediction where you just copy the top samples.
        // Note that if top samples are not available, the default value is
        // then 129, and not 127 as in the VerticalPred case.
        if top.is_some() {
            vertical_pred(dst, top, size);
        } else {
            fill(dst, 129, size);
        }
    }
}

/// Translation of `DCMode()`.
fn dc_mode(
    dst: &mut [u8],
    left: Option<&[u8]>,
    top: Option<&[u8]>,
    size: usize,
    round: i32,
    shift: i32,
) {
    let mut dc = 0i32;
    if let Some(top) = top {
        for &t in &top[..size] {
            dc += t as i32;
        }
        if let Some(left) = left {
            // top and left present
            for &l in &left[1..1 + size] {
                dc += l as i32;
            }
        } else {
            // top, but no left
            dc += dc;
        }
        dc = (dc + round) >> shift;
    } else if let Some(left) = left {
        // left but no top
        for &l in &left[1..1 + size] {
            dc += l as i32;
        }
        dc += dc;
        dc = (dc + round) >> shift;
    } else {
        // no top, no left, nothing.
        dc = 0x80;
    }
    fill(dst, dc, size);
}

//------------------------------------------------------------------------------
// Chroma 8x8 prediction (paragraph 12.2)

/// Translation of `IntraChromaPreds_C()` (`VP8EncPredChroma8`): `left` is
/// the U column, followed by the V one 16 samples after it.
pub(crate) fn vp8_enc_pred_chroma8(dst: &mut [u8], left: Option<&[u8]>, top: Option<&[u8]>) {
    // U block
    dc_mode(&mut dst[C8DC8..], left, top, 8, 8, 4);
    vertical_pred(&mut dst[C8VE8..], top, 8);
    horizontal_pred(&mut dst[C8HE8..], left, 8);
    true_motion(&mut dst[C8TM8..], left, top, 8);
    // V block
    let dst = &mut dst[8..];
    let top = top.map(|t| &t[8..]);
    let left = left.map(|l| &l[16..]);
    dc_mode(&mut dst[C8DC8..], left, top, 8, 8, 4);
    vertical_pred(&mut dst[C8VE8..], top, 8);
    horizontal_pred(&mut dst[C8HE8..], left, 8);
    true_motion(&mut dst[C8TM8..], left, top, 8);
}

//------------------------------------------------------------------------------
// luma 16x16 prediction (paragraph 12.3)

/// Translation of `Intra16Preds_C()` (`VP8EncPredLuma16`).
pub(crate) fn vp8_enc_pred_luma16(dst: &mut [u8], left: Option<&[u8]>, top: Option<&[u8]>) {
    dc_mode(&mut dst[I16DC16..], left, top, 16, 16, 5);
    vertical_pred(&mut dst[I16VE16..], top, 16);
    horizontal_pred(&mut dst[I16HE16..], left, 16);
    true_motion(&mut dst[I16TM16..], left, top, 16);
}

//------------------------------------------------------------------------------
// luma 4x4 prediction

/// Translation of the `DST()` macro.
fn dst_set(dst: &mut [u8], x: usize, y: usize, v: u8) {
    dst[x + y * BPS] = v;
}

/// Translation of the `AVG3()` macro.
fn avg3(a: i32, b: i32, c: i32) -> u8 {
    ((a + 2 * b + c + 2) >> 2) as u8
}

/// Translation of the `AVG2()` macro.
fn avg2(a: i32, b: i32) -> u8 {
    ((a + b + 1) >> 1) as u8
}

/// `top[k]` of the boundary `b` around index `top`.
fn tp(b: &[u8], top: usize, k: isize) -> i32 {
    b[(top as isize + k) as usize] as i32
}

/// vertical. Translation of `VE4()`.
fn ve4(dst: &mut [u8], b: &[u8], top: usize) {
    let vals = [
        avg3(tp(b, top, -1), tp(b, top, 0), tp(b, top, 1)),
        avg3(tp(b, top, 0), tp(b, top, 1), tp(b, top, 2)),
        avg3(tp(b, top, 1), tp(b, top, 2), tp(b, top, 3)),
        avg3(tp(b, top, 2), tp(b, top, 3), tp(b, top, 4)),
    ];
    for i in 0..4 {
        dst[i * BPS..i * BPS + 4].copy_from_slice(&vals);
    }
}

/// horizontal. Translation of `HE4()`.
fn he4(dst: &mut [u8], b: &[u8], top: usize) {
    let x = tp(b, top, -1);
    let i = tp(b, top, -2);
    let j = tp(b, top, -3);
    let k = tp(b, top, -4);
    let l = tp(b, top, -5);
    dst[0..4].fill(avg3(x, i, j));
    dst[BPS..BPS + 4].fill(avg3(i, j, k));
    dst[2 * BPS..2 * BPS + 4].fill(avg3(j, k, l));
    dst[3 * BPS..3 * BPS + 4].fill(avg3(k, l, l));
}

/// Translation of `DC4()`.
fn dc4(dst: &mut [u8], b: &[u8], top: usize) {
    let mut dc: u32 = 4;
    for i in 0..4 {
        dc += (tp(b, top, i) + tp(b, top, -5 + i)) as u32;
    }
    fill(dst, (dc >> 3) as i32, 4);
}

/// Translation of `RD4()`.
fn rd4(dst: &mut [u8], b: &[u8], top: usize) {
    let x = tp(b, top, -1);
    let i = tp(b, top, -2);
    let j = tp(b, top, -3);
    let k = tp(b, top, -4);
    let l = tp(b, top, -5);
    let a = tp(b, top, 0);
    let bb = tp(b, top, 1);
    let c = tp(b, top, 2);
    let d = tp(b, top, 3);
    dst_set(dst, 0, 3, avg3(j, k, l));
    let v = avg3(i, j, k);
    dst_set(dst, 0, 2, v);
    dst_set(dst, 1, 3, v);
    let v = avg3(x, i, j);
    dst_set(dst, 0, 1, v);
    dst_set(dst, 1, 2, v);
    dst_set(dst, 2, 3, v);
    let v = avg3(a, x, i);
    dst_set(dst, 0, 0, v);
    dst_set(dst, 1, 1, v);
    dst_set(dst, 2, 2, v);
    dst_set(dst, 3, 3, v);
    let v = avg3(bb, a, x);
    dst_set(dst, 1, 0, v);
    dst_set(dst, 2, 1, v);
    dst_set(dst, 3, 2, v);
    let v = avg3(c, bb, a);
    dst_set(dst, 2, 0, v);
    dst_set(dst, 3, 1, v);
    dst_set(dst, 3, 0, avg3(d, c, bb));
}

/// Translation of `LD4()`.
fn ld4(dst: &mut [u8], b: &[u8], top: usize) {
    let a = tp(b, top, 0);
    let bb = tp(b, top, 1);
    let c = tp(b, top, 2);
    let d = tp(b, top, 3);
    let e = tp(b, top, 4);
    let f = tp(b, top, 5);
    let g = tp(b, top, 6);
    let h = tp(b, top, 7);
    dst_set(dst, 0, 0, avg3(a, bb, c));
    let v = avg3(bb, c, d);
    dst_set(dst, 1, 0, v);
    dst_set(dst, 0, 1, v);
    let v = avg3(c, d, e);
    dst_set(dst, 2, 0, v);
    dst_set(dst, 1, 1, v);
    dst_set(dst, 0, 2, v);
    let v = avg3(d, e, f);
    dst_set(dst, 3, 0, v);
    dst_set(dst, 2, 1, v);
    dst_set(dst, 1, 2, v);
    dst_set(dst, 0, 3, v);
    let v = avg3(e, f, g);
    dst_set(dst, 3, 1, v);
    dst_set(dst, 2, 2, v);
    dst_set(dst, 1, 3, v);
    let v = avg3(f, g, h);
    dst_set(dst, 3, 2, v);
    dst_set(dst, 2, 3, v);
    dst_set(dst, 3, 3, avg3(g, h, h));
}

/// Translation of `VR4()`.
fn vr4(dst: &mut [u8], b: &[u8], top: usize) {
    let x = tp(b, top, -1);
    let i = tp(b, top, -2);
    let j = tp(b, top, -3);
    let k = tp(b, top, -4);
    let a = tp(b, top, 0);
    let bb = tp(b, top, 1);
    let c = tp(b, top, 2);
    let d = tp(b, top, 3);
    let v = avg2(x, a);
    dst_set(dst, 0, 0, v);
    dst_set(dst, 1, 2, v);
    let v = avg2(a, bb);
    dst_set(dst, 1, 0, v);
    dst_set(dst, 2, 2, v);
    let v = avg2(bb, c);
    dst_set(dst, 2, 0, v);
    dst_set(dst, 3, 2, v);
    dst_set(dst, 3, 0, avg2(c, d));

    dst_set(dst, 0, 3, avg3(k, j, i));
    dst_set(dst, 0, 2, avg3(j, i, x));
    let v = avg3(i, x, a);
    dst_set(dst, 0, 1, v);
    dst_set(dst, 1, 3, v);
    let v = avg3(x, a, bb);
    dst_set(dst, 1, 1, v);
    dst_set(dst, 2, 3, v);
    let v = avg3(a, bb, c);
    dst_set(dst, 2, 1, v);
    dst_set(dst, 3, 3, v);
    dst_set(dst, 3, 1, avg3(bb, c, d));
}

/// Translation of `VL4()`.
fn vl4(dst: &mut [u8], b: &[u8], top: usize) {
    let a = tp(b, top, 0);
    let bb = tp(b, top, 1);
    let c = tp(b, top, 2);
    let d = tp(b, top, 3);
    let e = tp(b, top, 4);
    let f = tp(b, top, 5);
    let g = tp(b, top, 6);
    let h = tp(b, top, 7);
    dst_set(dst, 0, 0, avg2(a, bb));
    let v = avg2(bb, c);
    dst_set(dst, 1, 0, v);
    dst_set(dst, 0, 2, v);
    let v = avg2(c, d);
    dst_set(dst, 2, 0, v);
    dst_set(dst, 1, 2, v);
    let v = avg2(d, e);
    dst_set(dst, 3, 0, v);
    dst_set(dst, 2, 2, v);

    dst_set(dst, 0, 1, avg3(a, bb, c));
    let v = avg3(bb, c, d);
    dst_set(dst, 1, 1, v);
    dst_set(dst, 0, 3, v);
    let v = avg3(c, d, e);
    dst_set(dst, 2, 1, v);
    dst_set(dst, 1, 3, v);
    let v = avg3(d, e, f);
    dst_set(dst, 3, 1, v);
    dst_set(dst, 2, 3, v);
    dst_set(dst, 3, 2, avg3(e, f, g));
    dst_set(dst, 3, 3, avg3(f, g, h));
}

/// Translation of `HU4()`.
fn hu4(dst: &mut [u8], b: &[u8], top: usize) {
    let i = tp(b, top, -2);
    let j = tp(b, top, -3);
    let k = tp(b, top, -4);
    let l = tp(b, top, -5);
    dst_set(dst, 0, 0, avg2(i, j));
    let v = avg2(j, k);
    dst_set(dst, 2, 0, v);
    dst_set(dst, 0, 1, v);
    let v = avg2(k, l);
    dst_set(dst, 2, 1, v);
    dst_set(dst, 0, 2, v);
    dst_set(dst, 1, 0, avg3(i, j, k));
    let v = avg3(j, k, l);
    dst_set(dst, 3, 0, v);
    dst_set(dst, 1, 1, v);
    let v = avg3(k, l, l);
    dst_set(dst, 3, 1, v);
    dst_set(dst, 1, 2, v);
    let l = l as u8;
    dst_set(dst, 3, 2, l);
    dst_set(dst, 2, 2, l);
    dst_set(dst, 0, 3, l);
    dst_set(dst, 1, 3, l);
    dst_set(dst, 2, 3, l);
    dst_set(dst, 3, 3, l);
}

/// Translation of `HD4()`.
fn hd4(dst: &mut [u8], b: &[u8], top: usize) {
    let x = tp(b, top, -1);
    let i = tp(b, top, -2);
    let j = tp(b, top, -3);
    let k = tp(b, top, -4);
    let l = tp(b, top, -5);
    let a = tp(b, top, 0);
    let bb = tp(b, top, 1);
    let c = tp(b, top, 2);

    let v = avg2(i, x);
    dst_set(dst, 0, 0, v);
    dst_set(dst, 2, 1, v);
    let v = avg2(j, i);
    dst_set(dst, 0, 1, v);
    dst_set(dst, 2, 2, v);
    let v = avg2(k, j);
    dst_set(dst, 0, 2, v);
    dst_set(dst, 2, 3, v);
    dst_set(dst, 0, 3, avg2(l, k));

    dst_set(dst, 3, 0, avg3(a, bb, c));
    dst_set(dst, 2, 0, avg3(x, a, bb));
    let v = avg3(i, x, a);
    dst_set(dst, 1, 0, v);
    dst_set(dst, 3, 1, v);
    let v = avg3(j, i, x);
    dst_set(dst, 1, 1, v);
    dst_set(dst, 3, 2, v);
    let v = avg3(k, j, i);
    dst_set(dst, 1, 2, v);
    dst_set(dst, 3, 3, v);
    dst_set(dst, 1, 3, avg3(l, k, j));
}

/// Translation of `TM4()`.
fn tm4(dst: &mut [u8], b: &[u8], top: usize) {
    let clip = 255 - tp(b, top, -1);
    for y in 0..4 {
        let clip_table = clip + tp(b, top, -2 - y as isize);
        for x in 0..4 {
            dst[y * BPS + x] = clip1(clip_table + tp(b, top, x as isize) - 255);
        }
    }
}

/// Left samples are top[-5 .. -2], top_left is top[-1], top are
/// located at top[0..3], and top right is top[4..7]
/// Translation of `Intra4Preds_C()` (`VP8EncPredLuma4`): `top` indexes
/// the boundary samples `b`.
pub(crate) fn vp8_enc_pred_luma4(dst: &mut [u8], b: &[u8], top: usize) {
    dc4(&mut dst[I4DC4..], b, top);
    tm4(&mut dst[I4TM4..], b, top);
    ve4(&mut dst[I4VE4..], b, top);
    he4(&mut dst[I4HE4..], b, top);
    rd4(&mut dst[I4RD4..], b, top);
    vr4(&mut dst[I4VR4..], b, top);
    ld4(&mut dst[I4LD4..], b, top);
    vl4(&mut dst[I4VL4..], b, top);
    hd4(&mut dst[I4HD4..], b, top);
    hu4(&mut dst[I4HU4..], b, top);
}

//------------------------------------------------------------------------------
// Metric

/// Translation of `GetSSE()`.
fn get_sse(a: &[u8], b: &[u8], w: usize, h: usize) -> i32 {
    let mut count = 0;
    for y in 0..h {
        for x in 0..w {
            let diff = a[y * BPS + x] as i32 - b[y * BPS + x] as i32;
            count += diff * diff;
        }
    }
    count
}

/// Translation of `SSE16x16_C()` (`VP8SSE16x16`).
pub(crate) fn vp8_sse16x16(a: &[u8], b: &[u8]) -> i32 {
    get_sse(a, b, 16, 16)
}

/// Translation of `SSE16x8_C()` (`VP8SSE16x8`).
pub(crate) fn vp8_sse16x8(a: &[u8], b: &[u8]) -> i32 {
    get_sse(a, b, 16, 8)
}

/// Translation of `SSE4x4_C()` (`VP8SSE4x4`).
pub(crate) fn vp8_sse4x4(a: &[u8], b: &[u8]) -> i32 {
    get_sse(a, b, 4, 4)
}

/// Translation of `Mean16x4_C()` (`VP8Mean16x4`).
pub(crate) fn vp8_mean16x4(ref_: &[u8], dc: &mut [u32; 4]) {
    for (k, d) in dc.iter_mut().enumerate() {
        let mut avg: u32 = 0;
        for y in 0..4 {
            for x in 0..4 {
                avg += ref_[k * 4 + x + y * BPS] as u32;
            }
        }
        *d = avg;
    }
}

//------------------------------------------------------------------------------
// Texture distortion
//
// We try to match the spectral content (weighted) between source and
// reconstructed samples.

/// Hadamard transform
/// Returns the weighted sum of the absolute value of transformed coefficients.
/// w[] contains a row-major 4 by 4 symmetric matrix.
/// Translation of `TTransform()`.
fn ttransform(input: &[u8], w: &[u16; 16]) -> i32 {
    let mut sum = 0;
    let mut tmp = [0i32; 16];
    // horizontal pass
    for i in 0..4 {
        let inp = |k: usize| input[i * BPS + k] as i32;
        let a0 = inp(0) + inp(2);
        let a1 = inp(1) + inp(3);
        let a2 = inp(1) - inp(3);
        let a3 = inp(0) - inp(2);
        tmp[i * 4] = a0 + a1;
        tmp[1 + i * 4] = a3 + a2;
        tmp[2 + i * 4] = a3 - a2;
        tmp[3 + i * 4] = a0 - a1;
    }
    // vertical pass
    for i in 0..4 {
        let a0 = tmp[i] + tmp[8 + i];
        let a1 = tmp[4 + i] + tmp[12 + i];
        let a2 = tmp[4 + i] - tmp[12 + i];
        let a3 = tmp[i] - tmp[8 + i];
        let b0 = a0 + a1;
        let b1 = a3 + a2;
        let b2 = a3 - a2;
        let b3 = a0 - a1;

        sum += w[i] as i32 * b0.abs();
        sum += w[4 + i] as i32 * b1.abs();
        sum += w[8 + i] as i32 * b2.abs();
        sum += w[12 + i] as i32 * b3.abs();
    }
    sum
}

/// Translation of `Disto4x4_C()` (`VP8TDisto4x4`).
pub(crate) fn vp8_tdisto4x4(a: &[u8], b: &[u8], w: &[u16; 16]) -> i32 {
    let sum1 = ttransform(a, w);
    let sum2 = ttransform(b, w);
    (sum2 - sum1).abs() >> 5
}

/// Translation of `Disto16x16_C()` (`VP8TDisto16x16`).
pub(crate) fn vp8_tdisto16x16(a: &[u8], b: &[u8], w: &[u16; 16]) -> i32 {
    let mut d = 0;
    for y in (0..16 * BPS).step_by(4 * BPS) {
        for x in (0..16).step_by(4) {
            d += vp8_tdisto4x4(&a[x + y..], &b[x + y..], w);
        }
    }
    d
}

//------------------------------------------------------------------------------
// Quantization
//

const K_ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// Simple quantization
/// Translation of `QuantizeBlock_C()` (`VP8EncQuantizeBlock` and
/// `VP8EncQuantizeBlockWHT`).
pub(crate) fn vp8_enc_quantize_block(input: &mut [i16], out: &mut [i16], mtx: &VP8Matrix) -> i32 {
    let mut last = -1;
    for n in 0..16 {
        let j = K_ZIGZAG[n];
        let sign = input[j] < 0;
        let coeff = (if sign {
            -(input[j] as i32)
        } else {
            input[j] as i32
        }) as u32
            + mtx.sharpen[j] as u32;
        if coeff > mtx.zthresh[j] {
            let q = mtx.q[j] as u32;
            let iq = mtx.iq[j] as u32;
            let b = mtx.bias[j];
            let mut level = quantdiv(coeff, iq, b);
            if level > MAX_LEVEL {
                level = MAX_LEVEL;
            }
            if sign {
                level = -level;
            }
            input[j] = level.wrapping_mul(q as i32) as i16;
            out[n] = level as i16;
            if level != 0 {
                last = n as i32;
            }
        } else {
            out[n] = 0;
            input[j] = 0;
        }
    }
    (last >= 0) as i32
}

/// Translation of `Quantize2Blocks_C()` (`VP8EncQuantize2Blocks`).
pub(crate) fn vp8_enc_quantize2_blocks(input: &mut [i16], out: &mut [i16], mtx: &VP8Matrix) -> i32 {
    let mut nz = vp8_enc_quantize_block(&mut input[..16], &mut out[..16], mtx);
    nz |= vp8_enc_quantize_block(&mut input[16..32], &mut out[16..32], mtx) << 1;
    nz
}

//------------------------------------------------------------------------------
// Block copy

/// Translation of `Copy()`.
fn copy(src: &[u8], dst: &mut [u8], w: usize, h: usize) {
    for y in 0..h {
        dst[y * BPS..y * BPS + w].copy_from_slice(&src[y * BPS..y * BPS + w]);
    }
}

/// Translation of `Copy4x4_C()` (`VP8Copy4x4`).
pub(crate) fn vp8_copy4x4(src: &[u8], dst: &mut [u8]) {
    copy(src, dst, 4, 4);
}

/// Translation of `Copy16x8_C()` (`VP8Copy16x8`).
pub(crate) fn vp8_copy16x8(src: &[u8], dst: &mut [u8]) {
    copy(src, dst, 16, 8);
}
