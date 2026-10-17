// Rust translation of silk/resampler.c, silk/resampler_private_IIR_FIR.c
// and silk/resampler_private_up2_HQ.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The SILK decoder's resampler. Only the decoder's direction is
//! translated: `silk_resampler_init()` with `forEnc` = 0, from 8, 12 or
//! 16 kHz. The downsampler (`silk_resampler_private_down_FIR()` and
//! `silk_resampler_private_AR2()`) is not translated: SDL_mixer's Opus
//! decoder always decodes at 48 kHz (opusfile's only rate), so the SILK
//! output is only ever copied or upsampled. A downsampling ratio fails to
//! initialize with -1, as an unsupported ratio does in C.

use super::sigproc_fix::{
    silk_div32, silk_div32_16, silk_lshift, silk_mul, silk_rshift_round, silk_sat16, silk_smlabb,
    silk_smlawb, silk_smulbb, silk_smulwb, silk_smulww,
};
use super::structs::SilkResamplerStateStruct;
use super::tables::{SILK_RESAMPLER_FRAC_FIR_12, SILK_RESAMPLER_UP2_HQ_0, SILK_RESAMPLER_UP2_HQ_1};

/* Number of input samples to process in the inner loop */
const RESAMPLER_MAX_BATCH_SIZE_MS: i32 = 10;
const RESAMPLER_MAX_FS_KHZ: i32 = 48;
#[allow(dead_code)]
const RESAMPLER_MAX_BATCH_SIZE_IN: i32 = RESAMPLER_MAX_BATCH_SIZE_MS * RESAMPLER_MAX_FS_KHZ;
const RESAMPLER_ORDER_FIR_12: usize = 8;

/*
 * Matrix of resampling methods used:
 *                                 Fs_out (kHz)
 *                        8      12     16     24     48
 *
 *               8        C      UF     U      UF     UF
 *              12        AF     C      UF     U      UF
 * Fs_in (kHz)  16        D      AF     C      UF     UF
 *              24        AF     D      AF     C      U
 *              48        AF     AF     AF     D      C
 *
 * C   -> Copy (no resampling)
 * D   -> Allpass-based 2x downsampling
 * U   -> Allpass-based 2x upsampling
 * UF  -> Allpass-based 2x upsampling followed by FIR interpolation
 * AF  -> AR2 filter followed by FIR interpolation
 */

/* Tables with delay compensation values to equalize total delay for different modes */
const DELAY_MATRIX_DEC: [[i8; 5]; 3] = [
    /* in  \ out  8  12  16  24  48 */
    /*  8 */ [4, 0, 2, 0, 0],
    /* 12 */ [0, 9, 4, 7, 4],
    /* 16 */ [0, 3, 12, 7, 7],
];

/* Simple way to make [8000, 12000, 16000, 24000, 48000] to [0, 1, 2, 3, 4] */
fn rate_id(r: i32) -> usize {
    ((((r >> 12) - (r > 16000) as i32) >> (r > 24000) as i32) - 1) as usize
}

const USE_SILK_RESAMPLER_COPY: i32 = 0;
const USE_SILK_RESAMPLER_PRIVATE_UP2_HQ_WRAPPER: i32 = 1;
const USE_SILK_RESAMPLER_PRIVATE_IIR_FIR: i32 = 2;
#[allow(dead_code)]
const USE_SILK_RESAMPLER_PRIVATE_DOWN_FIR: i32 = 3;

