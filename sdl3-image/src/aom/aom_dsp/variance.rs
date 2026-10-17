// Rust translation of aom_dsp/variance.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Variances, over any block size here (upstream's
//! `aom_variance<W>x<H>_c()`, `aom_sub_pixel_variance<W>x<H>_c()` and their
//! `aom_highbd_8_` and `aom_highbd_10_` versions), and the
//! `aom_variance_fn_ptr_t` table the encoder calls them through. The
//! samples are kept in 16 bits for any bit depth.

use super::aom_dsp_common::{round_power_of_two, round_power_of_two_i64, round_power_of_two_u64};
use super::sad::{aom_sad, aom_sad_skip, aom_sad_skipx4d, aom_sadx4d};
use crate::aom::av1::common::enums::{BlockSize, BLOCK_SIZE_HIGH, BLOCK_SIZE_WIDE};

/// `FILTER_BITS`
const FILTER_BITS: i32 = 7;

// 2 tap bilinear filters
/// `bilinear_filters_2t` (aom_filter.h)
pub(crate) const BILINEAR_FILTERS_2T: [[u8; 2]; 8] = [
    [128, 0],
    [112, 16],
    [96, 32],
    [80, 48],
    [64, 64],
    [48, 80],
    [32, 96],
    [16, 112],
];

/// Translation of `variance()` (the low bit depth sums).
fn variance(
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
    *sum = 0;
    *sse = 0;
    let (mut ao, mut bo) = (a_off, b_off);
    for _i in 0..h {
        for j in 0..w as usize {
            let diff = a[ao + j] as i32 - b[bo + j] as i32;
            *sum += diff;
            *sse = sse.wrapping_add((diff * diff) as u32);
        }
        ao += a_stride;
        bo += b_stride;
    }
}

/// Translation of `aom_sse_odd_size()`.
pub(crate) fn aom_sse_odd_size(
    a: &[u16],
    a_off: usize,
    a_stride: usize,
    b: &[u16],
    b_off: usize,
    b_stride: usize,
    w: i32,
    h: i32,
) -> u32 {
    let mut sse = 0;
    let mut sum = 0;
    variance(
        a, a_off, a_stride, b, b_off, b_stride, w, h, &mut sse, &mut sum,
    );
    sse
}

// Applies a 1-D 2-tap bilinear filter to the source block in either horizontal
// or vertical direction to produce the filtered output block. Used to implement
// the first-pass of 2-D separable filter.
//
// Produces int16_t output to retain precision for the next pass. Two filter
// taps should sum to FILTER_WEIGHT. pixel_step defines whether the filter is
// applied horizontally (pixel_step = 1) or vertically (pixel_step = stride).
// It defines the offset required to move from one input to the next.
/// Translation of `aom_var_filter_block2d_bil_first_pass_c()` (and of
/// `aom_highbd_var_filter_block2d_bil_first_pass()` and the second passes,
/// the same filter: the low bit depth second pass's output fits 8 bits).
pub(crate) fn aom_var_filter_block2d_bil_pass(
    a: &[u16],
    a_off: usize,
    b: &mut [u16],
    src_pixels_per_line: usize,
    pixel_step: usize,
    output_height: u32,
    output_width: u32,
    filter: &[u8; 2],
) {
    let mut ao = a_off;
    let mut bo = 0usize;
    for _i in 0..output_height {
        for j in 0..output_width as usize {
            b[bo + j] = round_power_of_two(
                a[ao] as i32 * filter[0] as i32 + a[ao + pixel_step] as i32 * filter[1] as i32,
                FILTER_BITS,
            ) as u16;
            ao += 1;
        }
        ao += src_pixels_per_line - output_width as usize;
        bo += output_width as usize;
    }
}

/// Translation of `aom_variance<W>x<H>_c()` (and
/// `aom_highbd_8_variance<W>x<H>_c()`, the same for 8-bit samples).
pub(crate) fn aom_variance(
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
    let mut sum = 0;
    variance(a, a_off, a_stride, b, b_off, b_stride, w, h, sse, &mut sum);
    sse.wrapping_sub(((sum as i64 * sum as i64) / (w * h) as i64) as u32)
}

/// Translation of `aom_mse<W>x<H>_c()`.
pub(crate) fn aom_mse(
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
    let mut sum = 0;
    variance(a, a_off, a_stride, b, b_off, b_stride, w, h, sse, &mut sum);
    *sse
}

/// Translation of `aom_get<S>x<S>var_c()` (and `aom_highbd_8_get..()`).
pub(crate) fn aom_getvar(
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
    variance(a, a_off, a_stride, b, b_off, b_stride, w, h, sse, sum);
}

