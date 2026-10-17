// Rust translation of celt/pitch.h and celt/pitch.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Pitch analysis: what the decoder's packet loss concealment uses
//! (`pitch_downsample()`, `pitch_search()` and the correlation kernels, in
//! their plain C versions; the encoder's `remove_doubling()` isn't
//! translated). SDL_mixer's CMake builds libopus with run-time selected
//! SSE versions of the kernels; this is the generic C code
//! (`OPUS_DISABLE_INTRINSICS`), which sums in a different order.

use super::arch::{max32, Q15ONE};
use super::celt_lpc::{_celt_autocorr, _celt_lpc};

/* OPT: This is the kernel you really want to optimize. It gets used a lot
by the prefilter and by the PLC. */
/// Translation of `xcorr_kernel_c()`: `x[..]` against `y[..]`.
#[inline]
pub(crate) fn xcorr_kernel(x: &[f32], y: &[f32], sum: &mut [f32; 4], len: i32) {
    debug_assert!(len >= 3);
    let mut xi = 0usize;
    let mut yi = 0usize;
    let mut y_3: f32 = 0.0; /* gcc doesn't realize that y_3 can't be used uninitialized */
    let mut y_0 = y[yi];
    yi += 1;
    let mut y_1 = y[yi];
    yi += 1;
    let mut y_2 = y[yi];
    yi += 1;
    let mut j = 0;
    while j < len - 3 {
        let mut tmp = x[xi];
        xi += 1;
        y_3 = y[yi];
        yi += 1;
        sum[0] += tmp * y_0;
        sum[1] += tmp * y_1;
        sum[2] += tmp * y_2;
        sum[3] += tmp * y_3;
        tmp = x[xi];
        xi += 1;
        y_0 = y[yi];
        yi += 1;
        sum[0] += tmp * y_1;
        sum[1] += tmp * y_2;
        sum[2] += tmp * y_3;
        sum[3] += tmp * y_0;
        tmp = x[xi];
        xi += 1;
        y_1 = y[yi];
        yi += 1;
        sum[0] += tmp * y_2;
        sum[1] += tmp * y_3;
        sum[2] += tmp * y_0;
        sum[3] += tmp * y_1;
        tmp = x[xi];
        xi += 1;
        y_2 = y[yi];
        yi += 1;
        sum[0] += tmp * y_3;
        sum[1] += tmp * y_0;
        sum[2] += tmp * y_1;
        sum[3] += tmp * y_2;
        j += 4;
    }
    let jj = j;
    j += 1;
    if jj < len {
        let tmp = x[xi];
        xi += 1;
        y_3 = y[yi];
        yi += 1;
        sum[0] += tmp * y_0;
        sum[1] += tmp * y_1;
        sum[2] += tmp * y_2;
        sum[3] += tmp * y_3;
    }
    let jj = j;
    j += 1;
    if jj < len {
        let tmp = x[xi];
        xi += 1;
        y_0 = y[yi];
        yi += 1;
        sum[0] += tmp * y_1;
        sum[1] += tmp * y_2;
        sum[2] += tmp * y_3;
        sum[3] += tmp * y_0;
    }
    if j < len {
        let tmp = x[xi];
        y_1 = y[yi];
        sum[0] += tmp * y_2;
        sum[1] += tmp * y_3;
        sum[2] += tmp * y_0;
        sum[3] += tmp * y_1;
    }
}

/// Translation of `dual_inner_prod_c()`.
#[inline]
pub(crate) fn dual_inner_prod(x: &[f32], y01: &[f32], y02: &[f32], n: i32) -> (f32, f32) {
    let mut xy01: f32 = 0.0;
    let mut xy02: f32 = 0.0;
    for i in 0..n as usize {
        xy01 += x[i] * y01[i];
        xy02 += x[i] * y02[i];
    }
    (xy01, xy02)
}

