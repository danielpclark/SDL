// Rust translation of src/looprestoration_tmpl.c and
// src/looprestoration.h from dav1d (https://code.videolan.org/videolan/dav1d,
// at the revision SDL_image's external/dav1d pins), the plain-C functions.
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The loop restoration filters: Wiener and self-guided.
//!
//! Although the spec applies restoration filters over 4x4 blocks,
//! they can be applied to a bigger surface.
//!    * w is constrained by the restoration unit size (w <= 256)
//!    * h is constrained by the stripe height (h <= 64)
//!
//! The filters work on one stripe: `p[po..]` in its plane (`stride` in
//! pixels), `left` the four pre-filter columns to its left, `lpf[lo..]`
//! the saved rows above and below (in the loop filter line buffer, of the
//! same stride).

#![allow(clippy::too_many_arguments)]

use super::bitdepth::{bitdepth_from_max, iclip_pixel, Pixel};
use super::intops::{iclip, imax, umin};
use super::tables::SGR_X_BY_X;

// enum LrEdgeFlags
pub(crate) const LR_HAVE_LEFT: u32 = 1 << 0;
pub(crate) const LR_HAVE_RIGHT: u32 = 1 << 1;
pub(crate) const LR_HAVE_TOP: u32 = 1 << 2;
pub(crate) const LR_HAVE_BOTTOM: u32 = 1 << 3;

/// Translation of `LooprestorationParams` (the union of the Wiener filter
/// taps and the self-guided parameters).
#[derive(Clone, Copy, Default)]
pub(crate) struct LooprestorationParams {
    pub(crate) filter: [[i16; 8]; 2],
    pub(crate) sgr_s0: u32,
    pub(crate) sgr_s1: u32,
    pub(crate) sgr_w0: i16,
    pub(crate) sgr_w1: i16,
}

// 256 * 1.5 + 3 + 3 = 390
const REST_UNIT_STRIDE: usize = 390;

// TODO Reuse p when no padding is needed (add and remove lpf pixels in p)
// TODO Chroma only requires 2 rows of padding.
#[inline(never)]
fn padding<P: Pixel>(
    dst: &mut [P],
    p: &[P],
    mut po: usize,
    stride: usize,
    left: &[[P; 4]],
    lpf: &[P],
    mut lo: usize,
    mut unit_w: usize,
    stripe_h: usize,
    edges: u32,
) {
    let have_left = (edges & LR_HAVE_LEFT) != 0;
    let have_right = (edges & LR_HAVE_RIGHT) != 0;

    // Copy more pixels if we don't have to pad them
    unit_w += 3 * have_left as usize + 3 * have_right as usize;
    let mut dst_l = 3 * !have_left as usize;
    po -= 3 * have_left as usize;
    lo -= 3 * have_left as usize;

    if (edges & LR_HAVE_TOP) != 0 {
        // Copy previous loop filtered rows
        let above_1 = lo;
        let above_2 = above_1 + stride;
        dst[dst_l..dst_l + unit_w].copy_from_slice(&lpf[above_1..above_1 + unit_w]);
        dst[dst_l + REST_UNIT_STRIDE..dst_l + REST_UNIT_STRIDE + unit_w]
            .copy_from_slice(&lpf[above_1..above_1 + unit_w]);
        dst[dst_l + 2 * REST_UNIT_STRIDE..dst_l + 2 * REST_UNIT_STRIDE + unit_w]
            .copy_from_slice(&lpf[above_2..above_2 + unit_w]);
    } else {
        // Pad with first row
        for k in 0..3 {
            dst[dst_l + k * REST_UNIT_STRIDE..dst_l + k * REST_UNIT_STRIDE + unit_w]
                .copy_from_slice(&p[po..po + unit_w]);
        }
        if have_left {
            for k in 0..3 {
                dst[dst_l + k * REST_UNIT_STRIDE..dst_l + k * REST_UNIT_STRIDE + 3]
                    .copy_from_slice(&left[0][1..4]);
            }
        }
    }

    let mut dst_tl = dst_l + 3 * REST_UNIT_STRIDE;
    if (edges & LR_HAVE_BOTTOM) != 0 {
        // Copy next loop filtered rows
        let below_1 = lo + 6 * stride;
        let below_2 = below_1 + stride;
        let r0 = dst_tl + stripe_h * REST_UNIT_STRIDE;
        dst[r0..r0 + unit_w].copy_from_slice(&lpf[below_1..below_1 + unit_w]);
        let r1 = dst_tl + (stripe_h + 1) * REST_UNIT_STRIDE;
        dst[r1..r1 + unit_w].copy_from_slice(&lpf[below_2..below_2 + unit_w]);
        let r2 = dst_tl + (stripe_h + 2) * REST_UNIT_STRIDE;
        dst[r2..r2 + unit_w].copy_from_slice(&lpf[below_2..below_2 + unit_w]);
    } else {
        // Pad with last row
        let src = po + (stripe_h - 1) * stride;
        for k in 0..3 {
            let r = dst_tl + (stripe_h + k) * REST_UNIT_STRIDE;
            dst[r..r + unit_w].copy_from_slice(&p[src..src + unit_w]);
        }
        if have_left {
            for k in 0..3 {
                let r = dst_tl + (stripe_h + k) * REST_UNIT_STRIDE;
                dst[r..r + 3].copy_from_slice(&left[stripe_h - 1][1..4]);
            }
        }
    }

    // Inner UNIT_WxSTRIPE_H
    let hl = 3 * have_left as usize;
    for _j in 0..stripe_h {
        dst[dst_tl + hl..dst_tl + unit_w].copy_from_slice(&p[po + hl..po + unit_w]);
        dst_tl += REST_UNIT_STRIDE;
        po += stride;
    }

    if !have_right {
        let mut pad = dst_l + unit_w;
        let mut row_last = dst_l + unit_w - 1;
        // Pad 3x(STRIPE_H+6) with last column
        for _j in 0..stripe_h + 6 {
            let v = dst[row_last];
            dst[pad..pad + 3].fill(v);
            pad += REST_UNIT_STRIDE;
            row_last += REST_UNIT_STRIDE;
        }
    }

    let mut d = 0;
    if !have_left {
        // Pad 3x(STRIPE_H+6) with first column
        for _j in 0..stripe_h + 6 {
            let v = dst[dst_l];
            dst[d..d + 3].fill(v);
            d += REST_UNIT_STRIDE;
            dst_l += REST_UNIT_STRIDE;
        }
    } else {
        d += 3 * REST_UNIT_STRIDE;
        for j in 0..stripe_h {
            dst[d..d + 3].copy_from_slice(&left[j][1..4]);
            d += REST_UNIT_STRIDE;
        }
    }
}

