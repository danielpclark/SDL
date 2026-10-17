// Rust translation of silk/decode_indices.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding the side information of a SILK frame.

use super::define::{
    CODE_CONDITIONALLY, CODE_INDEPENDENTLY, MAX_LPC_ORDER, MAX_NB_SUBFR, NLSF_QUANT_MAX_AMPLITUDE,
    TYPE_VOICED,
};
use super::nlsf::silk_nlsf_unpack;
use super::sigproc_fix::{silk_lshift, silk_rshift};
use super::structs::SilkDecoderState;
use super::tables::{
    SILK_DELTA_GAIN_ICDF, SILK_GAIN_ICDF, SILK_LTPSCALE_ICDF, SILK_LTP_GAIN_ICDF_PTRS,
    SILK_LTP_PER_INDEX_ICDF, SILK_NLSF_EXT_ICDF, SILK_NLSF_INTERPOLATION_FACTOR_ICDF,
    SILK_PITCH_DELTA_ICDF, SILK_PITCH_LAG_ICDF, SILK_TYPE_OFFSET_NO_VAD_ICDF,
    SILK_TYPE_OFFSET_VAD_ICDF, SILK_UNIFORM4_ICDF, SILK_UNIFORM8_ICDF,
};
use crate::opus::celt::entcode::EcDec;

