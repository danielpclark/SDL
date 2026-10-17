// Rust translation of silk/decode_frame.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding one SILK frame.

use super::cng::silk_cng;
use super::decode_core::silk_decode_core;
use super::decode_indices::silk_decode_indices;
use super::decode_parameters::silk_decode_parameters;
use super::decode_pulses::silk_decode_pulses;
use super::define::{
    FLAG_DECODE_LBRR, FLAG_DECODE_NORMAL, MAX_FRAME_LENGTH, SHELL_CODEC_FRAME_LENGTH,
};
use super::plc::{silk_plc, silk_plc_glue_frames};
use super::structs::{SilkDecoderControl, SilkDecoderState};
use crate::opus::celt::entcode::EcDec;

/****************/
/* Decode frame */
/****************/
/// Translation of `silk_decode_frame()`.
pub(crate) fn silk_decode_frame(
    ps_dec: &mut SilkDecoderState,
    ps_range_dec: &mut EcDec<'_>,
    p_out: &mut [i16],
    p_n: &mut i32,
    lost_flag: i32,
    cond_coding: i32,
) -> i32 {
    let ret = 0;

    let l = ps_dec.frame_length;
    let mut ps_dec_ctrl = SilkDecoderControl::default();
    ps_dec_ctrl.ltp_scale_q14 = 0;

    /* Safety checks */
    debug_assert!(l > 0 && l <= MAX_FRAME_LENGTH as i32);

    if lost_flag == FLAG_DECODE_NORMAL
        || (lost_flag == FLAG_DECODE_LBRR
            && ps_dec.lbrr_flags[ps_dec.n_frames_decoded as usize] == 1)
    {
        let mut pulses = [0i16;
            (MAX_FRAME_LENGTH + SHELL_CODEC_FRAME_LENGTH - 1) & !(SHELL_CODEC_FRAME_LENGTH - 1)];
        /*********************************************/
        /* Decode quantization indices of side info  */
        /*********************************************/
        silk_decode_indices(
            ps_dec,
            ps_range_dec,
            ps_dec.n_frames_decoded,
            lost_flag,
            cond_coding,
        );

        /*********************************************/
        /* Decode quantization indices of excitation */
        /*********************************************/
        silk_decode_pulses(
            ps_range_dec,
            &mut pulses,
            ps_dec.indices.signal_type as i32,
            ps_dec.indices.quant_offset_type as i32,
            ps_dec.frame_length,
        );

        /********************************************/
        /* Decode parameters and pulse signal       */
        /********************************************/
        silk_decode_parameters(ps_dec, &mut ps_dec_ctrl, cond_coding);

        /********************************************************/
        /* Run inverse NSQ                                      */
        /********************************************************/
        silk_decode_core(ps_dec, &mut ps_dec_ctrl, p_out, &pulses);

        /********************************************************/
        /* Update PLC state                                     */
        /********************************************************/
        silk_plc(ps_dec, &mut ps_dec_ctrl, p_out, 0);

        ps_dec.loss_cnt = 0;
        ps_dec.prev_signal_type = ps_dec.indices.signal_type as i32;
        debug_assert!(ps_dec.prev_signal_type >= 0 && ps_dec.prev_signal_type <= 2);

        /* A frame has been decoded without errors */
        ps_dec.first_frame_after_reset = 0;
    } else {
        /* Handle packet loss by extrapolation */
        silk_plc(ps_dec, &mut ps_dec_ctrl, p_out, 1);
    }

    /*************************/
    /* Update output buffer. */
    /*************************/
    debug_assert!(ps_dec.ltp_mem_length >= ps_dec.frame_length);
    let mv_len = (ps_dec.ltp_mem_length - ps_dec.frame_length) as usize;
    let fl = ps_dec.frame_length as usize;
    ps_dec.out_buf.copy_within(fl..fl + mv_len, 0);
    ps_dec.out_buf[mv_len..mv_len + fl].copy_from_slice(&p_out[..fl]);

    /************************************************/
    /* Comfort noise generation / estimation        */
    /************************************************/
    silk_cng(ps_dec, &ps_dec_ctrl, p_out, l);

    /****************************************************************/
    /* Ensure smooth connection of extrapolated and good frames     */
    /****************************************************************/
    silk_plc_glue_frames(ps_dec, p_out, l);

    /* Update some decoder state variables */
    ps_dec.lag_prev = ps_dec_ctrl.pitch_l[(ps_dec.nb_subfr - 1) as usize];

    /* Set output frame length */
    *p_n = l;

    ret
}
