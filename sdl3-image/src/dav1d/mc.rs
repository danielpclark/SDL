// Rust translation of src/mc_tmpl.c and src/mc.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins), the plain-C functions.
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion compensation: the subpel (8-tap and bilinear, plain and scaled)
//! put and prep ("mct") filters, the compound averages and masks, the OBMC
//! blends, the affine warp, the edge emulation and the super-resolution
//! upscaler.
//!
//! The `Dav1dMCDSPContext` function tables are the dispatch functions
//! [`mc`], [`mc_scaled`], [`mct`] and [`mct_scaled`] (indexed by the
//! `FILTER_2D_*` type); the others are called directly. Pointers are
//! (slice, offset) pairs and strides are in pixels.

use super::bitdepth::{bitdepth_from_max, iclip_pixel, ix, Pixel};
use super::headers::{
    DAV1D_FILTER_8TAP_REGULAR, DAV1D_FILTER_8TAP_SHARP, DAV1D_FILTER_8TAP_SMOOTH,
};
use super::intops::{iclip, imin};
use super::levels::*;
use super::tables::{MC_SUBPEL_FILTERS, MC_WARP_FILTER, OBMC_MASKS, RESIZE_FILTER};

#[inline(always)]
fn get_intermediate_bits<P: Pixel>(bitdepth_max: i32) -> i32 {
    if P::BPC8 {
        // Output in interval [-5132, 9212], fits in int16_t as is
        4
    } else {
        // 4 for 10 bits/component, 2 for 12 bits/component
        14 - bitdepth_from_max(bitdepth_max)
    }
}

/// `PREP_BIAS`: 0 for 8 bpc; for 16 bpc the output is in the interval
/// [-20588, 36956] (10-bit), [-20602, 36983] (12-bit), so a bias is
/// subtracted to ensure the output fits in int16_t.
#[inline(always)]
fn prep_bias<P: Pixel>() -> i32 {
    if P::BPC8 {
        0
    } else {
        8192
    }
}

#[inline(never)]
fn put_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
) {
    for _ in 0..h {
        dst[d..d + w].copy_from_slice(&src[s..s + w]);

        d += dst_stride;
        s += src_stride;
    }
}

#[inline(never)]
fn prep_c<P: Pixel>(
    tmp: &mut [i16],
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let mut t = 0;
    for _ in 0..h {
        for x in 0..w {
            tmp[t + x] = ((src[s + x].to_i32() << intermediate_bits) - prep_bias::<P>()) as i16;
        }

        t += w;
        s += src_stride;
    }
}

/// `FILTER_8TAP(src, x, F, stride)` on pixels.
#[inline(always)]
fn filter_8tap<P: Pixel>(src: &[P], i: usize, f: &[i8; 8], stride: usize) -> i32 {
    let b = i - 3 * stride;
    f[0] as i32 * src[b].to_i32()
        + f[1] as i32 * src[b + stride].to_i32()
        + f[2] as i32 * src[b + 2 * stride].to_i32()
        + f[3] as i32 * src[b + 3 * stride].to_i32()
        + f[4] as i32 * src[b + 4 * stride].to_i32()
        + f[5] as i32 * src[b + 5 * stride].to_i32()
        + f[6] as i32 * src[b + 6 * stride].to_i32()
        + f[7] as i32 * src[b + 7 * stride].to_i32()
}

/// `FILTER_8TAP(src, x, F, stride)` on the `int16_t` intermediate.
#[inline(always)]
fn filter_8tap_mid(src: &[i16], i: usize, f: &[i8; 8], stride: usize) -> i32 {
    let b = i - 3 * stride;
    f[0] as i32 * src[b] as i32
        + f[1] as i32 * src[b + stride] as i32
        + f[2] as i32 * src[b + 2 * stride] as i32
        + f[3] as i32 * src[b + 3 * stride] as i32
        + f[4] as i32 * src[b + 4 * stride] as i32
        + f[5] as i32 * src[b + 5 * stride] as i32
        + f[6] as i32 * src[b + 6 * stride] as i32
        + f[7] as i32 * src[b + 7 * stride] as i32
}

/// `DAV1D_FILTER_8TAP_RND`
#[inline(always)]
fn rnd(v: i32, sh: i32) -> i32 {
    (v + ((1 << sh) >> 1)) >> sh
}

