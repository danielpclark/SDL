// Rust translation of src/cdef_tmpl.c and src/cdef.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins), the plain-C functions.
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The constrained directional enhancement filter of one 8x8 (or 4x8, 4x4
//! chroma) block, and the direction search.
//!
//! CDEF operates entirely on pre-filter data; if bottom/right edges are
//! present (according to $edges), then the pre-filter data is located in
//! $dst. However, the edge pixels above $dst may be post-filter, so in
//! order to get access to pre-filter top pixels, use $top.

#![allow(clippy::too_many_arguments)]

use super::bitdepth::{bitdepth_from_max, Pixel};
use super::intops::{apply_sign, iclip, imax, imin, ulog2};
use super::tables::CDEF_DIRECTIONS;

// enum CdefEdgeFlags
pub(crate) const CDEF_HAVE_LEFT: u32 = 1 << 0;
pub(crate) const CDEF_HAVE_RIGHT: u32 = 1 << 1;
pub(crate) const CDEF_HAVE_TOP: u32 = 1 << 2;
pub(crate) const CDEF_HAVE_BOTTOM: u32 = 1 << 3;

#[inline]
fn constrain(diff: i32, threshold: i32, shift: i32) -> i32 {
    let adiff = diff.abs();
    apply_sign(imin(adiff, imax(0, threshold - (adiff >> shift))), diff)
}

#[inline]
fn fill(tmp: &mut [i16], mut t: usize, stride: usize, w: usize, h: usize) {
    /* Use a value that's a large positive number when interpreted as unsigned,
     * and a large negative number when interpreted as signed. */
    for _y in 0..h {
        tmp[t..t + w].fill(i16::MIN);
        t += stride;
    }
}

/// `padding()`: `tmp[t..]` is the block's top-left in the padded copy,
/// `src[s..]` the block in its plane, `top[tp..]` the saved pre-filter
/// rows above (stride `src_stride` too), the rows below are read from
/// `src`'s plane (`bottom` is `src + h * stride` in every caller).
fn padding<P: Pixel>(
    tmp: &mut [i16],
    t0: usize,
    tmp_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    left: &[[P; 2]],
    top: &[P],
    mut tp: usize,
    mut bottom: usize,
    w: usize,
    h: usize,
    edges: u32,
) {
    let ts = tmp_stride as isize;
    let at = |base: usize, k: isize| (base as isize + k) as usize;
    // fill extended input buffer
    let mut x_start: isize = -2;
    let mut x_end: isize = w as isize + 2;
    let mut y_start: isize = -2;
    let mut y_end: isize = h as isize + 2;
    if edges & CDEF_HAVE_TOP == 0 {
        fill(tmp, at(t0, -2 - 2 * ts), tmp_stride, w + 4, 2);
        y_start = 0;
    }
    if edges & CDEF_HAVE_BOTTOM == 0 {
        fill(tmp, at(t0, h as isize * ts - 2), tmp_stride, w + 4, 2);
        y_end -= 2;
    }
    if edges & CDEF_HAVE_LEFT == 0 {
        fill(
            tmp,
            at(t0, y_start * ts - 2),
            tmp_stride,
            2,
            (y_end - y_start) as usize,
        );
        x_start = 0;
    }
    if edges & CDEF_HAVE_RIGHT == 0 {
        fill(
            tmp,
            at(t0, y_start * ts + w as isize),
            tmp_stride,
            2,
            (y_end - y_start) as usize,
        );
        x_end -= 2;
    }

    for y in y_start..0 {
        for x in x_start..x_end {
            tmp[at(t0, x + y * ts)] = top[at(tp, x)].to_i32() as i16;
        }
        tp += src_stride;
    }
    for y in 0..h {
        for x in x_start..0 {
            tmp[at(t0, x + y as isize * ts)] = left[y][(2 + x) as usize].to_i32() as i16;
        }
    }
    let mut t = t0;
    for _y in 0..h {
        for x in 0..x_end {
            tmp[at(t, x)] = src[at(s, x)].to_i32() as i16;
        }
        s += src_stride;
        t += tmp_stride;
    }
    for _y in h as isize..y_end {
        for x in x_start..x_end {
            tmp[at(t, x)] = src[at(bottom, x)].to_i32() as i16;
        }
        bottom += src_stride;
        t += tmp_stride;
    }
}

/// Translation of `cdef_filter_block_c()` (`c->fb[]`): `dst[d..]` is the
/// block, `left` the two pre-filter columns to its left, `top[tp..]` the
/// pre-filter rows above it; the rows below are `dst[d + h * stride..]`.
#[inline(never)]
pub(crate) fn cdef_filter_block<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    left: &[[P; 2]],
    top: &[P],
    tp: usize,
    pri_strength: i32,
    sec_strength: i32,
    dir: i32,
    damping: i32,
    w: usize,
    mut h: usize,
    edges: u32,
    bitdepth_max: i32,
) {
    let tmp_stride = 12;
    debug_assert!((w == 4 || w == 8) && (h == 4 || h == 8));
    let mut tmp_buf = [0i16; 144]; // 12*12 is the maximum value of tmp_stride * (h + 4)
    let mut tmp = 2 * tmp_stride + 2;

    padding(
        &mut tmp_buf,
        tmp,
        tmp_stride,
        dst,
        d,
        dst_stride,
        left,
        top,
        tp,
        d + h * dst_stride,
        w,
        h,
        edges,
    );
    let t = |tmp_buf: &[i16; 144], tmp: usize, off: i32| {
        tmp_buf[(tmp as isize + off as isize) as usize] as i32
    };
    let dir = dir as usize;

    if pri_strength != 0 {
        let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;
        let pri_tap = 4 - ((pri_strength >> bitdepth_min_8) & 1);
        let pri_shift = imax(0, damping - ulog2(pri_strength as u32));
        if sec_strength != 0 {
            let sec_shift = damping - ulog2(sec_strength as u32);
            loop {
                for x in 0..w {
                    let px = dst[d + x].to_i32();
                    let mut sum = 0;
                    let mut max = px;
                    let mut min = px;
                    let mut pri_tap_k = pri_tap;
                    for k in 0..2 {
                        let off1 = CDEF_DIRECTIONS[dir + 2][k] as i32; // dir
                        let p0 = t(&tmp_buf, tmp + x, off1);
                        let p1 = t(&tmp_buf, tmp + x, -off1);
                        sum += pri_tap_k * constrain(p0 - px, pri_strength, pri_shift);
                        sum += pri_tap_k * constrain(p1 - px, pri_strength, pri_shift);
                        // if pri_tap_k == 4 then it becomes 2 else it remains 3
                        pri_tap_k = (pri_tap_k & 3) | 2;
                        min = (p0 as u32).min(min as u32) as i32;
                        max = imax(p0, max);
                        min = (p1 as u32).min(min as u32) as i32;
                        max = imax(p1, max);
                        let off2 = CDEF_DIRECTIONS[dir + 4][k] as i32; // dir + 2
                        let off3 = CDEF_DIRECTIONS[dir][k] as i32; // dir - 2
                        let s0 = t(&tmp_buf, tmp + x, off2);
                        let s1 = t(&tmp_buf, tmp + x, -off2);
                        let s2 = t(&tmp_buf, tmp + x, off3);
                        let s3 = t(&tmp_buf, tmp + x, -off3);
                        // sec_tap starts at 2 and becomes 1
                        let sec_tap = 2 - k as i32;
                        sum += sec_tap * constrain(s0 - px, sec_strength, sec_shift);
                        sum += sec_tap * constrain(s1 - px, sec_strength, sec_shift);
                        sum += sec_tap * constrain(s2 - px, sec_strength, sec_shift);
                        sum += sec_tap * constrain(s3 - px, sec_strength, sec_shift);
                        min = (s0 as u32).min(min as u32) as i32;
                        max = imax(s0, max);
                        min = (s1 as u32).min(min as u32) as i32;
                        max = imax(s1, max);
                        min = (s2 as u32).min(min as u32) as i32;
                        max = imax(s2, max);
                        min = (s3 as u32).min(min as u32) as i32;
                        max = imax(s3, max);
                    }
                    dst[d + x] =
                        P::from_i32(iclip(px + ((sum - (sum < 0) as i32 + 8) >> 4), min, max));
                }
                d += dst_stride;
                tmp += tmp_stride;
                h -= 1;
                if h == 0 {
                    break;
                }
            }
        } else {
            // pri_strength only
            loop {
                for x in 0..w {
                    let px = dst[d + x].to_i32();
                    let mut sum = 0;
                    let mut pri_tap_k = pri_tap;
                    for k in 0..2 {
                        let off = CDEF_DIRECTIONS[dir + 2][k] as i32; // dir
                        let p0 = t(&tmp_buf, tmp + x, off);
                        let p1 = t(&tmp_buf, tmp + x, -off);
                        sum += pri_tap_k * constrain(p0 - px, pri_strength, pri_shift);
                        sum += pri_tap_k * constrain(p1 - px, pri_strength, pri_shift);
                        pri_tap_k = (pri_tap_k & 3) | 2;
                    }
                    dst[d + x] = P::from_i32(px + ((sum - (sum < 0) as i32 + 8) >> 4));
                }
                d += dst_stride;
                tmp += tmp_stride;
                h -= 1;
                if h == 0 {
                    break;
                }
            }
        }
    } else {
        // sec_strength only
        debug_assert!(sec_strength != 0);
        let sec_shift = damping - ulog2(sec_strength as u32);
        loop {
            for x in 0..w {
                let px = dst[d + x].to_i32();
                let mut sum = 0;
                for k in 0..2 {
                    let off1 = CDEF_DIRECTIONS[dir + 4][k] as i32; // dir + 2
                    let off2 = CDEF_DIRECTIONS[dir][k] as i32; // dir - 2
                    let s0 = t(&tmp_buf, tmp + x, off1);
                    let s1 = t(&tmp_buf, tmp + x, -off1);
                    let s2 = t(&tmp_buf, tmp + x, off2);
                    let s3 = t(&tmp_buf, tmp + x, -off2);
                    let sec_tap = 2 - k as i32;
                    sum += sec_tap * constrain(s0 - px, sec_strength, sec_shift);
                    sum += sec_tap * constrain(s1 - px, sec_strength, sec_shift);
                    sum += sec_tap * constrain(s2 - px, sec_strength, sec_shift);
                    sum += sec_tap * constrain(s3 - px, sec_strength, sec_shift);
                }
                dst[d + x] = P::from_i32(px + ((sum - (sum < 0) as i32 + 8) >> 4));
            }
            d += dst_stride;
            tmp += tmp_stride;
            h -= 1;
            if h == 0 {
                break;
            }
        }
    }
}