/* Initialize/reset the resampler state for a given pair of input/output sampling rates */
/// Translation of `silk_resampler_init()` for the decoder (`forEnc` = 0).
pub(crate) fn silk_resampler_init(
    s: &mut SilkResamplerStateStruct, /* I/O  Resampler state                                             */
    fs_hz_in: i32, /* I    Input sampling rate (Hz)                                    */
    fs_hz_out: i32, /* I    Output sampling rate (Hz)                                   */
    for_enc: i32,  /* I    If 1: encoder; if 0: decoder                                */
) -> i32 {
    /* Clear state */
    *s = SilkResamplerStateStruct::default();

    /* Input checking */
    debug_assert!(for_enc == 0, "the encoder's resampler is not translated");
    if (fs_hz_in != 8000 && fs_hz_in != 12000 && fs_hz_in != 16000)
        || (fs_hz_out != 8000
            && fs_hz_out != 12000
            && fs_hz_out != 16000
            && fs_hz_out != 24000
            && fs_hz_out != 48000)
    {
        debug_assert!(false);
        return -1;
    }
    s.input_delay = DELAY_MATRIX_DEC[rate_id(fs_hz_in)][rate_id(fs_hz_out)] as i32;

    s.fs_in_khz = silk_div32_16(fs_hz_in, 1000);
    s.fs_out_khz = silk_div32_16(fs_hz_out, 1000);

    /* Number of samples processed per batch */
    s.batch_size = s.fs_in_khz * RESAMPLER_MAX_BATCH_SIZE_MS;

    /* Find resampler with the right sampling ratio */
    let mut up2x = 0;
    if fs_hz_out > fs_hz_in {
        /* Upsample */
        if fs_hz_out == silk_mul(fs_hz_in, 2) {
            /* Fs_out : Fs_in = 2 : 1 */
            /* Special case: directly use 2x upsampler */
            s.resampler_function = USE_SILK_RESAMPLER_PRIVATE_UP2_HQ_WRAPPER;
        } else {
            /* Default resampler */
            s.resampler_function = USE_SILK_RESAMPLER_PRIVATE_IIR_FIR;
            up2x = 1;
        }
    } else if fs_hz_out < fs_hz_in {
        /* Downsample */
        /* (not translated, see the module documentation) */
        return -1;
    } else {
        /* Input and output sampling rates are equal: copy */
        s.resampler_function = USE_SILK_RESAMPLER_COPY;
    }

    /* Ratio of input/output samples */
    s.inv_ratio_q16 = silk_lshift(silk_div32(silk_lshift(fs_hz_in, 14 + up2x), fs_hz_out), 2);
    /* Make sure the ratio is rounded up */
    while silk_smulww(s.inv_ratio_q16, fs_hz_out) < silk_lshift(fs_hz_in, up2x) {
        s.inv_ratio_q16 += 1;
    }

    0
}

/* Resampler: convert from one sampling rate to another */
/* Input and output sampling rate are at most 48000 Hz  */
/// Translation of `silk_resampler()`.
pub(crate) fn silk_resampler(
    s: &mut SilkResamplerStateStruct, /* I/O  Resampler state                                             */
    out: &mut [i16], /* O    Output signal                                               */
    in_: &[i16],     /* I    Input signal                                                */
    in_len: i32,     /* I    Number of input samples                                     */
) -> i32 {
    /* Need at least 1 ms of input data */
    debug_assert!(in_len >= s.fs_in_khz);
    /* Delay can't exceed the 1 ms of buffering */
    debug_assert!(s.input_delay <= s.fs_in_khz);

    let n_samples = (s.fs_in_khz - s.input_delay) as usize;
    let input_delay = s.input_delay as usize;
    let fs_in = s.fs_in_khz as usize;
    let fs_out = s.fs_out_khz as usize;

    /* Copy to delay buffer */
    s.delay_buf[input_delay..input_delay + n_samples].copy_from_slice(&in_[..n_samples]);

    let delay_buf = s.delay_buf;
    match s.resampler_function {
        USE_SILK_RESAMPLER_PRIVATE_UP2_HQ_WRAPPER => {
            silk_resampler_private_up2_hq_wrapper(s, out, &delay_buf, s.fs_in_khz);
            silk_resampler_private_up2_hq_wrapper(
                s,
                &mut out[fs_out..],
                &in_[n_samples..],
                in_len - s.fs_in_khz,
            );
        }
        USE_SILK_RESAMPLER_PRIVATE_IIR_FIR => {
            silk_resampler_private_iir_fir(s, out, &delay_buf, s.fs_in_khz);
            silk_resampler_private_iir_fir(
                s,
                &mut out[fs_out..],
                &in_[n_samples..],
                in_len - s.fs_in_khz,
            );
        }
        _ => {
            out[..fs_in].copy_from_slice(&delay_buf[..fs_in]);
            let rest = (in_len - s.fs_in_khz) as usize;
            out[fs_out..fs_out + rest].copy_from_slice(&in_[n_samples..n_samples + rest]);
        }
    }

    /* Copy to delay buffer */
    let il = in_len as usize;
    s.delay_buf[..input_delay].copy_from_slice(&in_[il - input_delay..il]);

    0
}

