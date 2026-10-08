// Rust translation of src/enc/quant_enc.c and src/dsp/quant.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Quantization and mode decision.
//!
//! The trellis quantization (`TrellisQuantizeBlock()`), `SimpleQuantize()`
//! and `RefineUsingDistortion()` are not translated: they serve methods 0
//! to 2 and 5 to 6, and SDL_image encodes with method 4.

use crate::webp::dec::{NUM_BMODES, NUM_MB_SEGMENTS, NUM_PRED_MODES};
use crate::webp::dsp::dec::transform_wht;
use crate::webp::dsp::enc::{
    vp8_copy16x8, vp8_copy4x4, vp8_enc_pred_chroma8, vp8_enc_pred_luma16, vp8_enc_pred_luma4,
    vp8_enc_quantize2_blocks, vp8_enc_quantize_block, vp8_ftransform, vp8_ftransform2,
    vp8_ftransform_wht, vp8_itransform, vp8_sse16x16, vp8_sse16x8, vp8_sse4x4, vp8_tdisto16x16,
    vp8_tdisto4x4,
};
use crate::webp::dsp::BPS;
use crate::webp::enc::cost_enc::{
    vp8_get_cost_luma16, vp8_get_cost_luma4, vp8_get_cost_uv, VP8_FIXED_COSTS_I16,
    VP8_FIXED_COSTS_I4, VP8_FIXED_COSTS_UV,
};
use crate::webp::enc::filter_enc::vp8_filter_strength_from_delta;
use crate::webp::enc::iterator_enc::{
    uv_top, vp8_iterator_rotate_i4, vp8_iterator_start_i4, vp8_set_intra16_mode,
    vp8_set_intra4_mode, vp8_set_intra_uv_mode, vp8_set_skip, y_top,
};
use crate::webp::enc::vp8i_enc::{
    bias, quantdiv, ScoreT, VP8EncIterator, VP8Encoder, VP8Matrix, VP8ModeScore, VP8RDLevel,
    VP8SegmentInfo, C8DC8, C8HE8, C8TM8, C8VE8, I16DC16, I16HE16, I16TM16, I16VE16, I4DC4, I4HD4,
    I4HE4, I4HU4, I4LD4, I4RD4, I4TM4, I4TMP, I4VE4, I4VL4, I4VR4, MAX_COST, QFIX, U_LEFT,
    U_OFF_ENC, Y_LEFT, Y_OFF_ENC,
};

/// neutral value for susceptibility
const MID_ALPHA: i32 = 64;
/// lowest usable value for susceptibility
const MIN_ALPHA: i32 = 30;
/// higher meaningful value for susceptibility
const MAX_ALPHA: i32 = 100;

/// Scaling constant between the sns value and the QP
/// power-law modulation. Must be strictly less than 1.
const SNS_TO_DQ: f64 = 0.9;

// number of non-zero coeffs below which we consider the block very flat
// (and apply a penalty to complex predictions)
/// I16 mode (special case)
const FLATNESS_LIMIT_I16: i32 = 0;
/// I4 mode
const FLATNESS_LIMIT_I4: i32 = 3;
/// UV mode
const FLATNESS_LIMIT_UV: i32 = 2;
/// roughly ~1bit per block
const FLATNESS_PENALTY: ScoreT = 140;

/// Translation of the `MULT_8B()` macro.
fn mult_8b(a: i32, b: i32) -> i32 {
    (a * b + 128) >> 8
}

/// distortion multiplier (equivalent of lambda)
const RD_DISTO_MULT: ScoreT = 256;

//------------------------------------------------------------------------------
// dsp/quant.h

/// Translation of `IsFlat_C()` (`IsFlat`).
fn is_flat(levels: &[i16], num_blocks: usize, thresh: i32) -> bool {
    let mut score = 0;
    for block in levels.chunks_exact(16).take(num_blocks) {
        // TODO(skal): refine positional scoring?
        for &level in &block[1..16] {
            // omit DC, we're only interested in AC
            score += (level != 0) as i32;
            if score > thresh {
                return false;
            }
        }
    }
    true
}

/// Translation of `IsFlatSource16()`.
fn is_flat_source16(src: &[u8]) -> bool {
    let v = src[0];
    for i in 0..16 {
        if src[i * BPS..i * BPS + 16].iter().any(|&s| s != v) {
            return false;
        }
    }
    true
}

//------------------------------------------------------------------------------

/// Translation of `clip()`.
fn clip(v: i32, m: i32, mx: i32) -> i32 {
    if v < m {
        m
    } else if v > mx {
        mx
    } else {
        v
    }
}

const K_DC_TABLE: [u8; 128] = [
    4, 5, 6, 7, 8, 9, 10, 10, 11, 12, 13, 14, 15, 16, 17, 17, 18, 19, 20, 20, 21, 21, 22, 22, 23,
    23, 24, 25, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 37, 38, 39, 40, 41, 42, 43, 44,
    45, 46, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67,
    68, 69, 70, 71, 72, 73, 74, 75, 76, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 91,
    93, 95, 96, 98, 100, 101, 102, 104, 106, 108, 110, 112, 114, 116, 118, 122, 124, 126, 128, 130,
    132, 134, 136, 138, 140, 143, 145, 148, 151, 154, 157,
];

pub(crate) const K_AC_TABLE: [u16; 128] = [
    4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
    29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
    53, 54, 55, 56, 57, 58, 60, 62, 64, 66, 68, 70, 72, 74, 76, 78, 80, 82, 84, 86, 88, 90, 92, 94,
    96, 98, 100, 102, 104, 106, 108, 110, 112, 114, 116, 119, 122, 125, 128, 131, 134, 137, 140,
    143, 146, 149, 152, 155, 158, 161, 164, 167, 170, 173, 177, 181, 185, 189, 193, 197, 201, 205,
    209, 213, 217, 221, 225, 229, 234, 239, 245, 249, 254, 259, 264, 269, 274, 279, 284,
];

