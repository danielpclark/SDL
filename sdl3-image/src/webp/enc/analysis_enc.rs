// Rust translation of src/enc/analysis_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Macroblock analysis. The segment job runs on the calling thread, as
//! upstream's does without `thread_level`.

use crate::webp::dec::NUM_MB_SEGMENTS;
use crate::webp::dsp::enc::{vp8_collect_histogram, vp8_mean16x4, VP8Histogram};
use crate::webp::dsp::BPS;
use crate::webp::enc::iterator_enc::{
    vp8_iterator_import, vp8_iterator_init, vp8_iterator_is_done, vp8_iterator_next,
    vp8_iterator_set_count_down, vp8_iterator_set_row, vp8_set_intra16_mode, vp8_set_intra4_mode,
    vp8_set_intra_uv_mode, vp8_set_segment, vp8_set_skip,
};
use crate::webp::enc::quant_enc::{
    vp8_make_chroma8_preds, vp8_make_luma16_preds, VP8_I16_MODE_OFFSETS, VP8_UV_MODE_OFFSETS,
};
use crate::webp::enc::vp8i_enc::{VP8EncIterator, VP8Encoder, VP8MBInfo, U_OFF_ENC, Y_OFF_ENC};

const MAX_ITERS_K_MEANS: usize = 6;

//------------------------------------------------------------------------------
// Smooth the segment map by replacing isolated block by the majority of its
// neighbours.

/// Translation of `SmoothSegmentMap()`.
fn smooth_segment_map(enc: &mut VP8Encoder<'_>) {
    let w = enc.mb_w as usize;
    let h = enc.mb_h as usize;
    let majority_cnt_3_x_3_grid = 5;
    let mut tmp = vec![0u8; w * h];

    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let mut cnt = [0i32; NUM_MB_SEGMENTS];
            let i = x + w * y;
            let seg = |k: usize| enc.mb_info[k].segment as usize;
            let mut majority_seg = enc.mb_info[i].segment;
            // Check the 8 neighbouring segment values.
            cnt[seg(i - w - 1)] += 1; // top-left
            cnt[seg(i - w)] += 1; // top
            cnt[seg(i - w + 1)] += 1; // top-right
            cnt[seg(i - 1)] += 1; // left
            cnt[seg(i + 1)] += 1; // right
            cnt[seg(i + w - 1)] += 1; // bottom-left
            cnt[seg(i + w)] += 1; // bottom
            cnt[seg(i + w + 1)] += 1; // bottom-right
            for (n, &c) in cnt.iter().enumerate() {
                if c >= majority_cnt_3_x_3_grid {
                    majority_seg = n as u8;
                    break;
                }
            }
            tmp[x + y * w] = majority_seg;
        }
    }
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            enc.mb_info[x + w * y].segment = tmp[x + y * w];
        }
    }
}

//------------------------------------------------------------------------------
// set segment susceptibility alpha_ / beta_

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

/// Translation of `SetSegmentAlphas()`.
fn set_segment_alphas(enc: &mut VP8Encoder<'_>, centers: &[i32; NUM_MB_SEGMENTS], mid: i32) {
    let nb = enc.segment_hdr.num_segments as usize;
    let mut min = centers[0];
    let mut max = centers[0];

    if nb > 1 {
        for &c in &centers[..nb] {
            if min > c {
                min = c;
            }
            if max < c {
                max = c;
            }
        }
    }
    if max == min {
        max = min + 1;
    }
    debug_assert!(mid <= max && mid >= min);
    for n in 0..nb {
        let alpha = 255 * (centers[n] - mid) / (max - min);
        let beta = 255 * (centers[n] - min) / (max - min);
        enc.dqm[n].alpha = clip(alpha, -127, 127);
        enc.dqm[n].beta = clip(beta, 0, 255);
    }
}

//------------------------------------------------------------------------------
// Compute susceptibility based on DCT-coeff histograms:
// the higher, the "easier" the macroblock is to compress.

/// 8b of precision for susceptibilities.
const MAX_ALPHA: i32 = 255;
/// scaling factor for alpha.
const ALPHA_SCALE: i32 = 2 * MAX_ALPHA;
const DEFAULT_ALPHA: i32 = -1;

/// Translation of the `IS_BETTER_ALPHA()` macro.
fn is_better_alpha(alpha: i32, best_alpha: i32) -> bool {
    alpha > best_alpha
}

