// Rust translation of silk/decode_pulses.c, silk/shell_coder.c (the
// decoder) and silk/code_signs.c (the decoder) from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding the excitation pulses of a SILK frame.

use super::define::{
    LOG2_SHELL_CODEC_FRAME_LENGTH, MAX_NB_SHELL_BLOCKS, N_RATE_LEVELS, SHELL_CODEC_FRAME_LENGTH,
    SILK_MAX_PULSES,
};
use super::sigproc_fix::{silk_lshift, silk_rshift, silk_smulbb};
use super::tables::{
    SILK_LSB_ICDF, SILK_PULSES_PER_BLOCK_ICDF, SILK_RATE_LEVELS_ICDF, SILK_SHELL_CODE_TABLE0,
    SILK_SHELL_CODE_TABLE1, SILK_SHELL_CODE_TABLE2, SILK_SHELL_CODE_TABLE3,
    SILK_SHELL_CODE_TABLE_OFFSETS, SILK_SIGN_ICDF,
};
use crate::opus::celt::entcode::EcDec;

/* shell coder; pulse-subframe length is hardcoded */

#[inline(always)]
fn decode_split(
    ps_range_dec: &mut EcDec<'_>,
    p: i32,             /* I    pulse amplitude of current subframe         */
    shell_table: &[u8], /* I    table of shell cdfs                         */
) -> (i16, i16) {
    if p > 0 {
        let c1 = ps_range_dec.ec_dec_icdf(
            &shell_table[SILK_SHELL_CODE_TABLE_OFFSETS[p as usize] as usize..],
            8,
        ) as i16;
        (c1, (p - c1 as i32) as i16)
    } else {
        (0, 0)
    }
}

/* Shell decoder, operates on one shell code frame of 16 pulses */
/// Translation of `silk_shell_decoder()`.
pub(crate) fn silk_shell_decoder(
    pulses0: &mut [i16], /* O    data: nonnegative pulse amplitudes          */
    ps_range_dec: &mut EcDec<'_>, /* I/O  Compressor data structure                   */
    pulses4: i32,        /* I    number of pulses per pulse-subframe         */
) {
    let mut pulses3 = [0i16; 2];
    let mut pulses2 = [0i16; 4];
    let mut pulses1 = [0i16; 8];

    /* this function operates on one shell code frame of 16 pulses */
    macro_rules! split {
        ($a:expr, $b:expr, $p:expr, $t:expr) => {{
            let (c1, c2) = decode_split(ps_range_dec, $p as i32, &$t);
            $a = c1;
            $b = c2;
        }};
    }

    split!(pulses3[0], pulses3[1], pulses4, SILK_SHELL_CODE_TABLE3);

    split!(pulses2[0], pulses2[1], pulses3[0], SILK_SHELL_CODE_TABLE2);

    split!(pulses1[0], pulses1[1], pulses2[0], SILK_SHELL_CODE_TABLE1);
    split!(pulses0[0], pulses0[1], pulses1[0], SILK_SHELL_CODE_TABLE0);
    split!(pulses0[2], pulses0[3], pulses1[1], SILK_SHELL_CODE_TABLE0);

    split!(pulses1[2], pulses1[3], pulses2[1], SILK_SHELL_CODE_TABLE1);
    split!(pulses0[4], pulses0[5], pulses1[2], SILK_SHELL_CODE_TABLE0);
    split!(pulses0[6], pulses0[7], pulses1[3], SILK_SHELL_CODE_TABLE0);

    split!(pulses2[2], pulses2[3], pulses3[1], SILK_SHELL_CODE_TABLE2);

    split!(pulses1[4], pulses1[5], pulses2[2], SILK_SHELL_CODE_TABLE1);
    split!(pulses0[8], pulses0[9], pulses1[4], SILK_SHELL_CODE_TABLE0);
    split!(pulses0[10], pulses0[11], pulses1[5], SILK_SHELL_CODE_TABLE0);

    split!(pulses1[6], pulses1[7], pulses2[3], SILK_SHELL_CODE_TABLE1);
    split!(pulses0[12], pulses0[13], pulses1[6], SILK_SHELL_CODE_TABLE0);
    split!(pulses0[14], pulses0[15], pulses1[7], SILK_SHELL_CODE_TABLE0);
}

/*#define silk_dec_map(a)                ((a) > 0 ? 1 : -1)*/
/* shifting avoids if-statement */
#[inline(always)]
fn silk_dec_map(a: i32) -> i32 {
    silk_lshift(a, 1) - 1
}

