// Rust translation of silk/decode_parameters.c, silk/gain_quant.c (the
// dequantizer), silk/decode_pitch.c, silk/lin2log.c and silk/log2lin.c from
// libopus 1.4 (https://opus-codec.org, at the revision SDL_mixer's
// external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Turning a SILK frame's indices into its parameters.

use super::define::*;
use super::nlsf::{silk_bwexpander, silk_nlsf2a, silk_nlsf_decode};
use super::sigproc_fix::{
    silk_add_lshift32, silk_add_rshift32, silk_clz_frac, silk_limit, silk_lshift, silk_mla,
    silk_mul, silk_rshift, silk_smlawb, silk_smulbb, silk_smulwb, SILK_INT32_MAX,
};
use super::structs::{SilkDecoderControl, SilkDecoderState};
use super::tables::{
    SILK_CB_LAGS_STAGE2, SILK_CB_LAGS_STAGE2_10_MS, SILK_CB_LAGS_STAGE3, SILK_CB_LAGS_STAGE3_10_MS,
    SILK_LTPSCALES_TABLE_Q14, SILK_LTP_VQ_PTRS_Q7,
};

/* Approximation of 128 * log2() (very close inverse of silk_log2lin()) */
/* Convert input to a log scale    */
/// Translation of `silk_lin2log()`.
#[allow(dead_code)]
pub(crate) fn silk_lin2log(in_lin: i32) -> i32 {
    let (lz, frac_q7) = silk_clz_frac(in_lin);

    /* Piece-wise parabolic approximation */
    silk_add_lshift32(
        silk_smlawb(frac_q7, silk_mul(frac_q7, 128 - frac_q7), 179),
        31 - lz,
        7,
    )
}

/* Approximation of 2^() (very close inverse of silk_lin2log()) */
/* Convert input to a linear scale    */
/// Translation of `silk_log2lin()`.
pub(crate) fn silk_log2lin(in_log_q7: i32) -> i32 {
    if in_log_q7 < 0 {
        return 0;
    } else if in_log_q7 >= 3967 {
        return SILK_INT32_MAX;
    }

    let mut out = silk_lshift(1, silk_rshift(in_log_q7, 7));
    let frac_q7 = in_log_q7 & 0x7F;
    if in_log_q7 < 2048 {
        /* Piece-wise parabolic approximation */
        out = silk_add_rshift32(
            out,
            silk_mul(
                out,
                silk_smlawb(frac_q7, silk_smulbb(frac_q7, 128 - frac_q7), -174),
            ),
            7,
        );
    } else {
        /* Piece-wise parabolic approximation */
        out = silk_mla(
            out,
            silk_rshift(out, 7),
            silk_smlawb(frac_q7, silk_smulbb(frac_q7, 128 - frac_q7), -174),
        );
    }
    out
}

const OFFSET: i32 = (MIN_QGAIN_DB * 128) / 6 + 16 * 128;
#[allow(dead_code)]
const SCALE_Q16: i32 = (65536 * (N_LEVELS_QGAIN - 1)) / (((MAX_QGAIN_DB - MIN_QGAIN_DB) * 128) / 6);
const INV_SCALE_Q16: i32 =
    (65536 * (((MAX_QGAIN_DB - MIN_QGAIN_DB) * 128) / 6)) / (N_LEVELS_QGAIN - 1);