// FIXME Could split into luma and chroma specific functions,
// (since first and last tops are always 0 for chroma)
// FIXME Could implement a version that requires less temporary memory
// (should be possible to implement with only 6 rows of temp storage)
fn wiener_c<P: Pixel>(
    p: &mut [P],
    po: usize,
    stride: usize,
    left: &[[P; 4]],
    lpf: &[P],
    lo: usize,
    w: usize,
    h: usize,
    params: &LooprestorationParams,
    edges: u32,
    bitdepth_max: i32,
) {
    // Wiener filtering is applied to a maximum stripe height of 64 + 3 pixels
    // of padding above and below
    let mut tmp = vec![P::default(); 70 /*(64 + 3 + 3)*/ * REST_UNIT_STRIDE];
    let mut tmp_ptr = 0;

    padding(&mut tmp, p, po, stride, left, lpf, lo, w, h, edges);

    // Values stored between horizontal and vertical filtering don't
    // fit in a uint8_t.
    let mut hor = vec![0u16; 70 /*(64 + 3 + 3)*/ * REST_UNIT_STRIDE];
    let mut hor_ptr = 0;

    let filter = &params.filter;
    let bitdepth = bitdepth_from_max(bitdepth_max);
    let round_bits_h = 3 + (bitdepth == 12) as i32 * 2;
    let rounding_off_h = 1 << (round_bits_h - 1);
    let clip_limit = 1 << (bitdepth + 1 + 7 - round_bits_h);
    for _j in 0..h + 6 {
        for i in 0..w {
            let mut sum = 1 << (bitdepth + 6);
            if P::BPC8 {
                sum += tmp[tmp_ptr + i + 3].to_i32() * 128;
            }

            for k in 0..7 {
                sum += tmp[tmp_ptr + i + k].to_i32() * filter[0][k] as i32;
            }

            hor[hor_ptr + i] =
                iclip((sum + rounding_off_h) >> round_bits_h, 0, clip_limit - 1) as u16;
        }
        tmp_ptr += REST_UNIT_STRIDE;
        hor_ptr += REST_UNIT_STRIDE;
    }

    let round_bits_v = 11 - (bitdepth == 12) as i32 * 2;
    let rounding_off_v = 1 << (round_bits_v - 1);
    let round_offset = 1 << (bitdepth + (round_bits_v - 1));
    for j in 0..h {
        for i in 0..w {
            let mut sum = -round_offset;

            for k in 0..7 {
                sum += hor[(j + k) * REST_UNIT_STRIDE + i] as i32 * filter[1][k] as i32;
            }

            p[po + j * stride + i] =
                iclip_pixel((sum + rounding_off_v) >> round_bits_v, bitdepth_max);
        }
    }
}

