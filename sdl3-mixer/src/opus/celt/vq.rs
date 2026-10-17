// Rust translation of celt/vq.h and celt/vq.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Vector quantisation of the residual: the decoding half
//! (`alg_unquant()`; the encoder's `op_pvq_search_c()`, `alg_quant()` and
//! `stereo_itheta()` aren't translated).

use super::arch::Q15ONE;
use super::bands::SPREAD_NONE;
use super::cwrs::decode_pulses;
use super::entcode::{celt_udiv, EcDec};
use super::mathops::{celt_cos_norm, celt_div, celt_rsqrt_norm};
use super::pitch::celt_inner_prod;

fn exp_rotation1(x: &mut [f32], len: i32, stride: i32, c: f32, s: f32) {
    let ms = -s;
    let stride = stride as usize;
    let mut xptr = 0usize;
    for _ in 0..len - stride as i32 {
        let x1 = x[xptr];
        let x2 = x[xptr + stride];
        x[xptr + stride] = c * x2 + s * x1;
        x[xptr] = c * x1 + ms * x2;
        xptr += 1;
    }
    let mut i = len - 2 * stride as i32 - 1;
    while i >= 0 {
        let xptr = i as usize;
        let x1 = x[xptr];
        let x2 = x[xptr + stride];
        x[xptr + stride] = c * x2 + s * x1;
        x[xptr] = c * x1 + ms * x2;
        i -= 1;
    }
}

/* Apply a rotation */
/// Translation of `exp_rotation()`.
pub(crate) fn exp_rotation(
    x: &mut [f32],
    mut len: i32,
    dir: i32,
    stride: i32,
    k: i32,
    spread: i32,
) {
    const SPREAD_FACTOR: [i32; 3] = [15, 10, 5];
    let mut stride2 = 0;

    if 2 * k >= len || spread == SPREAD_NONE {
        return;
    }
    let factor = SPREAD_FACTOR[(spread - 1) as usize];

    let gain = celt_div(Q15ONE * len as f32, (len + factor * k) as f32);
    let theta = 0.5f32 * (gain * gain);

    let c = celt_cos_norm(theta);
    let s = celt_cos_norm(Q15ONE - theta); /*  sin(theta) */

    if len >= 8 * stride {
        stride2 = 1;
        /* This is just a simple (equivalent) way of computing sqrt(len/stride) with rounding.
        It's basically incrementing long as (stride2+0.5)^2 < len/stride. */
        while (stride2 * stride2 + stride2) * stride + (stride >> 2) < len {
            stride2 += 1;
        }
    }
    /*NOTE: As a minor optimization, we could be passing around log2(B), not B, for both this and for
    extract_collapse_mask().*/
    len = celt_udiv(len as u32, stride as u32) as i32;
    for i in 0..stride {
        let xs = &mut x[(i * len) as usize..];
        if dir < 0 {
            if stride2 != 0 {
                exp_rotation1(xs, len, stride2, s, c);
            }
            exp_rotation1(xs, len, 1, c, s);
        } else {
            exp_rotation1(xs, len, 1, c, -s);
            if stride2 != 0 {
                exp_rotation1(xs, len, stride2, s, -c);
            }
        }
    }
}

/* Takes the pitch vector and the decoded residual vector, computes the gain
that will give ||p+g*y||=1 and mixes the residual with the pitch. */
fn normalise_residual(iy: &[i32], x: &mut [f32], n: i32, ryy: f32, gain: f32) {
    let t = ryy;
    let g = celt_rsqrt_norm(t) * gain;

    for i in 0..n as usize {
        x[i] = g * iy[i] as f32;
    }
}

fn extract_collapse_mask(iy: &[i32], n: i32, b: i32) -> u32 {
    if b <= 1 {
        return 1;
    }
    /*NOTE: As a minor optimization, we could be passing around log2(B), not B, for both this and for
    exp_rotation().*/
    let n0 = celt_udiv(n as u32, b as u32) as usize;
    let mut collapse_mask: u32 = 0;
    for i in 0..b as usize {
        let mut tmp: u32 = 0;
        for j in 0..n0 {
            tmp |= iy[i * n0 + j] as u32;
        }
        collapse_mask |= ((tmp != 0) as u32) << i;
    }
    collapse_mask
}

/* Algebraic pulse decoder
@param X Decoded normalised spectrum (returned)
@param N Number of samples to decode
@param K Number of pulses to use
@param dec Range decoder
@ret A mask indicating which blocks in the band received pulses
*/
/// Translation of `alg_unquant()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn alg_unquant(
    x: &mut [f32],
    n: i32,
    k: i32,
    spread: i32,
    b: i32,
    dec: &mut EcDec<'_>,
    gain: f32,
) -> u32 {
    debug_assert!(k > 0, "alg_unquant() needs at least one pulse");
    debug_assert!(n > 1, "alg_unquant() needs at least two dimensions");
    let mut iy = [0i32; 176];
    let iy = &mut iy[..n as usize];
    let ryy = decode_pulses(iy, n, k, dec);
    normalise_residual(iy, x, n, ryy, gain);
    exp_rotation(x, n, -1, b, k, spread);
    extract_collapse_mask(iy, n, b)
}

/// Translation of `renormalise_vector()`.
pub(crate) fn renormalise_vector(x: &mut [f32], n: i32, gain: f32) {
    let e = super::arch::EPSILON + celt_inner_prod(x, x, n);
    let t = e;
    let g = celt_rsqrt_norm(t) * gain;

    for v in x[..n as usize].iter_mut() {
        *v = g * *v;
    }
    /*return celt_sqrt(E);*/
}