/// `GET_H_FILTER(mx)`
#[inline(always)]
fn get_h_filter(mx: i32, w: usize, filter_type: i32) -> Option<&'static [i8; 8]> {
    if mx == 0 {
        None
    } else if w > 4 {
        Some(&MC_SUBPEL_FILTERS[(filter_type & 3) as usize][(mx - 1) as usize])
    } else {
        Some(&MC_SUBPEL_FILTERS[3 + (filter_type & 1) as usize][(mx - 1) as usize])
    }
}

/// `GET_V_FILTER(my)`
#[inline(always)]
fn get_v_filter(my: i32, h: usize, filter_type: i32) -> Option<&'static [i8; 8]> {
    if my == 0 {
        None
    } else if h > 4 {
        Some(&MC_SUBPEL_FILTERS[(filter_type >> 2) as usize][(my - 1) as usize])
    } else {
        Some(&MC_SUBPEL_FILTERS[3 + ((filter_type >> 2) & 1) as usize][(my - 1) as usize])
    }
}

#[inline(never)]
fn put_8tap_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    filter_type: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let intermediate_rnd = 32 + ((1 << (6 - intermediate_bits)) >> 1);

    let fh = get_h_filter(mx, w, filter_type);
    let fv = get_v_filter(my, h, filter_type);

    if let Some(fh) = fh {
        if let Some(fv) = fv {
            let tmp_h = h + 7;
            let mut mid = vec![0i16; 128 * 135];
            let mut m = 0;

            s -= src_stride * 3;
            for _ in 0..tmp_h {
                for x in 0..w {
                    mid[m + x] = rnd(filter_8tap(src, s + x, fh, 1), 6 - intermediate_bits) as i16;
                }

                m += 128;
                s += src_stride;
            }

            m = 128 * 3;
            for _ in 0..h {
                for x in 0..w {
                    dst[d + x] = iclip_pixel(
                        rnd(filter_8tap_mid(&mid, m + x, fv, 128), 6 + intermediate_bits),
                        bitdepth_max,
                    );
                }

                m += 128;
                d += dst_stride;
            }
        } else {
            for _ in 0..h {
                for x in 0..w {
                    dst[d + x] = iclip_pixel(
                        (filter_8tap(src, s + x, fh, 1) + intermediate_rnd) >> 6,
                        bitdepth_max,
                    );
                }

                d += dst_stride;
                s += src_stride;
            }
        }
    } else if let Some(fv) = fv {
        for _ in 0..h {
            for x in 0..w {
                dst[d + x] = iclip_pixel(
                    rnd(filter_8tap(src, s + x, fv, src_stride), 6),
                    bitdepth_max,
                );
            }

            d += dst_stride;
            s += src_stride;
        }
    } else {
        put_c(dst, d, dst_stride, src, s, src_stride, w, h);
    }
}

#[inline(never)]
fn put_8tap_scaled_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    mut my: i32,
    dx: i32,
    dy: i32,
    filter_type: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let intermediate_rnd = (1 << intermediate_bits) >> 1;
    let tmp_h = ((((h as i32 - 1) * dy + my) >> 10) + 8) as usize;
    let mut mid = vec![0i16; 128 * (256 + 7)];
    let mut m = 0;

    s -= src_stride * 3;
    for _ in 0..tmp_h {
        let mut imx = mx;
        let mut ioff = 0usize;

        for x in 0..w {
            let fh = get_h_filter(imx >> 6, w, filter_type);
            mid[m + x] = match fh {
                Some(fh) => rnd(filter_8tap(src, s + ioff, fh, 1), 6 - intermediate_bits),
                None => src[s + ioff].to_i32() << intermediate_bits,
            } as i16;
            imx += dx;
            ioff += (imx >> 10) as usize;
            imx &= 0x3ff;
        }

        m += 128;
        s += src_stride;
    }

    m = 128 * 3;
    for _ in 0..h {
        let fv = get_v_filter(my >> 6, h, filter_type);

        for x in 0..w {
            dst[d + x] = match fv {
                Some(fv) => iclip_pixel(
                    rnd(filter_8tap_mid(&mid, m + x, fv, 128), 6 + intermediate_bits),
                    bitdepth_max,
                ),
                None => iclip_pixel(
                    (mid[m + x] as i32 + intermediate_rnd) >> intermediate_bits,
                    bitdepth_max,
                ),
            };
        }

        my += dy;
        m += (my >> 10) as usize * 128;
        my &= 0x3ff;
        d += dst_stride;
    }
}

