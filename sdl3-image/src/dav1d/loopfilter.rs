// Rust translation of src/loopfilter_tmpl.c and src/loopfilter.h from
// dav1d (https://code.videolan.org/videolan/dav1d, at the revision
// SDL_image's external/dav1d pins), the plain-C functions.
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The deblocking filter of one superblock column or row.
//!
//! `loop_filter_sb[plane][dir]` (dimension 1: plane (0=luma, 1=chroma),
//! dimension 2: 0=col-edge filter (h), 1=row-edge filter (v)) is
//! [`loop_filter_sb`]; the level array `lvl` is the flattened
//! `uint8_t (*)[4]` of the frame (`l[k][0]` is `lvl[l + 4 * k]`).

#![allow(clippy::too_many_arguments)]

use super::bitdepth::{bitdepth_from_max, iclip_pixel, ix, Pixel};
use super::intops::{iclip, imin};
use super::lf_mask::Av1FilterLUT;

#[inline(never)]
fn loop_filter<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    mut e: i32,
    mut i: i32,
    mut h: i32,
    stridea: isize,
    strideb: isize,
    wd: i32,
    bitdepth_max: i32,
) {
    let bitdepth_min_8 = bitdepth_from_max(bitdepth_max) - 8;
    let f = 1 << bitdepth_min_8;
    e <<= bitdepth_min_8;
    i <<= bitdepth_min_8;
    h <<= bitdepth_min_8;

    for _i in 0..4 {
        let at = |dst: &[P], k: isize| dst[ix(d, strideb * k)].to_i32();
        let (mut p6, mut p5, mut p4, mut p3, mut p2) = (0, 0, 0, 0, 0);
        let p1 = at(dst, -2);
        let p0 = at(dst, -1);
        let q0 = at(dst, 0);
        let q1 = at(dst, 1);
        let (mut q2, mut q3, mut q4, mut q5, mut q6) = (0, 0, 0, 0, 0);
        let mut flat8out = false;
        let mut flat8in = false;

        let mut fm = (p1 - p0).abs() <= i
            && (q1 - q0).abs() <= i
            && (p0 - q0).abs() * 2 + ((p1 - q1).abs() >> 1) <= e;

        if wd > 4 {
            p2 = at(dst, -3);
            q2 = at(dst, 2);

            fm &= (p2 - p1).abs() <= i && (q2 - q1).abs() <= i;

            if wd > 6 {
                p3 = at(dst, -4);
                q3 = at(dst, 3);

                fm &= (p3 - p2).abs() <= i && (q3 - q2).abs() <= i;
            }
        }
        if !fm {
            d = ix(d, stridea);
            continue;
        }

        if wd >= 16 {
            p6 = at(dst, -7);
            p5 = at(dst, -6);
            p4 = at(dst, -5);
            q4 = at(dst, 4);
            q5 = at(dst, 5);
            q6 = at(dst, 6);

            flat8out = (p6 - p0).abs() <= f
                && (p5 - p0).abs() <= f
                && (p4 - p0).abs() <= f
                && (q4 - q0).abs() <= f
                && (q5 - q0).abs() <= f
                && (q6 - q0).abs() <= f;
        }

        if wd >= 6 {
            flat8in = (p2 - p0).abs() <= f
                && (p1 - p0).abs() <= f
                && (q1 - q0).abs() <= f
                && (q2 - q0).abs() <= f;
        }

        if wd >= 8 {
            flat8in &= (p3 - p0).abs() <= f && (q3 - q0).abs() <= f;
        }

        let mut set = |k: isize, v: i32| dst[ix(d, strideb * k)] = P::from_i32(v);
        if wd >= 16 && (flat8out & flat8in) {
            set(
                -6,
                (p6 + p6 + p6 + p6 + p6 + p6 * 2 + p5 * 2 + p4 * 2 + p3 + p2 + p1 + p0 + q0 + 8)
                    >> 4,
            );
            set(
                -5,
                (p6 + p6 + p6 + p6 + p6 + p5 * 2 + p4 * 2 + p3 * 2 + p2 + p1 + p0 + q0 + q1 + 8)
                    >> 4,
            );
            set(
                -4,
                (p6 + p6 + p6 + p6 + p5 + p4 * 2 + p3 * 2 + p2 * 2 + p1 + p0 + q0 + q1 + q2 + 8)
                    >> 4,
            );
            set(
                -3,
                (p6 + p6 + p6 + p5 + p4 + p3 * 2 + p2 * 2 + p1 * 2 + p0 + q0 + q1 + q2 + q3 + 8)
                    >> 4,
            );
            set(
                -2,
                (p6 + p6 + p5 + p4 + p3 + p2 * 2 + p1 * 2 + p0 * 2 + q0 + q1 + q2 + q3 + q4 + 8)
                    >> 4,
            );
            set(
                -1,
                (p6 + p5 + p4 + p3 + p2 + p1 * 2 + p0 * 2 + q0 * 2 + q1 + q2 + q3 + q4 + q5 + 8)
                    >> 4,
            );
            set(
                0,
                (p5 + p4 + p3 + p2 + p1 + p0 * 2 + q0 * 2 + q1 * 2 + q2 + q3 + q4 + q5 + q6 + 8)
                    >> 4,
            );
            set(
                1,
                (p4 + p3 + p2 + p1 + p0 + q0 * 2 + q1 * 2 + q2 * 2 + q3 + q4 + q5 + q6 + q6 + 8)
                    >> 4,
            );
            set(
                2,
                (p3 + p2 + p1 + p0 + q0 + q1 * 2 + q2 * 2 + q3 * 2 + q4 + q5 + q6 + q6 + q6 + 8)
                    >> 4,
            );
            set(
                3,
                (p2 + p1 + p0 + q0 + q1 + q2 * 2 + q3 * 2 + q4 * 2 + q5 + q6 + q6 + q6 + q6 + 8)
                    >> 4,
            );
            set(
                4,
                (p1 + p0 + q0 + q1 + q2 + q3 * 2 + q4 * 2 + q5 * 2 + q6 + q6 + q6 + q6 + q6 + 8)
                    >> 4,
            );
            set(
                5,
                (p0 + q0 + q1 + q2 + q3 + q4 * 2 + q5 * 2 + q6 * 2 + q6 + q6 + q6 + q6 + q6 + 8)
                    >> 4,
            );
        } else if wd >= 8 && flat8in {
            set(-3, (p3 + p3 + p3 + 2 * p2 + p1 + p0 + q0 + 4) >> 3);
            set(-2, (p3 + p3 + p2 + 2 * p1 + p0 + q0 + q1 + 4) >> 3);
            set(-1, (p3 + p2 + p1 + 2 * p0 + q0 + q1 + q2 + 4) >> 3);
            set(0, (p2 + p1 + p0 + 2 * q0 + q1 + q2 + q3 + 4) >> 3);
            set(1, (p1 + p0 + q0 + 2 * q1 + q2 + q3 + q3 + 4) >> 3);
            set(2, (p0 + q0 + q1 + 2 * q2 + q3 + q3 + q3 + 4) >> 3);
        } else if wd == 6 && flat8in {
            set(-2, (p2 + 2 * p2 + 2 * p1 + 2 * p0 + q0 + 4) >> 3);
            set(-1, (p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3);
            set(0, (p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3);
            set(1, (p0 + 2 * q0 + 2 * q1 + 2 * q2 + q2 + 4) >> 3);
        } else {
            let hev = (p1 - p0).abs() > h || (q1 - q0).abs() > h;

            let iclip_diff = |v: i32| {
                iclip(
                    v,
                    -128 * (1 << bitdepth_min_8),
                    128 * (1 << bitdepth_min_8) - 1,
                )
            };
            let mut setp = |k: isize, v: i32| {
                dst[ix(d, strideb * k)] = iclip_pixel::<P>(v, bitdepth_max);
            };

            if hev {
                let mut f = iclip_diff(p1 - q1);
                f = iclip_diff(3 * (q0 - p0) + f);

                let f1 = imin(f + 4, (128 << bitdepth_min_8) - 1) >> 3;
                let f2 = imin(f + 3, (128 << bitdepth_min_8) - 1) >> 3;

                setp(-1, p0 + f2);
                setp(0, q0 - f1);
            } else {
                let mut f = iclip_diff(3 * (q0 - p0));

                let f1 = imin(f + 4, (128 << bitdepth_min_8) - 1) >> 3;
                let f2 = imin(f + 3, (128 << bitdepth_min_8) - 1) >> 3;

                setp(-1, p0 + f2);
                setp(0, q0 - f1);

                f = (f1 + 1) >> 1;
                setp(-2, p1 + f);
                setp(1, q1 - f);
            }
        }
        d = ix(d, stridea);
    }
}

fn loop_filter_h_sb128y_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    vmask: &[u32],
    lvl: &[u8],
    mut l: usize,
    b4_stride: usize,
    lut: &Av1FilterLUT,
    _h: i32,
    bitdepth_max: i32,
) {
    let vm = vmask[0] | vmask[1] | vmask[2];
    let mut y: u32 = 1;
    while vm & !(y.wrapping_sub(1)) != 0 {
        if vm & y != 0 {
            let lv = if lvl[l] != 0 { lvl[l] } else { lvl[l - 4] } as i32;
            if lv != 0 {
                let h = lv >> 4;
                let e = lut.e[lv as usize] as i32;
                let i = lut.i[lv as usize] as i32;
                let idx = if vmask[2] & y != 0 {
                    2
                } else {
                    (vmask[1] & y != 0) as i32
                };
                loop_filter(dst, d, e, i, h, stride as isize, 1, 4 << idx, bitdepth_max);
            }
        }
        y <<= 1;
        d += 4 * stride;
        l += 4 * b4_stride;
        if y == 0 {
            break;
        }
    }
}

fn loop_filter_v_sb128y_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    vmask: &[u32],
    lvl: &[u8],
    mut l: usize,
    b4_stride: usize,
    lut: &Av1FilterLUT,
    _w: i32,
    bitdepth_max: i32,
) {
    let vm = vmask[0] | vmask[1] | vmask[2];
    let mut x: u32 = 1;
    while vm & !(x.wrapping_sub(1)) != 0 {
        if vm & x != 0 {
            let lv = if lvl[l] != 0 {
                lvl[l]
            } else {
                lvl[l - 4 * b4_stride]
            } as i32;
            if lv != 0 {
                let h = lv >> 4;
                let e = lut.e[lv as usize] as i32;
                let i = lut.i[lv as usize] as i32;
                let idx = if vmask[2] & x != 0 {
                    2
                } else {
                    (vmask[1] & x != 0) as i32
                };
                loop_filter(dst, d, e, i, h, 1, stride as isize, 4 << idx, bitdepth_max);
            }
        }
        x <<= 1;
        d += 4;
        l += 4;
        if x == 0 {
            break;
        }
    }
}

