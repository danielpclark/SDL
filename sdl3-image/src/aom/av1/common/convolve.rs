// Rust translation of the parts of av1/common/convolve.c, av1/common/convolve.h
// and aom_dsp/aom_convolve.c from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the real-time encoder reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The interpolation of the inter predictions (sub-pixel convolutions,
//! single and compound), unscaled. The samples are kept in 16 bits for any
//! bit depth: upstream's low bit depth functions are its high bit depth ones
//! with `bd` 8 (their sums, roundings and clipping to 8 bits are the same).

use super::filter::{av1_get_interp_filter_subpel_kernel, InterpFilterParams, SUBPEL_MASK};

/// `FILTER_BITS`
pub(crate) const FILTER_BITS: i32 = 7;
/// `ROUND0_BITS`
pub(crate) const ROUND0_BITS: i32 = 3;
/// `COMPOUND_ROUND1_BITS`
pub(crate) const COMPOUND_ROUND1_BITS: i32 = 7;
/// `DIST_PRECISION_BITS`
pub(crate) const DIST_PRECISION_BITS: i32 = 4;
/// `MAX_SB_SIZE` (the convolutions' intermediate block width)
const MAX_SB_SIZE: usize = 128;
/// `MAX_FILTER_TAP`
const MAX_FILTER_TAP: usize = 12;

/// Translation of `ConvolveParams`: `dst` (the compound prediction's
/// intermediate `CONV_BUF_TYPE` buffer) is passed to the convolutions
/// beside it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ConvolveParams {
    pub(crate) do_average: i32,
    pub(crate) dst_stride: i32,
    pub(crate) round_0: i32,
    pub(crate) round_1: i32,
    pub(crate) plane: i32,
    pub(crate) is_compound: i32,
    pub(crate) use_dist_wtd_comp_avg: i32,
    pub(crate) fwd_offset: i32,
    pub(crate) bck_offset: i32,
}

/// Translation of `get_conv_params_no_round()` (`dst` aside).
#[inline]
pub(crate) fn get_conv_params_no_round(
    cmp_index: i32,
    plane: i32,
    dst_stride: i32,
    is_compound: i32,
    bd: i32,
) -> ConvolveParams {
    debug_assert!(cmp_index == 0 || is_compound != 0);
    let mut conv_params = ConvolveParams {
        is_compound,
        use_dist_wtd_comp_avg: 0,
        round_0: ROUND0_BITS,
        ..ConvolveParams::default()
    };
    conv_params.round_1 = if is_compound != 0 {
        COMPOUND_ROUND1_BITS
    } else {
        2 * FILTER_BITS - conv_params.round_0
    };
    let intbufrange = bd + FILTER_BITS - conv_params.round_0 + 2;
    debug_assert!(bd >= 12 || intbufrange <= 16);
    if intbufrange > 16 {
        conv_params.round_0 += intbufrange - 16;
        if is_compound == 0 {
            conv_params.round_1 -= intbufrange - 16;
        }
    }
    // TODO(yunqing): The following dst should only be valid while
    // is_compound = 1;
    conv_params.dst_stride = dst_stride;
    conv_params.plane = plane;

    // By default, set do average to 1 if this is the second single prediction
    // in a compound mode.
    conv_params.do_average = cmp_index;
    conv_params
}

/// Translation of `get_conv_params()`.
#[inline]
pub(crate) fn get_conv_params(do_average: i32, plane: i32, bd: i32) -> ConvolveParams {
    get_conv_params_no_round(do_average, plane, 0, 0, bd)
}

#[inline]
fn round_power_of_two(value: i32, n: i32) -> i32 {
    (value + ((1 << n) >> 1)) >> n
}

/// `clip_pixel_highbd()` (`clip_pixel()` for `bd` 8)
#[inline]
fn clip_pixel_highbd(val: i32, bd: i32) -> u16 {
    let max = (1 << bd) - 1;
    val.clamp(0, max) as u16
}

/// A position in a sample buffer: the start offset, moved by a signed
/// amount (the filter taps reach before the block, into the frame border).
#[inline]
fn at(base: usize, delta: isize) -> usize {
    (base as isize + delta) as usize
}

