// Rust translation of src/enc/frame_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! frame coding: the token loop.
//!
//! Translated is the single pass using the token buffer (`VP8EncTokenLoop()`)
//! that SDL_image's settings take (method 4, one pass, no target size or
//! PSNR). Not translated are the encoding loop without the token buffer
//! (`VP8EncLoop()`, `StatLoop()`, `CodeResiduals()`, `RecordResiduals()`),
//! the size and PSNR search (`ComputeNextQ()`, the estimated sizes) and the
//! statistics and side info.

use crate::webp::dec::tree_dec::{COEFFS_PROBA0, COEFFS_UPDATE_PROBA};
use crate::webp::dec::{NUM_BANDS, NUM_CTX, NUM_MB_SEGMENTS, NUM_PROBAS, NUM_TYPES};
use crate::webp::decode::VP8_MAX_PARTITION0_SIZE;
use crate::webp::dsp::cost::vp8_set_residual_coeffs;
use crate::webp::enc::cost_enc::{vp8_bit_cost, vp8_calculate_level_costs, vp8_init_residual};
use crate::webp::enc::filter_enc::vp8_adjust_filter_strength;
use crate::webp::enc::iterator_enc::{
    vp8_iterator_bytes_to_nz, vp8_iterator_import, vp8_iterator_init, vp8_iterator_next,
    vp8_iterator_nz_to_bytes, vp8_iterator_save_boundary,
};
use crate::webp::enc::picture_enc::webp_encoding_set_error;
use crate::webp::enc::quant_enc::{vp8_decimate, vp8_set_segment_params};
use crate::webp::enc::token_enc::{vp8_emit_tokens, vp8_record_coeff_tokens, vp8_tbuffer_clear};
use crate::webp::enc::vp8i_enc::{
    VP8EncIterator, VP8EncProba, VP8Encoder, VP8ModeScore, VP8RDLevel,
};
use crate::webp::encode::WebPEncodingError;
use crate::webp::utils::bit_writer_utils::VP8BitWriter;

//------------------------------------------------------------------------------
// multi-pass convergence

/// convergence is considered reached if dq < DQ_LIMIT
const DQ_LIMIT: f32 = 0.4;
/// we allow 2k of extra head-room in PARTITION0 limit.
const PARTITION0_SIZE_LIMIT: u64 = (VP8_MAX_PARTITION0_SIZE as u64 - 2048) << 11;

/// Translation of `Clamp()`.
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// struct for organizing convergence in either size or PSNR
/// Translation of `PassStats` (the search's values aren't translated).
struct PassStats {
    dq: f32,
    q: f32,
    do_size_search: bool,
}

/// Translation of `InitPassStats()`.
fn init_pass_stats(enc: &VP8Encoder<'_>) -> PassStats {
    let target_size = enc.config.target_size as u64;
    let do_size_search = target_size != 0;
    let qmin = enc.config.qmin as f32;
    let qmax = enc.config.qmax as f32;
    PassStats {
        dq: 10.0,
        q: clamp(enc.config.quality, qmin, qmax),
        do_size_search,
    }
}

//------------------------------------------------------------------------------
// Tables for level coding

/// Translation of `VP8Cat3`.
pub(crate) const VP8_CAT3: [u8; 3] = [173, 148, 140];
/// Translation of `VP8Cat4`.
pub(crate) const VP8_CAT4: [u8; 4] = [176, 155, 140, 135];
/// Translation of `VP8Cat5`.
pub(crate) const VP8_CAT5: [u8; 5] = [180, 157, 141, 134, 130];
/// Translation of `VP8Cat6`.
pub(crate) const VP8_CAT6: [u8; 11] = [254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129];

//------------------------------------------------------------------------------
// Reset the statistics about: number of skips, token proba, level cost,...

/// Translation of `ResetStats()`.
fn reset_stats(enc: &mut VP8Encoder<'_>) {
    let proba = &mut enc.proba;
    vp8_calculate_level_costs(proba);
    proba.nb_skip = 0;
}

//------------------------------------------------------------------------------
// Skip decision probability

// (CalcSkipProba() and FinalizeSkipProba() serve VP8EncLoop() only.)