// Sum over a 3x3 area
// The dst and src pointers are positioned 3 pixels above and 3 pixels to the
// left of the top left corner. However, the self guided filter only needs 1
// pixel above and one pixel to the left. As for the pixels below and to the
// right they must be computed in the sums, but don't need to be stored.
//
// Example for a 4x4 block:
//      x x x x x x x x x x
//      x c c c c c c c c x
//      x i s s s s s s i x
//      x i s s s s s s i x
//      x i s s s s s s i x
//      x i s s s s s s i x
//      x i s s s s s s i x
//      x i s s s s s s i x
//      x c c c c c c c c x
//      x x x x x x x x x x
//
// s: Pixel summed and stored
// i: Pixel summed and stored (between loops)
// c: Pixel summed not stored
// x: Pixel not summed not stored
fn boxsum3<P: Pixel>(sumsq: &mut [i32], sum: &mut [i32], src: &[P], w: usize, h: usize) {
    // We skip the first row, as it is never used
    let src0 = REST_UNIT_STRIDE;

    // We skip the first and last columns, as they are never used
    for x in 1..w - 1 {
        let mut sum_v = x;
        let mut sumsq_v = x;
        let mut s = src0 + x;
        let mut a = src[s].to_i32();
        let mut a2 = a * a;
        let mut b = src[s + REST_UNIT_STRIDE].to_i32();
        let mut b2 = b * b;

        // We skip the first 2 rows, as they are skipped in the next loop and
        // we don't need the last 2 row as it is skipped in the next loop
        for _y in 2..h - 2 {
            s += REST_UNIT_STRIDE;
            let c = src[s + REST_UNIT_STRIDE].to_i32();
            let c2 = c * c;
            sum_v += REST_UNIT_STRIDE;
            sumsq_v += REST_UNIT_STRIDE;
            sum[sum_v] = a + b + c;
            sumsq[sumsq_v] = a2 + b2 + c2;
            a = b;
            a2 = b2;
            b = c;
            b2 = c2;
        }
    }

    // We skip the first row as it is never read
    let mut row = REST_UNIT_STRIDE;
    // We skip the last 2 rows as it is never read
    for _y in 2..h - 2 {
        let mut a = sum[row + 1];
        let mut a2 = sumsq[row + 1];
        let mut b = sum[row + 2];
        let mut b2 = sumsq[row + 2];

        // We don't store the first column as it is never read and
        // we don't store the last 2 columns as they are never read
        for x in 2..w - 2 {
            let c = sum[row + x + 1];
            let c2 = sumsq[row + x + 1];
            sum[row + x] = a + b + c;
            sumsq[row + x] = a2 + b2 + c2;
            a = b;
            a2 = b2;
            b = c;
            b2 = c2;
        }
        row += REST_UNIT_STRIDE;
    }
}

