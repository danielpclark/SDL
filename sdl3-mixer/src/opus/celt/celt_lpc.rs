// Rust translation of celt/celt_lpc.h and celt/celt_lpc.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2009-2010 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! LPC analysis and filtering for the packet loss concealment, the float
//! versions (and not `SMALL_FOOTPRINT`, so `celt_iir()` is the unrolled
//! one), with the plain C kernels (see `pitch.rs`).

use super::mathops::frac_div32;
use super::pitch::{celt_pitch_xcorr, xcorr_kernel};

pub(crate) const LPC_ORDER: usize = 24;

/// Translation of `_celt_lpc()`.
pub(crate) fn _celt_lpc(
    lpc: &mut [f32], /* out: [0...p-1] LPC coefficients      */
    ac: &[f32],      /* in:  [0...p] autocorrelation values  */
    p: usize,
) {
    let mut error = ac[0];

    lpc[..p].fill(0.0);
    if ac[0] > 1e-10f32 {
        for i in 0..p {
            /* Sum up this iteration's reflection coefficient */
            let mut rr: f32 = 0.0;
            for j in 0..i {
                rr += lpc[j] * ac[i - j];
            }
            rr += ac[i + 1];
            let r = -frac_div32(rr, error);
            /*  Update LPC coefficients and total error */
            lpc[i] = r;
            for j in 0..(i + 1) >> 1 {
                let tmp1 = lpc[j];
                let tmp2 = lpc[i - 1 - j];
                lpc[j] = tmp1 + r * tmp2;
                lpc[i - 1 - j] = tmp2 + r * tmp1;
            }

            error = error - (r * r) * error;
            /* Bail out once we get 30 dB gain */
            if error <= 0.001f32 * ac[0] {
                break;
            }
        }
    }
}

/// Translation of `celt_fir_c()`: `x[x_off..]` is `x` (whose `ord`
/// preceding samples are read too), `y` is `y`.
pub(crate) fn celt_fir(x: &[f32], x_off: usize, num: &[f32], y: &mut [f32], n: i32, ord: usize) {
    let mut rnum = [0.0f32; LPC_ORDER];
    let rnum = &mut rnum[..ord];
    for i in 0..ord {
        rnum[i] = num[ord - i - 1];
    }
    let mut i = 0i32;
    while i < n - 3 {
        let iu = i as usize;
        let mut sum = [
            x[x_off + iu],
            x[x_off + iu + 1],
            x[x_off + iu + 2],
            x[x_off + iu + 3],
        ];
        xcorr_kernel(rnum, &x[x_off + iu - ord..], &mut sum, ord as i32);
        y[iu] = sum[0];
        y[iu + 1] = sum[1];
        y[iu + 2] = sum[2];
        y[iu + 3] = sum[3];
        i += 4;
    }
    while i < n {
        let iu = i as usize;
        let mut sum = x[x_off + iu];
        for j in 0..ord {
            sum += rnum[j] * x[x_off + iu + j - ord];
        }
        y[iu] = sum;
        i += 1;
    }
}

/// Translation of `celt_iir()`, which the decoder only calls in place
/// (`_x == _y`): `xy` is both.
pub(crate) fn celt_iir(xy: &mut [f32], den: &[f32], n: i32, ord: usize, mem: &mut [f32]) {
    debug_assert!((ord & 3) == 0);
    let nu = n as usize;
    let mut rden = [0.0f32; LPC_ORDER];
    let rden = &mut rden[..ord];
    let mut y = vec![0.0f32; nu + ord];
    for i in 0..ord {
        rden[i] = den[ord - i - 1];
    }
    for i in 0..ord {
        y[i] = -mem[ord - i - 1];
    }
    let mut i = 0usize;
    while (i as i32) < n - 3 {
        /* Unroll by 4 as if it were an FIR filter */
        let mut sum = [xy[i], xy[i + 1], xy[i + 2], xy[i + 3]];
        xcorr_kernel(rden, &y[i..], &mut sum, ord as i32);

        /* Patch up the result to compensate for the fact that this is an IIR */
        y[i + ord] = -sum[0];
        xy[i] = sum[0];
        sum[1] += y[i + ord] * den[0];
        y[i + ord + 1] = -sum[1];
        xy[i + 1] = sum[1];
        sum[2] += y[i + ord + 1] * den[0];
        sum[2] += y[i + ord] * den[1];
        y[i + ord + 2] = -sum[2];
        xy[i + 2] = sum[2];

        sum[3] += y[i + ord + 2] * den[0];
        sum[3] += y[i + ord + 1] * den[1];
        sum[3] += y[i + ord] * den[2];
        y[i + ord + 3] = -sum[3];
        xy[i + 3] = sum[3];
        i += 4;
    }
    // (N is always a multiple of 4 here, so this tail, which stores y[]
    // without the negation the unrolled loop uses, never runs.)
    while i < nu {
        let mut sum = xy[i];
        for j in 0..ord {
            sum -= rden[j] * y[i + j];
        }
        y[i + ord] = sum;
        xy[i] = sum;
        i += 1;
    }
    for i in 0..ord {
        mem[i] = xy[nu - i - 1];
    }
}

/// Translation of `_celt_autocorr()`: `ac[0..=lag]`.
pub(crate) fn _celt_autocorr(
    x: &[f32],      /*  in: [0...n-1] samples x   */
    ac: &mut [f32], /* out: [0...lag-1] ac values */
    window: Option<&[f32]>,
    overlap: usize,
    lag: i32,
    n: i32,
) -> i32 {
    let fast_n = n - lag;
    let nu = n as usize;
    debug_assert!(n > 0);
    let xx_buf;
    let xptr: &[f32] = if overlap == 0 {
        x
    } else {
        let window = window.unwrap_or(&[]);
        let mut xx = x[..nu].to_vec();
        for i in 0..overlap {
            xx[i] = x[i] * window[i];
            xx[nu - i - 1] = x[nu - i - 1] * window[i];
        }
        xx_buf = xx;
        &xx_buf
    };
    let shift = 0;
    celt_pitch_xcorr(xptr, xptr, ac, fast_n, lag + 1);
    for k in 0..=lag as usize {
        let mut d: f32 = 0.0;
        for i in k + fast_n as usize..nu {
            d += xptr[i] * xptr[i - k];
        }
        ac[k] += d;
    }

    shift
}
