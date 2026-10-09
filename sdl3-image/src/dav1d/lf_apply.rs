// Rust translation of src/lf_apply_tmpl.c and src/lf_apply.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The deblocking filter of a superblock row, and the backup of the
//! loop-filtered rows CDEF and loop restoration read (`dav1d_copy_lpf()`).
//!
//! The picture pointers `p[]` are offsets in the current picture's planes.

use super::bitdepth::Pixel;
use super::headers::*;
use super::internal::Dav1dFrameContext;
use super::intops::imin;
use super::lf_mask::Av1FilterLUT;
use super::loopfilter::loop_filter_sb;
use super::lr_apply::{LR_RESTORE_U, LR_RESTORE_V, LR_RESTORE_Y};
use super::mc;

// The loop filter buffer stores 12 rows of pixels. A superblock block will
// contain at most 2 stripes. Each stripe requires 4 rows pixels (2 above
// and 2 below) the final 4 rows are used to swap the bottom of the last
// stripe with the top of the next super block row.
fn backup_lpf<P: Pixel>(
    f_frame_hdr: &Dav1dFrameHeader,
    resize_step: &[i32; 2],
    resize_start: &[i32; 2],
    bitdepth_max: i32,
    dst_buf: &mut [P],
    dst_stride: usize,
    src_buf: &[P],
    src: isize,
    src_stride: usize,
    ss_ver: i32,
    sb128: i32,
    mut row: i32,
    row_h: i32,
    src_w: i32,
    h: i32,
    ss_hor: i32,
    lr_backup: bool,
) {
    let cdef_backup = !lr_backup as i32;
    let dst_w = if f_frame_hdr.super_res.enabled != 0 {
        (f_frame_hdr.width[1] + ss_hor) >> ss_hor
    } else {
        src_w
    } as usize;
    let src_w = src_w as usize;

    // The first stripe of the frame is shorter by 8 luma pixel rows.
    let mut stripe_h = ((64 << (cdef_backup & sb128)) - 8 * (row == 0) as i32) >> ss_ver;
    let mut src = (src + (stripe_h as isize - 2) * src_stride as isize) as usize;

    let mut dst = 0usize;
    // (one tile thread)
    if row != 0 {
        let top = (4 << sb128) as usize;
        // Copy the top part of the stored loop filtered pixels from the
        // previous sb row needed above the first stripe of this sb row.
        for i in 0..4 {
            dst_buf.copy_within(
                dst_stride * (top + i)..dst_stride * (top + i) + dst_w,
                dst_stride * i,
            );
        }
    }
    dst += 4 * dst_stride;

    if lr_backup && f_frame_hdr.width[0] != f_frame_hdr.width[1] {
        while row + stripe_h <= row_h {
            let n_lines = 4 - (row + stripe_h + 1 == h) as usize;
            mc::resize::<P>(
                dst_buf,
                dst,
                dst_stride,
                src_buf,
                src,
                src_stride,
                dst_w,
                n_lines,
                src_w as i32,
                resize_step[ss_hor as usize],
                resize_start[ss_hor as usize],
                bitdepth_max,
            );
            row += stripe_h; // unmodified stripe_h for the 1st stripe
            stripe_h = 64 >> ss_ver;
            src += stripe_h as usize * src_stride;
            dst += n_lines * dst_stride;
            if n_lines == 3 {
                dst_buf.copy_within(dst - dst_stride..dst - dst_stride + dst_w, dst);
                dst += dst_stride;
            }
        }
    } else {
        while row + stripe_h <= row_h {
            let n_lines = 4 - (row + stripe_h + 1 == h) as usize;
            for i in 0..4 {
                if i == n_lines {
                    dst_buf.copy_within(dst - dst_stride..dst - dst_stride + src_w, dst);
                } else {
                    dst_buf[dst..dst + src_w].copy_from_slice(&src_buf[src..src + src_w]);
                }
                dst += dst_stride;
                src += src_stride;
            }
            row += stripe_h; // unmodified stripe_h for the 1st stripe
            stripe_h = 64 >> ss_ver;
            src += (stripe_h as usize - 4) * src_stride;
        }
    }
}