/// Translation of `FinalAlphaValue()`.
fn final_alpha_value(alpha: i32) -> i32 {
    let alpha = MAX_ALPHA - alpha;
    clip(alpha, 0, MAX_ALPHA)
}

/// Translation of `GetAlpha()`.
fn get_alpha(histo: &VP8Histogram) -> i32 {
    // 'alpha' will later be clipped to [0..MAX_ALPHA] range, clamping outer
    // values which happen to be mostly noise. This leaves the maximum precision
    // for handling the useful small values which contribute most.
    let max_value = histo.max_value;
    let last_non_zero = histo.last_non_zero;
    if max_value > 1 {
        ALPHA_SCALE * last_non_zero / max_value
    } else {
        0
    }
}

/// Translation of `InitHistogram()`.
fn init_histogram(histo: &mut VP8Histogram) {
    histo.max_value = 0;
    histo.last_non_zero = 1;
}

//------------------------------------------------------------------------------
// Simplified k-Means, to assign Nb segments based on alpha-histogram

/// Translation of `AssignSegments()`.
fn assign_segments(enc: &mut VP8Encoder<'_>, alphas: &[i32; MAX_ALPHA as usize + 1]) {
    // 'num_segments_' is previously validated and <= NUM_MB_SEGMENTS, but an
    // explicit check is needed to avoid spurious warning about 'n + 1' exceeding
    // array bounds of 'centers' with some compilers (noticed with gcc-4.9).
    let nb = (enc.segment_hdr.num_segments as usize).min(NUM_MB_SEGMENTS);
    let mut centers = [0i32; NUM_MB_SEGMENTS];
    let mut weighted_average = 0;
    let mut map = [0usize; MAX_ALPHA as usize + 1];
    // 'int' type is ok for histo, and won't overflow
    let mut accum = [0i32; NUM_MB_SEGMENTS];
    let mut dist_accum = [0i32; NUM_MB_SEGMENTS];

    debug_assert!(nb >= 1);
    debug_assert!(nb <= NUM_MB_SEGMENTS);

    // bracket the input
    let mut n = 0;
    while n <= MAX_ALPHA as usize && alphas[n] == 0 {
        n += 1;
    }
    let min_a = n;
    n = MAX_ALPHA as usize;
    while n > min_a && alphas[n] == 0 {
        n -= 1;
    }
    let max_a = n;
    let range_a = max_a as i32 - min_a as i32;

    // Spread initial centers evenly
    let mut n = 1;
    for c in centers.iter_mut().take(nb) {
        debug_assert!(n < 2 * nb as i32);
        *c = min_a as i32 + (n * range_a) / (2 * nb as i32);
        n += 2;
    }

    for _ in 0..MAX_ITERS_K_MEANS {
        // few iters are enough
        // Reset stats
        for n in 0..nb {
            accum[n] = 0;
            dist_accum[n] = 0;
        }
        // Assign nearest center for each 'a'
        let mut n = 0; // track the nearest center for current 'a'
        for a in min_a..=max_a {
            if alphas[a] != 0 {
                while n + 1 < nb
                    && (a as i32 - centers[n + 1]).abs() < (a as i32 - centers[n]).abs()
                {
                    n += 1;
                }
                map[a] = n;
                // accumulate contribution into best centroid
                dist_accum[n] += a as i32 * alphas[a];
                accum[n] += alphas[a];
            }
        }
        // All point are classified. Move the centroids to the
        // center of their respective cloud.
        let mut displaced = 0;
        weighted_average = 0;
        let mut total_weight = 0;
        for n in 0..nb {
            if accum[n] != 0 {
                let new_center = (dist_accum[n] + accum[n] / 2) / accum[n];
                displaced += (centers[n] - new_center).abs();
                centers[n] = new_center;
                weighted_average += new_center * accum[n];
                total_weight += accum[n];
            }
        }
        weighted_average = (weighted_average + total_weight / 2) / total_weight;
        if displaced < 5 {
            break; // no need to keep on looping...
        }
    }

    // Map each original value to the closest centroid
    for mb in enc.mb_info.iter_mut() {
        let alpha = mb.alpha as usize;
        mb.segment = map[alpha] as u8;
        mb.alpha = centers[map[alpha]] as u8; // for the record.
    }

    if nb > 1 {
        let smooth = (enc.config.preprocessing & 1) != 0;
        if smooth {
            smooth_segment_map(enc);
        }
    }

    set_segment_alphas(enc, &centers, weighted_average); // pick some alphas.
}

