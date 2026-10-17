// Rust translation of celt/celt.h and celt/celt.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2010 Xiph.Org Foundation
// Copyright (c) 2008 Gregory Maxwell
// Written by Jean-Marc Valin and Gregory Maxwell
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Contains all the functions for encoding and decoding audio: the parts
//! the decoder shares (the post-filter, the TF table, the caps), with the
//! plain C `comb_filter_const_c()` (not the ARM one).

use super::arch::imax;
use super::modes::OpusCustomMode;

pub(crate) static TRIM_ICDF: [u8; 11] = [126, 124, 119, 109, 87, 41, 19, 9, 4, 2, 0];
/* Probs: NONE: 21.875%, LIGHT: 6.25%, NORMAL: 65.625%, AGGRESSIVE: 6.25% */
pub(crate) static SPREAD_ICDF: [u8; 4] = [25, 23, 2, 0];

pub(crate) static TAPSET_ICDF: [u8; 3] = [2, 1, 0];

#[allow(dead_code)]
pub(crate) const COMBFILTER_MAXPERIOD: i32 = 1024;
pub(crate) const COMBFILTER_MINPERIOD: i32 = 15;

/// Translation of `resampling_factor()`.
pub(crate) fn resampling_factor(rate: i32) -> i32 {
    match rate {
        48000 => 1,
        24000 => 2,
        16000 => 3,
        12000 => 4,
        8000 => 6,
        _ => {
            debug_assert!(false);
            0
        }
    }
}

/// A comb filter's output: `y` itself, or (`y==x`, in place) `x`.
struct CombOut<'a>(Option<&'a mut [f32]>);

impl CombOut<'_> {
    #[inline(always)]
    fn put(&mut self, x: &mut [f32], x_off: usize, i: usize, v: f32) {
        match &mut self.0 {
            Some(y) => y[i] = v,
            None => x[x_off + i] = v,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn comb_filter_const_c(
    y: &mut CombOut<'_>,
    y_i: usize,
    x: &mut [f32],
    x_off: usize,
    t: i32,
    n: i32,
    g10: f32,
    g11: f32,
    g12: f32,
) {
    let t = t as usize;
    let mut x4 = x[x_off - t - 2];
    let mut x3 = x[x_off - t - 1];
    let mut x2 = x[x_off - t];
    let mut x1 = x[x_off - t + 1];
    for i in 0..n as usize {
        let x0 = x[x_off + i - t + 2];
        let v = x[x_off + i] + g10 * x2 + g11 * (x1 + x3) + g12 * (x0 + x4);
        y.put(x, x_off - y_i, y_i + i, v);
        x4 = x3;
        x3 = x2;
        x2 = x1;
        x1 = x0;
    }
}

/// Translation of `comb_filter()`: `x[x_off..]` is `x`; `y` is `y`, or
/// `None` for the in-place filter (`y==x`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn comb_filter(
    y: Option<&mut [f32]>,
    x: &mut [f32],
    x_off: usize,
    mut t0: i32,
    mut t1: i32,
    n: i32,
    g0: f32,
    g1: f32,
    tapset0: i32,
    tapset1: i32,
    window: &[f32],
    mut overlap: i32,
) {
    /* printf ("%d %d %f %f\n", T0, T1, g0, g1); */
    const GAINS: [[f32; 3]; 3] = [
        [0.3066406250, 0.2170410156, 0.1296386719],
        [0.4638671875, 0.2680664062, 0.0],
        [0.7998046875, 0.1000976562, 0.0],
    ];
    let mut y = CombOut(y);

    if g0 == 0.0 && g1 == 0.0 {
        /* OPT: Happens to work without the OPUS_MOVE(), but only because the current encoder already copies x to y */
        if let Some(yy) = y.0.as_deref_mut() {
            yy[..n as usize].copy_from_slice(&x[x_off..x_off + n as usize]);
        }
        return;
    }
    /* When the gain is zero, T0 and/or T1 is set to zero. We need
    to have then be at least 2 to avoid processing garbage data. */
    t0 = imax(t0, COMBFILTER_MINPERIOD);
    t1 = imax(t1, COMBFILTER_MINPERIOD);
    let g00 = g0 * GAINS[tapset0 as usize][0];
    let g01 = g0 * GAINS[tapset0 as usize][1];
    let g02 = g0 * GAINS[tapset0 as usize][2];
    let g10 = g1 * GAINS[tapset1 as usize][0];
    let g11 = g1 * GAINS[tapset1 as usize][1];
    let g12 = g1 * GAINS[tapset1 as usize][2];
    let t0u = t0 as usize;
    let t1u = t1 as usize;
    let mut x1 = x[x_off - t1u + 1];
    let mut x2 = x[x_off - t1u];
    let mut x3 = x[x_off - t1u - 1];
    let mut x4 = x[x_off - t1u - 2];
    /* If the filter didn't change, we don't need the overlap */
    if g0 == g1 && t0 == t1 && tapset0 == tapset1 {
        overlap = 0;
    }
    let mut i = 0usize;
    while (i as i32) < overlap {
        let x0 = x[x_off + i - t1u + 2];
        let f = window[i] * window[i];
        let xi = x_off + i;
        let v = x[xi]
            + ((super::arch::Q15ONE - f) * g00) * x[xi - t0u]
            + ((super::arch::Q15ONE - f) * g01) * (x[xi - t0u + 1] + x[xi - t0u - 1])
            + ((super::arch::Q15ONE - f) * g02) * (x[xi - t0u + 2] + x[xi - t0u - 2])
            + (f * g10) * x2
            + (f * g11) * (x1 + x3)
            + (f * g12) * (x0 + x4);
        y.put(x, x_off, i, v);
        x4 = x3;
        x3 = x2;
        x2 = x1;
        x1 = x0;
        i += 1;
    }
    if g1 == 0.0 {
        /* OPT: Happens to work without the OPUS_MOVE(), but only because the current encoder already copies x to y */
        if let Some(yy) = y.0.as_deref_mut() {
            let ov = overlap as usize;
            yy[ov..n as usize].copy_from_slice(&x[x_off + ov..x_off + n as usize]);
        }
        return;
    }

    /* Compute the part with the constant filter. */
    comb_filter_const_c(&mut y, i, x, x_off + i, t1, n - i as i32, g10, g11, g12);
}

/* TF change table. Positive values mean better frequency resolution (longer
effective window), whereas negative values mean better time resolution
(shorter effective window). The second index is computed as:
4*isTransient + 2*tf_select + per_band_flag */
pub(crate) static TF_SELECT_TABLE: [[i8; 8]; 4] = [
    /*isTransient=0     isTransient=1 */
    [0, -1, 0, -1, 0, -1, 0, -1], /* 2.5 ms */
    [0, -1, 0, -2, 1, 0, 1, -1],  /* 5 ms */
    [0, -2, 0, -3, 2, 0, 1, -1],  /* 10 ms */
    [0, -2, 0, -3, 3, 0, 1, -1],  /* 20 ms */
];

/// Translation of `init_caps()`.
pub(crate) fn init_caps(m: &OpusCustomMode, cap: &mut [i32], lm: i32, c: i32) {
    for i in 0..m.nb_ebands as usize {
        let n = ((m.ebands[i + 1] - m.ebands[i]) as i32) << lm;
        cap[i] =
            (m.cache.caps[(m.nb_ebands * (2 * lm + c - 1)) as usize + i] as i32 + 64) * c * n >> 2;
    }
}