const K_AC_TABLE2: [u16; 128] = [
    8, 8, 9, 10, 12, 13, 15, 17, 18, 20, 21, 23, 24, 26, 27, 29, 31, 32, 34, 35, 37, 38, 40, 41,
    43, 44, 46, 48, 49, 51, 52, 54, 55, 57, 58, 60, 62, 63, 65, 66, 68, 69, 71, 72, 74, 75, 77, 79,
    80, 82, 83, 85, 86, 88, 89, 93, 96, 99, 102, 105, 108, 111, 114, 117, 120, 124, 127, 130, 133,
    136, 139, 142, 145, 148, 151, 155, 158, 161, 164, 167, 170, 173, 176, 179, 184, 189, 193, 198,
    203, 207, 212, 217, 221, 226, 230, 235, 240, 244, 249, 254, 258, 263, 268, 274, 280, 286, 292,
    299, 305, 311, 317, 323, 330, 336, 342, 348, 354, 362, 370, 379, 385, 393, 401, 409, 416, 424,
    432, 440,
];

/// [luma-ac,luma-dc,chroma][dc,ac]
const K_BIAS_MATRICES: [[u8; 2]; 3] = [[96, 110], [96, 108], [110, 115]];

// Sharpening by (slightly) raising the hi-frequency coeffs.
// Hack-ish but helpful for mid-bitrate range. Use with care.
/// number of descaling bits for sharpening bias
const SHARPEN_BITS: u32 = 11;
const K_FREQ_SHARPENING: [u8; 16] = [
    0, 30, 60, 90, 30, 60, 90, 90, 60, 90, 90, 90, 90, 90, 90, 90,
];

//------------------------------------------------------------------------------
// Initialize quantization parameters in VP8Matrix

/// Returns the average quantizer. Translation of `ExpandMatrix()`.
fn expand_matrix(m: &mut VP8Matrix, type_: usize) -> i32 {
    for i in 0..2 {
        let is_ac_coeff = i > 0;
        let b = K_BIAS_MATRICES[type_][is_ac_coeff as usize] as u32;
        m.iq[i] = ((1 << QFIX) / m.q[i] as u32) as u16;
        m.bias[i] = bias(b);
        // zthresh_ is the exact value such that QUANTDIV(coeff, iQ, B) is:
        //   * zero if coeff <= zthresh
        //   * non-zero if coeff > zthresh
        m.zthresh[i] = ((1 << QFIX) - 1 - m.bias[i]) / m.iq[i] as u32;
    }
    for i in 2..16 {
        m.q[i] = m.q[1];
        m.iq[i] = m.iq[1];
        m.bias[i] = m.bias[1];
        m.zthresh[i] = m.zthresh[1];
    }
    let mut sum = 0;
    for i in 0..16 {
        if type_ == 0 {
            // we only use sharpening for AC luma coeffs
            m.sharpen[i] = ((K_FREQ_SHARPENING[i] as u32 * m.q[i] as u32) >> SHARPEN_BITS) as u16;
        } else {
            m.sharpen[i] = 0;
        }
        sum += m.q[i] as i32;
    }
    (sum + 8) >> 4
}

/// Translation of `CheckLambdaValue()`.
fn check_lambda_value(v: &mut i32) {
    if *v < 1 {
        *v = 1;
    }
}

/// Translation of `SetupMatrices()`.
fn setup_matrices(enc: &mut VP8Encoder<'_>) {
    let tlambda_scale = if enc.method >= 4 {
        enc.config.sns_strength
    } else {
        0
    };
    let num_segments = enc.segment_hdr.num_segments as usize;
    for i in 0..num_segments {
        let (dq_y1_dc, dq_y2_dc, dq_y2_ac) = (enc.dq_y1_dc, enc.dq_y2_dc, enc.dq_y2_ac);
        let (dq_uv_dc, dq_uv_ac) = (enc.dq_uv_dc, enc.dq_uv_ac);
        let m = &mut enc.dqm[i];
        let q = m.quant;
        m.y1.q[0] = K_DC_TABLE[clip(q + dq_y1_dc, 0, 127) as usize] as u16;
        m.y1.q[1] = K_AC_TABLE[clip(q, 0, 127) as usize];

        m.y2.q[0] = K_DC_TABLE[clip(q + dq_y2_dc, 0, 127) as usize] as u16 * 2;
        m.y2.q[1] = K_AC_TABLE2[clip(q + dq_y2_ac, 0, 127) as usize];

        m.uv.q[0] = K_DC_TABLE[clip(q + dq_uv_dc, 0, 117) as usize] as u16;
        m.uv.q[1] = K_AC_TABLE[clip(q + dq_uv_ac, 0, 127) as usize];

        let q_i4 = expand_matrix(&mut m.y1, 0);
        let q_i16 = expand_matrix(&mut m.y2, 1);
        let q_uv = expand_matrix(&mut m.uv, 2);

        m.lambda_i4 = (3 * q_i4 * q_i4) >> 7;
        m.lambda_i16 = 3 * q_i16 * q_i16;
        m.lambda_uv = (3 * q_uv * q_uv) >> 6;
        m.lambda_mode = (q_i4 * q_i4) >> 7;
        m.lambda_trellis_i4 = (7 * q_i4 * q_i4) >> 3;
        m.lambda_trellis_i16 = (q_i16 * q_i16) >> 2;
        m.lambda_trellis_uv = (q_uv * q_uv) << 1;
        m.tlambda = (tlambda_scale * q_i4) >> 5;

        // none of these constants should be < 1
        check_lambda_value(&mut m.lambda_i4);
        check_lambda_value(&mut m.lambda_i16);
        check_lambda_value(&mut m.lambda_uv);
        check_lambda_value(&mut m.lambda_mode);
        check_lambda_value(&mut m.lambda_trellis_i4);
        check_lambda_value(&mut m.lambda_trellis_i16);
        check_lambda_value(&mut m.lambda_trellis_uv);
        check_lambda_value(&mut m.tlambda);

        m.min_disto = 20 * m.y1.q[0] as i32; // quantization-aware min disto
        m.max_edge = 0;

        m.i4_penalty = 1000 * q_i4 as ScoreT * q_i4 as ScoreT;
    }
}

//------------------------------------------------------------------------------
// Initialize filtering parameters

/// Very small filter-strength values have close to no visual effect. So we can
/// save a little decoding-CPU by turning filtering off for these.
const FSTRENGTH_CUTOFF: i32 = 2;

