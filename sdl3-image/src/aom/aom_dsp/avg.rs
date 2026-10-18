// Rust translation of aom_dsp/avg.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The averages, Hadamard transforms and SATD the real-time mode decision
//! and partitioning reach (the samples are kept in 16 bits for any bit
//! depth, so the low bit depth averages are the high bit depth ones').

use super::aom_dsp_common::TranLowT;

/// Translation of `aom_avg_8x8_quad_c()`.
pub(crate) fn aom_avg_8x8_quad(
    s: &[u16],
    s_off: usize,
    p: usize,
    x16_idx: i32,
    y16_idx: i32,
    avg: &mut [i32; 4],
) {
    for (k, a) in avg.iter_mut().enumerate() {
        let x8_idx = x16_idx + ((k as i32 & 1) << 3);
        let y8_idx = y16_idx + ((k as i32 >> 1) << 3);
        let s_tmp = (s_off as i64 + y8_idx as i64 * p as i64 + x8_idx as i64) as usize;
        *a = aom_avg_8x8(s, s_tmp, p) as i32;
    }
}

/// Translation of `aom_avg_8x8_c()`.
pub(crate) fn aom_avg_8x8(s: &[u16], s_off: usize, p: usize) -> u32 {
    aom_highbd_avg_8x8(s, s_off, p)
}

/// Translation of `aom_avg_4x4_c()`.
pub(crate) fn aom_avg_4x4(s: &[u16], s_off: usize, p: usize) -> u32 {
    aom_highbd_avg_4x4(s, s_off, p)
}

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

/// Translation of `aom_hadamard_lp_8x8_c()`.
pub(crate) fn aom_hadamard_lp_8x8(
    src_diff: &[i16],
    src_off: usize,
    src_stride: usize,
    coeff: &mut [i16],
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

    coeff[..64].copy_from_slice(&buffer2);

    // Extra transpose to match SSE2 behavior(i.e., aom_hadamard_lp_8x8_sse2).
    for i in 0..8 {
        for j in 0..8 {
            coeff[i * 8 + j] = buffer2[j * 8 + i];
        }
    }
}

/// Translation of `aom_hadamard_lp_8x8_dual_c()`.
pub(crate) fn aom_hadamard_lp_8x8_dual(
    src_diff: &[i16],
    src_off: usize,
    src_stride: usize,
    coeff: &mut [i16],
) {
    for i in 0..2 {
        aom_hadamard_lp_8x8(src_diff, src_off + i * 8, src_stride, &mut coeff[i * 64..]);
    }
}

/// Translation of `aom_hadamard_lp_16x16_c()` (the int16_t intermediates
/// wrap as upstream's do).
pub(crate) fn aom_hadamard_lp_16x16(
    src_diff: &[i16],
    src_off: usize,
    src_stride: usize,
    coeff: &mut [i16],
) {
    for idx in 0..4 {
        // src_diff: 9 bit, dynamic range [-255, 255]
        let src_ptr = src_off + (idx >> 1) * 8 * src_stride + (idx & 0x01) * 8;
        aom_hadamard_lp_8x8(src_diff, src_ptr, src_stride, &mut coeff[idx * 64..]);
    }

    for idx in 0..64 {
        let a0 = coeff[idx] as i32;
        let a1 = coeff[idx + 64] as i32;
        let a2 = coeff[idx + 128] as i32;
        let a3 = coeff[idx + 192] as i32;

        let b0 = ((a0 + a1) >> 1) as i16 as i32; // (a0 + a1): 16 bit, [-32640, 32640]
        let b1 = ((a0 - a1) >> 1) as i16 as i32; // b0-b3: 15 bit, dynamic range
        let b2 = ((a2 + a3) >> 1) as i16 as i32; // [-16320, 16320]
        let b3 = ((a2 - a3) >> 1) as i16 as i32;

        coeff[idx] = (b0 + b2) as i16; // 16 bit, [-32640, 32640]
        coeff[idx + 64] = (b1 + b3) as i16;
        coeff[idx + 128] = (b0 - b2) as i16;
        coeff[idx + 192] = (b1 - b3) as i16;
    }
}

/// Translation of `aom_satd_lp_c()`.
pub(crate) fn aom_satd_lp(coeff: &[i16], length: i32) -> i32 {
    let mut satd: i32 = 0;
    for i in 0..length as usize {
        satd += (coeff[i] as i32).abs();
    }

    // satd: 26 bits, dynamic range [-32640 * 1024, 32640 * 1024]
    satd
}

/// Translation of `aom_int_pro_row_c()`: `ref_` from `ref_off`.
pub(crate) fn aom_int_pro_row(
    hbuf: &mut [i16],
    ref_: &[u16],
    ref_off: usize,
    ref_stride: usize,
    width: i32,
    height: i32,
    norm_factor: i32,
) {
    debug_assert!(height >= 2);
    for idx in 0..width as usize {
        hbuf[idx] = 0;
        // hbuf[idx]: 14 bit, dynamic range [0, 32640].
        for i in 0..height as usize {
            hbuf[idx] = hbuf[idx].wrapping_add(ref_[ref_off + idx + i * ref_stride] as i16);
        }
        // hbuf[idx]: 9 bit, dynamic range [0, 1020].
        hbuf[idx] >>= norm_factor;
    }
}

// width: value range {16, 32, 64, 128}.
/// Translation of `aom_int_pro_col_c()`: `ref_` from `ref_off`.
pub(crate) fn aom_int_pro_col(
    vbuf: &mut [i16],
    ref_: &[u16],
    ref_off: usize,
    ref_stride: usize,
    width: i32,
    height: i32,
    norm_factor: i32,
) {
    let mut o = ref_off;
    for ht in 0..height as usize {
        let mut sum: i16 = 0;
        // sum: 14 bit, dynamic range [0, 32640]
        for idx in 0..width as usize {
            sum = sum.wrapping_add(ref_[o + idx] as i16);
        }
        vbuf[ht] = sum >> norm_factor;
        o += ref_stride;
    }
}

// ref: [0 - 510]
// src: [0 - 510]
// bwl: {2, 3, 4, 5}
/// Translation of `aom_vector_var_c()`.
pub(crate) fn aom_vector_var(ref_: &[i16], src: &[i16], bwl: i32) -> i32 {
    let width = 4usize << bwl;
    let mut sse: i32 = 0;
    let mut mean: i32 = 0;

    for i in 0..width {
        let diff = ref_[i] as i32 - src[i] as i32; // diff: dynamic range [-510, 510], 10 bits.
        mean += diff; // mean: dynamic range 16 bits.
        sse += diff * diff; // sse:  dynamic range 26 bits.
    }

    // (mean * mean): dynamic range 31 bits.
    // If width == 128, the mean can be 510 * 128 = 65280, and log2(65280 ** 2) ~=
    // 31.99, so it needs to be casted to unsigned int to compute its square.
    let mean_abs = mean.unsigned_abs();
    sse.wrapping_sub((mean_abs.wrapping_mul(mean_abs) >> (bwl + 2)) as i32)
}
