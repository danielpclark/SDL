// Rust translation of silk/CNG.c from libopus 1.4 (https://opus-codec.org,
// at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Comfort noise generation.

use super::define::{
    CNG_BUF_MASK_MAX, CNG_GAIN_SMTH_Q16, CNG_GAIN_SMTH_THRESHOLD_Q16, CNG_NLSF_SMTH_Q16,
    MAX_FRAME_LENGTH, MAX_LPC_ORDER, TYPE_NO_VOICE_ACTIVITY,
};
use super::nlsf::silk_nlsf2a;
use super::sigproc_fix::{
    silk_add_sat16, silk_add_sat32, silk_div32_16, silk_lshift, silk_lshift_sat32, silk_rand,
    silk_rshift, silk_rshift_round, silk_sat16, silk_smlawb, silk_smultt, silk_smulwb, silk_smulww,
    silk_sqrt_approx, silk_sub_lshift32, SILK_INT16_MAX,
};
use super::structs::{SilkDecoderControl, SilkDecoderState};

/* Generates excitation for CNG LPC synthesis */
#[inline(always)]
fn silk_cng_exc(
    exc_q14: &mut [i32], /* O    CNG excitation signal Q10                   */
    exc_buf_q14: &[i32], /* I    Random samples buffer Q10                   */
    length: i32,         /* I    Length                                      */
    rand_seed: &mut i32, /* I/O  Seed to random index generator              */
) {
    let mut exc_mask = CNG_BUF_MASK_MAX;
    while exc_mask > length {
        exc_mask = silk_rshift(exc_mask, 1);
    }

    let mut seed = *rand_seed;
    for v in exc_q14[..length as usize].iter_mut() {
        seed = silk_rand(seed);
        let idx = (silk_rshift(seed, 24) & exc_mask) as usize;
        debug_assert!(idx as i32 <= CNG_BUF_MASK_MAX);
        *v = exc_buf_q14[idx];
    }
    *rand_seed = seed;
}

/// Translation of `silk_CNG_Reset()`.
pub(crate) fn silk_cng_reset(
    ps_dec: &mut SilkDecoderState, /* I/O  Decoder state                               */
) {
    let nlsf_step_q15 = silk_div32_16(SILK_INT16_MAX, ps_dec.lpc_order + 1);
    let mut nlsf_acc_q15 = 0i32;
    for i in 0..ps_dec.lpc_order as usize {
        nlsf_acc_q15 += nlsf_step_q15;
        ps_dec.s_cng.cng_smth_nlsf_q15[i] = nlsf_acc_q15 as i16;
    }
    ps_dec.s_cng.cng_smth_gain_q16 = 0;
    ps_dec.s_cng.rand_seed = 3176576;
}

