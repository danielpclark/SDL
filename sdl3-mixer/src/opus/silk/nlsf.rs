// Rust translation of silk/NLSF_unpack.c, silk/NLSF_decode.c,
// silk/NLSF_stabilize.c, silk/sort.c (silk_insertion_sort_increasing_all_
// values_int16()), silk/NLSF2A.c, silk/LPC_fit.c, silk/bwexpander.c,
// silk/bwexpander_32.c and silk/LPC_inv_pred_gain.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! From the decoded NLSF indices to LPC filter coefficients.

use super::define::{
    MAX_LPC_ORDER, MAX_LPC_STABILIZE_ITERATIONS, MAX_PREDICTION_POWER_GAIN, NLSF_QUANT_LEVEL_ADJ,
    NLSF_QUANT_MAX_AMPLITUDE,
};
use super::sigproc_fix::*;
use super::tables::{SilkNlsfCbStruct, SILK_LSFCOSTAB_FIX_Q12};

/* Unpack predictor values and indices for entropy coding tables */
/// Translation of `silk_NLSF_unpack()`.
pub(crate) fn silk_nlsf_unpack(
    ec_ix: &mut [i16; MAX_LPC_ORDER], /* O    Indices to entropy tables [ LPC_ORDER ]     */
    pred_q8: &mut [u8; MAX_LPC_ORDER], /* O    LSF predictor [ LPC_ORDER ]                 */
    ps_nlsf_cb: &SilkNlsfCbStruct,    /* I    Codebook object                             */
    cb1_index: i32,                   /* I    Index of vector in first LSF codebook       */
) {
    let order = ps_nlsf_cb.order as i32;
    let mut ec_sel_ptr = (cb1_index * order / 2) as usize;
    let mut i = 0usize;
    while (i as i32) < order {
        let entry = ps_nlsf_cb.ec_sel[ec_sel_ptr] as i32;
        ec_sel_ptr += 1;
        ec_ix[i] = silk_smulbb(silk_rshift(entry, 1) & 7, 2 * NLSF_QUANT_MAX_AMPLITUDE + 1) as i16;
        pred_q8[i] = ps_nlsf_cb.pred_q8[i + ((entry & 1) * (order - 1)) as usize];
        ec_ix[i + 1] =
            silk_smulbb(silk_rshift(entry, 5) & 7, 2 * NLSF_QUANT_MAX_AMPLITUDE + 1) as i16;
        pred_q8[i + 1] =
            ps_nlsf_cb.pred_q8[i + ((silk_rshift(entry, 4) & 1) * (order - 1)) as usize + 1];
        i += 2;
    }
}

/* Predictive dequantizer for NLSF residuals */
#[inline]
fn silk_nlsf_residual_dequant(
    x_q10: &mut [i16; MAX_LPC_ORDER], /* O    Output [ order ]                            */
    indices: &[i8],                   /* I    Quantization indices [ order ]              */
    pred_coef_q8: &[u8; MAX_LPC_ORDER], /* I    Backward predictor coefs [ order ]          */
    quant_step_size_q16: i32,         /* I    Quantization step size                      */
    order: i16,                       /* I    Number of input values                      */
) {
    let mut out_q10: i32 = 0;
    let mut i = order as i32 - 1;
    while i >= 0 {
        let iu = i as usize;
        let pred_q10 = silk_rshift(silk_smulbb(out_q10, pred_coef_q8[iu] as i16 as i32), 8);
        out_q10 = silk_lshift(indices[iu] as i32, 10);
        if out_q10 > 0 {
            out_q10 -= silk_fix_const(NLSF_QUANT_LEVEL_ADJ, 10);
        } else if out_q10 < 0 {
            out_q10 += silk_fix_const(NLSF_QUANT_LEVEL_ADJ, 10);
        }
        out_q10 = silk_smlawb(pred_q10, out_q10, quant_step_size_q16);
        x_q10[iu] = out_q10 as i16;
        i -= 1;
    }
}

