// Rust translation of aom_dsp/avg.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The averages, Hadamard transforms and SATD the real-time-style mode
//! decision and partitioning reach (high bit depth: no `_lp` versions).

use super::aom_dsp_common::TranLowT;

/// Translation of `aom_highbd_avg_4x4_c()`.
pub(crate) fn aom_highbd_avg_4x4(s: &[u16], s_off: usize, p: usize) -> u32 {
    let mut sum: i32 = 0;
    let mut o = s_off;
    for _i in 0..4 {
        for j in 0..4 {
            sum += s[o + j] as i32;
        }
        o += p;
    }

    ((sum + 8) >> 4) as u32
}

/// Translation of `aom_highbd_avg_8x8_c()`.
pub(crate) fn aom_highbd_avg_8x8(s: &[u16], s_off: usize, p: usize) -> u32 {
    let mut sum: i32 = 0;
    let mut o = s_off;
    for _i in 0..8 {
        for j in 0..8 {
            sum += s[o + j] as i32;
        }
        o += p;
    }

    ((sum + 32) >> 6) as u32
}

/// Translation of `aom_highbd_minmax_8x8_c()`.
pub(crate) fn aom_highbd_minmax_8x8(
    s: &[u16],
    s_off: usize,
    p: usize,
    d: &[u16],
    d_off: usize,
    dp: usize,
    min: &mut i32,
    max: &mut i32,
) {
    *min = 65535;
    *max = 0;
    let (mut so, mut dof) = (s_off, d_off);
    for _i in 0..8 {
        for j in 0..8 {
            let diff = (s[so + j] as i32 - d[dof + j] as i32).abs();
            *min = if diff < *min { diff } else { *min };
            *max = if diff > *max { diff } else { *max };
        }
        so += p;
        dof += dp;
    }
}

// src_diff: first pass, 9 bit, dynamic range [-255, 255]
//           second pass, 12 bit, dynamic range [-2040, 2040]
// (the int16_t intermediates wrap as upstream's do)
fn hadamard_col8(src_diff: &[i16], src_off: usize, src_stride: usize, coeff: &mut [i16]) {
    let s = |k: usize| src_diff[src_off + k * src_stride] as i32;
    let b0 = (s(0) + s(1)) as i16 as i32;
    let b1 = (s(0) - s(1)) as i16 as i32;
    let b2 = (s(2) + s(3)) as i16 as i32;
    let b3 = (s(2) - s(3)) as i16 as i32;
    let b4 = (s(4) + s(5)) as i16 as i32;
    let b5 = (s(4) - s(5)) as i16 as i32;
    let b6 = (s(6) + s(7)) as i16 as i32;
    let b7 = (s(6) - s(7)) as i16 as i32;

    let c0 = (b0 + b2) as i16 as i32;
    let c1 = (b1 + b3) as i16 as i32;
    let c2 = (b0 - b2) as i16 as i32;
    let c3 = (b1 - b3) as i16 as i32;
    let c4 = (b4 + b6) as i16 as i32;
    let c5 = (b5 + b7) as i16 as i32;
    let c6 = (b4 - b6) as i16 as i32;
    let c7 = (b5 - b7) as i16 as i32;

    coeff[0] = (c0 + c4) as i16;
    coeff[7] = (c1 + c5) as i16;
    coeff[3] = (c2 + c6) as i16;
    coeff[4] = (c3 + c7) as i16;
    coeff[2] = (c0 - c4) as i16;
    coeff[6] = (c1 - c5) as i16;
    coeff[1] = (c2 - c6) as i16;
    coeff[5] = (c3 - c7) as i16;
}

/// Translation of `aom_hadamard_8x8_c()`.
pub(crate) fn aom_hadamard_8x8(
    src_diff: &[i16],
    src_off: usize,
    src_stride: usize,
    coeff: &mut [TranLowT],
) {
    let mut buffer = [0i16; 64];
    let mut buffer2 = [0i16; 64];
    for idx in 0..8 {
        hadamard_col8(src_diff, src_off + idx, src_stride, &mut buffer[idx * 8..]);
        // src_diff: 9 bit
        // dynamic range [-255, 255]
    }

    for idx in 0..8 {
        hadamard_col8(&buffer, idx, 8, &mut buffer2[8 * idx..]); // tmp_buf: 12 bit
                                                                 // dynamic range [-2040, 2040]
                                                                 // buffer2: 15 bit
                                                                 // dynamic range [-16320, 16320]
    }

    // Extra transpose to match SSE2 behavior(i.e., aom_hadamard_8x8_sse2).
    for i in 0..8 {
        for j in 0..8 {
            coeff[i * 8 + j] = buffer2[j * 8 + i] as TranLowT;
        }
    }
}

// In place 16x16 2D Hadamard transform
/// Translation of `aom_hadamard_16x16_c()`.
pub(crate) fn aom_hadamard_16x16(
    src_diff: &[i16],
    src_off: usize,
    src_stride: usize,
    coeff: &mut [TranLowT],
) {
    for idx in 0..4 {
        // src_diff: 9 bit, dynamic range [-255, 255]
        let src_ptr = src_off + (idx >> 1) * 8 * src_stride + (idx & 0x01) * 8;
        aom_hadamard_8x8(src_diff, src_ptr, src_stride, &mut coeff[idx * 64..]);
    }

    // coeff: 15 bit, dynamic range [-16320, 16320]
    for idx in 0..64 {
        let a0 = coeff[idx];
        let a1 = coeff[idx + 64];
        let a2 = coeff[idx + 128];
        let a3 = coeff[idx + 192];

        let b0 = (a0 + a1) >> 1; // (a0 + a1): 16 bit, [-32640, 32640]
        let b1 = (a0 - a1) >> 1; // b0-b3: 15 bit, dynamic range
        let b2 = (a2 + a3) >> 1; // [-16320, 16320]
        let b3 = (a2 - a3) >> 1;

        coeff[idx] = b0 + b2; // 16 bit, [-32640, 32640]
        coeff[idx + 64] = b1 + b3;
        coeff[idx + 128] = b0 - b2;
        coeff[idx + 192] = b1 - b3;
    }

    // Extra shift to match AVX2 output (i.e., aom_hadamard_16x16_avx2).
    // Note that to match SSE2 output, it does not need this step.
    for i in 0..16 {
        for j in 0..4 {
            let temp = coeff[i * 16 + 4 + j];
            coeff[i * 16 + 4 + j] = coeff[i * 16 + 8 + j];
            coeff[i * 16 + 8 + j] = temp;
        }
    }
}

// coeff: 16 bits, dynamic range [-32640, 32640].
// length: value range {16, 64, 256, 1024}.
/// Translation of `aom_satd_c()`.
pub(crate) fn aom_satd(coeff: &[TranLowT], length: i32) -> i32 {
    let mut satd: i32 = 0;
    for i in 0..length as usize {
        satd += coeff[i].abs();
    }

    // satd: 26 bits, dynamic range [-32640 * 1024, 32640 * 1024]
    satd
}
