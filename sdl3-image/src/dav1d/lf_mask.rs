// Rust translation of src/lf_mask.c and src/lf_mask.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The loop filter edge masks and levels of each block, and the
//! restoration unit parameters.

use super::headers::{
    Dav1dFrameHeader, Dav1dLoopfilterModeRefDeltas, DAV1D_PIXEL_LAYOUT_I420,
    DAV1D_PIXEL_LAYOUT_I444,
};
use super::intops::{iclip, imax, imin};
use super::levels::*;
use super::tables::{BLOCK_DIMENSIONS, TXFM_DIMENSIONS};

/// Translation of `Av1FilterLUT`.
#[derive(Clone, Copy)]
pub(crate) struct Av1FilterLUT {
    pub(crate) e: [u8; 64],
    pub(crate) i: [u8; 64],
    pub(crate) sharp: [u64; 2],
}

impl Default for Av1FilterLUT {
    fn default() -> Self {
        Av1FilterLUT {
            e: [0; 64],
            i: [0; 64],
            sharp: [0; 2],
        }
    }
}

/// Translation of `Av1RestorationUnit`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Av1RestorationUnit {
    pub(crate) type_: u8,
    pub(crate) filter_h: [i8; 3],
    pub(crate) filter_v: [i8; 3],
    pub(crate) sgr_idx: u8,
    pub(crate) sgr_weights: [i8; 2],
}

/// Translation of `Av1Filter`: each struct describes one 128x128 area (1
/// or 4 SBs), pre-superres-scaling.
#[derive(Clone, Copy)]
pub(crate) struct Av1Filter {
    // each bit is 1 col
    pub(crate) filter_y: [[[[u16; 2]; 3]; 32]; 2],
    pub(crate) filter_uv: [[[[u16; 2]; 2]; 32]; 2],
    pub(crate) cdef_idx: [i8; 4],           // -1 means "unset"
    pub(crate) noskip_mask: [[u16; 2]; 16], // for 8x8 blocks, but stored on a 4x8 basis
}

impl Default for Av1Filter {
    fn default() -> Self {
        Av1Filter {
            filter_y: [[[[0; 2]; 3]; 32]; 2],
            filter_uv: [[[[0; 2]; 2]; 32]; 2],
            cdef_idx: [0; 4],
            noskip_mask: [[0; 2]; 16],
        }
    }
}

/// Translation of `Av1Restoration`: each struct describes one 128x128
/// area (1 or 4 SBs), post-superres-scaling.
#[derive(Clone, Copy, Default)]
pub(crate) struct Av1Restoration {
    pub(crate) lr: [[Av1RestorationUnit; 4]; 3],
}

/// The `txa[2 /* edge */][2 /* txsz, step */][32 /* y */][32 /* x */]`
/// scratch of `mask_edges_inter()`, with the (y, x) origin of
/// `decomp_tx()`'s sub-array pointers.
type Txa = [[[[u8; 32]; 32]; 2]; 2];