/// Translation of `SetupFilterStrength()`.
fn setup_filter_strength(enc: &mut VP8Encoder<'_>) {
    // level0 is in [0..500]. Using '-f 50' as filter_strength is mid-filtering.
    let level0 = 5 * enc.config.filter_strength;
    for i in 0..NUM_MB_SEGMENTS {
        let sharpness = enc.filter_hdr.sharpness;
        let m = &mut enc.dqm[i];
        // We focus on the quantization of AC coeffs.
        let qstep = (K_AC_TABLE[clip(m.quant, 0, 127) as usize] >> 2) as i32;
        let base_strength = vp8_filter_strength_from_delta(sharpness, qstep);
        // Segments with lower complexity ('beta') will be less filtered.
        let f = base_strength * level0 / (256 + m.beta);
        m.fstrength = if f < FSTRENGTH_CUTOFF {
            0
        } else if f > 63 {
            63
        } else {
            f
        };
    }
    // We record the initial strength (mainly for the case of 1-segment only).
    enc.filter_hdr.level = enc.dqm[0].fstrength;
    enc.filter_hdr.simple = enc.config.filter_type == 0;
    enc.filter_hdr.sharpness = enc.config.filter_sharpness;
}

//------------------------------------------------------------------------------

// Note: if you change the values below, remember that the max range
// allowed by the syntax for DQ_UV is [-16,16].
const MAX_DQ_UV: i32 = 6;
const MIN_DQ_UV: i32 = -4;

/// We want to emulate jpeg-like behaviour where the expected "good" quality
/// is around q=75. Internally, our "good" middle is around c=50. So we
/// map accordingly using linear piece-wise function
/// Translation of `QualityToCompression()`.
fn quality_to_compression(c: f64) -> f64 {
    let linear_c = if c < 0.75 { c * (2. / 3.) } else { 2. * c - 1. };
    // The file size roughly scales as pow(quantizer, 3.). Actually, the
    // exponent is somewhere between 2.8 and 3.2, but we're mostly interested
    // in the mid-quant range. So we scale the compressibility inversely to
    // this power-law: quant ~= compression ^ 1/3. This law holds well for
    // low quant. Finer modeling for high-quant would make use of kAcTable[]
    // more explicitly.
    linear_c.powf(1. / 3.)
}

/// Translation of `QualityToJPEGCompression()`.
fn quality_to_jpeg_compression(c: f64, alpha: f64) -> f64 {
    // We map the complexity 'alpha' and quality setting 'c' to a compression
    // exponent empirically matched to the compression curve of libjpeg6b.
    // On average, the WebP output size will be roughly similar to that of a
    // JPEG file compressed with same quality factor.
    let amin = 0.30;
    let amax = 0.85;
    let exp_min = 0.4;
    let exp_max = 0.9;
    let slope = (exp_min - exp_max) / (amax - amin);
    // Linearly interpolate 'expn' from exp_min to exp_max
    // in the [amin, amax] range.
    let expn = if alpha > amax {
        exp_min
    } else if alpha < amin {
        exp_max
    } else {
        exp_max + slope * (alpha - amin)
    };
    c.powf(expn)
}

/// Translation of `SegmentsAreEquivalent()`.
fn segments_are_equivalent(s1: &VP8SegmentInfo, s2: &VP8SegmentInfo) -> bool {
    (s1.quant == s2.quant) && (s1.fstrength == s2.fstrength)
}

/// Translation of `SimplifySegments()`.
fn simplify_segments(enc: &mut VP8Encoder<'_>) {
    let mut map: [u8; NUM_MB_SEGMENTS] = [0, 1, 2, 3];
    // 'num_segments_' is previously validated and <= NUM_MB_SEGMENTS, but an
    // explicit check is needed to avoid a spurious warning about 'i' exceeding
    // array bounds of 'dqm_' with some compilers (noticed with gcc-4.9).
    let num_segments = (enc.segment_hdr.num_segments as usize).min(NUM_MB_SEGMENTS);
    let mut num_final_segments = 1;
    for s1 in 1..num_segments {
        // find similar segments
        let mut found = false;
        // check if we already have similar segment
        let mut s2 = 0;
        while s2 < num_final_segments {
            if segments_are_equivalent(&enc.dqm[s1], &enc.dqm[s2]) {
                found = true;
                break;
            }
            s2 += 1;
        }
        map[s1] = s2 as u8;
        if !found {
            if num_final_segments != s1 {
                enc.dqm[num_final_segments] = enc.dqm[s1];
            }
            num_final_segments += 1;
        }
    }
    if num_final_segments < num_segments {
        // Remap
        for mb in enc.mb_info.iter_mut().rev() {
            mb.segment = map[mb.segment as usize];
        }
        enc.segment_hdr.num_segments = num_final_segments as i32;
        // Replicate the trailing segment infos (it's mostly cosmetics)
        for i in num_final_segments..num_segments {
            enc.dqm[i] = enc.dqm[num_final_segments - 1];
        }
    }
}

/// Sets up segment's quantization values, base_quant_ and filter strengths.
/// Translation of `VP8SetSegmentParams()`.
pub(crate) fn vp8_set_segment_params(enc: &mut VP8Encoder<'_>, quality: f32) {
    let num_segments = enc.segment_hdr.num_segments as usize;
    let amp = SNS_TO_DQ * enc.config.sns_strength as f64 / 100. / 128.;
    let q = quality as f64 / 100.;
    let c_base = if enc.config.emulate_jpeg_size != 0 {
        quality_to_jpeg_compression(q, enc.alpha as f64 / 255.)
    } else {
        quality_to_compression(q)
    };
    for i in 0..num_segments {
        // We modulate the base coefficient to accommodate for the quantization
        // susceptibility and allow denser segments to be quantized more.
        let expn = 1. - amp * enc.dqm[i].alpha as f64;
        let c = c_base.powf(expn);
        let q = (127. * (1. - c)) as i32;
        debug_assert!(expn > 0.);
        enc.dqm[i].quant = clip(q, 0, 127);
    }

    // purely indicative in the bitstream (except for the 1-segment case)
    enc.base_quant = enc.dqm[0].quant;

    // fill-in values for the unused segments (required by the syntax)
    for i in num_segments..NUM_MB_SEGMENTS {
        enc.dqm[i].quant = enc.base_quant;
    }

    // uv_alpha_ is normally spread around ~60. The useful range is
    // typically ~30 (quite bad) to ~100 (ok to decimate UV more).
    // We map it to the safe maximal range of MAX/MIN_DQ_UV for dq_uv.
    let mut dq_uv_ac =
        (enc.uv_alpha - MID_ALPHA) * (MAX_DQ_UV - MIN_DQ_UV) / (MAX_ALPHA - MIN_ALPHA);
    // we rescale by the user-defined strength of adaptation
    dq_uv_ac = dq_uv_ac * enc.config.sns_strength / 100;
    // and make it safe.
    dq_uv_ac = clip(dq_uv_ac, MIN_DQ_UV, MAX_DQ_UV);
    // We also boost the dc-uv-quant a little, based on sns-strength, since
    // U/V channels are quite more reactive to high quants (flat DC-blocks
    // tend to appear, and are unpleasant).
    let mut dq_uv_dc = -4 * enc.config.sns_strength / 100;
    dq_uv_dc = clip(dq_uv_dc, -15, 15); // 4bit-signed max allowed

    enc.dq_y1_dc = 0; // TODO(skal): dq-lum
    enc.dq_y2_dc = 0;
    enc.dq_y2_ac = 0;
    enc.dq_uv_dc = dq_uv_dc;
    enc.dq_uv_ac = dq_uv_ac;

    setup_filter_strength(enc); // initialize segments' filtering, eventually

    if num_segments > 1 {
        simplify_segments(enc);
    }

    setup_matrices(enc); // finalize quantization matrices
}

