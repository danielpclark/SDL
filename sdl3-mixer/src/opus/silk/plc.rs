// Rust translation of silk/PLC.c from libopus 1.4 (https://opus-codec.org,
// at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SILK packet loss concealment (`SMALL_FOOTPRINT` is not defined).

use super::define::{
    BWE_COEF, LOG2_INV_LPC_GAIN_HIGH_THRES, LOG2_INV_LPC_GAIN_LOW_THRES, LTP_MEM_LENGTH_MS,
    LTP_ORDER, MAX_FRAME_LENGTH, MAX_FS_KHZ, MAX_LPC_ORDER, MAX_NB_SUBFR, MAX_PITCH_LAG_MS,
    MAX_SUB_FRAME_LENGTH, PITCH_DRIFT_FAC_Q16, RAND_BUF_MASK, RAND_BUF_SIZE, TYPE_VOICED,
    V_PITCH_GAIN_START_MAX_Q14, V_PITCH_GAIN_START_MIN_Q14,
};
use super::lpc_analysis_filter::silk_lpc_analysis_filter;
use super::nlsf::{silk_bwexpander, silk_lpc_inverse_pred_gain};
use super::sigproc_fix::{
    silk_add_sat32, silk_clz32, silk_div32, silk_div32_16, silk_fix_const, silk_inverse32_varq,
    silk_lshift, silk_lshift_sat32, silk_rand, silk_rshift, silk_rshift_round, silk_sat16,
    silk_smlawb, silk_smulbb, silk_smulwb, silk_smulww, silk_sqrt_approx, SILK_INT32_MAX,
};
use super::structs::{SilkDecoderControl, SilkDecoderState};
use super::sum_sqr_shift::silk_sum_sqr_shift;

const NB_ATT: usize = 2;
const HARM_ATT_Q15: [i16; NB_ATT] = [32440, 31130]; /* 0.99, 0.95 */
const PLC_RAND_ATTENUATE_V_Q15: [i16; NB_ATT] = [31130, 26214]; /* 0.95, 0.8 */
const PLC_RAND_ATTENUATE_UV_Q15: [i16; NB_ATT] = [32440, 29491]; /* 0.99, 0.9 */

/// Translation of `silk_PLC_Reset()`.
pub(crate) fn silk_plc_reset(ps_dec: &mut SilkDecoderState /* I/O Decoder state        */) {
    ps_dec.s_plc.pitch_l_q8 = silk_lshift(ps_dec.frame_length, 8 - 1);
    ps_dec.s_plc.prev_gain_q16[0] = silk_fix_const(1.0, 16);
    ps_dec.s_plc.prev_gain_q16[1] = silk_fix_const(1.0, 16);
    ps_dec.s_plc.subfr_length = 20;
    ps_dec.s_plc.nb_subfr = 2;
}

/// Translation of `silk_PLC()`.
pub(crate) fn silk_plc(
    ps_dec: &mut SilkDecoderState,        /* I/O Decoder state        */
    ps_dec_ctrl: &mut SilkDecoderControl, /* I/O Decoder control      */
    frame: &mut [i16],                    /* I/O  signal              */
    lost: i32,                            /* I Loss flag              */
) {
    /* PLC control function */
    if ps_dec.fs_khz != ps_dec.s_plc.fs_khz {
        silk_plc_reset(ps_dec);
        ps_dec.s_plc.fs_khz = ps_dec.fs_khz;
    }

    if lost != 0 {
        /****************************/
        /* Generate Signal          */
        /****************************/
        silk_plc_conceal(ps_dec, ps_dec_ctrl, frame);

        ps_dec.loss_cnt += 1;
    } else {
        /****************************/
        /* Update state             */
        /****************************/
        silk_plc_update(ps_dec, ps_dec_ctrl);
    }
}

