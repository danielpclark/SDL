// Rust translation of src/cdef_apply_tmpl.c and src/cdef_apply.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CDEF of a superblock row (one tile thread: the pre-filter rows above
//! each 8x8 block row are kept in `f->lf.cdef_line`).

use super::bitdepth::Pixel;
use super::cdef::{
    cdef_filter_block, cdef_find_dir, CDEF_HAVE_BOTTOM, CDEF_HAVE_LEFT, CDEF_HAVE_RIGHT,
    CDEF_HAVE_TOP,
};
use super::headers::*;
use super::internal::{Dav1dFrameContext, Dav1dTaskContext};
use super::intops::{imin, ulog2};

// enum Backup2x8Flags
const BACKUP_2X8_Y: u32 = 1 << 0;
const BACKUP_2X8_UV: u32 = 1 << 1;

fn backup2lines<P: Pixel>(
    dst: &mut [Vec<P>; 3],
    src: &[Vec<P>; 3],
    s: [usize; 3],
    stride: [usize; 2],
    layout: i32,
) {
    let y_stride = stride[0];
    dst[0][..2 * y_stride].copy_from_slice(&src[0][s[0] + 6 * y_stride..s[0] + 8 * y_stride]);

    if layout != DAV1D_PIXEL_LAYOUT_I400 {
        let uv_stride = stride[1];
        let uv_off = if layout == DAV1D_PIXEL_LAYOUT_I420 {
            2
        } else {
            6
        };
        for pl in 1..3 {
            dst[pl][..2 * uv_stride].copy_from_slice(
                &src[pl][s[pl] + uv_off * uv_stride..s[pl] + (uv_off + 2) * uv_stride],
            );
        }
    }
}

fn backup2x8<P: Pixel>(
    dst: &mut [[[P; 2]; 8]; 3],
    src: &[Vec<P>; 3],
    s: [usize; 3],
    src_stride: [usize; 2],
    mut x_off: usize,
    layout: i32,
    flag: u32,
) {
    let mut y_off = 0;
    if flag & BACKUP_2X8_Y != 0 {
        for y in 0..8 {
            let o = s[0] + y_off + x_off - 2;
            dst[0][y].copy_from_slice(&src[0][o..o + 2]);
            y_off += src_stride[0];
        }
    }

    if layout == DAV1D_PIXEL_LAYOUT_I400 || flag & BACKUP_2X8_UV == 0 {
        return;
    }

    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as usize;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as usize;

    x_off >>= ss_hor;
    y_off = 0;
    for y in 0..(8 >> ss_ver) {
        let o1 = s[1] + y_off + x_off - 2;
        let o2 = s[2] + y_off + x_off - 2;
        dst[1][y].copy_from_slice(&src[1][o1..o1 + 2]);
        dst[2][y].copy_from_slice(&src[2][o2..o2 + 2]);
        y_off += src_stride[1];
    }
}

fn adjust_strength(strength: i32, var: u32) -> i32 {
    if var == 0 {
        return 0;
    }
    let i = if var >> 6 != 0 {
        imin(ulog2(var >> 6), 12)
    } else {
        0
    };
    (strength * (4 + i) + 8) >> 4
}