/* Gains scalar dequantization, uniform on log scale */
/// Translation of `silk_gains_dequant()`.
pub(crate) fn silk_gains_dequant(
    gain_q16: &mut [i32; MAX_NB_SUBFR], /* O    quantized gains                             */
    ind: &[i8; MAX_NB_SUBFR],           /* I    gain indices                                */
    prev_ind: &mut i8,                  /* I/O  last index in previous frame                */
    conditional: i32,                   /* I    first gain is delta coded if 1              */
    nb_subfr: i32,                      /* I    number of subframes                          */
) {
    for k in 0..nb_subfr as usize {
        if k == 0 && conditional == 0 {
            /* Gain index is not allowed to go down more than 16 steps (~21.8 dB) */
            *prev_ind = (ind[k] as i32).max(*prev_ind as i32 - 16) as i8;
        } else {
            /* Delta index */
            let ind_tmp = ind[k] as i32 + MIN_DELTA_GAIN_QUANT;

            /* Accumulate deltas */
            let double_step_size_threshold =
                2 * MAX_DELTA_GAIN_QUANT - N_LEVELS_QGAIN + *prev_ind as i32;
            if ind_tmp > double_step_size_threshold {
                *prev_ind =
                    (*prev_ind as i32 + silk_lshift(ind_tmp, 1) - double_step_size_threshold) as i8;
            } else {
                *prev_ind = (*prev_ind as i32 + ind_tmp) as i8;
            }
        }
        *prev_ind = silk_limit(*prev_ind as i32, 0, N_LEVELS_QGAIN - 1) as i8;

        /* Scale and convert to linear scale */
        gain_q16[k] =
            silk_log2lin((silk_smulwb(INV_SCALE_Q16, *prev_ind as i32) + OFFSET).min(3967));
        /* 3967 = 31 in Q7 */
    }
}

/***********************************************************
* Pitch analyser function
********************************************************** */
/// Translation of `silk_decode_pitch()`.
pub(crate) fn silk_decode_pitch(
    lag_index: i16,
    contour_index: i8,
    pitch_lags: &mut [i32; MAX_NB_SUBFR], /* O    4 pitch values                                              */
    fs_khz: i32,   /* I    sampling frequency (kHz)                                    */
    nb_subfr: i32, /* I    number of sub frames                                        */
) {
    let lag_cb: &[i8];
    let cbk_size: usize;

    if fs_khz == 8 {
        if nb_subfr == PE_MAX_NB_SUBFR {
            lag_cb = SILK_CB_LAGS_STAGE2.as_flattened();
            cbk_size = PE_NB_CBKS_STAGE2_EXT;
        } else {
            debug_assert!(nb_subfr == PE_MAX_NB_SUBFR >> 1);
            lag_cb = SILK_CB_LAGS_STAGE2_10_MS.as_flattened();
            cbk_size = PE_NB_CBKS_STAGE2_10MS;
        }
    } else if nb_subfr == PE_MAX_NB_SUBFR {
        lag_cb = SILK_CB_LAGS_STAGE3.as_flattened();
        cbk_size = PE_NB_CBKS_STAGE3_MAX;
    } else {
        debug_assert!(nb_subfr == PE_MAX_NB_SUBFR >> 1);
        lag_cb = SILK_CB_LAGS_STAGE3_10_MS.as_flattened();
        cbk_size = PE_NB_CBKS_STAGE3_10MS;
    }

    let min_lag = silk_smulbb(PE_MIN_LAG_MS, fs_khz);
    let max_lag = silk_smulbb(PE_MAX_LAG_MS, fs_khz);
    let lag = min_lag + lag_index as i32;

    for k in 0..nb_subfr as usize {
        pitch_lags[k] = lag + lag_cb[k * cbk_size + contour_index as usize] as i32;
        pitch_lags[k] = silk_limit(pitch_lags[k], min_lag, max_lag);
    }
}