#[inline(never)]
fn prep_8tap_c<P: Pixel>(
    tmp: &mut [i16],
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    filter_type: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let fh = get_h_filter(mx, w, filter_type);
    let fv = get_v_filter(my, h, filter_type);
    let mut t = 0;

    if let Some(fh) = fh {
        if let Some(fv) = fv {
            let tmp_h = h + 7;
            let mut mid = vec![0i16; 128 * 135];
            let mut m = 0;

            s -= src_stride * 3;
            for _ in 0..tmp_h {
                for x in 0..w {
                    mid[m + x] = rnd(filter_8tap(src, s + x, fh, 1), 6 - intermediate_bits) as i16;
                }

                m += 128;
                s += src_stride;
            }

            m = 128 * 3;
            for _ in 0..h {
                for x in 0..w {
                    let v = rnd(filter_8tap_mid(&mid, m + x, fv, 128), 6) - prep_bias::<P>();
                    debug_assert!(v >= i16::MIN as i32 && v <= i16::MAX as i32);
                    tmp[t + x] = v as i16;
                }

                m += 128;
                t += w;
            }
        } else {
            for _ in 0..h {
                for x in 0..w {
                    tmp[t + x] = (rnd(filter_8tap(src, s + x, fh, 1), 6 - intermediate_bits)
                        - prep_bias::<P>()) as i16;
                }

                t += w;
                s += src_stride;
            }
        }
    } else if let Some(fv) = fv {
        for _ in 0..h {
            for x in 0..w {
                tmp[t + x] = (rnd(
                    filter_8tap(src, s + x, fv, src_stride),
                    6 - intermediate_bits,
                ) - prep_bias::<P>()) as i16;
            }

            t += w;
            s += src_stride;
        }
    } else {
        prep_c(tmp, src, s, src_stride, w, h, bitdepth_max);
    }
}

#[inline(never)]
fn prep_8tap_scaled_c<P: Pixel>(
    tmp: &mut [i16],
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    mut my: i32,
    dx: i32,
    dy: i32,
    filter_type: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let tmp_h = ((((h as i32 - 1) * dy + my) >> 10) + 8) as usize;
    let mut mid = vec![0i16; 128 * (256 + 7)];
    let mut m = 0;
    let mut t = 0;

    s -= src_stride * 3;
    for _ in 0..tmp_h {
        let mut imx = mx;
        let mut ioff = 0usize;

        for x in 0..w {
            let fh = get_h_filter(imx >> 6, w, filter_type);
            mid[m + x] = match fh {
                Some(fh) => rnd(filter_8tap(src, s + ioff, fh, 1), 6 - intermediate_bits),
                None => src[s + ioff].to_i32() << intermediate_bits,
            } as i16;
            imx += dx;
            ioff += (imx >> 10) as usize;
            imx &= 0x3ff;
        }

        m += 128;
        s += src_stride;
    }

    m = 128 * 3;
    for _ in 0..h {
        let fv = get_v_filter(my >> 6, h, filter_type);

        for x in 0..w {
            tmp[t + x] = (match fv {
                Some(fv) => rnd(filter_8tap_mid(&mid, m + x, fv, 128), 6),
                None => mid[m + x] as i32,
            } - prep_bias::<P>()) as i16;
        }

        my += dy;
        m += (my >> 10) as usize * 128;
        my &= 0x3ff;
        t += w;
    }
}

// (filter_fns(): the 9 type_h | (type_v << 2) combinations)
fn filter_type_2d(filter_2d: u8) -> i32 {
    let (type_h, type_v) = match filter_2d {
        FILTER_2D_8TAP_REGULAR => (DAV1D_FILTER_8TAP_REGULAR, DAV1D_FILTER_8TAP_REGULAR),
        FILTER_2D_8TAP_REGULAR_SHARP => (DAV1D_FILTER_8TAP_REGULAR, DAV1D_FILTER_8TAP_SHARP),
        FILTER_2D_8TAP_REGULAR_SMOOTH => (DAV1D_FILTER_8TAP_REGULAR, DAV1D_FILTER_8TAP_SMOOTH),
        FILTER_2D_8TAP_SMOOTH => (DAV1D_FILTER_8TAP_SMOOTH, DAV1D_FILTER_8TAP_SMOOTH),
        FILTER_2D_8TAP_SMOOTH_REGULAR => (DAV1D_FILTER_8TAP_SMOOTH, DAV1D_FILTER_8TAP_REGULAR),
        FILTER_2D_8TAP_SMOOTH_SHARP => (DAV1D_FILTER_8TAP_SMOOTH, DAV1D_FILTER_8TAP_SHARP),
        FILTER_2D_8TAP_SHARP => (DAV1D_FILTER_8TAP_SHARP, DAV1D_FILTER_8TAP_SHARP),
        FILTER_2D_8TAP_SHARP_REGULAR => (DAV1D_FILTER_8TAP_SHARP, DAV1D_FILTER_8TAP_REGULAR),
        FILTER_2D_8TAP_SHARP_SMOOTH => (DAV1D_FILTER_8TAP_SHARP, DAV1D_FILTER_8TAP_SMOOTH),
        _ => unreachable!("not an 8-tap filter"),
    };
    type_h | (type_v << 2)
}