#[allow(clippy::too_many_arguments)]
fn decomp_tx(
    txa: &mut Txa,
    y0: usize,
    x0: usize,
    from: u8,
    depth: i32,
    y_off: i32,
    x_off: i32,
    tx_masks: &[u16; 2],
) {
    let t_dim = &TXFM_DIMENSIONS[from as usize];
    let is_split = if from == TX_4X4 || depth > 1 {
        false
    } else {
        ((tx_masks[depth as usize] >> (y_off * 4 + x_off)) & 1) != 0
    };

    if is_split {
        let sub = t_dim.sub;
        let htw4 = (t_dim.w >> 1) as usize;
        let hth4 = (t_dim.h >> 1) as usize;

        decomp_tx(txa, y0, x0, sub, depth + 1, y_off * 2, x_off * 2, tx_masks);
        if t_dim.w >= t_dim.h {
            decomp_tx(
                txa,
                y0,
                x0 + htw4,
                sub,
                depth + 1,
                y_off * 2,
                x_off * 2 + 1,
                tx_masks,
            );
        }
        if t_dim.h >= t_dim.w {
            decomp_tx(
                txa,
                y0 + hth4,
                x0,
                sub,
                depth + 1,
                y_off * 2 + 1,
                x_off * 2,
                tx_masks,
            );
            if t_dim.w >= t_dim.h {
                decomp_tx(
                    txa,
                    y0 + hth4,
                    x0 + htw4,
                    sub,
                    depth + 1,
                    y_off * 2 + 1,
                    x_off * 2 + 1,
                    tx_masks,
                );
            }
        }
    } else {
        let lw = imin(2, t_dim.lw as i32) as u8;
        let lh = imin(2, t_dim.lh as i32) as u8;
        let w = t_dim.w as usize;

        for y in 0..t_dim.h as usize {
            txa[0][0][y0 + y][x0..x0 + w].fill(lw);
            txa[1][0][y0 + y][x0..x0 + w].fill(lh);
            txa[0][1][y0 + y][x0] = t_dim.w;
        }
        txa[1][1][y0][x0..x0 + w].fill(t_dim.h);
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn mask_edges_inter(
    masks: &mut [[[[u16; 2]; 3]; 32]; 2],
    by4: usize,
    bx4: usize,
    w4: usize,
    h4: usize,
    skip: bool,
    max_tx: u8,
    tx_masks: &[u16; 2],
    a: &mut [u8],
    l: &mut [u8],
) {
    let t_dim = &TXFM_DIMENSIONS[max_tx as usize];

    let mut txa: Txa = [[[[0; 32]; 32]; 2]; 2];
    let mut y_off = 0;
    let mut y = 0;
    while y < h4 {
        let mut x_off = 0;
        let mut x = 0;
        while x < w4 {
            decomp_tx(&mut txa, y, x, max_tx, 0, y_off, x_off, tx_masks);
            x += t_dim.w as usize;
            x_off += 1;
        }
        y += t_dim.h as usize;
        y_off += 1;
    }

    // left block edge
    let mut mask: u32 = 1u32 << by4;
    for y in 0..h4 {
        let sidx = (mask >= 0x10000) as usize;
        let smask = mask >> (sidx << 4);
        masks[0][bx4][imin(txa[0][0][y][0] as i32, l[y] as i32) as usize][sidx] |= smask as u16;
        mask <<= 1;
    }

    // top block edge
    mask = 1u32 << bx4;
    for x in 0..w4 {
        let sidx = (mask >= 0x10000) as usize;
        let smask = mask >> (sidx << 4);
        masks[1][by4][imin(txa[1][0][0][x] as i32, a[x] as i32) as usize][sidx] |= smask as u16;
        mask <<= 1;
    }

    if !skip {
        // inner (tx) left|right edges
        mask = 1u32 << by4;
        for y in 0..h4 {
            let sidx = (mask >= 0x10000) as usize;
            let smask = mask >> (sidx << 4);
            let mut ltx = txa[0][0][y][0] as i32;
            let mut step = txa[0][1][y][0] as usize;
            let mut x = step;
            while x < w4 {
                let rtx = txa[0][0][y][x] as i32;
                masks[0][bx4 + x][imin(rtx, ltx) as usize][sidx] |= smask as u16;
                ltx = rtx;
                step = txa[0][1][y][x] as usize;
                x += step;
            }
            mask <<= 1;
        }

        //            top
        // inner (tx) --- edges
        //           bottom
        mask = 1u32 << bx4;
        for x in 0..w4 {
            let sidx = (mask >= 0x10000) as usize;
            let smask = mask >> (sidx << 4);
            let mut ttx = txa[1][0][0][x] as i32;
            let mut step = txa[1][1][0][x] as usize;
            let mut y = step;
            while y < h4 {
                let btx = txa[1][0][y][x] as i32;
                masks[1][by4 + y][imin(ttx, btx) as usize][sidx] |= smask as u16;
                ttx = btx;
                step = txa[1][1][y][x] as usize;
                y += step;
            }
            mask <<= 1;
        }
    }

    for y in 0..h4 {
        l[y] = txa[0][0][y][w4 - 1];
    }
    a[..w4].copy_from_slice(&txa[1][0][h4 - 1][..w4]);
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn mask_edges_intra(
    masks: &mut [[[[u16; 2]; 3]; 32]; 2],
    by4: usize,
    bx4: usize,
    w4: usize,
    h4: usize,
    tx: u8,
    a: &mut [u8],
    l: &mut [u8],
) {
    let t_dim = &TXFM_DIMENSIONS[tx as usize];
    let twl4 = t_dim.lw as i32;
    let thl4 = t_dim.lh as i32;
    let twl4c = imin(2, twl4);
    let thl4c = imin(2, thl4);

    // left block edge
    let mut mask: u32 = 1u32 << by4;
    for y in 0..h4 {
        let sidx = (mask >= 0x10000) as usize;
        let smask = mask >> (sidx << 4);
        masks[0][bx4][imin(twl4c, l[y] as i32) as usize][sidx] |= smask as u16;
        mask <<= 1;
    }

    // top block edge
    mask = 1u32 << bx4;
    for x in 0..w4 {
        let sidx = (mask >= 0x10000) as usize;
        let smask = mask >> (sidx << 4);
        masks[1][by4][imin(thl4c, a[x] as i32) as usize][sidx] |= smask as u16;
        mask <<= 1;
    }

    // inner (tx) left|right edges
    let hstep = t_dim.w as usize;
    let t = 1u32 << by4;
    let inner = ((t as u64) << h4).wrapping_sub(t as u64) as u32;
    let inner1 = inner & 0xffff;
    let inner2 = inner >> 16;
    let mut x = hstep;
    while x < w4 {
        if inner1 != 0 {
            masks[0][bx4 + x][twl4c as usize][0] |= inner1 as u16;
        }
        if inner2 != 0 {
            masks[0][bx4 + x][twl4c as usize][1] |= inner2 as u16;
        }
        x += hstep;
    }

    //            top
    // inner (tx) --- edges
    //           bottom
    let vstep = t_dim.h as usize;
    let t = 1u32 << bx4;
    let inner = ((t as u64) << w4).wrapping_sub(t as u64) as u32;
    let inner1 = inner & 0xffff;
    let inner2 = inner >> 16;
    let mut y = vstep;
    while y < h4 {
        if inner1 != 0 {
            masks[1][by4 + y][thl4c as usize][0] |= inner1 as u16;
        }
        if inner2 != 0 {
            masks[1][by4 + y][thl4c as usize][1] |= inner2 as u16;
        }
        y += vstep;
    }

    a[..w4].fill(thl4c as u8);
    l[..h4].fill(twl4c as u8);
}

#[allow(clippy::too_many_arguments)]
fn mask_edges_chroma(
    masks: &mut [[[[u16; 2]; 2]; 32]; 2],
    cby4: usize,
    cbx4: usize,
    cw4: usize,
    ch4: usize,
    skip_inter: bool,
    tx: u8,
    a: &mut [u8],
    l: &mut [u8],
    ss_hor: i32,
    ss_ver: i32,
) {
    let t_dim = &TXFM_DIMENSIONS[tx as usize];
    let twl4 = t_dim.lw as i32;
    let thl4 = t_dim.lh as i32;
    let twl4c = (twl4 != 0) as i32;
    let thl4c = (thl4 != 0) as i32;
    let vbits = 4 - ss_ver;
    let hbits = 4 - ss_hor;
    let vmask = 16 >> ss_ver;
    let hmask = 16 >> ss_hor;
    let vmax = 1u32 << vmask;
    let hmax = 1u32 << hmask;

    // left block edge
    let mut mask: u32 = 1u32 << cby4;
    for y in 0..ch4 {
        let sidx = (mask >= vmax) as usize;
        let smask = mask >> (sidx << vbits);
        masks[0][cbx4][imin(twl4c, l[y] as i32) as usize][sidx] |= smask as u16;
        mask <<= 1;
    }

    // top block edge
    mask = 1u32 << cbx4;
    for x in 0..cw4 {
        let sidx = (mask >= hmax) as usize;
        let smask = mask >> (sidx << hbits);
        masks[1][cby4][imin(thl4c, a[x] as i32) as usize][sidx] |= smask as u16;
        mask <<= 1;
    }

    if !skip_inter {
        // inner (tx) left|right edges
        let hstep = t_dim.w as usize;
        let t = 1u32 << cby4;
        let inner = ((t as u64) << ch4).wrapping_sub(t as u64) as u32;
        let inner1 = inner & ((1 << vmask) - 1);
        let inner2 = inner >> vmask;
        let mut x = hstep;
        while x < cw4 {
            if inner1 != 0 {
                masks[0][cbx4 + x][twl4c as usize][0] |= inner1 as u16;
            }
            if inner2 != 0 {
                masks[0][cbx4 + x][twl4c as usize][1] |= inner2 as u16;
            }
            x += hstep;
        }

        //            top
        // inner (tx) --- edges
        //           bottom
        let vstep = t_dim.h as usize;
        let t = 1u32 << cbx4;
        let inner = ((t as u64) << cw4).wrapping_sub(t as u64) as u32;
        let inner1 = inner & ((1 << hmask) - 1);
        let inner2 = inner >> hmask;
        let mut y = vstep;
        while y < ch4 {
            if inner1 != 0 {
                masks[1][cby4 + y][thl4c as usize][0] |= inner1 as u16;
            }
            if inner2 != 0 {
                masks[1][cby4 + y][thl4c as usize][1] |= inner2 as u16;
            }
            y += vstep;
        }
    }

    a[..cw4].fill(thl4c as u8);
    l[..ch4].fill(twl4c as u8);
}

/// The `level_cache` writes of the mask creators: `filter_level` is
/// `lflvl[seg_id][dir][ref][is_gmv]`, `(r, m)` the `[ref][is_gmv]` pair
/// upstream's pointer cast selects.
#[allow(clippy::too_many_arguments)]
fn fill_levels(
    level_cache: &mut [[u8; 4]],
    b4_stride: usize,
    filter_level: &[[[u8; 2]; 8]; 4],
    r: usize,
    m: usize,
    bx: usize,
    by: usize,
    bw4: usize,
    bh4: usize,
    planes: [usize; 2],
) {
    let mut ptr = by * b4_stride + bx;
    for _y in 0..bh4 {
        for x in 0..bw4 {
            level_cache[ptr + x][planes[0]] = filter_level[planes[0]][r][m];
            level_cache[ptr + x][planes[1]] = filter_level[planes[1]][r][m];
        }
        ptr += b4_stride;
    }
}

/// Translation of `dav1d_create_lf_mask_intra()`. `level` is
/// `ts->lflvl[b->seg_id]`; the chroma context arrays are `None` without
/// chroma.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_lf_mask_intra(
    lflvl: &mut Av1Filter,
    level_cache: &mut [[u8; 4]],
    b4_stride: usize,
    filter_level: &[[[u8; 2]; 8]; 4],
    bx: i32,
    by: i32,
    iw: i32,
    ih: i32,
    bs: u8,
    ytx: u8,
    uvtx: u8,
    layout: i32,
    ay: &mut [u8],
    ly: &mut [u8],
    auv_luv: Option<(&mut [u8], &mut [u8])>,
) {
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];
    let bw4 = imin(iw - bx, b_dim[0] as i32);
    let bh4 = imin(ih - by, b_dim[1] as i32);
    let bx4 = (bx & 31) as usize;
    let by4 = (by & 31) as usize;

    if bw4 != 0 && bh4 != 0 {
        fill_levels(
            level_cache,
            b4_stride,
            filter_level,
            0,
            0,
            bx as usize,
            by as usize,
            bw4 as usize,
            bh4 as usize,
            [0, 1],
        );

        mask_edges_intra(
            &mut lflvl.filter_y,
            by4,
            bx4,
            bw4 as usize,
            bh4 as usize,
            ytx,
            ay,
            ly,
        );
    }

    let Some((auv, luv)) = auv_luv else {
        return;
    };

    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let cbw4 = imin(
        ((iw + ss_hor) >> ss_hor) - (bx >> ss_hor),
        (b_dim[0] as i32 + ss_hor) >> ss_hor,
    );
    let cbh4 = imin(
        ((ih + ss_ver) >> ss_ver) - (by >> ss_ver),
        (b_dim[1] as i32 + ss_ver) >> ss_ver,
    );

    if cbw4 == 0 || cbh4 == 0 {
        return;
    }

    let cbx4 = bx4 >> ss_hor;
    let cby4 = by4 >> ss_ver;

    fill_levels(
        level_cache,
        b4_stride,
        filter_level,
        0,
        0,
        (bx >> ss_hor) as usize,
        (by >> ss_ver) as usize,
        cbw4 as usize,
        cbh4 as usize,
        [2, 3],
    );

    mask_edges_chroma(
        &mut lflvl.filter_uv,
        cby4,
        cbx4,
        cbw4 as usize,
        cbh4 as usize,
        false,
        uvtx,
        auv,
        luv,
        ss_hor,
        ss_ver,
    );
}

/// Translation of `dav1d_create_lf_mask_inter()`. `level` is
/// `ts->lflvl[b->seg_id]`, `(r, m)` the `[b->ref[0] + 1][!is_globalmv]`
/// of upstream's level pointer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_lf_mask_inter(
    lflvl: &mut Av1Filter,
    level_cache: &mut [[u8; 4]],
    b4_stride: usize,
    filter_level: &[[[u8; 2]; 8]; 4],
    r: usize,
    m: usize,
    bx: i32,
    by: i32,
    iw: i32,
    ih: i32,
    skip: bool,
    bs: u8,
    max_ytx: u8,
    tx_masks: &[u16; 2],
    uvtx: u8,
    layout: i32,
    ay: &mut [u8],
    ly: &mut [u8],
    auv_luv: Option<(&mut [u8], &mut [u8])>,
) {
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];
    let bw4 = imin(iw - bx, b_dim[0] as i32);
    let bh4 = imin(ih - by, b_dim[1] as i32);
    let bx4 = (bx & 31) as usize;
    let by4 = (by & 31) as usize;

    if bw4 != 0 && bh4 != 0 {
        fill_levels(
            level_cache,
            b4_stride,
            filter_level,
            r,
            m,
            bx as usize,
            by as usize,
            bw4 as usize,
            bh4 as usize,
            [0, 1],
        );

        mask_edges_inter(
            &mut lflvl.filter_y,
            by4,
            bx4,
            bw4 as usize,
            bh4 as usize,
            skip,
            max_ytx,
            tx_masks,
            ay,
            ly,
        );
    }

    let Some((auv, luv)) = auv_luv else {
        return;
    };

    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let cbw4 = imin(
        ((iw + ss_hor) >> ss_hor) - (bx >> ss_hor),
        (b_dim[0] as i32 + ss_hor) >> ss_hor,
    );
    let cbh4 = imin(
        ((ih + ss_ver) >> ss_ver) - (by >> ss_ver),
        (b_dim[1] as i32 + ss_ver) >> ss_ver,
    );

    if cbw4 == 0 || cbh4 == 0 {
        return;
    }

    let cbx4 = bx4 >> ss_hor;
    let cby4 = by4 >> ss_ver;

    fill_levels(
        level_cache,
        b4_stride,
        filter_level,
        r,
        m,
        (bx >> ss_hor) as usize,
        (by >> ss_ver) as usize,
        cbw4 as usize,
        cbh4 as usize,
        [2, 3],
    );

    mask_edges_chroma(
        &mut lflvl.filter_uv,
        cby4,
        cbx4,
        cbw4 as usize,
        cbh4 as usize,
        skip,
        uvtx,
        auv,
        luv,
        ss_hor,
        ss_ver,
    );
}