//------------------------------------------------------------------------------
// Macroblock analysis: collect histogram for each mode, deduce the maximal
// susceptibility and set best modes for this macroblock.
// Segment assignment is done later.

// Number of modes to inspect for alpha_ evaluation. We don't need to test all
// the possible modes during the analysis phase: we risk falling into a local
// optimum, or be subject to boundary effect
const MAX_INTRA16_MODE: usize = 2;
const MAX_UV_MODE: usize = 2;

/// Translation of `MBAnalyzeBestIntra16Mode()`.
fn mb_analyze_best_intra16_mode(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>) -> i32 {
    let max_mode = MAX_INTRA16_MODE;
    let mut best_alpha = DEFAULT_ALPHA;
    let mut best_mode = 0;

    vp8_make_luma16_preds(it, enc);
    for mode in 0..max_mode {
        let mut histo = VP8Histogram::default();

        init_histogram(&mut histo);
        vp8_collect_histogram(
            &it.yuv_in[Y_OFF_ENC..],
            &it.yuv_p[VP8_I16_MODE_OFFSETS[mode]..],
            0,
            16,
            &mut histo,
        );
        let alpha = get_alpha(&histo);
        if is_better_alpha(alpha, best_alpha) {
            best_alpha = alpha;
            best_mode = mode;
        }
    }
    vp8_set_intra16_mode(it, enc, best_mode as i32);
    best_alpha
}

/// Translation of `FastMBAnalyze()`.
fn fast_mb_analyze(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>) -> i32 {
    // Empirical cut-off value, should be around 16 (~=block size). We use the
    // [8-17] range and favor intra4 at high quality, intra16 for low quality.
    let q = enc.config.quality as i32;
    let k_threshold = (8 + (17 - 8) * q / 100) as u32;
    let mut dc = [0u32; 16];
    for k in (0..16).step_by(4) {
        let mut d = [0u32; 4];
        vp8_mean16x4(&it.yuv_in[Y_OFF_ENC + k * BPS..], &mut d);
        dc[k..k + 4].copy_from_slice(&d);
    }
    let mut m: u32 = 0;
    let mut m2: u32 = 0;
    for &d in &dc {
        m = m.wrapping_add(d);
        m2 = m2.wrapping_add(d.wrapping_mul(d));
    }
    if k_threshold.wrapping_mul(m2) < m.wrapping_mul(m) {
        vp8_set_intra16_mode(it, enc, 0); // DC16
    } else {
        let modes = [0u8; 16]; // DC4
        vp8_set_intra4_mode(it, enc, &modes);
    }
    0
}

/// Translation of `MBAnalyzeBestUVMode()`.
fn mb_analyze_best_uv_mode(it: &mut VP8EncIterator, enc: &mut VP8Encoder<'_>) -> i32 {
    let mut best_alpha = DEFAULT_ALPHA;
    let mut smallest_alpha = 0;
    let mut best_mode = 0;
    let max_mode = MAX_UV_MODE;

    vp8_make_chroma8_preds(it, enc);
    for mode in 0..max_mode {
        let mut histo = VP8Histogram::default();
        init_histogram(&mut histo);
        vp8_collect_histogram(
            &it.yuv_in[U_OFF_ENC..],
            &it.yuv_p[VP8_UV_MODE_OFFSETS[mode]..],
            16,
            16 + 4 + 4,
            &mut histo,
        );
        let alpha = get_alpha(&histo);
        if is_better_alpha(alpha, best_alpha) {
            best_alpha = alpha;
        }
        // The best prediction mode tends to be the one with the smallest alpha.
        if mode == 0 || alpha < smallest_alpha {
            smallest_alpha = alpha;
            best_mode = mode;
        }
    }
    vp8_set_intra_uv_mode(it, enc, best_mode as i32);
    best_alpha
}