/// `FILTER_BILIN(src, x, mxy, stride)` on pixels.
#[inline(always)]
fn filter_bilin<P: Pixel>(src: &[P], i: usize, mxy: i32, stride: usize) -> i32 {
    16 * src[i].to_i32() + mxy * (src[i + stride].to_i32() - src[i].to_i32())
}

/// `FILTER_BILIN(src, x, mxy, stride)` on the `int16_t` intermediate.
#[inline(always)]
fn filter_bilin_mid(src: &[i16], i: usize, mxy: i32, stride: usize) -> i32 {
    16 * src[i] as i32 + mxy * (src[i + stride] as i32 - src[i] as i32)
}

fn put_bilin_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let intermediate_rnd = (1 << intermediate_bits) >> 1;

    if mx != 0 {
        if my != 0 {
            let mut mid = vec![0i16; 128 * 129];
            let mut m = 0;
            let tmp_h = h + 1;

            for _ in 0..tmp_h {
                for x in 0..w {
                    mid[m + x] = rnd(filter_bilin(src, s + x, mx, 1), 4 - intermediate_bits) as i16;
                }

                m += 128;
                s += src_stride;
            }

            m = 0;
            for _ in 0..h {
                for x in 0..w {
                    dst[d + x] = iclip_pixel(
                        rnd(
                            filter_bilin_mid(&mid, m + x, my, 128),
                            4 + intermediate_bits,
                        ),
                        bitdepth_max,
                    );
                }

                m += 128;
                d += dst_stride;
            }
        } else {
            for _ in 0..h {
                for x in 0..w {
                    let px = rnd(filter_bilin(src, s + x, mx, 1), 4 - intermediate_bits);
                    dst[d + x] =
                        iclip_pixel((px + intermediate_rnd) >> intermediate_bits, bitdepth_max);
                }

                d += dst_stride;
                s += src_stride;
            }
        }
    } else if my != 0 {
        for _ in 0..h {
            for x in 0..w {
                dst[d + x] = iclip_pixel(
                    rnd(filter_bilin(src, s + x, my, src_stride), 4),
                    bitdepth_max,
                );
            }

            d += dst_stride;
            s += src_stride;
        }
    } else {
        put_c(dst, d, dst_stride, src, s, src_stride, w, h);
    }
}

fn put_bilin_scaled_c<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    mut my: i32,
    dx: i32,
    dy: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let tmp_h = ((((h as i32 - 1) * dy + my) >> 10) + 2) as usize;
    let mut mid = vec![0i16; 128 * (256 + 1)];
    let mut m = 0;

    for _ in 0..tmp_h {
        let mut imx = mx;
        let mut ioff = 0usize;

        for x in 0..w {
            mid[m + x] = rnd(
                filter_bilin(src, s + ioff, imx >> 6, 1),
                4 - intermediate_bits,
            ) as i16;
            imx += dx;
            ioff += (imx >> 10) as usize;
            imx &= 0x3ff;
        }

        m += 128;
        s += src_stride;
    }

    m = 0;
    for _ in 0..h {
        for x in 0..w {
            dst[d + x] = iclip_pixel(
                rnd(
                    filter_bilin_mid(&mid, m + x, my >> 6, 128),
                    4 + intermediate_bits,
                ),
                bitdepth_max,
            );
        }

        my += dy;
        m += (my >> 10) as usize * 128;
        my &= 0x3ff;
        d += dst_stride;
    }
}