/**************************************************/
/* Update state of PLC                            */
/**************************************************/
#[inline(always)]
fn silk_plc_update(
    ps_dec: &mut SilkDecoderState,    /* I/O Decoder state        */
    ps_dec_ctrl: &SilkDecoderControl, /* I/O Decoder control      */
) {
    let nb_subfr = ps_dec.nb_subfr as usize;

    /* Update parameters used in case of packet loss */
    ps_dec.prev_signal_type = ps_dec.indices.signal_type as i32;
    let mut ltp_gain_q14: i32 = 0;
    let ps_plc = &mut ps_dec.s_plc;
    if ps_dec.indices.signal_type as i32 == TYPE_VOICED {
        /* Find the parameters for the last subframe which contains a pitch pulse */
        let mut j = 0usize;
        while (j as i32) * ps_dec.subfr_length < ps_dec_ctrl.pitch_l[nb_subfr - 1] {
            if j == nb_subfr {
                break;
            }
            let mut temp_ltp_gain_q14: i32 = 0;
            for i in 0..LTP_ORDER {
                temp_ltp_gain_q14 +=
                    ps_dec_ctrl.ltp_coef_q14[(nb_subfr - 1 - j) * LTP_ORDER + i] as i32;
            }
            if temp_ltp_gain_q14 > ltp_gain_q14 {
                ltp_gain_q14 = temp_ltp_gain_q14;
                let o = silk_smulbb((nb_subfr - 1 - j) as i32, LTP_ORDER as i32) as usize;
                ps_plc
                    .ltp_coef_q14
                    .copy_from_slice(&ps_dec_ctrl.ltp_coef_q14[o..o + LTP_ORDER]);

                ps_plc.pitch_l_q8 = silk_lshift(ps_dec_ctrl.pitch_l[nb_subfr - 1 - j], 8);
            }
            j += 1;
        }

        ps_plc.ltp_coef_q14 = [0; LTP_ORDER];
        ps_plc.ltp_coef_q14[LTP_ORDER / 2] = ltp_gain_q14 as i16;

        /* Limit LT coefs */
        if ltp_gain_q14 < V_PITCH_GAIN_START_MIN_Q14 {
            let tmp = silk_lshift(V_PITCH_GAIN_START_MIN_Q14, 10);
            let scale_q10 = silk_div32(tmp, ltp_gain_q14.max(1));
            for v in ps_plc.ltp_coef_q14.iter_mut() {
                *v = silk_rshift(silk_smulbb(*v as i32, scale_q10), 10) as i16;
            }
        } else if ltp_gain_q14 > V_PITCH_GAIN_START_MAX_Q14 {
            let tmp = silk_lshift(V_PITCH_GAIN_START_MAX_Q14, 14);
            let scale_q14 = silk_div32(tmp, ltp_gain_q14.max(1));
            for v in ps_plc.ltp_coef_q14.iter_mut() {
                *v = silk_rshift(silk_smulbb(*v as i32, scale_q14), 14) as i16;
            }
        }
    } else {
        ps_plc.pitch_l_q8 = silk_lshift(silk_smulbb(ps_dec.fs_khz, 18), 8);
        ps_plc.ltp_coef_q14 = [0; LTP_ORDER];
    }

    /* Save LPC coeficients */
    let order = ps_dec.lpc_order as usize;
    ps_plc.prev_lpc_q12[..order].copy_from_slice(&ps_dec_ctrl.pred_coef_q12[1][..order]);
    ps_plc.prev_ltp_scale_q14 = ps_dec_ctrl.ltp_scale_q14 as i16;

    /* Save last two gains */
    ps_plc
        .prev_gain_q16
        .copy_from_slice(&ps_dec_ctrl.gains_q16[nb_subfr - 2..nb_subfr]);

    ps_plc.subfr_length = ps_dec.subfr_length;
    ps_plc.nb_subfr = ps_dec.nb_subfr;
}

/// Translation of `silk_PLC_energy()`; returns `(energy1, shift1, energy2, shift2)`.
#[inline(always)]
fn silk_plc_energy(
    exc_q14: &[i32],
    prev_gain_q10: &[i32; 2],
    subfr_length: i32,
    nb_subfr: i32,
) -> (i32, i32, i32, i32) {
    let sl = subfr_length as usize;
    let mut exc_buf = [0i16; 2 * MAX_SUB_FRAME_LENGTH];
    /* Find random noise component */
    /* Scale previous excitation signal */
    for k in 0..2usize {
        for i in 0..sl {
            exc_buf[k * sl + i] = silk_sat16(silk_rshift(
                silk_smulww(
                    exc_q14[i + (k + nb_subfr as usize - 2) * sl],
                    prev_gain_q10[k],
                ),
                8,
            )) as i16;
        }
    }
    /* Find the subframe with lowest energy of the last two and use that as random noise generator */
    let (energy1, shift1) = silk_sum_sqr_shift(&exc_buf, subfr_length);
    let (energy2, shift2) = silk_sum_sqr_shift(&exc_buf[sl..], subfr_length);
    (energy1, shift1, energy2, shift2)
}

