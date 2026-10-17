// Rust translation of celt/bands.h and celt/bands.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007-2008 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Copyright (c) 2008-2009 Gregory Maxwell
// Written by Jean-Marc Valin and Gregory Maxwell
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Band quantisation, as the decoder runs it: `quant_all_bands()` and the
//! functions under it with `encode` 0 and `resynth` 1 (the encoder's
//! branches, the energy analysis, `spreading_decision()`,
//! `hysteresis_decision()`, `intensity_stereo()`, `stereo_split()`,
//! `compute_channel_weights()` and the theta RDO aren't translated).
//! `DISABLE_UPDATE_DRAFT` is undefined, as upstream builds it.
//!
//! Where C passes pointers into the same buffers (the bands of `X_` and the
//! scratch band after them, the folding source and destination in `norm`),
//! the slices are split apart; the folding source (`lowband`), which can
//! overlap the band's output in `norm` (`lowband_out`), is passed as a
//! copy, since `quant_band()` only reads it before writing `lowband_out`
//! (and only changes it in place, without a scratch band, in the last band,
//! after which `norm` isn't read).

use super::arch::{max32, Q15ONE};
use super::entcode::{celt_sudiv, celt_udiv, ec_ilog, EcDec, BITRES};
use super::mathops::{celt_exp2, celt_rsqrt, celt_rsqrt_norm, celt_sqrt, isqrt32};
use super::modes::OpusCustomMode;
use super::pitch::dual_inner_prod;
use super::quant_bands::E_MEANS;
use super::rate::{
    bits2pulses, cache_bits, get_pulses, pulses2bits, QTHETA_OFFSET, QTHETA_OFFSET_TWOPHASE,
};
use super::vq::{alg_unquant, renormalise_vector};

pub(crate) const SPREAD_NONE: i32 = 0;
#[allow(dead_code)]
pub(crate) const SPREAD_LIGHT: i32 = 1;
pub(crate) const SPREAD_NORMAL: i32 = 2;
pub(crate) const SPREAD_AGGRESSIVE: i32 = 3;

/// Translation of `celt_lcg_rand()`.
#[inline]
pub(crate) fn celt_lcg_rand(seed: u32) -> u32 {
    1664525u32.wrapping_mul(seed).wrapping_add(1013904223)
}

/* Multiplies two 16-bit fractional values. Bit-exactness of this macro is important */
/// Translation of `FRAC_MUL16()` (from mathops.h).
#[inline(always)]
fn frac_mul16(a: i32, b: i32) -> i32 {
    (16384 + (a as i16 as i32) * (b as i16 as i32)) >> 15
}

/* This is a cos() approximation designed to be bit-exact on any platform. Bit exactness
with this approximation is important because it has an impact on the bit allocation */
/// Translation of `bitexact_cos()`.
pub(crate) fn bitexact_cos(x: i16) -> i16 {
    let tmp = (4096 + (x as i32) * (x as i32)) >> 13;
    let mut x2 = tmp as i16;
    x2 = ((32767 - x2 as i32)
        + frac_mul16(
            x2 as i32,
            -7651 + frac_mul16(x2 as i32, 8277 + frac_mul16(-626, x2 as i32)),
        )) as i16;
    (1 + x2 as i32) as i16
}

/// Translation of `bitexact_log2tan()`.
pub(crate) fn bitexact_log2tan(mut isin: i32, mut icos: i32) -> i32 {
    let lc = ec_ilog(icos as u32);
    let ls = ec_ilog(isin as u32);
    icos <<= 15 - lc;
    isin <<= 15 - ls;
    (ls - lc) * (1 << 11) + frac_mul16(isin, frac_mul16(isin, -2597) + 7932)
        - frac_mul16(icos, frac_mul16(icos, -2597) + 7932)
}

/* De-normalise the energy to produce the synthesis from the unit-energy bands */
/// Translation of `denormalise_bands()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn denormalise_bands(
    m: &OpusCustomMode,
    x: &[f32],
    freq: &mut [f32],
    band_log_e: &[f32],
    mut start: i32,
    mut end: i32,
    mm: i32,
    downsample: i32,
    silence: i32,
) {
    let ebands = m.ebands;
    let n = mm * m.short_mdct_size;
    let mut bound = mm * ebands[end as usize] as i32;
    if downsample != 1 {
        bound = bound.min(n / downsample);
    }
    if silence != 0 {
        bound = 0;
        start = 0;
        end = 0;
    }
    let mut f = 0usize;
    let mut xi = (mm * ebands[start as usize] as i32) as usize;
    for _ in 0..mm * ebands[start as usize] as i32 {
        freq[f] = 0.0;
        f += 1;
    }
    for i in start..end {
        let mut j = mm * ebands[i as usize] as i32;
        let band_end = mm * ebands[i as usize + 1] as i32;
        let lg = band_log_e[i as usize] + E_MEANS[i as usize];
        let g = celt_exp2(if 32.0f32 < lg { 32.0 } else { lg });
        /* Be careful of the fixed-point "else" just above when changing this code */
        loop {
            freq[f] = x[xi] * g;
            f += 1;
            xi += 1;
            j += 1;
            if j >= band_end {
                break;
            }
        }
    }
    debug_assert!(start <= end);
    freq[bound as usize..n as usize].fill(0.0);
}