fn loop_filter_h_sb128uv_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    vmask: &[u32],
    lvl: &[u8],
    mut l: usize,
    b4_stride: usize,
    lut: &Av1FilterLUT,
    _h: i32,
    bitdepth_max: i32,
) {
    let vm = vmask[0] | vmask[1];
    let mut y: u32 = 1;
    while vm & !(y.wrapping_sub(1)) != 0 {
        if vm & y != 0 {
            let lv = if lvl[l] != 0 { lvl[l] } else { lvl[l - 4] } as i32;
            if lv != 0 {
                let h = lv >> 4;
                let e = lut.e[lv as usize] as i32;
                let i = lut.i[lv as usize] as i32;
                let idx = (vmask[1] & y != 0) as i32;
                loop_filter(
                    dst,
                    d,
                    e,
                    i,
                    h,
                    stride as isize,
                    1,
                    4 + 2 * idx,
                    bitdepth_max,
                );
            }
        }
        y <<= 1;
        d += 4 * stride;
        l += 4 * b4_stride;
        if y == 0 {
            break;
        }
    }
}

fn loop_filter_v_sb128uv_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    stride: usize,
    vmask: &[u32],
    lvl: &[u8],
    mut l: usize,
    b4_stride: usize,
    lut: &Av1FilterLUT,
    _w: i32,
    bitdepth_max: i32,
) {
    let vm = vmask[0] | vmask[1];
    let mut x: u32 = 1;
    while vm & !(x.wrapping_sub(1)) != 0 {
        if vm & x != 0 {
            let lv = if lvl[l] != 0 {
                lvl[l]
            } else {
                lvl[l - 4 * b4_stride]
            } as i32;
            if lv != 0 {
                let h = lv >> 4;
                let e = lut.e[lv as usize] as i32;
                let i = lut.i[lv as usize] as i32;
                let idx = (vmask[1] & x != 0) as i32;
                loop_filter(
                    dst,
                    d,
                    e,
                    i,
                    h,
                    1,
                    stride as isize,
                    4 + 2 * idx,
                    bitdepth_max,
                );
            }
        }
        x <<= 1;
        d += 4;
        l += 4;
        if x == 0 {
            break;
        }
    }
}

