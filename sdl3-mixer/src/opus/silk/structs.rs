// Rust translation of silk/structs.h (the decoder's structures),
// silk/resampler_structs.h and silk/control.h's silk_DecControlStruct from
// libopus 1.4 (https://opus-codec.org, at the revision SDL_mixer's
// external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SILK decoder's state. `Default` is C's `memset(.., 0, ..)`; the
//! table pointers default to empty slices and the codebook to the narrowband
//! one until `silk_decoder_set_fs()` sets them.

use super::define::{
    LTP_ORDER, MAX_FRAMES_PER_PACKET, MAX_FRAME_LENGTH, MAX_LPC_ORDER, MAX_NB_SUBFR,
    MAX_SUB_FRAME_LENGTH,
};
use super::tables::{SilkNlsfCbStruct, SILK_NLSF_CB_NB_MB};

pub(crate) const SILK_RESAMPLER_MAX_FIR_ORDER: usize = 36;
pub(crate) const SILK_RESAMPLER_MAX_IIR_ORDER: usize = 6;

/// Translation of `silk_resampler_state_struct`. The `sFIR` union is used
/// as `opus_int16` only (by the upsampler).
#[derive(Clone, Copy)]
pub(crate) struct SilkResamplerStateStruct {
    pub s_iir: [i32; SILK_RESAMPLER_MAX_IIR_ORDER], /* this must be the first element of this struct */
    pub s_fir_i16: [i16; SILK_RESAMPLER_MAX_FIR_ORDER],
    pub delay_buf: [i16; 48],
    pub resampler_function: i32,
    pub batch_size: i32,
    pub inv_ratio_q16: i32,
    #[allow(dead_code)] /* (the downsampler's, which isn't translated) */ pub fir_order: i32,
    #[allow(dead_code)]
    pub fir_fracs: i32,
    pub fs_in_khz: i32,
    pub fs_out_khz: i32,
    pub input_delay: i32,
}

impl Default for SilkResamplerStateStruct {
    fn default() -> Self {
        SilkResamplerStateStruct {
            s_iir: [0; SILK_RESAMPLER_MAX_IIR_ORDER],
            s_fir_i16: [0; SILK_RESAMPLER_MAX_FIR_ORDER],
            delay_buf: [0; 48],
            resampler_function: 0,
            batch_size: 0,
            inv_ratio_q16: 0,
            fir_order: 0,
            fir_fracs: 0,
            fs_in_khz: 0,
            fs_out_khz: 0,
            input_delay: 0,
        }
    }
}

/// Translation of `stereo_dec_state`.
#[derive(Clone, Copy, Default)]
pub(crate) struct StereoDecState {
    pub pred_prev_q13: [i16; 2],
    pub s_mid: [i16; 2],
    pub s_side: [i16; 2],
}

/// Translation of `SideInfoIndices`.
#[derive(Clone, Copy, Default)]
pub(crate) struct SideInfoIndices {
    pub gains_indices: [i8; MAX_NB_SUBFR],
    pub ltp_index: [i8; MAX_NB_SUBFR],
    pub nlsf_indices: [i8; MAX_LPC_ORDER + 1],
    pub lag_index: i16,
    pub contour_index: i8,
    pub signal_type: i8,
    pub quant_offset_type: i8,
    pub nlsf_interp_coef_q2: i8,
    pub per_index: i8,
    pub ltp_scale_index: i8,
    pub seed: i8,
}

/* Struct for Packet Loss Concealment */
/// Translation of `silk_PLC_struct`.
#[derive(Clone, Copy, Default)]
pub(crate) struct SilkPlcStruct {
    pub pitch_l_q8: i32, /* Pitch lag to use for voiced concealment                          */
    pub ltp_coef_q14: [i16; LTP_ORDER], /* LTP coeficients to use for voiced concealment                    */
    pub prev_lpc_q12: [i16; MAX_LPC_ORDER],
    pub last_frame_lost: i32, /* Was previous frame lost                                          */
    pub rand_seed: i32,       /* Seed for unvoiced signal generation                              */
    pub rand_scale_q14: i16,  /* Scaling of unvoiced random signal                                */
    pub conc_energy: i32,
    pub conc_energy_shift: i32,
    pub prev_ltp_scale_q14: i16,
    pub prev_gain_q16: [i32; 2],
    pub fs_khz: i32,
    pub nb_subfr: i32,
    pub subfr_length: i32,
}

/* Struct for CNG */
/// Translation of `silk_CNG_struct`.
#[derive(Clone, Copy)]
pub(crate) struct SilkCngStruct {
    pub cng_exc_buf_q14: [i32; MAX_FRAME_LENGTH],
    pub cng_smth_nlsf_q15: [i16; MAX_LPC_ORDER],
    pub cng_synth_state: [i32; MAX_LPC_ORDER],
    pub cng_smth_gain_q16: i32,
    pub rand_seed: i32,
    pub fs_khz: i32,
}

impl Default for SilkCngStruct {
    fn default() -> Self {
        SilkCngStruct {
            cng_exc_buf_q14: [0; MAX_FRAME_LENGTH],
            cng_smth_nlsf_q15: [0; MAX_LPC_ORDER],
            cng_synth_state: [0; MAX_LPC_ORDER],
            cng_smth_gain_q16: 0,
            rand_seed: 0,
            fs_khz: 0,
        }
    }
}