const LTP_BUF_LENGTH: usize = (LTP_MEM_LENGTH_MS * MAX_FS_KHZ) as usize + MAX_FRAME_LENGTH;

#[inline(always)]
fn silk_plc_conceal(
    ps_dec: &mut SilkDecoderState,        /* I/O Decoder state        */
    ps_dec_ctrl: &mut SilkDecoderControl, /* I/O Decoder control      */
    frame: &mut [i16],                    /* O LPC residual signal    */
) {
    let mut a_q12 = [0i16; MAX_LPC_ORDER];
    let mut s_ltp = [0i16; (LTP_MEM_LENGTH_MS * MAX_FS_KHZ) as usize];
    let mut s_ltp_q14 = [0i32; LTP_BUF_LENGTH];
    let mut prev_gain_q10 = [0i32; 2];
    let ltp_mem_length = ps_dec.ltp_mem_length as usize;
    let order = ps_dec.lpc_order as usize;

    prev_gain_q10[0] = silk_rshift(ps_dec.s_plc.prev_gain_q16[0], 6);
    prev_gain_q10[1] = silk_rshift(ps_dec.s_plc.prev_gain_q16[1], 6);

    if ps_dec.first_frame_after_reset != 0 {
        ps_dec.s_plc.prev_lpc_q12 = [0; MAX_LPC_ORDER];
    }

    let (energy1, shift1, energy2, shift2) = silk_plc_energy(
        &ps_dec.exc_q14,
        &prev_gain_q10,
        ps_dec.subfr_length,
        ps_dec.nb_subfr,
    );

    let ps_plc = &mut ps_dec.s_plc;
    let rand_off = if silk_rshift(energy1, shift2) < silk_rshift(energy2, shift1) {
        /* First sub-frame has lowest energy */
        0.max((ps_plc.nb_subfr - 1) * ps_plc.subfr_length - RAND_BUF_SIZE)
    } else {
        /* Second sub-frame has lowest energy */
        0.max(ps_plc.nb_subfr * ps_plc.subfr_length - RAND_BUF_SIZE)
    } as usize;
    let rand_ptr = &ps_dec.exc_q14[rand_off..];

    /* Set up Gain to random noise component */
    /* (B_Q14 is psPLC->LTPCoef_Q14, used in place) */
    let mut rand_scale_q14 = ps_plc.rand_scale_q14;

    /* Set up attenuation gains */
    let att = (NB_ATT as i32 - 1).min(ps_dec.loss_cnt) as usize;
    let harm_gain_q15 = HARM_ATT_Q15[att] as i32;
    let mut rand_gain_q15 = if ps_dec.prev_signal_type == TYPE_VOICED {
        PLC_RAND_ATTENUATE_V_Q15[att] as i32
    } else {
        PLC_RAND_ATTENUATE_UV_Q15[att] as i32
    };

    /* LPC concealment. Apply BWE to previous LPC */
    silk_bwexpander(
        &mut ps_plc.prev_lpc_q12,
        ps_dec.lpc_order,
        silk_fix_const(BWE_COEF, 16),
    );

    /* Preload LPC coeficients to array on stack. Gives small performance gain */
    a_q12[..order].copy_from_slice(&ps_plc.prev_lpc_q12[..order]);

    /* First Lost frame */
    if ps_dec.loss_cnt == 0 {
        rand_scale_q14 = 1 << 14;

        /* Reduce random noise Gain for voiced frames */
        if ps_dec.prev_signal_type == TYPE_VOICED {
            for i in 0..LTP_ORDER {
                rand_scale_q14 = rand_scale_q14.wrapping_sub(ps_plc.ltp_coef_q14[i]);
            }
            rand_scale_q14 = 3277i16.max(rand_scale_q14); /* 0.2 */
            rand_scale_q14 = silk_rshift(
                silk_smulbb(rand_scale_q14 as i32, ps_plc.prev_ltp_scale_q14 as i32),
                14,
            ) as i16;
        } else {
            /* Reduce random noise for unvoiced frames with high LPC gain */
            let inv_gain_q30 = silk_lpc_inverse_pred_gain(&ps_plc.prev_lpc_q12, ps_dec.lpc_order);

            let mut down_scale_q30 =
                silk_rshift(1i32 << 30, LOG2_INV_LPC_GAIN_HIGH_THRES).min(inv_gain_q30);
            down_scale_q30 =
                silk_rshift(1i32 << 30, LOG2_INV_LPC_GAIN_LOW_THRES).max(down_scale_q30);
            down_scale_q30 = silk_lshift(down_scale_q30, LOG2_INV_LPC_GAIN_HIGH_THRES);

            rand_gain_q15 = silk_rshift(silk_smulwb(down_scale_q30, rand_gain_q15), 14);
        }
    }

    let mut rand_seed = ps_plc.rand_seed;
    let mut lag = silk_rshift_round(ps_plc.pitch_l_q8, 8);
    let mut s_ltp_buf_idx = ltp_mem_length;

    /* Rewhiten LTP state */
    let idx = ps_dec.ltp_mem_length - lag - ps_dec.lpc_order - LTP_ORDER as i32 / 2;
    debug_assert!(idx > 0);
    let idx = idx as usize;
    silk_lpc_analysis_filter(
        &mut s_ltp[idx..],
        &ps_dec.out_buf[idx..],
        &a_q12,
        ps_dec.ltp_mem_length - idx as i32,
        ps_dec.lpc_order,
    );
    /* Scale LTP state */
    let mut inv_gain_q30 = silk_inverse32_varq(ps_plc.prev_gain_q16[1], 46);
    inv_gain_q30 = inv_gain_q30.min(SILK_INT32_MAX >> 1);
    for i in idx + order..ltp_mem_length {
        s_ltp_q14[i] = silk_smulwb(inv_gain_q30, s_ltp[i] as i32);
    }

    /***************************/
    /* LTP synthesis filtering */
    /***************************/
    for _k in 0..ps_dec.nb_subfr {
        /* Set up pointer */
        let mut pred_lag_ptr = (s_ltp_buf_idx as i32 - lag + LTP_ORDER as i32 / 2) as usize;
        let b_q14 = &mut ps_plc.ltp_coef_q14;
        for _i in 0..ps_dec.subfr_length {
            /* Unrolled loop */
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            let mut ltp_pred_q12 = 2;
            ltp_pred_q12 = silk_smlawb(ltp_pred_q12, s_ltp_q14[pred_lag_ptr], b_q14[0] as i32);
            ltp_pred_q12 = silk_smlawb(ltp_pred_q12, s_ltp_q14[pred_lag_ptr - 1], b_q14[1] as i32);
            ltp_pred_q12 = silk_smlawb(ltp_pred_q12, s_ltp_q14[pred_lag_ptr - 2], b_q14[2] as i32);
            ltp_pred_q12 = silk_smlawb(ltp_pred_q12, s_ltp_q14[pred_lag_ptr - 3], b_q14[3] as i32);
            ltp_pred_q12 = silk_smlawb(ltp_pred_q12, s_ltp_q14[pred_lag_ptr - 4], b_q14[4] as i32);
            pred_lag_ptr += 1;

            /* Generate LPC excitation */
            rand_seed = silk_rand(rand_seed);
            let idx = (silk_rshift(rand_seed, 25) & RAND_BUF_MASK) as usize;
            s_ltp_q14[s_ltp_buf_idx] = silk_lshift(
                silk_smlawb(ltp_pred_q12, rand_ptr[idx], rand_scale_q14 as i32),
                2,
            );
            s_ltp_buf_idx += 1;
        }

        /* Gradually reduce LTP gain */
        for b in b_q14.iter_mut() {
            *b = silk_rshift(silk_smulbb(harm_gain_q15, *b as i32), 15) as i16;
        }
        /* Gradually reduce excitation gain */
        rand_scale_q14 = silk_rshift(silk_smulbb(rand_scale_q14 as i32, rand_gain_q15), 15) as i16;

        /* Slowly increase pitch lag */
        ps_plc.pitch_l_q8 = silk_smlawb(ps_plc.pitch_l_q8, ps_plc.pitch_l_q8, PITCH_DRIFT_FAC_Q16);
        ps_plc.pitch_l_q8 = ps_plc
            .pitch_l_q8
            .min(silk_lshift(silk_smulbb(MAX_PITCH_LAG_MS, ps_dec.fs_khz), 8));
        lag = silk_rshift_round(ps_plc.pitch_l_q8, 8);
    }

    /***************************/
    /* LPC synthesis filtering */
    /***************************/
    let s_lpc_q14 = &mut s_ltp_q14[ltp_mem_length - MAX_LPC_ORDER..];

    /* Copy LPC state */
    s_lpc_q14[..MAX_LPC_ORDER].copy_from_slice(&ps_dec.s_lpc_q14_buf);

    debug_assert!(ps_dec.lpc_order >= 10); /* check that unrolling works */
    let frame_length = ps_dec.frame_length as usize;
    for i in 0..frame_length {
        /* partly unrolled */
        /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
        let mut lpc_pred_q10 = silk_rshift(ps_dec.lpc_order, 1);
        for j in 0..order {
            lpc_pred_q10 = silk_smlawb(
                lpc_pred_q10,
                s_lpc_q14[MAX_LPC_ORDER + i - j - 1],
                a_q12[j] as i32,
            );
        }

        /* Add prediction to LPC excitation */
        s_lpc_q14[MAX_LPC_ORDER + i] = silk_add_sat32(
            s_lpc_q14[MAX_LPC_ORDER + i],
            silk_lshift_sat32(lpc_pred_q10, 4),
        );

        /* Scale with Gain */
        frame[i] = silk_sat16(silk_sat16(silk_rshift_round(
            silk_smulww(s_lpc_q14[MAX_LPC_ORDER + i], prev_gain_q10[1]),
            8,
        ))) as i16;
    }

    /* Save LPC state */
    ps_dec
        .s_lpc_q14_buf
        .copy_from_slice(&s_lpc_q14[frame_length..frame_length + MAX_LPC_ORDER]);

    /**************************************/
    /* Update states                      */
    /**************************************/
    ps_plc.rand_seed = rand_seed;
    ps_plc.rand_scale_q14 = rand_scale_q14;
    ps_dec_ctrl.pitch_l = [lag; MAX_NB_SUBFR];
}

