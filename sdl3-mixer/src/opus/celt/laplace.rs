// Rust translation of celt/laplace.h and celt/laplace.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2007 CSIRO
// Copyright (c) 2007-2009 Xiph.Org Foundation
// Written by Jean-Marc Valin
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Laplace-like distribution of the coarse energy (the decoding half;
//! `ec_laplace_encode()` isn't translated).

use super::entcode::EcDec;

/* The minimum probability of an energy delta (out of 32768). */
const LAPLACE_LOG_MINP: u32 = 0;
const LAPLACE_MINP: u32 = 1 << LAPLACE_LOG_MINP;
/* The minimum number of guaranteed representable energy deltas (in one
direction). */
const LAPLACE_NMIN: u32 = 16;

/* When called, decay is positive and at most 11456. */
fn ec_laplace_get_freq1(fs0: u32, decay: i32) -> u32 {
    let ft = 32768 - LAPLACE_MINP * (2 * LAPLACE_NMIN) - fs0;
    ft.wrapping_mul((16384 - decay) as u32) >> 15
}

/* Decode a value that is assumed to be the realisation of a
Laplace-distributed random process
@param dec Range decoder
@param decay Probability of the value +/- 1, multiplied by 16384
@return Value decoded
*/
/// Translation of `ec_laplace_decode()`.
pub(crate) fn ec_laplace_decode(dec: &mut EcDec<'_>, mut fs: u32, decay: i32) -> i32 {
    let mut val: i32 = 0;
    let fm = dec.ec_decode_bin(15);
    let mut fl: u32 = 0;
    if fm >= fs {
        val += 1;
        fl = fs;
        fs = ec_laplace_get_freq1(fs, decay) + LAPLACE_MINP;
        /* Search the decaying part of the PDF.*/
        while fs > LAPLACE_MINP && fm >= fl + 2 * fs {
            fs *= 2;
            fl += fs;
            fs = (fs - 2 * LAPLACE_MINP).wrapping_mul(decay as u32) >> 15;
            fs += LAPLACE_MINP;
            val += 1;
        }
        /* Everything beyond that has probability LAPLACE_MINP. */
        if fs <= LAPLACE_MINP {
            let di = ((fm - fl) >> (LAPLACE_LOG_MINP + 1)) as i32;
            val += di;
            fl += 2 * di as u32 * LAPLACE_MINP;
        }
        if fm < fl + fs {
            val = -val;
        } else {
            fl += fs;
        }
    }
    debug_assert!(fl < 32768);
    debug_assert!(fs > 0);
    debug_assert!(fl <= fm);
    debug_assert!(fm < (fl + fs).min(32768));
    dec.ec_dec_update(fl, (fl + fs).min(32768), 32768);
    val
}