fn prep_bilin_c<P: Pixel>(
    tmp: &mut [i16],
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let mut t = 0;

    if mx != 0 {
        if my != 0 {
            let mut mid = vec![0i16; 128 * 129];
            let mut m = 0;
            let tmp_h = h + 1;

            for _ in 0..tmp_h {
                for x in 0..w {
                    mid[m + x] = rnd(filter_bilin(src, s + x, mx, 1), 4 - intermediate_bits) as i16;
                }

                m += 128;
                s += src_stride;
            }

            m = 0;
            for _ in 0..h {
                for x in 0..w {
                    tmp[t + x] =
                        (rnd(filter_bilin_mid(&mid, m + x, my, 128), 4) - prep_bias::<P>()) as i16;
                }

                m += 128;
                t += w;
            }
        } else {
            for _ in 0..h {
                for x in 0..w {
                    tmp[t + x] = (rnd(filter_bilin(src, s + x, mx, 1), 4 - intermediate_bits)
                        - prep_bias::<P>()) as i16;
                }

                t += w;
                s += src_stride;
            }
        }
    } else if my != 0 {
        for _ in 0..h {
            for x in 0..w {
                tmp[t + x] = (rnd(
                    filter_bilin(src, s + x, my, src_stride),
                    4 - intermediate_bits,
                ) - prep_bias::<P>()) as i16;
            }

            t += w;
            s += src_stride;
        }
    } else {
        prep_c(tmp, src, s, src_stride, w, h, bitdepth_max);
    }
}

fn prep_bilin_scaled_c<P: Pixel>(
    tmp: &mut [i16],
    src: &[P],
    mut s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    mut my: i32,
    dx: i32,
    dy: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let tmp_h = ((((h as i32 - 1) * dy + my) >> 10) + 2) as usize;
    let mut mid = vec![0i16; 128 * (256 + 1)];
    let mut m = 0;
    let mut t = 0;

    for _ in 0..tmp_h {
        let mut imx = mx;
        let mut ioff = 0usize;

        for x in 0..w {
            mid[m + x] = rnd(
                filter_bilin(src, s + ioff, imx >> 6, 1),
                4 - intermediate_bits,
            ) as i16;
            imx += dx;
            ioff += (imx >> 10) as usize;
            imx &= 0x3ff;
        }

        m += 128;
        s += src_stride;
    }

    m = 0;
    for _ in 0..h {
        for x in 0..w {
            tmp[t + x] =
                (rnd(filter_bilin_mid(&mid, m + x, my >> 6, 128), 4) - prep_bias::<P>()) as i16;
        }

        my += dy;
        m += (my >> 10) as usize * 128;
        my &= 0x3ff;
        t += w;
    }
}

/// `dsp->mc.mc[filter_2d]`
pub(crate) fn mc<P: Pixel>(
    filter_2d: u8,
    dst: &mut [P],
    d: usize,
    dst_stride: usize,
    src: &[P],
    s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    bitdepth_max: i32,
) {
    if filter_2d == FILTER_2D_BILINEAR {
        put_bilin_c(
            dst,
            d,
            dst_stride,
            src,
            s,
            src_stride,
            w,
            h,
            mx,
            my,
            bitdepth_max,
        );
    } else {
        let ft = filter_type_2d(filter_2d);
        put_8tap_c(
            dst,
            d,
            dst_stride,
            src,
            s,
            src_stride,
            w,
            h,
            mx,
            my,
            ft,
            bitdepth_max,
        );
    }
}

/// `dsp->mc.mc_scaled[filter_2d]`
pub(crate) fn mc_scaled<P: Pixel>(
    filter_2d: u8,
    dst: &mut [P],
    d: usize,
    dst_stride: usize,
    src: &[P],
    s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    dx: i32,
    dy: i32,
    bitdepth_max: i32,
) {
    if filter_2d == FILTER_2D_BILINEAR {
        put_bilin_scaled_c(
            dst,
            d,
            dst_stride,
            src,
            s,
            src_stride,
            w,
            h,
            mx,
            my,
            dx,
            dy,
            bitdepth_max,
        );
    } else {
        let ft = filter_type_2d(filter_2d);
        put_8tap_scaled_c(
            dst,
            d,
            dst_stride,
            src,
            s,
            src_stride,
            w,
            h,
            mx,
            my,
            dx,
            dy,
            ft,
            bitdepth_max,
        );
    }
}

/// `dsp->mc.mct[filter_2d]`
pub(crate) fn mct<P: Pixel>(
    filter_2d: u8,
    tmp: &mut [i16],
    src: &[P],
    s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    bitdepth_max: i32,
) {
    if filter_2d == FILTER_2D_BILINEAR {
        prep_bilin_c(tmp, src, s, src_stride, w, h, mx, my, bitdepth_max);
    } else {
        let ft = filter_type_2d(filter_2d);
        prep_8tap_c(tmp, src, s, src_stride, w, h, mx, my, ft, bitdepth_max);
    }
}