/// Translation of `av1_highbd_convolve_2d_sr_c()` (and of
/// `av1_convolve_2d_sr_c()`, with `bd` 8).
fn av1_convolve_2d_sr(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    filter_params_x: &InterpFilterParams,
    filter_params_y: &InterpFilterParams,
    subpel_x_qn: i32,
    subpel_y_qn: i32,
    conv_params: &ConvolveParams,
    bd: i32,
) {
    let mut im_block = [0i16; (MAX_SB_SIZE + MAX_FILTER_TAP - 1) * MAX_SB_SIZE];
    let im_h = h + filter_params_y.taps as i32 - 1;
    let im_stride = w as usize;
    debug_assert!(w as usize <= MAX_SB_SIZE && h as usize <= MAX_SB_SIZE);
    let fo_vert = filter_params_y.taps as isize / 2 - 1;
    let fo_horiz = filter_params_x.taps as isize / 2 - 1;
    let bits = FILTER_BITS * 2 - conv_params.round_0 - conv_params.round_1;
    debug_assert!(bits >= 0);

    // horizontal filter
    let src_horiz = at(src_off, -fo_vert * src_stride as isize);
    let x_filter = av1_get_interp_filter_subpel_kernel(filter_params_x, subpel_x_qn & SUBPEL_MASK);
    for y in 0..im_h as usize {
        for x in 0..w as usize {
            let mut sum: i32 = 1 << (bd + FILTER_BITS - 1);
            for k in 0..filter_params_x.taps as usize {
                sum += x_filter[k] as i32
                    * src[at(src_horiz + y * src_stride + x, k as isize - fo_horiz)] as i32;
            }
            debug_assert!(0 <= sum && sum < (1 << (bd + FILTER_BITS + 1)));
            im_block[y * im_stride + x] = round_power_of_two(sum, conv_params.round_0) as i16;
        }
    }

    // vertical filter
    let src_vert = fo_vert as usize * im_stride;
    let y_filter = av1_get_interp_filter_subpel_kernel(filter_params_y, subpel_y_qn & SUBPEL_MASK);
    let offset_bits = bd + 2 * FILTER_BITS - conv_params.round_0;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut sum: i32 = 1 << offset_bits;
            for k in 0..filter_params_y.taps as usize {
                sum += y_filter[k] as i32
                    * im_block[at(
                        src_vert + x,
                        (y as isize - fo_vert + k as isize) * im_stride as isize,
                    )] as i32;
            }
            debug_assert!(0 <= sum && sum < (1 << (offset_bits + 2)));
            // (the low bit depth version keeps the difference in an int16_t:
            // it fits)
            let res = round_power_of_two(sum, conv_params.round_1)
                - ((1 << (offset_bits - conv_params.round_1))
                    + (1 << (offset_bits - conv_params.round_1 - 1)));
            dst[dst_off + y * dst_stride + x] =
                clip_pixel_highbd(round_power_of_two(res, bits), bd);
        }
    }
}

/// Translation of `av1_highbd_convolve_y_sr_c()` (and of
/// `av1_convolve_y_sr_c()`).
fn av1_convolve_y_sr(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    filter_params_y: &InterpFilterParams,
    subpel_y_qn: i32,
    bd: i32,
) {
    let fo_vert = filter_params_y.taps as isize / 2 - 1;
    // vertical filter
    let y_filter = av1_get_interp_filter_subpel_kernel(filter_params_y, subpel_y_qn & SUBPEL_MASK);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut res: i32 = 0;
            for k in 0..filter_params_y.taps as usize {
                res += y_filter[k] as i32
                    * src[at(
                        src_off + x,
                        (y as isize - fo_vert + k as isize) * src_stride as isize,
                    )] as i32;
            }
            dst[dst_off + y * dst_stride + x] =
                clip_pixel_highbd(round_power_of_two(res, FILTER_BITS), bd);
        }
    }
}

/// Translation of `av1_highbd_convolve_x_sr_c()` (and of
/// `av1_convolve_x_sr_c()`).
fn av1_convolve_x_sr(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    filter_params_x: &InterpFilterParams,
    subpel_x_qn: i32,
    conv_params: &ConvolveParams,
    bd: i32,
) {
    let fo_horiz = filter_params_x.taps as isize / 2 - 1;
    let bits = FILTER_BITS - conv_params.round_0;

    debug_assert!(bits >= 0);
    debug_assert!(
        (FILTER_BITS - conv_params.round_1) >= 0
            || ((conv_params.round_0 + conv_params.round_1) == 2 * FILTER_BITS)
    );

    // horizontal filter
    let x_filter = av1_get_interp_filter_subpel_kernel(filter_params_x, subpel_x_qn & SUBPEL_MASK);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut res: i32 = 0;
            for k in 0..filter_params_x.taps as usize {
                res += x_filter[k] as i32
                    * src[at(src_off + y * src_stride + x, k as isize - fo_horiz)] as i32;
            }
            res = round_power_of_two(res, conv_params.round_0);
            dst[dst_off + y * dst_stride + x] =
                clip_pixel_highbd(round_power_of_two(res, bits), bd);
        }
    }
}

