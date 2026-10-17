// Rust translation of lib/mdct.c and lib/mdct.h (the float transform) from
// libvorbis 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2009
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: normalized modified discrete cosine transform
//!            power of two length transform only [64 <= n ]
//!
//! Original algorithm adapted long ago from _The use of multirate filter
//! banks for coding of high quality digital audio_, by T. Sporer,
//! K. Brandenburg and B. Edler, collection of the European Signal
//! Processing Conference (EUSIPCO), Amsterdam, June 1992, Vol.1, pp
//! 211-214
//!
//! The below code implements an algorithm that no longer looks much like
//! that presented in the paper, but the basic structure remains if you
//! dig deep enough to see it.
//!
//! This module DOES NOT INCLUDE code to generate/apply the window
//! function.  Everybody has their own weird favorite including me... I
//! happen to like the properties of y=sin(.5PI*sin^2(x)), but others may
//! vehemently disagree.
//!
//! The float transform (`MDCT_INTEGERIZED` isn't defined), as decoding
//! uses it: `mdct_forward()` is the encoder's and isn't translated. C's
//! pointers into the arrays are indices; `mdct_backward()` is only ever
//! called in place, so it takes the one array.

#![allow(non_upper_case_globals)]

use sdl3::stdlib::math::{cos, log, sin};

const PI: f64 = std::f64::consts::PI;

const cPI3_8: f32 = 0.38268343236508977175;
const cPI2_8: f32 = 0.70710678118654752441;
const cPI1_8: f32 = 0.92387953251128675613;

/// Translation of `mdct_lookup`.
#[derive(Clone, Debug, Default)]
pub(crate) struct MdctLookup {
    pub n: i32,
    pub log2n: i32,

    pub trig: Vec<f32>,
    pub bitrev: Vec<i32>,

    pub scale: f32,
}

/* build lookups for trig functions; also pre-figure scaling and
some window function algebra. */

/// Translation of `mdct_init()`.
pub(crate) fn mdct_init(lookup: &mut MdctLookup, n: i32) {
    let mut bitrev = vec![0i32; (n / 4) as usize];
    let mut T = vec![0f32; (n + n / 4) as usize];

    let n2 = (n >> 1) as usize;
    let log2n = (log(n as f32 as f64) / log(2.0f32 as f64)).round_ties_even() as i32;
    lookup.log2n = log2n;
    lookup.n = n;

    /* trig lookups... */

    for i in 0..(n / 4) as usize {
        T[i * 2] = cos((PI / n as f64) * (4 * i) as f64) as f32;
        T[i * 2 + 1] = -sin((PI / n as f64) * (4 * i) as f64) as f32;
        T[n2 + i * 2] = cos((PI / (2 * n) as f64) * (2 * i + 1) as f64) as f32;
        T[n2 + i * 2 + 1] = sin((PI / (2 * n) as f64) * (2 * i + 1) as f64) as f32;
    }
    for i in 0..(n / 8) as usize {
        T[n as usize + i * 2] = (cos((PI / n as f64) * (4 * i + 2) as f64) * 0.5) as f32;
        T[n as usize + i * 2 + 1] = (-sin((PI / n as f64) * (4 * i + 2) as f64) * 0.5) as f32;
    }

    /* bitreverse lookup... */

    {
        let mask = (1 << (log2n - 1)) - 1;
        let msb = 1 << (log2n - 2);
        for i in 0..n / 8 {
            let mut acc = 0;
            let mut j = 0;
            while (msb >> j) != 0 {
                if (msb >> j) & i != 0 {
                    acc |= 1 << j;
                }
                j += 1;
            }
            bitrev[(i * 2) as usize] = ((!acc) & mask) - 1;
            bitrev[(i * 2 + 1) as usize] = acc;
        }
    }
    lookup.scale = 4.0f32 / n as f32;
    lookup.trig = T;
    lookup.bitrev = bitrev;
}

/* 8 point butterfly (in place, 4 register) */
/// Translation of `mdct_butterfly_8()`.
#[inline]
fn mdct_butterfly_8(x: &mut [f32]) {
    let mut r0 = x[6] + x[2];
    let mut r1 = x[6] - x[2];
    let mut r2 = x[4] + x[0];
    let r3 = x[4] - x[0];

    x[6] = r0 + r2;
    x[4] = r0 - r2;

    r0 = x[5] - x[1];
    r2 = x[7] - x[3];
    x[0] = r1 + r0;
    x[2] = r1 - r0;

    r0 = x[5] + x[1];
    r1 = x[7] + x[3];
    x[3] = r2 + r3;
    x[1] = r2 - r3;
    x[7] = r1 + r0;
    x[5] = r1 - r0;
}

