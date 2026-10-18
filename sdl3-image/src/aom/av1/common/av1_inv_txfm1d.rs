// Rust translation of av1/common/av1_inv_txfm1d.c and
// av1/common/av1_inv_txfm1d.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The inverse 1D DCTs of 4 to 32 points (the saver's transforms are at
//! most 32 samples wide: its largest partition is 32x32 and its transforms
//! are all DCTs, or the 4x4 Walsh-Hadamard transform when lossless). The
//! `av1_range_check_buf()` calls do nothing in this configuration and are
//! left out.

use super::av1_txfm::{cospi_arr, half_btf, round_shift, NEW_SQRT2, NEW_SQRT2_BITS};
use crate::aom::aom_dsp::aom_dsp_common::clamp64;

#[inline]
pub(crate) fn clamp_value(value: i32, bit: i8) -> i32 {
    if bit <= 0 {
        return value; // Do nothing for invalid clamp bit.
    }
    let max_value = (1i64 << (bit - 1)) - 1;
    let min_value = -(1i64 << (bit - 1));
    clamp64(value as i64, min_value, max_value) as i32
}

#[inline]
pub(crate) fn clamp_buf(buf: &mut [i32], size: i32, bit: i8) {
    for i in 0..size as usize {
        buf[i] = clamp_value(buf[i], bit);
    }
}

pub(crate) fn av1_idct4(input: &[i32], output: &mut [i32], cos_bit: i8, stage_range: &[i8]) {
    let cospi = cospi_arr(cos_bit as i32);
    let mut stage: usize = 0;
    let mut step = [0i32; 4];

    // stage 0;

    // stage 1;
    stage += 1;
    {
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = input[0];
        bf1[1] = input[2];
        bf1[2] = input[1];
        bf1[3] = input[3];
    }

    // stage 2
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = half_btf(cospi[32], bf0[0], cospi[32], bf0[1], cos_bit);
        bf1[1] = half_btf(cospi[32], bf0[0], -cospi[32], bf0[1], cos_bit);
        bf1[2] = half_btf(cospi[48], bf0[2], -cospi[16], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[16], bf0[2], cospi[48], bf0[3], cos_bit);
    }

    // stage 3
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[3], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[2], stage_range[stage]);
        bf1[2] = clamp_value(bf0[1] - bf0[2], stage_range[stage]);
        bf1[3] = clamp_value(bf0[0] - bf0[3], stage_range[stage]);
    }
}

pub(crate) fn av1_idct8(input: &[i32], output: &mut [i32], cos_bit: i8, stage_range: &[i8]) {
    let cospi = cospi_arr(cos_bit as i32);
    let mut stage: usize = 0;
    let mut step = [0i32; 8];

    // stage 0;

    // stage 1;
    stage += 1;
    {
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = input[0];
        bf1[1] = input[4];
        bf1[2] = input[2];
        bf1[3] = input[6];
        bf1[4] = input[1];
        bf1[5] = input[5];
        bf1[6] = input[3];
        bf1[7] = input[7];
    }

    // stage 2
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = half_btf(cospi[56], bf0[4], -cospi[8], bf0[7], cos_bit);
        bf1[5] = half_btf(cospi[24], bf0[5], -cospi[40], bf0[6], cos_bit);
        bf1[6] = half_btf(cospi[40], bf0[5], cospi[24], bf0[6], cos_bit);
        bf1[7] = half_btf(cospi[8], bf0[4], cospi[56], bf0[7], cos_bit);
    }

    // stage 3
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = half_btf(cospi[32], bf0[0], cospi[32], bf0[1], cos_bit);
        bf1[1] = half_btf(cospi[32], bf0[0], -cospi[32], bf0[1], cos_bit);
        bf1[2] = half_btf(cospi[48], bf0[2], -cospi[16], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[16], bf0[2], cospi[48], bf0[3], cos_bit);
        bf1[4] = clamp_value(bf0[4] + bf0[5], stage_range[stage]);
        bf1[5] = clamp_value(bf0[4] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(-bf0[6] + bf0[7], stage_range[stage]);
        bf1[7] = clamp_value(bf0[6] + bf0[7], stage_range[stage]);
    }

    // stage 4
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = clamp_value(bf0[0] + bf0[3], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[2], stage_range[stage]);
        bf1[2] = clamp_value(bf0[1] - bf0[2], stage_range[stage]);
        bf1[3] = clamp_value(bf0[0] - bf0[3], stage_range[stage]);
        bf1[4] = bf0[4];
        bf1[5] = half_btf(-cospi[32], bf0[5], cospi[32], bf0[6], cos_bit);
        bf1[6] = half_btf(cospi[32], bf0[5], cospi[32], bf0[6], cos_bit);
        bf1[7] = bf0[7];
    }

    // stage 5
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[7], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[6], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[5], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[4], stage_range[stage]);
        bf1[4] = clamp_value(bf0[3] - bf0[4], stage_range[stage]);
        bf1[5] = clamp_value(bf0[2] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(bf0[1] - bf0[6], stage_range[stage]);
        bf1[7] = clamp_value(bf0[0] - bf0[7], stage_range[stage]);
    }
}

