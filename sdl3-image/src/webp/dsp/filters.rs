// Rust translation of src/dsp/filters.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the unfilters.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Spatial prediction using various filters: the inverse filters the alpha
//! plane decoder applies (the forward filters are the encoder's).
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
