// Rust translation of src/ipred_tmpl.c and src/ipred.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins), the plain-C functions.
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Intra prediction.
//!
//! - `a` is the angle (in degrees) for directional intra predictors. For other
//!   modes, it is ignored;
//! - `topleft` is the same as the argument given to dav1d_prepare_intra_edges(),
//!   see ipred_prepare.h for more detailed documentation; here it is a
//!   buffer and the offset of the top/left pixel (`tl`).
//!
//! `dav1d_intra_pred_dsp_init()`'s tables are [`intra_pred`],
//! [`cfl_pred`], [`cfl_ac`] and [`pal_pred`].

#![allow(clippy::too_many_arguments)]

use super::bitdepth::{iclip_pixel, ix, Pixel};
use super::headers::{DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422};
use super::intops::{apply_sign, ctz, iclip, imax, imin};
use super::levels::*;
use super::tables::{DR_INTRA_DERIVATIVE, FILTER_INTRA_TAPS, SM_WEIGHTS};

#[inline(never)]
fn splat_dc<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    width: i32,
    height: i32,
    dc: i32,
) {
    debug_assert!(dc <= if P::BPC8 { 0xff } else { 0xffff });
    let v = P::from_i32(dc);
    for _y in 0..height {
        dst[d..d + width as usize].fill(v);
        d += stride;
    }
}

#[inline(never)]
fn cfl_pred_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    width: i32,
    height: i32,
    dc: i32,
    ac: &[i16],
    alpha: i32,
    bitdepth_max: i32,
) {
    let mut a = 0;
    for _y in 0..height {
        for x in 0..width as usize {
            let diff = alpha * ac[a + x] as i32;
            dst[d + x] = iclip_pixel(dc + apply_sign((diff.abs() + 32) >> 6, diff), bitdepth_max);
        }
        a += width as usize;
        d += stride;
    }
}

fn dc_gen_top<P: Pixel>(topleft: &[P], tl: usize, width: i32) -> u32 {
    let mut dc = (width >> 1) as u32;
    for i in 0..width as usize {
        dc += topleft[tl + 1 + i].to_i32() as u32;
    }
    dc >> ctz(width as u32)
}

fn dc_gen_left<P: Pixel>(topleft: &[P], tl: usize, height: i32) -> u32 {
    let mut dc = (height >> 1) as u32;
    for i in 0..height as usize {
        dc += topleft[tl - (1 + i)].to_i32() as u32;
    }
    dc >> ctz(height as u32)
}

fn dc_gen<P: Pixel>(topleft: &[P], tl: usize, width: i32, height: i32) -> u32 {
    let (multiplier_1x2, multiplier_1x4, base_shift) = if P::BPC8 {
        (0x5556u32, 0x3334u32, 16)
    } else {
        (0xAAABu32, 0x6667u32, 17)
    };
    let mut dc = ((width + height) >> 1) as u32;
    for i in 0..width as usize {
        dc += topleft[tl + i + 1].to_i32() as u32;
    }
    for i in 0..height as usize {
        dc += topleft[tl - (i + 1)].to_i32() as u32;
    }
    dc >>= ctz((width + height) as u32);

    if width != height {
        dc *= if width > height * 2 || height > width * 2 {
            multiplier_1x4
        } else {
            multiplier_1x2
        };
        dc >>= base_shift;
    }
    dc
}

fn dc_128<P: Pixel>(bitdepth_max: i32) -> i32 {
    if P::BPC8 {
        128
    } else {
        (bitdepth_max + 1) >> 1
    }
}

fn ipred_v_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft: &[P],
    tl: usize,
    width: i32,
    height: i32,
) {
    let w = width as usize;
    for _y in 0..height {
        dst[d..d + w].copy_from_slice(&topleft[tl + 1..tl + 1 + w]);
        d += stride;
    }
}

fn ipred_h_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft: &[P],
    tl: usize,
    width: i32,
    height: i32,
) {
    let w = width as usize;
    for y in 0..height as usize {
        dst[d..d + w].fill(topleft[tl - (1 + y)]);
        d += stride;
    }
}

