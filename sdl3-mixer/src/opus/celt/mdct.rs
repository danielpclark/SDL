// Rust translation of celt/mdct.h and celt/mdct.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2008 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a simple MDCT implementation that uses a N/4 complex FFT
//! to do most of the work. It should be relatively straightforward to
//! plug in pretty much and FFT here.
//!
//! This replaces the Vorbis FFT (and uses the exact same API), which
//! was a bit too messy and that was ending up duplicating code
//! (might as well use the same FFT everywhere).
//!
//! The algorithm is similar to (and inspired from) Fabrice Bellard's
//! MDCT implementation in FFMPEG, but has differences in signs, ordering
//! and scaling in many places.
//!
//! Only the backward MDCT (`clt_mdct_backward_c()`), which the decoder
//! uses, is translated; the lookup is the static mode's.

use super::kiss_fft::{opus_fft_impl, KissFftState};

/// Translation of `mdct_lookup`.
pub(crate) struct MdctLookup {
    pub n: i32,
    #[allow(dead_code)]
    pub maxshift: i32,
    pub kfft: [&'static KissFftState; 4],
    pub trig: &'static [f32],
}

/* Compute a backward MDCT (no scaling) and performs weighted overlap-add
(scales implicitly by 1/2) */
/// Translation of `clt_mdct_backward_c()`. `input[in_off..]` is `in`;
/// `out` starts at `out`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn clt_mdct_backward(
    l: &MdctLookup,
    input: &[f32],
    in_off: usize,
    out: &mut [f32],
    window: &[f32],
    overlap: i32,
    shift: i32,
    stride: i32,
) {
    let mut n = l.n;
    let mut trig_off = 0usize;
    for _ in 0..shift {
        n >>= 1;
        trig_off += n as usize;
    }
    let trig = &l.trig[trig_off..];
    let n2 = n >> 1;
    let n4 = n >> 2;
    let stride = stride as usize;
    let half = (overlap >> 1) as usize;

    /* Pre-rotate */
    {
        /* Temp pointers to make it really clear to the compiler what we're doing */
        let mut xp1 = in_off;
        let mut xp2 = in_off + stride * (n2 as usize - 1);
        let yp = &mut out[half..];
        let t = trig;
        let bitrev = l.kfft[shift as usize].bitrev;
        for i in 0..n4 as usize {
            let rev = bitrev[i] as usize;
            let yr = input[xp2] * t[i] + input[xp1] * t[n4 as usize + i];
            let yi = input[xp1] * t[i] - input[xp2] * t[n4 as usize + i];
            /* We swap real and imag because we use an FFT instead of an IFFT. */
            yp[2 * rev + 1] = yr;
            yp[2 * rev] = yi;
            /* Storing the pre-rotation directly in the bitrev order. */
            xp1 += 2 * stride;
            xp2 = xp2.wrapping_sub(2 * stride);
        }
    }

    opus_fft_impl(l.kfft[shift as usize], &mut out[half..]);

    /* Post-rotate and de-shuffle from both ends of the buffer at once to make
    it in-place. */
    {
        let yp = &mut out[half..];
        let mut yp0 = 0usize;
        let mut yp1 = n2 as usize - 2;
        let t = trig;
        let n4u = n4 as usize;
        let n2u = n2 as usize;
        /* Loop to (N4+1)>>1 to handle odd N4. When N4 is odd, the
        middle pair will be computed twice. */
        for i in 0..((n4 + 1) >> 1) as usize {
            /* We swap real and imag because we're using an FFT instead of an IFFT. */
            let mut re = yp[yp0 + 1];
            let mut im = yp[yp0];
            let mut t0 = t[i];
            let mut t1 = t[n4u + i];
            /* We'd scale up by 2 here, but instead it's done when mixing the windows */
            let mut yr = re * t0 + im * t1;
            let mut yi = re * t1 - im * t0;
            /* We swap real and imag because we're using an FFT instead of an IFFT. */
            re = yp[yp1 + 1];
            im = yp[yp1];
            yp[yp0] = yr;
            yp[yp1 + 1] = yi;

            t0 = t[n4u - i - 1];
            t1 = t[n2u - i - 1];
            /* We'd scale up by 2 here, but instead it's done when mixing the windows */
            yr = re * t0 + im * t1;
            yi = re * t1 - im * t0;
            yp[yp1] = yr;
            yp[yp0 + 1] = yi;
            yp0 += 2;
            yp1 = yp1.wrapping_sub(2);
        }
    }

    /* Mirror on both sides for TDAC */
    {
        let mut xp1 = overlap as usize;
        let mut yp1 = 0usize;
        let mut wp1 = 0usize;
        let mut wp2 = overlap as usize;
        for _ in 0..overlap / 2 {
            xp1 -= 1;
            wp2 -= 1;
            let x1 = out[xp1];
            let x2 = out[yp1];
            out[yp1] = window[wp2] * x2 - window[wp1] * x1;
            yp1 += 1;
            out[xp1] = window[wp1] * x2 + window[wp2] * x1;
            wp1 += 1;
        }
    }
}