/* 16 point butterfly (in place, 4 register) */
/// Translation of `mdct_butterfly_16()`.
#[inline]
fn mdct_butterfly_16(x: &mut [f32]) {
    let mut r0 = x[1] - x[9];
    let mut r1 = x[0] - x[8];

    x[8] += x[0];
    x[9] += x[1];
    x[0] = (r0 + r1) * cPI2_8;
    x[1] = (r0 - r1) * cPI2_8;

    r0 = x[3] - x[11];
    r1 = x[10] - x[2];
    x[10] += x[2];
    x[11] += x[3];
    x[2] = r0;
    x[3] = r1;

    r0 = x[12] - x[4];
    r1 = x[13] - x[5];
    x[12] += x[4];
    x[13] += x[5];
    x[4] = (r0 - r1) * cPI2_8;
    x[5] = (r0 + r1) * cPI2_8;

    r0 = x[14] - x[6];
    r1 = x[15] - x[7];
    x[14] += x[6];
    x[15] += x[7];
    x[6] = r0;
    x[7] = r1;

    mdct_butterfly_8(&mut x[..8]);
    mdct_butterfly_8(&mut x[8..16]);
}

/* 32 point butterfly (in place, 4 register) */
/// Translation of `mdct_butterfly_32()`.
#[inline]
fn mdct_butterfly_32(x: &mut [f32]) {
    let mut r0 = x[30] - x[14];
    let mut r1 = x[31] - x[15];

    x[30] += x[14];
    x[31] += x[15];
    x[14] = r0;
    x[15] = r1;

    r0 = x[28] - x[12];
    r1 = x[29] - x[13];
    x[28] += x[12];
    x[29] += x[13];
    x[12] = r0 * cPI1_8 - r1 * cPI3_8;
    x[13] = r0 * cPI3_8 + r1 * cPI1_8;

    r0 = x[26] - x[10];
    r1 = x[27] - x[11];
    x[26] += x[10];
    x[27] += x[11];
    x[10] = (r0 - r1) * cPI2_8;
    x[11] = (r0 + r1) * cPI2_8;

    r0 = x[24] - x[8];
    r1 = x[25] - x[9];
    x[24] += x[8];
    x[25] += x[9];
    x[8] = r0 * cPI3_8 - r1 * cPI1_8;
    x[9] = r1 * cPI3_8 + r0 * cPI1_8;

    r0 = x[22] - x[6];
    r1 = x[7] - x[23];
    x[22] += x[6];
    x[23] += x[7];
    x[6] = r1;
    x[7] = r0;

    r0 = x[4] - x[20];
    r1 = x[5] - x[21];
    x[20] += x[4];
    x[21] += x[5];
    x[4] = r1 * cPI1_8 + r0 * cPI3_8;
    x[5] = r1 * cPI3_8 - r0 * cPI1_8;

    r0 = x[2] - x[18];
    r1 = x[3] - x[19];
    x[18] += x[2];
    x[19] += x[3];
    x[2] = (r1 + r0) * cPI2_8;
    x[3] = (r1 - r0) * cPI2_8;

    r0 = x[0] - x[16];
    r1 = x[1] - x[17];
    x[16] += x[0];
    x[17] += x[1];
    x[0] = r1 * cPI3_8 + r0 * cPI1_8;
    x[1] = r1 * cPI1_8 - r0 * cPI3_8;

    mdct_butterfly_16(&mut x[..16]);
    mdct_butterfly_16(&mut x[16..32]);
}

/* N point first stage butterfly (in place, 2 register) */
/// Translation of `mdct_butterfly_first()`.
#[inline]
fn mdct_butterfly_first(T: &[f32], x: &mut [f32], points: i32) {
    let mut x1 = points as isize - 8;
    let mut x2 = (points >> 1) as isize - 8;
    let mut t = 0usize;
    let mut r0: f32;
    let mut r1: f32;

    loop {
        let (a, b) = (x1 as usize, x2 as usize);
        r0 = x[a + 6] - x[b + 6];
        r1 = x[a + 7] - x[b + 7];
        x[a + 6] += x[b + 6];
        x[a + 7] += x[b + 7];
        x[b + 6] = r1 * T[t + 1] + r0 * T[t];
        x[b + 7] = r1 * T[t] - r0 * T[t + 1];

        r0 = x[a + 4] - x[b + 4];
        r1 = x[a + 5] - x[b + 5];
        x[a + 4] += x[b + 4];
        x[a + 5] += x[b + 5];
        x[b + 4] = r1 * T[t + 5] + r0 * T[t + 4];
        x[b + 5] = r1 * T[t + 4] - r0 * T[t + 5];

        r0 = x[a + 2] - x[b + 2];
        r1 = x[a + 3] - x[b + 3];
        x[a + 2] += x[b + 2];
        x[a + 3] += x[b + 3];
        x[b + 2] = r1 * T[t + 9] + r0 * T[t + 8];
        x[b + 3] = r1 * T[t + 8] - r0 * T[t + 9];

        r0 = x[a] - x[b];
        r1 = x[a + 1] - x[b + 1];
        x[a] += x[b];
        x[a + 1] += x[b + 1];
        x[b] = r1 * T[t + 13] + r0 * T[t + 12];
        x[b + 1] = r1 * T[t + 12] - r0 * T[t + 13];

        x1 -= 8;
        x2 -= 8;
        t += 16;

        if x2 < 0 {
            break;
        }
    }
}