fn ipred_paeth_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    tl_ptr: &[P],
    tl: usize,
    width: i32,
    height: i32,
) {
    let topleft = tl_ptr[tl].to_i32();
    for y in 0..height as usize {
        let left = tl_ptr[tl - (y + 1)].to_i32();
        for x in 0..width as usize {
            let top = tl_ptr[tl + 1 + x].to_i32();
            let base = left + top - topleft;
            let ldiff = (left - base).abs();
            let tdiff = (top - base).abs();
            let tldiff = (topleft - base).abs();

            dst[d + x] = P::from_i32(if ldiff <= tdiff && ldiff <= tldiff {
                left
            } else if tdiff <= tldiff {
                top
            } else {
                topleft
            });
        }
        d += stride;
    }
}

fn ipred_smooth_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft: &[P],
    tl: usize,
    width: i32,
    height: i32,
) {
    let weights_hor = &SM_WEIGHTS[width as usize..];
    let weights_ver = &SM_WEIGHTS[height as usize..];
    let right = topleft[tl + width as usize].to_i32();
    let bottom = topleft[tl - height as usize].to_i32();

    for y in 0..height as usize {
        for x in 0..width as usize {
            let pred = weights_ver[y] as i32 * topleft[tl + 1 + x].to_i32()
                + (256 - weights_ver[y] as i32) * bottom
                + weights_hor[x] as i32 * topleft[tl - (1 + y)].to_i32()
                + (256 - weights_hor[x] as i32) * right;
            dst[d + x] = P::from_i32((pred + 256) >> 9);
        }
        d += stride;
    }
}

fn ipred_smooth_v_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft: &[P],
    tl: usize,
    width: i32,
    height: i32,
) {
    let weights_ver = &SM_WEIGHTS[height as usize..];
    let bottom = topleft[tl - height as usize].to_i32();

    for y in 0..height as usize {
        for x in 0..width as usize {
            let pred = weights_ver[y] as i32 * topleft[tl + 1 + x].to_i32()
                + (256 - weights_ver[y] as i32) * bottom;
            dst[d + x] = P::from_i32((pred + 128) >> 8);
        }
        d += stride;
    }
}

fn ipred_smooth_h_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft: &[P],
    tl: usize,
    width: i32,
    height: i32,
) {
    let weights_hor = &SM_WEIGHTS[width as usize..];
    let right = topleft[tl + width as usize].to_i32();

    for y in 0..height as usize {
        for x in 0..width as usize {
            let pred = weights_hor[x] as i32 * topleft[tl - (y + 1)].to_i32()
                + (256 - weights_hor[x] as i32) * right;
            dst[d + x] = P::from_i32((pred + 128) >> 8);
        }
        d += stride;
    }
}

#[inline(never)]
fn get_filter_strength(wh: i32, angle: i32, is_sm: bool) -> i32 {
    if is_sm {
        if wh <= 8 {
            if angle >= 64 {
                return 2;
            }
            if angle >= 40 {
                return 1;
            }
        } else if wh <= 16 {
            if angle >= 48 {
                return 2;
            }
            if angle >= 20 {
                return 1;
            }
        } else if wh <= 24 {
            if angle >= 4 {
                return 3;
            }
        } else {
            return 3;
        }
    } else if wh <= 8 {
        if angle >= 56 {
            return 1;
        }
    } else if wh <= 16 {
        if angle >= 40 {
            return 1;
        }
    } else if wh <= 24 {
        if angle >= 32 {
            return 3;
        }
        if angle >= 16 {
            return 2;
        }
        if angle >= 8 {
            return 1;
        }
    } else if wh <= 32 {
        if angle >= 32 {
            return 3;
        }
        if angle >= 4 {
            return 2;
        }
        return 1;
    } else {
        return 3;
    }
    0
}

