// Rust translation of silk/define.h, silk/errors.h, silk/control.h (the
// decoder's parts), silk/pitch_est_defines.h and silk/PLC.h's constants from
// libopus 1.4 (https://opus-codec.org, at the revision SDL_mixer's
// external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SILK's constants (the ones the decoder uses).

#![allow(dead_code)]

/* Number of decoder channels (1/2) */
pub(crate) const DECODER_NUM_CHANNELS: usize = 2;

pub(crate) const MAX_FRAMES_PER_PACKET: usize = 3;

/* Maximum sampling frequency */
pub(crate) const MAX_FS_KHZ: i32 = 16;
pub(crate) const MAX_API_FS_KHZ: i32 = 48;

/* Signal types */
pub(crate) const TYPE_NO_VOICE_ACTIVITY: i32 = 0;
pub(crate) const TYPE_UNVOICED: i32 = 1;
pub(crate) const TYPE_VOICED: i32 = 2;

/* Conditional coding types */
pub(crate) const CODE_INDEPENDENTLY: i32 = 0;
pub(crate) const CODE_INDEPENDENTLY_NO_LTP_SCALING: i32 = 1;
pub(crate) const CODE_CONDITIONALLY: i32 = 2;

/* Settings for stereo processing */
pub(crate) const STEREO_QUANT_TAB_SIZE: usize = 16;
pub(crate) const STEREO_QUANT_SUB_STEPS: i32 = 5;
pub(crate) const STEREO_INTERP_LEN_MS: i32 = 8; /* must be even */

/* Range of pitch lag estimates */
pub(crate) const PITCH_EST_MIN_LAG_MS: i32 = 2; /* 2 ms -> 500 Hz */
pub(crate) const PITCH_EST_MAX_LAG_MS: i32 = 18; /* 18 ms -> 56 Hz */

/* Maximum number of subframes */
pub(crate) const MAX_NB_SUBFR: usize = 4;

/* Number of samples per frame */
pub(crate) const LTP_MEM_LENGTH_MS: i32 = 20;
pub(crate) const SUB_FRAME_LENGTH_MS: i32 = 5;
pub(crate) const MAX_SUB_FRAME_LENGTH: usize = (SUB_FRAME_LENGTH_MS * MAX_FS_KHZ) as usize;
pub(crate) const MAX_FRAME_LENGTH_MS: i32 = SUB_FRAME_LENGTH_MS * MAX_NB_SUBFR as i32;
pub(crate) const MAX_FRAME_LENGTH: usize = (MAX_FRAME_LENGTH_MS * MAX_FS_KHZ) as usize;

/* dB level of lowest gain quantization level */
pub(crate) const MIN_QGAIN_DB: i32 = 2;
/* dB level of highest gain quantization level */
pub(crate) const MAX_QGAIN_DB: i32 = 88;
/* Number of gain quantization levels */
pub(crate) const N_LEVELS_QGAIN: i32 = 64;
/* Max increase in gain quantization index */
pub(crate) const MAX_DELTA_GAIN_QUANT: i32 = 36;
/* Max decrease in gain quantization index */
pub(crate) const MIN_DELTA_GAIN_QUANT: i32 = -4;

/* Quantization offsets (multiples of 4) */
pub(crate) const OFFSET_VL_Q10: i32 = 32;
pub(crate) const OFFSET_VH_Q10: i32 = 100;
pub(crate) const OFFSET_UVL_Q10: i32 = 100;
pub(crate) const OFFSET_UVH_Q10: i32 = 240;

pub(crate) const QUANT_LEVEL_ADJUST_Q10: i32 = 80;

/* Maximum numbers of iterations used to stabilize an LPC vector */
pub(crate) const MAX_LPC_STABILIZE_ITERATIONS: i32 = 16;
pub(crate) const MAX_PREDICTION_POWER_GAIN: f32 = 1e4;

pub(crate) const MAX_LPC_ORDER: usize = 16;
pub(crate) const MIN_LPC_ORDER: i32 = 10;

/* Find Pred Coef defines */
pub(crate) const LTP_ORDER: usize = 5;

/* LTP quantization settings */
pub(crate) const NB_LTP_CBKS: usize = 3;