/// Translation of `cdef_find_dir_c()` (`c->dir`): returns the direction
/// and the variance.
pub(crate) fn cdef_find_dir<P: Pixel>(
    img: &[P],
    mut i: usize,
    stride: usize,
    bitdepth_max: i32,
) -> (i32, u32) {
    let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;
    let mut partial_sum_hv = [[0i32; 8]; 2];
    let mut partial_sum_diag = [[0i32; 15]; 2];
    let mut partial_sum_alt = [[0i32; 11]; 4];

    for y in 0..8usize {
        for x in 0..8usize {
            let px = (img[i + x].to_i32() >> bitdepth_min_8) - 128;

            partial_sum_diag[0][y + x] += px;
            partial_sum_alt[0][y + (x >> 1)] += px;
            partial_sum_hv[0][y] += px;
            partial_sum_alt[1][3 + y - (x >> 1)] += px;
            partial_sum_diag[1][7 + y - x] += px;
            partial_sum_alt[2][3 - (y >> 1) + x] += px;
            partial_sum_hv[1][x] += px;
            partial_sum_alt[3][(y >> 1) + x] += px;
        }
        i += stride;
    }

    let mut cost = [0u32; 8];
    for n in 0..8 {
        cost[2] = cost[2].wrapping_add((partial_sum_hv[0][n] * partial_sum_hv[0][n]) as u32);
        cost[6] = cost[6].wrapping_add((partial_sum_hv[1][n] * partial_sum_hv[1][n]) as u32);
    }
    cost[2] = cost[2].wrapping_mul(105);
    cost[6] = cost[6].wrapping_mul(105);

    static DIV_TABLE: [u16; 7] = [840, 420, 280, 210, 168, 140, 120];
    for n in 0..7 {
        let d = DIV_TABLE[n] as i32;
        cost[0] = cost[0].wrapping_add(
            ((partial_sum_diag[0][n] * partial_sum_diag[0][n]
                + partial_sum_diag[0][14 - n] * partial_sum_diag[0][14 - n])
                * d) as u32,
        );
        cost[4] = cost[4].wrapping_add(
            ((partial_sum_diag[1][n] * partial_sum_diag[1][n]
                + partial_sum_diag[1][14 - n] * partial_sum_diag[1][14 - n])
                * d) as u32,
        );
    }
    cost[0] = cost[0].wrapping_add((partial_sum_diag[0][7] * partial_sum_diag[0][7] * 105) as u32);
    cost[4] = cost[4].wrapping_add((partial_sum_diag[1][7] * partial_sum_diag[1][7] * 105) as u32);

    for n in 0..4 {
        let cost_ptr = n * 2 + 1;
        for m in 0..5 {
            cost[cost_ptr] = cost[cost_ptr]
                .wrapping_add((partial_sum_alt[n][3 + m] * partial_sum_alt[n][3 + m]) as u32);
        }
        cost[cost_ptr] = cost[cost_ptr].wrapping_mul(105);
        for m in 0..3 {
            let d = DIV_TABLE[2 * m + 1] as i32;
            cost[cost_ptr] = cost[cost_ptr].wrapping_add(
                ((partial_sum_alt[n][m] * partial_sum_alt[n][m]
                    + partial_sum_alt[n][10 - m] * partial_sum_alt[n][10 - m])
                    * d) as u32,
            );
        }
    }

    let mut best_dir = 0;
    let mut best_cost = cost[0];
    for n in 1..8 {
        if cost[n] > best_cost {
            best_cost = cost[n];
            best_dir = n;
        }
    }

    (
        best_dir as i32,
        best_cost.wrapping_sub(cost[best_dir ^ 4]) >> 10,
    )
}