#[inline(always)]
fn silk_resampler_private_iir_fir_interpol(
    out: &mut [i16],
    buf: &[i16],
    max_index_q16: i32,
    index_increment_q16: i32,
) -> usize {
    let mut o = 0usize;
    /* Interpolate upsampled signal and store in output array */
    let mut index_q16 = 0i32;
    while index_q16 < max_index_q16 {
        let table_index = silk_smulwb(index_q16 & 0xFFFF, 12) as usize;
        let buf_ptr = &buf[(index_q16 >> 16) as usize..];

        let fir = &SILK_RESAMPLER_FRAC_FIR_12;
        let mut res_q15 = silk_smulbb(buf_ptr[0] as i32, fir[table_index][0] as i32);
        res_q15 = silk_smlabb(res_q15, buf_ptr[1] as i32, fir[table_index][1] as i32);
        res_q15 = silk_smlabb(res_q15, buf_ptr[2] as i32, fir[table_index][2] as i32);
        res_q15 = silk_smlabb(res_q15, buf_ptr[3] as i32, fir[table_index][3] as i32);
        res_q15 = silk_smlabb(res_q15, buf_ptr[4] as i32, fir[11 - table_index][3] as i32);
        res_q15 = silk_smlabb(res_q15, buf_ptr[5] as i32, fir[11 - table_index][2] as i32);
        res_q15 = silk_smlabb(res_q15, buf_ptr[6] as i32, fir[11 - table_index][1] as i32);
        res_q15 = silk_smlabb(res_q15, buf_ptr[7] as i32, fir[11 - table_index][0] as i32);
        out[o] = silk_sat16(silk_rshift_round(res_q15, 15)) as i16;
        o += 1;
        index_q16 += index_increment_q16;
    }
    o
}

/* Upsample using a combination of allpass-based 2x upsampling and FIR interpolation */
/// Translation of `silk_resampler_private_IIR_FIR()`.
fn silk_resampler_private_iir_fir(
    s: &mut SilkResamplerStateStruct, /* I/O  Resampler state             */
    out: &mut [i16],                  /* O    Output signal               */
    in_: &[i16],                      /* I    Input signal                */
    mut in_len: i32,                  /* I    Number of input samples     */
) {
    let mut buf = [0i16; 2 * 16 * RESAMPLER_MAX_BATCH_SIZE_MS as usize + RESAMPLER_ORDER_FIR_12];
    let mut n_samples_in;
    let mut out_off = 0usize;
    let mut in_off = 0usize;

    /* Copy buffered samples to start of buffer */
    buf[..RESAMPLER_ORDER_FIR_12].copy_from_slice(&s.s_fir_i16[..RESAMPLER_ORDER_FIR_12]);

    /* Iterate over blocks of frameSizeIn input samples */
    let index_increment_q16 = s.inv_ratio_q16;
    loop {
        n_samples_in = in_len.min(s.batch_size);

        /* Upsample 2x */
        silk_resampler_private_up2_hq(
            &mut s.s_iir,
            &mut buf[RESAMPLER_ORDER_FIR_12..],
            &in_[in_off..],
            n_samples_in,
        );

        let max_index_q16 = silk_lshift(n_samples_in, 16 + 1); /* + 1 because 2x upsampling */
        out_off += silk_resampler_private_iir_fir_interpol(
            &mut out[out_off..],
            &buf,
            max_index_q16,
            index_increment_q16,
        );
        in_off += n_samples_in as usize;
        in_len -= n_samples_in;

        if in_len > 0 {
            /* More iterations to do; copy last part of filtered signal to beginning of buffer */
            let n2 = (n_samples_in << 1) as usize;
            buf.copy_within(n2..n2 + RESAMPLER_ORDER_FIR_12, 0);
        } else {
            break;
        }
    }

    /* Copy last part of filtered signal to the state for the next call */
    let n2 = (n_samples_in << 1) as usize;
    s.s_fir_i16[..RESAMPLER_ORDER_FIR_12].copy_from_slice(&buf[n2..n2 + RESAMPLER_ORDER_FIR_12]);
}