/// `dsp->mc.mct_scaled[filter_2d]`
pub(crate) fn mct_scaled<P: Pixel>(
    filter_2d: u8,
    tmp: &mut [i16],
    src: &[P],
    s: usize,
    src_stride: usize,
    w: usize,
    h: usize,
    mx: i32,
    my: i32,
    dx: i32,
    dy: i32,
    bitdepth_max: i32,
) {
    if filter_2d == FILTER_2D_BILINEAR {
        prep_bilin_scaled_c(tmp, src, s, src_stride, w, h, mx, my, dx, dy, bitdepth_max);
    } else {
        let ft = filter_type_2d(filter_2d);
        prep_8tap_scaled_c(
            tmp,
            src,
            s,
            src_stride,
            w,
            h,
            mx,
            my,
            dx,
            dy,
            ft,
            bitdepth_max,
        );
    }
}

/// `avg_c`
pub(crate) fn avg<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    tmp1: &[i16],
    tmp2: &[i16],
    w: usize,
    h: usize,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let sh = intermediate_bits + 1;
    let rnd = (1 << intermediate_bits) + prep_bias::<P>() * 2;
    let mut t = 0;
    for _ in 0..h {
        for x in 0..w {
            dst[d + x] = iclip_pixel(
                (tmp1[t + x] as i32 + tmp2[t + x] as i32 + rnd) >> sh,
                bitdepth_max,
            );
        }

        t += w;
        d += dst_stride;
    }
}

/// `w_avg_c`
pub(crate) fn w_avg<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    tmp1: &[i16],
    tmp2: &[i16],
    w: usize,
    h: usize,
    weight: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let sh = intermediate_bits + 4;
    let rnd = (8 << intermediate_bits) + prep_bias::<P>() * 16;
    let mut t = 0;
    for _ in 0..h {
        for x in 0..w {
            dst[d + x] = iclip_pixel(
                (tmp1[t + x] as i32 * weight + tmp2[t + x] as i32 * (16 - weight) + rnd) >> sh,
                bitdepth_max,
            );
        }

        t += w;
        d += dst_stride;
    }
}

/// `mask_c`
pub(crate) fn mask<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    tmp1: &[i16],
    tmp2: &[i16],
    w: usize,
    h: usize,
    mask: &[u8],
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let sh = intermediate_bits + 6;
    let rnd = (32 << intermediate_bits) + prep_bias::<P>() * 64;
    let mut t = 0;
    for _ in 0..h {
        for x in 0..w {
            let m = mask[t + x] as i32;
            dst[d + x] = iclip_pixel(
                (tmp1[t + x] as i32 * m + tmp2[t + x] as i32 * (64 - m) + rnd) >> sh,
                bitdepth_max,
            );
        }

        t += w;
        d += dst_stride;
    }
}

/// `blend_px(a, b, m)`
#[inline(always)]
fn blend_px<P: Pixel>(a: P, b: P, m: i32) -> P {
    P::from_i32(((a.to_i32() * (64 - m) + b.to_i32() * m) + 32) >> 6)
}

/// `blend_c`
pub(crate) fn blend<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    tmp: &[P],
    w: usize,
    h: usize,
    mask: &[u8],
) {
    let mut t = 0;
    for _ in 0..h {
        for x in 0..w {
            dst[d + x] = blend_px(dst[d + x], tmp[t + x], mask[t + x] as i32);
        }
        d += dst_stride;
        t += w;
    }
}

/// `blend_v_c`
pub(crate) fn blend_v<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    tmp: &[P],
    w: usize,
    h: usize,
) {
    let mask = &OBMC_MASKS[w..];
    let mut t = 0;
    for _ in 0..h {
        for x in 0..(w * 3) >> 2 {
            dst[d + x] = blend_px(dst[d + x], tmp[t + x], mask[x] as i32);
        }
        d += dst_stride;
        t += w;
    }
}

/// `blend_h_c`
pub(crate) fn blend_h<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    tmp: &[P],
    w: usize,
    h: usize,
) {
    let mask = &OBMC_MASKS[h..];
    let h = (h * 3) >> 2;
    let mut t = 0;
    for y in 0..h {
        let m = mask[y] as i32;
        for x in 0..w {
            dst[d + x] = blend_px(dst[d + x], tmp[t + x], m);
        }
        d += dst_stride;
        t += w;
    }
}