/// `dav1d_copy_lpf()`
pub(crate) fn copy_lpf<P: Pixel>(f: &mut Dav1dFrameContext, src: [usize; 3], sby: i32) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    let seq_hdr = f.seq_hdr.clone().expect("sequence header");
    let resize = frame_hdr.width[0] != frame_hdr.width[1];
    let offset = 8 * (sby != 0) as isize;
    let cur = f.cur_px.as_ref().expect("picture");
    let src_stride = cur.stride;
    let lr_stride = f.sr_px.as_ref().unwrap_or(cur).stride;
    let planes = P::planes(cur);
    let fpx = P::frame_px_mut(&mut f.px);
    let sb128 = seq_hdr.sb128;

    // TODO Also check block level restore type to reduce copying.
    let restore_planes = f.lf.restore_planes;

    if seq_hdr.cdef != 0 || restore_planes & LR_RESTORE_Y != 0 {
        let h = f.cur.p.h;
        let w = f.bw << 2;
        let row_h = imin((sby + 1) << (6 + sb128), h - 1);
        let y_stripe = (sby << (6 + sb128)) - offset as i32;
        if restore_planes & LR_RESTORE_Y != 0 || !resize {
            backup_lpf::<P>(
                &frame_hdr,
                &f.resize_step,
                &f.resize_start,
                f.bitdepth_max,
                &mut fpx.lr_lpf_line[0],
                lr_stride[0],
                &planes[0],
                src[0] as isize - offset * src_stride[0] as isize,
                src_stride[0],
                0,
                sb128,
                y_stripe,
                row_h,
                w,
                h,
                0,
                true,
            );
        }
    }
    if (seq_hdr.cdef != 0 || restore_planes & (LR_RESTORE_U | LR_RESTORE_V) != 0)
        && f.cur.p.layout != DAV1D_PIXEL_LAYOUT_I400
    {
        let ss_ver = (f.sr_cur.p.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
        let ss_hor = (f.sr_cur.p.p.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
        let h = (f.cur.p.h + ss_ver) >> ss_ver;
        let w = f.bw << (2 - ss_hor);
        let row_h = imin((sby + 1) << ((6 - ss_ver) + sb128), h - 1);
        let offset_uv = offset >> ss_ver;
        let y_stripe = (sby << ((6 - ss_ver) + sb128)) - offset_uv as i32;
        for pl in 1..3 {
            let flag = if pl == 1 { LR_RESTORE_U } else { LR_RESTORE_V };
            if seq_hdr.cdef != 0 || restore_planes & flag != 0 {
                if restore_planes & flag != 0 || !resize {
                    backup_lpf::<P>(
                        &frame_hdr,
                        &f.resize_step,
                        &f.resize_start,
                        f.bitdepth_max,
                        &mut fpx.lr_lpf_line[pl],
                        lr_stride[1],
                        &planes[pl],
                        src[pl] as isize - offset_uv * src_stride[1] as isize,
                        src_stride[1],
                        ss_ver,
                        sb128,
                        y_stripe,
                        row_h,
                        w,
                        h,
                        ss_hor,
                        true,
                    );
                }
            }
        }
    }
}

#[inline]
fn filter_plane_cols_y<P: Pixel>(
    lut: &Av1FilterLUT,
    bitdepth_max: i32,
    have_left: bool,
    lvl: &[u8],
    l: usize,
    b4_stride: usize,
    mask: &[[[u16; 2]; 3]; 32],
    dst: &mut [P],
    d: usize,
    ls: usize,
    w: i32,
    starty4: u32,
    endy4: u32,
) {
    // filter edges between columns (e.g. block1 | block2)
    for x in 0..w as usize {
        if !have_left && x == 0 {
            continue;
        }
        let mut hmask = [0u32; 4];
        if starty4 == 0 {
            hmask[0] = mask[x][0][0] as u32;
            hmask[1] = mask[x][1][0] as u32;
            hmask[2] = mask[x][2][0] as u32;
            if endy4 > 16 {
                hmask[0] |= (mask[x][0][1] as u32) << 16;
                hmask[1] |= (mask[x][1][1] as u32) << 16;
                hmask[2] |= (mask[x][2][1] as u32) << 16;
            }
        } else {
            hmask[0] = mask[x][0][1] as u32;
            hmask[1] = mask[x][1][1] as u32;
            hmask[2] = mask[x][2][1] as u32;
        }
        hmask[3] = 0;
        loop_filter_sb::<P>(
            0,
            0,
            dst,
            d + x * 4,
            ls,
            &hmask,
            lvl,
            l + 4 * x,
            b4_stride,
            lut,
            (endy4 - starty4) as i32,
            bitdepth_max,
        );
    }
}

#[inline]
fn filter_plane_rows_y<P: Pixel>(
    lut: &Av1FilterLUT,
    bitdepth_max: i32,
    have_top: bool,
    lvl: &[u8],
    mut l: usize,
    b4_stride: usize,
    mask: &[[[u16; 2]; 3]; 32],
    dst: &mut [P],
    mut d: usize,
    ls: usize,
    w: i32,
    starty4: u32,
    endy4: u32,
) {
    //                                 block1
    // filter edges between rows (e.g. ------)
    //                                 block2
    for y in starty4..endy4 {
        if have_top || y != 0 {
            let y = y as usize;
            let vmask = [
                mask[y][0][0] as u32 | ((mask[y][0][1] as u32) << 16),
                mask[y][1][0] as u32 | ((mask[y][1][1] as u32) << 16),
                mask[y][2][0] as u32 | ((mask[y][2][1] as u32) << 16),
                0,
            ];
            loop_filter_sb::<P>(
                0,
                1,
                dst,
                d,
                ls,
                &vmask,
                lvl,
                l + 1,
                b4_stride,
                lut,
                w,
                bitdepth_max,
            );
        }
        d += 4 * ls;
        l += 4 * b4_stride;
    }
}

#[inline]
fn filter_plane_cols_uv<P: Pixel>(
    lut: &Av1FilterLUT,
    bitdepth_max: i32,
    have_left: bool,
    lvl: &[u8],
    l: usize,
    b4_stride: usize,
    mask: &[[[u16; 2]; 2]; 32],
    u: &mut [P],
    v: &mut [P],
    d: usize,
    ls: usize,
    w: i32,
    starty4: u32,
    endy4: u32,
    ss_ver: i32,
) {
    // filter edges between columns (e.g. block1 | block2)
    for x in 0..w as usize {
        if !have_left && x == 0 {
            continue;
        }
        let mut hmask = [0u32; 3];
        if starty4 == 0 {
            hmask[0] = mask[x][0][0] as u32;
            hmask[1] = mask[x][1][0] as u32;
            if endy4 > (16 >> ss_ver) {
                hmask[0] |= (mask[x][0][1] as u32) << (16 >> ss_ver);
                hmask[1] |= (mask[x][1][1] as u32) << (16 >> ss_ver);
            }
        } else {
            hmask[0] = mask[x][0][1] as u32;
            hmask[1] = mask[x][1][1] as u32;
        }
        hmask[2] = 0;
        loop_filter_sb::<P>(
            1,
            0,
            u,
            d + x * 4,
            ls,
            &hmask,
            lvl,
            l + 4 * x + 2,
            b4_stride,
            lut,
            (endy4 - starty4) as i32,
            bitdepth_max,
        );
        loop_filter_sb::<P>(
            1,
            0,
            v,
            d + x * 4,
            ls,
            &hmask,
            lvl,
            l + 4 * x + 3,
            b4_stride,
            lut,
            (endy4 - starty4) as i32,
            bitdepth_max,
        );
    }
}

#[inline]
fn filter_plane_rows_uv<P: Pixel>(
    lut: &Av1FilterLUT,
    bitdepth_max: i32,
    have_top: bool,
    lvl: &[u8],
    mut l: usize,
    b4_stride: usize,
    mask: &[[[u16; 2]; 2]; 32],
    u: &mut [P],
    v: &mut [P],
    d: usize,
    ls: usize,
    w: i32,
    starty4: u32,
    endy4: u32,
    ss_hor: i32,
) {
    let mut off_l = 0;

    //                                 block1
    // filter edges between rows (e.g. ------)
    //                                 block2
    for y in starty4..endy4 {
        if have_top || y != 0 {
            let y = y as usize;
            let vmask = [
                mask[y][0][0] as u32 | ((mask[y][0][1] as u32) << (16 >> ss_hor)),
                mask[y][1][0] as u32 | ((mask[y][1][1] as u32) << (16 >> ss_hor)),
                0,
            ];
            loop_filter_sb::<P>(
                1,
                1,
                u,
                d + off_l,
                ls,
                &vmask,
                lvl,
                l + 2,
                b4_stride,
                lut,
                w,
                bitdepth_max,
            );
            loop_filter_sb::<P>(
                1,
                1,
                v,
                d + off_l,
                ls,
                &vmask,
                lvl,
                l + 3,
                b4_stride,
                lut,
                w,
                bitdepth_max,
            );
        }
        off_l += 4 * ls;
        l += 4 * b4_stride;
    }
}

/// `dav1d_loopfilter_sbrow_cols()`: `lflvl` is the index of the row's
/// first mask in `f->lf.mask`.
pub(crate) fn loopfilter_sbrow_cols<P: Pixel>(
    f: &mut Dav1dFrameContext,
    p: [usize; 3],
    lflvl: usize,
    sby: i32,
    start_of_tile_row: i32,
) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    // Don't filter outside the frame
    let is_sb64 = (f.seq_hdr().sb128 == 0) as i32;
    let starty4 = ((sby & is_sb64) << 4) as u32;
    let sbsz = 32 >> is_sb64;
    let sbl2 = 5 - is_sb64;
    let halign = ((f.bh + 31) & !31) as usize;
    let layout = f.cur.p.layout;
    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let vmask = 16 >> ss_ver;
    let hmask = 16 >> ss_hor;
    let vmax = 1u32 << vmask;
    let hmax = 1u32 << hmask;
    let endy4 = starty4 + imin(f.h4 - sby * sbsz, sbsz) as u32;
    let uv_endy4 = (endy4 + ss_ver as u32) >> ss_ver;

    // fix lpf strength at tile col boundaries
    let mut lpf_y = (sby << sbl2) as usize;
    let mut lpf_uv = (sby << (sbl2 - ss_ver)) as usize;
    let mut tile_col = 1;
    loop {
        let mut x = frame_hdr.tiling.col_start_sb[tile_col] as i32;
        if (x << sbl2) >= f.bw {
            break;
        }
        let bx4 = if x & is_sb64 != 0 { 16 } else { 0 };
        let cbx4 = bx4 >> ss_hor;
        x >>= is_sb64;

        let lpf_right_y = &f.lf.tx_lpf_right_edge[0];
        let y_hmask = &mut f.lf.mask[lflvl + x as usize].filter_y[0][bx4];
        let mut mask = 1u32 << starty4;
        for y in starty4..endy4 {
            let sidx = (mask >= 0x10000) as usize;
            let smask = (mask >> (sidx << 4)) as u16;
            let idx = 2 * (y_hmask[2][sidx] & smask != 0) as usize
                + (y_hmask[1][sidx] & smask != 0) as usize;
            y_hmask[2][sidx] &= !smask;
            y_hmask[1][sidx] &= !smask;
            y_hmask[0][sidx] &= !smask;
            y_hmask[imin(
                idx as i32,
                lpf_right_y[lpf_y + (y - starty4) as usize] as i32,
            ) as usize][sidx] |= smask;
            mask <<= 1;
        }

        if layout != DAV1D_PIXEL_LAYOUT_I400 {
            let lpf_right_uv = &f.lf.tx_lpf_right_edge[1];
            let uv_hmask = &mut f.lf.mask[lflvl + x as usize].filter_uv[0][cbx4];
            let mut uv_mask = 1u32 << (starty4 >> ss_ver);
            for y in (starty4 >> ss_ver)..uv_endy4 {
                let sidx = (uv_mask >= vmax) as usize;
                let smask = (uv_mask >> (sidx << (4 - ss_ver))) as u16;
                let idx = (uv_hmask[1][sidx] & smask != 0) as usize;
                uv_hmask[1][sidx] &= !smask;
                uv_hmask[0][sidx] &= !smask;
                uv_hmask[imin(
                    idx as i32,
                    lpf_right_uv[lpf_uv + (y - (starty4 >> ss_ver)) as usize] as i32,
                ) as usize][sidx] |= smask;
                uv_mask <<= 1;
            }
        }
        lpf_y += halign;
        lpf_uv += halign >> ss_ver;
        tile_col += 1;
    }

    // fix lpf strength at tile row boundaries
    if start_of_tile_row != 0 {
        let mut a = (f.sb128w * (start_of_tile_row - 1)) as usize;
        for x in 0..f.sb128w as usize {
            let ab = &f.a[a];
            let y_vmask = &mut f.lf.mask[lflvl + x].filter_y[1][starty4 as usize];
            let w = imin(32, f.w4 - ((x as i32) << 5)) as u32;
            let mut mask = 1u32;
            for i in 0..w as usize {
                let sidx = (mask >= 0x10000) as usize;
                let smask = (mask >> (sidx << 4)) as u16;
                let idx = 2 * (y_vmask[2][sidx] & smask != 0) as usize
                    + (y_vmask[1][sidx] & smask != 0) as usize;
                y_vmask[2][sidx] &= !smask;
                y_vmask[1][sidx] &= !smask;
                y_vmask[0][sidx] &= !smask;
                y_vmask[imin(idx as i32, ab.tx_lpf_y[i] as i32) as usize][sidx] |= smask;
                mask <<= 1;
            }

            if layout != DAV1D_PIXEL_LAYOUT_I400 {
                let cw = (w + ss_hor as u32) >> ss_hor;
                let uv_vmask = &mut f.lf.mask[lflvl + x].filter_uv[1][(starty4 >> ss_ver) as usize];
                let mut uv_mask = 1u32;
                for i in 0..cw as usize {
                    let sidx = (uv_mask >= hmax) as usize;
                    let smask = (uv_mask >> (sidx << (4 - ss_hor))) as u16;
                    let idx = (uv_vmask[1][sidx] & smask != 0) as usize;
                    uv_vmask[1][sidx] &= !smask;
                    uv_vmask[0][sidx] &= !smask;
                    uv_vmask[imin(idx as i32, ab.tx_lpf_uv[i] as i32) as usize][sidx] |= smask;
                    uv_mask <<= 1;
                }
            }
            a += 1;
        }
    }

    let b4_stride = f.b4_stride;
    let bdmax = f.bitdepth_max;
    let stride = f.cur_px.as_ref().expect("picture").stride;
    let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
    let lvl = f.lf.level.as_flattened();
    let lut = &f.lf.lim_lut;
    let mut level_ptr = 4 * b4_stride * (sby * sbsz) as usize;
    let mut ptr = p[0];
    for x in 0..f.sb128w as usize {
        filter_plane_cols_y::<P>(
            lut,
            bdmax,
            x != 0,
            lvl,
            level_ptr,
            b4_stride,
            &f.lf.mask[lflvl + x].filter_y[0],
            &mut planes[0],
            ptr,
            stride[0],
            imin(32, f.w4 - x as i32 * 32),
            starty4,
            endy4,
        );
        ptr += 128;
        level_ptr += 4 * 32;
    }

    if frame_hdr.loopfilter.level_u == 0 && frame_hdr.loopfilter.level_v == 0 {
        return;
    }

    let mut level_ptr = 4 * b4_stride * ((sby * sbsz) >> ss_ver) as usize;
    let mut uv_off = 0;
    let [_, u, v] = planes;
    for x in 0..f.sb128w as usize {
        filter_plane_cols_uv::<P>(
            lut,
            bdmax,
            x != 0,
            lvl,
            level_ptr,
            b4_stride,
            &f.lf.mask[lflvl + x].filter_uv[0],
            u,
            v,
            p[1] + uv_off,
            stride[1],
            (imin(32, f.w4 - x as i32 * 32) + ss_hor) >> ss_hor,
            starty4 >> ss_ver,
            uv_endy4,
            ss_ver,
        );
        uv_off += 128 >> ss_hor;
        level_ptr += 4 * (32 >> ss_hor);
    }
}