/*We make sure a C version is always available for cases where the overhead of
vectorization and passing around an arch flag aren't worth it.*/
/// Translation of `celt_inner_prod_c()`.
#[inline]
pub(crate) fn celt_inner_prod(x: &[f32], y: &[f32], n: i32) -> f32 {
    let mut xy: f32 = 0.0;
    for i in 0..n as usize {
        xy += x[i] * y[i];
    }
    xy
}

fn find_best_pitch(xcorr: &[f32], y: &[f32], len: i32, max_pitch: i32, best_pitch: &mut [i32; 2]) {
    let mut syy: f32 = 1.0;
    let mut best_num = [-1.0f32; 2];
    let mut best_den = [0.0f32; 2];

    best_pitch[0] = 0;
    best_pitch[1] = 1;
    for j in 0..len as usize {
        syy += y[j] * y[j];
    }
    for i in 0..max_pitch as usize {
        if xcorr[i] > 0.0 {
            let mut xcorr16 = xcorr[i];
            /* Considering the range of xcorr16, this should avoid both underflows
            and overflows (inf) when squaring xcorr16 */
            xcorr16 *= 1e-12f32;
            let num = xcorr16 * xcorr16;
            if num * best_den[1] > best_num[1] * syy {
                if num * best_den[0] > best_num[0] * syy {
                    best_num[1] = best_num[0];
                    best_den[1] = best_den[0];
                    best_pitch[1] = best_pitch[0];
                    best_num[0] = num;
                    best_den[0] = syy;
                    best_pitch[0] = i as i32;
                } else {
                    best_num[1] = num;
                    best_den[1] = syy;
                    best_pitch[1] = i as i32;
                }
            }
        }
        let a = y[i + len as usize];
        let b = y[i];
        syy += a * a - b * b;
        syy = max32(1.0, syy);
    }
}

fn celt_fir5(x: &mut [f32], num: &[f32; 5], n: i32) {
    let num0 = num[0];
    let num1 = num[1];
    let num2 = num[2];
    let num3 = num[3];
    let num4 = num[4];
    let mut mem0: f32 = 0.0;
    let mut mem1: f32 = 0.0;
    let mut mem2: f32 = 0.0;
    let mut mem3: f32 = 0.0;
    let mut mem4: f32 = 0.0;
    for v in x[..n as usize].iter_mut() {
        let mut sum = *v;
        sum += num0 * mem0;
        sum += num1 * mem1;
        sum += num2 * mem2;
        sum += num3 * mem3;
        sum += num4 * mem4;
        mem4 = mem3;
        mem3 = mem2;
        mem2 = mem1;
        mem1 = mem0;
        mem0 = *v;
        *v = sum;
    }
}

/// Translation of `pitch_downsample()`.
pub(crate) fn pitch_downsample(x: [&[f32]; 2], x_lp: &mut [f32], len: i32, c: i32) {
    let mut ac = [0.0f32; 5];
    let mut tmp = Q15ONE;
    let mut lpc = [0.0f32; 4];
    let mut lpc2 = [0.0f32; 5];
    let c1: f32 = 0.8;
    let half = (len >> 1) as usize;
    for i in 1..half {
        x_lp[i] = 0.25f32 * x[0][2 * i - 1] + 0.25f32 * x[0][2 * i + 1] + 0.5f32 * x[0][2 * i];
    }
    x_lp[0] = 0.25f32 * x[0][1] + 0.5f32 * x[0][0];
    if c == 2 {
        for i in 1..half {
            x_lp[i] += 0.25f32 * x[1][2 * i - 1] + 0.25f32 * x[1][2 * i + 1] + 0.5f32 * x[1][2 * i];
        }
        x_lp[0] += 0.25f32 * x[1][1] + 0.5f32 * x[1][0];
    }
    _celt_autocorr(x_lp, &mut ac, None, 0, 4, len >> 1);

    /* Noise floor -40 dB */
    ac[0] *= 1.0001f32;
    /* Lag windowing */
    for (i, a) in ac.iter_mut().enumerate().skip(1) {
        /*ac[i] *= exp(-.5*(2*M_PI*.002*i)*(2*M_PI*.002*i));*/
        *a -= *a * (0.008f32 * i as f32) * (0.008f32 * i as f32);
    }

    _celt_lpc(&mut lpc, &ac, 4);
    for l in lpc.iter_mut() {
        tmp *= 0.9f32;
        *l *= tmp;
    }
    /* Add a zero */
    lpc2[0] = lpc[0] + 0.8f32;
    lpc2[1] = lpc[1] + c1 * lpc[0];
    lpc2[2] = lpc[2] + c1 * lpc[1];
    lpc2[3] = lpc[3] + c1 * lpc[2];
    lpc2[4] = c1 * lpc[3];
    celt_fir5(x_lp, &lpc2, len >> 1);
}

