// Rust translation of silk/LPC_analysis_filter.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The LPC analysis filter (the plain C loop; `USE_CELT_FIR` is 0 and only
//! the fixed-point build could use it anyway).

use super::sigproc_fix::{
    silk_lshift, silk_rshift_round, silk_sat16, silk_smlabb_ovflw, silk_smulbb, silk_sub32_ovflw,
};

/*******************************************/
/* LPC analysis filter                     */
/* NB! State is kept internally and the    */
/* filter always starts with zero state    */
/* first d output samples are set to zero  */
/*******************************************/

/// Translation of `silk_LPC_analysis_filter()`.
pub(crate) fn silk_lpc_analysis_filter(
    out: &mut [i16], /* O    Output signal                                               */
    in_: &[i16],     /* I    Input signal                                                */
    b: &[i16],       /* I    MA prediction coefficients, Q12 [order]                     */
    len: i32,        /* I    Signal length                                               */
    d: i32,          /* I    Filter order                                                */
) {
    debug_assert!(d >= 6);
    debug_assert!((d & 1) == 0);
    debug_assert!(d <= len);

    let d = d as usize;
    for ix in d..len as usize {
        let in_ptr = |k: usize| in_[ix - 1 - k] as i32;

        let mut out32_q12 = silk_smulbb(in_ptr(0), b[0] as i32);
        /* Allowing wrap around so that two wraps can cancel each other. The rare
        cases where the result wraps around can only be triggered by invalid streams*/
        out32_q12 = silk_smlabb_ovflw(out32_q12, in_ptr(1), b[1] as i32);
        out32_q12 = silk_smlabb_ovflw(out32_q12, in_ptr(2), b[2] as i32);
        out32_q12 = silk_smlabb_ovflw(out32_q12, in_ptr(3), b[3] as i32);
        out32_q12 = silk_smlabb_ovflw(out32_q12, in_ptr(4), b[4] as i32);
        out32_q12 = silk_smlabb_ovflw(out32_q12, in_ptr(5), b[5] as i32);
        let mut j = 6;
        while j < d {
            out32_q12 = silk_smlabb_ovflw(out32_q12, in_ptr(j), b[j] as i32);
            out32_q12 = silk_smlabb_ovflw(out32_q12, in_ptr(j + 1), b[j + 1] as i32);
            j += 2;
        }

        /* Subtract prediction */
        out32_q12 = silk_sub32_ovflw(silk_lshift(in_[ix] as i32, 12), out32_q12);

        /* Scale to Q0 */
        let out32 = silk_rshift_round(out32_q12, 12);

        /* Saturate output */
        out[ix] = silk_sat16(out32) as i16;
    }

    /* Set first d output samples to zero */
    out[..d].fill(0);
}
