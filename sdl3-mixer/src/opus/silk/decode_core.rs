// Rust translation of silk/decode_core.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SILK core decoder: inverse noise shaping quantization, LTP + LPC.

use super::define::{
    LTP_MEM_LENGTH_MS, LTP_ORDER, MAX_FRAME_LENGTH, MAX_FS_KHZ, MAX_LPC_ORDER, MAX_NB_SUBFR,
    MAX_SUB_FRAME_LENGTH, QUANT_LEVEL_ADJUST_Q10, TYPE_VOICED,
};
use super::lpc_analysis_filter::silk_lpc_analysis_filter;
use super::sigproc_fix::{
    silk_add32_ovflw, silk_add_lshift32, silk_add_sat32, silk_div32_varq, silk_fix_const,
    silk_inverse32_varq, silk_lshift, silk_lshift_sat32, silk_rand, silk_rshift, silk_rshift_round,
    silk_sat16, silk_smlawb, silk_smulwb, silk_smulww,
};
use super::structs::{SilkDecoderControl, SilkDecoderState};
use super::tables::SILK_QUANTIZATION_OFFSETS_Q10;

const LTP_MEM_MAX: usize = (LTP_MEM_LENGTH_MS * MAX_FS_KHZ) as usize;

/**********************************************************/
/* Core decoder. Performs inverse NSQ operation LTP + LPC */
/**********************************************************/
/// Translation of `silk_decode_core()`.
pub(crate) fn silk_decode_core(
    ps_dec: &mut SilkDecoderState, /* I/O  Decoder state                               */
    ps_dec_ctrl: &mut SilkDecoderControl, /* I    Decoder control                             */
    xq: &mut [i16],                /* O    Decoded speech                              */
    pulses: &[i16],                /* I    Pulse signal                                */
) {
    let mut lag = 0i32;
    let mut a_q12_tmp = [0i16; MAX_LPC_ORDER];
    let mut s_ltp = [0i16; LTP_MEM_MAX];
    let mut s_ltp_q15 = [0i32; LTP_MEM_MAX + MAX_FRAME_LENGTH];
    let mut res_q14 = [0i32; MAX_SUB_FRAME_LENGTH];
    let mut s_lpc_q14 = [0i32; MAX_SUB_FRAME_LENGTH + MAX_LPC_ORDER];

    debug_assert!(ps_dec.prev_gain_q16 != 0);

    let ltp_mem_length = ps_dec.ltp_mem_length as usize;
    let subfr_length = ps_dec.subfr_length as usize;
    let order = ps_dec.lpc_order as usize;

    let offset_q10 = SILK_QUANTIZATION_OFFSETS_Q10[(ps_dec.indices.signal_type >> 1) as usize]
        [ps_dec.indices.quant_offset_type as usize] as i32;

    let nlsf_interpolation_flag = ps_dec.indices.nlsf_interp_coef_q2 < 1 << 2;

    /* Decode excitation */
    let mut rand_seed = ps_dec.indices.seed as i32;
    for i in 0..ps_dec.frame_length as usize {
        rand_seed = silk_rand(rand_seed);
        let mut e = silk_lshift(pulses[i] as i32, 14);
        if e > 0 {
            e -= QUANT_LEVEL_ADJUST_Q10 << 4;
        } else if e < 0 {
            e += QUANT_LEVEL_ADJUST_Q10 << 4;
        }
        e += offset_q10 << 4;
        if rand_seed < 0 {
            e = -e;
        }
        ps_dec.exc_q14[i] = e;

        rand_seed = silk_add32_ovflw(rand_seed, pulses[i] as i32);
    }

    /* Copy LPC state */
    s_lpc_q14[..MAX_LPC_ORDER].copy_from_slice(&ps_dec.s_lpc_q14_buf);

    let mut pexc_q14 = 0usize;
    let mut pxq = 0usize;
    let mut s_ltp_buf_idx = ltp_mem_length;
    /* Loop over subframes */
    for k in 0..ps_dec.nb_subfr as usize {
        let a_q12 = &ps_dec_ctrl.pred_coef_q12[k >> 1];

        /* Preload LPC coeficients to array on stack. Gives small performance gain */
        a_q12_tmp[..order].copy_from_slice(&a_q12[..order]);
        let b_off = k * LTP_ORDER;
        let mut signal_type = ps_dec.indices.signal_type as i32;

        let gain_q10 = silk_rshift(ps_dec_ctrl.gains_q16[k], 6);
        let mut inv_gain_q31 = silk_inverse32_varq(ps_dec_ctrl.gains_q16[k], 47);

        /* Calculate gain adjustment factor */
        let gain_adj_q16 = if ps_dec_ctrl.gains_q16[k] != ps_dec.prev_gain_q16 {
            let gain_adj_q16 = silk_div32_varq(ps_dec.prev_gain_q16, ps_dec_ctrl.gains_q16[k], 16);

            /* Scale short term state */
            for v in s_lpc_q14[..MAX_LPC_ORDER].iter_mut() {
                *v = silk_smulww(gain_adj_q16, *v);
            }
            gain_adj_q16
        } else {
            1i32 << 16
        };

        /* Save inv_gain */
        debug_assert!(inv_gain_q31 != 0);
        ps_dec.prev_gain_q16 = ps_dec_ctrl.gains_q16[k];

        /* Avoid abrupt transition from voiced PLC to unvoiced normal decoding */
        if ps_dec.loss_cnt != 0
            && ps_dec.prev_signal_type == TYPE_VOICED
            && ps_dec.indices.signal_type as i32 != TYPE_VOICED
            && k < MAX_NB_SUBFR / 2
        {
            ps_dec_ctrl.ltp_coef_q14[b_off..b_off + LTP_ORDER].fill(0);
            ps_dec_ctrl.ltp_coef_q14[b_off + LTP_ORDER / 2] = silk_fix_const(0.25, 14) as i16;

            signal_type = TYPE_VOICED;
            ps_dec_ctrl.pitch_l[k] = ps_dec.lag_prev;
        }

        if signal_type == TYPE_VOICED {
            /* Voiced */
            lag = ps_dec_ctrl.pitch_l[k];

            /* Re-whitening */
            if k == 0 || (k == 2 && nlsf_interpolation_flag) {
                /* Rewhiten with new A coefs */
                let start_idx =
                    ps_dec.ltp_mem_length - lag - ps_dec.lpc_order - LTP_ORDER as i32 / 2;
                debug_assert!(start_idx > 0);
                let start_idx = start_idx as usize;

                if k == 2 {
                    ps_dec.out_buf[ltp_mem_length..ltp_mem_length + 2 * subfr_length]
                        .copy_from_slice(&xq[..2 * subfr_length]);
                }

                silk_lpc_analysis_filter(
                    &mut s_ltp[start_idx..],
                    &ps_dec.out_buf[start_idx + k * subfr_length..],
                    &ps_dec_ctrl.pred_coef_q12[k >> 1],
                    ps_dec.ltp_mem_length - start_idx as i32,
                    ps_dec.lpc_order,
                );

                /* After rewhitening the LTP state is unscaled */
                if k == 0 {
                    /* Do LTP downscaling to reduce inter-packet dependency */
                    inv_gain_q31 =
                        silk_lshift(silk_smulwb(inv_gain_q31, ps_dec_ctrl.ltp_scale_q14), 2);
                }
                for i in 0..(lag + LTP_ORDER as i32 / 2) as usize {
                    s_ltp_q15[s_ltp_buf_idx - i - 1] =
                        silk_smulwb(inv_gain_q31, s_ltp[ltp_mem_length - i - 1] as i32);
                }
            } else {
                /* Update LTP state when Gain changes */
                if gain_adj_q16 != 1i32 << 16 {
                    for i in 0..(lag + LTP_ORDER as i32 / 2) as usize {
                        s_ltp_q15[s_ltp_buf_idx - i - 1] =
                            silk_smulww(gain_adj_q16, s_ltp_q15[s_ltp_buf_idx - i - 1]);
                    }
                }
            }
        }

        /* Long-term prediction */
        let pres_q14: &[i32] = if signal_type == TYPE_VOICED {
            let b_q14 = &ps_dec_ctrl.ltp_coef_q14[b_off..b_off + LTP_ORDER];
            /* Set up pointer */
            let mut pred_lag_ptr = (s_ltp_buf_idx as i32 - lag + LTP_ORDER as i32 / 2) as usize;
            for i in 0..subfr_length {
                /* Unrolled loop */
                /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
                let mut ltp_pred_q13 = 2;
                ltp_pred_q13 = silk_smlawb(ltp_pred_q13, s_ltp_q15[pred_lag_ptr], b_q14[0] as i32);
                ltp_pred_q13 =
                    silk_smlawb(ltp_pred_q13, s_ltp_q15[pred_lag_ptr - 1], b_q14[1] as i32);
                ltp_pred_q13 =
                    silk_smlawb(ltp_pred_q13, s_ltp_q15[pred_lag_ptr - 2], b_q14[2] as i32);
                ltp_pred_q13 =
                    silk_smlawb(ltp_pred_q13, s_ltp_q15[pred_lag_ptr - 3], b_q14[3] as i32);
                ltp_pred_q13 =
                    silk_smlawb(ltp_pred_q13, s_ltp_q15[pred_lag_ptr - 4], b_q14[4] as i32);
                pred_lag_ptr += 1;

                /* Generate LPC excitation */
                res_q14[i] = silk_add_lshift32(ps_dec.exc_q14[pexc_q14 + i], ltp_pred_q13, 1);

                /* Update states */
                s_ltp_q15[s_ltp_buf_idx] = silk_lshift(res_q14[i], 1);
                s_ltp_buf_idx += 1;
            }
            &res_q14
        } else {
            &ps_dec.exc_q14[pexc_q14..]
        };

        for i in 0..subfr_length {
            /* Short-term prediction */
            debug_assert!(ps_dec.lpc_order == 10 || ps_dec.lpc_order == 16);
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            let mut lpc_pred_q10 = silk_rshift(ps_dec.lpc_order, 1);
            for j in 0..10 {
                lpc_pred_q10 = silk_smlawb(
                    lpc_pred_q10,
                    s_lpc_q14[MAX_LPC_ORDER + i - 1 - j],
                    a_q12_tmp[j] as i32,
                );
            }
            if ps_dec.lpc_order == 16 {
                for j in 10..16 {
                    lpc_pred_q10 = silk_smlawb(
                        lpc_pred_q10,
                        s_lpc_q14[MAX_LPC_ORDER + i - 1 - j],
                        a_q12_tmp[j] as i32,
                    );
                }
            }

            /* Add prediction to LPC excitation */
            s_lpc_q14[MAX_LPC_ORDER + i] =
                silk_add_sat32(pres_q14[i], silk_lshift_sat32(lpc_pred_q10, 4));

            /* Scale with gain */
            xq[pxq + i] = silk_sat16(silk_rshift_round(
                silk_smulww(s_lpc_q14[MAX_LPC_ORDER + i], gain_q10),
                8,
            )) as i16;
        }

        /* Update LPC filter state */
        s_lpc_q14.copy_within(subfr_length..subfr_length + MAX_LPC_ORDER, 0);
        pexc_q14 += subfr_length;
        pxq += subfr_length;
    }

    /* Save LPC state */
    ps_dec
        .s_lpc_q14_buf
        .copy_from_slice(&s_lpc_q14[..MAX_LPC_ORDER]);
}