/// Collect statistics and deduce probabilities for next coding pass.
/// Return the total bit-cost for coding the probability updates.
/// Translation of `CalcTokenProba()`.
fn calc_token_proba(nb: i32, total: i32) -> i32 {
    debug_assert!(nb <= total);
    if nb != 0 {
        255 - nb * 255 / total
    } else {
        255
    }
}

/// Cost of coding 'nb' 1's and 'total-nb' 0's using 'proba' probability.
/// Translation of `BranchCost()`.
fn branch_cost(nb: i32, total: i32, proba: i32) -> i32 {
    nb * vp8_bit_cost(true, proba as u8) + (total - nb) * vp8_bit_cost(false, proba as u8)
}

/// Translation of `ResetTokenStats()`.
fn reset_token_stats(enc: &mut VP8Encoder<'_>) {
    let proba = &mut enc.proba;
    proba.stats = [[[[0; NUM_PROBAS]; NUM_CTX]; NUM_BANDS]; NUM_TYPES];
}

/// Translation of `FinalizeTokenProbas()`.
fn finalize_token_probas(proba: &mut VP8EncProba) -> i32 {
    let mut has_changed = false;
    let mut size = 0;
    for t in 0..NUM_TYPES {
        for b in 0..NUM_BANDS {
            for c in 0..NUM_CTX {
                for p in 0..NUM_PROBAS {
                    let stats = proba.stats[t][b][c][p];
                    let nb = (stats & 0xffff) as i32;
                    let total = ((stats >> 16) & 0xffff) as i32;
                    let update_proba = COEFFS_UPDATE_PROBA[t][b][c][p];
                    let old_p = COEFFS_PROBA0[t][b][c][p] as i32;
                    let new_p = calc_token_proba(nb, total);
                    let old_cost =
                        branch_cost(nb, total, old_p) + vp8_bit_cost(false, update_proba);
                    let new_cost =
                        branch_cost(nb, total, new_p) + vp8_bit_cost(true, update_proba) + 8 * 256;
                    let use_new_p = old_cost > new_cost;
                    size += vp8_bit_cost(use_new_p, update_proba);
                    if use_new_p {
                        // only use proba that seem meaningful enough.
                        proba.coeffs[t][b][c][p] = new_p as u8;
                        has_changed |= new_p != old_p;
                        size += 8 * 256;
                    } else {
                        proba.coeffs[t][b][c][p] = old_p as u8;
                    }
                }
            }
        }
    }
    proba.dirty = has_changed;
    size
}

//------------------------------------------------------------------------------
// Finalize Segment probability based on the coding tree

/// Translation of `GetProba()`.
fn get_proba(a: i32, b: i32) -> i32 {
    let total = a + b;
    if total == 0 {
        255 // that's the default probability.
    } else {
        (255 * a + total / 2) / total // rounded proba
    }
}

/// Translation of `ResetSegments()`.
fn reset_segments(enc: &mut VP8Encoder<'_>) {
    for mb in enc.mb_info.iter_mut() {
        mb.segment = 0;
    }
}

/// Translation of `SetSegmentProbas()`.
fn set_segment_probas(enc: &mut VP8Encoder<'_>) {
    let mut p = [0i32; NUM_MB_SEGMENTS];

    for mb in &enc.mb_info {
        p[mb.segment as usize] += 1;
    }
    if enc.segment_hdr.num_segments > 1 {
        let probas = &mut enc.proba.segments;
        probas[0] = get_proba(p[0] + p[1], p[2] + p[3]) as u8;
        probas[1] = get_proba(p[0], p[1]) as u8;
        probas[2] = get_proba(p[2], p[3]) as u8;
        let probas = *probas;

        enc.segment_hdr.update_map = (probas[0] != 255) || (probas[1] != 255) || (probas[2] != 255);
        if !enc.segment_hdr.update_map {
            reset_segments(enc);
        }
        enc.segment_hdr.size = p[0]
            * (vp8_bit_cost(false, probas[0]) + vp8_bit_cost(false, probas[1]))
            + p[1] * (vp8_bit_cost(false, probas[0]) + vp8_bit_cost(true, probas[1]))
            + p[2] * (vp8_bit_cost(true, probas[0]) + vp8_bit_cost(false, probas[2]))
            + p[3] * (vp8_bit_cost(true, probas[0]) + vp8_bit_cost(true, probas[2]));
    } else {
        enc.segment_hdr.update_map = false;
        enc.segment_hdr.size = 0;
    }
}

