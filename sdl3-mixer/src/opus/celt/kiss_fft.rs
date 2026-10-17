// Rust translation of celt/kiss_fft.h, celt/_kiss_fft_guts.h and
// celt/kiss_fft.c from libopus 1.4 (https://opus-codec.org, at the revision
// SDL_mixer's external/opus pins).
// Copyright (c) 2003-2004, Mark Borgerding
// Lots of modifications by Jean-Marc Valin
// Copyright (c) 2005-2007, Xiph.Org Foundation
// Copyright (c) 2008,      Xiph.Org Foundation, CSIRO
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The FFT: what the decoder's MDCT uses (`opus_fft_impl()`), in the float
//! build. This code is originally from Mark Borgerding's KISS-FFT but has
//! been heavily modified to better suit Opus.
//!
//! The state is always one of the static mode's (without `CUSTOM_MODES`,
//! `opus_fft_alloc()` and its bitrev/factor computations aren't built), and
//! the complex buffer is a slice of interleaved `r, i` floats, as the MDCT
//! casts its output buffer to `kiss_fft_cpx *`.

/// Translation of `kiss_fft_cpx`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct KissFftCpx {
    pub r: f32,
    pub i: f32,
}

/// Translation of `kiss_twiddle_cpx`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KissTwiddleCpx {
    pub r: f32,
    pub i: f32,
}

pub(crate) const MAXFACTORS: usize = 8;
/* e.g. an fft of length 128 has 4 factors
as far as kissfft is concerned
4*4*4*2
*/

/// Translation of `kiss_fft_state`.
pub(crate) struct KissFftState {
    #[allow(dead_code)]
    pub nfft: i32,
    #[allow(dead_code)]
    pub scale: f32,
    pub shift: i32,
    pub factors: [i16; 2 * MAXFACTORS],
    pub bitrev: &'static [i16],
    pub twiddles: &'static [KissTwiddleCpx],
}

/// `fout[k]`.
#[inline(always)]
fn ld(f: &[f32], k: usize) -> KissFftCpx {
    KissFftCpx {
        r: f[2 * k],
        i: f[2 * k + 1],
    }
}

/// `fout[k] = v`.
#[inline(always)]
fn st(f: &mut [f32], k: usize, v: KissFftCpx) {
    f[2 * k] = v.r;
    f[2 * k + 1] = v.i;
}

/// Translation of `C_MUL()`: `m = a*b`.
#[inline(always)]
fn c_mul(a: KissFftCpx, b: KissTwiddleCpx) -> KissFftCpx {
    KissFftCpx {
        r: a.r * b.r - a.i * b.i,
        i: a.r * b.i + a.i * b.r,
    }
}

/// Translation of `C_ADD()`.
#[inline(always)]
fn c_add(a: KissFftCpx, b: KissFftCpx) -> KissFftCpx {
    KissFftCpx {
        r: a.r + b.r,
        i: a.i + b.i,
    }
}

/// Translation of `C_SUB()`.
#[inline(always)]
fn c_sub(a: KissFftCpx, b: KissFftCpx) -> KissFftCpx {
    KissFftCpx {
        r: a.r - b.r,
        i: a.i - b.i,
    }
}

/* The guts header contains all the multiplication and addition macros that are defined for
complex numbers.  It also delares the kf_ internal functions.
*/

fn kf_bfly2(fout: &mut [f32], m: i32, n: i32) {
    let _ = m;
    let tw: f32 = 0.7071067812;
    /* We know that m==4 here because the radix-2 is just after a radix-4 */
    debug_assert!(m == 4);
    let mut f = 0usize;
    for _ in 0..n {
        let f2 = f + 4;
        let mut t = ld(fout, f2);
        st(fout, f2, c_sub(ld(fout, f), t));
        st(fout, f, c_add(ld(fout, f), t));

        let v = ld(fout, f2 + 1);
        t.r = (v.r + v.i) * tw;
        t.i = (v.i - v.r) * tw;
        st(fout, f2 + 1, c_sub(ld(fout, f + 1), t));
        st(fout, f + 1, c_add(ld(fout, f + 1), t));

        let v = ld(fout, f2 + 2);
        t.r = v.i;
        t.i = -v.r;
        st(fout, f2 + 2, c_sub(ld(fout, f + 2), t));
        st(fout, f + 2, c_add(ld(fout, f + 2), t));

        let v = ld(fout, f2 + 3);
        t.r = (v.i - v.r) * tw;
        t.i = (-(v.i + v.r)) * tw;
        st(fout, f2 + 3, c_sub(ld(fout, f + 3), t));
        st(fout, f + 3, c_add(ld(fout, f + 3), t));
        f += 8;
    }
}

