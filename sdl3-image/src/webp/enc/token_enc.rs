// Rust translation of src/enc/token_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Paginated token buffer
//!
//!  A 'token' is a bit value associated with a probability, either fixed
//! or a later-to-be-determined after statistics have been collected.
//! For dynamic probability, we just record the slot id (idx) for the probability
//! value in the final probability array (uint8_t* probas in VP8EmitTokens).
//!
//! The pages are one vector here: upstream fills each page from its end
//! and emits it backward, which is the order the tokens were added in.
//! (`VP8EstimateTokenSize()` serves the size search only, and isn't
//! translated.)

use crate::webp::dec::{NUM_BANDS, NUM_CTX, NUM_PROBAS};
use crate::webp::enc::cost_enc::{enc_band, vp8_record_stats, VP8Residual};
use crate::webp::enc::frame_enc::{VP8_CAT3, VP8_CAT4, VP8_CAT5, VP8_CAT6};
use crate::webp::enc::vp8i_enc::{ProbaT, StatsArray, VP8TBuffer};
use crate::webp::utils::bit_writer_utils::VP8BitWriter;

/// minimum number of token per page
const MIN_PAGE_SIZE: i32 = 8192;
const FIXED_PROBA_BIT: u32 = 1 << 14;

// token_t: bit #15: bit value
//          bit #14: flags for constant proba or idx
//          bits #0..13: slot or constant proba

//------------------------------------------------------------------------------

/// initialize an empty buffer. Translation of `VP8TBufferInit()`.
pub(crate) fn vp8_tbuffer_init(b: &mut VP8TBuffer, page_size: i32) {
    b.tokens = Vec::new();
    b.page_size = if page_size < MIN_PAGE_SIZE {
        MIN_PAGE_SIZE
    } else {
        page_size
    };
    b.error = false;
}

/// de-allocate pages memory. Translation of `VP8TBufferClear()`.
pub(crate) fn vp8_tbuffer_clear(b: &mut VP8TBuffer) {
    let page_size = b.page_size;
    vp8_tbuffer_init(b, page_size);
}

/// Translation of `TBufferNewPage()`: room for one more page of tokens.
fn tbuffer_new_page(b: &mut VP8TBuffer) -> bool {
    if b.error || b.tokens.try_reserve(b.page_size as usize).is_err() {
        b.error = true;
        return false;
    }
    true
}

//------------------------------------------------------------------------------

/// Translation of the `TOKEN_ID()` macro.
fn token_id(t: usize, b: usize, ctx: usize) -> u32 {
    (NUM_PROBAS * (ctx + NUM_CTX * (b + NUM_BANDS * t))) as u32
}

/// Translation of `AddToken()`.
fn add_token(b: &mut VP8TBuffer, bit: bool, proba_idx: u32, stats: &mut ProbaT) -> bool {
    debug_assert!(proba_idx < FIXED_PROBA_BIT);
    if b.tokens.len() < b.tokens.capacity() || tbuffer_new_page(b) {
        b.tokens.push(((bit as u32) << 15 | proba_idx) as u16);
    }
    vp8_record_stats(bit, stats);
    bit
}

/// Translation of `AddConstantToken()`.
fn add_constant_token(b: &mut VP8TBuffer, bit: bool, proba: u32) {
    debug_assert!(proba < 256);
    if b.tokens.len() < b.tokens.capacity() || tbuffer_new_page(b) {
        b.tokens
            .push(((bit as u32) << 15 | FIXED_PROBA_BIT | proba) as u16);
    }
}