//------------------------------------------------------------------------------
// Form the predictions in cache

/// Must be ordered using {DC_PRED, TM_PRED, V_PRED, H_PRED} as index
/// Translation of `VP8I16ModeOffsets`.
pub(crate) const VP8_I16_MODE_OFFSETS: [usize; 4] = [I16DC16, I16TM16, I16VE16, I16HE16];
/// Translation of `VP8UVModeOffsets`.
pub(crate) const VP8_UV_MODE_OFFSETS: [usize; 4] = [C8DC8, C8TM8, C8VE8, C8HE8];

/// Must be indexed using {B_DC_PRED -> B_HU_PRED} as index
/// Translation of `VP8I4ModeOffsets`.
pub(crate) const VP8_I4_MODE_OFFSETS: [usize; NUM_BMODES as usize] = [
    I4DC4, I4TM4, I4VE4, I4HE4, I4RD4, I4VR4, I4LD4, I4VL4, I4HD4, I4HU4,
];

/// Form all the four Intra16x16 predictions in the yuv_p_ cache
/// Translation of `VP8MakeLuma16Preds()`.
pub(crate) fn vp8_make_luma16_preds(it: &mut VP8EncIterator, enc: &VP8Encoder<'_>) {
    let mut yuv_p = std::mem::take(&mut it.yuv_p);
    {
        let left = if it.x != 0 {
            Some(&it.yuv_left_mem[Y_LEFT - 1..])
        } else {
            None
        };
        let top = if it.y != 0 {
            Some(y_top(it, enc))
        } else {
            None
        };
        vp8_enc_pred_luma16(&mut yuv_p, left, top);
    }
    it.yuv_p = yuv_p;
}

/// Form all the four Chroma8x8 predictions in the yuv_p_ cache
/// Translation of `VP8MakeChroma8Preds()`.
pub(crate) fn vp8_make_chroma8_preds(it: &mut VP8EncIterator, enc: &VP8Encoder<'_>) {
    let mut yuv_p = std::mem::take(&mut it.yuv_p);
    {
        let left = if it.x != 0 {
            Some(&it.yuv_left_mem[U_LEFT - 1..])
        } else {
            None
        };
        let top = if it.y != 0 {
            Some(uv_top(it, enc))
        } else {
            None
        };
        vp8_enc_pred_chroma8(&mut yuv_p, left, top);
    }
    it.yuv_p = yuv_p;
}

/// Form all the ten Intra4x4 predictions in the yuv_p_ cache
/// for the 4x4 block it->i4_
/// Translation of `VP8MakeIntra4Preds()`.
pub(crate) fn vp8_make_intra4_preds(it: &mut VP8EncIterator) {
    vp8_enc_pred_luma4(&mut it.yuv_p, &it.i4_boundary, it.i4_top);
}

//------------------------------------------------------------------------------
// Quantize

// Layout:
// +----+----+
// |YYYY|UUVV| 0
// |YYYY|UUVV| 4
// |YYYY|....| 8
// |YYYY|....| 12
// +----+----+

/// Luma. Translation of `VP8Scan`.
pub(crate) const VP8_SCAN: [u16; 16] = [
    0,
    4,
    8,
    12,
    4 * BPS as u16,
    4 + 4 * BPS as u16,
    8 + 4 * BPS as u16,
    12 + 4 * BPS as u16,
    8 * BPS as u16,
    4 + 8 * BPS as u16,
    8 + 8 * BPS as u16,
    12 + 8 * BPS as u16,
    12 * BPS as u16,
    4 + 12 * BPS as u16,
    8 + 12 * BPS as u16,
    12 + 12 * BPS as u16,
];

/// Translation of `VP8ScanUV`.
const VP8_SCAN_UV: [usize; 4 + 4] = [
    0,
    4,
    4 * BPS,
    4 + 4 * BPS, // U
    8,
    12,
    8 + 4 * BPS,
    12 + 4 * BPS, // V
];

//------------------------------------------------------------------------------
// Distortion measurement

const K_WEIGHT_Y: [u16; 16] = [38, 32, 20, 9, 32, 28, 17, 7, 20, 17, 10, 4, 9, 7, 4, 2];

/// Init/Copy the common fields in score. Translation of `InitScore()`.
fn init_score(rd: &mut VP8ModeScore) {
    rd.d = 0;
    rd.sd = 0;
    rd.r = 0;
    rd.h = 0;
    rd.nz = 0;
    rd.score = MAX_COST;
}

/// Translation of `CopyScore()`.
fn copy_score(dst: &mut VP8ModeScore, src: &VP8ModeScore) {
    dst.d = src.d;
    dst.sd = src.sd;
    dst.r = src.r;
    dst.h = src.h;
    dst.nz = src.nz; // note that nz is not accumulated, but just copied.
    dst.score = src.score;
}

