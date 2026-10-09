// Rust translation of src/itx_tmpl.c and src/itx.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins), the plain-C functions.
// Copyright © 2018-2019, VideoLAN and dav1d authors
// Copyright © 2018-2019, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The two-dimensional inverse transforms, added to the prediction.
//!
//! `dav1d_itx_dsp_init()`'s `itxfm_add[tx_size][tx_type]` function table
//! is [`itxfm_add`], which dispatches to [`inv_txfm_add`] with the 1-D
//! transforms, the shift and the DC-only shortcut of each combination
//! (the `inv_txfm_fn*()` macros).

use super::bitdepth::{iclip_pixel, Pixel};
use super::intops::{iclip, imin};
use super::itx_1d::*;
use super::levels::*;

/// Translation of `itx_1d_fn`.
type Itx1dFn = fn(&mut [i32], usize, usize, i32, i32);

/// Translation of `inv_txfm_add_c()`: `dst[dst_off..]` is the block,
/// `stride` in pixels.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn inv_txfm_add<P: Pixel>(
    dst: &mut [P],
    mut dst_off: usize,
    stride: usize,
    coeff: &mut [i32],
    eob: i32,
    w: usize,
    h: usize,
    shift: i32,
    first_1d_fn: Itx1dFn,
    second_1d_fn: Itx1dFn,
    has_dconly: i32,
    bitdepth_max: i32,
) {
    debug_assert!((4..=64).contains(&w));
    debug_assert!((4..=64).contains(&h));
    debug_assert!(eob >= 0);

    let is_rect2 = w * 2 == h || h * 2 == w;
    let rnd = (1 << shift) >> 1;

    if eob < has_dconly {
        let mut dc = coeff[0];
        coeff[0] = 0;
        if is_rect2 {
            dc = (dc * 181 + 128) >> 8;
        }
        dc = (dc * 181 + 128) >> 8;
        dc = (dc + rnd) >> shift;
        dc = (dc * 181 + 128 + 2048) >> 12;
        for _y in 0..h {
            for x in 0..w {
                let d = &mut dst[dst_off + x];
                *d = iclip_pixel(d.to_i32() + dc, bitdepth_max);
            }
            dst_off += stride;
        }
        return;
    }

    let sh = imin(h as i32, 32) as usize;
    let sw = imin(w as i32, 32) as usize;
    let (row_clip_min, col_clip_min) = if P::BPC8 {
        (i16::MIN as i32, i16::MIN as i32)
    } else {
        (
            (!(bitdepth_max as u32) << 7) as i32,
            (!(bitdepth_max as u32) << 5) as i32,
        )
    };
    let row_clip_max = !row_clip_min;
    let col_clip_max = !col_clip_min;

    let mut tmp = [0i32; 64 * 64];
    let mut c = 0;
    for y in 0..sh {
        if is_rect2 {
            for x in 0..sw {
                tmp[c + x] = (coeff[y + x * sh] * 181 + 128) >> 8;
            }
        } else {
            for x in 0..sw {
                tmp[c + x] = coeff[y + x * sh];
            }
        }
        first_1d_fn(&mut tmp, c, 1, row_clip_min, row_clip_max);
        c += w;
    }

    coeff[..sw * sh].fill(0);
    for t in &mut tmp[..w * sh] {
        *t = iclip((*t + rnd) >> shift, col_clip_min, col_clip_max);
    }

    for x in 0..w {
        second_1d_fn(&mut tmp, x, w, col_clip_min, col_clip_max);
    }

    let mut c = 0;
    for _y in 0..h {
        for x in 0..w {
            let d = &mut dst[dst_off + x];
            *d = iclip_pixel(d.to_i32() + ((tmp[c] + 8) >> 4), bitdepth_max);
            c += 1;
        }
        dst_off += stride;
    }
}

/// Translation of `inv_txfm_add_wht_wht_4x4_c()`.
fn inv_txfm_add_wht_wht_4x4<P: Pixel>(
    dst: &mut [P],
    mut dst_off: usize,
    stride: usize,
    coeff: &mut [i32],
    bitdepth_max: i32,
) {
    let mut tmp = [0i32; 4 * 4];
    let mut c = 0;
    for y in 0..4 {
        for x in 0..4 {
            tmp[c + x] = coeff[y + x * 4] >> 2;
        }
        inv_wht4_1d(&mut tmp, c, 1);
        c += 4;
    }
    coeff[..4 * 4].fill(0);

    for x in 0..4 {
        inv_wht4_1d(&mut tmp, x, 4);
    }

    let mut c = 0;
    for _y in 0..4 {
        for x in 0..4 {
            let d = &mut dst[dst_off + x];
            *d = iclip_pixel(d.to_i32() + tmp[c], bitdepth_max);
            c += 1;
        }
        dst_off += stride;
    }
}