/// Translation of `dav1d_calc_eih()`.
pub(crate) fn calc_eih(lim_lut: &mut Av1FilterLUT, filter_sharpness: i32) {
    // set E/I/H values from loopfilter level
    let sharp = filter_sharpness;
    for level in 0..64 {
        let mut limit = level;

        if sharp > 0 {
            limit >>= (sharp + 3) >> 2;
            limit = imin(limit, 9 - sharp);
        }
        limit = imax(limit, 1);

        lim_lut.i[level as usize] = limit as u8;
        lim_lut.e[level as usize] = (2 * (level + 2) + limit) as u8;
    }
    lim_lut.sharp[0] = ((sharp + 3) >> 2) as u64;
    lim_lut.sharp[1] = if sharp != 0 { (9 - sharp) as u64 } else { 0xff };
}

fn calc_lf_value(
    lflvl_values: &mut [[u8; 2]; 8],
    base_lvl: i32,
    lf_delta: i32,
    seg_delta: i32,
    mr_delta: Option<&Dav1dLoopfilterModeRefDeltas>,
) {
    let base = iclip(iclip(base_lvl + lf_delta, 0, 63) + seg_delta, 0, 63);

    match mr_delta {
        None => {
            *lflvl_values = [[base as u8; 2]; 8];
        }
        Some(mr_delta) => {
            let sh = (base >= 32) as i32;
            let v = iclip(base + mr_delta.ref_delta[0] * (1 << sh), 0, 63) as u8;
            lflvl_values[0][0] = v;
            lflvl_values[0][1] = v;
            for r in 1..8 {
                for m in 0..2 {
                    let delta = mr_delta.mode_delta[m] + mr_delta.ref_delta[r];
                    lflvl_values[r][m] = iclip(base + delta * (1 << sh), 0, 63) as u8;
                }
            }
        }
    }
}

