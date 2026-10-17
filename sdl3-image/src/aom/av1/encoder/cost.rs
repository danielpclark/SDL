// Rust translation of av1/encoder/cost.c and av1/encoder/cost.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Symbol costs (in 1/512 bits) from CDFs.

use crate::aom::aom_dsp::aom_dsp_common::{clamp, get_msb};
use crate::aom::aom_dsp::entenc::EC_MIN_PROB;
use crate::aom::aom_dsp::prob::{aom_icdf, get_prob, AomCdfProb, CDF_PROB_BITS, CDF_PROB_TOP};

// The factor to scale from cost in bits to cost in av1_prob_cost units.
pub(crate) const AV1_PROB_COST_SHIFT: i32 = 9;

// Cost of coding an n bit literal, using 128 (i.e. 50%) probability
// for each bit.
#[inline]
pub(crate) const fn av1_cost_literal(n: i32) -> i32 {
    n * (1 << AV1_PROB_COST_SHIFT)
}

// Calculate the cost of a symbol with probability p15 / 2^15
#[inline]
pub(crate) fn av1_cost_symbol(p15: AomCdfProb) -> i32 {
    // p15 can be out of range [1, CDF_PROB_TOP - 1]. Clamping it, so that the
    // following cost calculation works correctly. Otherwise, if p15 =
    // CDF_PROB_TOP, shift would be -1, and "p15 << shift" would be wrong.
    let p15 = clamp(p15 as i32, 1, CDF_PROB_TOP as i32 - 1) as AomCdfProb;
    debug_assert!(0 < p15 && (p15 as u32) < CDF_PROB_TOP);
    let shift = CDF_PROB_BITS as i32 - 1 - get_msb(p15 as u32);
    let prob = get_prob((p15 as u32) << shift, CDF_PROB_TOP) as i32;
    debug_assert!(prob >= 128);
    AV1_PROB_COST[(prob - 128) as usize] as i32 + av1_cost_literal(shift)
}

// (cost.c)

// round(-log2(i/256.) * (1 << AV1_PROB_COST_SHIFT)); i = 128~255.
pub(crate) static AV1_PROB_COST: [u16; 128] = [
    512, 506, 501, 495, 489, 484, 478, 473, 467, 462, 456, 451, 446, 441, 435, 430, 425, 420, 415,
    410, 405, 400, 395, 390, 385, 380, 375, 371, 366, 361, 356, 352, 347, 343, 338, 333, 329, 324,
    320, 316, 311, 307, 302, 298, 294, 289, 285, 281, 277, 273, 268, 264, 260, 256, 252, 248, 244,
    240, 236, 232, 228, 224, 220, 216, 212, 209, 205, 201, 197, 194, 190, 186, 182, 179, 175, 171,
    168, 164, 161, 157, 153, 150, 146, 143, 139, 136, 132, 129, 125, 122, 119, 115, 112, 109, 105,
    102, 99, 95, 92, 89, 86, 82, 79, 76, 73, 70, 66, 63, 60, 57, 54, 51, 48, 45, 42, 38, 35, 32,
    29, 26, 23, 20, 18, 15, 12, 9, 6, 3,
];

pub(crate) fn av1_cost_tokens_from_cdf(
    costs: &mut [i32],
    cdf: &[AomCdfProb],
    inv_map: Option<&[i32]>,
) {
    let mut prev_cdf: AomCdfProb = 0;
    let mut i = 0usize;
    loop {
        let mut p15 = aom_icdf(cdf[i] as u32).wrapping_sub(prev_cdf);
        p15 = if p15 < EC_MIN_PROB as AomCdfProb {
            EC_MIN_PROB as AomCdfProb
        } else {
            p15
        };
        prev_cdf = aom_icdf(cdf[i] as u32);

        if let Some(inv_map) = inv_map {
            costs[inv_map[i] as usize] = av1_cost_symbol(p15);
        } else {
            costs[i] = av1_cost_symbol(p15);
        }

        // Stop once we reach the end of the CDF
        if cdf[i] == aom_icdf(CDF_PROB_TOP) {
            break;
        }
        i += 1;
    }
}