/* Decode parameters from payload */
/// Translation of `silk_decode_parameters()`.
pub(crate) fn silk_decode_parameters(
    ps_dec: &mut SilkDecoderState,
    ps_dec_ctrl: &mut SilkDecoderControl,
    cond_coding: i32,
) {
    let mut p_nlsf_q15 = [0i16; MAX_LPC_ORDER];
    let mut p_nlsf0_q15 = [0i16; MAX_LPC_ORDER];
    let order = ps_dec.lpc_order as usize;

    /* Dequant Gains */
    silk_gains_dequant(
        &mut ps_dec_ctrl.gains_q16,
        &ps_dec.indices.gains_indices,
        &mut ps_dec.last_gain_index,
        (cond_coding == CODE_CONDITIONALLY) as i32,
        ps_dec.nb_subfr,
    );

    /****************/
    /* Decode NLSFs */
    /****************/
    silk_nlsf_decode(
        &mut p_nlsf_q15,
        &ps_dec.indices.nlsf_indices,
        ps_dec.ps_nlsf_cb,
    );

    /* Convert NLSF parameters to AR prediction filter coefficients */
    silk_nlsf2a(
        &mut ps_dec_ctrl.pred_coef_q12[1],
        &p_nlsf_q15,
        ps_dec.lpc_order,
    );

    /* If just reset, e.g., because internal Fs changed, do not allow interpolation */
    /* improves the case of packet loss in the first frame after a switch           */
    if ps_dec.first_frame_after_reset == 1 {
        ps_dec.indices.nlsf_interp_coef_q2 = 4;
    }

    if ps_dec.indices.nlsf_interp_coef_q2 < 4 {
        /* Calculation of the interpolated NLSF0 vector from the interpolation factor, */
        /* the previous NLSF1, and the current NLSF1                                   */
        for i in 0..order {
            p_nlsf0_q15[i] = (ps_dec.prev_nlsf_q15[i] as i32
                + silk_rshift(
                    silk_mul(
                        ps_dec.indices.nlsf_interp_coef_q2 as i32,
                        p_nlsf_q15[i] as i32 - ps_dec.prev_nlsf_q15[i] as i32,
                    ),
                    2,
                )) as i16;
        }

        /* Convert NLSF parameters to AR prediction filter coefficients */
        silk_nlsf2a(
            &mut ps_dec_ctrl.pred_coef_q12[0],
            &p_nlsf0_q15,
            ps_dec.lpc_order,
        );
    } else {
        /* Copy LPC coefficients for first half from second half */
        let (a, b) = ps_dec_ctrl.pred_coef_q12.split_at_mut(1);
        a[0][..order].copy_from_slice(&b[0][..order]);
    }

    ps_dec.prev_nlsf_q15[..order].copy_from_slice(&p_nlsf_q15[..order]);

    /* After a packet loss do BWE of LPC coefs */
    if ps_dec.loss_cnt != 0 {
        silk_bwexpander(
            &mut ps_dec_ctrl.pred_coef_q12[0],
            ps_dec.lpc_order,
            BWE_AFTER_LOSS_Q16,
        );
        silk_bwexpander(
            &mut ps_dec_ctrl.pred_coef_q12[1],
            ps_dec.lpc_order,
            BWE_AFTER_LOSS_Q16,
        );
    }

    if ps_dec.indices.signal_type as i32 == TYPE_VOICED {
        /*********************/
        /* Decode pitch lags */
        /*********************/

        /* Decode pitch values */
        silk_decode_pitch(
            ps_dec.indices.lag_index,
            ps_dec.indices.contour_index,
            &mut ps_dec_ctrl.pitch_l,
            ps_dec.fs_khz,
            ps_dec.nb_subfr,
        );

        /* Decode Codebook Index */
        let cbk_ptr_q7 = SILK_LTP_VQ_PTRS_Q7[ps_dec.indices.per_index as usize]; /* set pointer to start of codebook */

        for k in 0..ps_dec.nb_subfr as usize {
            let ix = ps_dec.indices.ltp_index[k] as usize;
            for i in 0..LTP_ORDER {
                ps_dec_ctrl.ltp_coef_q14[k * LTP_ORDER + i] =
                    silk_lshift(cbk_ptr_q7[ix * LTP_ORDER + i] as i32, 7) as i16;
            }
        }

        /**********************/
        /* Decode LTP scaling */
        /**********************/
        let ix = ps_dec.indices.ltp_scale_index as usize;
        ps_dec_ctrl.ltp_scale_q14 = SILK_LTPSCALES_TABLE_Q14[ix] as i32;
    } else {
        let nb = ps_dec.nb_subfr as usize;
        ps_dec_ctrl.pitch_l[..nb].fill(0);
        ps_dec_ctrl.ltp_coef_q14[..LTP_ORDER * nb].fill(0);
        ps_dec.indices.per_index = 0;
        ps_dec_ctrl.ltp_scale_q14 = 0;
    }
}
