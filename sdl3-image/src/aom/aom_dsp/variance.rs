// Rust translation of aom_dsp/variance.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The 10-bit variance (`aom_highbd_10_variance<W>x<H>_c()`, over any
//! size here), the only variance the encoder reaches: the variance-based
//! partitioning of 10-bit sources.

use super::aom_dsp_common::{round_power_of_two_i64, round_power_of_two_u64};

fn highbd_variance64(
    a: &[u16],
    a_off: usize,
    a_stride: usize,
    b: &[u16],
    b_off: usize,
    b_stride: usize,
    w: i32,
    h: i32,
    sse: &mut u64,
    sum: &mut i64,
) {
    let mut tsum: i64 = 0;
    let mut tsse: u64 = 0;
    let (mut ao, mut bo) = (a_off, b_off);
    for _i in 0..h {
        let mut lsum: i32 = 0;
        for j in 0..w as usize {
            let diff = a[ao + j] as i32 - b[bo + j] as i32;
            lsum += diff;
            tsse += (diff * diff) as u32 as u64;
        }
        tsum += lsum as i64;
        ao += a_stride;
        bo += b_stride;
    }
    *sum = tsum;
    *sse = tsse;
}

fn highbd_10_variance(
    a: &[u16],
    a_off: usize,
    a_stride: usize,
    b: &[u16],
    b_off: usize,
    b_stride: usize,
    w: i32,
    h: i32,
    sse: &mut u32,
    sum: &mut i32,
) {
    let mut sse_long: u64 = 0;
    let mut sum_long: i64 = 0;
    highbd_variance64(
        a,
        a_off,
        a_stride,
        b,
        b_off,
        b_stride,
        w,
        h,
        &mut sse_long,
        &mut sum_long,
    );
    *sse = round_power_of_two_u64(sse_long, 4) as u32;
    *sum = round_power_of_two_i64(sum_long, 2) as i32;
}

/// Translation of `aom_highbd_10_variance<W>x<H>_c()`.
pub(crate) fn aom_highbd_10_variance(
    a: &[u16],
    a_off: usize,
    a_stride: usize,
    b: &[u16],
    b_off: usize,
    b_stride: usize,
    w: i32,
    h: i32,
    sse: &mut u32,
) -> u32 {
    let mut sum: i32 = 0;
    let var: i64;
    highbd_10_variance(a, a_off, a_stride, b, b_off, b_stride, w, h, sse, &mut sum);
    var = *sse as i64 - ((sum as i64 * sum as i64) / (w * h) as i64);
    if var >= 0 {
        var as u32
    } else {
        0
    }
}