/***********************/
/* NLSF vector decoder */
/***********************/
/// Translation of `silk_NLSF_decode()`.
pub(crate) fn silk_nlsf_decode(
    p_nlsf_q15: &mut [i16; MAX_LPC_ORDER], /* O    Quantized NLSF vector [ LPC_ORDER ]         */
    nlsf_indices: &[i8; MAX_LPC_ORDER + 1], /* I    Codebook path vector [ LPC_ORDER + 1 ]      */
    ps_nlsf_cb: &SilkNlsfCbStruct,         /* I    Codebook object                             */
) {
    let mut pred_q8 = [0u8; MAX_LPC_ORDER];
    let mut ec_ix = [0i16; MAX_LPC_ORDER];
    let mut res_q10 = [0i16; MAX_LPC_ORDER];

    /* Unpack entropy table indices and predictor for current CB1 index */
    silk_nlsf_unpack(&mut ec_ix, &mut pred_q8, ps_nlsf_cb, nlsf_indices[0] as i32);

    /* Predictive residual dequantizer */
    silk_nlsf_residual_dequant(
        &mut res_q10,
        &nlsf_indices[1..],
        &pred_q8,
        ps_nlsf_cb.quant_step_size_q16 as i32,
        ps_nlsf_cb.order,
    );

    /* Apply inverse square-rooted weights to first stage and add to output */
    let order = ps_nlsf_cb.order as usize;
    let base = nlsf_indices[0] as i32 as isize * order as isize;
    let p_cb_element = &ps_nlsf_cb.cb1_nlsf_q8[base as usize..];
    let p_cb_wght_q9 = &ps_nlsf_cb.cb1_wght_q9[base as usize..];
    for i in 0..order {
        let nlsf_q15_tmp = silk_add_lshift32(
            silk_div32_16(silk_lshift(res_q10[i] as i32, 14), p_cb_wght_q9[i] as i32),
            p_cb_element[i] as i16 as i32,
            7,
        );
        p_nlsf_q15[i] = silk_limit(nlsf_q15_tmp, 0, 32767) as i16;
    }

    /* NLSF stabilization */
    silk_nlsf_stabilize(
        p_nlsf_q15,
        ps_nlsf_cb.delta_min_q15,
        ps_nlsf_cb.order as i32,
    );
}

/* Insertion sort (fast for already almost sorted arrays):   */
/// Translation of `silk_insertion_sort_increasing_all_values_int16()`.
fn silk_insertion_sort_increasing_all_values_int16(a: &mut [i16], l: i32) {
    /* Safety checks */
    debug_assert!(l > 0);

    /* Sort vector elements by value, increasing order */
    for i in 1..l as usize {
        let value = a[i];
        let mut j = i as isize - 1;
        while j >= 0 && value < a[j as usize] {
            a[j as usize + 1] = a[j as usize]; /* Shift value */
            j -= 1;
        }
        a[(j + 1) as usize] = value; /* Write value */
    }
}

/* Constant Definitions */
const MAX_LOOPS: i32 = 20;