/* N/stage point generic N stage butterfly (in place, 2 register) */
/// Translation of `mdct_butterfly_generic()`.
#[inline]
fn mdct_butterfly_generic(T: &[f32], x: &mut [f32], points: i32, trigint: i32) {
    let mut x1 = points as isize - 8;
    let mut x2 = (points >> 1) as isize - 8;
    let mut t = 0usize;
    let trigint = trigint as usize;
    let mut r0: f32;
    let mut r1: f32;

    loop {
        let (a, b) = (x1 as usize, x2 as usize);
        r0 = x[a + 6] - x[b + 6];
        r1 = x[a + 7] - x[b + 7];
        x[a + 6] += x[b + 6];
        x[a + 7] += x[b + 7];
        x[b + 6] = r1 * T[t + 1] + r0 * T[t];
        x[b + 7] = r1 * T[t] - r0 * T[t + 1];

        t += trigint;

        r0 = x[a + 4] - x[b + 4];
        r1 = x[a + 5] - x[b + 5];
        x[a + 4] += x[b + 4];
        x[a + 5] += x[b + 5];
        x[b + 4] = r1 * T[t + 1] + r0 * T[t];
        x[b + 5] = r1 * T[t] - r0 * T[t + 1];

        t += trigint;

        r0 = x[a + 2] - x[b + 2];
        r1 = x[a + 3] - x[b + 3];
        x[a + 2] += x[b + 2];
        x[a + 3] += x[b + 3];
        x[b + 2] = r1 * T[t + 1] + r0 * T[t];
        x[b + 3] = r1 * T[t] - r0 * T[t + 1];

        t += trigint;

        r0 = x[a] - x[b];
        r1 = x[a + 1] - x[b + 1];
        x[a] += x[b];
        x[a + 1] += x[b + 1];
        x[b] = r1 * T[t + 1] + r0 * T[t];
        x[b + 1] = r1 * T[t] - r0 * T[t + 1];

        t += trigint;
        x1 -= 8;
        x2 -= 8;

        if x2 < 0 {
            break;
        }
    }
}

/// Translation of `mdct_butterflies()`.
#[inline]
fn mdct_butterflies(init: &MdctLookup, x: &mut [f32], points: i32) {
    let T = &init.trig[..];
    let mut stages = init.log2n - 5;

    stages -= 1;
    if stages > 0 {
        mdct_butterfly_first(T, x, points);
    }

    let mut i = 1;
    loop {
        stages -= 1;
        if stages <= 0 {
            break;
        }
        for j in 0..(1 << i) {
            let start = ((points >> i) * j) as usize;
            mdct_butterfly_generic(T, &mut x[start..], points >> i, 4 << i);
        }
        i += 1;
    }

    let mut j = 0;
    while j < points as usize {
        mdct_butterfly_32(&mut x[j..j + 32]);
        j += 32;
    }
}

/// Translation of `mdct_bitreverse()`.
#[inline]
fn mdct_bitreverse(init: &MdctLookup, x: &mut [f32]) {
    let n = init.n as usize;
    let bit = &init.bitrev[..];
    let mut b = 0usize;
    let mut w0 = 0usize;
    let xo = n >> 1;
    let mut w1 = xo;
    let T = &init.trig[n..];
    let mut t = 0usize;

    loop {
        let mut x0 = xo + bit[b] as usize;
        let mut x1 = xo + bit[b + 1] as usize;

        let mut r0 = x[x0 + 1] - x[x1 + 1];
        let mut r1 = x[x0] + x[x1];
        let r2 = r1 * T[t] + r0 * T[t + 1];
        let r3 = r1 * T[t + 1] - r0 * T[t];

        w1 -= 4;

        r0 = (x[x0 + 1] + x[x1 + 1]) * 0.5;
        r1 = (x[x0] - x[x1]) * 0.5;

        x[w0] = r0 + r2;
        x[w1 + 2] = r0 - r2;
        x[w0 + 1] = r1 + r3;
        x[w1 + 3] = r3 - r1;

        x0 = xo + bit[b + 2] as usize;
        x1 = xo + bit[b + 3] as usize;

        r0 = x[x0 + 1] - x[x1 + 1];
        r1 = x[x0] + x[x1];
        let r2 = r1 * T[t + 2] + r0 * T[t + 3];
        let r3 = r1 * T[t + 3] - r0 * T[t + 2];

        r0 = (x[x0 + 1] + x[x1 + 1]) * 0.5;
        r1 = (x[x0] - x[x1]) * 0.5;

        x[w0 + 2] = r0 + r2;
        x[w1] = r0 - r2;
        x[w0 + 3] = r1 + r3;
        x[w1 + 1] = r3 - r1;

        t += 4;
        b += 4;
        w0 += 4;

        if w0 >= w1 {
            break;
        }
    }
}