/* Number of subframes for excitation entropy coding */
pub(crate) const SHELL_CODEC_FRAME_LENGTH: usize = 16;
pub(crate) const LOG2_SHELL_CODEC_FRAME_LENGTH: i32 = 4;
pub(crate) const MAX_NB_SHELL_BLOCKS: usize = MAX_FRAME_LENGTH / SHELL_CODEC_FRAME_LENGTH;

/* Number of rate levels, for entropy coding of excitation */
pub(crate) const N_RATE_LEVELS: usize = 10;

/* Maximum sum of pulses per shell coding frame */
pub(crate) const SILK_MAX_PULSES: i32 = 16;

/* Size of the piecewise linear cosine approximation table for the LSFs */
pub(crate) const LSF_COS_TAB_SZ_FIX: i32 = 128;

/******************/
/* NLSF quantizer */
/******************/
pub(crate) const NLSF_QUANT_MAX_AMPLITUDE: i32 = 4;
pub(crate) const NLSF_QUANT_LEVEL_ADJ: f64 = 0.1;

/* BWE factors to apply after packet loss */
pub(crate) const BWE_AFTER_LOSS_Q16: i32 = 63570;

/* Defines for CN generation */
pub(crate) const CNG_BUF_MASK_MAX: i32 = 255; /* 2^floor(log2(MAX_FRAME_LENGTH))-1    */
pub(crate) const CNG_GAIN_SMTH_Q16: i32 = 4634; /* 0.25^(1/4)                           */
pub(crate) const CNG_GAIN_SMTH_THRESHOLD_Q16: i32 = 46396; /* -3 dB                                */
pub(crate) const CNG_NLSF_SMTH_Q16: i32 = 16348; /* 0.25                                 */

/* errors.h */
pub(crate) const SILK_NO_ERROR: i32 = 0;
pub(crate) const SILK_DEC_INVALID_SAMPLING_FREQUENCY: i32 = -200;
pub(crate) const SILK_DEC_PAYLOAD_TOO_LARGE: i32 = -201;
pub(crate) const SILK_DEC_PAYLOAD_ERROR: i32 = -202;
pub(crate) const SILK_DEC_INVALID_FRAME_SIZE: i32 = -203;

/* control.h: Decoder API flags */
pub(crate) const FLAG_DECODE_NORMAL: i32 = 0;
pub(crate) const FLAG_PACKET_LOST: i32 = 1;
pub(crate) const FLAG_DECODE_LBRR: i32 = 2;

/* pitch_est_defines.h */
pub(crate) const PE_MAX_NB_SUBFR: i32 = 4;
pub(crate) const PE_MAX_LAG_MS: i32 = 18; /* 18 ms -> 56 Hz */
pub(crate) const PE_MIN_LAG_MS: i32 = 2; /* 2 ms -> 500 Hz */
pub(crate) const PE_NB_CBKS_STAGE2_EXT: usize = 11;
pub(crate) const PE_NB_CBKS_STAGE3_MAX: usize = 34;
pub(crate) const PE_NB_CBKS_STAGE3_10MS: usize = 12;
pub(crate) const PE_NB_CBKS_STAGE2_10MS: usize = 3;

/* PLC.h */
pub(crate) const BWE_COEF: f64 = 0.99;
pub(crate) const V_PITCH_GAIN_START_MIN_Q14: i32 = 11469; /* 0.7 in Q14               */
pub(crate) const V_PITCH_GAIN_START_MAX_Q14: i32 = 15565; /* 0.95 in Q14              */
pub(crate) const MAX_PITCH_LAG_MS: i32 = 18;
pub(crate) const RAND_BUF_SIZE: i32 = 128;
pub(crate) const RAND_BUF_MASK: i32 = RAND_BUF_SIZE - 1;
pub(crate) const LOG2_INV_LPC_GAIN_HIGH_THRES: i32 = 3; /* 2^3 = 8 dB LPC gain      */
pub(crate) const LOG2_INV_LPC_GAIN_LOW_THRES: i32 = 8; /* 2^8 = 24 dB LPC gain     */
pub(crate) const PITCH_DRIFT_FAC_Q16: i32 = 655; /* 0.01 in Q16              */