/* NLSF stabilizer, for a single input data vector */
/// Translation of `silk_NLSF_stabilize()`.
pub(crate) fn silk_nlsf_stabilize(
    nlsf_q15: &mut [i16], /* I/O   Unstable/stabilized normalized LSF vector in Q15 [L]       */
    n_delta_min_q15: &[i16], /* I     Min distance vector, NDeltaMin_Q15[L] must be >= 1 [L+1]   */
    l: i32,               /* I     Number of NLSF parameters in the input vector              */
) {
    let lu = l as usize;
    let mut ii: usize = 0;
    let mut loops = 0;
    let d = |k: usize| n_delta_min_q15[k] as i32;

    /* This is necessary to ensure an output within range of a opus_int16 */
    while loops < MAX_LOOPS {
        /**************************/
        /* Find smallest distance */
        /**************************/
        /* First element */
        let mut min_diff_q15 = nlsf_q15[0] as i32 - d(0);
        ii = 0;
        /* Middle elements */
        for i in 1..lu {
            let diff_q15 = nlsf_q15[i] as i32 - (nlsf_q15[i - 1] as i32 + d(i));
            if diff_q15 < min_diff_q15 {
                min_diff_q15 = diff_q15;
                ii = i;
            }
        }
        /* Last element */
        let diff_q15 = (1 << 15) - (nlsf_q15[lu - 1] as i32 + d(lu));
        if diff_q15 < min_diff_q15 {
            min_diff_q15 = diff_q15;
            ii = lu;
        }

        /***************************************************/
        /* Now check if the smallest distance non-negative */
        /***************************************************/
        if min_diff_q15 >= 0 {
            return;
        }

        if ii == 0 {
            /* Move away from lower limit */
            nlsf_q15[0] = n_delta_min_q15[0];
        } else if ii == lu {
            /* Move away from higher limit */
            nlsf_q15[lu - 1] = ((1 << 15) - d(lu)) as i16;
        } else {
            /* Find the lower extreme for the location of the current center frequency */
            let mut min_center_q15: i32 = 0;
            for k in 0..ii {
                min_center_q15 += d(k);
            }
            min_center_q15 += silk_rshift(d(ii), 1);

            /* Find the upper extreme for the location of the current center frequency */
            let mut max_center_q15: i32 = 1 << 15;
            let mut k = lu;
            while k > ii {
                max_center_q15 -= d(k);
                k -= 1;
            }
            max_center_q15 -= silk_rshift(d(ii), 1);

            /* Move apart, sorted by value, keeping the same center frequency */
            let center_freq_q15 = silk_limit(
                silk_rshift_round(nlsf_q15[ii - 1] as i32 + nlsf_q15[ii] as i32, 1),
                min_center_q15,
                max_center_q15,
            ) as i16;
            nlsf_q15[ii - 1] = (center_freq_q15 as i32 - silk_rshift(d(ii), 1)) as i16;
            nlsf_q15[ii] = (nlsf_q15[ii - 1] as i32 + d(ii)) as i16;
        }
        loops += 1;
    }
    let _ = ii;

    /* Safe and simple fall back method, which is less ideal than the above */
    if loops == MAX_LOOPS {
        /* Insertion sort (fast for already almost sorted arrays):   */
        /* Best case:  O(n)   for an already sorted array            */
        /* Worst case: O(n^2) for an inversely sorted array          */
        silk_insertion_sort_increasing_all_values_int16(nlsf_q15, l);

        /* First NLSF should be no less than NDeltaMin[0] */
        nlsf_q15[0] = (nlsf_q15[0] as i32).max(d(0)) as i16;

        /* Keep delta_min distance between the NLSFs */
        for i in 1..lu {
            nlsf_q15[i] = (nlsf_q15[i] as i32)
                .max(silk_add_sat16(nlsf_q15[i - 1] as i32, d(i)) as i32)
                as i16;
        }

        /* Last NLSF should be no higher than 1 - NDeltaMin[L] */
        nlsf_q15[lu - 1] = (nlsf_q15[lu - 1] as i32).min((1 << 15) - d(lu)) as i16;

        /* Keep NDeltaMin distance between the NLSFs */
        let mut i = lu as isize - 2;
        while i >= 0 {
            let iu = i as usize;
            nlsf_q15[iu] = (nlsf_q15[iu] as i32).min(nlsf_q15[iu + 1] as i32 - d(iu + 1)) as i16;
            i -= 1;
        }
    }
}

/* conversion between prediction filter coefficients and LSFs   */
/* order should be even                                         */
/* a piecewise linear approximation maps LSF <-> cos(LSF)       */
/* therefore the result is not accurate LSFs, but the two       */
/* functions are accurate inverses of each other                */

const QA: i32 = 16;

/* helper function for NLSF2A(..) */
#[inline]
fn silk_nlsf2a_find_poly(
    out: &mut [i32], /* O    intermediate polynomial, QA [dd+1]        */
    c_lsf: &[i32],   /* I    vector of interleaved 2*cos(LSFs), QA [d] */
    dd: usize,       /* I    polynomial order (= 1/2 * filter order)   */
) {
    out[0] = silk_lshift(1, QA);
    out[1] = c_lsf[0].wrapping_neg();
    for k in 1..dd {
        let ftmp = c_lsf[2 * k]; /* QA*/
        out[k + 1] = silk_lshift(out[k - 1], 1)
            .wrapping_sub(silk_rshift_round64(silk_smull(ftmp, out[k]), QA) as i32);
        let mut n = k;
        while n > 1 {
            out[n] = out[n].wrapping_add(
                out[n - 2]
                    .wrapping_sub(silk_rshift_round64(silk_smull(ftmp, out[n - 1]), QA) as i32),
            );
            n -= 1;
        }
        out[1] = out[1].wrapping_sub(ftmp);
    }
}

