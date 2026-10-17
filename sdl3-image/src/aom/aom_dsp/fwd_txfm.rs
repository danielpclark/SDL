// Rust translation of aom_dsp/fwd_txfm.c and the constants of
// aom_dsp/txfm_common.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The 4x4 DCT of the real-time-style RD estimate (high bit depth uses
//! `aom_fdct4x4_c()`; the `_lp` version is the low bit depth one).

use super::aom_dsp_common::{TranHighT, TranLowT};

// Constants and Macros used by all idct/dct functions
const DCT_CONST_BITS: i32 = 14;

const COSPI_8_64: TranHighT = 15137;
const COSPI_16_64: TranHighT = 11585;
const COSPI_24_64: TranHighT = 6270;

#[inline]
fn fdct_round_shift(input: TranHighT) -> TranHighT {
    let rv = (input + ((1i32 << DCT_CONST_BITS) >> 1) as i64) >> DCT_CONST_BITS;
    rv
}

/// Translation of `aom_fdct4x4_c()`.
pub(crate) fn aom_fdct4x4(input: &[i16], input_off: usize, output: &mut [TranLowT], stride: usize) {
    // The 2D transform is done with two passes which are actually pretty
    // similar. In the first one, we transform the columns and transpose
    // the results. In the second one, we transform the rows. To achieve that,
    // as the first pass results are transposed, we transpose the columns (that
    // is the transposed rows) and transpose the results (so that it goes back
    // in normal/row positions).
    // We need an intermediate buffer between passes.
    let mut intermediate: [TranLowT; 4 * 4] = [0; 16];
    // Do the two transform/transpose passes
    for pass in 0..2 {
        let mut in_high: [TranHighT; 4] = [0; 4]; // canbe16
        let mut step: [TranHighT; 4] = [0; 4]; // canbe16
        let mut temp1: TranHighT;
        let mut temp2: TranHighT; // needs32
        let mut inp = input_off;
        for i in 0..4 {
            // Load inputs.
            if pass == 0 {
                in_high[0] = input[inp] as TranHighT * 16;
                in_high[1] = input[inp + stride] as TranHighT * 16;
                in_high[2] = input[inp + 2 * stride] as TranHighT * 16;
                in_high[3] = input[inp + 3 * stride] as TranHighT * 16;
                if i == 0 && in_high[0] != 0 {
                    in_high[0] += 1;
                }
            } else {
                in_high[0] = intermediate[i] as TranHighT;
                in_high[1] = intermediate[i + 4] as TranHighT;
                in_high[2] = intermediate[i + 2 * 4] as TranHighT;
                in_high[3] = intermediate[i + 3 * 4] as TranHighT;
            }
            // Transform.
            step[0] = in_high[0] + in_high[3];
            step[1] = in_high[1] + in_high[2];
            step[2] = in_high[1] - in_high[2];
            step[3] = in_high[0] - in_high[3];
            temp1 = (step[0] + step[1]) * COSPI_16_64;
            temp2 = (step[0] - step[1]) * COSPI_16_64;
            let out: &mut [TranLowT] = if pass == 0 {
                &mut intermediate[i * 4..]
            } else {
                &mut output[i * 4..]
            };
            out[0] = fdct_round_shift(temp1) as TranLowT;
            out[2] = fdct_round_shift(temp2) as TranLowT;
            temp1 = step[2] * COSPI_24_64 + step[3] * COSPI_8_64;
            temp2 = -step[2] * COSPI_8_64 + step[3] * COSPI_24_64;
            out[1] = fdct_round_shift(temp1) as TranLowT;
            out[3] = fdct_round_shift(temp2) as TranLowT;
            // Do next column (which is a transposed row in second/horizontal pass)
            inp += 1;
        }
        // Setup in/out for next pass.
    }

    for i in 0..4 {
        for j in 0..4 {
            output[j + i * 4] = (output[j + i * 4] + 1) >> 2;
        }
    }
}