pub(crate) fn av1_idct16(input: &[i32], output: &mut [i32], cos_bit: i8, stage_range: &[i8]) {
    let cospi = cospi_arr(cos_bit as i32);
    let mut stage: usize = 0;
    let mut step = [0i32; 16];

    // stage 0;

    // stage 1;
    stage += 1;
    {
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = input[0];
        bf1[1] = input[8];
        bf1[2] = input[4];
        bf1[3] = input[12];
        bf1[4] = input[2];
        bf1[5] = input[10];
        bf1[6] = input[6];
        bf1[7] = input[14];
        bf1[8] = input[1];
        bf1[9] = input[9];
        bf1[10] = input[5];
        bf1[11] = input[13];
        bf1[12] = input[3];
        bf1[13] = input[11];
        bf1[14] = input[7];
        bf1[15] = input[15];
    }

    // stage 2
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = bf0[4];
        bf1[5] = bf0[5];
        bf1[6] = bf0[6];
        bf1[7] = bf0[7];
        bf1[8] = half_btf(cospi[60], bf0[8], -cospi[4], bf0[15], cos_bit);
        bf1[9] = half_btf(cospi[28], bf0[9], -cospi[36], bf0[14], cos_bit);
        bf1[10] = half_btf(cospi[44], bf0[10], -cospi[20], bf0[13], cos_bit);
        bf1[11] = half_btf(cospi[12], bf0[11], -cospi[52], bf0[12], cos_bit);
        bf1[12] = half_btf(cospi[52], bf0[11], cospi[12], bf0[12], cos_bit);
        bf1[13] = half_btf(cospi[20], bf0[10], cospi[44], bf0[13], cos_bit);
        bf1[14] = half_btf(cospi[36], bf0[9], cospi[28], bf0[14], cos_bit);
        bf1[15] = half_btf(cospi[4], bf0[8], cospi[60], bf0[15], cos_bit);
    }

    // stage 3
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = half_btf(cospi[56], bf0[4], -cospi[8], bf0[7], cos_bit);
        bf1[5] = half_btf(cospi[24], bf0[5], -cospi[40], bf0[6], cos_bit);
        bf1[6] = half_btf(cospi[40], bf0[5], cospi[24], bf0[6], cos_bit);
        bf1[7] = half_btf(cospi[8], bf0[4], cospi[56], bf0[7], cos_bit);
        bf1[8] = clamp_value(bf0[8] + bf0[9], stage_range[stage]);
        bf1[9] = clamp_value(bf0[8] - bf0[9], stage_range[stage]);
        bf1[10] = clamp_value(-bf0[10] + bf0[11], stage_range[stage]);
        bf1[11] = clamp_value(bf0[10] + bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(bf0[12] + bf0[13], stage_range[stage]);
        bf1[13] = clamp_value(bf0[12] - bf0[13], stage_range[stage]);
        bf1[14] = clamp_value(-bf0[14] + bf0[15], stage_range[stage]);
        bf1[15] = clamp_value(bf0[14] + bf0[15], stage_range[stage]);
    }

    // stage 4
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = half_btf(cospi[32], bf0[0], cospi[32], bf0[1], cos_bit);
        bf1[1] = half_btf(cospi[32], bf0[0], -cospi[32], bf0[1], cos_bit);
        bf1[2] = half_btf(cospi[48], bf0[2], -cospi[16], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[16], bf0[2], cospi[48], bf0[3], cos_bit);
        bf1[4] = clamp_value(bf0[4] + bf0[5], stage_range[stage]);
        bf1[5] = clamp_value(bf0[4] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(-bf0[6] + bf0[7], stage_range[stage]);
        bf1[7] = clamp_value(bf0[6] + bf0[7], stage_range[stage]);
        bf1[8] = bf0[8];
        bf1[9] = half_btf(-cospi[16], bf0[9], cospi[48], bf0[14], cos_bit);
        bf1[10] = half_btf(-cospi[48], bf0[10], -cospi[16], bf0[13], cos_bit);
        bf1[11] = bf0[11];
        bf1[12] = bf0[12];
        bf1[13] = half_btf(-cospi[16], bf0[10], cospi[48], bf0[13], cos_bit);
        bf1[14] = half_btf(cospi[48], bf0[9], cospi[16], bf0[14], cos_bit);
        bf1[15] = bf0[15];
    }

    // stage 5
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[3], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[2], stage_range[stage]);
        bf1[2] = clamp_value(bf0[1] - bf0[2], stage_range[stage]);
        bf1[3] = clamp_value(bf0[0] - bf0[3], stage_range[stage]);
        bf1[4] = bf0[4];
        bf1[5] = half_btf(-cospi[32], bf0[5], cospi[32], bf0[6], cos_bit);
        bf1[6] = half_btf(cospi[32], bf0[5], cospi[32], bf0[6], cos_bit);
        bf1[7] = bf0[7];
        bf1[8] = clamp_value(bf0[8] + bf0[11], stage_range[stage]);
        bf1[9] = clamp_value(bf0[9] + bf0[10], stage_range[stage]);
        bf1[10] = clamp_value(bf0[9] - bf0[10], stage_range[stage]);
        bf1[11] = clamp_value(bf0[8] - bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(-bf0[12] + bf0[15], stage_range[stage]);
        bf1[13] = clamp_value(-bf0[13] + bf0[14], stage_range[stage]);
        bf1[14] = clamp_value(bf0[13] + bf0[14], stage_range[stage]);
        bf1[15] = clamp_value(bf0[12] + bf0[15], stage_range[stage]);
    }

    // stage 6
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = clamp_value(bf0[0] + bf0[7], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[6], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[5], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[4], stage_range[stage]);
        bf1[4] = clamp_value(bf0[3] - bf0[4], stage_range[stage]);
        bf1[5] = clamp_value(bf0[2] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(bf0[1] - bf0[6], stage_range[stage]);
        bf1[7] = clamp_value(bf0[0] - bf0[7], stage_range[stage]);
        bf1[8] = bf0[8];
        bf1[9] = bf0[9];
        bf1[10] = half_btf(-cospi[32], bf0[10], cospi[32], bf0[13], cos_bit);
        bf1[11] = half_btf(-cospi[32], bf0[11], cospi[32], bf0[12], cos_bit);
        bf1[12] = half_btf(cospi[32], bf0[11], cospi[32], bf0[12], cos_bit);
        bf1[13] = half_btf(cospi[32], bf0[10], cospi[32], bf0[13], cos_bit);
        bf1[14] = bf0[14];
        bf1[15] = bf0[15];
    }

    // stage 7
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[15], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[14], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[13], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[12], stage_range[stage]);
        bf1[4] = clamp_value(bf0[4] + bf0[11], stage_range[stage]);
        bf1[5] = clamp_value(bf0[5] + bf0[10], stage_range[stage]);
        bf1[6] = clamp_value(bf0[6] + bf0[9], stage_range[stage]);
        bf1[7] = clamp_value(bf0[7] + bf0[8], stage_range[stage]);
        bf1[8] = clamp_value(bf0[7] - bf0[8], stage_range[stage]);
        bf1[9] = clamp_value(bf0[6] - bf0[9], stage_range[stage]);
        bf1[10] = clamp_value(bf0[5] - bf0[10], stage_range[stage]);
        bf1[11] = clamp_value(bf0[4] - bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(bf0[3] - bf0[12], stage_range[stage]);
        bf1[13] = clamp_value(bf0[2] - bf0[13], stage_range[stage]);
        bf1[14] = clamp_value(bf0[1] - bf0[14], stage_range[stage]);
        bf1[15] = clamp_value(bf0[0] - bf0[15], stage_range[stage]);
    }
}