/// Translation of `AddScore()`.
fn add_score(dst: &mut VP8ModeScore, src: &VP8ModeScore) {
    dst.d += src.d;
    dst.sd += src.sd;
    dst.r += src.r;
    dst.h += src.h;
    dst.nz |= src.nz; // here, new nz bits are accumulated.
    dst.score += src.score;
}

//------------------------------------------------------------------------------
// Performs trellis-optimized quantization.

/// Translation of `SetRDScore()`.
fn set_rd_score(lambda: i32, rd: &mut VP8ModeScore) {
    rd.score = (rd.r + rd.h) * lambda as ScoreT + RD_DISTO_MULT * (rd.d + rd.sd);
}

// (TrellisQuantizeBlock() is not translated.)

/// quantized levels in *levels.
/// Translation of `ReconstructIntra16()`, writing into `it->yuv_out2_`
/// (the only `yuv_out` passed with the RD-opt levels translated).
fn reconstruct_intra16(
    it: &mut VP8EncIterator,
    enc: &VP8Encoder<'_>,
    rd: &mut VP8ModeScore,
    mode: usize,
) -> u32 {
    let ref_ = &it.yuv_p[VP8_I16_MODE_OFFSETS[mode]..];
    let src = &it.yuv_in[Y_OFF_ENC..];
    let dqm = &enc.dqm[enc.mb_info[it.mb].segment as usize];
    let mut nz: u32 = 0;
    let mut tmp = [[0i16; 16]; 16];
    let mut dc_tmp = [0i16; 16];

    for n in (0..16).step_by(2) {
        let s = VP8_SCAN[n] as usize;
        vp8_ftransform2(&src[s..], &ref_[s..], tmp[n..n + 2].as_flattened_mut());
    }
    vp8_ftransform_wht(tmp.as_flattened(), &mut dc_tmp);
    nz |= (vp8_enc_quantize_block(&mut dc_tmp, &mut rd.y_dc_levels, &dqm.y2) as u32) << 24;

    debug_assert!(!it.do_trellis, "the trellis quantization is not translated");
    for n in (0..16).step_by(2) {
        // Zero-out the first coeff, so that: a) nz is correct below, and
        // b) finding 'last' non-zero coeffs in SetResidualCoeffs() is simplified.
        tmp[n][0] = 0;
        tmp[n + 1][0] = 0;
        nz |= (vp8_enc_quantize2_blocks(
            tmp[n..n + 2].as_flattened_mut(),
            rd.y_ac_levels[n..n + 2].as_flattened_mut(),
            &dqm.y1,
        ) as u32)
            << n;
        debug_assert!(rd.y_ac_levels[n][0] == 0);
        debug_assert!(rd.y_ac_levels[n + 1][0] == 0);
    }

    // Transform back
    transform_wht(&dc_tmp, tmp.as_flattened_mut());
    let yuv_out = &mut it.yuv_out2[Y_OFF_ENC..];
    for n in (0..16).step_by(2) {
        let s = VP8_SCAN[n] as usize;
        vp8_itransform(
            &ref_[s..],
            tmp[n..n + 2].as_flattened(),
            &mut yuv_out[s..],
            true,
        );
    }

    nz
}

/// Translation of `ReconstructIntra4()`: the block is `src` in
/// `it->yuv_in_` and `dst` in `it->yuv_p_` (if `dst_in_p`) or
/// `it->yuv_out2_`.
fn reconstruct_intra4(
    it: &mut VP8EncIterator,
    enc: &VP8Encoder<'_>,
    levels: &mut [i16; 16],
    src: usize,
    dst_in_p: bool,
    dst: usize,
    mode: usize,
) -> i32 {
    // (the prediction is copied out, the output possibly sharing its buffer)
    let mut ref_ = [0u8; 3 * BPS + 4];
    let r = VP8_I4_MODE_OFFSETS[mode];
    ref_.copy_from_slice(&it.yuv_p[r..r + 3 * BPS + 4]);
    let dqm = &enc.dqm[enc.mb_info[it.mb].segment as usize];
    let mut tmp = [0i16; 16];

    vp8_ftransform(&it.yuv_in[src..], &ref_, &mut tmp);
    debug_assert!(!it.do_trellis, "the trellis quantization is not translated");
    let nz = vp8_enc_quantize_block(&mut tmp, levels, &dqm.y1);
    let yuv_out = if dst_in_p {
        &mut it.yuv_p
    } else {
        &mut it.yuv_out2
    };
    vp8_itransform(&ref_, &tmp, &mut yuv_out[dst..], false);
    nz
}

//------------------------------------------------------------------------------
// DC-error diffusion

// Diffusion weights. We under-correct a bit (15/16th of the error is actually
// diffused) to avoid 'rainbow' chessboard pattern of blocks at q~=0.
/// fraction of error sent to the 4x4 block below
const C1: i32 = 7;
/// fraction of error sent to the 4x4 block on the right
const C2: i32 = 8;
const DSHIFT: i32 = 4;
/// storage descaling, needed to make the error fit int8_t
const DSCALE: i32 = 1;

/// Quantize as usual, but also compute and return the quantization error.
/// Error is already divided by DSHIFT.
/// Translation of `QuantizeSingle()`.
fn quantize_single(v: &mut i16, mtx: &VP8Matrix) -> i32 {
    let mut vv = *v as i32;
    let sign = vv < 0;
    if sign {
        vv = -vv;
    }
    if vv > mtx.zthresh[0] as i32 {
        let qv = quantdiv(vv as u32, mtx.iq[0] as u32, mtx.bias[0]) * mtx.q[0] as i32;
        let err = vv - qv;
        *v = (if sign { -qv } else { qv }) as i16;
        return (if sign { -err } else { err }) >> DSCALE;
    }
    *v = 0;
    (if sign { -vv } else { vv }) >> DSCALE
}