/* This prevents energy collapse for transients with multiple short MDCTs */
/// Translation of `anti_collapse()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn anti_collapse(
    m: &OpusCustomMode,
    x_: &mut [f32],
    collapse_masks: &[u8],
    lm: i32,
    c: i32,
    size: i32,
    start: i32,
    end: i32,
    log_e: &[f32],
    prev1log_e: &[f32],
    prev2log_e: &[f32],
    pulses: &[i32],
    mut seed: u32,
) {
    let nb = m.nb_ebands as usize;
    for i in start as usize..end as usize {
        let n0 = (m.ebands[i + 1] - m.ebands[i]) as i32;
        /* depth in 1/8 bits */
        debug_assert!(pulses[i] >= 0);
        let depth = (celt_udiv(
            (1 + pulses[i]) as u32,
            (m.ebands[i + 1] - m.ebands[i]) as u32,
        ) >> lm) as i32;

        let thresh = 0.5f32 * celt_exp2(-0.125f32 * depth as f32);
        let sqrt_1 = celt_rsqrt((n0 << lm) as f64);

        for ci in 0..c as usize {
            let mut renormalize = false;
            let mut prev1 = prev1log_e[ci * nb + i];
            let mut prev2 = prev2log_e[ci * nb + i];
            if c == 1 {
                prev1 = super::arch::max16(prev1, prev1log_e[nb + i]);
                prev2 = super::arch::max16(prev2, prev2log_e[nb + i]);
            }
            let mut ediff = log_e[ci * nb + i] - super::arch::min16(prev1, prev2);
            ediff = max32(0.0, ediff);

            /* r needs to be multiplied by 2 or 2*sqrt(2) depending on LM because
            short blocks don't have the same energy as long */
            let mut r = 2.0f32 * celt_exp2(-ediff);
            if lm == 3 {
                r *= 1.41421356f32;
            }
            r = super::arch::min16(thresh, r);
            r *= sqrt_1;
            let xoff = ci * size as usize + ((m.ebands[i] as usize) << lm);
            let xs = &mut x_[xoff..];
            for k in 0..1usize << lm {
                /* Detect collapse */
                if collapse_masks[i * c as usize + ci] as u32 & (1u32 << k) == 0 {
                    /* Fill with noise */
                    for j in 0..n0 as usize {
                        seed = celt_lcg_rand(seed);
                        xs[(j << lm) + k] = if seed & 0x8000 != 0 { r } else { -r };
                    }
                    renormalize = true;
                }
            }
            /* We just added some energy, so we need to renormalise */
            if renormalize {
                renormalise_vector(xs, n0 << lm, Q15ONE);
            }
        }
    }
}

fn stereo_merge(x: &mut [f32], y: &mut [f32], mid: f32, n: i32) {
    /* Compute the norm of X+Y and X-Y as |X|^2 + |Y|^2 +/- sum(xy) */
    let (mut xp, side) = dual_inner_prod(y, x, y, n);
    /* Compensating for the mid normalization */
    xp *= mid;
    /* mid and side are in Q15, not Q14 like X and Y */
    let mid2 = mid;
    let el = mid2 * mid2 + side - 2.0 * xp;
    let er = mid2 * mid2 + side + 2.0 * xp;
    if er < 6e-4f32 || el < 6e-4f32 {
        let nu = n as usize;
        y[..nu].copy_from_slice(&x[..nu]);
        return;
    }

    let t = el;
    let lgain = celt_rsqrt_norm(t);
    let t = er;
    let rgain = celt_rsqrt_norm(t);

    for j in 0..n as usize {
        /* Apply mid scaling (side is already scaled) */
        let l = mid * x[j];
        let r = y[j];
        x[j] = lgain * (l - r);
        y[j] = rgain * (l + r);
    }
}

static ORDERY_TABLE: [i32; 30] = [
    1, 0, 3, 0, 2, 1, 7, 0, 4, 3, 6, 1, 5, 2, 15, 0, 8, 7, 12, 3, 11, 4, 14, 1, 9, 6, 13, 2, 10, 5,
];

fn deinterleave_hadamard(x: &mut [f32], n0: i32, stride: i32, hadamard: bool) {
    let n0 = n0 as usize;
    let stride = stride as usize;
    let n = n0 * stride;
    let mut tmp = vec![0.0f32; n];
    debug_assert!(stride > 0);
    if hadamard {
        let ordery = &ORDERY_TABLE[stride - 2..];
        for i in 0..stride {
            for j in 0..n0 {
                tmp[ordery[i] as usize * n0 + j] = x[j * stride + i];
            }
        }
    } else {
        for i in 0..stride {
            for j in 0..n0 {
                tmp[i * n0 + j] = x[j * stride + i];
            }
        }
    }
    x[..n].copy_from_slice(&tmp);
}

fn interleave_hadamard(x: &mut [f32], n0: i32, stride: i32, hadamard: bool) {
    let n0 = n0 as usize;
    let stride = stride as usize;
    let n = n0 * stride;
    let mut tmp = vec![0.0f32; n];
    if hadamard {
        let ordery = &ORDERY_TABLE[stride - 2..];
        for i in 0..stride {
            for j in 0..n0 {
                tmp[j * stride + i] = x[ordery[i] as usize * n0 + j];
            }
        }
    } else {
        for i in 0..stride {
            for j in 0..n0 {
                tmp[j * stride + i] = x[i * n0 + j];
            }
        }
    }
    x[..n].copy_from_slice(&tmp);
}

