// Rust translation of the decoder's part of src/libFLAC/lpc.c from
// libFLAC 1.3.4 (https://xiph.org/flac/, at the revision SDL_mixer's
// external/flac pins).
// Copyright (C) 2000-2009  Josh Coalson
// Copyright (C) 2011-2016  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The LPC signal restoration, in the `FLAC__OVERFLOW_DETECT` versions
//! upstream's RelWithDebInfo build compiles (rather than the unrolled
//! `FLAC__LPC_UNROLLED_FILTER_LOOPS` ones, which compute the same values
//! where nothing overflows). The overflow reports they print to stderr are
//! left out; the wide version's stop at the first overflowing sample is
//! kept. The analysis functions (windowing, autocorrelation, coefficient
//! computation and quantization, residual computation) are the encoder's
//! and aren't translated.

use super::bitmath::FLAC__bitmath_silog2;

/// `FLAC__lpc_restore_signal()`: restores `data_len` samples of `data`
/// from `start` on (C's `data` pointer), the history before it.
///
/// The 32-bit sums wrap where C's signed overflow (undefined; the
/// `FLAC__OVERFLOW_DETECT` build reports it) does on GCC's x86-64 code.
pub(crate) fn FLAC__lpc_restore_signal(
    residual: &[i32],
    data_len: u32,
    qlp_coeff: &[i32],
    order: u32,
    lp_quantization: i32,
    data: &mut [i32],
    start: usize,
) {
    debug_assert!(order > 0);

    for i in 0..data_len as usize {
        let mut sum: i32 = 0;
        let history = start + i;
        for j in 0..order as usize {
            sum = sum.wrapping_add(qlp_coeff[j].wrapping_mul(data[history - j - 1]));
            // (sumo, the 64-bit sum checked for overflow, only feeds the
            // overflow report.)
        }
        data[start + i] = residual[i].wrapping_add(sum >> lp_quantization);
    }

    /* Here's a slower but clearer version:
    for(i = 0; i < data_len; i++) {
        sum = 0;
        for(j = 0; j < order; j++)
            sum += qlp_coeff[j] * data[i-j-1];
        data[i] = residual[i] + (sum >> lp_quantization);
    }
    */
}

/// `FLAC__lpc_restore_signal_wide()`: as [`FLAC__lpc_restore_signal()`],
/// with 64-bit sums; it stops at the first sample whose prediction or
/// value doesn't fit in 32 bits, leaving the rest of `data` as it was.
///
/// FIXME (upstream): what it was is the previous frames' samples, or, in a
/// buffer `allocate_output_()` has just allocated, uninitialized memory
/// (here, zeros); the frame's CRC check zeroes the block only when the
/// CRC doesn't match.
pub(crate) fn FLAC__lpc_restore_signal_wide(
    residual: &[i32],
    data_len: u32,
    qlp_coeff: &[i32],
    order: u32,
    lp_quantization: i32,
    data: &mut [i32],
    start: usize,
) {
    debug_assert!(order > 0);

    for i in 0..data_len as usize {
        let mut sum: i64 = 0;
        let history = start + i;
        for j in 0..order as usize {
            sum = sum.wrapping_add(qlp_coeff[j] as i64 * data[history - j - 1] as i64);
        }
        if FLAC__bitmath_silog2(sum >> lp_quantization) > 32 {
            // (upstream prints "FLAC__lpc_restore_signal_wide: OVERFLOW, ..." to stderr.)
            break;
        }
        if FLAC__bitmath_silog2((residual[i] as i64).wrapping_add(sum >> lp_quantization)) > 32 {
            // (likewise)
            break;
        }
        data[start + i] = residual[i].wrapping_add((sum >> lp_quantization) as i32);
    }
}