/// Translation of `aom_get_var_sse_sum_8x8_quad_c()`.
pub(crate) fn aom_get_var_sse_sum_8x8_quad(
    a: &[u16],
    a_off: usize,
    a_stride: usize,
    b: &[u16],
    b_off: usize,
    b_stride: usize,
    sse8x8: &mut [u32],
    sum8x8: &mut [i32],
    tot_sse: &mut u32,
    tot_sum: &mut i32,
    var8x8: &mut [u32],
) {
    // Loop over 4 8x8 blocks. Process one 8x32 block.
    for k in 0..4 {
        variance(
            a,
            a_off + k * 8,
            a_stride,
            b,
            b_off + k * 8,
            b_stride,
            8,
            8,
            &mut sse8x8[k],
            &mut sum8x8[k],
        );
    }

    // Calculate variance at 8x8 level and total sse, sum of 8x32 block.
    *tot_sse = tot_sse
        .wrapping_add(sse8x8[0])
        .wrapping_add(sse8x8[1])
        .wrapping_add(sse8x8[2])
        .wrapping_add(sse8x8[3]);
    *tot_sum += sum8x8[0] + sum8x8[1] + sum8x8[2] + sum8x8[3];
    for i in 0..4 {
        var8x8[i] = sse8x8[i].wrapping_sub(((sum8x8[i] as i64 * sum8x8[i] as i64) >> 6) as u32);
    }
}

/// Translation of `aom_get_var_sse_sum_16x16_dual_c()`.
pub(crate) fn aom_get_var_sse_sum_16x16_dual(
    src: &[u16],
    src_off: usize,
    source_stride: usize,
    r: &[u16],
    ref_off: usize,
    ref_stride: usize,
    sse16x16: &mut [u32],
    tot_sse: &mut u32,
    tot_sum: &mut i32,
    var16x16: &mut [u32],
) {
    let mut sum16x16 = [0i32; 64];
    // Loop over two consecutive 16x16 blocks and process as one 16x32 block.
    for k in 0..2 {
        variance(
            src,
            src_off + k * 16,
            source_stride,
            r,
            ref_off + k * 16,
            ref_stride,
            16,
            16,
            &mut sse16x16[k],
            &mut sum16x16[k],
        );
    }

    // Calculate variance at 16x16 level and total sse, sum of 16x32 block.
    *tot_sse = tot_sse.wrapping_add(sse16x16[0]).wrapping_add(sse16x16[1]);
    *tot_sum += sum16x16[0] + sum16x16[1];
    for i in 0..2 {
        var16x16[i] =
            sse16x16[i].wrapping_sub(((sum16x16[i] as i64 * sum16x16[i] as i64) >> 8) as u32);
    }
}

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
    highbd_10_variance(a, a_off, a_stride, b, b_off, b_stride, w, h, sse, &mut sum);
    let var = *sse as i64 - ((sum as i64 * sum as i64) / (w * h) as i64);
    if var >= 0 {
        var as u32
    } else {
        0
    }
}

/// Translation of `aom_highbd_10_mse<W>x<H>_c()`.
pub(crate) fn aom_highbd_10_mse(
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
    highbd_10_variance(a, a_off, a_stride, b, b_off, b_stride, w, h, sse, &mut sum);
    *sse
}

/// Translation of `aom_highbd_10_get<S>x<S>var_c()`.
pub(crate) fn aom_highbd_10_getvar(
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
    highbd_10_variance(a, a_off, a_stride, b, b_off, b_stride, w, h, sse, sum);
}

/// The variance function table, `aom_variance_fn_ptr_t ppi->fn_ptr[]`: the
/// low bit depth functions, or (`highbd_set_var_fns()`) the high bit depth
/// ones of the sequence's bit depth, whose sums of absolute differences are
/// scaled to 8 bits (the `_bits10` wrappers shift them by 2).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AomVarianceFnPtr {
    /// 0 for the low bit depth functions, else the high bit depth
    /// functions' bit depth (8 or 10).
    pub(crate) hbd_bd: i32,
}

impl AomVarianceFnPtr {
    #[inline]
    fn sad_shift(&self) -> u32 {
        if self.hbd_bd == 10 {
            2
        } else {
            0
        }
    }

    /// `fn_ptr[bsize].sdf`
    #[inline]
    pub(crate) fn sdf(
        &self,
        bsize: BlockSize,
        src: &[u16],
        src_off: usize,
        src_stride: usize,
        r: &[u16],
        ref_off: usize,
        ref_stride: usize,
    ) -> u32 {
        let w = BLOCK_SIZE_WIDE[bsize as usize] as i32;
        let h = BLOCK_SIZE_HIGH[bsize as usize] as i32;
        aom_sad(src, src_off, src_stride, r, ref_off, ref_stride, w, h) >> self.sad_shift()
    }