/* Decode side-information parameters from payload */
/// Translation of `silk_decode_indices()`.
pub(crate) fn silk_decode_indices(
    ps_dec: &mut SilkDecoderState,
    ps_range_dec: &mut EcDec<'_>,
    frame_index: i32,
    decode_lbrr: i32,
    cond_coding: i32,
) {
    let mut ix: i32;
    let mut ec_ix = [0i16; MAX_LPC_ORDER];
    let mut pred_q8 = [0u8; MAX_LPC_ORDER];

    /*******************************************/
    /* Decode signal type and quantizer offset */
    /*******************************************/
    if decode_lbrr != 0 || ps_dec.vad_flags[frame_index as usize] != 0 {
        ix = ps_range_dec.ec_dec_icdf(&SILK_TYPE_OFFSET_VAD_ICDF, 8) + 2;
    } else {
        ix = ps_range_dec.ec_dec_icdf(&SILK_TYPE_OFFSET_NO_VAD_ICDF, 8);
    }
    ps_dec.indices.signal_type = silk_rshift(ix, 1) as i8;
    ps_dec.indices.quant_offset_type = (ix & 1) as i8;

    /****************/
    /* Decode gains */
    /****************/
    /* First subframe */
    if cond_coding == CODE_CONDITIONALLY {
        /* Conditional coding */
        ps_dec.indices.gains_indices[0] = ps_range_dec.ec_dec_icdf(&SILK_DELTA_GAIN_ICDF, 8) as i8;
    } else {
        /* Independent coding, in two stages: MSB bits followed by 3 LSBs */
        ps_dec.indices.gains_indices[0] = silk_lshift(
            ps_range_dec.ec_dec_icdf(&SILK_GAIN_ICDF[ps_dec.indices.signal_type as usize], 8),
            3,
        ) as i8;
        ps_dec.indices.gains_indices[0] = ps_dec.indices.gains_indices[0]
            .wrapping_add(ps_range_dec.ec_dec_icdf(&SILK_UNIFORM8_ICDF, 8) as i8);
    }

    /* Remaining subframes */
    for i in 1..ps_dec.nb_subfr as usize {
        ps_dec.indices.gains_indices[i] = ps_range_dec.ec_dec_icdf(&SILK_DELTA_GAIN_ICDF, 8) as i8;
    }

    /**********************/
    /* Decode LSF Indices */
    /**********************/
    let cb = ps_dec.ps_nlsf_cb;
    ps_dec.indices.nlsf_indices[0] = ps_range_dec.ec_dec_icdf(
        &cb.cb1_icdf[((ps_dec.indices.signal_type >> 1) as i32 * cb.n_vectors as i32) as usize..],
        8,
    ) as i8;
    silk_nlsf_unpack(
        &mut ec_ix,
        &mut pred_q8,
        cb,
        ps_dec.indices.nlsf_indices[0] as i32,
    );
    debug_assert!(cb.order as i32 == ps_dec.lpc_order);
    for i in 0..cb.order as usize {
        ix = ps_range_dec.ec_dec_icdf(&cb.ec_icdf[ec_ix[i] as usize..], 8);
        if ix == 0 {
            ix -= ps_range_dec.ec_dec_icdf(&SILK_NLSF_EXT_ICDF, 8);
        } else if ix == 2 * NLSF_QUANT_MAX_AMPLITUDE {
            ix += ps_range_dec.ec_dec_icdf(&SILK_NLSF_EXT_ICDF, 8);
        }
        ps_dec.indices.nlsf_indices[i + 1] = (ix - NLSF_QUANT_MAX_AMPLITUDE) as i8;
    }

    /* Decode LSF interpolation factor */
    if ps_dec.nb_subfr == MAX_NB_SUBFR as i32 {
        ps_dec.indices.nlsf_interp_coef_q2 =
            ps_range_dec.ec_dec_icdf(&SILK_NLSF_INTERPOLATION_FACTOR_ICDF, 8) as i8;
    } else {
        ps_dec.indices.nlsf_interp_coef_q2 = 4;
    }

    if ps_dec.indices.signal_type as i32 == TYPE_VOICED {
        /*********************/
        /* Decode pitch lags */
        /*********************/
        /* Get lag index */
        let mut decode_absolute_lag_index = true;
        if cond_coding == CODE_CONDITIONALLY && ps_dec.ec_prev_signal_type == TYPE_VOICED {
            /* Decode Delta index */
            let mut delta_lag_index =
                ps_range_dec.ec_dec_icdf(&SILK_PITCH_DELTA_ICDF, 8) as i16 as i32;
            if delta_lag_index > 0 {
                delta_lag_index -= 9;
                ps_dec.indices.lag_index =
                    (ps_dec.ec_prev_lag_index as i32 + delta_lag_index) as i16;
                decode_absolute_lag_index = false;
            }
        }
        if decode_absolute_lag_index {
            /* Absolute decoding */
            ps_dec.indices.lag_index = (ps_range_dec.ec_dec_icdf(&SILK_PITCH_LAG_ICDF, 8) as i16
                as i32
                * silk_rshift(ps_dec.fs_khz, 1)) as i16;
            ps_dec.indices.lag_index = (ps_dec.indices.lag_index as i32
                + ps_range_dec.ec_dec_icdf(ps_dec.pitch_lag_low_bits_icdf, 8) as i16 as i32)
                as i16;
        }
        ps_dec.ec_prev_lag_index = ps_dec.indices.lag_index;

        /* Get countour index */
        ps_dec.indices.contour_index = ps_range_dec.ec_dec_icdf(ps_dec.pitch_contour_icdf, 8) as i8;

        /********************/
        /* Decode LTP gains */
        /********************/
        /* Decode PERIndex value */
        ps_dec.indices.per_index = ps_range_dec.ec_dec_icdf(&SILK_LTP_PER_INDEX_ICDF, 8) as i8;

        for k in 0..ps_dec.nb_subfr as usize {
            ps_dec.indices.ltp_index[k] = ps_range_dec.ec_dec_icdf(
                SILK_LTP_GAIN_ICDF_PTRS[ps_dec.indices.per_index as usize],
                8,
            ) as i8;
        }

        /**********************/
        /* Decode LTP scaling */
        /**********************/
        if cond_coding == CODE_INDEPENDENTLY {
            ps_dec.indices.ltp_scale_index = ps_range_dec.ec_dec_icdf(&SILK_LTPSCALE_ICDF, 8) as i8;
        } else {
            ps_dec.indices.ltp_scale_index = 0;
        }
    }
    ps_dec.ec_prev_signal_type = ps_dec.indices.signal_type as i32;

    /***************/
    /* Decode seed */
    /***************/
    ps_dec.indices.seed = ps_range_dec.ec_dec_icdf(&SILK_UNIFORM4_ICDF, 8) as i8;
}