//------------------------------------------------------------------------------
// Token buffer

/// Translation of `RecordTokens()`.
fn record_tokens(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>, rd: &VP8ModeScore) -> bool {
    vp8_iterator_nz_to_bytes(it, enc);
    let proba = &mut enc.proba;
    let tokens = &mut enc.tokens;
    let mut res_ac = if enc.mb_info[it.mb].type_ == 1 {
        // i16x16
        let ctx = (it.top_nz[8] + it.left_nz[8]) as usize;
        let mut res = vp8_init_residual(0, 1);
        vp8_set_residual_coeffs(&rd.y_dc_levels, &mut res);
        let nz = vp8_record_coeff_tokens(ctx, &res, &mut proba.stats[1], tokens);
        it.top_nz[8] = nz;
        it.left_nz[8] = nz;
        vp8_init_residual(1, 0)
    } else {
        vp8_init_residual(0, 3)
    };

    // luma-AC
    for y in 0..4 {
        for x in 0..4 {
            let ctx = (it.top_nz[x] + it.left_nz[y]) as usize;
            vp8_set_residual_coeffs(&rd.y_ac_levels[x + y * 4], &mut res_ac);
            let t = res_ac.coeff_type as usize;
            let nz = vp8_record_coeff_tokens(ctx, &res_ac, &mut proba.stats[t], tokens);
            it.top_nz[x] = nz;
            it.left_nz[y] = nz;
        }
    }

    // U/V
    let mut res = vp8_init_residual(0, 2);
    for ch in [0, 2] {
        for y in 0..2 {
            for x in 0..2 {
                let ctx = (it.top_nz[4 + ch + x] + it.left_nz[4 + ch + y]) as usize;
                vp8_set_residual_coeffs(&rd.uv_levels[ch * 2 + x + y * 2], &mut res);
                let nz = vp8_record_coeff_tokens(ctx, &res, &mut proba.stats[2], tokens);
                it.top_nz[4 + ch + x] = nz;
                it.left_nz[4 + ch + y] = nz;
            }
        }
    }
    vp8_iterator_bytes_to_nz(it, enc);
    !enc.tokens.error
}

//------------------------------------------------------------------------------
//  StatLoop(): only collect statistics (number of skips, token usage, ...).
//  This is used for deciding optimal probabilities. It also modifies the
//  quantizer value if some target (size, PSNR) was specified.

/// Translation of `SetLoopParams()`.
fn set_loop_params(enc: &mut VP8Encoder<'_>, q: f32) {
    // Make sure the quality parameter is inside valid bounds
    let q = clamp(q, 0.0, 100.0);

    vp8_set_segment_params(enc, q); // setup segment quantizations and filters
    set_segment_probas(enc); // compute segment probabilities

    reset_stats(enc);
}

//------------------------------------------------------------------------------
// Main loops
//

const K_AVERAGE_BYTES_PER_MB: [u8; 8] = [50, 24, 16, 9, 7, 5, 3, 2];

/// Translation of `PreLoopInitialize()`.
fn pre_loop_initialize(enc: &mut VP8Encoder<'_>) -> bool {
    let average_bytes_per_mb = K_AVERAGE_BYTES_PER_MB[(enc.base_quant >> 4) as usize] as i32;
    let bytes_per_parts = enc.mb_w * enc.mb_h * average_bytes_per_mb / enc.num_parts;
    // Initialize the bit-writers
    for p in 0..enc.num_parts as usize {
        enc.parts[p] = VP8BitWriter::new(bytes_per_parts as usize);
    }
    true
}

