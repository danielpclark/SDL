// Rust translation of aom_dsp/intrapred.c and aom_dsp/intrapred_common.h
// from aom (https://aomedia.googlesource.com/aom, at the revision
// SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The high bit depth intra predictors the encoder reaches (its mode
//! search tries DC, V, H and SMOOTH only): V, H, SMOOTH and the four DC
//! variants, over every block size (`aom_highbd_<type>_predictor_<w>x<h>_c()`
//! are the functions here with the size as arguments). `dst` is the
//! destination buffer from `dst_off` on; `above` and `left` start at the
//! first above and left neighbors.

// Weights are quadratic from '1' to '1 / block_size', scaled by
// 2^SMOOTH_WEIGHT_LOG2_SCALE.
const SMOOTH_WEIGHT_LOG2_SCALE: u32 = 8;

static SMOOTH_WEIGHTS: [u8; 124] = [
    // bs = 4
    255, 149, 85, 64, // bs = 8
    255, 197, 146, 105, 73, 50, 37, 32, // bs = 16
    255, 225, 196, 170, 145, 123, 102, 84, 68, 54, 43, 33, 26, 20, 17, 16, // bs = 32
    255, 240, 225, 210, 196, 182, 169, 157, 145, 133, 122, 111, 101, 92, 83, 74, 66, 59, 52, 45,
    39, 34, 29, 25, 21, 17, 14, 12, 10, 9, 8, 8, // bs = 64
    255, 248, 240, 233, 225, 218, 210, 203, 196, 189, 182, 176, 169, 163, 156, 150, 144, 138, 133,
    127, 121, 116, 111, 106, 101, 96, 91, 86, 82, 77, 73, 69, 65, 61, 57, 54, 50, 47, 44, 41, 38,
    35, 32, 29, 27, 25, 22, 20, 18, 16, 15, 13, 12, 10, 9, 8, 7, 6, 6, 5, 5, 4, 4, 4,
];

#[inline]
fn divide_round(value: u32, bits: u32) -> u32 {
    (value + (1 << (bits - 1))) >> bits
}

#[inline]
fn divide_using_multiply_shift(num: i32, shift1: i32, multiplier: i32, shift2: i32) -> i32 {
    let interm = num >> shift1;
    interm * multiplier >> shift2
}

#[inline]
fn aom_memset16(dst: &mut [u16], val: i32, n: usize) {
    for d in dst[..n].iter_mut() {
        *d = val as u16;
    }
}

pub(crate) fn highbd_v_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    _left: &[u16],
    _bd: i32,
) {
    let mut d = dst_off;
    for _r in 0..bh {
        dst[d..d + bw as usize].copy_from_slice(&above[..bw as usize]);
        d += stride;
    }
}

pub(crate) fn highbd_h_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    _above: &[u16],
    left: &[u16],
    _bd: i32,
) {
    let mut d = dst_off;
    for r in 0..bh as usize {
        aom_memset16(&mut dst[d..], left[r] as i32, bw as usize);
        d += stride;
    }
}

pub(crate) fn highbd_smooth_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    left: &[u16],
    _bd: i32,
) {
    let below_pred = left[bh as usize - 1]; // estimated by bottom-left pixel
    let right_pred = above[bw as usize - 1]; // estimated by top-right pixel
    let sm_weights_w = &SMOOTH_WEIGHTS[bw as usize - 4..];
    let sm_weights_h = &SMOOTH_WEIGHTS[bh as usize - 4..];
    // scale = 2 * 2^SMOOTH_WEIGHT_LOG2_SCALE
    let log2_scale = 1 + SMOOTH_WEIGHT_LOG2_SCALE;
    let scale: u16 = 1 << SMOOTH_WEIGHT_LOG2_SCALE;
    let mut d = dst_off;
    for r in 0..bh as usize {
        for c in 0..bw as usize {
            let pixels = [above[c], below_pred, left[r], right_pred];
            let weights = [
                sm_weights_h[r],
                (scale - sm_weights_h[r] as u16) as u8,
                sm_weights_w[c],
                (scale - sm_weights_w[c] as u16) as u8,
            ];
            let mut this_pred: u32 = 0;
            debug_assert!(scale >= sm_weights_h[r] as u16 && scale >= sm_weights_w[c] as u16);
            for i in 0..4 {
                this_pred += weights[i] as u32 * pixels[i] as u32;
            }
            dst[d + c] = divide_round(this_pred, log2_scale) as u16;
        }
        d += stride;
    }
}