// Sum over a 5x5 area
// The dst and src pointers are positioned 3 pixels above and 3 pixels to the
// left of the top left corner. However, the self guided filter only needs 1
// pixel above and one pixel to the left. As for the pixels below and to the
// right they must be computed in the sums, but don't need to be stored.
//
// Example for a 4x4 block:
//      c c c c c c c c c c
//      c c c c c c c c c c
//      i i s s s s s s i i
//      i i s s s s s s i i
//      i i s s s s s s i i
//      i i s s s s s s i i
//      i i s s s s s s i i
//      i i s s s s s s i i
//      c c c c c c c c c c
//      c c c c c c c c c c
//
// s: Pixel summed and stored
// i: Pixel summed and stored (between loops)
// c: Pixel summed not stored
// x: Pixel not summed not stored
fn boxsum5<P: Pixel>(sumsq: &mut [i32], sum: &mut [i32], src: &[P], w: usize, h: usize) {
    for x in 0..w {
        let mut sum_v = x;
        let mut sumsq_v = x;
        let mut s = 3 * REST_UNIT_STRIDE + x;
        let mut a = src[s - 3 * REST_UNIT_STRIDE].to_i32();
        let mut a2 = a * a;
        let mut b = src[s - 2 * REST_UNIT_STRIDE].to_i32();
        let mut b2 = b * b;
        let mut c = src[s - REST_UNIT_STRIDE].to_i32();
        let mut c2 = c * c;
        let mut d = src[s].to_i32();
        let mut d2 = d * d;

        // We skip the first 2 rows, as they are skipped in the next loop and
        // we don't need the last 2 row as it is skipped in the next loop
        for _y in 2..h - 2 {
            s += REST_UNIT_STRIDE;
            let e = src[s].to_i32();
            let e2 = e * e;
            sum_v += REST_UNIT_STRIDE;
            sumsq_v += REST_UNIT_STRIDE;
            sum[sum_v] = a + b + c + d + e;
            sumsq[sumsq_v] = a2 + b2 + c2 + d2 + e2;
            a = b;
            b = c;
            c = d;
            d = e;
            a2 = b2;
            b2 = c2;
            c2 = d2;
            d2 = e2;
        }
    }

    // We skip the first row as it is never read
    let mut row = REST_UNIT_STRIDE;
    for _y in 2..h - 2 {
        let mut a = sum[row];
        let mut a2 = sumsq[row];
        let mut b = sum[row + 1];
        let mut b2 = sumsq[row + 1];
        let mut c = sum[row + 2];
        let mut c2 = sumsq[row + 2];
        let mut d = sum[row + 3];
        let mut d2 = sumsq[row + 3];

        for x in 2..w - 2 {
            let e = sum[row + x + 2];
            let e2 = sumsq[row + x + 2];
            sum[row + x] = a + b + c + d + e;
            sumsq[row + x] = a2 + b2 + c2 + d2 + e2;
            a = b;
            b = c;
            c = d;
            d = e;
            a2 = b2;
            b2 = c2;
            c2 = d2;
            d2 = e2;
        }
        row += REST_UNIT_STRIDE;
    }
}