/// The averaging of a compound prediction's second reference with the
/// first's intermediate values (or the storing of the first's).
#[inline]
fn dist_wtd_store(
    dst: &mut [u16],
    di: usize,
    dst16: &mut [u16],
    d16i: usize,
    res: i32,
    round_offset: i32,
    round_bits: i32,
    conv_params: &ConvolveParams,
    bd: i32,
) {
    if conv_params.do_average != 0 {
        let mut tmp = dst16[d16i] as i32;
        if conv_params.use_dist_wtd_comp_avg != 0 {
            tmp = tmp * conv_params.fwd_offset + res * conv_params.bck_offset;
            tmp >>= DIST_PRECISION_BITS;
        } else {
            tmp += res;
            tmp >>= 1;
        }
        tmp -= round_offset;
        dst[di] = clip_pixel_highbd(round_power_of_two(tmp, round_bits), bd);
    } else {
        // (CONV_BUF_TYPE is uint16_t)
        dst16[d16i] = res as u16;
    }
}

/// Translation of `av1_highbd_dist_wtd_convolve_2d_c()` (and of
/// `av1_dist_wtd_convolve_2d_c()`).
fn av1_dist_wtd_convolve_2d(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    filter_params_x: &InterpFilterParams,
    filter_params_y: &InterpFilterParams,
    subpel_x_qn: i32,
    subpel_y_qn: i32,
    conv_params: &ConvolveParams,
    dst16: &mut [u16],
    bd: i32,
) {
    let mut im_block = [0i16; (MAX_SB_SIZE + MAX_FILTER_TAP - 1) * MAX_SB_SIZE];
    let dst16_stride = conv_params.dst_stride as usize;
    let im_h = h + filter_params_y.taps as i32 - 1;
    let im_stride = w as usize;
    let fo_vert = filter_params_y.taps as isize / 2 - 1;
    let fo_horiz = filter_params_x.taps as isize / 2 - 1;
    let round_bits = 2 * FILTER_BITS - conv_params.round_0 - conv_params.round_1;
    debug_assert!(round_bits >= 0);

    // horizontal filter
    let src_horiz = at(src_off, -fo_vert * src_stride as isize);
    let x_filter = av1_get_interp_filter_subpel_kernel(filter_params_x, subpel_x_qn & SUBPEL_MASK);
    for y in 0..im_h as usize {
        for x in 0..w as usize {
            let mut sum: i32 = 1 << (bd + FILTER_BITS - 1);
            for k in 0..filter_params_x.taps as usize {
                sum += x_filter[k] as i32
                    * src[at(src_horiz + y * src_stride + x, k as isize - fo_horiz)] as i32;
            }
            debug_assert!(0 <= sum && sum < (1 << (bd + FILTER_BITS + 1)));
            im_block[y * im_stride + x] = round_power_of_two(sum, conv_params.round_0) as i16;
        }
    }

    // vertical filter
    let src_vert = fo_vert as usize * im_stride;
    let y_filter = av1_get_interp_filter_subpel_kernel(filter_params_y, subpel_y_qn & SUBPEL_MASK);
    let offset_bits = bd + 2 * FILTER_BITS - conv_params.round_0;
    let round_offset =
        (1 << (offset_bits - conv_params.round_1)) + (1 << (offset_bits - conv_params.round_1 - 1));
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut sum: i32 = 1 << offset_bits;
            for k in 0..filter_params_y.taps as usize {
                sum += y_filter[k] as i32
                    * im_block[at(
                        src_vert + x,
                        (y as isize - fo_vert + k as isize) * im_stride as isize,
                    )] as i32;
            }
            debug_assert!(0 <= sum && sum < (1 << (offset_bits + 2)));
            let res = round_power_of_two(sum, conv_params.round_1);
            dist_wtd_store(
                dst,
                dst_off + y * dst_stride + x,
                dst16,
                y * dst16_stride + x,
                res,
                round_offset,
                round_bits,
                conv_params,
                bd,
            );
        }
    }
}