/// Translation of `CorrectDCValues()`.
fn correct_dc_values(
    it: &VP8EncIterator,
    enc: &VP8Encoder<'_>,
    mtx: &VP8Matrix,
    tmp: &mut [[i16; 16]; 8],
    rd: &mut VP8ModeScore,
) {
    //         | top[0] | top[1]
    // --------+--------+---------
    // left[0] | tmp[0]   tmp[1]  <->   err0 err1
    // left[1] | tmp[2]   tmp[3]        err2 err3
    //
    // Final errors {err1,err2,err3} are preserved and later restored
    // as top[]/left[] on the next block.
    for ch in 0..=1 {
        let top = enc.top_derr[it.x as usize][ch].map(|v| v as i32);
        let left = it.left_derr[ch].map(|v| v as i32);
        let c = &mut tmp[ch * 4..ch * 4 + 4];
        c[0][0] = (c[0][0] as i32 + ((C1 * top[0] + C2 * left[0]) >> (DSHIFT - DSCALE))) as i16;
        let err0 = quantize_single(&mut c[0][0], mtx);
        c[1][0] = (c[1][0] as i32 + ((C1 * top[1] + C2 * err0) >> (DSHIFT - DSCALE))) as i16;
        let err1 = quantize_single(&mut c[1][0], mtx);
        c[2][0] = (c[2][0] as i32 + ((C1 * err0 + C2 * left[1]) >> (DSHIFT - DSCALE))) as i16;
        let err2 = quantize_single(&mut c[2][0], mtx);
        c[3][0] = (c[3][0] as i32 + ((C1 * err1 + C2 * err2) >> (DSHIFT - DSCALE))) as i16;
        let err3 = quantize_single(&mut c[3][0], mtx);
        // error 'err' is bounded by mtx->q_[0] which is 132 at max. Hence
        // err >> DSCALE will fit in an int8_t type if DSCALE>=1.
        debug_assert!(err1.abs() <= 127 && err2.abs() <= 127 && err3.abs() <= 127);
        rd.derr[ch][0] = err1 as i8;
        rd.derr[ch][1] = err2 as i8;
        rd.derr[ch][2] = err3 as i8;
    }
}

/// Translation of `StoreDiffusionErrors()`.
fn store_diffusion_errors(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>, rd: &VP8ModeScore) {
    for ch in 0..=1 {
        let top = &mut enc.top_derr[it.x as usize][ch];
        let left = &mut it.left_derr[ch];
        left[0] = rd.derr[ch][0]; // restore err1
        left[1] = ((3 * rd.derr[ch][2] as i32) >> 2) as i8; //     ... 3/4th of err3
        top[0] = rd.derr[ch][1]; //     ... err2
        top[1] = (rd.derr[ch][2] as i32 - left[1] as i32) as i8; //     ... 1/4th of err3.
    }
}

//------------------------------------------------------------------------------

/// Translation of `ReconstructUV()`, writing into `it->yuv_out2_` (if
/// `to_out2`) or `it->yuv_out_`.
fn reconstruct_uv(
    it: &mut VP8EncIterator,
    enc: &VP8Encoder<'_>,
    rd: &mut VP8ModeScore,
    to_out2: bool,
    mode: usize,
) -> u32 {
    let dqm = &enc.dqm[enc.mb_info[it.mb].segment as usize];
    let mut nz: u32 = 0;
    let mut tmp = [[0i16; 16]; 8];

    {
        let ref_ = &it.yuv_p[VP8_UV_MODE_OFFSETS[mode]..];
        let src = &it.yuv_in[U_OFF_ENC..];
        for n in (0..8).step_by(2) {
            let s = VP8_SCAN_UV[n];
            vp8_ftransform2(&src[s..], &ref_[s..], tmp[n..n + 2].as_flattened_mut());
        }
    }
    if it.has_top_derr {
        correct_dc_values(it, enc, &dqm.uv, &mut tmp, rd);
    }

    // (DO_TRELLIS_UV is 0)
    for n in (0..8).step_by(2) {
        nz |= (vp8_enc_quantize2_blocks(
            tmp[n..n + 2].as_flattened_mut(),
            rd.uv_levels[n..n + 2].as_flattened_mut(),
            &dqm.uv,
        ) as u32)
            << n;
    }

    let ref_ = &it.yuv_p[VP8_UV_MODE_OFFSETS[mode]..];
    let yuv_out = if to_out2 {
        &mut it.yuv_out2
    } else {
        &mut it.yuv_out
    };
    let yuv_out = &mut yuv_out[U_OFF_ENC..];
    for n in (0..8).step_by(2) {
        let s = VP8_SCAN_UV[n];
        vp8_itransform(
            &ref_[s..],
            tmp[n..n + 2].as_flattened(),
            &mut yuv_out[s..],
            true,
        );
    }
    nz << 16
}

//------------------------------------------------------------------------------
// RD-opt decision. Reconstruct each modes, evalue distortion and bit-cost.
// Pick the mode is lower RD-cost = Rate + lambda * Distortion.

/// Translation of `StoreMaxDelta()`.
fn store_max_delta(dqm: &mut VP8SegmentInfo, dcs: &[i16; 16]) {
    // We look at the first three AC coefficients to determine what is the average
    // delta between each sub-4x4 block.
    let v0 = (dcs[1] as i32).abs();
    let v1 = (dcs[2] as i32).abs();
    let v2 = (dcs[4] as i32).abs();
    let mut max_v = if v1 > v0 { v1 } else { v0 };
    max_v = if v2 > max_v { v2 } else { max_v };
    if max_v > dqm.max_edge {
        dqm.max_edge = max_v;
    }
}

/// Translation of `SwapOut()`.
fn swap_out(it: &mut VP8EncIterator) {
    std::mem::swap(&mut it.yuv_out, &mut it.yuv_out2);
}