/// `filter_edge()`: `out[o..]`, `in[i..]` with the `from..to` range
/// relative to `i`.
#[inline(never)]
fn filter_edge<P: Pixel>(
    out: &mut [P],
    o: usize,
    sz: i32,
    lim_from: i32,
    lim_to: i32,
    r#in: &[P],
    i0: usize,
    from: i32,
    to: i32,
    strength: i32,
) {
    static KERNEL: [[u8; 5]; 3] = [[0, 4, 8, 4, 0], [0, 5, 6, 5, 0], [2, 4, 4, 4, 2]];

    debug_assert!(strength > 0);
    let at = |k: i32| r#in[ix(i0, iclip(k, from, to - 1) as isize)];
    let mut i = 0;
    while i < imin(sz, lim_from) {
        out[o + i as usize] = at(i);
        i += 1;
    }
    while i < imin(lim_to, sz) {
        let mut s = 0;
        for j in 0..5 {
            s += at(i - 2 + j).to_i32() * KERNEL[(strength - 1) as usize][j as usize] as i32;
        }
        out[o + i as usize] = P::from_i32((s + 8) >> 4);
        i += 1;
    }
    while i < sz {
        out[o + i as usize] = at(i);
        i += 1;
    }
}

#[inline]
fn get_upsample(wh: i32, angle: i32, is_sm: bool) -> bool {
    angle < 40 && wh <= 16 >> is_sm as i32
}

#[inline(never)]
fn upsample_edge<P: Pixel>(
    out: &mut [P],
    o: usize,
    hsz: i32,
    r#in: &[P],
    i0: usize,
    from: i32,
    to: i32,
    bitdepth_max: i32,
) {
    static KERNEL: [i8; 4] = [-1, 9, 9, -1];
    let at = |k: i32| r#in[ix(i0, iclip(k, from, to - 1) as isize)];
    let mut i = 0;
    while i < hsz - 1 {
        out[o + (i * 2) as usize] = at(i);

        let mut s = 0;
        for j in 0..4 {
            s += at(i + j - 1).to_i32() * KERNEL[j as usize] as i32;
        }
        out[o + (i * 2 + 1) as usize] = iclip_pixel((s + 8) >> 4, bitdepth_max);
        i += 1;
    }
    out[o + (i * 2) as usize] = at(i);
}

fn ipred_z1_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft_in: &[P],
    tl: usize,
    width: i32,
    height: i32,
    mut angle: i32,
    bitdepth_max: i32,
) {
    let is_sm = (angle >> 9) & 0x1 != 0;
    let enable_intra_edge_filter = (angle >> 10) != 0;
    angle &= 511;
    debug_assert!(angle < 90);
    let mut dx = DR_INTRA_DERIVATIVE[(angle >> 1) as usize] as i32;
    let mut top_out = [P::default(); 64 + 64];
    let top: &[P];
    let top0: usize;
    let max_base_x;
    let upsample_above =
        enable_intra_edge_filter && get_upsample(width + height, 90 - angle, is_sm);
    if upsample_above {
        upsample_edge(
            &mut top_out,
            0,
            width + height,
            topleft_in,
            tl + 1,
            -1,
            width + imin(width, height),
            bitdepth_max,
        );
        top = &top_out;
        top0 = 0;
        max_base_x = 2 * (width + height) - 2;
        dx <<= 1;
    } else {
        let filter_strength = if enable_intra_edge_filter {
            get_filter_strength(width + height, 90 - angle, is_sm)
        } else {
            0
        };
        if filter_strength != 0 {
            filter_edge(
                &mut top_out,
                0,
                width + height,
                0,
                width + height,
                topleft_in,
                tl + 1,
                -1,
                width + imin(width, height),
                filter_strength,
            );
            top = &top_out;
            top0 = 0;
            max_base_x = width + height - 1;
        } else {
            top = topleft_in;
            top0 = tl + 1;
            max_base_x = width + imin(width, height) - 1;
        }
    }
    let base_inc = 1 + upsample_above as i32;
    let mut xpos = dx;
    for _y in 0..height {
        let frac = xpos & 0x3E;

        let mut base = xpos >> 6;
        for x in 0..width {
            if base < max_base_x {
                let v = top[top0 + base as usize].to_i32() * (64 - frac)
                    + top[top0 + base as usize + 1].to_i32() * frac;
                dst[d + x as usize] = P::from_i32((v + 32) >> 6);
            } else {
                dst[d + x as usize..d + width as usize].fill(top[top0 + max_base_x as usize]);
                break;
            }
            base += base_inc;
        }
        d += stride;
        xpos += dx;
    }
}