/// Translation of `haar1()`.
pub(crate) fn haar1(x: &mut [f32], mut n0: i32, stride: i32) {
    n0 >>= 1;
    let stride = stride as usize;
    for i in 0..stride {
        for j in 0..n0 as usize {
            let tmp1 = 0.70710678f32 * x[stride * 2 * j + i];
            let tmp2 = 0.70710678f32 * x[stride * (2 * j + 1) + i];
            x[stride * 2 * j + i] = tmp1 + tmp2;
            x[stride * (2 * j + 1) + i] = tmp1 - tmp2;
        }
    }
}

fn compute_qn(n: i32, b: i32, offset: i32, pulse_cap: i32, stereo: i32) -> i32 {
    const EXP2_TABLE8: [i16; 8] = [16384, 17866, 19483, 21247, 23170, 25267, 27554, 30048];
    let mut n2 = 2 * n - 1;
    if stereo != 0 && n == 2 {
        n2 -= 1;
    }
    /* The upper limit ensures that in a stereo split with itheta==16384, we'll
    always have enough bits left over to code at least one pulse in the
    side; otherwise it would collapse, since it doesn't get folded. */
    let mut qb = celt_sudiv(b.wrapping_add(n2.wrapping_mul(offset)), n2);
    qb = qb.min(b - pulse_cap - (4 << BITRES));

    qb = qb.min(8 << BITRES);

    let qn = if qb < (1 << BITRES >> 1) {
        1
    } else {
        let qn = EXP2_TABLE8[(qb & 0x7) as usize] as i32 >> (14 - (qb >> BITRES));
        (qn + 1) >> 1 << 1
    };
    debug_assert!(qn <= 256);
    qn
}

/// Translation of `struct band_ctx` (the decoder's fields).
struct BandCtx<'a, 'b> {
    m: &'static OpusCustomMode,
    i: i32,
    intensity: i32,
    spread: i32,
    tf_change: i32,
    ec: &'a mut EcDec<'b>,
    remaining_bits: i32,
    seed: u32,
    disable_inv: i32,
}

/// Translation of `struct split_ctx`.
#[derive(Default)]
struct SplitCtx {
    inv: i32,
    imid: i32,
    iside: i32,
    delta: i32,
    itheta: i32,
    qalloc: i32,
}

#[allow(clippy::too_many_arguments)]
fn compute_theta(
    ctx: &mut BandCtx<'_, '_>,
    sctx: &mut SplitCtx,
    n: i32,
    b: &mut i32,
    bb: i32,
    b0: i32,
    lm: i32,
    stereo: i32,
    fill: &mut i32,
) {
    let mut itheta: i32 = 0;
    let delta: i32;
    let imid: i32;
    let iside: i32;
    let mut inv: i32 = 0;

    let m = ctx.m;
    let i = ctx.i;
    let intensity = ctx.intensity;

    /* Decide on the resolution to give to the split parameter theta */
    let pulse_cap = m.log_n[i as usize] as i32 + lm * (1 << BITRES);
    let offset = (pulse_cap >> 1)
        - if stereo != 0 && n == 2 {
            QTHETA_OFFSET_TWOPHASE
        } else {
            QTHETA_OFFSET
        };
    let mut qn = compute_qn(n, *b, offset, pulse_cap, stereo);
    if stereo != 0 && i >= intensity {
        qn = 1;
    }
    let tell = ctx.ec.ec_tell_frac() as i32;
    if qn != 1 {
        /* Entropy coding of the angle. We use a uniform pdf for the
        time split, a step for stereo, and a triangular one for the rest. */
        if stereo != 0 && n > 2 {
            let p0 = 3;
            let x;
            let x0 = qn / 2;
            let ft = p0 * (x0 + 1) + x0;
            /* Use a probability of p0 up to itheta=8192 and then use 1 after */
            let fs = ctx.ec.ec_decode(ft as u32) as i32;
            if fs < (x0 + 1) * p0 {
                x = fs / p0;
            } else {
                x = x0 + 1 + (fs - (x0 + 1) * p0);
            }
            ctx.ec.ec_dec_update(
                (if x <= x0 {
                    p0 * x
                } else {
                    (x - 1 - x0) + (x0 + 1) * p0
                }) as u32,
                (if x <= x0 {
                    p0 * (x + 1)
                } else {
                    (x - x0) + (x0 + 1) * p0
                }) as u32,
                ft as u32,
            );
            itheta = x;
        } else if b0 > 1 || stereo != 0 {
            /* Uniform pdf */
            itheta = ctx.ec.ec_dec_uint((qn + 1) as u32) as i32;
        } else {
            let fs;
            let fl;
            let ft = ((qn >> 1) + 1) * ((qn >> 1) + 1);
            /* Triangular pdf */
            let fm = ctx.ec.ec_decode(ft as u32) as i32;

            if fm < ((qn >> 1) * ((qn >> 1) + 1) >> 1) {
                itheta = (isqrt32((8 * fm as u32).wrapping_add(1)).wrapping_sub(1) >> 1) as i32;
                fs = itheta + 1;
                fl = itheta * (itheta + 1) >> 1;
            } else {
                itheta = (((2 * (qn + 1)) as u32)
                    .wrapping_sub(isqrt32((8 * (ft - fm - 1) as u32).wrapping_add(1)))
                    >> 1) as i32;
                fs = qn + 1 - itheta;
                fl = ft - ((qn + 1 - itheta) * (qn + 2 - itheta) >> 1);
            }

            ctx.ec.ec_dec_update(fl as u32, (fl + fs) as u32, ft as u32);
        }
        debug_assert!(itheta >= 0);
        itheta = celt_udiv((itheta * 16384) as u32, qn as u32) as i32;
        /* NOTE: Renormalising X and Y *may* help fixed-point a bit at very high rate.
        Let's do that at higher complexity */
    } else if stereo != 0 {
        if *b > 2 << BITRES && ctx.remaining_bits > 2 << BITRES {
            inv = ctx.ec.ec_dec_bit_logp(2);
        } else {
            inv = 0;
        }
        /* inv flag override to avoid problems with downmixing. */
        if ctx.disable_inv != 0 {
            inv = 0;
        }
        itheta = 0;
    }
    let qalloc = ctx.ec.ec_tell_frac() as i32 - tell;
    *b -= qalloc;

    if itheta == 0 {
        imid = 32767;
        iside = 0;
        *fill &= (1 << bb) - 1;
        delta = -16384;
    } else if itheta == 16384 {
        imid = 0;
        iside = 32767;
        *fill &= ((1 << bb) - 1) << bb;
        delta = 16384;
    } else {
        imid = bitexact_cos(itheta as i16) as i32;
        iside = bitexact_cos((16384 - itheta) as i16) as i32;
        /* This is the mid vs side allocation that minimizes squared error
        in that band. */
        delta = frac_mul16((n - 1) << 7, bitexact_log2tan(iside, imid));
    }

    sctx.inv = inv;
    sctx.imid = imid;
    sctx.iside = iside;
    sctx.delta = delta;
    sctx.itheta = itheta;
    sctx.qalloc = qalloc;
}