/// `selfguided_filter()`: `dst` has a stride of 384, `src` is the padded
/// stripe (`REST_UNIT_STRIDE`).
#[inline(never)]
fn selfguided_filter<P: Pixel>(
    dst: &mut [i32],
    src: &[P],
    w: usize,
    h: usize,
    n: i32,
    s: u32,
    bitdepth_max: i32,
) {
    let sgr_one_by_x: u32 = if n == 25 { 164 } else { 455 };

    // Selfguided filter is applied to a maximum stripe height of 64 + 3 pixels
    // of padding above and below
    let mut sumsq = vec![0i32; 68 /*(64 + 2 + 2)*/ * REST_UNIT_STRIDE];
    let a_off = 2 * REST_UNIT_STRIDE + 3;
    // By inverting A and B after the boxsums, B can be of size coef instead
    // of int32_t
    let mut sum = vec![0i32; 68 /*(64 + 2 + 2)*/ * REST_UNIT_STRIDE];
    let b_off = 2 * REST_UNIT_STRIDE + 3;

    let step = (n == 25) as usize + 1;
    if n == 25 {
        boxsum5(&mut sumsq, &mut sum, src, w + 6, h + 6);
    } else {
        boxsum3(&mut sumsq, &mut sum, src, w + 6, h + 6);
    }
    let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;

    let (aa_arr, bb_arr) = (&mut sumsq, &mut sum);
    let mut aa = a_off - REST_UNIT_STRIDE;
    let mut bb = b_off - REST_UNIT_STRIDE;
    let mut j: i32 = -1;
    while j < h as i32 + 1 {
        for i in -1..w as isize + 1 {
            let ai = (aa as isize + i) as usize;
            let bi = (bb as isize + i) as usize;
            let a = (aa_arr[ai] + ((1 << (2 * bitdepth_min_8)) >> 1)) >> (2 * bitdepth_min_8);
            let b = (bb_arr[bi] + ((1 << bitdepth_min_8) >> 1)) >> bitdepth_min_8;

            let p = imax(a * n - b * b, 0) as u32;
            let z = (p.wrapping_mul(s).wrapping_add(1 << 19)) >> 20;
            let x = SGR_X_BY_X[umin(z, 255) as usize] as u32;

            // This is where we invert A and B, so that B is of size coef.
            aa_arr[ai] = (x
                .wrapping_mul(bb_arr[bi] as u32)
                .wrapping_mul(sgr_one_by_x)
                .wrapping_add(1 << 11)
                >> 12) as i32;
            bb_arr[bi] = x as i32;
        }
        aa += step * REST_UNIT_STRIDE;
        bb += step * REST_UNIT_STRIDE;
        j += step as i32;
    }

    let mut src_i = 3 * REST_UNIT_STRIDE + 3;
    let mut d = 0;
    let mut a_p = a_off;
    let mut b_p = b_off;
    let aa_arr = &sumsq;
    let bb_arr = &sum;
    let at = |arr: &[i32], base: usize, k: isize| arr[(base as isize + k) as usize];
    let rs = REST_UNIT_STRIDE as isize;
    if n == 25 {
        // #define SIX_NEIGHBORS(P, i)
        let six = |arr: &[i32], base: usize, i: isize| {
            (at(arr, base, i - rs) + at(arr, base, i + rs)) * 6
                + (at(arr, base, i - 1 - rs)
                    + at(arr, base, i - 1 + rs)
                    + at(arr, base, i + 1 - rs)
                    + at(arr, base, i + 1 + rs))
                    * 5
        };
        let mut j = 0;
        while j + 1 < h {
            for i in 0..w {
                let a = six(bb_arr, b_p, i as isize);
                let b = six(aa_arr, a_p, i as isize);
                dst[d + i] = (b - a * src[src_i + i].to_i32() + (1 << 8)) >> 9;
            }
            d += 384 /* Maximum restoration width is 384 (256 * 1.5) */;
            src_i += REST_UNIT_STRIDE;
            b_p += REST_UNIT_STRIDE;
            a_p += REST_UNIT_STRIDE;
            for i in 0..w {
                let ii = i as isize;
                let a = at(bb_arr, b_p, ii) * 6
                    + (at(bb_arr, b_p, ii - 1) + at(bb_arr, b_p, ii + 1)) * 5;
                let b = at(aa_arr, a_p, ii) * 6
                    + (at(aa_arr, a_p, ii - 1) + at(aa_arr, a_p, ii + 1)) * 5;
                dst[d + i] = (b - a * src[src_i + i].to_i32() + (1 << 7)) >> 8;
            }
            d += 384 /* Maximum restoration width is 384 (256 * 1.5) */;
            src_i += REST_UNIT_STRIDE;
            b_p += REST_UNIT_STRIDE;
            a_p += REST_UNIT_STRIDE;
            j += 2;
        }
        if j + 1 == h {
            // Last row, when number of rows is odd
            for i in 0..w {
                let a = six(bb_arr, b_p, i as isize);
                let b = six(aa_arr, a_p, i as isize);
                dst[d + i] = (b - a * src[src_i + i].to_i32() + (1 << 8)) >> 9;
            }
        }
    } else {
        // #define EIGHT_NEIGHBORS(P, i)
        let eight = |arr: &[i32], base: usize, i: isize| {
            (at(arr, base, i)
                + at(arr, base, i - 1)
                + at(arr, base, i + 1)
                + at(arr, base, i - rs)
                + at(arr, base, i + rs))
                * 4
                + (at(arr, base, i - 1 - rs)
                    + at(arr, base, i - 1 + rs)
                    + at(arr, base, i + 1 - rs)
                    + at(arr, base, i + 1 + rs))
                    * 3
        };
        for _j in 0..h {
            for i in 0..w {
                let a = eight(bb_arr, b_p, i as isize);
                let b = eight(aa_arr, a_p, i as isize);
                dst[d + i] = (b - a * src[src_i + i].to_i32() + (1 << 8)) >> 9;
            }
            d += 384;
            src_i += REST_UNIT_STRIDE;
            b_p += REST_UNIT_STRIDE;
            a_p += REST_UNIT_STRIDE;
        }
    }
}

fn sgr_5x5_c<P: Pixel>(
    p: &mut [P],
    mut po: usize,
    stride: usize,
    left: &[[P; 4]],
    lpf: &[P],
    lo: usize,
    w: usize,
    h: usize,
    params: &LooprestorationParams,
    edges: u32,
    bitdepth_max: i32,
) {
    // Selfguided filter is applied to a maximum stripe height of 64 + 3 pixels
    // of padding above and below
    let mut tmp = vec![P::default(); 70 /*(64 + 3 + 3)*/ * REST_UNIT_STRIDE];

    // Selfguided filter outputs to a maximum stripe height of 64 and a
    // maximum restoration width of 384 (256 * 1.5)
    let mut dst = vec![0i32; 64 * 384];

    padding(&mut tmp, p, po, stride, left, lpf, lo, w, h, edges);
    selfguided_filter(&mut dst, &tmp, w, h, 25, params.sgr_s0, bitdepth_max);

    let w0 = params.sgr_w0 as i32;
    for j in 0..h {
        for i in 0..w {
            let v = w0 * dst[j * 384 + i];
            p[po + i] = iclip_pixel(p[po + i].to_i32() + ((v + (1 << 10)) >> 11), bitdepth_max);
        }
        po += stride;
    }
}