fn ipred_z2_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft_in: &[P],
    tl: usize,
    width: i32,
    height: i32,
    mut angle: i32,
    max_width: i32,
    max_height: i32,
    bitdepth_max: i32,
) {
    let is_sm = (angle >> 9) & 0x1 != 0;
    let enable_intra_edge_filter = (angle >> 10) != 0;
    angle &= 511;
    debug_assert!(angle > 90 && angle < 180);
    let mut dy = DR_INTRA_DERIVATIVE[((angle - 90) >> 1) as usize] as i32;
    let mut dx = DR_INTRA_DERIVATIVE[((180 - angle) >> 1) as usize] as i32;
    let upsample_left =
        enable_intra_edge_filter && get_upsample(width + height, 180 - angle, is_sm);
    let upsample_above =
        enable_intra_edge_filter && get_upsample(width + height, angle - 90, is_sm);
    let mut edge = [P::default(); 64 + 64 + 1];
    let topleft = 64usize;

    if upsample_above {
        upsample_edge(
            &mut edge,
            topleft,
            width + 1,
            topleft_in,
            tl,
            0,
            width + 1,
            bitdepth_max,
        );
        dx <<= 1;
    } else {
        let filter_strength = if enable_intra_edge_filter {
            get_filter_strength(width + height, angle - 90, is_sm)
        } else {
            0
        };

        if filter_strength != 0 {
            filter_edge(
                &mut edge,
                topleft + 1,
                width,
                0,
                max_width,
                topleft_in,
                tl + 1,
                -1,
                width,
                filter_strength,
            );
        } else {
            edge[topleft + 1..topleft + 1 + width as usize]
                .copy_from_slice(&topleft_in[tl + 1..tl + 1 + width as usize]);
        }
    }
    if upsample_left {
        upsample_edge(
            &mut edge,
            topleft - (height * 2) as usize,
            height + 1,
            topleft_in,
            tl - height as usize,
            0,
            height + 1,
            bitdepth_max,
        );
        dy <<= 1;
    } else {
        let filter_strength = if enable_intra_edge_filter {
            get_filter_strength(width + height, 180 - angle, is_sm)
        } else {
            0
        };

        if filter_strength != 0 {
            filter_edge(
                &mut edge,
                topleft - height as usize,
                height,
                height - max_height,
                height,
                topleft_in,
                tl - height as usize,
                0,
                height + 1,
                filter_strength,
            );
        } else {
            edge[topleft - height as usize..topleft]
                .copy_from_slice(&topleft_in[tl - height as usize..tl]);
        }
    }
    edge[topleft] = topleft_in[tl];

    let base_inc_x = 1 + upsample_above as i32;
    let left = topleft - (1 + upsample_left as usize);
    let mut xpos = ((1 + upsample_above as i32) << 6) - dx;
    for y in 0..height {
        let mut base_x = xpos >> 6;
        let frac_x = xpos & 0x3E;

        let mut ypos = (y << (6 + upsample_left as i32)) - dy;
        for x in 0..width {
            let v;
            if base_x >= 0 {
                v = edge[topleft + base_x as usize].to_i32() * (64 - frac_x)
                    + edge[topleft + base_x as usize + 1].to_i32() * frac_x;
            } else {
                let base_y = ypos >> 6;
                debug_assert!(base_y >= -(1 + upsample_left as i32));
                let frac_y = ypos & 0x3E;
                v = edge[ix(left, -base_y as isize)].to_i32() * (64 - frac_y)
                    + edge[ix(left, -(base_y + 1) as isize)].to_i32() * frac_y;
            }
            dst[d + x as usize] = P::from_i32((v + 32) >> 6);
            base_x += base_inc_x;
            ypos -= dy;
        }
        xpos -= dx;
        d += stride;
    }
}