fn quant_band_n1(
    ctx: &mut BandCtx<'_, '_>,
    x: &mut [f32],
    y: Option<&mut [f32]>,
    lowband_out: Option<&mut [f32]>,
) -> u32 {
    let stereo = y.is_some();
    let mut chans: [Option<&mut [f32]>; 2] = [Some(&mut *x), y];
    for c in 0..1 + stereo as usize {
        let mut sign = 0;
        if ctx.remaining_bits >= 1 << BITRES {
            sign = ctx.ec.ec_dec_bits(1);
            ctx.remaining_bits -= 1 << BITRES;
        }
        if let Some(xc) = chans[c].as_deref_mut() {
            xc[0] = if sign != 0 { -1.0 } else { 1.0 };
        }
    }
    if let Some(lo) = lowband_out {
        lo[0] = x[0];
    }
    1
}

/* This function is responsible for encoding and decoding a mono partition.
It can split the band in two and transmit the energy difference with
the two half-bands. It can be called recursively so bands can end up being
split in 8 parts. */
#[allow(clippy::too_many_arguments)]
fn quant_partition(
    ctx: &mut BandCtx<'_, '_>,
    x: &mut [f32],
    mut n: i32,
    mut b: i32,
    mut bb: i32,
    lowband: Option<&[f32]>,
    mut lm: i32,
    gain: f32,
    mut fill: i32,
) -> u32 {
    let b0 = bb;
    let mut cm: u32 = 0;
    let m = ctx.m;
    let i = ctx.i;
    let spread = ctx.spread;

    /* If we need 1.5 more bit than we can produce, split the band in two. */
    let split = lm != -1 && n > 2 && {
        let cache = cache_bits(m, lm + 1, i);
        b > cache[cache[0] as usize] as i32 + 12
    };
    if split {
        let mut sctx = SplitCtx::default();

        n >>= 1;
        let (xx, yy) = x.split_at_mut(n as usize);
        lm -= 1;
        if bb == 1 {
            fill = (fill & 1) | (fill << 1);
        }
        bb = (bb + 1) >> 1;

        compute_theta(ctx, &mut sctx, n, &mut b, bb, b0, lm, 0, &mut fill);
        let imid = sctx.imid;
        let iside = sctx.iside;
        let mut delta = sctx.delta;
        let itheta = sctx.itheta;
        let qalloc = sctx.qalloc;
        let mid = (1.0f32 / 32768.0) * imid as f32;
        let side = (1.0f32 / 32768.0) * iside as f32;

        /* Give more bits to low-energy MDCTs than they would otherwise deserve */
        if b0 > 1 && (itheta & 0x3fff) != 0 {
            if itheta > 8192 {
                /* Rough approximation for pre-echo masking */
                delta -= delta >> (4 - lm);
            } else {
                /* Corresponds to a forward-masking slope of 1.5 dB per 10 ms */
                delta = 0.min(delta + (n << BITRES >> (5 - lm)));
            }
        }
        let mut mbits = 0.max(b.min((b - delta) / 2));
        let mut sbits = b - mbits;
        ctx.remaining_bits -= qalloc;

        let next_lowband2 = lowband.map(|l| &l[n as usize..]); /* >32-bit split case */

        let mut rebalance = ctx.remaining_bits;
        if mbits >= sbits {
            cm = quant_partition(ctx, xx, n, mbits, bb, lowband, lm, gain * mid, fill);
            rebalance = mbits - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 0 {
                sbits += rebalance - (3 << BITRES);
            }
            cm |= quant_partition(
                ctx,
                yy,
                n,
                sbits,
                bb,
                next_lowband2,
                lm,
                gain * side,
                fill >> bb,
            ) << (b0 >> 1);
        } else {
            cm = quant_partition(
                ctx,
                yy,
                n,
                sbits,
                bb,
                next_lowband2,
                lm,
                gain * side,
                fill >> bb,
            ) << (b0 >> 1);
            rebalance = sbits - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 16384 {
                mbits += rebalance - (3 << BITRES);
            }
            cm |= quant_partition(ctx, xx, n, mbits, bb, lowband, lm, gain * mid, fill);
        }
    } else {
        /* This is the basic no-split case */
        let mut q = bits2pulses(m, i, lm, b);
        let mut curr_bits = pulses2bits(m, i, lm, q);
        ctx.remaining_bits -= curr_bits;

        /* Ensures we can never bust the budget */
        while ctx.remaining_bits < 0 && q > 0 {
            ctx.remaining_bits += curr_bits;
            q -= 1;
            curr_bits = pulses2bits(m, i, lm, q);
            ctx.remaining_bits -= curr_bits;
        }

        if q != 0 {
            let k = get_pulses(q);

            /* Finally do the actual quantization */
            cm = alg_unquant(x, n, k, spread, bb, ctx.ec, gain);
        } else {
            /* If there's no pulse, fill the band anyway */
            /* B can be as large as 16, so this shift might overflow an int on a
            16-bit platform; use a long to get defined behavior.*/
            let cm_mask = ((1u64 << bb) - 1) as u32;
            fill &= cm_mask as i32;
            let nu = n as usize;
            if fill == 0 {
                x[..nu].fill(0.0);
            } else {
                match lowband {
                    None => {
                        /* Noise */
                        for v in x[..nu].iter_mut() {
                            ctx.seed = celt_lcg_rand(ctx.seed);
                            *v = (ctx.seed as i32 >> 20) as f32;
                        }
                        cm = cm_mask;
                    }
                    Some(lowband) => {
                        /* Folded spectrum */
                        for j in 0..nu {
                            ctx.seed = celt_lcg_rand(ctx.seed);
                            /* About 48 dB below the "normal" folding level */
                            let mut tmp = 1.0f32 / 256.0;
                            tmp = if ctx.seed & 0x8000 != 0 { tmp } else { -tmp };
                            x[j] = lowband[j] + tmp;
                        }
                        cm = fill as u32;
                    }
                }
                renormalise_vector(x, n, gain);
            }
        }
    }

    cm
}