/* Upsample by a factor 2, high quality */
/* Uses 2nd order allpass filters for the 2x upsampling, followed by a      */
/* notch filter just above Nyquist.                                         */
/// Translation of `silk_resampler_private_up2_HQ()`.
fn silk_resampler_private_up2_hq(
    s: &mut [i32; 6], /* I/O  Resampler state [ 6 ]       */
    out: &mut [i16],  /* O    Output signal [ 2 * len ]   */
    in_: &[i16],      /* I    Input signal [ len ]        */
    len: i32,         /* I    Number of input samples     */
) {
    let hq0 = &SILK_RESAMPLER_UP2_HQ_0;
    let hq1 = &SILK_RESAMPLER_UP2_HQ_1;
    debug_assert!(hq0[0] > 0);
    debug_assert!(hq0[1] > 0);
    debug_assert!(hq0[2] < 0);
    debug_assert!(hq1[0] > 0);
    debug_assert!(hq1[1] > 0);
    debug_assert!(hq1[2] < 0);

    /* Internal variables and state are in Q10 format */
    for k in 0..len as usize {
        /* Convert to Q10 */
        let in32 = silk_lshift(in_[k] as i32, 10);

        /* First all-pass section for even output sample */
        let mut y = in32.wrapping_sub(s[0]);
        let mut x = silk_smulwb(y, hq0[0] as i32);
        let mut out32_1 = s[0].wrapping_add(x);
        s[0] = in32.wrapping_add(x);

        /* Second all-pass section for even output sample */
        y = out32_1.wrapping_sub(s[1]);
        x = silk_smulwb(y, hq0[1] as i32);
        let mut out32_2 = s[1].wrapping_add(x);
        s[1] = out32_1.wrapping_add(x);

        /* Third all-pass section for even output sample */
        y = out32_2.wrapping_sub(s[2]);
        x = silk_smlawb(y, y, hq0[2] as i32);
        out32_1 = s[2].wrapping_add(x);
        s[2] = out32_2.wrapping_add(x);

        /* Apply gain in Q15, convert back to int16 and store to output */
        out[2 * k] = silk_sat16(silk_rshift_round(out32_1, 10)) as i16;

        /* First all-pass section for odd output sample */
        y = in32.wrapping_sub(s[3]);
        x = silk_smulwb(y, hq1[0] as i32);
        out32_1 = s[3].wrapping_add(x);
        s[3] = in32.wrapping_add(x);

        /* Second all-pass section for odd output sample */
        y = out32_1.wrapping_sub(s[4]);
        x = silk_smulwb(y, hq1[1] as i32);
        out32_2 = s[4].wrapping_add(x);
        s[4] = out32_1.wrapping_add(x);

        /* Third all-pass section for odd output sample */
        y = out32_2.wrapping_sub(s[5]);
        x = silk_smlawb(y, y, hq1[2] as i32);
        out32_1 = s[5].wrapping_add(x);
        s[5] = out32_2.wrapping_add(x);

        /* Apply gain in Q15, convert back to int16 and store to output */
        out[2 * k + 1] = silk_sat16(silk_rshift_round(out32_1, 10)) as i16;
    }
}

/// Translation of `silk_resampler_private_up2_HQ_wrapper()`.
fn silk_resampler_private_up2_hq_wrapper(
    s: &mut SilkResamplerStateStruct, /* I/O  Resampler state (unused)    */
    out: &mut [i16],                  /* O    Output signal [ 2 * len ]   */
    in_: &[i16],                      /* I    Input signal [ len ]        */
    len: i32,                         /* I    Number of input samples     */
) {
    silk_resampler_private_up2_hq(&mut s.s_iir, out, in_, len);
}