fn ipred_z3_c<P: Pixel>(
    dst: &mut [P],
    d: usize,
    stride: usize,
    topleft_in: &[P],
    tl: usize,
    width: i32,
    height: i32,
    mut angle: i32,
    bitdepth_max: i32,
) {
    let is_sm = (angle >> 9) & 0x1 != 0;
    let enable_intra_edge_filter = (angle >> 10) != 0;
    angle &= 511;
    debug_assert!(angle > 180);
    let mut dy = DR_INTRA_DERIVATIVE[((270 - angle) >> 1) as usize] as i32;
    let mut left_out = [P::default(); 64 + 64];
    let left: &[P];
    let left0: usize;
    let max_base_y;
    let upsample_left =
        enable_intra_edge_filter && get_upsample(width + height, angle - 180, is_sm);
    if upsample_left {
        upsample_edge(
            &mut left_out,
            0,
            width + height,
            topleft_in,
            tl - (width + height) as usize,
            imax(width - height, 0),
            width + height + 1,
            bitdepth_max,
        );
        left = &left_out;
        left0 = (2 * (width + height) - 2) as usize;
        max_base_y = 2 * (width + height) - 2;
        dy <<= 1;
    } else {
        let filter_strength = if enable_intra_edge_filter {
            get_filter_strength(width + height, angle - 180, is_sm)
        } else {
            0
        };

        if filter_strength != 0 {
            filter_edge(
                &mut left_out,
                0,
                width + height,
                0,
                width + height,
                topleft_in,
                tl - (width + height) as usize,
                imax(width - height, 0),
                width + height + 1,
                filter_strength,
            );
            left = &left_out;
            left0 = (width + height - 1) as usize;
            max_base_y = width + height - 1;
        } else {
            left = topleft_in;
            left0 = tl - 1;
            max_base_y = height + imin(width, height) - 1;
        }
    }
    let base_inc = 1 + upsample_left as i32;
    let mut ypos = dy;
    for x in 0..width as usize {
        let frac = ypos & 0x3E;

        let mut base = ypos >> 6;
        let mut y = 0;
        while y < height as usize {
            if base < max_base_y {
                let v = left[left0 - base as usize].to_i32() * (64 - frac)
                    + left[left0 - (base + 1) as usize].to_i32() * frac;
                dst[d + y * stride + x] = P::from_i32((v + 32) >> 6);
            } else {
                loop {
                    dst[d + y * stride + x] = left[left0 - max_base_y as usize];
                    y += 1;
                    if y >= height as usize {
                        break;
                    }
                }
                break;
            }
            y += 1;
            base += base_inc;
        }
        ypos += dy;
    }
}

/// The non-x86 `FILTER()` macro (`FLT_INCR` 1) over the matching layout of
/// `dav1d_filter_intra_taps`.
#[inline(always)]
fn filter(flt: &[i8], f: usize, p: [i32; 7]) -> i32 {
    flt[f] as i32 * p[0]
        + flt[f + 8] as i32 * p[1]
        + flt[f + 16] as i32 * p[2]
        + flt[f + 24] as i32 * p[3]
        + flt[f + 32] as i32 * p[4]
        + flt[f + 40] as i32 * p[5]
        + flt[f + 48] as i32 * p[6]
}