/* compute whitening filter coefficients from normalized line spectral frequencies */
/// Translation of `silk_NLSF2A()`.
pub(crate) fn silk_nlsf2a(
    a_q12: &mut [i16], /* O    monic whitening filter coefficients in Q12,  [ d ]          */
    nlsf: &[i16],      /* I    normalized line spectral frequencies in Q15, [ d ]          */
    d: i32,            /* I    filter order (should be even)                               */
) {
    /* This ordering was found to maximize quality. It improves numerical accuracy of
    silk_NLSF2A_find_poly() compared to "standard" ordering. */
    const ORDERING16: [u8; 16] = [0, 15, 8, 7, 4, 11, 12, 3, 2, 13, 10, 5, 6, 9, 14, 1];
    const ORDERING10: [u8; 10] = [0, 9, 6, 3, 4, 5, 8, 1, 2, 7];
    let mut cos_lsf_qa = [0i32; SILK_MAX_ORDER_LPC];
    let mut p = [0i32; SILK_MAX_ORDER_LPC / 2 + 1];
    let mut q = [0i32; SILK_MAX_ORDER_LPC / 2 + 1];
    let mut a32_qa1 = [0i32; SILK_MAX_ORDER_LPC];
    let du = d as usize;

    debug_assert!(d == 10 || d == 16);

    /* convert LSFs to 2*cos(LSF), using piecewise linear curve from table */
    let ordering: &[u8] = if d == 16 { &ORDERING16 } else { &ORDERING10 };
    for k in 0..du {
        /* f_int on a scale 0-127 (rounded down) */
        let f_int = silk_rshift(nlsf[k] as i32, 15 - 7);

        /* f_frac, range: 0..255 */
        let f_frac = nlsf[k] as i32 - silk_lshift(f_int, 15 - 7);

        /* Read start and end value from table */
        let cos_val = SILK_LSFCOSTAB_FIX_Q12[f_int as usize] as i32; /* Q12 */
        let delta = SILK_LSFCOSTAB_FIX_Q12[f_int as usize + 1] as i32 - cos_val; /* Q12, with a range of 0..200 */

        /* Linear interpolation */
        cos_lsf_qa[ordering[k] as usize] =
            silk_rshift_round(silk_lshift(cos_val, 8) + silk_mul(delta, f_frac), 20 - QA);
        /* QA */
    }

    let dd = silk_rshift(d, 1) as usize;

    /* generate even and odd polynomials using convolution */
    silk_nlsf2a_find_poly(&mut p, &cos_lsf_qa[0..], dd);
    silk_nlsf2a_find_poly(&mut q, &cos_lsf_qa[1..], dd);

    /* convert even and odd polynomials to opus_int32 Q12 filter coefs */
    for k in 0..dd {
        let ptmp = p[k + 1].wrapping_add(p[k]);
        let qtmp = q[k + 1].wrapping_sub(q[k]);

        /* the Ptmp and Qtmp values at this stage need to fit in int32 */
        a32_qa1[k] = qtmp.wrapping_neg().wrapping_sub(ptmp); /* QA+1 */
        a32_qa1[du - k - 1] = qtmp.wrapping_sub(ptmp); /* QA+1 */
    }

    /* Convert int32 coefficients to Q12 int16 coefs */
    silk_lpc_fit(a_q12, &mut a32_qa1, 12, QA + 1, d);

    let mut i = 0;
    while silk_lpc_inverse_pred_gain(a_q12, d) == 0 && i < MAX_LPC_STABILIZE_ITERATIONS {
        /* Prediction coefficients are (too close to) unstable; apply bandwidth expansion   */
        /* on the unscaled coefficients, convert to Q12 and measure again                   */
        silk_bwexpander_32(&mut a32_qa1, d, 65536 - silk_lshift(2, i));
        for k in 0..du {
            a_q12[k] = silk_rshift_round(a32_qa1[k], QA + 1 - 12) as i16; /* QA+1 -> Q12 */
        }
        i += 1;
    }
}