pub(crate) fn av1_idct32(input: &[i32], output: &mut [i32], cos_bit: i8, stage_range: &[i8]) {
    let cospi = cospi_arr(cos_bit as i32);
    let mut stage: usize = 0;
    let mut step = [0i32; 32];

    // stage 0;

    // stage 1;
    stage += 1;
    {
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = input[0];
        bf1[1] = input[16];
        bf1[2] = input[8];
        bf1[3] = input[24];
        bf1[4] = input[4];
        bf1[5] = input[20];
        bf1[6] = input[12];
        bf1[7] = input[28];
        bf1[8] = input[2];
        bf1[9] = input[18];
        bf1[10] = input[10];
        bf1[11] = input[26];
        bf1[12] = input[6];
        bf1[13] = input[22];
        bf1[14] = input[14];
        bf1[15] = input[30];
        bf1[16] = input[1];
        bf1[17] = input[17];
        bf1[18] = input[9];
        bf1[19] = input[25];
        bf1[20] = input[5];
        bf1[21] = input[21];
        bf1[22] = input[13];
        bf1[23] = input[29];
        bf1[24] = input[3];
        bf1[25] = input[19];
        bf1[26] = input[11];
        bf1[27] = input[27];
        bf1[28] = input[7];
        bf1[29] = input[23];
        bf1[30] = input[15];
        bf1[31] = input[31];
    }

    // stage 2
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = bf0[4];
        bf1[5] = bf0[5];
        bf1[6] = bf0[6];
        bf1[7] = bf0[7];
        bf1[8] = bf0[8];
        bf1[9] = bf0[9];
        bf1[10] = bf0[10];
        bf1[11] = bf0[11];
        bf1[12] = bf0[12];
        bf1[13] = bf0[13];
        bf1[14] = bf0[14];
        bf1[15] = bf0[15];
        bf1[16] = half_btf(cospi[62], bf0[16], -cospi[2], bf0[31], cos_bit);
        bf1[17] = half_btf(cospi[30], bf0[17], -cospi[34], bf0[30], cos_bit);
        bf1[18] = half_btf(cospi[46], bf0[18], -cospi[18], bf0[29], cos_bit);
        bf1[19] = half_btf(cospi[14], bf0[19], -cospi[50], bf0[28], cos_bit);
        bf1[20] = half_btf(cospi[54], bf0[20], -cospi[10], bf0[27], cos_bit);
        bf1[21] = half_btf(cospi[22], bf0[21], -cospi[42], bf0[26], cos_bit);
        bf1[22] = half_btf(cospi[38], bf0[22], -cospi[26], bf0[25], cos_bit);
        bf1[23] = half_btf(cospi[6], bf0[23], -cospi[58], bf0[24], cos_bit);
        bf1[24] = half_btf(cospi[58], bf0[23], cospi[6], bf0[24], cos_bit);
        bf1[25] = half_btf(cospi[26], bf0[22], cospi[38], bf0[25], cos_bit);
        bf1[26] = half_btf(cospi[42], bf0[21], cospi[22], bf0[26], cos_bit);
        bf1[27] = half_btf(cospi[10], bf0[20], cospi[54], bf0[27], cos_bit);
        bf1[28] = half_btf(cospi[50], bf0[19], cospi[14], bf0[28], cos_bit);
        bf1[29] = half_btf(cospi[18], bf0[18], cospi[46], bf0[29], cos_bit);
        bf1[30] = half_btf(cospi[34], bf0[17], cospi[30], bf0[30], cos_bit);
        bf1[31] = half_btf(cospi[2], bf0[16], cospi[62], bf0[31], cos_bit);
    }

    // stage 3
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = bf0[4];
        bf1[5] = bf0[5];
        bf1[6] = bf0[6];
        bf1[7] = bf0[7];
        bf1[8] = half_btf(cospi[60], bf0[8], -cospi[4], bf0[15], cos_bit);
        bf1[9] = half_btf(cospi[28], bf0[9], -cospi[36], bf0[14], cos_bit);
        bf1[10] = half_btf(cospi[44], bf0[10], -cospi[20], bf0[13], cos_bit);
        bf1[11] = half_btf(cospi[12], bf0[11], -cospi[52], bf0[12], cos_bit);
        bf1[12] = half_btf(cospi[52], bf0[11], cospi[12], bf0[12], cos_bit);
        bf1[13] = half_btf(cospi[20], bf0[10], cospi[44], bf0[13], cos_bit);
        bf1[14] = half_btf(cospi[36], bf0[9], cospi[28], bf0[14], cos_bit);
        bf1[15] = half_btf(cospi[4], bf0[8], cospi[60], bf0[15], cos_bit);
        bf1[16] = clamp_value(bf0[16] + bf0[17], stage_range[stage]);
        bf1[17] = clamp_value(bf0[16] - bf0[17], stage_range[stage]);
        bf1[18] = clamp_value(-bf0[18] + bf0[19], stage_range[stage]);
        bf1[19] = clamp_value(bf0[18] + bf0[19], stage_range[stage]);
        bf1[20] = clamp_value(bf0[20] + bf0[21], stage_range[stage]);
        bf1[21] = clamp_value(bf0[20] - bf0[21], stage_range[stage]);
        bf1[22] = clamp_value(-bf0[22] + bf0[23], stage_range[stage]);
        bf1[23] = clamp_value(bf0[22] + bf0[23], stage_range[stage]);
        bf1[24] = clamp_value(bf0[24] + bf0[25], stage_range[stage]);
        bf1[25] = clamp_value(bf0[24] - bf0[25], stage_range[stage]);
        bf1[26] = clamp_value(-bf0[26] + bf0[27], stage_range[stage]);
        bf1[27] = clamp_value(bf0[26] + bf0[27], stage_range[stage]);
        bf1[28] = clamp_value(bf0[28] + bf0[29], stage_range[stage]);
        bf1[29] = clamp_value(bf0[28] - bf0[29], stage_range[stage]);
        bf1[30] = clamp_value(-bf0[30] + bf0[31], stage_range[stage]);
        bf1[31] = clamp_value(bf0[30] + bf0[31], stage_range[stage]);
    }

    // stage 4
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = half_btf(cospi[56], bf0[4], -cospi[8], bf0[7], cos_bit);
        bf1[5] = half_btf(cospi[24], bf0[5], -cospi[40], bf0[6], cos_bit);
        bf1[6] = half_btf(cospi[40], bf0[5], cospi[24], bf0[6], cos_bit);
        bf1[7] = half_btf(cospi[8], bf0[4], cospi[56], bf0[7], cos_bit);
        bf1[8] = clamp_value(bf0[8] + bf0[9], stage_range[stage]);
        bf1[9] = clamp_value(bf0[8] - bf0[9], stage_range[stage]);
        bf1[10] = clamp_value(-bf0[10] + bf0[11], stage_range[stage]);
        bf1[11] = clamp_value(bf0[10] + bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(bf0[12] + bf0[13], stage_range[stage]);
        bf1[13] = clamp_value(bf0[12] - bf0[13], stage_range[stage]);
        bf1[14] = clamp_value(-bf0[14] + bf0[15], stage_range[stage]);
        bf1[15] = clamp_value(bf0[14] + bf0[15], stage_range[stage]);
        bf1[16] = bf0[16];
        bf1[17] = half_btf(-cospi[8], bf0[17], cospi[56], bf0[30], cos_bit);
        bf1[18] = half_btf(-cospi[56], bf0[18], -cospi[8], bf0[29], cos_bit);
        bf1[19] = bf0[19];
        bf1[20] = bf0[20];
        bf1[21] = half_btf(-cospi[40], bf0[21], cospi[24], bf0[26], cos_bit);
        bf1[22] = half_btf(-cospi[24], bf0[22], -cospi[40], bf0[25], cos_bit);
        bf1[23] = bf0[23];
        bf1[24] = bf0[24];
        bf1[25] = half_btf(-cospi[40], bf0[22], cospi[24], bf0[25], cos_bit);
        bf1[26] = half_btf(cospi[24], bf0[21], cospi[40], bf0[26], cos_bit);
        bf1[27] = bf0[27];
        bf1[28] = bf0[28];
        bf1[29] = half_btf(-cospi[8], bf0[18], cospi[56], bf0[29], cos_bit);
        bf1[30] = half_btf(cospi[56], bf0[17], cospi[8], bf0[30], cos_bit);
        bf1[31] = bf0[31];
    }

    // stage 5
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = half_btf(cospi[32], bf0[0], cospi[32], bf0[1], cos_bit);
        bf1[1] = half_btf(cospi[32], bf0[0], -cospi[32], bf0[1], cos_bit);
        bf1[2] = half_btf(cospi[48], bf0[2], -cospi[16], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[16], bf0[2], cospi[48], bf0[3], cos_bit);
        bf1[4] = clamp_value(bf0[4] + bf0[5], stage_range[stage]);
        bf1[5] = clamp_value(bf0[4] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(-bf0[6] + bf0[7], stage_range[stage]);
        bf1[7] = clamp_value(bf0[6] + bf0[7], stage_range[stage]);
        bf1[8] = bf0[8];
        bf1[9] = half_btf(-cospi[16], bf0[9], cospi[48], bf0[14], cos_bit);
        bf1[10] = half_btf(-cospi[48], bf0[10], -cospi[16], bf0[13], cos_bit);
        bf1[11] = bf0[11];
        bf1[12] = bf0[12];
        bf1[13] = half_btf(-cospi[16], bf0[10], cospi[48], bf0[13], cos_bit);
        bf1[14] = half_btf(cospi[48], bf0[9], cospi[16], bf0[14], cos_bit);
        bf1[15] = bf0[15];
        bf1[16] = clamp_value(bf0[16] + bf0[19], stage_range[stage]);
        bf1[17] = clamp_value(bf0[17] + bf0[18], stage_range[stage]);
        bf1[18] = clamp_value(bf0[17] - bf0[18], stage_range[stage]);
        bf1[19] = clamp_value(bf0[16] - bf0[19], stage_range[stage]);
        bf1[20] = clamp_value(-bf0[20] + bf0[23], stage_range[stage]);
        bf1[21] = clamp_value(-bf0[21] + bf0[22], stage_range[stage]);
        bf1[22] = clamp_value(bf0[21] + bf0[22], stage_range[stage]);
        bf1[23] = clamp_value(bf0[20] + bf0[23], stage_range[stage]);
        bf1[24] = clamp_value(bf0[24] + bf0[27], stage_range[stage]);
        bf1[25] = clamp_value(bf0[25] + bf0[26], stage_range[stage]);
        bf1[26] = clamp_value(bf0[25] - bf0[26], stage_range[stage]);
        bf1[27] = clamp_value(bf0[24] - bf0[27], stage_range[stage]);
        bf1[28] = clamp_value(-bf0[28] + bf0[31], stage_range[stage]);
        bf1[29] = clamp_value(-bf0[29] + bf0[30], stage_range[stage]);
        bf1[30] = clamp_value(bf0[29] + bf0[30], stage_range[stage]);
        bf1[31] = clamp_value(bf0[28] + bf0[31], stage_range[stage]);
    }

    // stage 6
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = clamp_value(bf0[0] + bf0[3], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[2], stage_range[stage]);
        bf1[2] = clamp_value(bf0[1] - bf0[2], stage_range[stage]);
        bf1[3] = clamp_value(bf0[0] - bf0[3], stage_range[stage]);
        bf1[4] = bf0[4];
        bf1[5] = half_btf(-cospi[32], bf0[5], cospi[32], bf0[6], cos_bit);
        bf1[6] = half_btf(cospi[32], bf0[5], cospi[32], bf0[6], cos_bit);
        bf1[7] = bf0[7];
        bf1[8] = clamp_value(bf0[8] + bf0[11], stage_range[stage]);
        bf1[9] = clamp_value(bf0[9] + bf0[10], stage_range[stage]);
        bf1[10] = clamp_value(bf0[9] - bf0[10], stage_range[stage]);
        bf1[11] = clamp_value(bf0[8] - bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(-bf0[12] + bf0[15], stage_range[stage]);
        bf1[13] = clamp_value(-bf0[13] + bf0[14], stage_range[stage]);
        bf1[14] = clamp_value(bf0[13] + bf0[14], stage_range[stage]);
        bf1[15] = clamp_value(bf0[12] + bf0[15], stage_range[stage]);
        bf1[16] = bf0[16];
        bf1[17] = bf0[17];
        bf1[18] = half_btf(-cospi[16], bf0[18], cospi[48], bf0[29], cos_bit);
        bf1[19] = half_btf(-cospi[16], bf0[19], cospi[48], bf0[28], cos_bit);
        bf1[20] = half_btf(-cospi[48], bf0[20], -cospi[16], bf0[27], cos_bit);
        bf1[21] = half_btf(-cospi[48], bf0[21], -cospi[16], bf0[26], cos_bit);
        bf1[22] = bf0[22];
        bf1[23] = bf0[23];
        bf1[24] = bf0[24];
        bf1[25] = bf0[25];
        bf1[26] = half_btf(-cospi[16], bf0[21], cospi[48], bf0[26], cos_bit);
        bf1[27] = half_btf(-cospi[16], bf0[20], cospi[48], bf0[27], cos_bit);
        bf1[28] = half_btf(cospi[48], bf0[19], cospi[16], bf0[28], cos_bit);
        bf1[29] = half_btf(cospi[48], bf0[18], cospi[16], bf0[29], cos_bit);
        bf1[30] = bf0[30];
        bf1[31] = bf0[31];
    }

    // stage 7
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[7], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[6], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[5], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[4], stage_range[stage]);
        bf1[4] = clamp_value(bf0[3] - bf0[4], stage_range[stage]);
        bf1[5] = clamp_value(bf0[2] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(bf0[1] - bf0[6], stage_range[stage]);
        bf1[7] = clamp_value(bf0[0] - bf0[7], stage_range[stage]);
        bf1[8] = bf0[8];
        bf1[9] = bf0[9];
        bf1[10] = half_btf(-cospi[32], bf0[10], cospi[32], bf0[13], cos_bit);
        bf1[11] = half_btf(-cospi[32], bf0[11], cospi[32], bf0[12], cos_bit);
        bf1[12] = half_btf(cospi[32], bf0[11], cospi[32], bf0[12], cos_bit);
        bf1[13] = half_btf(cospi[32], bf0[10], cospi[32], bf0[13], cos_bit);
        bf1[14] = bf0[14];
        bf1[15] = bf0[15];
        bf1[16] = clamp_value(bf0[16] + bf0[23], stage_range[stage]);
        bf1[17] = clamp_value(bf0[17] + bf0[22], stage_range[stage]);
        bf1[18] = clamp_value(bf0[18] + bf0[21], stage_range[stage]);
        bf1[19] = clamp_value(bf0[19] + bf0[20], stage_range[stage]);
        bf1[20] = clamp_value(bf0[19] - bf0[20], stage_range[stage]);
        bf1[21] = clamp_value(bf0[18] - bf0[21], stage_range[stage]);
        bf1[22] = clamp_value(bf0[17] - bf0[22], stage_range[stage]);
        bf1[23] = clamp_value(bf0[16] - bf0[23], stage_range[stage]);
        bf1[24] = clamp_value(-bf0[24] + bf0[31], stage_range[stage]);
        bf1[25] = clamp_value(-bf0[25] + bf0[30], stage_range[stage]);
        bf1[26] = clamp_value(-bf0[26] + bf0[29], stage_range[stage]);
        bf1[27] = clamp_value(-bf0[27] + bf0[28], stage_range[stage]);
        bf1[28] = clamp_value(bf0[27] + bf0[28], stage_range[stage]);
        bf1[29] = clamp_value(bf0[26] + bf0[29], stage_range[stage]);
        bf1[30] = clamp_value(bf0[25] + bf0[30], stage_range[stage]);
        bf1[31] = clamp_value(bf0[24] + bf0[31], stage_range[stage]);
    }

    // stage 8
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = clamp_value(bf0[0] + bf0[15], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[14], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[13], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[12], stage_range[stage]);
        bf1[4] = clamp_value(bf0[4] + bf0[11], stage_range[stage]);
        bf1[5] = clamp_value(bf0[5] + bf0[10], stage_range[stage]);
        bf1[6] = clamp_value(bf0[6] + bf0[9], stage_range[stage]);
        bf1[7] = clamp_value(bf0[7] + bf0[8], stage_range[stage]);
        bf1[8] = clamp_value(bf0[7] - bf0[8], stage_range[stage]);
        bf1[9] = clamp_value(bf0[6] - bf0[9], stage_range[stage]);
        bf1[10] = clamp_value(bf0[5] - bf0[10], stage_range[stage]);
        bf1[11] = clamp_value(bf0[4] - bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(bf0[3] - bf0[12], stage_range[stage]);
        bf1[13] = clamp_value(bf0[2] - bf0[13], stage_range[stage]);
        bf1[14] = clamp_value(bf0[1] - bf0[14], stage_range[stage]);
        bf1[15] = clamp_value(bf0[0] - bf0[15], stage_range[stage]);
        bf1[16] = bf0[16];
        bf1[17] = bf0[17];
        bf1[18] = bf0[18];
        bf1[19] = bf0[19];
        bf1[20] = half_btf(-cospi[32], bf0[20], cospi[32], bf0[27], cos_bit);
        bf1[21] = half_btf(-cospi[32], bf0[21], cospi[32], bf0[26], cos_bit);
        bf1[22] = half_btf(-cospi[32], bf0[22], cospi[32], bf0[25], cos_bit);
        bf1[23] = half_btf(-cospi[32], bf0[23], cospi[32], bf0[24], cos_bit);
        bf1[24] = half_btf(cospi[32], bf0[23], cospi[32], bf0[24], cos_bit);
        bf1[25] = half_btf(cospi[32], bf0[22], cospi[32], bf0[25], cos_bit);
        bf1[26] = half_btf(cospi[32], bf0[21], cospi[32], bf0[26], cos_bit);
        bf1[27] = half_btf(cospi[32], bf0[20], cospi[32], bf0[27], cos_bit);
        bf1[28] = bf0[28];
        bf1[29] = bf0[29];
        bf1[30] = bf0[30];
        bf1[31] = bf0[31];
    }

    // stage 9
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[31], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[30], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[29], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[28], stage_range[stage]);
        bf1[4] = clamp_value(bf0[4] + bf0[27], stage_range[stage]);
        bf1[5] = clamp_value(bf0[5] + bf0[26], stage_range[stage]);
        bf1[6] = clamp_value(bf0[6] + bf0[25], stage_range[stage]);
        bf1[7] = clamp_value(bf0[7] + bf0[24], stage_range[stage]);
        bf1[8] = clamp_value(bf0[8] + bf0[23], stage_range[stage]);
        bf1[9] = clamp_value(bf0[9] + bf0[22], stage_range[stage]);
        bf1[10] = clamp_value(bf0[10] + bf0[21], stage_range[stage]);
        bf1[11] = clamp_value(bf0[11] + bf0[20], stage_range[stage]);
        bf1[12] = clamp_value(bf0[12] + bf0[19], stage_range[stage]);
        bf1[13] = clamp_value(bf0[13] + bf0[18], stage_range[stage]);
        bf1[14] = clamp_value(bf0[14] + bf0[17], stage_range[stage]);
        bf1[15] = clamp_value(bf0[15] + bf0[16], stage_range[stage]);
        bf1[16] = clamp_value(bf0[15] - bf0[16], stage_range[stage]);
        bf1[17] = clamp_value(bf0[14] - bf0[17], stage_range[stage]);
        bf1[18] = clamp_value(bf0[13] - bf0[18], stage_range[stage]);
        bf1[19] = clamp_value(bf0[12] - bf0[19], stage_range[stage]);
        bf1[20] = clamp_value(bf0[11] - bf0[20], stage_range[stage]);
        bf1[21] = clamp_value(bf0[10] - bf0[21], stage_range[stage]);
        bf1[22] = clamp_value(bf0[9] - bf0[22], stage_range[stage]);
        bf1[23] = clamp_value(bf0[8] - bf0[23], stage_range[stage]);
        bf1[24] = clamp_value(bf0[7] - bf0[24], stage_range[stage]);
        bf1[25] = clamp_value(bf0[6] - bf0[25], stage_range[stage]);
        bf1[26] = clamp_value(bf0[5] - bf0[26], stage_range[stage]);
        bf1[27] = clamp_value(bf0[4] - bf0[27], stage_range[stage]);
        bf1[28] = clamp_value(bf0[3] - bf0[28], stage_range[stage]);
        bf1[29] = clamp_value(bf0[2] - bf0[29], stage_range[stage]);
        bf1[30] = clamp_value(bf0[1] - bf0[30], stage_range[stage]);
        bf1[31] = clamp_value(bf0[0] - bf0[31], stage_range[stage]);
    }
}