pub(crate) fn highbd_dc_128_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    _above: &[u16],
    _left: &[u16],
    bd: i32,
) {
    let mut d = dst_off;
    for _r in 0..bh {
        aom_memset16(&mut dst[d..], 128 << (bd - 8), bw as usize);
        d += stride;
    }
}

pub(crate) fn highbd_dc_left_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    _above: &[u16],
    left: &[u16],
    _bd: i32,
) {
    let mut sum: i32 = 0;

    for i in 0..bh as usize {
        sum += left[i] as i32;
    }
    let expected_dc = (sum + (bh >> 1)) / bh;

    let mut d = dst_off;
    for _r in 0..bh {
        aom_memset16(&mut dst[d..], expected_dc, bw as usize);
        d += stride;
    }
}

pub(crate) fn highbd_dc_top_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    _left: &[u16],
    _bd: i32,
) {
    let mut sum: i32 = 0;

    for i in 0..bw as usize {
        sum += above[i] as i32;
    }
    let expected_dc = (sum + (bw >> 1)) / bw;

    let mut d = dst_off;
    for _r in 0..bh {
        aom_memset16(&mut dst[d..], expected_dc, bw as usize);
        d += stride;
    }
}

fn highbd_dc_predictor_square(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    left: &[u16],
    _bd: i32,
) {
    let mut sum: i32 = 0;
    let count = bw + bh;

    for i in 0..bw as usize {
        sum += above[i] as i32;
    }
    for i in 0..bh as usize {
        sum += left[i] as i32;
    }

    let expected_dc = (sum + (count >> 1)) / count;

    let mut d = dst_off;
    for _r in 0..bh {
        aom_memset16(&mut dst[d..], expected_dc, bw as usize);
        d += stride;
    }
}

// Obtained similarly as DC_MULTIPLIER_1X2 and DC_MULTIPLIER_1X4 above, but
// assume 2nd shift of 17 bits instead of 16.
// Note: Strictly speaking, 2nd shift needs to be 17 only when:
// - bit depth == 12, and
// - bw + bh is divisible by 5 (as opposed to divisible by 3).
// All other cases can use half the multipliers with a shift of 16 instead.
// This special optimization can be used when writing assembly code.
const HIGHBD_DC_MULTIPLIER_1X2: i32 = 0xAAAB;
// Note: This constant is odd, but a smaller even constant (0x199a) with the
// appropriate shift should work for neon in 8/10-bit.
const HIGHBD_DC_MULTIPLIER_1X4: i32 = 0x6667;

const HIGHBD_DC_SHIFT2: i32 = 17;

fn highbd_dc_predictor_rect(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    left: &[u16],
    bd: i32,
    shift1: i32,
    multiplier: i32,
) {
    let mut sum: i32 = 0;

    for i in 0..bw as usize {
        sum += above[i] as i32;
    }
    for i in 0..bh as usize {
        sum += left[i] as i32;
    }

    let expected_dc =
        divide_using_multiply_shift(sum + ((bw + bh) >> 1), shift1, multiplier, HIGHBD_DC_SHIFT2);
    debug_assert!(expected_dc < (1 << bd));

    let mut d = dst_off;
    for _r in 0..bh {
        aom_memset16(&mut dst[d..], expected_dc, bw as usize);
        d += stride;
    }
}

/// `aom_highbd_dc_predictor_<w>x<h>_c()`: the square sizes average, the
/// rectangular ones multiply and shift (with the shifts and multipliers of
/// each size's function).
pub(crate) fn highbd_dc_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    left: &[u16],
    bd: i32,
) {
    let (shift1, multiplier) = match (bw, bh) {
        (4, 8) | (8, 4) => (2, HIGHBD_DC_MULTIPLIER_1X2),
        (4, 16) | (16, 4) => (2, HIGHBD_DC_MULTIPLIER_1X4),
        (8, 16) | (16, 8) => (3, HIGHBD_DC_MULTIPLIER_1X2),
        (8, 32) | (32, 8) => (3, HIGHBD_DC_MULTIPLIER_1X4),
        (16, 32) | (32, 16) => (4, HIGHBD_DC_MULTIPLIER_1X2),
        (16, 64) | (64, 16) => (4, HIGHBD_DC_MULTIPLIER_1X4),
        (32, 64) | (64, 32) => (5, HIGHBD_DC_MULTIPLIER_1X2),
        _ => {
            highbd_dc_predictor_square(dst, dst_off, stride, bw, bh, above, left, bd);
            return;
        }
    };
    highbd_dc_predictor_rect(
        dst, dst_off, stride, bw, bh, above, left, bd, shift1, multiplier,
    );
}