/// record the coding of coefficients without knowing the probabilities yet
/// Translation of `VP8RecordCoeffTokens()`: `stats` is the encoder's
/// `stats_[res->coeff_type]`.
pub(crate) fn vp8_record_coeff_tokens(
    ctx: usize,
    res: &VP8Residual<'_>,
    stats: &mut [StatsArray; NUM_BANDS],
    tokens: &mut VP8TBuffer,
) -> i32 {
    let coeffs = res.coeffs;
    let coeff_type = res.coeff_type as usize;
    let last = res.last;
    let mut n = res.first as usize;
    let mut base_id = token_id(coeff_type, n, ctx);
    // should be stats[VP8EncBands[n]], but it's equivalent for n=0 or 1
    let (mut sb, mut sc) = (n, ctx);
    if !add_token(tokens, last >= 0, base_id, &mut stats[sb][sc][0]) {
        return 0;
    }

    while n < 16 {
        let c = coeffs[n] as i32;
        n += 1;
        let sign = c < 0;
        let v = c.unsigned_abs();
        if !add_token(tokens, v != 0, base_id + 1, &mut stats[sb][sc][1]) {
            base_id = token_id(coeff_type, enc_band(n), 0); // ctx=0
            (sb, sc) = (enc_band(n), 0);
            continue;
        }
        if !add_token(tokens, v > 1, base_id + 2, &mut stats[sb][sc][2]) {
            base_id = token_id(coeff_type, enc_band(n), 1); // ctx=1
            (sb, sc) = (enc_band(n), 1);
        } else {
            let s = &mut stats[sb][sc];
            if !add_token(tokens, v > 4, base_id + 3, &mut s[3]) {
                if add_token(tokens, v != 2, base_id + 4, &mut s[4]) {
                    add_token(tokens, v == 4, base_id + 5, &mut s[5]);
                }
            } else if !add_token(tokens, v > 10, base_id + 6, &mut s[6]) {
                if !add_token(tokens, v > 6, base_id + 7, &mut s[7]) {
                    add_constant_token(tokens, v == 6, 159);
                } else {
                    add_constant_token(tokens, v >= 9, 165);
                    add_constant_token(tokens, v & 1 == 0, 145);
                }
            } else {
                let mut mask: u32;
                let tab: &[u8];
                let mut residue = v - 3;
                if residue < (8 << 1) {
                    // VP8Cat3  (3b)
                    add_token(tokens, false, base_id + 8, &mut s[8]);
                    add_token(tokens, false, base_id + 9, &mut s[9]);
                    residue -= 8; // (8 << 0)
                    mask = 1 << 2;
                    tab = &VP8_CAT3;
                } else if residue < (8 << 2) {
                    // VP8Cat4  (4b)
                    add_token(tokens, false, base_id + 8, &mut s[8]);
                    add_token(tokens, true, base_id + 9, &mut s[9]);
                    residue -= 8 << 1;
                    mask = 1 << 3;
                    tab = &VP8_CAT4;
                } else if residue < (8 << 3) {
                    // VP8Cat5  (5b)
                    add_token(tokens, true, base_id + 8, &mut s[8]);
                    // FIXME (upstream): the statistics of proba #10 are
                    // recorded in those of #9 (s + 9).
                    add_token(tokens, false, base_id + 10, &mut s[9]);
                    residue -= 8 << 2;
                    mask = 1 << 4;
                    tab = &VP8_CAT5;
                } else {
                    // VP8Cat6 (11b)
                    add_token(tokens, true, base_id + 8, &mut s[8]);
                    // FIXME (upstream): as above, s + 9 for proba #10.
                    add_token(tokens, true, base_id + 10, &mut s[9]);
                    residue -= 8 << 3;
                    mask = 1 << 10;
                    tab = &VP8_CAT6;
                }
                let mut t = 0;
                while mask != 0 {
                    add_constant_token(tokens, residue & mask != 0, tab[t] as u32);
                    t += 1;
                    mask >>= 1;
                }
            }
            base_id = token_id(coeff_type, enc_band(n), 2); // ctx=2
            (sb, sc) = (enc_band(n), 2);
        }
        add_constant_token(tokens, sign, 128);
        if n == 16 || !add_token(tokens, n as i32 <= last, base_id, &mut stats[sb][sc][0]) {
            return 1; // EOB
        }
    }
    1
}

//------------------------------------------------------------------------------
// Final coding pass, with known probabilities

/// Finalizes bitstream when probabilities are known.
/// Deletes the allocated token memory if final_pass is true.
/// Translation of `VP8EmitTokens()`: `probas` is the flattened
/// `coeffs_` array.
pub(crate) fn vp8_emit_tokens(
    b: &mut VP8TBuffer,
    bw: &mut VP8BitWriter,
    probas: &[u8],
    final_pass: bool,
) -> bool {
    debug_assert!(!b.error);
    for &token in &b.tokens {
        let bit = (token >> 15) & 1 != 0;
        if token as u32 & FIXED_PROBA_BIT != 0 {
            bw.put_bit(bit, (token & 0xff) as i32); // constant proba
        } else {
            bw.put_bit(bit, probas[(token & 0x3fff) as usize] as i32);
        }
    }
    if final_pass {
        b.tokens = Vec::new();
    }
    true
}