/// `w_mask_c` (`dsp->mc.w_mask[0, 1, 2]` are `ss_hor`, `ss_ver` (0, 0),
/// (1, 0) and (1, 1))
pub(crate) fn w_mask<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    tmp1: &[i16],
    tmp2: &[i16],
    w: usize,
    h: usize,
    mask: &mut [u8],
    sign: i32,
    ss_hor: bool,
    ss_ver: bool,
    bitdepth_max: i32,
) {
    // store mask at 2x2 resolution, i.e. store 2x1 sum for even rows,
    // and then load this intermediate to calculate final value for odd rows
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let bitdepth = bitdepth_from_max(bitdepth_max);
    let sh = intermediate_bits + 6;
    let rnd = (32 << intermediate_bits) + prep_bias::<P>() * 64;
    let mask_sh = bitdepth + intermediate_bits - 4;
    let mask_rnd = 1 << (mask_sh - 5);
    let mut t = 0;
    let mut mk = 0;
    let mut hh = h;
    while hh > 0 {
        let mut x = 0;
        while x < w {
            let (a, b) = (tmp1[t + x] as i32, tmp2[t + x] as i32);
            let m = imin(38 + (((a - b).abs() + mask_rnd) >> mask_sh), 64);
            dst[d + x] = iclip_pixel((a * m + b * (64 - m) + rnd) >> sh, bitdepth_max);

            if ss_hor {
                x += 1;

                let (a, b) = (tmp1[t + x] as i32, tmp2[t + x] as i32);
                let n = imin(38 + (((a - b).abs() + mask_rnd) >> mask_sh), 64);
                dst[d + x] = iclip_pixel((a * n + b * (64 - n) + rnd) >> sh, bitdepth_max);

                let mi = mk + (x >> 1);
                if (hh & 1) != 0 && ss_ver {
                    mask[mi] = ((m + n + mask[mi] as i32 + 2 - sign) >> 2) as u8;
                } else if ss_ver {
                    mask[mi] = (m + n) as u8;
                } else {
                    mask[mi] = ((m + n + 1 - sign) >> 1) as u8;
                }
            } else {
                mask[mk + x] = m as u8;
            }
            x += 1;
        }

        t += w;
        d += dst_stride;
        if !ss_ver || (hh & 1) != 0 {
            mk += w >> ss_hor as usize;
        }
        hh -= 1;
    }
}

/// `FILTER_WARP_RND(src, x, F, stride, sh)` on pixels.
#[inline(always)]
fn filter_warp_rnd<P: Pixel>(src: &[P], i: usize, f: &[i8; 8], stride: usize, sh: i32) -> i32 {
    rnd(filter_8tap(src, i, f, stride), sh)
}

/// `warp_affine_8x8_c`
pub(crate) fn warp8x8<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    abcd: &[i16; 4],
    mut mx: i32,
    mut my: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let mut mid = [0i16; 15 * 8];
    let mut m = 0;

    s -= 3 * src_stride;
    for _ in 0..15 {
        let mut tmx = mx;
        for x in 0..8 {
            let filter = &MC_WARP_FILTER[(64 + ((tmx + 512) >> 10)) as usize];

            mid[m + x] = filter_warp_rnd(src, s + x, filter, 1, 7 - intermediate_bits) as i16;
            tmx += abcd[0] as i32;
        }
        s += src_stride;
        m += 8;
        mx += abcd[1] as i32;
    }

    m = 3 * 8;
    for _ in 0..8 {
        let mut tmy = my;
        for x in 0..8 {
            let filter = &MC_WARP_FILTER[(64 + ((tmy + 512) >> 10)) as usize];

            dst[d + x] = iclip_pixel(
                rnd(
                    filter_8tap_mid(&mid, m + x, filter, 8),
                    7 + intermediate_bits,
                ),
                bitdepth_max,
            );
            tmy += abcd[2] as i32;
        }
        m += 8;
        d += dst_stride;
        my += abcd[3] as i32;
    }
}

/// `warp_affine_8x8t_c`
pub(crate) fn warp8x8t<P: Pixel>(
    tmp: &mut [i16],
    mut t: usize,
    tmp_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    abcd: &[i16; 4],
    mut mx: i32,
    mut my: i32,
    bitdepth_max: i32,
) {
    let intermediate_bits = get_intermediate_bits::<P>(bitdepth_max);
    let mut mid = [0i16; 15 * 8];
    let mut m = 0;

    s -= 3 * src_stride;
    for _ in 0..15 {
        let mut tmx = mx;
        for x in 0..8 {
            let filter = &MC_WARP_FILTER[(64 + ((tmx + 512) >> 10)) as usize];

            mid[m + x] = filter_warp_rnd(src, s + x, filter, 1, 7 - intermediate_bits) as i16;
            tmx += abcd[0] as i32;
        }
        s += src_stride;
        m += 8;
        mx += abcd[1] as i32;
    }

    m = 3 * 8;
    for _ in 0..8 {
        let mut tmy = my;
        for x in 0..8 {
            let filter = &MC_WARP_FILTER[(64 + ((tmy + 512) >> 10)) as usize];

            tmp[t + x] =
                (rnd(filter_8tap_mid(&mid, m + x, filter, 8), 7) - prep_bias::<P>()) as i16;
            tmy += abcd[2] as i32;
        }
        m += 8;
        t += tmp_stride;
        my += abcd[3] as i32;
    }
}