/* Decodes signs of excitation */
/// Translation of `silk_decode_signs()`.
pub(crate) fn silk_decode_signs(
    ps_range_dec: &mut EcDec<'_>,
    pulses: &mut [i16],
    mut length: i32,
    signal_type: i32,
    quant_offset_type: i32,
    sum_pulses: &[i32; MAX_NB_SHELL_BLOCKS],
) {
    let mut icdf = [0u8; 2];
    let mut q = 0usize;

    icdf[1] = 0;
    let i0 = silk_smulbb(7, quant_offset_type + (signal_type << 1)) as usize;
    let icdf_ptr = &SILK_SIGN_ICDF[i0..];
    length = silk_rshift(
        length + SHELL_CODEC_FRAME_LENGTH as i32 / 2,
        LOG2_SHELL_CODEC_FRAME_LENGTH,
    );
    for &p in sum_pulses.iter().take(length as usize) {
        if p > 0 {
            icdf[0] = icdf_ptr[(p & 0x1F).min(6) as usize];
            for j in 0..SHELL_CODEC_FRAME_LENGTH {
                if pulses[q + j] > 0 {
                    /* attach sign */
                    /* implementation with shift, subtraction, multiplication */
                    pulses[q + j] = (pulses[q + j] as i32
                        * silk_dec_map(ps_range_dec.ec_dec_icdf(&icdf, 8)))
                        as i16;
                }
            }
        }
        q += SHELL_CODEC_FRAME_LENGTH;
    }
}

/*********************************************/
/* Decode quantization indices of excitation */
/*********************************************/
/// Translation of `silk_decode_pulses()`.
pub(crate) fn silk_decode_pulses(
    ps_range_dec: &mut EcDec<'_>,
    pulses: &mut [i16],
    signal_type: i32,
    quant_offset_type: i32,
    frame_length: i32,
) {
    let mut sum_pulses = [0i32; MAX_NB_SHELL_BLOCKS];
    let mut n_lshifts = [0i32; MAX_NB_SHELL_BLOCKS];

    /*********************/
    /* Decode rate level */
    /*********************/
    let rate_level_index =
        ps_range_dec.ec_dec_icdf(&SILK_RATE_LEVELS_ICDF[(signal_type >> 1) as usize], 8);

    /* Calculate number of shell blocks */
    let mut iter = silk_rshift(frame_length, LOG2_SHELL_CODEC_FRAME_LENGTH) as usize;
    if iter * SHELL_CODEC_FRAME_LENGTH < frame_length as usize {
        debug_assert!(frame_length == 12 * 10); /* Make sure only happens for 10 ms @ 12 kHz */
        iter += 1;
    }

    /***************************************************/
    /* Sum-Weighted-Pulses Decoding                    */
    /***************************************************/
    let cdf_ptr = &SILK_PULSES_PER_BLOCK_ICDF[rate_level_index as usize];
    for i in 0..iter {
        n_lshifts[i] = 0;
        sum_pulses[i] = ps_range_dec.ec_dec_icdf(cdf_ptr, 8);

        /* LSB indication */
        while sum_pulses[i] == SILK_MAX_PULSES + 1 {
            n_lshifts[i] += 1;
            /* When we've already got 10 LSBs, we shift the table to not allow (SILK_MAX_PULSES + 1) */
            sum_pulses[i] = ps_range_dec.ec_dec_icdf(
                &SILK_PULSES_PER_BLOCK_ICDF[N_RATE_LEVELS - 1][(n_lshifts[i] == 10) as usize..],
                8,
            );
        }
    }

    /***************************************************/
    /* Shell decoding                                  */
    /***************************************************/
    for i in 0..iter {
        let off = silk_smulbb(i as i32, SHELL_CODEC_FRAME_LENGTH as i32) as usize;
        if sum_pulses[i] > 0 {
            silk_shell_decoder(&mut pulses[off..], ps_range_dec, sum_pulses[i]);
        } else {
            pulses[off..off + SHELL_CODEC_FRAME_LENGTH].fill(0);
        }
    }

    /***************************************************/
    /* LSB Decoding                                    */
    /***************************************************/
    for i in 0..iter {
        if n_lshifts[i] > 0 {
            let n_ls = n_lshifts[i];
            let off = silk_smulbb(i as i32, SHELL_CODEC_FRAME_LENGTH as i32) as usize;
            for k in 0..SHELL_CODEC_FRAME_LENGTH {
                let mut abs_q = pulses[off + k] as i32;
                for _ in 0..n_ls {
                    abs_q = silk_lshift(abs_q, 1);
                    abs_q += ps_range_dec.ec_dec_icdf(&SILK_LSB_ICDF, 8);
                }
                pulses[off + k] = abs_q as i16;
            }
            /* Mark the number of pulses non-zero for sign decoding. */
            sum_pulses[i] |= n_ls << 5;
        }
    }

    /****************************************/
    /* Decode and add signs to pulse signal */
    /****************************************/
    silk_decode_signs(
        ps_range_dec,
        pulses,
        frame_length,
        signal_type,
        quant_offset_type,
        &sum_pulses,
    );
}