/// Translation of `PickBestIntra16()`: `rd_cur` and `rd_best` swap
/// between `rd` and a temporary score, as upstream's pointers do.
fn pick_best_intra16(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>, rd: &mut VP8ModeScore) {
    const K_NUM_BLOCKS: usize = 16;
    let segment = enc.mb_info[it.mb].segment as usize;
    let lambda = enc.dqm[segment].lambda_i16;
    let tlambda = enc.dqm[segment].tlambda;
    let mut rd_tmp = VP8ModeScore::default();
    // whether rd_best is rd_tmp (and rd_cur is rd)
    let mut best_is_tmp = false;
    let mut flat = is_flat_source16(&it.yuv_in[Y_OFF_ENC..]);

    rd.mode_i16 = -1;
    for mode in 0..NUM_PRED_MODES as usize {
        // scratch buffer: it->yuv_out2_ + Y_OFF_ENC
        let rd_cur = if best_is_tmp { &mut *rd } else { &mut rd_tmp };
        rd_cur.mode_i16 = mode as i32;

        // Reconstruct
        rd_cur.nz = reconstruct_intra16(it, enc, rd_cur, mode);

        // Measure RD-score
        let src = &it.yuv_in[Y_OFF_ENC..];
        let tmp_dst = &it.yuv_out2[Y_OFF_ENC..];
        rd_cur.d = vp8_sse16x16(src, tmp_dst) as ScoreT;
        rd_cur.sd = if tlambda != 0 {
            mult_8b(tlambda, vp8_tdisto16x16(src, tmp_dst, &K_WEIGHT_Y)) as ScoreT
        } else {
            0
        };
        rd_cur.h = VP8_FIXED_COSTS_I16[mode] as ScoreT;
        rd_cur.r = vp8_get_cost_luma16(it, enc, rd_cur) as ScoreT;
        if flat {
            // refine the first impression (which was in pixel space)
            flat = is_flat(
                rd_cur.y_ac_levels.as_flattened(),
                K_NUM_BLOCKS,
                FLATNESS_LIMIT_I16,
            );
            if flat {
                // Block is very flat. We put emphasis on the distortion being very low!
                rd_cur.d *= 2;
                rd_cur.sd *= 2;
            }
        }

        // Since we always examine Intra16 first, we can overwrite *rd directly.
        set_rd_score(lambda, rd_cur);
        let best_score = if best_is_tmp { rd_tmp.score } else { rd.score };
        let cur_score = if best_is_tmp { rd.score } else { rd_tmp.score };
        if mode == 0 || cur_score < best_score {
            best_is_tmp = !best_is_tmp;
            swap_out(it);
        }
    }
    if best_is_tmp {
        *rd = rd_tmp;
    }
    let dqm = &mut enc.dqm[segment];
    set_rd_score(dqm.lambda_mode, rd); // finalize score for mode decision.
    vp8_set_intra16_mode(it, enc, rd.mode_i16);

    // we have a blocky macroblock (only DCs are non-zero) with fairly high
    // distortion, record max delta so we can later adjust the minimal filtering
    // strength needed to smooth these blocks out.
    let dqm = &mut enc.dqm[segment];
    if (rd.nz & 0x100ffff) == 0x1000000 && rd.d > dqm.min_disto as ScoreT {
        store_max_delta(dqm, &rd.y_dc_levels);
    }
}

//------------------------------------------------------------------------------

/// return the cost array corresponding to the surrounding prediction modes.
/// Translation of `GetCostModeI4()`.
fn get_cost_mode_i4(
    it: &VP8EncIterator,
    enc: &VP8Encoder<'_>,
    modes: &[u8; 16],
) -> &'static [u16; 10] {
    let preds_w = enc.preds_w as usize;
    let x = (it.i4 & 3) as usize;
    let y = (it.i4 >> 2) as usize;
    let left = if x == 0 {
        enc.preds[it.preds + y * preds_w - 1]
    } else {
        modes[it.i4 as usize - 1]
    };
    let top = if y == 0 {
        enc.preds[it.preds - preds_w + x]
    } else {
        modes[it.i4 as usize - 4]
    };
    &VP8_FIXED_COSTS_I4[top as usize][left as usize]
}

/// Translation of `PickBestIntra4()`.
fn pick_best_intra4(
    it: &mut VP8EncIterator,
    enc: &mut VP8Encoder<'_>,
    rd: &mut VP8ModeScore,
) -> bool {
    let segment = enc.mb_info[it.mb].segment as usize;
    let dqm = enc.dqm[segment];
    let lambda = dqm.lambda_i4;
    let tlambda = dqm.tlambda;
    // src0 = it->yuv_in_ + Y_OFF_ENC; best_blocks = it->yuv_out2_ + Y_OFF_ENC
    let mut total_header_bits = 0;
    let mut rd_best = VP8ModeScore::default();

    if enc.max_i4_header_bits == 0 {
        return false;
    }

    init_score(&mut rd_best);
    rd_best.h = 211; // '211' is the value of VP8BitCost(0, 145)
    set_rd_score(dqm.lambda_mode, &mut rd_best);
    vp8_iterator_start_i4(it, enc);
    loop {
        const K_NUM_BLOCKS: usize = 1;
        let mut rd_i4 = VP8ModeScore::default();
        let mut best_mode: i32 = -1;
        let scan = VP8_SCAN[it.i4 as usize] as usize;
        let src = Y_OFF_ENC + scan;
        let mode_costs = get_cost_mode_i4(it, enc, &rd.modes_i4);
        // best_block = best_blocks + VP8Scan[it->i4_] and tmp_dst =
        // it->yuv_p_ + I4TMP (scratch buffer), swapped as pointers upstream:
        // whether tmp_dst is in yuv_p_.
        let mut tmp_in_p = true;

        init_score(&mut rd_i4);
        vp8_make_intra4_preds(it);
        for mode in 0..NUM_BMODES as usize {
            let mut rd_tmp = VP8ModeScore::default();
            let mut tmp_levels = [0i16; 16];
            let tmp_dst = if tmp_in_p { I4TMP } else { Y_OFF_ENC + scan };

            // Reconstruct
            rd_tmp.nz = (reconstruct_intra4(it, enc, &mut tmp_levels, src, tmp_in_p, tmp_dst, mode)
                as u32)
                << it.i4;

            // Compute RD-score
            let src_blk = &it.yuv_in[src..];
            let dst_blk = if tmp_in_p {
                &it.yuv_p[tmp_dst..]
            } else {
                &it.yuv_out2[tmp_dst..]
            };
            rd_tmp.d = vp8_sse4x4(src_blk, dst_blk) as ScoreT;
            rd_tmp.sd = if tlambda != 0 {
                mult_8b(tlambda, vp8_tdisto4x4(src_blk, dst_blk, &K_WEIGHT_Y)) as ScoreT
            } else {
                0
            };
            rd_tmp.h = mode_costs[mode] as ScoreT;

            // Add flatness penalty, to avoid flat area to be mispredicted
            // by a complex mode.
            if mode > 0 && is_flat(&tmp_levels, K_NUM_BLOCKS, FLATNESS_LIMIT_I4) {
                rd_tmp.r = FLATNESS_PENALTY * K_NUM_BLOCKS as ScoreT;
            } else {
                rd_tmp.r = 0;
            }

            // early-out check
            set_rd_score(lambda, &mut rd_tmp);
            if best_mode >= 0 && rd_tmp.score >= rd_i4.score {
                continue;
            }

            // finish computing score
            rd_tmp.r += vp8_get_cost_luma4(it, enc, &tmp_levels) as ScoreT;
            set_rd_score(lambda, &mut rd_tmp);

            if best_mode < 0 || rd_tmp.score < rd_i4.score {
                copy_score(&mut rd_i4, &rd_tmp);
                best_mode = mode as i32;
                tmp_in_p = !tmp_in_p;
                rd_best.y_ac_levels[it.i4 as usize] = tmp_levels;
            }
        }
        set_rd_score(dqm.lambda_mode, &mut rd_i4);
        add_score(&mut rd_best, &rd_i4);
        if rd_best.score >= rd.score {
            return false;
        }
        total_header_bits += rd_i4.h as i32; // <- equal to mode_costs[best_mode];
        if total_header_bits > enc.max_i4_header_bits {
            return false;
        }
        // Copy selected samples if not in the right place already.
        if !tmp_in_p {
            // (best_block is in yuv_p_)
            let (p, out2) = (&it.yuv_p[I4TMP..], &mut it.yuv_out2[Y_OFF_ENC + scan..]);
            vp8_copy4x4(p, out2);
        }
        rd.modes_i4[it.i4 as usize] = best_mode as u8;
        let nz = (rd_i4.nz != 0) as i32;
        it.top_nz[(it.i4 & 3) as usize] = nz;
        it.left_nz[(it.i4 >> 2) as usize] = nz;
        if !vp8_iterator_rotate_i4(it, false) {
            break;
        }
    }

    // finalize state
    copy_score(rd, &rd_best);
    vp8_set_intra4_mode(it, enc, &rd.modes_i4);
    swap_out(it);
    rd.y_ac_levels = rd_best.y_ac_levels;
    true // select intra4x4 over intra16x16
}

