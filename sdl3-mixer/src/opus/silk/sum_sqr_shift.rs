// Rust translation of silk/sum_sqr_shift.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The energy of a vector of 16-bit samples, shifted to fit 32 bits.

use super::sigproc_fix::{silk_add_rshift_uint, silk_clz32};

/* Compute number of bits to right shift the sum of squares of a vector */
/* of int16s to make it fit in an int32                                 */
/// Translation of `silk_sum_sqr_shift()`; returns `(energy, shift)`.
pub(crate) fn silk_sum_sqr_shift(
    x: &[i16], /* I   Input vector                                                 */
    len: i32,  /* I   Length of input vector                                       */
) -> (i32, i32) {
    let sq = |i: usize| (x[i] as i32 * x[i] as i32) as u32;
    /* The two passes of the C code are the same loop with different shifts and start values. */
    let pass = |shft: i32, mut nrg: i32| {
        let mut i = 0usize;
        while (i as i32) < len - 1 {
            let mut nrg_tmp = sq(i);
            nrg_tmp = nrg_tmp.wrapping_add(sq(i + 1));
            nrg = silk_add_rshift_uint(nrg as u32, nrg_tmp, shft) as i32;
            i += 2;
        }
        if (i as i32) < len {
            /* One sample left to process */
            let nrg_tmp = sq(i);
            nrg = silk_add_rshift_uint(nrg as u32, nrg_tmp, shft) as i32;
        }
        nrg
    };

    /* Do a first run with the maximum shift we could have. */
    let mut shft = 31 - silk_clz32(len);
    /* Let's be conservative with rounding and start with nrg=len. */
    let mut nrg = pass(shft, len);
    debug_assert!(nrg >= 0);
    /* Make sure the result will fit in a 32-bit signed integer with two bits
    of headroom. */
    shft = 0.max(shft + 3 - silk_clz32(nrg));
    nrg = pass(shft, 0);

    debug_assert!(nrg >= 0);

    /* Output arguments */
    (nrg, shft)
}