/* Glues concealed frames with new good received frames */
/// Translation of `silk_PLC_glue_frames()`.
pub(crate) fn silk_plc_glue_frames(
    ps_dec: &mut SilkDecoderState, /* I/O decoder state        */
    frame: &mut [i16],             /* I/O signal               */
    length: i32,                   /* I length of signal       */
) {
    let ps_plc = &mut ps_dec.s_plc;

    if ps_dec.loss_cnt != 0 {
        /* Calculate energy in concealed residual */
        (ps_plc.conc_energy, ps_plc.conc_energy_shift) = silk_sum_sqr_shift(frame, length);

        ps_plc.last_frame_lost = 1;
    } else {
        if ps_plc.last_frame_lost != 0 {
            /* Calculate residual in decoded signal if last frame was lost */
            let (mut energy, energy_shift) = silk_sum_sqr_shift(frame, length);

            /* Normalize energies */
            if energy_shift > ps_plc.conc_energy_shift {
                ps_plc.conc_energy =
                    silk_rshift(ps_plc.conc_energy, energy_shift - ps_plc.conc_energy_shift);
            } else if energy_shift < ps_plc.conc_energy_shift {
                energy = silk_rshift(energy, ps_plc.conc_energy_shift - energy_shift);
            }

            /* Fade in the energy difference */
            if energy > ps_plc.conc_energy {
                let mut lz = silk_clz32(ps_plc.conc_energy);
                lz -= 1;
                ps_plc.conc_energy = silk_lshift(ps_plc.conc_energy, lz);
                energy = silk_rshift(energy, (24 - lz).max(0));

                let frac_q24 = silk_div32(ps_plc.conc_energy, energy.max(1));

                let mut gain_q16 = silk_lshift(silk_sqrt_approx(frac_q24), 4);
                let mut slope_q16 = silk_div32_16((1i32 << 16) - gain_q16, length);
                /* Make slope 4x steeper to avoid missing onsets after DTX */
                slope_q16 = silk_lshift(slope_q16, 2);

                for v in frame[..length as usize].iter_mut() {
                    *v = silk_smulwb(gain_q16, *v as i32) as i16;
                    gain_q16 += slope_q16;
                    if gain_q16 > 1i32 << 16 {
                        break;
                    }
                }
            }
        }
        ps_plc.last_frame_lost = 0;
    }
}