/// Translation of `av1_highbd_dist_wtd_convolve_y_c()` (and of
/// `av1_dist_wtd_convolve_y_c()`).
fn av1_dist_wtd_convolve_y(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    filter_params_y: &InterpFilterParams,
    subpel_y_qn: i32,
    conv_params: &ConvolveParams,
    dst16: &mut [u16],
    bd: i32,
) {
    let dst16_stride = conv_params.dst_stride as usize;
    let fo_vert = filter_params_y.taps as isize / 2 - 1;
    let bits = FILTER_BITS - conv_params.round_0;
    let offset_bits = bd + 2 * FILTER_BITS - conv_params.round_0;
    let round_offset =
        (1 << (offset_bits - conv_params.round_1)) + (1 << (offset_bits - conv_params.round_1 - 1));
    let round_bits = 2 * FILTER_BITS - conv_params.round_0 - conv_params.round_1;
    debug_assert!(round_bits >= 0);
    debug_assert!(bits >= 0);

    // vertical filter
    let y_filter = av1_get_interp_filter_subpel_kernel(filter_params_y, subpel_y_qn & SUBPEL_MASK);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut res: i32 = 0;
            for k in 0..filter_params_y.taps as usize {
                res += y_filter[k] as i32
                    * src[at(
                        src_off + x,
                        (y as isize - fo_vert + k as isize) * src_stride as isize,
                    )] as i32;
            }
            res *= 1 << bits;
            res = round_power_of_two(res, conv_params.round_1) + round_offset;
            dist_wtd_store(
                dst,
                dst_off + y * dst_stride + x,
                dst16,
                y * dst16_stride + x,
                res,
                round_offset,
                round_bits,
                conv_params,
                bd,
            );
        }
    }
}

/// Translation of `av1_highbd_dist_wtd_convolve_x_c()` (and of
/// `av1_dist_wtd_convolve_x_c()`).
fn av1_dist_wtd_convolve_x(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    filter_params_x: &InterpFilterParams,
    subpel_x_qn: i32,
    conv_params: &ConvolveParams,
    dst16: &mut [u16],
    bd: i32,
) {
    let dst16_stride = conv_params.dst_stride as usize;
    let fo_horiz = filter_params_x.taps as isize / 2 - 1;
    let bits = FILTER_BITS - conv_params.round_1;
    let offset_bits = bd + 2 * FILTER_BITS - conv_params.round_0;
    let round_offset =
        (1 << (offset_bits - conv_params.round_1)) + (1 << (offset_bits - conv_params.round_1 - 1));
    let round_bits = 2 * FILTER_BITS - conv_params.round_0 - conv_params.round_1;
    debug_assert!(round_bits >= 0);
    debug_assert!(bits >= 0);
    // horizontal filter
    let x_filter = av1_get_interp_filter_subpel_kernel(filter_params_x, subpel_x_qn & SUBPEL_MASK);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut res: i32 = 0;
            for k in 0..filter_params_x.taps as usize {
                res += x_filter[k] as i32
                    * src[at(src_off + y * src_stride + x, k as isize - fo_horiz)] as i32;
            }
            res = (1 << bits) * round_power_of_two(res, conv_params.round_0);
            res += round_offset;
            dist_wtd_store(
                dst,
                dst_off + y * dst_stride + x,
                dst16,
                y * dst16_stride + x,
                res,
                round_offset,
                round_bits,
                conv_params,
                bd,
            );
        }
    }
}

/// Translation of `av1_highbd_dist_wtd_convolve_2d_copy_c()` (and of
/// `av1_dist_wtd_convolve_2d_copy_c()`).
fn av1_dist_wtd_convolve_2d_copy(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    conv_params: &ConvolveParams,
    dst16: &mut [u16],
    bd: i32,
) {
    let dst16_stride = conv_params.dst_stride as usize;
    let bits = FILTER_BITS * 2 - conv_params.round_1 - conv_params.round_0;
    let offset_bits = bd + 2 * FILTER_BITS - conv_params.round_0;
    let round_offset =
        (1 << (offset_bits - conv_params.round_1)) + (1 << (offset_bits - conv_params.round_1 - 1));
    debug_assert!(bits >= 0);

    for y in 0..h as usize {
        for x in 0..w as usize {
            // (CONV_BUF_TYPE is uint16_t)
            let mut res = ((src[src_off + y * src_stride + x] as i32) << bits) as u16 as i32;
            res = (res + round_offset) as u16 as i32;
            dist_wtd_store(
                dst,
                dst_off + y * dst_stride + x,
                dst16,
                y * dst16_stride + x,
                res,
                round_offset,
                bits,
                conv_params,
                bd,
            );
        }
    }
}