fn kf_bfly4(fout: &mut [f32], fstride: usize, st_: &KissFftState, m: i32, n: i32, mm: i32) {
    if m == 1 {
        /* Degenerate case where all the twiddles are 1. */
        let mut f = 0usize;
        for _ in 0..n {
            let scratch0 = c_sub(ld(fout, f), ld(fout, f + 2));
            st(fout, f, c_add(ld(fout, f), ld(fout, f + 2)));
            let mut scratch1 = c_add(ld(fout, f + 1), ld(fout, f + 3));
            st(fout, f + 2, c_sub(ld(fout, f), scratch1));
            st(fout, f, c_add(ld(fout, f), scratch1));
            scratch1 = c_sub(ld(fout, f + 1), ld(fout, f + 3));

            st(
                fout,
                f + 1,
                KissFftCpx {
                    r: scratch0.r + scratch1.i,
                    i: scratch0.i - scratch1.r,
                },
            );
            st(
                fout,
                f + 3,
                KissFftCpx {
                    r: scratch0.r - scratch1.i,
                    i: scratch0.i + scratch1.r,
                },
            );
            f += 4;
        }
    } else {
        let m = m as usize;
        let m2 = 2 * m;
        let m3 = 3 * m;
        let tw = st_.twiddles;
        for i in 0..n as usize {
            let mut f = i * mm as usize;
            let (mut tw1, mut tw2, mut tw3) = (0usize, 0usize, 0usize);
            /* m is guaranteed to be a multiple of 4. */
            for _ in 0..m {
                let s0 = c_mul(ld(fout, f + m), tw[tw1]);
                let s1 = c_mul(ld(fout, f + m2), tw[tw2]);
                let s2 = c_mul(ld(fout, f + m3), tw[tw3]);

                let s5 = c_sub(ld(fout, f), s1);
                st(fout, f, c_add(ld(fout, f), s1));
                let s3 = c_add(s0, s2);
                let s4 = c_sub(s0, s2);
                st(fout, f + m2, c_sub(ld(fout, f), s3));
                tw1 += fstride;
                tw2 += fstride * 2;
                tw3 += fstride * 3;
                st(fout, f, c_add(ld(fout, f), s3));

                st(
                    fout,
                    f + m,
                    KissFftCpx {
                        r: s5.r + s4.i,
                        i: s5.i - s4.r,
                    },
                );
                st(
                    fout,
                    f + m3,
                    KissFftCpx {
                        r: s5.r - s4.i,
                        i: s5.i + s4.r,
                    },
                );
                f += 1;
            }
        }
    }
}

fn kf_bfly3(fout: &mut [f32], fstride: usize, st_: &KissFftState, m: i32, n: i32, mm: i32) {
    let m = m as usize;
    let m2 = 2 * m;
    let tw = st_.twiddles;
    let epi3 = tw[fstride * m];
    for i in 0..n as usize {
        let mut f = i * mm as usize;
        let (mut tw1, mut tw2) = (0usize, 0usize);
        /* For non-custom modes, m is guaranteed to be a multiple of 4. */
        let mut k = m;
        loop {
            let s1 = c_mul(ld(fout, f + m), tw[tw1]);
            let s2 = c_mul(ld(fout, f + m2), tw[tw2]);

            let s3 = c_add(s1, s2);
            let mut s0 = c_sub(s1, s2);
            tw1 += fstride;
            tw2 += fstride * 2;

            let f0 = ld(fout, f);
            let mut fm = KissFftCpx {
                r: f0.r - s3.r * 0.5,
                i: f0.i - s3.i * 0.5,
            };

            s0.r *= epi3.i;
            s0.i *= epi3.i;

            st(fout, f, c_add(f0, s3));

            st(
                fout,
                f + m2,
                KissFftCpx {
                    r: fm.r + s0.i,
                    i: fm.i - s0.r,
                },
            );

            fm.r -= s0.i;
            fm.i += s0.r;
            st(fout, f + m, fm);

            f += 1;
            k -= 1;
            if k == 0 {
                break;
            }
        }
    }
}

