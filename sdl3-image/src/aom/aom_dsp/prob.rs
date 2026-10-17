// Rust translation of aom_dsp/prob.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CDFs: the inverse-CDF representation, `AOM_CDFn()` (as `aom_cdf()`, a
//! `const fn` over the list of cumulative probabilities), `get_prob()` and
//! the adaptation `update_cdf()`.

pub(crate) type AomCdfProb = u16;

pub(crate) const fn cdf_size(x: usize) -> usize {
    x + 1
}
pub(crate) const CDF_PROB_BITS: u32 = 15;
pub(crate) const CDF_PROB_TOP: u32 = 1 << CDF_PROB_BITS;
pub(crate) const CDF_INIT_TOP: u32 = 32768;
pub(crate) const CDF_SHIFT: u32 = 15 - CDF_PROB_BITS;
/*The value stored in an iCDF is CDF_PROB_TOP minus the actual cumulative
probability (an "inverse" CDF).
This function converts from one representation to the other (and is its own
inverse).*/
pub(crate) const fn aom_icdf(x: u32) -> u16 {
    (CDF_PROB_TOP - x) as u16
}

/// `AOM_CDFn(a0, ..., an-2)` with `CDF_SHIFT == 0`: the inverse CDF of
/// the `N - 1` cumulative probabilities, `AOM_ICDF(CDF_PROB_TOP)` and the
/// zero counter, in an `N + 1` array (`N` is `A + 2`; `A` the input's
/// length).
pub(crate) const fn aom_cdf<const A: usize, const N: usize>(a: [u32; A]) -> [u16; N] {
    let mut out = [0u16; N];
    let mut i = 0;
    while i < A {
        out[i] = aom_icdf(a[i]);
        i += 1;
    }
    out[A] = aom_icdf(CDF_PROB_TOP);
    out
}

/// An `AOM_CDFn(...)` row initializing an `N`-entry CDF array (C pads the
/// rest of the array with zeros): the inverse CDF of the cumulative
/// probabilities, then `AOM_ICDF(CDF_PROB_TOP)` and the counter, zeros.
/// `cdf_pad(&[])` is a row left `{ 0 }`.
pub(crate) const fn cdf_pad<const N: usize>(a: &[u32]) -> [u16; N] {
    let mut out = [0u16; N];
    if a.is_empty() {
        return out;
    }
    let mut i = 0;
    while i < a.len() {
        out[i] = aom_icdf(a[i]);
        i += 1;
    }
    out[a.len()] = aom_icdf(CDF_PROB_TOP);
    out
}

pub(crate) fn get_prob(num: u32, den: u32) -> u8 {
    debug_assert!(den != 0);
    {
        let p = ((num as u64 * 256 + (den >> 1) as u64) / den as u64) as i32;
        // (p > 255) ? 255 : (p < 1) ? 1 : p;
        let clipped_prob = p | ((255 - p) >> 23) | (p == 0) as i32;
        clipped_prob as u8
    }
}

pub(crate) fn update_cdf(cdf: &mut [AomCdfProb], val: i8, nsymbs: i32) {
    debug_assert!(nsymbs < 17);
    let count = cdf[nsymbs as usize] as i32;

    // rate is computed in the spec as:
    //  3 + ( cdf[N] > 15 ) + ( cdf[N] > 31 ) + Min(FloorLog2(N), 2)
    // In this case cdf[N] is |count|.
    // Min(FloorLog2(N), 2) is 1 for nsymbs == {2, 3} and 2 for all
    // nsymbs > 3. So the equation becomes:
    //  4 + (count > 15) + (count > 31) + (nsymbs > 3).
    // Note that the largest value for count is 32 (it is not incremented beyond
    // 32). So using that information:
    //  count >> 4 is 0 for count from 0 to 15.
    //  count >> 4 is 1 for count from 16 to 31.
    //  count >> 4 is 2 for count == 31.
    // Now, the equation becomes:
    //  4 + (count >> 4) + (nsymbs > 3).
    let rate = 4 + (count >> 4) + (nsymbs > 3) as i32;

    let mut i = 0;
    loop {
        if i < val as i32 {
            cdf[i as usize] += ((CDF_PROB_TOP - cdf[i as usize] as u32) >> rate) as u16;
        } else {
            cdf[i as usize] -= cdf[i as usize] >> rate;
        }
        i += 1;
        if i >= nsymbs - 1 {
            break;
        }
    }
    cdf[nsymbs as usize] += (count < 32) as u16;
}