/// The 1-D transforms of each size: (dct, adst, flipadst, identity);
/// `None` where `dav1d_inv_*_1d_c()` doesn't exist.
fn fns_for(n: usize) -> (Itx1dFn, Option<Itx1dFn>, Option<Itx1dFn>, Option<Itx1dFn>) {
    match n {
        4 => (
            inv_dct4_1d,
            Some(inv_adst4_1d),
            Some(inv_flipadst4_1d),
            Some(inv_identity4_1d),
        ),
        8 => (
            inv_dct8_1d,
            Some(inv_adst8_1d),
            Some(inv_flipadst8_1d),
            Some(inv_identity8_1d),
        ),
        16 => (
            inv_dct16_1d,
            Some(inv_adst16_1d),
            Some(inv_flipadst16_1d),
            Some(inv_identity16_1d),
        ),
        32 => (inv_dct32_1d, None, None, Some(inv_identity32_1d)),
        _ => (inv_dct64_1d, None, None, None),
    }
}

/// Translation of `c->itxfm_add[tx][txtp](dst, stride, coeff, eob)` as
/// `dav1d_itx_dsp_init()` assigns it (`inv_txfm_add_<type1>_<type2>_<w>x<h>_c`,
/// type1 being the horizontal (first, row) transform). Combinations
/// upstream leaves unassigned can't be selected by the bitstream parser.
#[allow(clippy::too_many_arguments)]
pub(crate) fn itxfm_add<P: Pixel>(
    tx: u8,
    txtp: u8,
    dst: &mut [P],
    dst_off: usize,
    stride: usize,
    coeff: &mut [i32],
    eob: i32,
    bitdepth_max: i32,
) {
    if txtp == WHT_WHT {
        debug_assert!(tx == TX_4X4);
        inv_txfm_add_wht_wht_4x4(dst, dst_off, stride, coeff, bitdepth_max);
        return;
    }
    // inv_txfm_fn84( 4,  4, 0) ... inv_txfm_fn64(64, 64, 2)
    let (w, h, shift) = match tx {
        TX_4X4 => (4, 4, 0),
        RTX_4X8 => (4, 8, 0),
        RTX_4X16 => (4, 16, 1),
        RTX_8X4 => (8, 4, 0),
        TX_8X8 => (8, 8, 1),
        RTX_8X16 => (8, 16, 1),
        RTX_8X32 => (8, 32, 2),
        RTX_16X4 => (16, 4, 1),
        RTX_16X8 => (16, 8, 1),
        TX_16X16 => (16, 16, 2),
        RTX_16X32 => (16, 32, 1),
        RTX_16X64 => (16, 64, 2),
        RTX_32X8 => (32, 8, 2),
        RTX_32X16 => (32, 16, 1),
        TX_32X32 => (32, 32, 2),
        RTX_32X64 => (32, 64, 1),
        RTX_64X16 => (64, 16, 2),
        RTX_64X32 => (64, 32, 1),
        _ => (64, 64, 2), // TX_64X64
    };
    let (hdct, hadst, hflipadst, hidentity) = fns_for(w);
    let (vdct, vadst, vflipadst, videntity) = fns_for(h);
    // (first = horizontal = type1, second = vertical = type2, has_dconly)
    let (first, second, has_dconly) = match txtp {
        DCT_DCT => (Some(hdct), Some(vdct), 1),
        IDTX => (hidentity, videntity, 0),
        // DCT_ADST: DCT in vertical, ADST in horizontal: adst_dct
        DCT_ADST => (hadst, Some(vdct), 0),
        ADST_DCT => (Some(hdct), vadst, 0),
        ADST_ADST => (hadst, vadst, 0),
        ADST_FLIPADST => (hflipadst, vadst, 0),
        FLIPADST_ADST => (hadst, vflipadst, 0),
        DCT_FLIPADST => (hflipadst, Some(vdct), 0),
        FLIPADST_DCT => (Some(hdct), vflipadst, 0),
        FLIPADST_FLIPADST => (hflipadst, vflipadst, 0),
        H_DCT => (Some(hdct), videntity, 0),
        V_DCT => (hidentity, Some(vdct), 0),
        H_FLIPADST => (hflipadst, videntity, 0),
        V_FLIPADST => (hidentity, vflipadst, 0),
        H_ADST => (hadst, videntity, 0),
        _ => (hidentity, vadst, 0), // V_ADST
    };
    if let (Some(first), Some(second)) = (first, second) {
        inv_txfm_add(
            dst,
            dst_off,
            stride,
            coeff,
            eob,
            w,
            h,
            shift,
            first,
            second,
            has_dconly,
            bitdepth_max,
        );
    }
}