/* Convert int32 coefficients to int16 coefs and make sure there's no wrap-around.
This logic is reused in _celt_lpc(). Any bug fixes should also be applied there. */
/// Translation of `silk_LPC_fit()`.
pub(crate) fn silk_lpc_fit(
    a_qout: &mut [i16], /* O    Output signal                                               */
    a_qin: &mut [i32],  /* I/O  Input signal                                                */
    qout: i32,          /* I    Input Q domain                                              */
    qin: i32,           /* I    Input Q domain                                              */
    d: i32,             /* I    Filter order                                                */
) {
    let du = d as usize;
    let mut idx = 0usize;

    /* Limit the maximum absolute value of the prediction coefficients, so that they'll fit in int16 */
    let mut i = 0;
    while i < 10 {
        /* Find maximum absolute value and its index */
        let mut maxabs: i32 = 0;
        for k in 0..du {
            let absval = silk_abs(a_qin[k]);
            if absval > maxabs {
                maxabs = absval;
                idx = k;
            }
        }
        maxabs = silk_rshift_round(maxabs, qin - qout);

        if maxabs > SILK_INT16_MAX {
            /* Reduce magnitude of prediction coefficients */
            maxabs = maxabs.min(163838); /* ( silk_int32_MAX >> 14 ) + silk_int16_MAX = 163838 */
            let chirp_q16 = silk_fix_const(0.999, 16)
                - silk_div32(
                    silk_lshift(maxabs - SILK_INT16_MAX, 14),
                    silk_rshift(silk_mul(maxabs, idx as i32 + 1), 2),
                );
            silk_bwexpander_32(a_qin, d, chirp_q16);
        } else {
            break;
        }
        i += 1;
    }

    if i == 10 {
        /* Reached the last iteration, clip the coefficients */
        for k in 0..du {
            a_qout[k] = silk_sat16(silk_rshift_round(a_qin[k], qin - qout)) as i16;
            a_qin[k] = silk_lshift(a_qout[k] as i32, qin - qout);
        }
    } else {
        for k in 0..du {
            a_qout[k] = silk_rshift_round(a_qin[k], qin - qout) as i16;
        }
    }
}

/* Chirp (bandwidth expand) LP AR filter */
/// Translation of `silk_bwexpander()`.
pub(crate) fn silk_bwexpander(
    ar: &mut [i16], /* I/O  AR filter to be expanded (without leading 1)                */
    d: i32,         /* I    Length of ar                                                */
    mut chirp_q16: i32, /* I    Chirp factor (typically in the range 0 to 1)                */
) {
    let chirp_minus_one_q16 = chirp_q16 - 65536;
    let du = d as usize;

    /* NB: Dont use silk_SMULWB, instead of silk_RSHIFT_ROUND( silk_MUL(), 16 ), below.  */
    /* Bias in silk_SMULWB can lead to unstable filters                                */
    for v in ar[..du - 1].iter_mut() {
        *v = silk_rshift_round(silk_mul(chirp_q16, *v as i32), 16) as i16;
        chirp_q16 += silk_rshift_round(silk_mul(chirp_q16, chirp_minus_one_q16), 16);
    }
    ar[du - 1] = silk_rshift_round(silk_mul(chirp_q16, ar[du - 1] as i32), 16) as i16;
}

/* Chirp (bandwidth expand) LP AR filter.
This logic is reused in _celt_lpc(). Any bug fixes should also be applied there. */
/// Translation of `silk_bwexpander_32()`.
pub(crate) fn silk_bwexpander_32(
    ar: &mut [i32], /* I/O  AR filter to be expanded (without leading 1)                */
    d: i32,         /* I    Length of ar                                                */
    mut chirp_q16: i32, /* I    Chirp factor in Q16                                         */
) {
    let chirp_minus_one_q16 = chirp_q16 - 65536;
    let du = d as usize;

    for v in ar[..du - 1].iter_mut() {
        *v = silk_smulww(chirp_q16, *v);
        chirp_q16 += silk_rshift_round(silk_mul(chirp_q16, chirp_minus_one_q16), 16);
    }
    ar[du - 1] = silk_smulww(chirp_q16, ar[du - 1]);
}

const QA24: i32 = 24;
const A_LIMIT: i32 = silk_fix_const(0.99975, QA24 as u32);

#[inline(always)]
fn mul32_frac_q(a32: i32, b32: i32, q: i32) -> i32 {
    silk_rshift_round64(silk_smull(a32, b32), q) as i32
}