/* Up to 32x32 only */
fn ipred_filter_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    topleft_in: &[P],
    tl: usize,
    width: i32,
    height: i32,
    mut filt_idx: i32,
    bitdepth_max: i32,
) {
    filt_idx &= 511;
    debug_assert!(filt_idx < 5);

    let flt = &FILTER_INTRA_TAPS[filt_idx as usize];
    // The top row comes from topleft_in for the first row pair, then from
    // dst; `top_in_dst` tracks which buffer `top` points into.
    let mut top_in_dst = false;
    let mut top = tl + 1;
    let mut y = 0;
    while y < height {
        let mut topleft_in_dst = false;
        let mut topleft = ix(tl, -y as isize);
        let mut left_in_dst = false;
        let mut left = topleft - 1;
        let mut left_stride: isize = -1;
        let mut x = 0;
        while x < width as usize {
            let get = |dst: &[P], in_dst: bool, i: usize| -> i32 {
                if in_dst {
                    dst[i].to_i32()
                } else {
                    topleft_in[i].to_i32()
                }
            };
            let p0 = get(dst, topleft_in_dst, topleft);
            let p1 = get(dst, top_in_dst, top);
            let p2 = get(dst, top_in_dst, top + 1);
            let p3 = get(dst, top_in_dst, top + 2);
            let p4 = get(dst, top_in_dst, top + 3);
            let p5 = get(dst, left_in_dst, left);
            let p6 = get(dst, left_in_dst, ix(left, left_stride));
            let p = [p0, p1, p2, p3, p4, p5, p6];
            let mut ptr = d + x;
            let mut f = 0;

            for _yy in 0..2 {
                for xx in 0..4 {
                    let acc = filter(flt, f, p);
                    dst[ptr + xx] = iclip_pixel((acc + 8) >> 4, bitdepth_max);
                    f += 1;
                }
                ptr += stride;
            }
            left = d + x + 4 - 1;
            left_in_dst = true;
            left_stride = stride as isize;
            top += 4;
            topleft = top - 1;
            topleft_in_dst = top_in_dst;
            x += 4;
        }
        top = d + stride;
        top_in_dst = true;
        d += stride * 2;
        y += 2;
    }
}

#[inline(never)]
fn cfl_ac_c<P: Pixel>(
    ac: &mut [i16],
    ypx: &[P],
    mut y0: usize,
    stride: usize,
    w_pad: i32,
    h_pad: i32,
    width: i32,
    height: i32,
    ss_hor: i32,
    ss_ver: i32,
) {
    debug_assert!(w_pad >= 0 && w_pad * 4 < width);
    debug_assert!(h_pad >= 0 && h_pad * 4 < height);
    let w = width as usize;

    let mut a = 0usize;
    let mut y = 0;
    while y < height - 4 * h_pad {
        let mut x = 0;
        while x < width - 4 * w_pad {
            let xu = x as usize;
            let mut ac_sum = ypx[y0 + (xu << ss_hor)].to_i32();
            if ss_hor != 0 {
                ac_sum += ypx[y0 + xu * 2 + 1].to_i32();
            }
            if ss_ver != 0 {
                ac_sum += ypx[y0 + (xu << ss_hor) + stride].to_i32();
                if ss_hor != 0 {
                    ac_sum += ypx[y0 + xu * 2 + 1 + stride].to_i32();
                }
            }
            ac[a + xu] = (ac_sum << (1 + (ss_ver == 0) as i32 + (ss_hor == 0) as i32)) as i16;
            x += 1;
        }
        while x < width {
            ac[a + x as usize] = ac[a + x as usize - 1];
            x += 1;
        }
        a += w;
        y0 += stride << ss_ver;
        y += 1;
    }
    while y < height {
        ac.copy_within(a - w..a, a);
        a += w;
        y += 1;
    }

    let log2sz = ctz(width as u32) + ctz(height as u32);
    let mut sum = (1 << log2sz) >> 1;
    for v in &ac[..w * height as usize] {
        sum += *v as i32;
    }
    sum >>= log2sz;

    // subtract DC
    for v in &mut ac[..w * height as usize] {
        *v = (*v as i32 - sum) as i16;
    }
}

/// Translation of `c->cfl_ac[layout - 1]`: create a subsampled Y plane
/// with the DC subtracted.
/// - w/h_pad is the edge of the width/height that extends outside the visible
///   portion of the frame in 4px units;
/// - ac has a stride of 16.
pub(crate) fn cfl_ac<P: Pixel>(
    layout: i32,
    ac: &mut [i16],
    y: &[P],
    y0: usize,
    stride: usize,
    w_pad: i32,
    h_pad: i32,
    cw: i32,
    ch: i32,
) {
    // cfl_ac_fn(420, 1, 1) / cfl_ac_fn(422, 1, 0) / cfl_ac_fn(444, 0, 0)
    let (ss_hor, ss_ver) = match layout {
        DAV1D_PIXEL_LAYOUT_I420 => (1, 1),
        DAV1D_PIXEL_LAYOUT_I422 => (1, 0),
        _ => (0, 0),
    };
    cfl_ac_c(ac, y, y0, stride, w_pad, h_pad, cw, ch, ss_hor, ss_ver);
}