/* Updates CNG estimate, and applies the CNG when packet was lost   */
/// Translation of `silk_CNG()`.
pub(crate) fn silk_cng(
    ps_dec: &mut SilkDecoderState, /* I/O  Decoder state                               */
    ps_dec_ctrl: &SilkDecoderControl, /* I/O  Decoder control                             */
    frame: &mut [i16],             /* I/O  Signal                                      */
    length: i32,                   /* I    Length of residual                          */
) {
    let mut a_q12 = [0i16; MAX_LPC_ORDER];

    if ps_dec.fs_khz != ps_dec.s_cng.fs_khz {
        /* Reset state */
        silk_cng_reset(ps_dec);

        ps_dec.s_cng.fs_khz = ps_dec.fs_khz;
    }
    let order = ps_dec.lpc_order as usize;
    if ps_dec.loss_cnt == 0 && ps_dec.prev_signal_type == TYPE_NO_VOICE_ACTIVITY {
        let ps_cng = &mut ps_dec.s_cng;
        /* Update CNG parameters */

        /* Smoothing of LSF's  */
        for i in 0..order {
            ps_cng.cng_smth_nlsf_q15[i] = (ps_cng.cng_smth_nlsf_q15[i] as i32
                + silk_smulwb(
                    ps_dec.prev_nlsf_q15[i] as i32 - ps_cng.cng_smth_nlsf_q15[i] as i32,
                    CNG_NLSF_SMTH_Q16,
                )) as i16;
        }
        /* Find the subframe with the highest gain */
        let mut max_gain_q16 = 0;
        let mut subfr = 0usize;
        for i in 0..ps_dec.nb_subfr as usize {
            if ps_dec_ctrl.gains_q16[i] > max_gain_q16 {
                max_gain_q16 = ps_dec_ctrl.gains_q16[i];
                subfr = i;
            }
        }
        /* Update CNG excitation buffer with excitation from this subframe */
        let sl = ps_dec.subfr_length as usize;
        ps_cng
            .cng_exc_buf_q14
            .copy_within(0..(ps_dec.nb_subfr as usize - 1) * sl, sl);
        ps_cng.cng_exc_buf_q14[..sl].copy_from_slice(&ps_dec.exc_q14[subfr * sl..subfr * sl + sl]);

        /* Smooth gains */
        for i in 0..ps_dec.nb_subfr as usize {
            ps_cng.cng_smth_gain_q16 = ps_cng.cng_smth_gain_q16.wrapping_add(silk_smulwb(
                ps_dec_ctrl.gains_q16[i].wrapping_sub(ps_cng.cng_smth_gain_q16),
                CNG_GAIN_SMTH_Q16,
            ));
            /* If the smoothed gain is 3 dB greater than this subframe's gain, use this subframe's gain to adapt faster. */
            if silk_smulww(ps_cng.cng_smth_gain_q16, CNG_GAIN_SMTH_THRESHOLD_Q16)
                > ps_dec_ctrl.gains_q16[i]
            {
                ps_cng.cng_smth_gain_q16 = ps_dec_ctrl.gains_q16[i];
            }
        }
    }

    /* Add CNG when packet is lost or during DTX */
    if ps_dec.loss_cnt != 0 {
        let len = length as usize;
        let mut cng_sig_q14 = [0i32; MAX_FRAME_LENGTH + MAX_LPC_ORDER];
        let ps_cng = &mut ps_dec.s_cng;

        /* Generate CNG excitation */
        let mut gain_q16 = silk_smulww(
            ps_dec.s_plc.rand_scale_q14 as i32,
            ps_dec.s_plc.prev_gain_q16[1],
        );
        if gain_q16 >= (1 << 21) || ps_cng.cng_smth_gain_q16 > (1 << 23) {
            gain_q16 = silk_smultt(gain_q16, gain_q16);
            gain_q16 = silk_sub_lshift32(
                silk_smultt(ps_cng.cng_smth_gain_q16, ps_cng.cng_smth_gain_q16),
                gain_q16,
                5,
            );
            gain_q16 = silk_lshift(silk_sqrt_approx(gain_q16), 16);
        } else {
            gain_q16 = silk_smulww(gain_q16, gain_q16);
            gain_q16 = silk_sub_lshift32(
                silk_smulww(ps_cng.cng_smth_gain_q16, ps_cng.cng_smth_gain_q16),
                gain_q16,
                5,
            );
            gain_q16 = silk_lshift(silk_sqrt_approx(gain_q16), 8);
        }
        let gain_q10 = silk_rshift(gain_q16, 6);

        silk_cng_exc(
            &mut cng_sig_q14[MAX_LPC_ORDER..],
            &ps_cng.cng_exc_buf_q14,
            length,
            &mut ps_cng.rand_seed,
        );

        /* Convert CNG NLSF to filter representation */
        silk_nlsf2a(&mut a_q12, &ps_cng.cng_smth_nlsf_q15, ps_dec.lpc_order);

        /* Generate CNG signal, by synthesis filtering */
        cng_sig_q14[..MAX_LPC_ORDER].copy_from_slice(&ps_cng.cng_synth_state);
        debug_assert!(ps_dec.lpc_order == 10 || ps_dec.lpc_order == 16);
        for i in 0..len {
            let s = &cng_sig_q14[i..MAX_LPC_ORDER + i];
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            let mut lpc_pred_q10 = silk_rshift(ps_dec.lpc_order, 1);
            for j in 0..10 {
                lpc_pred_q10 = silk_smlawb(lpc_pred_q10, s[MAX_LPC_ORDER - 1 - j], a_q12[j] as i32);
            }
            if ps_dec.lpc_order == 16 {
                for j in 10..16 {
                    lpc_pred_q10 =
                        silk_smlawb(lpc_pred_q10, s[MAX_LPC_ORDER - 1 - j], a_q12[j] as i32);
                }
            }

            /* Update states */
            cng_sig_q14[MAX_LPC_ORDER + i] = silk_add_sat32(
                cng_sig_q14[MAX_LPC_ORDER + i],
                silk_lshift_sat32(lpc_pred_q10, 4),
            );

            /* Scale with Gain and add to input signal */
            frame[i] = silk_add_sat16(
                frame[i] as i32,
                silk_sat16(silk_rshift_round(
                    silk_smulww(cng_sig_q14[MAX_LPC_ORDER + i], gain_q10),
                    8,
                )),
            );
        }
        ps_cng
            .cng_synth_state
            .copy_from_slice(&cng_sig_q14[len..len + MAX_LPC_ORDER]);
    } else {
        ps_dec.s_cng.cng_synth_state[..order].fill(0);
    }
}