/// `dav1d_cdef_brow()`: `p` are the plane offsets of the first row,
/// `lflvl` the index of the row's first mask in `f->lf.mask`.
pub(crate) fn cdef_brow<P: Pixel>(
    f: &mut Dav1dFrameContext,
    tc: &mut Dav1dTaskContext,
    p: [usize; 3],
    lflvl: usize,
    by_start: i32,
    by_end: i32,
    _sbrow_start: bool,
    _sby: i32,
) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    let bitdepth_min_8 = if P::BPC8 { 0 } else { f.cur.p.bpc - 8 };
    let mut edges = CDEF_HAVE_BOTTOM | if by_start > 0 { CDEF_HAVE_TOP } else { 0 };
    let mut ptrs = p;
    let sbsz = 16;
    let sb64w = f.sb128w << 1;
    let damping = frame_hdr.cdef.damping + bitdepth_min_8;
    let layout = f.cur.p.layout;
    let uv_idx = DAV1D_PIXEL_LAYOUT_I444 - layout;
    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as usize;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as usize;
    const UV_DIRS: [[u8; 8]; 2] = [[0, 1, 2, 3, 4, 5, 6, 7], [7, 0, 2, 4, 5, 6, 6, 6]];
    let uv_dir = &UV_DIRS[(layout == DAV1D_PIXEL_LAYOUT_I422) as usize];
    // (one tile thread: have_tt is 0)
    let bdmax = f.bitdepth_max;
    let stride = f.cur_px.as_ref().expect("picture").stride;
    let y_stride = stride[0];
    let uv_stride = stride[1];
    let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
    let fpx = P::frame_px_mut(&mut f.px);

    let mut lr_bak = [[[[P::default(); 2]; 8]; 3]; 2 /* idx */];

    let mut bit = 0usize;
    let mut by = by_start;
    while by < by_end {
        let tf = tc.top_pre_cdef_toggle as usize;
        let by_idx = ((by & 30) >> 1) as usize;
        if by + 2 >= f.bh {
            edges &= !CDEF_HAVE_BOTTOM;
        }

        if edges & CDEF_HAVE_BOTTOM != 0 {
            // backup pre-filter data for next iteration
            backup2lines::<P>(&mut fpx.cdef_line[1 - tf], planes, ptrs, stride, layout);
        }

        let mut iptrs = ptrs;
        edges &= !CDEF_HAVE_LEFT;
        edges |= CDEF_HAVE_RIGHT;
        let mut prev_flag = 0u32;
        let mut last_skip = true;
        for sbx in 0..sb64w {
            let sb128x = (sbx >> 1) as usize;
            let sb64_idx = (((by & sbsz) >> 3) + (sbx & 1)) as usize;
            let cdef_idx = f.lf.mask[lflvl + sb128x].cdef_idx[sb64_idx] as i32;
            if cdef_idx == -1
                || (frame_hdr.cdef.y_strength[cdef_idx as usize] == 0
                    && frame_hdr.cdef.uv_strength[cdef_idx as usize] == 0)
            {
                last_skip = true;
            } else {
                // Create a complete 32-bit mask for the sb row ahead of time.
                let noskip_row = &f.lf.mask[lflvl + sb128x].noskip_mask[by_idx];
                let noskip_mask = (noskip_row[1] as u32) << 16 | noskip_row[0] as u32;

                let y_lvl = frame_hdr.cdef.y_strength[cdef_idx as usize];
                let uv_lvl = frame_hdr.cdef.uv_strength[cdef_idx as usize];
                let flag = (y_lvl != 0) as u32 + (((uv_lvl != 0) as u32) << 1);

                let y_pri_lvl = (y_lvl >> 2) << bitdepth_min_8;
                let mut y_sec_lvl = y_lvl & 3;
                y_sec_lvl += (y_sec_lvl == 3) as i32;
                y_sec_lvl <<= bitdepth_min_8;

                let uv_pri_lvl = (uv_lvl >> 2) << bitdepth_min_8;
                let mut uv_sec_lvl = uv_lvl & 3;
                uv_sec_lvl += (uv_sec_lvl == 3) as i32;
                uv_sec_lvl <<= bitdepth_min_8;

                let mut bptrs = iptrs;
                let mut bx = sbx * sbsz;
                while bx < imin((sbx + 1) * sbsz, f.bw) {
                    if bx + 2 >= f.bw {
                        edges &= !CDEF_HAVE_RIGHT;
                    }

                    // check if this 8x8 block had any coded coefficients; if not,
                    // go to the next block
                    let bx_mask = 3u32 << (bx & 30);
                    if noskip_mask & bx_mask == 0 {
                        last_skip = true;
                    } else {
                        let do_left = if last_skip {
                            flag
                        } else {
                            (prev_flag ^ flag) & flag
                        };
                        prev_flag = flag;
                        if do_left != 0 && edges & CDEF_HAVE_LEFT != 0 {
                            // we didn't backup the prefilter data because it wasn't
                            // there, so do it here instead
                            backup2x8::<P>(
                                &mut lr_bak[bit],
                                planes,
                                bptrs,
                                stride,
                                0,
                                layout,
                                do_left,
                            );
                        }
                        if edges & CDEF_HAVE_RIGHT != 0 {
                            // backup pre-filter data for next iteration
                            backup2x8::<P>(
                                &mut lr_bak[1 - bit],
                                planes,
                                bptrs,
                                stride,
                                8,
                                layout,
                                flag,
                            );
                        }

                        let mut dir = 0;
                        let mut variance = 0u32;
                        if y_pri_lvl != 0 || uv_pri_lvl != 0 {
                            let (d, v) = cdef_find_dir::<P>(&planes[0], bptrs[0], y_stride, bdmax);
                            dir = d;
                            variance = v;
                        }

                        // st_y:
                        let top = &fpx.cdef_line[tf][0];
                        let tp = bx as usize * 4;
                        if y_pri_lvl != 0 {
                            let adj_y_pri_lvl = adjust_strength(y_pri_lvl, variance);
                            if adj_y_pri_lvl != 0 || y_sec_lvl != 0 {
                                cdef_filter_block::<P>(
                                    &mut planes[0],
                                    bptrs[0],
                                    y_stride,
                                    &lr_bak[bit][0],
                                    top,
                                    tp,
                                    adj_y_pri_lvl,
                                    y_sec_lvl,
                                    dir,
                                    damping,
                                    8,
                                    8,
                                    edges,
                                    bdmax,
                                );
                            }
                        } else if y_sec_lvl != 0 {
                            cdef_filter_block::<P>(
                                &mut planes[0],
                                bptrs[0],
                                y_stride,
                                &lr_bak[bit][0],
                                top,
                                tp,
                                0,
                                y_sec_lvl,
                                0,
                                damping,
                                8,
                                8,
                                edges,
                                bdmax,
                            );
                        }

                        if uv_lvl != 0 {
                            debug_assert!(layout != DAV1D_PIXEL_LAYOUT_I400);

                            let uvdir = if uv_pri_lvl != 0 {
                                uv_dir[dir as usize] as i32
                            } else {
                                0
                            };
                            // (cdef.fb[uv_idx]: 8x8, 4x8 or 4x4)
                            let (bw, bh) = match uv_idx {
                                0 => (8, 8),
                                1 => (4, 8),
                                _ => (4, 4),
                            };
                            for pl in 1..=2 {
                                // st_uv:
                                let top = &fpx.cdef_line[tf][pl];
                                let tp = (bx as usize * 4) >> ss_hor;
                                cdef_filter_block::<P>(
                                    &mut planes[pl],
                                    bptrs[pl],
                                    uv_stride,
                                    &lr_bak[bit][pl],
                                    top,
                                    tp,
                                    uv_pri_lvl,
                                    uv_sec_lvl,
                                    uvdir,
                                    damping - 1,
                                    bw,
                                    bh,
                                    edges,
                                    bdmax,
                                );
                            }
                        }

                        // skip_uv:
                        bit ^= 1;
                        last_skip = false;
                    }

                    // next_b:
                    bptrs[0] += 8;
                    bptrs[1] += 8 >> ss_hor;
                    bptrs[2] += 8 >> ss_hor;
                    bx += 2;
                    edges |= CDEF_HAVE_LEFT;
                }
            }

            // next_sb:
            iptrs[0] += sbsz as usize * 4;
            iptrs[1] += (sbsz as usize * 4) >> ss_hor;
            iptrs[2] += (sbsz as usize * 4) >> ss_hor;
            edges |= CDEF_HAVE_LEFT;
        }

        ptrs[0] += 8 * y_stride;
        ptrs[1] += (8 * uv_stride) >> ss_ver;
        ptrs[2] += (8 * uv_stride) >> ss_ver;
        tc.top_pre_cdef_toggle ^= 1;
        by += 2;
        edges |= CDEF_HAVE_TOP;
    }
}