/* Compute inverse of LPC prediction gain, and                          */
/* test if LPC coefficients are stable (all poles within unit circle)   */
fn lpc_inverse_pred_gain_qa_c(
    a_qa: &mut [i32; SILK_MAX_ORDER_LPC], /* I   Prediction coefficients                                  */
    order: i32, /* I   Prediction order                                         */
) -> i32 {
    // (SILK_FIX_CONST( 1.0f / MAX_PREDICTION_POWER_GAIN, 30 ): the float
    // quotient times a float 2^30, plus 0.5 in double.)
    let min_inv_gain_q30 =
        (((1.0f32 / MAX_PREDICTION_POWER_GAIN) * (1i64 << 30) as f32) as f64 + 0.5) as i32;
    let mut inv_gain_q30 = silk_fix_const(1.0, 30);
    let mut k = order - 1;
    while k > 0 {
        let ku = k as usize;
        /* Check for stability */
        if (a_qa[ku] > A_LIMIT) || (a_qa[ku] < -A_LIMIT) {
            return 0;
        }

        /* Set RC equal to negated AR coef */
        let rc_q31 = silk_lshift(a_qa[ku], 31 - QA24).wrapping_neg();

        /* rc_mult1_Q30 range: [ 1 : 2^30 ] */
        let rc_mult1_q30 = silk_fix_const(1.0, 30).wrapping_sub(silk_smmul(rc_q31, rc_q31));

        /* Update inverse gain */
        /* invGain_Q30 range: [ 0 : 2^30 ] */
        inv_gain_q30 = silk_lshift(silk_smmul(inv_gain_q30, rc_mult1_q30), 2);
        if inv_gain_q30 < min_inv_gain_q30 {
            return 0;
        }

        /* rc_mult2 range: [ 2^30 : silk_int32_MAX ] */
        let mult2q = 32 - silk_clz32(silk_abs(rc_mult1_q30));
        let rc_mult2 = silk_inverse32_varq(rc_mult1_q30, mult2q + 30);

        /* Update AR coefficient */
        for n in 0..((k + 1) >> 1) as usize {
            let tmp1 = a_qa[n];
            let tmp2 = a_qa[ku - n - 1];
            let mut tmp64 = silk_rshift_round64(
                silk_smull(
                    silk_sub_sat32(tmp1, mul32_frac_q(tmp2, rc_q31, 31)),
                    rc_mult2,
                ),
                mult2q,
            );
            if tmp64 > SILK_INT32_MAX as i64 || tmp64 < SILK_INT32_MIN as i64 {
                return 0;
            }
            a_qa[n] = tmp64 as i32;
            tmp64 = silk_rshift_round64(
                silk_smull(
                    silk_sub_sat32(tmp2, mul32_frac_q(tmp1, rc_q31, 31)),
                    rc_mult2,
                ),
                mult2q,
            );
            if tmp64 > SILK_INT32_MAX as i64 || tmp64 < SILK_INT32_MIN as i64 {
                return 0;
            }
            a_qa[ku - n - 1] = tmp64 as i32;
        }
        k -= 1;
    }

    /* Check for stability */
    if (a_qa[k as usize] > A_LIMIT) || (a_qa[k as usize] < -A_LIMIT) {
        return 0;
    }

    /* Set RC equal to negated AR coef */
    let rc_q31 = silk_lshift(a_qa[0], 31 - QA24).wrapping_neg();

    /* Range: [ 1 : 2^30 ] */
    let rc_mult1_q30 = silk_fix_const(1.0, 30).wrapping_sub(silk_smmul(rc_q31, rc_q31));

    /* Update inverse gain */
    /* Range: [ 0 : 2^30 ] */
    inv_gain_q30 = silk_lshift(silk_smmul(inv_gain_q30, rc_mult1_q30), 2);
    if inv_gain_q30 < min_inv_gain_q30 {
        return 0;
    }

    inv_gain_q30
}

/* For input in Q12 domain */
/// Translation of `silk_LPC_inverse_pred_gain_c()`.
pub(crate) fn silk_lpc_inverse_pred_gain(
    a_q12: &[i16], /* I   Prediction coefficients, Q12 [order]                         */
    order: i32,    /* I   Prediction order                                             */
) -> i32 {
    let mut atmp_qa = [0i32; SILK_MAX_ORDER_LPC];
    let mut dc_resp: i32 = 0;

    /* Increase Q domain of the AR coefficients */
    for k in 0..order as usize {
        dc_resp += a_q12[k] as i32;
        atmp_qa[k] = silk_lshift(a_q12[k] as i32, QA24 - 12);
    }
    /* If the DC is unstable, we don't even need to do the full calculations */
    if dc_resp >= 4096 {
        return 0;
    }
    lpc_inverse_pred_gain_qa_c(&mut atmp_qa, order)
}
