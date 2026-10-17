// Rust translation of silk/dec_API.c, silk/init_decoder.c and
// silk/decoder_set_fs.c from libopus 1.4 (https://opus-codec.org, at the
// revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SILK decoder's API. `silk_Decode()` decodes into a temporary buffer
//! of its own (C reuses the output buffer for it when it's large enough,
//! `delay_stack_alloc`, which gives the same result).

use super::cng::silk_cng_reset;
use super::decode_frame::silk_decode_frame;
use super::decode_indices::silk_decode_indices;
use super::decode_pulses::silk_decode_pulses;
use super::define::*;
use super::plc::silk_plc_reset;
use super::resampler::{silk_resampler, silk_resampler_init};
use super::sigproc_fix::{silk_div32, silk_rshift, silk_smulbb};
use super::stereo::{silk_stereo_decode_mid_only, silk_stereo_decode_pred, silk_stereo_ms_to_lr};
use super::structs::{SilkDecControlStruct, SilkDecoderState, StereoDecState};
use super::tables::{
    SILK_LBRR_FLAGS_ICDF_PTR, SILK_NLSF_CB_NB_MB, SILK_NLSF_CB_WB, SILK_PITCH_CONTOUR_10_MS_ICDF,
    SILK_PITCH_CONTOUR_10_MS_NB_ICDF, SILK_PITCH_CONTOUR_ICDF, SILK_PITCH_CONTOUR_NB_ICDF,
    SILK_UNIFORM4_ICDF, SILK_UNIFORM6_ICDF, SILK_UNIFORM8_ICDF,
};
use crate::opus::celt::entcode::EcDec;

/************************/
/* Decoder Super Struct */
/************************/
/// Translation of `silk_decoder`.
#[derive(Clone, Default)]
pub(crate) struct SilkDecoder {
    pub channel_state: [Box<SilkDecoderState>; DECODER_NUM_CHANNELS],
    pub s_stereo: StereoDecState,
    pub n_channels_api: i32,
    pub n_channels_internal: i32,
    pub prev_decode_only_middle: i32,
}

/************************/
/* Init Decoder State   */
/************************/
/// Translation of `silk_init_decoder()`.
pub(crate) fn silk_init_decoder(ps_dec: &mut SilkDecoderState) -> i32 {
    /* Clear the entire encoder state, except anything copied */
    *ps_dec = SilkDecoderState::default();

    /* Used to deactivate LSF interpolation */
    ps_dec.first_frame_after_reset = 1;
    ps_dec.prev_gain_q16 = 65536;

    /* Reset CNG state */
    silk_cng_reset(ps_dec);

    /* Reset PLC state */
    silk_plc_reset(ps_dec);

    0
}