#[inline]
fn calc_lf_value_chroma(
    lflvl_values: &mut [[u8; 2]; 8],
    base_lvl: i32,
    lf_delta: i32,
    seg_delta: i32,
    mr_delta: Option<&Dav1dLoopfilterModeRefDeltas>,
) {
    if base_lvl == 0 {
        *lflvl_values = [[0; 2]; 8];
    } else {
        calc_lf_value(lflvl_values, base_lvl, lf_delta, seg_delta, mr_delta);
    }
}

/// Translation of `dav1d_calc_lf_values()`.
pub(crate) fn calc_lf_values(
    lflvl_values: &mut [[[[u8; 2]; 8]; 4]; 8],
    hdr: &Dav1dFrameHeader,
    lf_delta: &[i8; 4],
) {
    let n_seg = if hdr.segmentation.enabled != 0 { 8 } else { 1 };

    if hdr.loopfilter.level_y[0] == 0 && hdr.loopfilter.level_y[1] == 0 {
        for v in &mut lflvl_values[..n_seg] {
            *v = [[[0; 2]; 8]; 4];
        }
        return;
    }

    let mr_deltas = if hdr.loopfilter.mode_ref_delta_enabled != 0 {
        Some(&hdr.loopfilter.mode_ref_deltas)
    } else {
        None
    };
    let lf_delta = |i: usize| lf_delta[i] as i32;
    for s in 0..n_seg {
        let segd = if hdr.segmentation.enabled != 0 {
            Some(&hdr.segmentation.seg_data.d[s])
        } else {
            None
        };

        calc_lf_value(
            &mut lflvl_values[s][0],
            hdr.loopfilter.level_y[0],
            lf_delta(0),
            segd.map_or(0, |d| d.delta_lf_y_v),
            mr_deltas,
        );
        calc_lf_value(
            &mut lflvl_values[s][1],
            hdr.loopfilter.level_y[1],
            lf_delta(if hdr.delta.lf.multi != 0 { 1 } else { 0 }),
            segd.map_or(0, |d| d.delta_lf_y_h),
            mr_deltas,
        );
        calc_lf_value_chroma(
            &mut lflvl_values[s][2],
            hdr.loopfilter.level_u,
            lf_delta(if hdr.delta.lf.multi != 0 { 2 } else { 0 }),
            segd.map_or(0, |d| d.delta_lf_u),
            mr_deltas,
        );
        calc_lf_value_chroma(
            &mut lflvl_values[s][3],
            hdr.loopfilter.level_v,
            lf_delta(if hdr.delta.lf.multi != 0 { 3 } else { 0 }),
            segd.map_or(0, |d| d.delta_lf_v),
            mr_deltas,
        );
    }
}