pub(crate) fn av1_iadst8(input: &[i32], output: &mut [i32], cos_bit: i8, stage_range: &[i8]) {
    let cospi = cospi_arr(cos_bit as i32);
    let mut stage: usize = 0;
    let mut step = [0i32; 8];
    // stage 0;
    // stage 1;
    stage += 1;
    {
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = input[7];
        bf1[1] = input[0];
        bf1[2] = input[5];
        bf1[3] = input[2];
        bf1[4] = input[3];
        bf1[5] = input[4];
        bf1[6] = input[1];
        bf1[7] = input[6];
    }
    // stage 2
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = half_btf(cospi[4], bf0[0], cospi[60], bf0[1], cos_bit);
        bf1[1] = half_btf(cospi[60], bf0[0], -cospi[4], bf0[1], cos_bit);
        bf1[2] = half_btf(cospi[20], bf0[2], cospi[44], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[44], bf0[2], -cospi[20], bf0[3], cos_bit);
        bf1[4] = half_btf(cospi[36], bf0[4], cospi[28], bf0[5], cos_bit);
        bf1[5] = half_btf(cospi[28], bf0[4], -cospi[36], bf0[5], cos_bit);
        bf1[6] = half_btf(cospi[52], bf0[6], cospi[12], bf0[7], cos_bit);
        bf1[7] = half_btf(cospi[12], bf0[6], -cospi[52], bf0[7], cos_bit);
    }
    // stage 3
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[4], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[5], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[6], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[7], stage_range[stage]);
        bf1[4] = clamp_value(bf0[0] - bf0[4], stage_range[stage]);
        bf1[5] = clamp_value(bf0[1] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(bf0[2] - bf0[6], stage_range[stage]);
        bf1[7] = clamp_value(bf0[3] - bf0[7], stage_range[stage]);
    }
    // stage 4
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = half_btf(cospi[16], bf0[4], cospi[48], bf0[5], cos_bit);
        bf1[5] = half_btf(cospi[48], bf0[4], -cospi[16], bf0[5], cos_bit);
        bf1[6] = half_btf(-cospi[48], bf0[6], cospi[16], bf0[7], cos_bit);
        bf1[7] = half_btf(cospi[16], bf0[6], cospi[48], bf0[7], cos_bit);
    }
    // stage 5
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[2], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[3], stage_range[stage]);
        bf1[2] = clamp_value(bf0[0] - bf0[2], stage_range[stage]);
        bf1[3] = clamp_value(bf0[1] - bf0[3], stage_range[stage]);
        bf1[4] = clamp_value(bf0[4] + bf0[6], stage_range[stage]);
        bf1[5] = clamp_value(bf0[5] + bf0[7], stage_range[stage]);
        bf1[6] = clamp_value(bf0[4] - bf0[6], stage_range[stage]);
        bf1[7] = clamp_value(bf0[5] - bf0[7], stage_range[stage]);
    }
    // stage 6
    // (stage++: this stage clamps nothing)
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = half_btf(cospi[32], bf0[2], cospi[32], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[32], bf0[2], -cospi[32], bf0[3], cos_bit);
        bf1[4] = bf0[4];
        bf1[5] = bf0[5];
        bf1[6] = half_btf(cospi[32], bf0[6], cospi[32], bf0[7], cos_bit);
        bf1[7] = half_btf(cospi[32], bf0[6], -cospi[32], bf0[7], cos_bit);
    }
    // stage 7
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = bf0[0];
        bf1[1] = -bf0[4];
        bf1[2] = bf0[6];
        bf1[3] = -bf0[2];
        bf1[4] = bf0[3];
        bf1[5] = -bf0[7];
        bf1[6] = bf0[5];
        bf1[7] = -bf0[1];
    }
}