/// Translation of `aom_highbd_convolve_copy_c()` (and of
/// `aom_convolve_copy_c()`).
fn aom_convolve_copy(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
) {
    for r in 0..h as usize {
        let s = src_off + r * src_stride;
        let d = dst_off + r * dst_stride;
        dst[d..d + w as usize].copy_from_slice(&src[s..s + w as usize]);
    }
}

/// Translation of `av1_convolve_2d_facade()` and
/// `av1_highbd_convolve_2d_facade()` (with `convolve_2d_facade_compound()`,
/// `convolve_2d_facade_single()` and their high bit depth versions):
/// `conv_dst` is `conv_params->dst`. `Err` for the scaled convolutions and
/// the intra block copy filters, which the saver never uses.
pub(crate) fn av1_convolve_2d_facade(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    w: i32,
    h: i32,
    interp_filters: [&InterpFilterParams; 2],
    subpel_x_qn: i32,
    x_step_q4: i32,
    subpel_y_qn: i32,
    y_step_q4: i32,
    scaled: bool,
    conv_params: &ConvolveParams,
    conv_dst: &mut [u16],
    bd: i32,
) -> Result<(), ()> {
    let _ = (x_step_q4, y_step_q4);
    let filter_params_x = interp_filters[0];
    let filter_params_y = interp_filters[1];

    // TODO(jingning, yunqing): Add SIMD support to 2-tap filter case.
    // Do we have SIMD support to 4-tap case?
    // 2-tap filter indicates that it is for IntraBC.
    if filter_params_x.taps == 2 || filter_params_y.taps == 2 {
        // (intra block copy is off)
        return Err(());
    }

    if scaled {
        // (convolve_2d_scale_wrapper(): the saver's references are never
        // scaled)
        return Err(());
    }
    let need_x = subpel_x_qn != 0;
    let need_y = subpel_y_qn != 0;
    if conv_params.is_compound != 0 {
        if !need_x && !need_y {
            av1_dist_wtd_convolve_2d_copy(
                src,
                src_off,
                src_stride,
                dst,
                dst_off,
                dst_stride,
                w,
                h,
                conv_params,
                conv_dst,
                bd,
            );
        } else if need_x && !need_y {
            av1_dist_wtd_convolve_x(
                src,
                src_off,
                src_stride,
                dst,
                dst_off,
                dst_stride,
                w,
                h,
                filter_params_x,
                subpel_x_qn,
                conv_params,
                conv_dst,
                bd,
            );
        } else if !need_x && need_y {
            av1_dist_wtd_convolve_y(
                src,
                src_off,
                src_stride,
                dst,
                dst_off,
                dst_stride,
                w,
                h,
                filter_params_y,
                subpel_y_qn,
                conv_params,
                conv_dst,
                bd,
            );
        } else {
            debug_assert!(need_y && need_x);
            av1_dist_wtd_convolve_2d(
                src,
                src_off,
                src_stride,
                dst,
                dst_off,
                dst_stride,
                w,
                h,
                filter_params_x,
                filter_params_y,
                subpel_x_qn,
                subpel_y_qn,
                conv_params,
                conv_dst,
                bd,
            );
        }
    } else if !need_x && !need_y {
        aom_convolve_copy(src, src_off, src_stride, dst, dst_off, dst_stride, w, h);
    } else if need_x && !need_y {
        av1_convolve_x_sr(
            src,
            src_off,
            src_stride,
            dst,
            dst_off,
            dst_stride,
            w,
            h,
            filter_params_x,
            subpel_x_qn,
            conv_params,
            bd,
        );
    } else if !need_x && need_y {
        av1_convolve_y_sr(
            src,
            src_off,
            src_stride,
            dst,
            dst_off,
            dst_stride,
            w,
            h,
            filter_params_y,
            subpel_y_qn,
            bd,
        );
    } else {
        debug_assert!(need_x && need_y);
        av1_convolve_2d_sr(
            src,
            src_off,
            src_stride,
            dst,
            dst_off,
            dst_stride,
            w,
            h,
            filter_params_x,
            filter_params_y,
            subpel_x_qn,
            subpel_y_qn,
            conv_params,
            bd,
        );
    }
    Ok(())
}