/* Set decoder sampling rate */
/// Translation of `silk_decoder_set_fs()`.
pub(crate) fn silk_decoder_set_fs(
    ps_dec: &mut SilkDecoderState,
    fs_khz: i32,
    fs_api_hz: i32,
) -> i32 {
    let mut ret = 0;

    debug_assert!(fs_khz == 8 || fs_khz == 12 || fs_khz == 16);
    debug_assert!(
        ps_dec.nb_subfr == MAX_NB_SUBFR as i32 || ps_dec.nb_subfr == MAX_NB_SUBFR as i32 / 2
    );

    /* New (sub)frame length */
    ps_dec.subfr_length = silk_smulbb(SUB_FRAME_LENGTH_MS, fs_khz);
    let frame_length = silk_smulbb(ps_dec.nb_subfr, ps_dec.subfr_length);

    /* Initialize resampler when switching internal or external sampling frequency */
    if ps_dec.fs_khz != fs_khz || ps_dec.fs_api_hz != fs_api_hz {
        /* Initialize the resampler for dec_API.c preparing resampling from fs_kHz to API_fs_Hz */
        ret += silk_resampler_init(
            &mut ps_dec.resampler_state,
            silk_smulbb(fs_khz, 1000),
            fs_api_hz,
            0,
        );

        ps_dec.fs_api_hz = fs_api_hz;
    }

    if ps_dec.fs_khz != fs_khz || frame_length != ps_dec.frame_length {
        if fs_khz == 8 {
            if ps_dec.nb_subfr == MAX_NB_SUBFR as i32 {
                ps_dec.pitch_contour_icdf = &SILK_PITCH_CONTOUR_NB_ICDF;
            } else {
                ps_dec.pitch_contour_icdf = &SILK_PITCH_CONTOUR_10_MS_NB_ICDF;
            }
        } else if ps_dec.nb_subfr == MAX_NB_SUBFR as i32 {
            ps_dec.pitch_contour_icdf = &SILK_PITCH_CONTOUR_ICDF;
        } else {
            ps_dec.pitch_contour_icdf = &SILK_PITCH_CONTOUR_10_MS_ICDF;
        }
        if ps_dec.fs_khz != fs_khz {
            ps_dec.ltp_mem_length = silk_smulbb(LTP_MEM_LENGTH_MS, fs_khz);
            if fs_khz == 8 || fs_khz == 12 {
                ps_dec.lpc_order = MIN_LPC_ORDER;
                ps_dec.ps_nlsf_cb = &SILK_NLSF_CB_NB_MB;
            } else {
                ps_dec.lpc_order = MAX_LPC_ORDER as i32;
                ps_dec.ps_nlsf_cb = &SILK_NLSF_CB_WB;
            }
            if fs_khz == 16 {
                ps_dec.pitch_lag_low_bits_icdf = &SILK_UNIFORM8_ICDF;
            } else if fs_khz == 12 {
                ps_dec.pitch_lag_low_bits_icdf = &SILK_UNIFORM6_ICDF;
            } else if fs_khz == 8 {
                ps_dec.pitch_lag_low_bits_icdf = &SILK_UNIFORM4_ICDF;
            } else {
                /* unsupported sampling rate */
                debug_assert!(false);
            }
            ps_dec.first_frame_after_reset = 1;
            ps_dec.lag_prev = 100;
            ps_dec.last_gain_index = 10;
            ps_dec.prev_signal_type = TYPE_NO_VOICE_ACTIVITY;
            ps_dec.out_buf.fill(0);
            ps_dec.s_lpc_q14_buf.fill(0);
        }

        ps_dec.fs_khz = fs_khz;
        ps_dec.frame_length = frame_length;
    }

    /* Check that settings are valid */
    debug_assert!(ps_dec.frame_length > 0 && ps_dec.frame_length <= MAX_FRAME_LENGTH as i32);

    ret
}

/*********************/
/* Decoder functions */
/*********************/

/* Reset decoder state */
/// Translation of `silk_InitDecoder()`.
pub(crate) fn silk_init_decoder_all(dec_state: &mut SilkDecoder) -> i32 {
    let mut ret = SILK_NO_ERROR;
    for n in 0..DECODER_NUM_CHANNELS {
        ret = silk_init_decoder(&mut dec_state.channel_state[n]);
    }
    dec_state.s_stereo = StereoDecState::default();
    /* Not strictly needed, but it's cleaner that way */
    dec_state.prev_decode_only_middle = 0;

    ret
}

