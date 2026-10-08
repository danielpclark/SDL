// Rust translation of src/dsp/filters.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Spatial prediction using various filters: the forward filters of the
//! alpha plane encoder and the inverse filters its decoder applies.
//!
//! Upstream's unfilters read a row `in` and write it to `out`, which may be
//! the same memory; here the row is unfiltered in place in `plane` at
//! `out` (the caller copies its input there first, which gives the same
//! result: each sample is read before it is written), with the previous
//! row, if any, at `prev` in the same plane.

use crate::webp::dsp::WebpFilterType;

/// Translation of `GradientPredictor_C()`.
fn gradient_predictor(a: u8, b: u8, c: u8) -> u8 {
    let g = a as i32 + b as i32 - c as i32;
    if (g & !0xff) == 0 {
        g as u8
    } else if g < 0 {
        0
    } else {
        255
    } // clip to 8bit
}

/// Translation of `PredictLine_C()` (forward: `inverse` is 0).
fn predict_line(src: &[u8], pred: &[u8], dst: &mut [u8], length: usize) {
    for i in 0..length {
        dst[i] = src[i].wrapping_sub(pred[i]);
    }
}

//------------------------------------------------------------------------------
// Horizontal filter.

/// Translation of `DoHorizontalFilter_C()` (forward, on the whole image:
/// `row` 0, `num_rows` the height).
fn do_horizontal_filter(input: &[u8], width: usize, height: usize, stride: usize, out: &mut [u8]) {
    // Leftmost pixel is the same as input for topmost scanline.
    out[0] = input[0];
    predict_line(&input[1..], input, &mut out[1..], width - 1);
    let mut row = 1;
    let mut off = stride;

    // Filter line-by-line.
    while row < height {
        // Leftmost pixel is predicted from above.
        predict_line(&input[off..], &input[off - stride..], &mut out[off..], 1);
        predict_line(
            &input[off + 1..],
            &input[off..],
            &mut out[off + 1..],
            width - 1,
        );
        row += 1;
        off += stride;
    }
}

//------------------------------------------------------------------------------
// Vertical filter.

/// Translation of `DoVerticalFilter_C()` (forward, on the whole image).
fn do_vertical_filter(input: &[u8], width: usize, height: usize, stride: usize, out: &mut [u8]) {
    // Very first top-left pixel is copied.
    out[0] = input[0];
    // Rest of top scan-line is left-predicted.
    predict_line(&input[1..], input, &mut out[1..], width - 1);
    let mut row = 1;
    let mut off = stride;

    // Filter line-by-line.
    while row < height {
        predict_line(
            &input[off..],
            &input[off - stride..],
            &mut out[off..],
            width,
        );
        row += 1;
        off += stride;
    }
}

//------------------------------------------------------------------------------
// Gradient filter.

/// Translation of `DoGradientFilter_C()` (forward, on the whole image).
fn do_gradient_filter(input: &[u8], width: usize, height: usize, stride: usize, out: &mut [u8]) {
    // left prediction for top scan-line
    out[0] = input[0];
    predict_line(&input[1..], input, &mut out[1..], width - 1);
    let mut row = 1;
    let mut off = stride;

    // Filter line-by-line.
    while row < height {
        // leftmost pixel: predict from above.
        predict_line(&input[off..], &input[off - stride..], &mut out[off..], 1);
        for w in 1..width {
            let p = off + w;
            let pred = gradient_predictor(input[p - 1], input[p - stride], input[p - stride - 1]);
            out[p] = input[p].wrapping_sub(pred);
        }
        row += 1;
        off += stride;
    }
}

//------------------------------------------------------------------------------

/// Translation of `WebPFilters[filter](data, width, height, stride,
/// filtered_data)`: `HorizontalFilter_C()`, `VerticalFilter_C()` and
/// `GradientFilter_C()` (`WebPFilters[WEBP_FILTER_NONE]` is NULL: its
/// callers copy the data instead).
pub(crate) fn webp_filter(
    filter: WebpFilterType,
    data: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    filtered_data: &mut [u8],
) {
    match filter {
        WebpFilterType::None => unreachable!("WebPFilters[WEBP_FILTER_NONE] is NULL"),
        WebpFilterType::Horizontal => {
            do_horizontal_filter(data, width, height, stride, filtered_data)
        }
        WebpFilterType::Vertical => do_vertical_filter(data, width, height, stride, filtered_data),
        WebpFilterType::Gradient => do_gradient_filter(data, width, height, stride, filtered_data),
    }
}

//------------------------------------------------------------------------------

/// Translation of `HorizontalUnfilter_C()`.
fn horizontal_unfilter(plane: &mut [u8], prev: Option<usize>, out: usize, width: usize) {
    let mut pred: u8 = match prev {
        None => 0,
        Some(prev) => plane[prev],
    };
    for i in 0..width {
        plane[out + i] = pred.wrapping_add(plane[out + i]);
        pred = plane[out + i];
    }
}

/// Translation of `VerticalUnfilter_C()`.
fn vertical_unfilter(plane: &mut [u8], prev: Option<usize>, out: usize, width: usize) {
    match prev {
        None => horizontal_unfilter(plane, None, out, width),
        Some(prev) => {
            for i in 0..width {
                plane[out + i] = plane[prev + i].wrapping_add(plane[out + i]);
            }
        }
    }
}

/// Translation of `GradientUnfilter_C()`.
fn gradient_unfilter(plane: &mut [u8], prev: Option<usize>, out: usize, width: usize) {
    match prev {
        None => horizontal_unfilter(plane, None, out, width),
        Some(prev) => {
            let mut top = plane[prev];
            let mut top_left = top;
            let mut left = top;
            for i in 0..width {
                top = plane[prev + i]; // need to read this first, in case prev==out
                left = plane[out + i].wrapping_add(gradient_predictor(left, top, top_left));
                top_left = top;
                plane[out + i] = left;
            }
        }
    }
}

/// Translation of `WebPUnfilters[filter](prev, out, out, width)`.
pub(crate) fn webp_unfilter(
    filter: WebpFilterType,
    plane: &mut [u8],
    prev: Option<usize>,
    out: usize,
    width: usize,
) {
    match filter {
        WebpFilterType::None => {}
        WebpFilterType::Horizontal => horizontal_unfilter(plane, prev, out, width),
        WebpFilterType::Vertical => vertical_unfilter(plane, prev, out, width),
        WebpFilterType::Gradient => gradient_unfilter(plane, prev, out, width),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfilters_invert_the_predictions() {
        // two rows of three samples: the first stored as horizontal
        // deltas, the second as deltas of each filter's prediction
        let first = [10u8, 20, 30];
        let mut plane = vec![10u8, 10, 10, 1, 2, 3];
        horizontal_unfilter(&mut plane, None, 0, 3);
        assert_eq!(plane[..3], first);
        vertical_unfilter(&mut plane, Some(0), 3, 3);
        assert_eq!(plane[3..], [11, 22, 33]);
        let mut plane = vec![10u8, 20, 30, 1, 1, 1];
        gradient_unfilter(&mut plane, Some(0), 3, 3);
        // left starts at prev[0]: 10 + clip(10 + 10 - 10) = 11...
        assert_eq!(plane[3], 11);
        assert_eq!(plane[4], 1u8.wrapping_add(gradient_predictor(11, 20, 10)));
        assert_eq!(gradient_predictor(250, 250, 0), 255);
        assert_eq!(gradient_predictor(0, 0, 250), 0);
    }
}