/* Pure C implementation. */
/// Translation of `celt_pitch_xcorr_c()` (the unrolled version).
pub(crate) fn celt_pitch_xcorr(x: &[f32], y: &[f32], xcorr: &mut [f32], len: i32, max_pitch: i32) {
    /*The EDSP version requires that max_pitch is at least 1, and that _x is
    32-bit aligned.
    Since it's hard to put asserts in assembly, put them here.*/
    debug_assert!(max_pitch > 0);
    let mut i = 0i32;
    while i < max_pitch - 3 {
        let mut sum = [0.0f32; 4];
        xcorr_kernel(x, &y[i as usize..], &mut sum, len);
        xcorr[i as usize] = sum[0];
        xcorr[i as usize + 1] = sum[1];
        xcorr[i as usize + 2] = sum[2];
        xcorr[i as usize + 3] = sum[3];
        i += 4;
    }
    /* In case max_pitch isn't a multiple of 4, do non-unrolled version. */
    while i < max_pitch {
        let sum = celt_inner_prod(x, &y[i as usize..], len);
        xcorr[i as usize] = sum;
        i += 1;
    }
}

/// Translation of `pitch_search()`.
pub(crate) fn pitch_search(x_lp: &[f32], y: &[f32], len: i32, max_pitch: i32, pitch: &mut i32) {
    let mut best_pitch = [0i32; 2];

    debug_assert!(len > 0);
    debug_assert!(max_pitch > 0);
    let lag = len + max_pitch;

    let mut x_lp4 = vec![0.0f32; (len >> 2) as usize];
    let mut y_lp4 = vec![0.0f32; (lag >> 2) as usize];
    let mut xcorr = vec![0.0f32; (max_pitch >> 1) as usize];

    /* Downsample by 2 again */
    for (j, v) in x_lp4.iter_mut().enumerate() {
        *v = x_lp[2 * j];
    }
    for (j, v) in y_lp4.iter_mut().enumerate() {
        *v = y[2 * j];
    }

    /* Coarse search with 4x decimation */

    celt_pitch_xcorr(&x_lp4, &y_lp4, &mut xcorr, len >> 2, max_pitch >> 2);

    find_best_pitch(&xcorr, &y_lp4, len >> 2, max_pitch >> 2, &mut best_pitch);

    /* Finer search with 2x decimation */
    for i in 0..(max_pitch >> 1) {
        xcorr[i as usize] = 0.0;
        if (i - 2 * best_pitch[0]).abs() > 2 && (i - 2 * best_pitch[1]).abs() > 2 {
            continue;
        }
        let sum = celt_inner_prod(x_lp, &y[i as usize..], len >> 1);
        xcorr[i as usize] = max32(-1.0, sum);
    }
    find_best_pitch(&xcorr, y, len >> 1, max_pitch >> 1, &mut best_pitch);

    /* Refine by pseudo-interpolation */
    let offset = if best_pitch[0] > 0 && best_pitch[0] < (max_pitch >> 1) - 1 {
        let a = xcorr[best_pitch[0] as usize - 1];
        let b = xcorr[best_pitch[0] as usize];
        let c = xcorr[best_pitch[0] as usize + 1];
        if (c - a) > 0.7f32 * (b - a) {
            1
        } else if (a - c) > 0.7f32 * (b - c) {
            -1
        } else {
            0
        }
    } else {
        0
    };
    *pitch = 2 * best_pitch[0] - offset;
}