    /// `fn_ptr[bsize].sdsf`
    #[inline]
    pub(crate) fn sdsf(
        &self,
        bsize: BlockSize,
        src: &[u16],
        src_off: usize,
        src_stride: usize,
        r: &[u16],
        ref_off: usize,
        ref_stride: usize,
    ) -> u32 {
        let w = BLOCK_SIZE_WIDE[bsize as usize] as i32;
        let h = BLOCK_SIZE_HIGH[bsize as usize] as i32;
        aom_sad_skip(src, src_off, src_stride, r, ref_off, ref_stride, w, h) >> self.sad_shift()
    }

    /// `fn_ptr[bsize].sdx4df` (and `sdx3df`, which computes the same four
    /// sums with the plain-C functions).
    #[inline]
    pub(crate) fn sdx4df(
        &self,
        bsize: BlockSize,
        src: &[u16],
        src_off: usize,
        src_stride: usize,
        r: &[u16],
        ref_offs: &[usize; 4],
        ref_stride: usize,
        sad_array: &mut [u32; 4],
    ) {
        let w = BLOCK_SIZE_WIDE[bsize as usize] as i32;
        let h = BLOCK_SIZE_HIGH[bsize as usize] as i32;
        aom_sadx4d(
            src, src_off, src_stride, r, ref_offs, ref_stride, w, h, sad_array,
        );
        let shift = self.sad_shift();
        for s in sad_array.iter_mut() {
            *s >>= shift;
        }
    }

    /// `fn_ptr[bsize].sdsx4df`
    #[inline]
    pub(crate) fn sdsx4df(
        &self,
        bsize: BlockSize,
        src: &[u16],
        src_off: usize,
        src_stride: usize,
        r: &[u16],
        ref_offs: &[usize; 4],
        ref_stride: usize,
        sad_array: &mut [u32; 4],
    ) {
        let w = BLOCK_SIZE_WIDE[bsize as usize] as i32;
        let h = BLOCK_SIZE_HIGH[bsize as usize] as i32;
        aom_sad_skipx4d(
            src, src_off, src_stride, r, ref_offs, ref_stride, w, h, sad_array,
        );
        let shift = self.sad_shift();
        for s in sad_array.iter_mut() {
            *s >>= shift;
        }
    }

    /// `fn_ptr[bsize].vf`
    #[inline]
    pub(crate) fn vf(
        &self,
        bsize: BlockSize,
        src: &[u16],
        src_off: usize,
        src_stride: usize,
        r: &[u16],
        ref_off: usize,
        ref_stride: usize,
        sse: &mut u32,
    ) -> u32 {
        let w = BLOCK_SIZE_WIDE[bsize as usize] as i32;
        let h = BLOCK_SIZE_HIGH[bsize as usize] as i32;
        if self.hbd_bd == 10 {
            aom_highbd_10_variance(src, src_off, src_stride, r, ref_off, ref_stride, w, h, sse)
        } else {
            aom_variance(src, src_off, src_stride, r, ref_off, ref_stride, w, h, sse)
        }
    }

    /// `fn_ptr[bsize].svf`: the variance of the bilinear interpolation of
    /// `src` at (`xoffset`, `yoffset`) eighths against `r`.
    pub(crate) fn svf(
        &self,
        bsize: BlockSize,
        src: &[u16],
        src_off: usize,
        src_stride: usize,
        xoffset: i32,
        yoffset: i32,
        r: &[u16],
        ref_off: usize,
        ref_stride: usize,
        sse: &mut u32,
    ) -> u32 {
        let w = BLOCK_SIZE_WIDE[bsize as usize] as u32;
        let h = BLOCK_SIZE_HIGH[bsize as usize] as u32;
        let mut fdata3 = vec![0u16; ((h + 1) * w) as usize];
        let mut temp2 = vec![0u16; (h * w) as usize];
        aom_var_filter_block2d_bil_pass(
            src,
            src_off,
            &mut fdata3,
            src_stride,
            1,
            h + 1,
            w,
            &BILINEAR_FILTERS_2T[xoffset as usize],
        );
        aom_var_filter_block2d_bil_pass(
            &fdata3,
            0,
            &mut temp2,
            w as usize,
            w as usize,
            h,
            w,
            &BILINEAR_FILTERS_2T[yoffset as usize],
        );
        if self.hbd_bd == 10 {
            aom_highbd_10_variance(
                &temp2, 0, w as usize, r, ref_off, ref_stride, w as i32, h as i32, sse,
            )
        } else {
            aom_variance(
                &temp2, 0, w as usize, r, ref_off, ref_stride, w as i32, h as i32, sse,
            )
        }
    }
}