/// Translation of `MBAnalyze()`.
fn mb_analyze(
    it: &mut VP8EncIterator,
    enc: &mut VP8Encoder<'_>,
    alphas: &mut [i32; MAX_ALPHA as usize + 1],
    alpha: &mut i32,
    uv_alpha: &mut i32,
) {
    vp8_set_intra16_mode(it, enc, 0); // default: Intra16, DC_PRED
    vp8_set_skip(it, enc, false); // not skipped
    vp8_set_segment(it, enc, 0); // default segment, spec-wise.

    let mut best_alpha = if enc.method <= 1 {
        fast_mb_analyze(it, enc)
    } else {
        mb_analyze_best_intra16_mode(it, enc)
    };
    let best_uv_alpha = mb_analyze_best_uv_mode(it, enc);

    // Final susceptibility mix
    best_alpha = (3 * best_alpha + best_uv_alpha + 2) >> 2;
    best_alpha = final_alpha_value(best_alpha);
    alphas[best_alpha as usize] += 1;
    enc.mb_info[it.mb].alpha = best_alpha as u8; // for later remapping.

    // Accumulate for later complexity analysis.
    *alpha += best_alpha; // mixed susceptibility (not just luma)
    *uv_alpha += best_uv_alpha;
}

/// Translation of `DefaultMBInfo()`.
fn default_mb_info(mb: &mut VP8MBInfo) {
    mb.type_ = 1; // I16x16
    mb.uv_mode = 0;
    mb.skip = false; // not skipped
    mb.segment = 0; // default segment
    mb.alpha = 0;
}

//------------------------------------------------------------------------------
// Main analysis loop:
// Collect all susceptibilities for each macroblock and record their
// distribution in alphas[]. Segments is assigned a-posteriori, based on
// this histogram.
// We also pick an intra16 prediction mode, which shouldn't be considered
// final except for fast-encode settings. We can also pick some intra4 modes
// and decide intra4/intra16, but that's usually almost always a bad choice at
// this stage.

/// Translation of `ResetAllMBInfo()`.
fn reset_all_mb_info(enc: &mut VP8Encoder<'_>) {
    for mb in enc.mb_info.iter_mut() {
        default_mb_info(mb);
    }
    // Default susceptibilities.
    enc.dqm[0].alpha = 0;
    enc.dqm[0].beta = 0;
    // Note: we can't compute this alpha_ / uv_alpha_ -> set to default value.
    enc.alpha = 0;
    enc.uv_alpha = 0;
}

/// struct used to collect job result. Translation of `SegmentJob`.
struct SegmentJob {
    alphas: [i32; MAX_ALPHA as usize + 1],
    alpha: i32,
    uv_alpha: i32,
    it: VP8EncIterator,
}

/// main work call. Translation of `DoSegmentsJob()`.
fn do_segments_job(job: &mut SegmentJob, enc: &mut VP8Encoder<'_>) -> bool {
    let it = &mut job.it;
    if !vp8_iterator_is_done(it) {
        loop {
            // Let's pretend we have perfect lossless reconstruction.
            vp8_iterator_import(it, enc, true);
            mb_analyze(it, enc, &mut job.alphas, &mut job.alpha, &mut job.uv_alpha);
            if !vp8_iterator_next(it, enc) {
                break;
            }
        }
    }
    true
}

/// initialize the job struct with some tasks to perform
/// Translation of `InitSegmentJob()`.
fn init_segment_job(enc: &mut VP8Encoder<'_>, start_row: i32, end_row: i32) -> SegmentJob {
    let mut it = vp8_iterator_init(enc);
    vp8_iterator_set_row(&mut it, enc, start_row);
    vp8_iterator_set_count_down(&mut it, (end_row - start_row) * enc.mb_w);
    SegmentJob {
        alphas: [0; MAX_ALPHA as usize + 1],
        alpha: 0,
        uv_alpha: 0,
        it,
    }
}

/// Main analysis loop. Decides the segmentations and complexity.
/// Assigns a first guess for Intra16 and uvmode_ prediction modes.
/// Translation of `VP8EncAnalyze()`.
pub(crate) fn vp8_enc_analyze(enc: &mut VP8Encoder<'_>) -> bool {
    let mut ok = true;
    let do_segments = enc.config.emulate_jpeg_size != 0 // We need the complexity evaluation.
        || (enc.segment_hdr.num_segments > 1)
        || (enc.method <= 1); // for method 0 - 1, we need preds_[] to be filled.
    if do_segments {
        let last_row = enc.mb_h;
        let total_mb = last_row * enc.mb_w;
        let mut main_job = init_segment_job(enc, 0, last_row);
        ok &= do_segments_job(&mut main_job, enc);
        if ok {
            enc.alpha = main_job.alpha / total_mb;
            enc.uv_alpha = main_job.uv_alpha / total_mb;
            assign_segments(enc, &main_job.alphas);
        }
    } else {
        // Use only one default segment.
        reset_all_mb_info(enc);
    }
    ok
}