fn kf_bfly5(fout: &mut [f32], fstride: usize, st_: &KissFftState, m: i32, n: i32, mm: i32) {
    let m = m as usize;
    let tw = st_.twiddles;
    let ya = tw[fstride * m];
    let yb = tw[fstride * 2 * m];

    for i in 0..n as usize {
        let f = i * mm as usize;
        let (mut f0, mut f1, mut f2, mut f3, mut f4) = (f, f + m, f + 2 * m, f + 3 * m, f + 4 * m);

        /* For non-custom modes, m is guaranteed to be a multiple of 4. */
        for u in 0..m {
            let s0 = ld(fout, f0);

            let s1 = c_mul(ld(fout, f1), tw[u * fstride]);
            let s2 = c_mul(ld(fout, f2), tw[2 * u * fstride]);
            let s3 = c_mul(ld(fout, f3), tw[3 * u * fstride]);
            let s4 = c_mul(ld(fout, f4), tw[4 * u * fstride]);

            let s7 = c_add(s1, s4);
            let s10 = c_sub(s1, s4);
            let s8 = c_add(s2, s3);
            let s9 = c_sub(s2, s3);

            st(
                fout,
                f0,
                KissFftCpx {
                    r: s0.r + (s7.r + s8.r),
                    i: s0.i + (s7.i + s8.i),
                },
            );

            let s5 = KissFftCpx {
                r: s0.r + (s7.r * ya.r + s8.r * yb.r),
                i: s0.i + (s7.i * ya.r + s8.i * yb.r),
            };

            let s6 = KissFftCpx {
                r: s10.i * ya.i + s9.i * yb.i,
                i: -(s10.r * ya.i + s9.r * yb.i),
            };

            st(fout, f1, c_sub(s5, s6));
            st(fout, f4, c_add(s5, s6));

            let s11 = KissFftCpx {
                r: s0.r + (s7.r * yb.r + s8.r * ya.r),
                i: s0.i + (s7.i * yb.r + s8.i * ya.r),
            };
            let s12 = KissFftCpx {
                r: s9.i * ya.i - s10.i * yb.i,
                i: s10.r * yb.i - s9.r * ya.i,
            };

            st(fout, f2, c_add(s11, s12));
            st(fout, f3, c_sub(s11, s12));

            f0 += 1;
            f1 += 1;
            f2 += 1;
            f3 += 1;
            f4 += 1;
        }
    }
}

/// Translation of `opus_fft_impl()`.
pub(crate) fn opus_fft_impl(st_: &KissFftState, fout: &mut [f32]) {
    let mut fstride = [0i32; MAXFACTORS];

    /* st->shift can be -1 */
    let shift = if st_.shift > 0 { st_.shift } else { 0 };

    fstride[0] = 1;
    let mut l = 0usize;
    loop {
        let p = st_.factors[2 * l] as i32;
        let m = st_.factors[2 * l + 1] as i32;
        fstride[l + 1] = fstride[l] * p;
        l += 1;
        if m == 1 {
            break;
        }
    }
    let mut m = st_.factors[2 * l - 1] as i32;
    for i in (0..l).rev() {
        let m2 = if i != 0 {
            st_.factors[2 * i - 1] as i32
        } else {
            1
        };
        match st_.factors[2 * i] {
            2 => kf_bfly2(fout, m, fstride[i]),
            4 => kf_bfly4(fout, (fstride[i] << shift) as usize, st_, m, fstride[i], m2),
            3 => kf_bfly3(fout, (fstride[i] << shift) as usize, st_, m, fstride[i], m2),
            5 => kf_bfly5(fout, (fstride[i] << shift) as usize, st_, m, fstride[i], m2),
            _ => {}
        }
        m = m2;
    }
}