/// Translation of `mdct_backward()`, in place (C's `in` and `out` are
/// `x`).
pub(crate) fn mdct_backward(init: &MdctLookup, x: &mut [f32]) {
    let n = init.n as usize;
    let n2 = n >> 1;
    let n4 = n >> 2;

    /* rotate */

    let mut iX = n2 as isize - 7;
    let mut oX = n2 + n4;
    let T = &init.trig[..];
    let mut t = n4;

    loop {
        let i = iX as usize;
        oX -= 4;
        x[oX] = -x[i + 2] * T[t + 3] - x[i] * T[t + 2];
        x[oX + 1] = x[i] * T[t + 3] - x[i + 2] * T[t + 2];
        x[oX + 2] = -x[i + 6] * T[t + 1] - x[i + 4] * T[t];
        x[oX + 3] = x[i + 4] * T[t + 1] - x[i + 6] * T[t];
        iX -= 8;
        t += 4;
        if iX < 0 {
            break;
        }
    }

    iX = n2 as isize - 8;
    oX = n2 + n4;
    t = n4;

    loop {
        let i = iX as usize;
        t -= 4;
        x[oX] = x[i + 4] * T[t + 3] + x[i + 6] * T[t + 2];
        x[oX + 1] = x[i + 4] * T[t + 2] - x[i + 6] * T[t + 3];
        x[oX + 2] = x[i] * T[t + 1] + x[i + 2] * T[t];
        x[oX + 3] = x[i] * T[t] - x[i + 2] * T[t + 1];
        iX -= 8;
        oX += 4;
        if iX < 0 {
            break;
        }
    }

    mdct_butterflies(init, &mut x[n2..], n2 as i32);
    mdct_bitreverse(init, x);

    /* roatate + window */

    {
        let mut oX1 = n2 + n4;
        let mut oX2 = n2 + n4;
        let mut iX = 0usize;
        let mut t = n2;

        loop {
            oX1 -= 4;

            x[oX1 + 3] = x[iX] * T[t + 1] - x[iX + 1] * T[t];
            x[oX2] = -(x[iX] * T[t] + x[iX + 1] * T[t + 1]);

            x[oX1 + 2] = x[iX + 2] * T[t + 3] - x[iX + 3] * T[t + 2];
            x[oX2 + 1] = -(x[iX + 2] * T[t + 2] + x[iX + 3] * T[t + 3]);

            x[oX1 + 1] = x[iX + 4] * T[t + 5] - x[iX + 5] * T[t + 4];
            x[oX2 + 2] = -(x[iX + 4] * T[t + 4] + x[iX + 5] * T[t + 5]);

            x[oX1] = x[iX + 6] * T[t + 7] - x[iX + 7] * T[t + 6];
            x[oX2 + 3] = -(x[iX + 6] * T[t + 6] + x[iX + 7] * T[t + 7]);

            oX2 += 4;
            iX += 8;
            t += 8;
            if iX >= oX1 {
                break;
            }
        }

        iX = n2 + n4;
        oX1 = n4;
        oX2 = oX1;

        loop {
            oX1 -= 4;
            iX -= 4;

            x[oX1 + 3] = x[iX + 3];
            x[oX2] = -x[oX1 + 3];
            x[oX1 + 2] = x[iX + 2];
            x[oX2 + 1] = -x[oX1 + 2];
            x[oX1 + 1] = x[iX + 1];
            x[oX2 + 2] = -x[oX1 + 1];
            x[oX1] = x[iX];
            x[oX2 + 3] = -x[oX1];

            oX2 += 4;
            if oX2 >= iX {
                break;
            }
        }

        iX = n2 + n4;
        oX1 = n2 + n4;
        oX2 = n2;
        loop {
            oX1 -= 4;
            x[oX1] = x[iX + 3];
            x[oX1 + 1] = x[iX + 2];
            x[oX1 + 2] = x[iX + 1];
            x[oX1 + 3] = x[iX];
            iX += 4;
            if oX1 <= oX2 {
                break;
            }
        }
    }
}
