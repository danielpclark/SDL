// Rust translation of src/enc/tree_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Coding of token probabilities, intra modes and segments. The default
//! and update probabilities (`VP8CoeffsProba0`, `VP8CoeffsUpdateProba`)
//! and `kBModesProba` are the decoder's tables, which they equal.

use crate::webp::dec::tree_dec::{COEFFS_PROBA0, COEFFS_UPDATE_PROBA, K_B_MODES_PROBA};
use crate::webp::dec::{
    B_DC_PRED, B_HD_PRED, B_HE_PRED, B_LD_PRED, B_RD_PRED, B_TM_PRED, B_VE_PRED, B_VL_PRED,
    DC_PRED, H_PRED, NUM_BANDS, NUM_CTX, NUM_PROBAS, NUM_TYPES, TM_PRED, V_PRED,
};
use crate::webp::enc::iterator_enc::{vp8_iterator_init, vp8_iterator_next};
use crate::webp::enc::vp8i_enc::{VP8EncProba, VP8Encoder};
use crate::webp::utils::bit_writer_utils::VP8BitWriter;

//------------------------------------------------------------------------------
// Default probabilities

/// Reset the token probabilities to their initial (default) values
/// Translation of `VP8DefaultProbas()`.
pub(crate) fn vp8_default_probas(enc: &mut VP8Encoder<'_>) {
    let probas = &mut enc.proba;
    probas.use_skip_proba = false;
    probas.segments = [255; 3];
    probas.coeffs = COEFFS_PROBA0;
    // Note: we could hard-code the level_costs_ corresponding to VP8CoeffsProba0,
    // but that's ~11k of static data. Better call VP8CalculateLevelCosts() later.
    probas.dirty = true;
}

/// Translation of `PutI4Mode()`.
fn put_i4_mode(bw: &mut VP8BitWriter, mode: i32, prob: &[u8]) -> i32 {
    let p = |i: usize| prob[i] as i32;
    if bw.put_bit(mode != B_DC_PRED, p(0))
        && bw.put_bit(mode != B_TM_PRED, p(1))
        && bw.put_bit(mode != B_VE_PRED, p(2))
    {
        if !bw.put_bit(mode >= B_LD_PRED, p(3)) {
            if bw.put_bit(mode != B_HE_PRED, p(4)) {
                bw.put_bit(mode != B_RD_PRED, p(5));
            }
        } else if bw.put_bit(mode != B_LD_PRED, p(6)) && bw.put_bit(mode != B_VL_PRED, p(7)) {
            bw.put_bit(mode != B_HD_PRED, p(8));
        }
    }
    mode
}

/// Translation of `PutI16Mode()`.
fn put_i16_mode(bw: &mut VP8BitWriter, mode: i32) {
    if bw.put_bit(mode == TM_PRED || mode == H_PRED, 156) {
        bw.put_bit(mode == TM_PRED, 128); // TM or HE
    } else {
        bw.put_bit(mode == V_PRED, 163); // VE or DC
    }
}

/// Translation of `PutUVMode()`.
fn put_uv_mode(bw: &mut VP8BitWriter, uv_mode: i32) {
    if bw.put_bit(uv_mode != DC_PRED, 142) && bw.put_bit(uv_mode != V_PRED, 114) {
        bw.put_bit(uv_mode != H_PRED, 183); // else: TM_PRED
    }
}

/// Translation of `PutSegment()`.
fn put_segment(bw: &mut VP8BitWriter, s: i32, p: &[u8; 3]) {
    let mut p0 = 0;
    if bw.put_bit(s >= 2, p[0] as i32) {
        p0 += 1;
    }
    bw.put_bit(s & 1 != 0, p[p0 + 1] as i32);
}

/// Writes the partition #0 modes (that is: all intra modes)
/// Translation of `VP8CodeIntraModes()`.
pub(crate) fn vp8_code_intra_modes(enc: &mut VP8Encoder<'_>) {
    let mut it = vp8_iterator_init(enc);
    loop {
        let mb = enc.mb_info[it.mb];
        let mut preds = it.preds;
        let bw = &mut enc.bw;
        if enc.segment_hdr.update_map {
            put_segment(bw, mb.segment as i32, &enc.proba.segments);
        }
        if enc.proba.use_skip_proba {
            bw.put_bit(mb.skip, enc.proba.skip_proba as i32);
        }
        if bw.put_bit(mb.type_ != 0, 145) {
            // i16x16
            put_i16_mode(bw, enc.preds[preds] as i32);
        } else {
            let preds_w = enc.preds_w as usize;
            let mut top_pred = preds - preds_w;
            for _ in 0..4 {
                let mut left = enc.preds[preds - 1] as usize;
                for x in 0..4 {
                    let probas = &K_B_MODES_PROBA[enc.preds[top_pred + x] as usize][left];
                    left = put_i4_mode(bw, enc.preds[preds + x] as i32, probas) as usize;
                }
                top_pred = preds;
                preds += preds_w;
            }
        }
        put_uv_mode(bw, mb.uv_mode as i32);
        if !vp8_iterator_next(&mut it, enc) {
            break;
        }
    }
}

//------------------------------------------------------------------------------
// Paragraph 13

/// Write the token probabilities. Translation of `VP8WriteProbas()`.
pub(crate) fn vp8_write_probas(bw: &mut VP8BitWriter, probas: &VP8EncProba) {
    for t in 0..NUM_TYPES {
        for b in 0..NUM_BANDS {
            for c in 0..NUM_CTX {
                for p in 0..NUM_PROBAS {
                    let p0 = probas.coeffs[t][b][c][p];
                    let update = p0 != COEFFS_PROBA0[t][b][c][p];
                    if bw.put_bit(update, COEFFS_UPDATE_PROBA[t][b][c][p] as i32) {
                        bw.put_bits(p0 as u32, 8);
                    }
                }
            }
        }
    }
    if bw.put_bit_uniform(probas.use_skip_proba) {
        bw.put_bits(probas.skip_proba as u32, 8);
    }
}