/// `emu_edge_c`: `r` is the offset of the reference plane's top-left pixel
/// in `ref_`.
pub(crate) fn emu_edge<P: Pixel>(
    bw: isize,
    bh: isize,
    iw: isize,
    ih: isize,
    x: isize,
    y: isize,
    dst: &mut [P],
    dst_stride: usize,
    ref_: &[P],
    r: usize,
    ref_stride: usize,
) {
    // find offset in reference of visible block to copy
    let mut r = r
        + iclip(y as i32, 0, ih as i32 - 1) as usize * ref_stride
        + iclip(x as i32, 0, iw as i32 - 1) as usize;

    // number of pixels to extend (left, right, top, bottom)
    let left_ext = iclip(-x as i32, 0, bw as i32 - 1) as usize;
    let right_ext = iclip((x + bw - iw) as i32, 0, bw as i32 - 1) as usize;
    debug_assert!(((left_ext + right_ext) as isize) < bw);
    let top_ext = iclip(-y as i32, 0, bh as i32 - 1) as usize;
    let bottom_ext = iclip((y + bh - ih) as i32, 0, bh as i32 - 1) as usize;
    debug_assert!(((top_ext + bottom_ext) as isize) < bh);
    let bw = bw as usize;
    let bh = bh as usize;

    // copy visible portion first
    let mut blk = top_ext * dst_stride;
    let center_w = bw - left_ext - right_ext;
    let center_h = bh - top_ext - bottom_ext;
    for _ in 0..center_h {
        dst[blk + left_ext..blk + left_ext + center_w].copy_from_slice(&ref_[r..r + center_w]);
        // extend left edge for this line
        if left_ext != 0 {
            let v = dst[blk + left_ext];
            dst[blk..blk + left_ext].fill(v);
        }
        // extend right edge for this line
        if right_ext != 0 {
            let v = dst[blk + left_ext + center_w - 1];
            dst[blk + left_ext + center_w..blk + left_ext + center_w + right_ext].fill(v);
        }
        r += ref_stride;
        blk += dst_stride;
    }

    // copy top
    let mut d = 0;
    blk = top_ext * dst_stride;
    for _ in 0..top_ext {
        dst.copy_within(blk..blk + bw, d);
        d += dst_stride;
    }

    // copy bottom
    d += center_h * dst_stride;
    for _ in 0..bottom_ext {
        dst.copy_within(ix(d, -(dst_stride as isize))..d - dst_stride + bw, d);
        d += dst_stride;
    }
}

/// `resize_c`
pub(crate) fn resize<P: Pixel>(
    dst: &mut [P],
    mut d: usize,
    dst_stride: usize,
    src: &[P],
    mut s: usize,
    src_stride: usize,
    dst_w: usize,
    h: usize,
    src_w: i32,
    dx: i32,
    mx0: i32,
    bitdepth_max: i32,
) {
    for _ in 0..h {
        let mut mx = mx0;
        let mut src_x = -1;
        for x in 0..dst_w {
            let f = &RESIZE_FILTER[(mx >> 8) as usize];
            let px = |k: i32| src[s + iclip(src_x + k, 0, src_w - 1) as usize].to_i32();
            dst[d + x] = iclip_pixel(
                (-(f[0] as i32 * px(-3)
                    + f[1] as i32 * px(-2)
                    + f[2] as i32 * px(-1)
                    + f[3] as i32 * px(0)
                    + f[4] as i32 * px(1)
                    + f[5] as i32 * px(2)
                    + f[6] as i32 * px(3)
                    + f[7] as i32 * px(4))
                    + 64)
                    >> 7,
                bitdepth_max,
            );
            mx += dx;
            src_x += mx >> 14;
            mx &= 0x3fff;
        }

        d += dst_stride;
        s += src_stride;
    }
}
