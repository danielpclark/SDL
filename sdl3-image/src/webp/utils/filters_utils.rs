// Rust translation of src/utils/filters_utils.c and filters_utils.h from
// libwebp (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! filter estimation

use crate::webp::dsp::{WebpFilterType, WEBP_FILTER_LAST};

// -----------------------------------------------------------------------------
// Quick estimate of a potentially interesting filter mode to try.

const SMAX: usize = 16;

/// Scoring diff, in [0..SMAX). Translation of `SDIFF()`.
fn sdiff(a: i32, b: i32) -> usize {
    ((a - b).abs() >> 4) as usize
}

/// Translation of `GradientPredictor()`.
fn gradient_predictor(a: u8, b: u8, c: u8) -> i32 {
    let g = a as i32 + b as i32 - c as i32;
    if (g & !0xff) == 0 {
        g
    } else if g < 0 {
        0
    } else {
        255 // clip to 8bit
    }
}

/// Fast estimate of a potentially good filter. Translation of
/// `WebPEstimateBestFilter()`.
pub(crate) fn webp_estimate_best_filter(
    data: &[u8],
    width: i32,
    height: i32,
    stride: i32,
) -> WebpFilterType {
    let mut bins = [[0i32; SMAX]; WEBP_FILTER_LAST as usize];

    // We only sample every other pixels. That's enough.
    let mut j = 2;
    while j < height - 1 {
        let p = (j * stride) as usize;
        let mut mean = data[p] as i32;
        let mut i = 2;
        while i < width - 1 {
            let pi = p + i as usize;
            // (the row above is at -width: upstream assumes stride == width,
            // as the alpha plane it is called on has)
            let up = pi - width as usize;
            let diff0 = sdiff(data[pi] as i32, mean);
            let diff1 = sdiff(data[pi] as i32, data[pi - 1] as i32);
            let diff2 = sdiff(data[pi] as i32, data[up] as i32);
            let grad_pred = gradient_predictor(data[pi - 1], data[up], data[up - 1]);
            let diff3 = sdiff(data[pi] as i32, grad_pred);
            bins[WebpFilterType::None as usize][diff0] = 1;
            bins[WebpFilterType::Horizontal as usize][diff1] = 1;
            bins[WebpFilterType::Vertical as usize][diff2] = 1;
            bins[WebpFilterType::Gradient as usize][diff3] = 1;
            mean = (3 * mean + data[pi] as i32 + 2) >> 2;
            i += 2;
        }
        j += 2;
    }
    {
        const FILTERS: [WebpFilterType; 4] = [
            WebpFilterType::None,
            WebpFilterType::Horizontal,
            WebpFilterType::Vertical,
            WebpFilterType::Gradient,
        ];
        let mut best_filter = WebpFilterType::None;
        let mut best_score = 0x7fffffff;
        for filter in FILTERS {
            let mut score = 0;
            for (i, &b) in bins[filter as usize].iter().enumerate() {
                if b > 0 {
                    score += i as i32;
                }
            }
            if score < best_score {
                best_score = score;
                best_filter = filter;
            }
        }
        best_filter
    }
}