/// Translation of `PostLoopFinalize()`.
fn post_loop_finalize(enc: &mut VP8Encoder<'_>, mut ok: bool) -> bool {
    if ok {
        // Finalize the partitions, check for extra errors.
        for p in 0..enc.num_parts as usize {
            enc.parts[p].finish();
            ok &= !enc.parts[p].error;
        }
    }

    if ok {
        // All good. Finish up.
        vp8_adjust_filter_strength(enc); // ...and store filter stats.
    } else {
        // Something bad happened -> need to do some memory cleanup.
        crate::webp::enc::syntax_enc::vp8_enc_free_bit_writers(enc);
        return webp_encoding_set_error(enc.pic, WebPEncodingError::OutOfMemory);
    }
    ok
}

//------------------------------------------------------------------------------
// Single pass using Token Buffer.

/// minimum number of macroblocks before updating stats
const MIN_COUNT: i32 = 96;

/// Translation of `VP8EncTokenLoop()`.
pub(crate) fn vp8_enc_token_loop(enc: &mut VP8Encoder<'_>) -> bool {
    // Roughly refresh the proba eight times per pass
    let mut max_count = (enc.mb_w * enc.mb_h) >> 3;
    let mut num_pass_left = enc.config.pass;
    let do_search = enc.do_search;
    let rd_opt = enc.rd_opt_level;
    let stats = init_pass_stats(enc);
    let mut ok = pre_loop_initialize(enc);
    if !ok {
        return false;
    }

    if max_count < MIN_COUNT {
        max_count = MIN_COUNT;
    }

    debug_assert!(enc.num_parts == 1);
    debug_assert!(enc.use_tokens);
    debug_assert!(!enc.proba.use_skip_proba);
    debug_assert!(rd_opt >= VP8RDLevel::Basic); // otherwise, token-buffer won't be useful
    debug_assert!(num_pass_left > 0);
    debug_assert!(
        !do_search && !stats.do_size_search,
        "the size and PSNR search are not translated"
    );

    while ok && num_pass_left > 0 {
        num_pass_left -= 1;
        let is_last_pass =
            (stats.dq.abs() <= DQ_LIMIT) || (num_pass_left == 0) || (enc.max_i4_header_bits == 0);
        let mut size_p0: u64 = 0;
        let mut cnt = max_count;
        let mut it = vp8_iterator_init(enc);
        set_loop_params(enc, stats.q);
        if is_last_pass {
            reset_token_stats(enc);
            // (VP8InitFilter(): don't collect stats until last pass (too costly))
        }
        vp8_tbuffer_clear(&mut enc.tokens);
        loop {
            let mut info = VP8ModeScore::default();
            vp8_iterator_import(&mut it, enc, false);
            cnt -= 1;
            if cnt < 0 {
                finalize_token_probas(&mut enc.proba);
                vp8_calculate_level_costs(&mut enc.proba); // refresh cost tables for rd-opt
                cnt = max_count;
            }
            vp8_decimate(&mut it, enc, &mut info, rd_opt);
            ok = record_tokens(&mut it, enc, &info);
            if !ok {
                webp_encoding_set_error(enc.pic, WebPEncodingError::OutOfMemory);
                break;
            }
            size_p0 += info.h as u64;
            // (is_last_pass: StoreSideInfo(), VP8StoreFilterStats() and
            // VP8IteratorExport() do nothing with SDL_image's settings.)
            vp8_iterator_save_boundary(&mut it, enc);
            if !(ok && vp8_iterator_next(&mut it, enc)) {
                break;
            }
        }
        if !ok {
            break;
        }

        size_p0 += enc.segment_hdr.size as u64;
        // (stats.value: the size or PSNR, for the search.)

        if enc.max_i4_header_bits > 0 && size_p0 > PARTITION0_SIZE_LIMIT {
            num_pass_left += 1;
            enc.max_i4_header_bits >>= 1; // strengthen header bit limitation...
            continue; // ...and start over
        }
        if is_last_pass {
            break; // done
        }
    }
    if ok {
        if !stats.do_size_search {
            finalize_token_probas(&mut enc.proba);
        }
        let probas: Vec<u8> = enc
            .proba
            .coeffs
            .as_flattened()
            .as_flattened()
            .as_flattened()
            .to_vec();
        ok = vp8_emit_tokens(&mut enc.tokens, &mut enc.parts[0], &probas, true);
    }
    post_loop_finalize(enc, ok)
}