/// `dav1d_loopfilter_sbrow_rows()`
pub(crate) fn loopfilter_sbrow_rows<P: Pixel>(
    f: &mut Dav1dFrameContext,
    p: [usize; 3],
    lflvl: usize,
    sby: i32,
) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    // Don't filter outside the frame
    let have_top = sby > 0;
    let is_sb64 = (f.seq_hdr().sb128 == 0) as i32;
    let starty4 = ((sby & is_sb64) << 4) as u32;
    let sbsz = 32 >> is_sb64;
    let layout = f.cur.p.layout;
    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let endy4 = starty4 + imin(f.h4 - sby * sbsz, sbsz) as u32;
    let uv_endy4 = (endy4 + ss_ver as u32) >> ss_ver;

    let b4_stride = f.b4_stride;
    let bdmax = f.bitdepth_max;
    let stride = f.cur_px.as_ref().expect("picture").stride;
    let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
    let lvl = f.lf.level.as_flattened();
    let lut = &f.lf.lim_lut;
    let mut level_ptr = 4 * b4_stride * (sby * sbsz) as usize;
    let mut ptr = p[0];
    for x in 0..f.sb128w as usize {
        filter_plane_rows_y::<P>(
            lut,
            bdmax,
            have_top,
            lvl,
            level_ptr,
            b4_stride,
            &f.lf.mask[lflvl + x].filter_y[1],
            &mut planes[0],
            ptr,
            stride[0],
            imin(32, f.w4 - x as i32 * 32),
            starty4,
            endy4,
        );
        ptr += 128;
        level_ptr += 4 * 32;
    }

    if frame_hdr.loopfilter.level_u == 0 && frame_hdr.loopfilter.level_v == 0 {
        return;
    }

    let mut level_ptr = 4 * b4_stride * ((sby * sbsz) >> ss_ver) as usize;
    let mut uv_off = 0;
    let [_, u, v] = planes;
    for x in 0..f.sb128w as usize {
        filter_plane_rows_uv::<P>(
            lut,
            bdmax,
            have_top,
            lvl,
            level_ptr,
            b4_stride,
            &f.lf.mask[lflvl + x].filter_uv[1],
            u,
            v,
            p[1] + uv_off,
            stride[1],
            (imin(32, f.w4 - x as i32 * 32) + ss_hor) >> ss_hor,
            starty4 >> ss_ver,
            uv_endy4,
            ss_hor,
        );
        uv_off += 128 >> ss_hor;
        level_ptr += 4 * (32 >> ss_hor);
    }
}
