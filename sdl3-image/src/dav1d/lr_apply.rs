// Rust translation of src/lr_apply_tmpl.c and src/lr_apply.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loop restoration of a superblock row (on the upscaled picture).

use super::bitdepth::Pixel;
use super::headers::*;
use super::internal::Dav1dFrameContext;
use super::intops::{imax, imin};
use super::lf_mask::Av1RestorationUnit;
use super::looprestoration::{
    lr_filter, LooprestorationParams, LrFilter, LR_HAVE_BOTTOM, LR_HAVE_LEFT, LR_HAVE_RIGHT,
    LR_HAVE_TOP,
};
use super::tables::SGR_PARAMS;

// enum LrRestorePlanes
pub(crate) const LR_RESTORE_Y: i32 = 1 << 0;
pub(crate) const LR_RESTORE_U: i32 = 1 << 1;
pub(crate) const LR_RESTORE_V: i32 = 1 << 2;

/// The frame values the stripes need.
struct LrFrame {
    layout: i32,
    sb128: i32,
    sbh: i32,
    bitdepth_max: i32,
}

fn lr_stripe<P: Pixel>(
    fr: &LrFrame,
    plane_px: &mut [P],
    stride: usize,
    lpf_buf: &[P],
    mut p: usize,
    left: &[[P; 4]],
    x: i32,
    mut y: i32,
    plane: usize,
    unit_w: i32,
    row_h: i32,
    lr: &Av1RestorationUnit,
    mut edges: u32,
) {
    let chroma = plane != 0;
    let ss_ver = (chroma && fr.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let sby = (y + if y != 0 { 8 << ss_ver } else { 0 }) >> (6 - ss_ver + fr.sb128);
    // (one tile thread: have_tt is 0)
    let mut lpf = x as usize;
    let mut left_i = 0;

    // The first stripe of the frame is shorter by 8 luma pixel rows.
    let mut stripe_h = imin((64 - 8 * (y == 0) as i32) >> ss_ver, row_h - y);

    let lr_fn;
    let mut params = LooprestorationParams::default();
    if lr.type_ == DAV1D_RESTORATION_WIENER {
        let filter = &mut params.filter;
        filter[0][0] = lr.filter_h[0] as i16;
        filter[0][6] = lr.filter_h[0] as i16;
        filter[0][1] = lr.filter_h[1] as i16;
        filter[0][5] = lr.filter_h[1] as i16;
        filter[0][2] = lr.filter_h[2] as i16;
        filter[0][4] = lr.filter_h[2] as i16;
        filter[0][3] = -(filter[0][0] + filter[0][1] + filter[0][2]) * 2;
        if !P::BPC8 {
            /* For 8-bit SIMD it's beneficial to handle the +128 separately
             * in order to avoid overflows. */
            filter[0][3] += 128;
        }

        filter[1][0] = lr.filter_v[0] as i16;
        filter[1][6] = lr.filter_v[0] as i16;
        filter[1][1] = lr.filter_v[1] as i16;
        filter[1][5] = lr.filter_v[1] as i16;
        filter[1][2] = lr.filter_v[2] as i16;
        filter[1][4] = lr.filter_v[2] as i16;
        filter[1][3] = 128 - (filter[1][0] + filter[1][1] + filter[1][2]) * 2;

        // (dsp->lr.wiener[!(filter[0][0] | filter[1][0])]: both are wiener_c)
        lr_fn = LrFilter::Wiener;
    } else {
        debug_assert!(lr.type_ == DAV1D_RESTORATION_SGRPROJ);
        let sgr_params = &SGR_PARAMS[lr.sgr_idx as usize];
        params.sgr_s0 = sgr_params[0] as u32;
        params.sgr_s1 = sgr_params[1] as u32;
        params.sgr_w0 = lr.sgr_weights[0] as i16;
        params.sgr_w1 = 128 - (lr.sgr_weights[0] as i16 + lr.sgr_weights[1] as i16);

        lr_fn = match (sgr_params[0] != 0) as i32 + (sgr_params[1] != 0) as i32 * 2 - 1 {
            0 => LrFilter::Sgr5x5,
            1 => LrFilter::Sgr3x3,
            _ => LrFilter::SgrMix,
        };
    }

    while y + stripe_h <= row_h {
        // Change the HAVE_BOTTOM bit in edges to (sby + 1 != f->sbh || y + stripe_h != row_h)
        let bottom = sby + 1 != fr.sbh || y + stripe_h != row_h;
        edges = (edges & !LR_HAVE_BOTTOM) | if bottom { LR_HAVE_BOTTOM } else { 0 };
        lr_filter::<P>(
            lr_fn,
            plane_px,
            p,
            stride,
            &left[left_i..],
            lpf_buf,
            lpf,
            unit_w as usize,
            stripe_h as usize,
            &params,
            edges,
            fr.bitdepth_max,
        );

        left_i += stripe_h as usize;
        y += stripe_h;
        p += stripe_h as usize * stride;
        edges |= LR_HAVE_TOP;
        stripe_h = imin(64 >> ss_ver, row_h - y);
        if stripe_h == 0 {
            break;
        }
        lpf += 4 * stride;
    }
}

fn backup4xu<P: Pixel>(dst: &mut [[P; 4]], src: &[P], mut s: usize, src_stride: usize, u: i32) {
    for d in dst.iter_mut().take(imax(u, 0) as usize) {
        d.copy_from_slice(&src[s..s + 4]);
        s += src_stride;
    }
}

fn lr_sbrow_plane<P: Pixel>(
    f: &mut Dav1dFrameContext,
    p0: usize,
    y: i32,
    w: i32,
    h: i32,
    row_h: i32,
    plane: usize,
) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    let chroma = plane != 0;
    let layout = f.sr_cur.p.p.layout;
    let ss_ver = (chroma && layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (chroma && layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let fr = LrFrame {
        layout,
        sb128: f.seq_hdr().sb128,
        sbh: f.sbh,
        bitdepth_max: f.bitdepth_max,
    };
    let pic = match f.sr_px.as_mut() {
        Some(p) => p,
        None => f.cur_px.as_mut().expect("picture"),
    };
    let p_stride = pic.stride[chroma as usize];
    let planes = P::planes_mut(pic);
    let plane_px = &mut planes[plane];
    let lpf_buf = &P::frame_px(&f.px).lr_lpf_line[plane];

    let unit_size_log2 = frame_hdr.restoration.unit_size[chroma as usize];
    let unit_size = 1 << unit_size_log2;
    let half_unit_size = unit_size >> 1;
    let max_unit_size = unit_size + half_unit_size;

    // Y coordinate of the sbrow (y is 8 luma pixel rows above row_y)
    let row_y = y + ((8 >> ss_ver) * (y != 0) as i32);

    // FIXME This is an ugly hack to lookup the proper AV1Filter unit for
    // chroma planes. Question: For Multithreaded decoding, is it better
    // to store the chroma LR information with collocated Luma information?
    // In other words. For a chroma restoration unit locate at 128,128 and
    // with a 4:2:0 chroma subsampling, do we store the filter information at
    // the AV1Filter unit located at (128,128) or (256,256)
    // TODO Support chroma subsampling.
    let shift_hor = 7 - ss_hor;

    /* maximum sbrow height is 128 + 8 rows offset */
    let mut pre_lr_border = [[[P::default(); 4]; 128 + 8]; 2];
    let mut lr = [Av1RestorationUnit::default(); 2];

    let mut edges = (if y > 0 { LR_HAVE_TOP } else { 0 }) | LR_HAVE_RIGHT;

    let mut aligned_unit_pos = row_y & !(unit_size - 1);
    if aligned_unit_pos != 0 && aligned_unit_pos + half_unit_size > h {
        aligned_unit_pos -= unit_size;
    }
    aligned_unit_pos <<= ss_ver;
    let sb_idx = ((aligned_unit_pos >> 7) * f.sr_sb128w) as usize;
    let unit_idx = (((aligned_unit_pos >> 6) & 1) << 1) as usize;
    lr[0] = f.lf.lr_mask[sb_idx].lr[plane][unit_idx];
    let mut restore = lr[0].type_ != DAV1D_RESTORATION_NONE;
    let mut x = 0;
    let mut bit = 0usize;
    let mut p = p0;
    while x + max_unit_size <= w {
        let next_x = x + unit_size;
        let next_u_idx = unit_idx + ((next_x >> (shift_hor - 1)) & 1) as usize;
        lr[1 - bit] = f.lf.lr_mask[sb_idx + (next_x >> shift_hor) as usize].lr[plane][next_u_idx];
        let restore_next = lr[1 - bit].type_ != DAV1D_RESTORATION_NONE;
        if restore_next {
            backup4xu::<P>(
                &mut pre_lr_border[bit],
                plane_px,
                p + unit_size as usize - 4,
                p_stride,
                row_h - y,
            );
        }
        if restore {
            lr_stripe::<P>(
                &fr,
                plane_px,
                p_stride,
                lpf_buf,
                p,
                &pre_lr_border[1 - bit],
                x,
                y,
                plane,
                unit_size,
                row_h,
                &lr[bit],
                edges,
            );
        }
        x = next_x;
        restore = restore_next;
        p += unit_size as usize;
        edges |= LR_HAVE_LEFT;
        bit ^= 1;
    }
    if restore {
        edges &= !LR_HAVE_RIGHT;
        let unit_w = w - x;
        lr_stripe::<P>(
            &fr,
            plane_px,
            p_stride,
            lpf_buf,
            p,
            &pre_lr_border[1 - bit],
            x,
            y,
            plane,
            unit_w,
            row_h,
            &lr[bit],
            edges,
        );
    }
}

/// `dav1d_lr_sbrow()`: `dst` are the plane offsets of the superblock row
/// in the upscaled picture.
pub(crate) fn lr_sbrow<P: Pixel>(f: &mut Dav1dFrameContext, dst: [usize; 3], sby: i32) {
    let offset_y = 8 * (sby != 0) as i32;
    let dst_stride = f
        .sr_px
        .as_ref()
        .or(f.cur_px.as_ref())
        .expect("picture")
        .stride;
    let restore_planes = f.lf.restore_planes;
    let not_last = (sby + 1 < f.sbh) as i32;
    let sb128 = f.seq_hdr().sb128;

    if restore_planes & LR_RESTORE_Y != 0 {
        let h = f.sr_cur.p.p.h;
        let w = f.sr_cur.p.p.w;
        let next_row_y = (sby + 1) << (6 + sb128);
        let row_h = imin(next_row_y - 8 * not_last, h);
        let y_stripe = (sby << (6 + sb128)) - offset_y;
        lr_sbrow_plane::<P>(
            f,
            dst[0] - offset_y as usize * dst_stride[0],
            y_stripe,
            w,
            h,
            row_h,
            0,
        );
    }
    if restore_planes & (LR_RESTORE_U | LR_RESTORE_V) != 0 {
        let ss_ver = (f.sr_cur.p.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
        let ss_hor = (f.sr_cur.p.p.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
        let h = (f.sr_cur.p.p.h + ss_ver) >> ss_ver;
        let w = (f.sr_cur.p.p.w + ss_hor) >> ss_hor;
        let next_row_y = (sby + 1) << ((6 - ss_ver) + sb128);
        let row_h = imin(next_row_y - (8 >> ss_ver) * not_last, h);
        let offset_uv = offset_y >> ss_ver;
        let y_stripe = (sby << ((6 - ss_ver) + sb128)) - offset_uv;
        if restore_planes & LR_RESTORE_U != 0 {
            lr_sbrow_plane::<P>(
                f,
                dst[1] - offset_uv as usize * dst_stride[1],
                y_stripe,
                w,
                h,
                row_h,
                1,
            );
        }

        if restore_planes & LR_RESTORE_V != 0 {
            lr_sbrow_plane::<P>(
                f,
                dst[2] - offset_uv as usize * dst_stride[1],
                y_stripe,
                w,
                h,
                row_h,
                2,
            );
        }
    }
}