/* This function is responsible for encoding and decoding a band for the mono case. */
#[allow(clippy::too_many_arguments)]
fn quant_band<'l>(
    ctx: &mut BandCtx<'_, '_>,
    x: &mut [f32],
    n: i32,
    b: i32,
    mut bb: i32,
    mut lowband: Option<&'l mut [f32]>,
    lm: i32,
    lowband_out: Option<&mut [f32]>,
    gain: f32,
    lowband_scratch: Option<&'l mut [f32]>,
    mut fill: i32,
) -> u32 {
    let n0 = n;
    let mut n_b = n;
    let mut b0 = bb;
    let mut time_divide = 0;
    let mut recombine = 0;
    let mut cm: u32;
    let mut tf_change = ctx.tf_change;

    let long_blocks = b0 == 1;

    n_b = celt_udiv(n_b as u32, bb as u32) as i32;

    /* Special case for one sample */
    if n == 1 {
        return quant_band_n1(ctx, x, None, lowband_out);
    }

    if tf_change > 0 {
        recombine = tf_change;
    }
    /* Band recombining to increase frequency resolution */

    if let (Some(scratch), Some(lb)) = (lowband_scratch, lowband.as_deref()) {
        if recombine != 0 || ((n_b & 1) == 0 && tf_change < 0) || b0 > 1 {
            scratch[..n as usize].copy_from_slice(&lb[..n as usize]);
            lowband = Some(scratch);
        }
    }

    for k in 0..recombine {
        const BIT_INTERLEAVE_TABLE: [u8; 16] = [0, 1, 1, 1, 2, 3, 3, 3, 2, 3, 3, 3, 2, 3, 3, 3];
        if let Some(lb) = lowband.as_deref_mut() {
            haar1(lb, n >> k, 1 << k);
        }
        fill = BIT_INTERLEAVE_TABLE[(fill & 0xF) as usize] as i32
            | (BIT_INTERLEAVE_TABLE[(fill >> 4) as usize & 0xF] as i32) << 2;
    }
    bb >>= recombine;
    n_b <<= recombine;

    /* Increasing the time resolution */
    while (n_b & 1) == 0 && tf_change < 0 {
        if let Some(lb) = lowband.as_deref_mut() {
            haar1(lb, n_b, bb);
        }
        fill |= fill << bb;
        bb <<= 1;
        n_b >>= 1;
        time_divide += 1;
        tf_change += 1;
    }
    b0 = bb;
    let n_b0 = n_b;

    /* Reorganize the samples in time order instead of frequency order */
    if b0 > 1 {
        if let Some(lb) = lowband.as_deref_mut() {
            deinterleave_hadamard(lb, n_b >> recombine, b0 << recombine, long_blocks);
        }
    }

    cm = quant_partition(ctx, x, n, b, bb, lowband.as_deref(), lm, gain, fill);

    /* This code is used by the decoder and by the resynthesis-enabled encoder */
    {
        /* Undo the sample reorganization going from time order to frequency order */
        if b0 > 1 {
            interleave_hadamard(x, n_b >> recombine, b0 << recombine, long_blocks);
        }

        /* Undo time-freq changes that we did earlier */
        n_b = n_b0;
        bb = b0;
        for _ in 0..time_divide {
            bb >>= 1;
            n_b <<= 1;
            cm |= cm >> bb;
            haar1(x, n_b, bb);
        }

        for k in 0..recombine {
            const BIT_DEINTERLEAVE_TABLE: [u8; 16] = [
                0x00, 0x03, 0x0C, 0x0F, 0x30, 0x33, 0x3C, 0x3F, 0xC0, 0xC3, 0xCC, 0xCF, 0xF0, 0xF3,
                0xFC, 0xFF,
            ];
            cm = BIT_DEINTERLEAVE_TABLE[(cm & 0xF) as usize] as u32;
            haar1(x, n0 >> k, 1 << k);
        }
        bb <<= recombine;

        /* Scale output for later folding */
        if let Some(lo) = lowband_out {
            let nn = celt_sqrt(n0 as f64);
            for j in 0..n0 as usize {
                lo[j] = nn * x[j];
            }
        }
        cm &= ((1i64 << bb) - 1) as u32;
    }
    cm
}