/********************************/
/* Decoder state                */
/********************************/
/// Translation of `silk_decoder_state`.
#[derive(Clone)]
pub(crate) struct SilkDecoderState {
    pub prev_gain_q16: i32,
    pub exc_q14: [i32; MAX_FRAME_LENGTH],
    pub s_lpc_q14_buf: [i32; MAX_LPC_ORDER],
    pub out_buf: [i16; MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH], /* Buffer for output signal                     */
    pub lag_prev: i32, /* Previous Lag                                                     */
    pub last_gain_index: i8, /* Previous gain index                                              */
    pub fs_khz: i32,   /* Sampling frequency in kHz                                        */
    pub fs_api_hz: i32, /* API sample frequency (Hz)                                        */
    pub nb_subfr: i32, /* Number of 5 ms subframes in a frame                              */
    pub frame_length: i32, /* Frame length (samples)                                           */
    pub subfr_length: i32, /* Subframe length (samples)                                        */
    pub ltp_mem_length: i32, /* Length of LTP memory                                             */
    pub lpc_order: i32, /* LPC order                                                        */
    pub prev_nlsf_q15: [i16; MAX_LPC_ORDER], /* Used to interpolate LSFs                                         */
    pub first_frame_after_reset: i32, /* Flag for deactivating NLSF interpolation                         */
    pub pitch_lag_low_bits_icdf: &'static [u8], /* Pointer to iCDF table for low bits of pitch lag index            */
    pub pitch_contour_icdf: &'static [u8], /* Pointer to iCDF table for pitch contour index                    */

    /* For buffering payload in case of more frames per packet */
    pub n_frames_decoded: i32,
    pub n_frames_per_packet: i32,

    /* Specifically for entropy coding */
    pub ec_prev_signal_type: i32,
    pub ec_prev_lag_index: i16,

    pub vad_flags: [i32; MAX_FRAMES_PER_PACKET],
    pub lbrr_flag: i32,
    pub lbrr_flags: [i32; MAX_FRAMES_PER_PACKET],

    pub resampler_state: SilkResamplerStateStruct,

    pub ps_nlsf_cb: &'static SilkNlsfCbStruct, /* Pointer to NLSF codebook                                         */

    /* Quantization indices */
    pub indices: SideInfoIndices,

    /* CNG state */
    pub s_cng: SilkCngStruct,

    /* Stuff used for PLC */
    pub loss_cnt: i32,
    pub prev_signal_type: i32,

    pub s_plc: SilkPlcStruct,
}

impl Default for SilkDecoderState {
    fn default() -> Self {
        SilkDecoderState {
            prev_gain_q16: 0,
            exc_q14: [0; MAX_FRAME_LENGTH],
            s_lpc_q14_buf: [0; MAX_LPC_ORDER],
            out_buf: [0; MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH],
            lag_prev: 0,
            last_gain_index: 0,
            fs_khz: 0,
            fs_api_hz: 0,
            nb_subfr: 0,
            frame_length: 0,
            subfr_length: 0,
            ltp_mem_length: 0,
            lpc_order: 0,
            prev_nlsf_q15: [0; MAX_LPC_ORDER],
            first_frame_after_reset: 0,
            pitch_lag_low_bits_icdf: &[],
            pitch_contour_icdf: &[],
            n_frames_decoded: 0,
            n_frames_per_packet: 0,
            ec_prev_signal_type: 0,
            ec_prev_lag_index: 0,
            vad_flags: [0; MAX_FRAMES_PER_PACKET],
            lbrr_flag: 0,
            lbrr_flags: [0; MAX_FRAMES_PER_PACKET],
            resampler_state: SilkResamplerStateStruct::default(),
            ps_nlsf_cb: &SILK_NLSF_CB_NB_MB,
            indices: SideInfoIndices::default(),
            s_cng: SilkCngStruct::default(),
            loss_cnt: 0,
            prev_signal_type: 0,
            s_plc: SilkPlcStruct::default(),
        }
    }
}

/************************/
/* Decoder control      */
/************************/
/// Translation of `silk_decoder_control`.
#[derive(Clone, Copy, Default)]
pub(crate) struct SilkDecoderControl {
    /* Prediction and coding parameters */
    pub pitch_l: [i32; MAX_NB_SUBFR],
    pub gains_q16: [i32; MAX_NB_SUBFR],
    /* Holds interpolated and final coefficients, 4-byte aligned */
    pub pred_coef_q12: [[i16; MAX_LPC_ORDER]; 2],
    pub ltp_coef_q14: [i16; LTP_ORDER * MAX_NB_SUBFR],
    pub ltp_scale_q14: i32,
}

/* Structure for controlling decoder operation and reading decoder status */
/// Translation of `silk_DecControlStruct`.
#[derive(Clone, Copy, Default)]
pub(crate) struct SilkDecControlStruct {
    /* I:   Number of channels; 1/2                                                         */
    pub n_channels_api: i32,

    /* I:   Number of channels; 1/2                                                         */
    pub n_channels_internal: i32,

    /* I:   Output signal sampling rate in Hertz; 8000/12000/16000/24000/32000/44100/48000  */
    pub api_sample_rate: i32,

    /* I:   Internal sampling rate used, in Hertz; 8000/12000/16000                         */
    pub internal_sample_rate: i32,

    /* I:   Number of samples per packet in milliseconds; 10/20/40/60                       */
    pub payload_size_ms: i32,

    /* O:   Pitch lag of previous frame (0 if unvoiced), measured in samples at 48 kHz      */
    pub prev_pitch_lag: i32,
}