/// Translation of `c->loop_filter_sb[plane][dir]`: `dst[d..]` is the
/// superblock column or row (`stride` in pixels), `lvl[l..]` its levels.
pub(crate) fn loop_filter_sb<P: Pixel>(
    plane: usize,
    dir: usize,
    dst: &mut [P],
    d: usize,
    stride: usize,
    mask: &[u32],
    lvl: &[u8],
    l: usize,
    b4_stride: usize,
    lut: &Av1FilterLUT,
    wh: i32,
    bitdepth_max: i32,
) {
    match (plane, dir) {
        (0, 0) => loop_filter_h_sb128y_c(
            dst,
            d,
            stride,
            mask,
            lvl,
            l,
            b4_stride,
            lut,
            wh,
            bitdepth_max,
        ),
        (0, _) => loop_filter_v_sb128y_c(
            dst,
            d,
            stride,
            mask,
            lvl,
            l,
            b4_stride,
            lut,
            wh,
            bitdepth_max,
        ),
        (_, 0) => loop_filter_h_sb128uv_c(
            dst,
            d,
            stride,
            mask,
            lvl,
            l,
            b4_stride,
            lut,
            wh,
            bitdepth_max,
        ),
        _ => loop_filter_v_sb128uv_c(
            dst,
            d,
            stride,
            mask,
            lvl,
            l,
            b4_stride,
            lut,
            wh,
            bitdepth_max,
        ),
    }
}