/* This function is responsible for encoding and decoding a band for the stereo case. */
#[allow(clippy::too_many_arguments)]
fn quant_band_stereo(
    ctx: &mut BandCtx<'_, '_>,
    x: &mut [f32],
    y: &mut [f32],
    n: i32,
    mut b: i32,
    bb: i32,
    lowband: Option<&mut [f32]>,
    lm: i32,
    lowband_out: Option<&mut [f32]>,
    lowband_scratch: Option<&mut [f32]>,
    mut fill: i32,
) -> u32 {
    let mut cm: u32;
    let mbits: i32;
    let sbits: i32;
    let mut sctx = SplitCtx::default();

    /* Special case for one sample */
    if n == 1 {
        return quant_band_n1(ctx, x, Some(y), lowband_out);
    }

    let orig_fill = fill;

    compute_theta(ctx, &mut sctx, n, &mut b, bb, bb, lm, 1, &mut fill);
    let inv = sctx.inv;
    let imid = sctx.imid;
    let iside = sctx.iside;
    let delta = sctx.delta;
    let itheta = sctx.itheta;
    let qalloc = sctx.qalloc;
    let mid = (1.0f32 / 32768.0) * imid as f32;
    let side = (1.0f32 / 32768.0) * iside as f32;

    /* This is a special case for N=2 that only works for stereo and takes
    advantage of the fact that mid and side are orthogonal to encode
    the side with just one bit. */
    if n == 2 {
        let mut sign = 0;
        let mut mb = b;
        let mut sb = 0;
        /* Only need one bit for the side. */
        if itheta != 0 && itheta != 16384 {
            sb = 1 << BITRES;
        }
        mb -= sb;
        let c = itheta > 8192;
        ctx.remaining_bits -= qalloc + sb;

        if sb != 0 {
            sign = ctx.ec.ec_dec_bits(1) as i32;
        }
        sign = 1 - 2 * sign;
        {
            let (x2, y2): (&mut [f32], &mut [f32]) = if c {
                (&mut *y, &mut *x)
            } else {
                (&mut *x, &mut *y)
            };
            /* We use orig_fill here because we want to fold the side, but if
            itheta==16384, we'll have cleared the low bits of fill. */
            cm = quant_band(
                ctx,
                x2,
                n,
                mb,
                bb,
                lowband,
                lm,
                lowband_out,
                Q15ONE,
                lowband_scratch,
                orig_fill,
            );
            /* We don't split N=2 bands, so cm is either 1 or 0 (for a fold-collapse),
            and there's no need to worry about mixing with the other channel. */
            y2[0] = -sign as f32 * x2[1];
            y2[1] = sign as f32 * x2[0];
        }
        {
            x[0] *= mid;
            x[1] *= mid;
            y[0] *= side;
            y[1] *= side;
            let mut tmp = x[0];
            x[0] = tmp - y[0];
            y[0] = tmp + y[0];
            tmp = x[1];
            x[1] = tmp - y[1];
            y[1] = tmp + y[1];
        }
    } else {
        /* "Normal" split code */
        let mut mb = 0.max(b.min((b - delta) / 2));
        let mut sb = b - mb;
        ctx.remaining_bits -= qalloc;

        let mut rebalance = ctx.remaining_bits;
        if mb >= sb {
            /* In stereo mode, we do not apply a scaling to the mid because we need the normalized
            mid for folding later. */
            cm = quant_band(
                ctx,
                x,
                n,
                mb,
                bb,
                lowband,
                lm,
                lowband_out,
                Q15ONE,
                lowband_scratch,
                fill,
            );
            rebalance = mb - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 0 {
                sb += rebalance - (3 << BITRES);
            }

            /* For a stereo split, the high bits of fill are always zero, so no
            folding will be done to the side. */
            cm |= quant_band(ctx, y, n, sb, bb, None, lm, None, side, None, fill >> bb);
        } else {
            /* For a stereo split, the high bits of fill are always zero, so no
            folding will be done to the side. */
            cm = quant_band(ctx, y, n, sb, bb, None, lm, None, side, None, fill >> bb);
            rebalance = sb - (rebalance - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 16384 {
                mb += rebalance - (3 << BITRES);
            }
            /* In stereo mode, we do not apply a scaling to the mid because we need the normalized
            mid for folding later. */
            cm |= quant_band(
                ctx,
                x,
                n,
                mb,
                bb,
                lowband,
                lm,
                lowband_out,
                Q15ONE,
                lowband_scratch,
                fill,
            );
        }
        mbits = mb;
        sbits = sb;
        let _ = (mbits, sbits);
    }

    /* This code is used by the decoder and by the resynthesis-enabled encoder */
    {
        if n != 2 {
            stereo_merge(x, y, mid, n);
        }
        if inv != 0 {
            for v in y[..n as usize].iter_mut() {
                *v = -*v;
            }
        }
    }
    cm
}

fn special_hybrid_folding(
    m: &OpusCustomMode,
    norm: &mut [f32],
    norm2: &mut [f32],
    start: i32,
    mm: i32,
    dual_stereo: i32,
) {
    let ebands = m.ebands;
    let n1 = (mm * (ebands[start as usize + 1] - ebands[start as usize]) as i32) as usize;
    let n2 = (mm * (ebands[start as usize + 2] - ebands[start as usize + 1]) as i32) as usize;
    /* Duplicate enough of the first band folding data to be able to fold the second band.
    Copies no data for CELT-only mode. */
    if n2 > n1 {
        norm.copy_within(2 * n1 - n2..n1, n1);
        if dual_stereo != 0 {
            norm2.copy_within(2 * n1 - n2..n1, n1);
        }
    }
}

/// Translation of `quant_all_bands()`, decoding (`encode` 0): `x_` is
/// `X_` (holding the scratch band after the coded bands), `y_` is `Y_`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn quant_all_bands(
    m: &'static OpusCustomMode,
    start: i32,
    end: i32,
    x_: &mut [f32],
    mut y_: Option<&mut [f32]>,
    collapse_masks: &mut [u8],
    pulses: &[i32],
    short_blocks: i32,
    spread: i32,
    mut dual_stereo: i32,
    intensity: i32,
    tf_res: &[i32],
    total_bits: i32,
    mut balance: i32,
    ec: &mut EcDec<'_>,
    lm: i32,
    coded_bands: i32,
    seed: &mut u32,
    disable_inv: i32,
) {
    let ebands = m.ebands;
    let eb = |j: i32| ebands[j as usize] as i32;
    let mut update_lowband = true;
    let c: usize = if y_.is_some() { 2 } else { 1 };

    let mm = 1 << lm;
    let bb = if short_blocks != 0 { mm } else { 1 };
    let norm_offset = mm * eb(start);
    /* No need to allocate norm for the last band because we don't need an
    output in that band. */
    let norm_len = (mm * eb(m.nb_ebands - 1) - norm_offset) as usize;
    let mut norm_buf = vec![0.0f32; c * norm_len];
    let (norm, norm2) = norm_buf.split_at_mut(norm_len);

    /* For decoding, we can use the last band as scratch space because we don't need that
    scratch space for the last band and we don't care about the data there until we're
    decoding the last band. */
    let scratch_off = (mm * eb(m.eff_ebands - 1)) as usize;

    let mut lowband_offset = 0;
    let mut ctx = BandCtx {
        m,
        i: 0,
        intensity,
        spread,
        tf_change: 0,
        ec,
        remaining_bits: 0,
        seed: *seed,
        disable_inv,
    };
    for i in start..end {
        let iu = i as usize;
        let mut effective_lowband: i32 = -1;
        let x_cm: u32;
        let y_cm: u32;
        let mut x_cm_f: u32;
        let mut y_cm_f: u32;

        ctx.i = i;
        let last = i == end - 1;

        let n = mm * eb(i + 1) - mm * eb(i);
        debug_assert!(n > 0);
        let tell = ctx.ec.ec_tell_frac() as i32;

        /* Compute how many bits we want to allocate to this band */
        if i != start {
            balance -= tell;
        }
        let remaining_bits = total_bits - tell - 1;
        ctx.remaining_bits = remaining_bits;
        let b = if i <= coded_bands - 1 {
            let curr_balance = celt_sudiv(balance, 3.min(coded_bands - i));
            0.max(16383.min((remaining_bits + 1).min(pulses[iu] + curr_balance)))
        } else {
            0
        };

        if (mm * eb(i) - n >= mm * eb(start) || i == start + 1)
            && (update_lowband || lowband_offset == 0)
        {
            lowband_offset = i;
        }
        if i == start + 1 {
            special_hybrid_folding(m, norm, norm2, start, mm, dual_stereo);
        }

        let tf_change = tf_res[iu];
        ctx.tf_change = tf_change;
        // (i>=m->effEBands, where X and Y would fold into norm, can't
        // happen with the static mode, whose effEBands is nbEBands.)
        debug_assert!(i < m.eff_ebands);

        /* Get a conservative estimate of the collapse_mask's for the bands we're
        going to be folding from. */
        if lowband_offset != 0 && (spread != SPREAD_AGGRESSIVE || bb > 1 || tf_change < 0) {
            /* This ensures we never repeat spectral content within one band */
            effective_lowband = 0.max(mm * eb(lowband_offset) - norm_offset - n);
            let mut fold_start = lowband_offset;
            loop {
                fold_start -= 1;
                if mm * eb(fold_start) <= effective_lowband + norm_offset {
                    break;
                }
            }
            let mut fold_end = lowband_offset - 1;
            loop {
                fold_end += 1;
                if !(fold_end < i && mm * eb(fold_end) < effective_lowband + norm_offset + n) {
                    break;
                }
            }
            x_cm_f = 0;
            y_cm_f = 0;
            let mut fold_i = fold_start;
            loop {
                x_cm_f |= collapse_masks[fold_i as usize * c] as u32;
                y_cm_f |= collapse_masks[fold_i as usize * c + c - 1] as u32;
                fold_i += 1;
                if fold_i >= fold_end {
                    break;
                }
            }
        }
        /* Otherwise, we'll be using the LCG to fold, so all blocks will (almost
        always) be non-zero. */
        else {
            x_cm_f = ((1u64 << bb) - 1) as u32;
            y_cm_f = x_cm_f;
        }

        if dual_stereo != 0 && i == intensity {
            /* Switch off dual stereo to do intensity. */
            dual_stereo = 0;
            for j in 0..(mm * eb(i) - norm_offset) as usize {
                norm[j] = 0.5f32 * (norm[j] + norm2[j]);
            }
        }

        // The band, and the scratch band (the last coded band of X_), which
        // isn't used for the last band.
        let nu = n as usize;
        let xo = (mm * eb(i)) as usize;
        let (xband, mut scratch): (&mut [f32], Option<&mut [f32]>) = if last {
            (&mut x_[xo..xo + nu], None)
        } else {
            let (lo, hi) = x_.split_at_mut(scratch_off);
            (&mut lo[xo..xo + nu], Some(hi))
        };
        let out_off = (mm * eb(i) - norm_offset) as usize;
        let lowband_copy = |nb: &[f32]| -> Option<Vec<f32>> {
            if effective_lowband != -1 {
                let lo = effective_lowband as usize;
                Some(nb[lo..lo + nu].to_vec())
            } else {
                None
            }
        };

        if dual_stereo != 0 {
            let y = y_
                .as_deref_mut()
                .map(|y| &mut y[xo..xo + nu])
                .unwrap_or(&mut []);
            let mut lb = lowband_copy(norm);
            x_cm_f = quant_band(
                &mut ctx,
                xband,
                n,
                b / 2,
                bb,
                lb.as_deref_mut(),
                lm,
                if last {
                    None
                } else {
                    Some(&mut norm[out_off..out_off + nu])
                },
                Q15ONE,
                scratch.as_deref_mut(),
                x_cm_f as i32,
            );
            let mut lb2 = lowband_copy(norm2);
            y_cm_f = quant_band(
                &mut ctx,
                y,
                n,
                b / 2,
                bb,
                lb2.as_deref_mut(),
                lm,
                if last {
                    None
                } else {
                    Some(&mut norm2[out_off..out_off + nu])
                },
                Q15ONE,
                scratch.as_deref_mut(),
                y_cm_f as i32,
            );
            x_cm = x_cm_f;
            y_cm = y_cm_f;
        } else {
            let mut lb = lowband_copy(norm);
            let lowband_out = if last {
                None
            } else {
                Some(&mut norm[out_off..out_off + nu])
            };
            if let Some(y_) = y_.as_deref_mut() {
                let y = &mut y_[xo..xo + nu];
                x_cm = quant_band_stereo(
                    &mut ctx,
                    xband,
                    y,
                    n,
                    b,
                    bb,
                    lb.as_deref_mut(),
                    lm,
                    lowband_out,
                    scratch,
                    (x_cm_f | y_cm_f) as i32,
                );
            } else {
                x_cm = quant_band(
                    &mut ctx,
                    xband,
                    n,
                    b,
                    bb,
                    lb.as_deref_mut(),
                    lm,
                    lowband_out,
                    Q15ONE,
                    scratch,
                    (x_cm_f | y_cm_f) as i32,
                );
            }
            y_cm = x_cm;
        }
        collapse_masks[iu * c] = x_cm as u8;
        collapse_masks[iu * c + c - 1] = y_cm as u8;
        balance += pulses[iu] + tell;

        /* Update the folding position only as long as we have 1 bit/sample depth. */
        update_lowband = b > (n << BITRES);
    }
    *seed = ctx.seed;
}