fn sgr_3x3_c<P: Pixel>(
    p: &mut [P],
    mut po: usize,
    stride: usize,
    left: &[[P; 4]],
    lpf: &[P],
    lo: usize,
    w: usize,
    h: usize,
    params: &LooprestorationParams,
    edges: u32,
    bitdepth_max: i32,
) {
    let mut tmp = vec![P::default(); 70 /*(64 + 3 + 3)*/ * REST_UNIT_STRIDE];
    let mut dst = vec![0i32; 64 * 384];

    padding(&mut tmp, p, po, stride, left, lpf, lo, w, h, edges);
    selfguided_filter(&mut dst, &tmp, w, h, 9, params.sgr_s1, bitdepth_max);

    let w1 = params.sgr_w1 as i32;
    for j in 0..h {
        for i in 0..w {
            let v = w1 * dst[j * 384 + i];
            p[po + i] = iclip_pixel(p[po + i].to_i32() + ((v + (1 << 10)) >> 11), bitdepth_max);
        }
        po += stride;
    }
}

fn sgr_mix_c<P: Pixel>(
    p: &mut [P],
    mut po: usize,
    stride: usize,
    left: &[[P; 4]],
    lpf: &[P],
    lo: usize,
    w: usize,
    h: usize,
    params: &LooprestorationParams,
    edges: u32,
    bitdepth_max: i32,
) {
    let mut tmp = vec![P::default(); 70 /*(64 + 3 + 3)*/ * REST_UNIT_STRIDE];
    let mut dst0 = vec![0i32; 64 * 384];
    let mut dst1 = vec![0i32; 64 * 384];

    padding(&mut tmp, p, po, stride, left, lpf, lo, w, h, edges);
    selfguided_filter(&mut dst0, &tmp, w, h, 25, params.sgr_s0, bitdepth_max);
    selfguided_filter(&mut dst1, &tmp, w, h, 9, params.sgr_s1, bitdepth_max);

    let w0 = params.sgr_w0 as i32;
    let w1 = params.sgr_w1 as i32;
    for j in 0..h {
        for i in 0..w {
            let v = w0 * dst0[j * 384 + i] + w1 * dst1[j * 384 + i];
            p[po + i] = iclip_pixel(p[po + i].to_i32() + ((v + (1 << 10)) >> 11), bitdepth_max);
        }
        po += stride;
    }
}

/// The filters of `Dav1dLoopRestorationDSPContext`: `wiener[2]` (7-tap,
/// 5-tap, the same function) and `sgr[3]` (5x5, 3x3, mix).
#[derive(Clone, Copy)]
pub(crate) enum LrFilter {
    Wiener,
    Sgr5x5,
    Sgr3x3,
    SgrMix,
}

/// Call a loop restoration filter (`lr_fn(...)`).
pub(crate) fn lr_filter<P: Pixel>(
    f: LrFilter,
    p: &mut [P],
    po: usize,
    stride: usize,
    left: &[[P; 4]],
    lpf: &[P],
    lo: usize,
    w: usize,
    h: usize,
    params: &LooprestorationParams,
    edges: u32,
    bitdepth_max: i32,
) {
    match f {
        LrFilter::Wiener => wiener_c(
            p,
            po,
            stride,
            left,
            lpf,
            lo,
            w,
            h,
            params,
            edges,
            bitdepth_max,
        ),
        LrFilter::Sgr5x5 => sgr_5x5_c(
            p,
            po,
            stride,
            left,
            lpf,
            lo,
            w,
            h,
            params,
            edges,
            bitdepth_max,
        ),
        LrFilter::Sgr3x3 => sgr_3x3_c(
            p,
            po,
            stride,
            left,
            lpf,
            lo,
            w,
            h,
            params,
            edges,
            bitdepth_max,
        ),
        LrFilter::SgrMix => sgr_mix_c(
            p,
            po,
            stride,
            left,
            lpf,
            lo,
            w,
            h,
            params,
            edges,
            bitdepth_max,
        ),
    }
}