pub(crate) fn av1_iadst16(input: &[i32], output: &mut [i32], cos_bit: i8, stage_range: &[i8]) {
    let cospi = cospi_arr(cos_bit as i32);
    let mut stage: usize = 0;
    let mut step = [0i32; 16];
    // stage 0;
    // stage 1;
    stage += 1;
    {
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = input[15];
        bf1[1] = input[0];
        bf1[2] = input[13];
        bf1[3] = input[2];
        bf1[4] = input[11];
        bf1[5] = input[4];
        bf1[6] = input[9];
        bf1[7] = input[6];
        bf1[8] = input[7];
        bf1[9] = input[8];
        bf1[10] = input[5];
        bf1[11] = input[10];
        bf1[12] = input[3];
        bf1[13] = input[12];
        bf1[14] = input[1];
        bf1[15] = input[14];
    }
    // stage 2
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = half_btf(cospi[2], bf0[0], cospi[62], bf0[1], cos_bit);
        bf1[1] = half_btf(cospi[62], bf0[0], -cospi[2], bf0[1], cos_bit);
        bf1[2] = half_btf(cospi[10], bf0[2], cospi[54], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[54], bf0[2], -cospi[10], bf0[3], cos_bit);
        bf1[4] = half_btf(cospi[18], bf0[4], cospi[46], bf0[5], cos_bit);
        bf1[5] = half_btf(cospi[46], bf0[4], -cospi[18], bf0[5], cos_bit);
        bf1[6] = half_btf(cospi[26], bf0[6], cospi[38], bf0[7], cos_bit);
        bf1[7] = half_btf(cospi[38], bf0[6], -cospi[26], bf0[7], cos_bit);
        bf1[8] = half_btf(cospi[34], bf0[8], cospi[30], bf0[9], cos_bit);
        bf1[9] = half_btf(cospi[30], bf0[8], -cospi[34], bf0[9], cos_bit);
        bf1[10] = half_btf(cospi[42], bf0[10], cospi[22], bf0[11], cos_bit);
        bf1[11] = half_btf(cospi[22], bf0[10], -cospi[42], bf0[11], cos_bit);
        bf1[12] = half_btf(cospi[50], bf0[12], cospi[14], bf0[13], cos_bit);
        bf1[13] = half_btf(cospi[14], bf0[12], -cospi[50], bf0[13], cos_bit);
        bf1[14] = half_btf(cospi[58], bf0[14], cospi[6], bf0[15], cos_bit);
        bf1[15] = half_btf(cospi[6], bf0[14], -cospi[58], bf0[15], cos_bit);
    }
    // stage 3
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[8], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[9], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[10], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[11], stage_range[stage]);
        bf1[4] = clamp_value(bf0[4] + bf0[12], stage_range[stage]);
        bf1[5] = clamp_value(bf0[5] + bf0[13], stage_range[stage]);
        bf1[6] = clamp_value(bf0[6] + bf0[14], stage_range[stage]);
        bf1[7] = clamp_value(bf0[7] + bf0[15], stage_range[stage]);
        bf1[8] = clamp_value(bf0[0] - bf0[8], stage_range[stage]);
        bf1[9] = clamp_value(bf0[1] - bf0[9], stage_range[stage]);
        bf1[10] = clamp_value(bf0[2] - bf0[10], stage_range[stage]);
        bf1[11] = clamp_value(bf0[3] - bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(bf0[4] - bf0[12], stage_range[stage]);
        bf1[13] = clamp_value(bf0[5] - bf0[13], stage_range[stage]);
        bf1[14] = clamp_value(bf0[6] - bf0[14], stage_range[stage]);
        bf1[15] = clamp_value(bf0[7] - bf0[15], stage_range[stage]);
    }
    // stage 4
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = bf0[4];
        bf1[5] = bf0[5];
        bf1[6] = bf0[6];
        bf1[7] = bf0[7];
        bf1[8] = half_btf(cospi[8], bf0[8], cospi[56], bf0[9], cos_bit);
        bf1[9] = half_btf(cospi[56], bf0[8], -cospi[8], bf0[9], cos_bit);
        bf1[10] = half_btf(cospi[40], bf0[10], cospi[24], bf0[11], cos_bit);
        bf1[11] = half_btf(cospi[24], bf0[10], -cospi[40], bf0[11], cos_bit);
        bf1[12] = half_btf(-cospi[56], bf0[12], cospi[8], bf0[13], cos_bit);
        bf1[13] = half_btf(cospi[8], bf0[12], cospi[56], bf0[13], cos_bit);
        bf1[14] = half_btf(-cospi[24], bf0[14], cospi[40], bf0[15], cos_bit);
        bf1[15] = half_btf(cospi[40], bf0[14], cospi[24], bf0[15], cos_bit);
    }
    // stage 5
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[4], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[5], stage_range[stage]);
        bf1[2] = clamp_value(bf0[2] + bf0[6], stage_range[stage]);
        bf1[3] = clamp_value(bf0[3] + bf0[7], stage_range[stage]);
        bf1[4] = clamp_value(bf0[0] - bf0[4], stage_range[stage]);
        bf1[5] = clamp_value(bf0[1] - bf0[5], stage_range[stage]);
        bf1[6] = clamp_value(bf0[2] - bf0[6], stage_range[stage]);
        bf1[7] = clamp_value(bf0[3] - bf0[7], stage_range[stage]);
        bf1[8] = clamp_value(bf0[8] + bf0[12], stage_range[stage]);
        bf1[9] = clamp_value(bf0[9] + bf0[13], stage_range[stage]);
        bf1[10] = clamp_value(bf0[10] + bf0[14], stage_range[stage]);
        bf1[11] = clamp_value(bf0[11] + bf0[15], stage_range[stage]);
        bf1[12] = clamp_value(bf0[8] - bf0[12], stage_range[stage]);
        bf1[13] = clamp_value(bf0[9] - bf0[13], stage_range[stage]);
        bf1[14] = clamp_value(bf0[10] - bf0[14], stage_range[stage]);
        bf1[15] = clamp_value(bf0[11] - bf0[15], stage_range[stage]);
    }
    // stage 6
    stage += 1;
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = bf0[2];
        bf1[3] = bf0[3];
        bf1[4] = half_btf(cospi[16], bf0[4], cospi[48], bf0[5], cos_bit);
        bf1[5] = half_btf(cospi[48], bf0[4], -cospi[16], bf0[5], cos_bit);
        bf1[6] = half_btf(-cospi[48], bf0[6], cospi[16], bf0[7], cos_bit);
        bf1[7] = half_btf(cospi[16], bf0[6], cospi[48], bf0[7], cos_bit);
        bf1[8] = bf0[8];
        bf1[9] = bf0[9];
        bf1[10] = bf0[10];
        bf1[11] = bf0[11];
        bf1[12] = half_btf(cospi[16], bf0[12], cospi[48], bf0[13], cos_bit);
        bf1[13] = half_btf(cospi[48], bf0[12], -cospi[16], bf0[13], cos_bit);
        bf1[14] = half_btf(-cospi[48], bf0[14], cospi[16], bf0[15], cos_bit);
        bf1[15] = half_btf(cospi[16], bf0[14], cospi[48], bf0[15], cos_bit);
    }
    // stage 7
    stage += 1;
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = clamp_value(bf0[0] + bf0[2], stage_range[stage]);
        bf1[1] = clamp_value(bf0[1] + bf0[3], stage_range[stage]);
        bf1[2] = clamp_value(bf0[0] - bf0[2], stage_range[stage]);
        bf1[3] = clamp_value(bf0[1] - bf0[3], stage_range[stage]);
        bf1[4] = clamp_value(bf0[4] + bf0[6], stage_range[stage]);
        bf1[5] = clamp_value(bf0[5] + bf0[7], stage_range[stage]);
        bf1[6] = clamp_value(bf0[4] - bf0[6], stage_range[stage]);
        bf1[7] = clamp_value(bf0[5] - bf0[7], stage_range[stage]);
        bf1[8] = clamp_value(bf0[8] + bf0[10], stage_range[stage]);
        bf1[9] = clamp_value(bf0[9] + bf0[11], stage_range[stage]);
        bf1[10] = clamp_value(bf0[8] - bf0[10], stage_range[stage]);
        bf1[11] = clamp_value(bf0[9] - bf0[11], stage_range[stage]);
        bf1[12] = clamp_value(bf0[12] + bf0[14], stage_range[stage]);
        bf1[13] = clamp_value(bf0[13] + bf0[15], stage_range[stage]);
        bf1[14] = clamp_value(bf0[12] - bf0[14], stage_range[stage]);
        bf1[15] = clamp_value(bf0[13] - bf0[15], stage_range[stage]);
    }
    // stage 8
    // (stage++: this stage clamps nothing)
    {
        let bf0: &[i32] = &*output;
        let bf1: &mut [i32] = &mut step;
        bf1[0] = bf0[0];
        bf1[1] = bf0[1];
        bf1[2] = half_btf(cospi[32], bf0[2], cospi[32], bf0[3], cos_bit);
        bf1[3] = half_btf(cospi[32], bf0[2], -cospi[32], bf0[3], cos_bit);
        bf1[4] = bf0[4];
        bf1[5] = bf0[5];
        bf1[6] = half_btf(cospi[32], bf0[6], cospi[32], bf0[7], cos_bit);
        bf1[7] = half_btf(cospi[32], bf0[6], -cospi[32], bf0[7], cos_bit);
        bf1[8] = bf0[8];
        bf1[9] = bf0[9];
        bf1[10] = half_btf(cospi[32], bf0[10], cospi[32], bf0[11], cos_bit);
        bf1[11] = half_btf(cospi[32], bf0[10], -cospi[32], bf0[11], cos_bit);
        bf1[12] = bf0[12];
        bf1[13] = bf0[13];
        bf1[14] = half_btf(cospi[32], bf0[14], cospi[32], bf0[15], cos_bit);
        bf1[15] = half_btf(cospi[32], bf0[14], -cospi[32], bf0[15], cos_bit);
    }
    // stage 9
    {
        let bf0: &[i32] = &step;
        let bf1: &mut [i32] = &mut *output;
        bf1[0] = bf0[0];
        bf1[1] = -bf0[8];
        bf1[2] = bf0[12];
        bf1[3] = -bf0[4];
        bf1[4] = bf0[6];
        bf1[5] = -bf0[14];
        bf1[6] = bf0[10];
        bf1[7] = -bf0[2];
        bf1[8] = bf0[3];
        bf1[9] = -bf0[11];
        bf1[10] = bf0[15];
        bf1[11] = -bf0[7];
        bf1[12] = bf0[5];
        bf1[13] = -bf0[13];
        bf1[14] = bf0[9];
        bf1[15] = -bf0[1];
    }
}

pub(crate) fn av1_iidentity8_c(
    input: &[i32],
    output: &mut [i32],
    _cos_bit: i8,
    _stage_range: &[i8],
) {
    for i in 0..8 {
        output[i] = (input[i] as i64 * 2) as i32;
    }
}

pub(crate) fn av1_iidentity16_c(
    input: &[i32],
    output: &mut [i32],
    _cos_bit: i8,
    stage_range: &[i8],
) {
    for i in 0..16 {
        output[i] = round_shift(NEW_SQRT2 as i64 * 2 * input[i] as i64, NEW_SQRT2_BITS);
    }
    debug_assert!(stage_range[0] as i32 + NEW_SQRT2_BITS <= 32);
}