//------------------------------------------------------------------------------

/// Translation of `PickBestUV()`.
fn pick_best_uv(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>, rd: &mut VP8ModeScore) {
    const K_NUM_BLOCKS: usize = 8;
    let segment = enc.mb_info[it.mb].segment as usize;
    let lambda = enc.dqm[segment].lambda_uv;
    // tmp_dst = it->yuv_out2_ + U_OFF_ENC (scratch buffer) and
    // dst = dst0 = it->yuv_out_ + U_OFF_ENC, swapped as pointers upstream:
    // whether tmp_dst is in yuv_out2_.
    let mut tmp_in_out2 = true;
    let mut rd_best = VP8ModeScore::default();

    rd.mode_uv = -1;
    init_score(&mut rd_best);
    for mode in 0..NUM_PRED_MODES as usize {
        let mut rd_uv = VP8ModeScore::default();

        // Reconstruct
        rd_uv.nz = reconstruct_uv(it, enc, &mut rd_uv, tmp_in_out2, mode);

        // Compute RD-score
        let tmp_dst = if tmp_in_out2 {
            &it.yuv_out2
        } else {
            &it.yuv_out
        };
        rd_uv.d = vp8_sse16x8(&it.yuv_in[U_OFF_ENC..], &tmp_dst[U_OFF_ENC..]) as ScoreT;
        rd_uv.sd = 0; // not calling TDisto here: it tends to flatten areas.
        rd_uv.h = VP8_FIXED_COSTS_UV[mode] as ScoreT;
        rd_uv.r = vp8_get_cost_uv(it, enc, &rd_uv) as ScoreT;
        if mode > 0
            && is_flat(
                rd_uv.uv_levels.as_flattened(),
                K_NUM_BLOCKS,
                FLATNESS_LIMIT_UV,
            )
        {
            rd_uv.r += FLATNESS_PENALTY * K_NUM_BLOCKS as ScoreT;
        }

        set_rd_score(lambda, &mut rd_uv);
        if mode == 0 || rd_uv.score < rd_best.score {
            copy_score(&mut rd_best, &rd_uv);
            rd.mode_uv = mode as i32;
            rd.uv_levels = rd_uv.uv_levels;
            if it.has_top_derr {
                rd.derr = rd_uv.derr;
            }
            tmp_in_out2 = !tmp_in_out2;
        }
    }
    vp8_set_intra_uv_mode(it, enc, rd.mode_uv);
    add_score(rd, &rd_best);
    if !tmp_in_out2 {
        // copy 16x8 block if needed (dst is in yuv_out2_)
        let (out, out2) = (&mut it.yuv_out, &it.yuv_out2);
        vp8_copy16x8(&out2[U_OFF_ENC..], &mut out[U_OFF_ENC..]);
    }
    if it.has_top_derr {
        // store diffusion errors for next block
        store_diffusion_errors(it, enc, rd);
    }
}

//------------------------------------------------------------------------------
// Final reconstruction and quantization.

// (SimpleQuantize() and RefineUsingDistortion() are not translated.)

//------------------------------------------------------------------------------
// Entry point

/// Pick best modes and fills the levels. Returns true if skipped.
/// Translation of `VP8Decimate()`.
pub(crate) fn vp8_decimate(
    it: &mut VP8EncIterator,
    enc: &mut VP8Encoder<'_>,
    rd: &mut VP8ModeScore,
    rd_opt: VP8RDLevel,
) -> bool {
    let method = enc.method;

    init_score(rd);

    // We can perform predictions for Luma16x16 and Chroma8x8 already.
    // Luma4x4 predictions needs to be done as-we-go.
    vp8_make_luma16_preds(it, enc);
    vp8_make_chroma8_preds(it, enc);

    debug_assert!(
        rd_opt == VP8RDLevel::Basic,
        "only the basic RD-opt level (methods 3 and 4) is translated"
    );
    it.do_trellis = rd_opt >= VP8RDLevel::TrellisAll;
    pick_best_intra16(it, enc, rd);
    if method >= 2 {
        pick_best_intra4(it, enc, rd);
    }
    pick_best_uv(it, enc, rd);
    let is_skipped = rd.nz == 0;
    vp8_set_skip(it, enc, is_skipped);
    is_skipped
}