/* Decode a frame */
/// Translation of `silk_Decode()`: decodes into `samples_out`, and returns
/// the error code and the number of samples decoded (`nSamplesOut`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn silk_decode(
    ps_dec: &mut SilkDecoder,
    dec_control: &mut SilkDecControlStruct,
    lost_flag: i32,
    new_packet_flag: i32,
    ps_range_dec: &mut EcDec<'_>,
    samples_out: &mut [i16],
    n_samples_out: &mut i32,
) -> i32 {
    let mut decode_only_middle = 0;
    let mut ret = SILK_NO_ERROR;
    let mut n_samples_out_dec: i32 = 0;
    let mut ms_pred_q13 = [0i32; 2];

    debug_assert!(dec_control.n_channels_internal == 1 || dec_control.n_channels_internal == 2);
    let n_ch_int = dec_control.n_channels_internal as usize;

    /**********************************/
    /* Test if first frame in payload */
    /**********************************/
    if new_packet_flag != 0 {
        for n in 0..n_ch_int {
            ps_dec.channel_state[n].n_frames_decoded = 0; /* Used to count frames in packet */
        }
    }

    /* If Mono -> Stereo transition in bitstream: init state of second channel */
    if dec_control.n_channels_internal > ps_dec.n_channels_internal {
        ret += silk_init_decoder(&mut ps_dec.channel_state[1]);
    }

    let stereo_to_mono = dec_control.n_channels_internal == 1
        && ps_dec.n_channels_internal == 2
        && (dec_control.internal_sample_rate == 1000 * ps_dec.channel_state[0].fs_khz);

    if ps_dec.channel_state[0].n_frames_decoded == 0 {
        for n in 0..n_ch_int {
            let cs = &mut ps_dec.channel_state[n];
            if dec_control.payload_size_ms == 0 {
                /* Assuming packet loss, use 10 ms */
                cs.n_frames_per_packet = 1;
                cs.nb_subfr = 2;
            } else if dec_control.payload_size_ms == 10 {
                cs.n_frames_per_packet = 1;
                cs.nb_subfr = 2;
            } else if dec_control.payload_size_ms == 20 {
                cs.n_frames_per_packet = 1;
                cs.nb_subfr = 4;
            } else if dec_control.payload_size_ms == 40 {
                cs.n_frames_per_packet = 2;
                cs.nb_subfr = 4;
            } else if dec_control.payload_size_ms == 60 {
                cs.n_frames_per_packet = 3;
                cs.nb_subfr = 4;
            } else {
                debug_assert!(false);
                return SILK_DEC_INVALID_FRAME_SIZE;
            }
            let fs_khz_dec = (dec_control.internal_sample_rate >> 10) + 1;
            if fs_khz_dec != 8 && fs_khz_dec != 12 && fs_khz_dec != 16 {
                debug_assert!(false);
                return SILK_DEC_INVALID_SAMPLING_FREQUENCY;
            }
            ret += silk_decoder_set_fs(cs, fs_khz_dec, dec_control.api_sample_rate);
        }
    }

    if dec_control.n_channels_api == 2
        && dec_control.n_channels_internal == 2
        && (ps_dec.n_channels_api == 1 || ps_dec.n_channels_internal == 1)
    {
        ps_dec.s_stereo.pred_prev_q13 = [0; 2];
        ps_dec.s_stereo.s_side = [0; 2];
        let rs = ps_dec.channel_state[0].resampler_state;
        ps_dec.channel_state[1].resampler_state = rs;
    }
    ps_dec.n_channels_api = dec_control.n_channels_api;
    ps_dec.n_channels_internal = dec_control.n_channels_internal;

    if dec_control.api_sample_rate > MAX_API_FS_KHZ * 1000 || dec_control.api_sample_rate < 8000 {
        ret = SILK_DEC_INVALID_SAMPLING_FREQUENCY;
        return ret;
    }

    if lost_flag != FLAG_PACKET_LOST && ps_dec.channel_state[0].n_frames_decoded == 0 {
        /* First decoder call for this payload */
        /* Decode VAD flags and LBRR flag */
        for n in 0..n_ch_int {
            let cs = &mut ps_dec.channel_state[n];
            for i in 0..cs.n_frames_per_packet as usize {
                cs.vad_flags[i] = ps_range_dec.ec_dec_bit_logp(1);
            }
            cs.lbrr_flag = ps_range_dec.ec_dec_bit_logp(1);
        }
        /* Decode LBRR flags */
        for n in 0..n_ch_int {
            let cs = &mut ps_dec.channel_state[n];
            cs.lbrr_flags = [0; MAX_FRAMES_PER_PACKET];
            if cs.lbrr_flag != 0 {
                if cs.n_frames_per_packet == 1 {
                    cs.lbrr_flags[0] = 1;
                } else {
                    let lbrr_symbol = ps_range_dec.ec_dec_icdf(
                        SILK_LBRR_FLAGS_ICDF_PTR[(cs.n_frames_per_packet - 2) as usize],
                        8,
                    ) + 1;
                    for i in 0..cs.n_frames_per_packet as usize {
                        cs.lbrr_flags[i] = silk_rshift(lbrr_symbol, i as i32) & 1;
                    }
                }
            }
        }

        if lost_flag == FLAG_DECODE_NORMAL {
            /* Regular decoding: skip all LBRR data */
            for i in 0..ps_dec.channel_state[0].n_frames_per_packet as usize {
                for n in 0..n_ch_int {
                    if ps_dec.channel_state[n].lbrr_flags[i] != 0 {
                        let mut pulses = [0i16; MAX_FRAME_LENGTH];

                        if dec_control.n_channels_internal == 2 && n == 0 {
                            silk_stereo_decode_pred(ps_range_dec, &mut ms_pred_q13);
                            if ps_dec.channel_state[1].lbrr_flags[i] == 0 {
                                silk_stereo_decode_mid_only(ps_range_dec, &mut decode_only_middle);
                            }
                        }
                        /* Use conditional coding if previous frame available */
                        let cond_coding = if i > 0 && ps_dec.channel_state[n].lbrr_flags[i - 1] != 0
                        {
                            CODE_CONDITIONALLY
                        } else {
                            CODE_INDEPENDENTLY
                        };
                        let cs = &mut ps_dec.channel_state[n];
                        silk_decode_indices(cs, ps_range_dec, i as i32, 1, cond_coding);
                        silk_decode_pulses(
                            ps_range_dec,
                            &mut pulses,
                            cs.indices.signal_type as i32,
                            cs.indices.quant_offset_type as i32,
                            cs.frame_length,
                        );
                    }
                }
            }
        }
    }

    /* Get MS predictor index */
    if dec_control.n_channels_internal == 2 {
        let fd = ps_dec.channel_state[0].n_frames_decoded as usize;
        if lost_flag == FLAG_DECODE_NORMAL
            || (lost_flag == FLAG_DECODE_LBRR && ps_dec.channel_state[0].lbrr_flags[fd] == 1)
        {
            silk_stereo_decode_pred(ps_range_dec, &mut ms_pred_q13);
            /* For LBRR data, decode mid-only flag only if side-channel's LBRR flag is false */
            if (lost_flag == FLAG_DECODE_NORMAL && ps_dec.channel_state[1].vad_flags[fd] == 0)
                || (lost_flag == FLAG_DECODE_LBRR && ps_dec.channel_state[1].lbrr_flags[fd] == 0)
            {
                silk_stereo_decode_mid_only(ps_range_dec, &mut decode_only_middle);
            } else {
                decode_only_middle = 0;
            }
        } else {
            for n in 0..2 {
                ms_pred_q13[n] = ps_dec.s_stereo.pred_prev_q13[n] as i32;
            }
        }
    }

    /* Reset side channel decoder prediction memory for first frame with side coding */
    if dec_control.n_channels_internal == 2
        && decode_only_middle == 0
        && ps_dec.prev_decode_only_middle == 1
    {
        let cs = &mut ps_dec.channel_state[1];
        cs.out_buf.fill(0);
        cs.s_lpc_q14_buf.fill(0);
        cs.lag_prev = 100;
        cs.last_gain_index = 10;
        cs.prev_signal_type = TYPE_NO_VOICE_ACTIVITY;
        cs.first_frame_after_reset = 1;
    }

    /* Check if the temp buffer fits into the output PCM buffer. If it fits,
    we can delay allocating the temp buffer until after the SILK peak stack
    usage. We need to use a < and not a <= because of the two extra samples. */
    let fl2 = (ps_dec.channel_state[0].frame_length + 2) as usize;
    let mut samples_out1_tmp_storage = vec![0i16; n_ch_int * fl2];

    let has_side = if lost_flag == FLAG_DECODE_NORMAL {
        (decode_only_middle == 0) as i32
    } else {
        (ps_dec.prev_decode_only_middle == 0
            || (dec_control.n_channels_internal == 2
                && lost_flag == FLAG_DECODE_LBRR
                && ps_dec.channel_state[1].lbrr_flags
                    [ps_dec.channel_state[1].n_frames_decoded as usize]
                    == 1)) as i32
    };
    /* Call decoder for one frame */
    for n in 0..n_ch_int {
        let tmp = &mut samples_out1_tmp_storage[n * fl2..(n + 1) * fl2];
        if n == 0 || has_side != 0 {
            let frame_index = ps_dec.channel_state[0].n_frames_decoded - n as i32;
            /* Use independent coding if no previous frame available */
            let cond_coding = if frame_index <= 0 {
                CODE_INDEPENDENTLY
            } else if lost_flag == FLAG_DECODE_LBRR {
                if ps_dec.channel_state[n].lbrr_flags[(frame_index - 1) as usize] != 0 {
                    CODE_CONDITIONALLY
                } else {
                    CODE_INDEPENDENTLY
                }
            } else if n > 0 && ps_dec.prev_decode_only_middle != 0 {
                /* If we skipped a side frame in this packet, we don't
                need LTP scaling; the LTP state is well-defined. */
                CODE_INDEPENDENTLY_NO_LTP_SCALING
            } else {
                CODE_CONDITIONALLY
            };
            ret += silk_decode_frame(
                &mut ps_dec.channel_state[n],
                ps_range_dec,
                &mut tmp[2..],
                &mut n_samples_out_dec,
                lost_flag,
                cond_coding,
            );
        } else {
            tmp[2..2 + n_samples_out_dec as usize].fill(0);
        }
        ps_dec.channel_state[n].n_frames_decoded += 1;
    }

    {
        let (t0, t1) = samples_out1_tmp_storage.split_at_mut(fl2);
        if dec_control.n_channels_api == 2 && dec_control.n_channels_internal == 2 {
            /* Convert Mid/Side to Left/Right */
            silk_stereo_ms_to_lr(
                &mut ps_dec.s_stereo,
                t0,
                t1,
                &ms_pred_q13,
                ps_dec.channel_state[0].fs_khz,
                n_samples_out_dec,
            );
        } else {
            /* Buffering */
            t0[..2].copy_from_slice(&ps_dec.s_stereo.s_mid);
            let nso = n_samples_out_dec as usize;
            ps_dec.s_stereo.s_mid.copy_from_slice(&t0[nso..nso + 2]);
        }
    }

    /* Number of output samples */
    *n_samples_out = silk_div32(
        n_samples_out_dec * dec_control.api_sample_rate,
        silk_smulbb(ps_dec.channel_state[0].fs_khz, 1000),
    );

    /* Set up pointers to temp buffers */
    let mut samples_out2_tmp = vec![
        0i16;
        if dec_control.n_channels_api == 2 {
            *n_samples_out as usize
        } else {
            0
        }
    ];

    for n in 0..(dec_control
        .n_channels_api
        .min(dec_control.n_channels_internal)) as usize
    {
        /* Resample decoded signal to API_sampleRate */
        let input = &samples_out1_tmp_storage[n * fl2 + 1..];
        if dec_control.n_channels_api == 2 {
            ret += silk_resampler(
                &mut ps_dec.channel_state[n].resampler_state,
                &mut samples_out2_tmp,
                input,
                n_samples_out_dec,
            );

            /* Interleave if stereo output and stereo stream */
            for i in 0..*n_samples_out as usize {
                samples_out[n + 2 * i] = samples_out2_tmp[i];
            }
        } else {
            ret += silk_resampler(
                &mut ps_dec.channel_state[n].resampler_state,
                samples_out,
                input,
                n_samples_out_dec,
            );
        }
    }

    /* Create two channel output from mono stream */
    if dec_control.n_channels_api == 2 && dec_control.n_channels_internal == 1 {
        if stereo_to_mono {
            /* Resample right channel for newly collapsed stereo just in case
            we weren't doing collapsing when switching to mono */
            ret += silk_resampler(
                &mut ps_dec.channel_state[1].resampler_state,
                &mut samples_out2_tmp,
                &samples_out1_tmp_storage[1..],
                n_samples_out_dec,
            );

            for i in 0..*n_samples_out as usize {
                samples_out[1 + 2 * i] = samples_out2_tmp[i];
            }
        } else {
            for i in 0..*n_samples_out as usize {
                samples_out[1 + 2 * i] = samples_out[2 * i];
            }
        }
    }

    /* Export pitch lag, measured at 48 kHz sampling rate */
    if ps_dec.channel_state[0].prev_signal_type == TYPE_VOICED {
        let mult_tab: [i32; 3] = [6, 4, 3];
        dec_control.prev_pitch_lag = ps_dec.channel_state[0].lag_prev
            * mult_tab[((ps_dec.channel_state[0].fs_khz - 8) >> 2) as usize];
    } else {
        dec_control.prev_pitch_lag = 0;
    }

    if lost_flag == FLAG_PACKET_LOST {
        /* On packet loss, remove the gain clamping to prevent having the energy "bounce back"
        if we lose packets when the energy is going down */
        for i in 0..ps_dec.n_channels_internal as usize {
            ps_dec.channel_state[i].last_gain_index = 10;
        }
    } else {
        ps_dec.prev_decode_only_middle = decode_only_middle;
    }
    ret
}
