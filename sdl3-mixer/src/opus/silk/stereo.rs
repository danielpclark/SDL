// Rust translation of silk/stereo_decode_pred.c and silk/stereo_MS_to_LR.c
// from libopus 1.4 (https://opus-codec.org, at the revision SDL_mixer's
// external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SILK's mid/side stereo.

use super::define::{STEREO_INTERP_LEN_MS, STEREO_QUANT_SUB_STEPS};
use super::sigproc_fix::{
    silk_add_lshift32, silk_div32_16, silk_fix_const, silk_lshift, silk_rshift_round, silk_sat16,
    silk_smlabb, silk_smlawb, silk_smulbb, silk_smulwb,
};
use super::structs::StereoDecState;
use super::tables::{
    SILK_STEREO_ONLY_CODE_MID_ICDF, SILK_STEREO_PRED_JOINT_ICDF, SILK_STEREO_PRED_QUANT_Q13,
    SILK_UNIFORM3_ICDF, SILK_UNIFORM5_ICDF,
};
use crate::opus::celt::entcode::EcDec;

/* Decode mid/side predictors */
/// Translation of `silk_stereo_decode_pred()`.
pub(crate) fn silk_stereo_decode_pred(
    ps_range_dec: &mut EcDec<'_>, /* I/O  Compressor data structure                   */
    pred_q13: &mut [i32; 2],      /* O    Predictors                                  */
) {
    let mut ix = [[0i32; 3]; 2];

    /* Entropy decoding */
    let n = ps_range_dec.ec_dec_icdf(&SILK_STEREO_PRED_JOINT_ICDF, 8);
    ix[0][2] = silk_div32_16(n, 5);
    ix[1][2] = n - 5 * ix[0][2];
    for row in ix.iter_mut() {
        row[0] = ps_range_dec.ec_dec_icdf(&SILK_UNIFORM3_ICDF, 8);
        row[1] = ps_range_dec.ec_dec_icdf(&SILK_UNIFORM5_ICDF, 8);
    }

    /* Dequantize */
    for n in 0..2 {
        ix[n][0] += 3 * ix[n][2];
        let low_q13 = SILK_STEREO_PRED_QUANT_Q13[ix[n][0] as usize] as i32;
        let step_q13 = silk_smulwb(
            SILK_STEREO_PRED_QUANT_Q13[ix[n][0] as usize + 1] as i32 - low_q13,
            silk_fix_const(0.5 / STEREO_QUANT_SUB_STEPS as f64, 16),
        );
        pred_q13[n] = silk_smlabb(low_q13, step_q13, 2 * ix[n][1] + 1);
    }

    /* Subtract second from first predictor (helps when actually applying these) */
    pred_q13[0] -= pred_q13[1];
}

/* Decode mid-only flag */
/// Translation of `silk_stereo_decode_mid_only()`.
pub(crate) fn silk_stereo_decode_mid_only(
    ps_range_dec: &mut EcDec<'_>, /* I/O  Compressor data structure                   */
    decode_only_mid: &mut i32,    /* O    Flag that only mid channel has been coded   */
) {
    /* Decode flag that only mid channel is coded */
    *decode_only_mid = ps_range_dec.ec_dec_icdf(&SILK_STEREO_ONLY_CODE_MID_ICDF, 8);
}

/* Convert adaptive Mid/Side representation to Left/Right stereo signal */
/// Translation of `silk_stereo_MS_to_LR()`.
pub(crate) fn silk_stereo_ms_to_lr(
    state: &mut StereoDecState, /* I/O  State                                       */
    x1: &mut [i16],             /* I/O  Left input signal, becomes mid signal       */
    x2: &mut [i16],             /* I/O  Right input signal, becomes side signal     */
    pred_q13: &[i32; 2],        /* I    Predictors                                  */
    fs_khz: i32,                /* I    Samples rate (kHz)                          */
    frame_length: i32,          /* I    Number of samples                           */
) {
    let fl = frame_length as usize;

    /* Buffering */
    x1[..2].copy_from_slice(&state.s_mid);
    x2[..2].copy_from_slice(&state.s_side);
    state.s_mid.copy_from_slice(&x1[fl..fl + 2]);
    state.s_side.copy_from_slice(&x2[fl..fl + 2]);

    let step = |x1: &[i16], x2: &mut [i16], n: usize, pred0_q13: i32, pred1_q13: i32| {
        let mut sum = silk_lshift(
            silk_add_lshift32(x1[n] as i32 + x1[n + 2] as i32, x1[n + 1] as i32, 1),
            9,
        ); /* Q11 */
        sum = silk_smlawb(silk_lshift(x2[n + 1] as i32, 8), sum, pred0_q13); /* Q8  */
        sum = silk_smlawb(sum, silk_lshift(x1[n + 1] as i32, 11), pred1_q13); /* Q8  */
        x2[n + 1] = silk_sat16(silk_rshift_round(sum, 8)) as i16;
    };

    /* Interpolate predictors and add prediction to side channel */
    let mut pred0_q13 = state.pred_prev_q13[0] as i32;
    let mut pred1_q13 = state.pred_prev_q13[1] as i32;
    let denom_q16 = silk_div32_16(1i32 << 16, STEREO_INTERP_LEN_MS * fs_khz);
    let delta0_q13 = silk_rshift_round(
        silk_smulbb(pred_q13[0] - state.pred_prev_q13[0] as i32, denom_q16),
        16,
    );
    let delta1_q13 = silk_rshift_round(
        silk_smulbb(pred_q13[1] - state.pred_prev_q13[1] as i32, denom_q16),
        16,
    );
    let interp = (STEREO_INTERP_LEN_MS * fs_khz) as usize;
    for n in 0..interp {
        pred0_q13 += delta0_q13;
        pred1_q13 += delta1_q13;
        step(x1, x2, n, pred0_q13, pred1_q13);
    }
    pred0_q13 = pred_q13[0];
    pred1_q13 = pred_q13[1];
    for n in interp..fl {
        step(x1, x2, n, pred0_q13, pred1_q13);
    }
    state.pred_prev_q13[0] = pred_q13[0] as i16;
    state.pred_prev_q13[1] = pred_q13[1] as i16;

    /* Convert to left/right signals */
    for n in 0..fl {
        let sum = x1[n + 1] as i32 + x2[n + 1] as i32;
        let diff = x1[n + 1] as i32 - x2[n + 1] as i32;
        x1[n + 1] = silk_sat16(sum) as i16;
        x2[n + 1] = silk_sat16(diff) as i16;
    }
}