/// Translation of `c->pal_pred`: dst[x,y] = pal[idx[x,y]]
/// - palette indices are [0-7]
pub(crate) fn pal_pred<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    pal: &[u16; 8],
    idx: &[u8],
    w: i32,
    h: i32,
) {
    let mut i = 0;
    for _y in 0..h {
        for x in 0..w as usize {
            dst[d + x] = P::from_i32(pal[idx[i + x] as usize] as i32);
        }
        i += w as usize;
        d += stride;
    }
}

/// Translation of `c->intra_pred[mode]` (`dav1d_intra_pred_dsp_init()`).
pub(crate) fn intra_pred<P: Pixel>(
    mode: u8,
    dst: &mut [P],
    d: usize,
    stride: usize,
    topleft: &[P],
    tl: usize,
    width: i32,
    height: i32,
    angle: i32,
    max_width: i32,
    max_height: i32,
    bitdepth_max: i32,
) {
    match mode {
        DC_PRED => splat_dc(
            dst,
            d,
            stride,
            width,
            height,
            dc_gen(topleft, tl, width, height) as i32,
        ),
        DC_128_PRED => splat_dc(dst, d, stride, width, height, dc_128::<P>(bitdepth_max)),
        TOP_DC_PRED => splat_dc(
            dst,
            d,
            stride,
            width,
            height,
            dc_gen_top(topleft, tl, width) as i32,
        ),
        LEFT_DC_PRED => splat_dc(
            dst,
            d,
            stride,
            width,
            height,
            dc_gen_left(topleft, tl, height) as i32,
        ),
        HOR_PRED => ipred_h_c(dst, d, stride, topleft, tl, width, height),
        VERT_PRED => ipred_v_c(dst, d, stride, topleft, tl, width, height),
        PAETH_PRED => ipred_paeth_c(dst, d, stride, topleft, tl, width, height),
        SMOOTH_PRED => ipred_smooth_c(dst, d, stride, topleft, tl, width, height),
        SMOOTH_V_PRED => ipred_smooth_v_c(dst, d, stride, topleft, tl, width, height),
        SMOOTH_H_PRED => ipred_smooth_h_c(dst, d, stride, topleft, tl, width, height),
        Z1_PRED => ipred_z1_c(
            dst,
            d,
            stride,
            topleft,
            tl,
            width,
            height,
            angle,
            bitdepth_max,
        ),
        Z2_PRED => ipred_z2_c(
            dst,
            d,
            stride,
            topleft,
            tl,
            width,
            height,
            angle,
            max_width,
            max_height,
            bitdepth_max,
        ),
        Z3_PRED => ipred_z3_c(
            dst,
            d,
            stride,
            topleft,
            tl,
            width,
            height,
            angle,
            bitdepth_max,
        ),
        _ => ipred_filter_c(
            dst,
            d,
            stride,
            topleft,
            tl,
            width,
            height,
            angle,
            bitdepth_max,
        ), // FILTER_PRED
    }
}

/// Translation of `c->cfl_pred[mode]`: dst[x,y] += alpha * ac[x,y]
/// - alpha contains a q3 scalar in [-16,16] range;
pub(crate) fn cfl_pred<P: Pixel>(
    mode: u8,
    dst: &mut [P],
    d: usize,
    stride: usize,
    topleft: &[P],
    tl: usize,
    width: i32,
    height: i32,
    ac: &[i16],
    alpha: i32,
    bitdepth_max: i32,
) {
    let dc = match mode {
        DC_PRED => dc_gen(topleft, tl, width, height) as i32,
        DC_128_PRED => dc_128::<P>(bitdepth_max),
        TOP_DC_PRED => dc_gen_top(topleft, tl, width) as i32,
        _ => dc_gen_left(topleft, tl, height) as i32, // LEFT_DC_PRED
    };
    cfl_pred_c(dst, d, stride, width, height, dc, ac, alpha, bitdepth_max);
}
