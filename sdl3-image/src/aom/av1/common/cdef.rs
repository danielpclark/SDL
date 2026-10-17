// Rust translation of the parts of av1/common/cdef.c, av1/common/cdef.h,
// av1/common/cdef_block.c and av1/common/cdef_block.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the constrained directional enhancement filter applied to the
// reconstructed frame (single threaded, C versions of the DSP functions).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CDEF (constrained directional enhancement filter).
//!
//! libaom keeps the line, column and source buffers in `CdefInfo` and only
//! reallocates them when the frame size changes; their contents never carry
//! over from one frame to the next (every value read is written first in the
//! same frame), so they are allocated for each filtered frame here.
//!
//! With several workers libaom runs `av1_cdef_frame_mt()`, which filters the
//! same 64x64 units with the same inputs (each row saves the unfiltered lines
//! it needs before any neighbour is filtered); the output is the same as the
//! single-threaded `av1_cdef_frame()` translated here (the AV1 decoding
//! process defines it).

use super::alloccommon::try_vec;
use super::av1_common_int::{av1_num_planes, Av1Common, CommonModeInfoParams};
use super::blockd::{get_plane_type, MbModeInfo, MAX_MB_PLANE, MI_NONE};
use super::enums::*;
use crate::aom::aom_dsp::aom_dsp_common::get_msb;

pub(crate) const CDEF_STRENGTH_BITS: i32 = 6;

pub(crate) const CDEF_PRI_STRENGTHS: i32 = 16;
pub(crate) const CDEF_SEC_STRENGTHS: i32 = 4;

const CDEF_NBLOCKS: usize = (1 << MAX_SB_SIZE_LOG2) / 8;

/* We need to buffer two vertical lines. */
const CDEF_VBORDER: i32 = 2;
/* We only need to buffer three horizontal pixels too, but let's align to
16 bytes (8 x 16 bits) to make vectorization easier. */
const CDEF_HBORDER: i32 = 8;
const CDEF_BSTRIDE: i32 = ((1 << MAX_SB_SIZE_LOG2) + 2 * CDEF_HBORDER + 7) & !7;

const CDEF_VERY_LARGE: u16 = 0x4000;
const CDEF_INBUF_SIZE: i32 = CDEF_BSTRIDE * ((1 << MAX_SB_SIZE_LOG2) + 2 * CDEF_VBORDER);

const CDEF_BLOCKSIZE: i32 = 64;

const TOP: usize = 0;
const LEFT: usize = 1;
const BOTTOM: usize = 2;
const RIGHT: usize = 3;

#[derive(Debug, Clone, Copy, Default)]
struct CdefList {
    by: u8,
    bx: u8,
}

/// Translation of `constrain()`.
#[inline]
fn constrain(diff: i32, threshold: i32, damping: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }

    let shift = 0.max(damping - get_msb(threshold as u32));
    diff.signum() * diff.abs().min(0.max(threshold - (diff.abs() >> shift)))
}

/// Translation of `is_8x8_block_skip()`.
fn is_8x8_block_skip(
    mi_params: &CommonModeInfoParams,
    mi_alloc: &[MbModeInfo],
    mi_row: i32,
    mi_col: i32,
) -> Result<bool, ()> {
    let mi_stride = mi_params.mi_stride;
    for r in 0..MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32 {
        for c in 0..MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32 {
            let mbmi =
                grid_mbmi(mi_params, mi_alloc, (mi_row + r) * mi_stride + mi_col + c)?.ok_or(())?;
            if mbmi.skip_txfm == 0 {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

/// `mi_params->mi_grid_base[idx]` (`None` for a NULL pointer).
fn grid_mbmi<'a>(
    mi_params: &CommonModeInfoParams,
    mi_alloc: &'a [MbModeInfo],
    idx: i32,
) -> Result<Option<&'a MbModeInfo>, ()> {
    let i = *mi_params
        .mi_grid_base
        .get(usize::try_from(idx).map_err(|_| ())?)
        .ok_or(())?;
    if i == MI_NONE {
        return Ok(None);
    }
    mi_alloc.get(i as usize).map(Some).ok_or(())
}

/// Translation of `av1_cdef_compute_sb_list()`.
fn av1_cdef_compute_sb_list(
    mi_params: &CommonModeInfoParams,
    mi_alloc: &[MbModeInfo],
    mi_row: i32,
    mi_col: i32,
    dlist: &mut [CdefList],
    bs: BlockSize,
) -> Result<i32, ()> {
    let mut maxc = mi_params.mi_cols - mi_col;
    let mut maxr = mi_params.mi_rows - mi_row;

    if bs == BLOCK_128X128 || bs == BLOCK_128X64 {
        maxc = maxc.min(MI_SIZE_128X128);
    } else {
        maxc = maxc.min(MI_SIZE_64X64);
    }
    if bs == BLOCK_128X128 || bs == BLOCK_64X128 {
        maxr = maxr.min(MI_SIZE_128X128);
    } else {
        maxr = maxr.min(MI_SIZE_64X64);
    }

    let r_step = 2; // mi_size_high[BLOCK_8X8]
    let c_step = 2; // mi_size_wide[BLOCK_8X8]
    let r_shift = 1;
    let c_shift = 1;
    let mut count = 0;
    let mut r = 0;
    while r < maxr {
        let mut c = 0;
        while c < maxc {
            if !is_8x8_block_skip(mi_params, mi_alloc, mi_row + r, mi_col + c)? {
                let d = dlist.get_mut(count).ok_or(())?;
                d.by = (r >> r_shift) as u8;
                d.bx = (c >> c_shift) as u8;
                count += 1;
            }
            c += c_step;
        }
        r += r_step;
    }
    Ok(count as i32)
}

/// Translation of `cdef_copy_rect8_8bit_to_16bit_c()` and
/// `cdef_copy_rect8_16bit_to_16bit_c()` (the frame buffers hold 16-bit
/// samples at either bit depth here, so both are the same copy), through
/// `av1_cdef_copy_sb8_16()`.
#[allow(clippy::too_many_arguments)]
fn av1_cdef_copy_sb8_16(
    dst: &mut [u16],
    dst_off: usize,
    dstride: i32,
    src: &[u16],
    src_base: usize,
    src_voffset: i32,
    src_hoffset: i32,
    sstride: i32,
    vsize: i32,
    hsize: i32,
) -> Result<(), ()> {
    let base = src_base as isize + (src_voffset * sstride + src_hoffset) as isize;
    for i in 0..vsize {
        let s = usize::try_from(base + (i * sstride) as isize).map_err(|_| ())?;
        let d = dst_off + (i * dstride) as usize;
        let n = hsize as usize;
        dst.get_mut(d..d + n)
            .ok_or(())?
            .copy_from_slice(src.get(s..s + n).ok_or(())?);
    }
    Ok(())
}

/// Translation of `copy_rect()`.
fn copy_rect(
    dst: &mut [u16],
    dst_off: usize,
    dstride: i32,
    src: &[u16],
    src_off: usize,
    sstride: i32,
    v: i32,
    h: i32,
) -> Result<(), ()> {
    for i in 0..v {
        let d = dst_off + (i * dstride) as usize;
        let s = src_off + (i * sstride) as usize;
        let n = h as usize;
        dst.get_mut(d..d + n)
            .ok_or(())?
            .copy_from_slice(src.get(s..s + n).ok_or(())?);
    }
    Ok(())
}

/// Translation of `fill_rect()`.
fn fill_rect(
    dst: &mut [u16],
    dst_off: usize,
    dstride: i32,
    v: i32,
    h: i32,
    x: u16,
) -> Result<(), ()> {
    for i in 0..v {
        let d = dst_off + (i * dstride) as usize;
        dst.get_mut(d..d + h as usize).ok_or(())?.fill(x);
    }
    Ok(())
}

/// The parts of `CdefBlockInfo` the filter uses.
struct CdefBlockInfo {
    /// Offset of the plane in the frame buffer (`dst`).
    dst: usize,
    dst_stride: i32,
    top_linebuf: [usize; MAX_MB_PLANE],
    bot_linebuf: [usize; MAX_MB_PLANE],
    xdec: i32,
    ydec: i32,
    mi_wide_l2: i32,
    mi_high_l2: i32,
    frame_boundary: [bool; 4],
    damping: i32,
    coeff_shift: i32,
    level: i32,
    sec_strength: i32,
    cdef_count: i32,
    dir: [[i32; CDEF_NBLOCKS]; CDEF_NBLOCKS],
    var: [[i32; CDEF_NBLOCKS]; CDEF_NBLOCKS],
    dlist: [CdefList; (MI_SIZE_64X64 * MI_SIZE_64X64) as usize],
    roffset: i32,
    coffset: i32,
}

/// The CDEF buffers (`cdef_info->linebuf`, `colbuf` and `srcbuf`).
struct CdefBufs {
    linebuf: [Vec<u16>; MAX_MB_PLANE],
    colbuf: [Vec<u16>; MAX_MB_PLANE],
    src: Vec<u16>,
}

/// The frame being filtered with the plane geometry of
/// `av1_setup_dst_planes()`.
struct CdefFrame<'a> {
    buf: &'a mut [u16],
    plane_buf: [usize; MAX_MB_PLANE],
    plane_stride: [i32; MAX_MB_PLANE],
    ss_x: [i32; MAX_MB_PLANE],
    ss_y: [i32; MAX_MB_PLANE],
}

/// Translation of `cdef_prepare_fb()`.
#[allow(clippy::too_many_arguments)]
fn cdef_prepare_fb(
    cm: &Av1Common,
    frame: &CdefFrame,
    fb_info: &CdefBlockInfo,
    bufs: &mut CdefBufs,
    cdef_left: bool,
    fbc: i32,
    fbr: i32,
    plane: usize,
) -> Result<(), ()> {
    let mi_params = &cm.mi_params;
    let src = &mut bufs.src;
    let luma_stride = ((mi_params.mi_cols << MI_SIZE_LOG2) + 15) & !15;
    let nvfb = (mi_params.mi_rows + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    let nhfb = (mi_params.mi_cols + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    let mut cstart = 0;
    if !cdef_left {
        cstart = -CDEF_HBORDER;
    }
    let nhb = MI_SIZE_64X64.min(mi_params.mi_cols - MI_SIZE_64X64 * fbc);
    let nvb = MI_SIZE_64X64.min(mi_params.mi_rows - MI_SIZE_64X64 * fbr);
    let hsize = nhb << fb_info.mi_wide_l2;
    let vsize = nvb << fb_info.mi_high_l2;
    let top_linebuf = &bufs.linebuf[plane][fb_info.top_linebuf[plane]..];
    let bot_linebuf = &bufs.linebuf[plane][fb_info.bot_linebuf[plane]..];
    let bot_offset = ((vsize + CDEF_VBORDER) * CDEF_BSTRIDE) as usize;
    let stride = luma_stride
        >> (if plane == AOM_PLANE_Y {
            0
        } else {
            cm.seq_params.subsampling_x
        });

    let cend = if fbc == nhfb - 1 {
        hsize
    } else {
        hsize + CDEF_HBORDER
    };

    let rend = if fbr == nvfb - 1 {
        vsize
    } else {
        vsize + CDEF_VBORDER
    };

    let hb = CDEF_HBORDER as usize;
    let coffset = fb_info.coffset as usize;
    let hs = hsize as usize;

    /* Copy in the pixels we need from the current superblock for
    deringing.*/
    av1_cdef_copy_sb8_16(
        src,
        (CDEF_VBORDER * CDEF_BSTRIDE + CDEF_HBORDER + cstart) as usize,
        CDEF_BSTRIDE,
        frame.buf,
        fb_info.dst,
        fb_info.roffset,
        fb_info.coffset + cstart,
        fb_info.dst_stride,
        vsize,
        cend - cstart,
    )?;

    /* Copy in the pixels we need for the current superblock from bottom buffer.*/
    if fbr < nvfb - 1 {
        copy_rect(
            src,
            bot_offset + hb,
            CDEF_BSTRIDE,
            bot_linebuf,
            coffset,
            stride,
            CDEF_VBORDER,
            hsize,
        )?;
    } else {
        fill_rect(
            src,
            bot_offset + hb,
            CDEF_BSTRIDE,
            CDEF_VBORDER,
            hsize,
            CDEF_VERY_LARGE,
        )?;
    }
    if fbr < nvfb - 1 && fbc > 0 {
        copy_rect(
            src,
            bot_offset,
            CDEF_BSTRIDE,
            bot_linebuf,
            coffset - hb,
            stride,
            CDEF_VBORDER,
            CDEF_HBORDER,
        )?;
    } else {
        fill_rect(
            src,
            bot_offset,
            CDEF_BSTRIDE,
            CDEF_VBORDER,
            CDEF_HBORDER,
            CDEF_VERY_LARGE,
        )?;
    }
    if fbr < nvfb - 1 && fbc < nhfb - 1 {
        copy_rect(
            src,
            bot_offset + hs + hb,
            CDEF_BSTRIDE,
            bot_linebuf,
            coffset + hs,
            stride,
            CDEF_VBORDER,
            CDEF_HBORDER,
        )?;
    } else {
        fill_rect(
            src,
            bot_offset + hs + hb,
            CDEF_BSTRIDE,
            CDEF_VBORDER,
            CDEF_HBORDER,
            CDEF_VERY_LARGE,
        )?;
    }

    /* Copy in the pixels we need from the current superblock from top buffer.*/
    if fbr > 0 {
        copy_rect(
            src,
            hb,
            CDEF_BSTRIDE,
            top_linebuf,
            coffset,
            stride,
            CDEF_VBORDER,
            hsize,
        )?;
    } else {
        fill_rect(src, hb, CDEF_BSTRIDE, CDEF_VBORDER, hsize, CDEF_VERY_LARGE)?;
    }
    if fbr > 0 && fbc > 0 {
        copy_rect(
            src,
            0,
            CDEF_BSTRIDE,
            top_linebuf,
            coffset - hb,
            stride,
            CDEF_VBORDER,
            CDEF_HBORDER,
        )?;
    } else {
        fill_rect(
            src,
            0,
            CDEF_BSTRIDE,
            CDEF_VBORDER,
            CDEF_HBORDER,
            CDEF_VERY_LARGE,
        )?;
    }
    if fbr > 0 && fbc < nhfb - 1 {
        copy_rect(
            src,
            hs + hb,
            CDEF_BSTRIDE,
            top_linebuf,
            coffset + hs,
            stride,
            CDEF_VBORDER,
            CDEF_HBORDER,
        )?;
    } else {
        fill_rect(
            src,
            hs + hb,
            CDEF_BSTRIDE,
            CDEF_VBORDER,
            CDEF_HBORDER,
            CDEF_VERY_LARGE,
        )?;
    }
    if cdef_left {
        /* If we deringed the superblock on the left then we need to copy in
        saved pixels. */
        copy_rect(
            src,
            0,
            CDEF_BSTRIDE,
            &bufs.colbuf[plane],
            0,
            CDEF_HBORDER,
            rend + CDEF_VBORDER,
            CDEF_HBORDER,
        )?;
    }
    /* Saving pixels in case we need to dering the superblock on the
    right. */
    copy_rect(
        &mut bufs.colbuf[plane],
        0,
        CDEF_HBORDER,
        src,
        hs,
        CDEF_BSTRIDE,
        rend + CDEF_VBORDER,
        CDEF_HBORDER,
    )?;

    if fb_info.frame_boundary[LEFT] {
        fill_rect(
            src,
            0,
            CDEF_BSTRIDE,
            vsize + 2 * CDEF_VBORDER,
            CDEF_HBORDER,
            CDEF_VERY_LARGE,
        )?;
    }
    if fb_info.frame_boundary[RIGHT] {
        fill_rect(
            src,
            hs + hb,
            CDEF_BSTRIDE,
            vsize + 2 * CDEF_VBORDER,
            CDEF_HBORDER,
            CDEF_VERY_LARGE,
        )?;
    }
    Ok(())
}

/* This is Cdef_Directions (section 7.15.3) with 2 padding entries at the
beginning and end of the table. The cdef direction range is [0, 7] and the
first index is offset +/-2. This removes the need to constrain the first
index to the same range using e.g., & 7. */
const CDEF_DIRECTIONS_PADDED: [[i32; 2]; 12] = [
    /* Padding: cdef_directions[6] */
    [CDEF_BSTRIDE, 2 * CDEF_BSTRIDE],
    /* Padding: cdef_directions[7] */
    [CDEF_BSTRIDE, 2 * CDEF_BSTRIDE - 1],
    /* Begin cdef_directions */
    [-CDEF_BSTRIDE + 1, -2 * CDEF_BSTRIDE + 2],
    [1, -CDEF_BSTRIDE + 2],
    [1, 2],
    [1, CDEF_BSTRIDE + 2],
    [CDEF_BSTRIDE + 1, 2 * CDEF_BSTRIDE + 2],
    [CDEF_BSTRIDE, 2 * CDEF_BSTRIDE + 1],
    [CDEF_BSTRIDE, 2 * CDEF_BSTRIDE],
    [CDEF_BSTRIDE, 2 * CDEF_BSTRIDE - 1],
    /* End cdef_directions */
    /* Padding: cdef_directions[0] */
    [-CDEF_BSTRIDE + 1, -2 * CDEF_BSTRIDE + 2],
    /* Padding: cdef_directions[1] */
    [1, -CDEF_BSTRIDE + 2],
];

/// `cdef_directions[dir][k]` (`dir` in -2..=9).
#[inline]
fn cdef_directions(dir: i32, k: usize) -> isize {
    CDEF_DIRECTIONS_PADDED[(dir + 2) as usize][k] as isize
}

/* Detect direction. 0 means 45-degree up-right, 2 is horizontal, and so on.
The search minimizes the weighted variance along all the lines in a
particular direction, i.e. the squared error between the input and a
"predicted" block where each pixel is replaced by the average along a line
in a particular direction. Since each direction have the same sum(x^2) term,
that term is never computed. See Section 2, step 2, of:
http://jmvalin.ca/notes/intra_paint.pdf */
/// Translation of `cdef_find_dir_c()`.
fn cdef_find_dir_c(img: &[u16], off: usize, stride: i32, var: &mut i32, coeff_shift: i32) -> i32 {
    let mut cost = [0i32; 8];
    let mut partial = [[0i32; 15]; 8];
    let mut best_cost = 0i32;
    let mut best_dir = 0usize;
    /* Instead of dividing by n between 2 and 8, we multiply by 3*5*7*8/n.
    The output is then 840 times larger, but we don't care for finding
    the max. */
    const DIV_TABLE: [i32; 9] = [0, 840, 420, 280, 210, 168, 140, 120, 105];
    for i in 0..8usize {
        for j in 0..8usize {
            /* We subtract 128 here to reduce the maximum range of the squared
            partial sums. */
            let x = (img[off + i * stride as usize + j] as i32 >> coeff_shift) - 128;
            partial[0][i + j] += x;
            partial[1][i + j / 2] += x;
            partial[2][i] += x;
            partial[3][3 + i - j / 2] += x;
            partial[4][7 + i - j] += x;
            partial[5][3 - i / 2 + j] += x;
            partial[6][j] += x;
            partial[7][i / 2 + j] += x;
        }
    }
    for i in 0..8 {
        cost[2] = cost[2].wrapping_add(partial[2][i].wrapping_mul(partial[2][i]));
        cost[6] = cost[6].wrapping_add(partial[6][i].wrapping_mul(partial[6][i]));
    }
    cost[2] = cost[2].wrapping_mul(DIV_TABLE[8]);
    cost[6] = cost[6].wrapping_mul(DIV_TABLE[8]);
    for i in 0..7 {
        cost[0] = cost[0].wrapping_add(
            (partial[0][i] * partial[0][i] + partial[0][14 - i] * partial[0][14 - i])
                .wrapping_mul(DIV_TABLE[i + 1]),
        );
        cost[4] = cost[4].wrapping_add(
            (partial[4][i] * partial[4][i] + partial[4][14 - i] * partial[4][14 - i])
                .wrapping_mul(DIV_TABLE[i + 1]),
        );
    }
    cost[0] = cost[0].wrapping_add((partial[0][7] * partial[0][7]).wrapping_mul(DIV_TABLE[8]));
    cost[4] = cost[4].wrapping_add((partial[4][7] * partial[4][7]).wrapping_mul(DIV_TABLE[8]));
    let mut i = 1;
    while i < 8 {
        for j in 0..4 + 1 {
            cost[i] = cost[i].wrapping_add(partial[i][3 + j] * partial[i][3 + j]);
        }
        cost[i] = cost[i].wrapping_mul(DIV_TABLE[8]);
        for j in 0..4 - 1 {
            cost[i] = cost[i].wrapping_add(
                (partial[i][j] * partial[i][j] + partial[i][10 - j] * partial[i][10 - j])
                    .wrapping_mul(DIV_TABLE[2 * j + 2]),
            );
        }
        i += 2;
    }
    for (i, &c) in cost.iter().enumerate() {
        if c > best_cost {
            best_cost = c;
            best_dir = i;
        }
    }
    /* Difference between the optimal variance and the variance along the
    orthogonal direction. Again, the sum(x^2) terms cancel out. */
    *var = best_cost.wrapping_sub(cost[(best_dir + 4) & 7]);
    /* We'd normally divide by 840, but dividing by 1024 is close enough
    for what we're going to do with this. */
    *var >>= 10;
    best_dir as i32
}

const CDEF_PRI_TAPS: [[i32; 2]; 2] = [[4, 2], [3, 3]];
const CDEF_SEC_TAPS: [i32; 2] = [2, 1];

/// Translation of `cdef_filter_block_internal()` (through the
/// `cdef_filter_8_*_c()` and `cdef_filter_16_*_c()` wrappers): smooth in
/// the direction detected.
#[allow(clippy::too_many_arguments)]
fn cdef_filter_block_internal(
    dst: &mut [u16],
    dst_off: usize,
    is_dst8: bool,
    dstride: i32,
    input: &[u16],
    in_off: usize,
    pri_strength: i32,
    sec_strength: i32,
    dir: i32,
    pri_damping: i32,
    sec_damping: i32,
    coeff_shift: i32,
    block_width: i32,
    block_height: i32,
    enable_primary: bool,
    enable_secondary: bool,
) -> Result<(), ()> {
    let clipping_required = enable_primary && enable_secondary;
    let s = CDEF_BSTRIDE as isize;
    let pri_taps = &CDEF_PRI_TAPS[((pri_strength >> coeff_shift) & 1) as usize];
    let sec_taps = &CDEF_SEC_TAPS;
    let at = |p: isize| -> Result<i16, ()> {
        let i = usize::try_from(p).map_err(|_| ())?;
        input.get(i).map(|&v| v as i16).ok_or(())
    };
    for i in 0..block_height as isize {
        for j in 0..block_width as isize {
            let mut sum: i16 = 0;
            let pos = in_off as isize + i * s + j;
            let x = at(pos)?;
            let mut max = x as i32;
            let mut min = x as i32;
            for k in 0..2 {
                if enable_primary {
                    let p0 = at(pos + cdef_directions(dir, k))?;
                    let p1 = at(pos - cdef_directions(dir, k))?;
                    sum = (sum as i32
                        + pri_taps[k] * constrain(p0 as i32 - x as i32, pri_strength, pri_damping))
                        as i16;
                    sum = (sum as i32
                        + pri_taps[k] * constrain(p1 as i32 - x as i32, pri_strength, pri_damping))
                        as i16;
                    if clipping_required {
                        if p0 as u16 != CDEF_VERY_LARGE {
                            max = max.max(p0 as i32);
                        }
                        if p1 as u16 != CDEF_VERY_LARGE {
                            max = max.max(p1 as i32);
                        }
                        min = min.min(p0 as i32);
                        min = min.min(p1 as i32);
                    }
                }
                if enable_secondary {
                    let s0 = at(pos + cdef_directions(dir + 2, k))?;
                    let s1 = at(pos - cdef_directions(dir + 2, k))?;
                    let s2 = at(pos + cdef_directions(dir - 2, k))?;
                    let s3 = at(pos - cdef_directions(dir - 2, k))?;
                    if clipping_required {
                        for sv in [s0, s1, s2, s3] {
                            if sv as u16 != CDEF_VERY_LARGE {
                                max = max.max(sv as i32);
                            }
                        }
                        min = min.min(s0 as i32);
                        min = min.min(s1 as i32);
                        min = min.min(s2 as i32);
                        min = min.min(s3 as i32);
                    }
                    for sv in [s0, s1, s2, s3] {
                        sum = (sum as i32
                            + sec_taps[k]
                                * constrain(sv as i32 - x as i32, sec_strength, sec_damping))
                            as i16;
                    }
                }
            }
            let sum = sum as i32;
            let mut y = (x as i32 + ((8 + sum - (sum < 0) as i32) >> 4)) as i16;
            if clipping_required {
                y = (y as i32).clamp(min, max) as i16;
            }

            let d = dst
                .get_mut(dst_off + (i as i32 * dstride) as usize + j as usize)
                .ok_or(())?;
            if is_dst8 {
                *d = y as u8 as u16;
            } else {
                *d = y as u16;
            }
        }
    }
    Ok(())
}

/* Compute the primary filter strength for an 8x8 block based on the
directional variance difference. A high variance difference means
that we have a highly directional pattern (e.g. a high contrast
edge), so we can apply more deringing. A low variance means that we
either have a low contrast edge, or a non-directional texture, so
we want to be careful not to blur. */
/// Translation of `adjust_strength()`.
#[inline]
fn adjust_strength(strength: i32, var: i32) -> i32 {
    let i = if (var >> 6) != 0 {
        get_msb((var >> 6) as u32).min(12)
    } else {
        0
    };
    /* We use the variance of 8x8 blocks to adjust the strength. */
    if var != 0 {
        (strength * (4 + i) + 8) >> 4
    } else {
        0
    }
}

/// Translation of `aom_cdef_find_dir()` (with `cdef_find_dir_dual_c()`).
fn aom_cdef_find_dir(
    input: &[u16],
    in_off: usize,
    dlist: &[CdefList],
    var: &mut [[i32; CDEF_NBLOCKS]; CDEF_NBLOCKS],
    cdef_count: i32,
    coeff_shift: i32,
    dir: &mut [[i32; CDEF_NBLOCKS]; CDEF_NBLOCKS],
) {
    let bs = CDEF_BSTRIDE as usize;
    let mut bi = 0usize;
    let count = cdef_count as usize;

    // Find direction of two 8x8 blocks together.
    while bi + 1 < count {
        for b in [bi, bi + 1] {
            let by = dlist[b].by as usize;
            let bx = dlist[b].bx as usize;
            let pos = 8 * by * bs + 8 * bx;
            dir[by][bx] = cdef_find_dir_c(
                input,
                in_off + pos,
                CDEF_BSTRIDE,
                &mut var[by][bx],
                coeff_shift,
            );
        }
        bi += 2;
    }

    // Process remaining 8x8 blocks here. One 8x8 at a time.
    if count % 2 != 0 {
        let by = dlist[bi].by as usize;
        let bx = dlist[bi].bx as usize;
        dir[by][bx] = cdef_find_dir_c(
            input,
            in_off + 8 * by * bs + 8 * bx,
            CDEF_BSTRIDE,
            &mut var[by][bx],
            coeff_shift,
        );
    }
}

/// Translation of `av1_cdef_filter_fb()` for the frame filter (`dirinit`
/// NULL: only `av1_cdef_search()` sets it).
#[allow(clippy::too_many_arguments)]
fn av1_cdef_filter_fb(
    dst: &mut [u16],
    dst_off: usize,
    is_dst8: bool,
    dstride: i32,
    input: &[u16],
    in_off: usize,
    fb: &mut CdefBlockInfo,
    pli: usize,
) -> Result<(), ()> {
    let xdec = fb.xdec;
    let ydec = fb.ydec;
    let pri_strength = fb.level << fb.coeff_shift;
    let sec_strength = fb.sec_strength << fb.coeff_shift;
    let damping = fb.damping + fb.coeff_shift - (pli != AOM_PLANE_Y) as i32;
    let bw_log2 = 3 - xdec;
    let bh_log2 = 3 - ydec;
    let cdef_count = fb.cdef_count;

    if pli == 0 {
        aom_cdef_find_dir(
            input,
            in_off,
            &fb.dlist,
            &mut fb.var,
            cdef_count,
            fb.coeff_shift,
            &mut fb.dir,
        );
    }
    if pli == 1 && xdec != ydec {
        const CONV422: [i32; 8] = [7, 0, 2, 4, 5, 6, 6, 6];
        const CONV440: [i32; 8] = [1, 2, 2, 2, 3, 4, 6, 0];
        for d in &fb.dlist[..cdef_count as usize] {
            let (by, bx) = (d.by as usize, d.bx as usize);
            fb.dir[by][bx] = (if xdec != 0 { CONV422 } else { CONV440 })[fb.dir[by][bx] as usize];
        }
    }

    let block_width = 8 >> xdec;
    let block_height = 8 >> ydec;
    /*
     * strength_index == 0 : enable_primary = 1, enable_secondary = 1
     * strength_index == 1 : enable_primary = 1, enable_secondary = 0
     * strength_index == 2 : enable_primary = 0, enable_secondary = 1
     * strength_index == 3 : enable_primary = 0, enable_secondary = 0
     */
    for d in &fb.dlist[..cdef_count as usize] {
        let by = d.by as i32;
        let bx = d.bx as i32;
        let t = if pli != 0 {
            pri_strength
        } else {
            adjust_strength(pri_strength, fb.var[by as usize][bx as usize])
        };
        let strength_index = (sec_strength == 0) as i32 | (((t == 0) as i32) << 1);

        cdef_filter_block_internal(
            dst,
            dst_off + ((by << bh_log2) * dstride + (bx << bw_log2)) as usize,
            is_dst8,
            dstride,
            input,
            in_off + ((by * CDEF_BSTRIDE << bh_log2) + (bx << bw_log2)) as usize,
            t,
            sec_strength,
            if pri_strength != 0 {
                fb.dir[by as usize][bx as usize]
            } else {
                0
            },
            damping,
            damping,
            fb.coeff_shift,
            block_width,
            block_height,
            strength_index & 2 == 0,
            strength_index & 1 == 0,
        )?;
    }
    Ok(())
}

/// Translation of `cdef_filter_fb()`.
fn cdef_filter_fb(
    frame: &mut CdefFrame,
    src: &[u16],
    fb_info: &mut CdefBlockInfo,
    plane: usize,
    use_highbitdepth: bool,
) -> Result<(), ()> {
    let offset =
        fb_info.dst as isize + (fb_info.dst_stride * fb_info.roffset + fb_info.coffset) as isize;
    let offset = usize::try_from(offset).map_err(|_| ())?;
    let dst_stride = fb_info.dst_stride;
    av1_cdef_filter_fb(
        frame.buf,
        offset,
        !use_highbitdepth,
        dst_stride,
        src,
        (CDEF_VBORDER * CDEF_BSTRIDE + CDEF_HBORDER) as usize,
        fb_info,
        plane,
    )
}

/// Translation of `cdef_init_fb_col()`: initializes block-level parameters
/// for CDEF.
fn cdef_init_fb_col(
    frame: &CdefFrame,
    fb_info: &mut CdefBlockInfo,
    level: &[i32; PLANE_TYPES as usize],
    sec_strength: &[i32; PLANE_TYPES as usize],
    fbc: i32,
    fbr: i32,
    plane: usize,
) {
    let plane_type = get_plane_type(plane) as usize;
    fb_info.level = level[plane_type];
    fb_info.sec_strength = sec_strength[plane_type];
    fb_info.dst = frame.plane_buf[plane];
    fb_info.dst_stride = frame.plane_stride[plane];

    fb_info.xdec = frame.ss_x[plane];
    fb_info.ydec = frame.ss_y[plane];
    fb_info.mi_wide_l2 = MI_SIZE_LOG2 - frame.ss_x[plane];
    fb_info.mi_high_l2 = MI_SIZE_LOG2 - frame.ss_y[plane];
    fb_info.roffset = (MI_SIZE_64X64 * fbr) << fb_info.mi_high_l2;
    fb_info.coffset = (MI_SIZE_64X64 * fbc) << fb_info.mi_wide_l2;
}

/// Translation of `cdef_fb_col()`.
#[allow(clippy::too_many_arguments)]
fn cdef_fb_col(
    cm: &Av1Common,
    mi_alloc: &[MbModeInfo],
    frame: &mut CdefFrame,
    fb_info: &mut CdefBlockInfo,
    bufs: &mut CdefBufs,
    cdef_left: &mut [bool; MAX_MB_PLANE],
    fbc: i32,
    fbr: i32,
) -> Result<(), ()> {
    let mi_params = &cm.mi_params;
    let num_planes = av1_num_planes(cm);
    let mut is_zero_level = [true, true];
    let mut level = [0i32; PLANE_TYPES as usize];
    let mut sec_strength = [0i32; PLANE_TYPES as usize];
    let cdef_info = &cm.cdef_info;

    let mbmi = grid_mbmi(
        mi_params,
        mi_alloc,
        MI_SIZE_64X64 * fbr * mi_params.mi_stride + MI_SIZE_64X64 * fbc,
    )?;
    let mbmi_cdef_strength = match mbmi {
        Some(m) if m.cdef_strength != -1 => m.cdef_strength as usize,
        _ => {
            cdef_left[..num_planes].fill(false);
            return Ok(());
        }
    };
    let strength = *cdef_info.cdef_strengths.get(mbmi_cdef_strength).ok_or(())?;
    let uv_strength = *cdef_info
        .cdef_uv_strengths
        .get(mbmi_cdef_strength)
        .ok_or(())?;

    // Compute level and secondary strength for planes
    let y = PLANE_TYPE_Y as usize;
    let uv = PLANE_TYPE_UV as usize;
    level[y] = strength / CDEF_SEC_STRENGTHS;
    sec_strength[y] = strength % CDEF_SEC_STRENGTHS;
    sec_strength[y] += (sec_strength[y] == 3) as i32;
    is_zero_level[y] = (level[y] == 0) && (sec_strength[y] == 0);

    if num_planes > 1 {
        level[uv] = uv_strength / CDEF_SEC_STRENGTHS;
        sec_strength[uv] = uv_strength % CDEF_SEC_STRENGTHS;
        sec_strength[uv] += (sec_strength[uv] == 3) as i32;
        is_zero_level[uv] = (level[uv] == 0) && (sec_strength[uv] == 0);
    }

    if is_zero_level[y] && is_zero_level[uv] {
        cdef_left[..num_planes].fill(false);
        return Ok(());
    }

    fb_info.cdef_count = av1_cdef_compute_sb_list(
        mi_params,
        mi_alloc,
        fbr * MI_SIZE_64X64,
        fbc * MI_SIZE_64X64,
        &mut fb_info.dlist,
        BLOCK_64X64,
    )?;
    if fb_info.cdef_count == 0 {
        cdef_left[..num_planes].fill(false);
        return Ok(());
    }

    for plane in 0..num_planes {
        // Do not skip cdef filtering for luma plane as filter direction is
        // computed based on luma.
        if plane != 0 && is_zero_level[get_plane_type(plane) as usize] {
            cdef_left[plane] = false;
            continue;
        }
        cdef_init_fb_col(frame, fb_info, &level, &sec_strength, fbc, fbr, plane);
        cdef_prepare_fb(cm, frame, fb_info, bufs, cdef_left[plane], fbc, fbr, plane)?;
        let src = std::mem::take(&mut bufs.src);
        let r = cdef_filter_fb(
            frame,
            &src,
            fb_info,
            plane,
            cm.seq_params.use_highbitdepth != 0,
        );
        bufs.src = src;
        r?;
        cdef_left[plane] = true;
    }
    Ok(())
}

/// Translation of `av1_cdef_init_fb_row()`: initializes row-level
/// parameters for CDEF frame.
fn av1_cdef_init_fb_row(
    cm: &Av1Common,
    frame: &CdefFrame,
    fb_info: &mut CdefBlockInfo,
    bufs: &mut CdefBufs,
    fbr: i32,
) -> Result<(), ()> {
    let num_planes = av1_num_planes(cm);
    let nvfb = (cm.mi_params.mi_rows + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    let luma_stride = ((cm.mi_params.mi_cols << MI_SIZE_LOG2) + 15) & !15;
    let ping_pong = (fbr & 1) as usize;
    // for the current filter block, it's top left corner mi structure (mi_tl)
    // is first accessed to check whether the top and left boundaries are
    // frame boundaries. Then bottom-left and top-right mi structures are
    // accessed to check whether the bottom and right boundaries
    // (respectively) are frame boundaries.
    //
    // Note that we can't just check the bottom-right mi structure - eg. if
    // we're at the right-hand edge of the frame but not the bottom, then
    // the bottom-right mi is NULL but the bottom-left is not.
    fb_info.frame_boundary[TOP] = MI_SIZE_64X64 * fbr == 0;
    if fbr != nvfb - 1 {
        fb_info.frame_boundary[BOTTOM] = MI_SIZE_64X64 * (fbr + 1) == cm.mi_params.mi_rows;
    } else {
        fb_info.frame_boundary[BOTTOM] = true;
    }

    fb_info.damping = cm.cdef_info.cdef_damping;
    fb_info.coeff_shift = (cm.seq_params.bit_depth - 8).max(0);
    fb_info.dir = [[0; CDEF_NBLOCKS]; CDEF_NBLOCKS];
    fb_info.var = [[0; CDEF_NBLOCKS]; CDEF_NBLOCKS];

    for plane in 0..num_planes {
        let mi_high_l2 = MI_SIZE_LOG2 - frame.ss_y[plane];
        let offset = (MI_SIZE_64X64 * (fbr + 1)) << mi_high_l2;
        let stride = luma_stride >> frame.ss_x[plane];
        // here ping-pong buffers are maintained for top linebuf
        // to avoid linebuf over-write by consecutive row.
        let top_linebuf = ping_pong * (CDEF_VBORDER * stride) as usize;
        fb_info.bot_linebuf[plane] = ((CDEF_VBORDER << 1) * stride) as usize;

        if fbr != nvfb - 1 {
            // top line buffer copy
            av1_cdef_copy_sb8_16(
                &mut bufs.linebuf[plane],
                top_linebuf,
                stride,
                frame.buf,
                frame.plane_buf[plane],
                offset - CDEF_VBORDER,
                0,
                frame.plane_stride[plane],
                CDEF_VBORDER,
                stride,
            )?;
        }
        fb_info.top_linebuf[plane] = (1 - ping_pong) * (CDEF_VBORDER * stride) as usize;

        if fbr != nvfb - 1 {
            // bottom line buffer copy
            av1_cdef_copy_sb8_16(
                &mut bufs.linebuf[plane],
                fb_info.bot_linebuf[plane],
                stride,
                frame.buf,
                frame.plane_buf[plane],
                offset,
                0,
                frame.plane_stride[plane],
                CDEF_VBORDER,
                stride,
            )?;
        }
    }
    Ok(())
}

/// Translation of `av1_cdef_fb_row()`.
fn av1_cdef_fb_row(
    cm: &Av1Common,
    mi_alloc: &[MbModeInfo],
    frame: &mut CdefFrame,
    fb_info: &mut CdefBlockInfo,
    bufs: &mut CdefBufs,
    fbr: i32,
) -> Result<(), ()> {
    let mut cdef_left = [true; MAX_MB_PLANE];
    let nhfb = (cm.mi_params.mi_cols + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;

    av1_cdef_init_fb_row(cm, frame, fb_info, bufs, fbr)?;
    for fbc in 0..nhfb {
        fb_info.frame_boundary[LEFT] = MI_SIZE_64X64 * fbc == 0;
        if fbc != nhfb - 1 {
            fb_info.frame_boundary[RIGHT] = MI_SIZE_64X64 * (fbc + 1) == cm.mi_params.mi_cols;
        } else {
            fb_info.frame_boundary[RIGHT] = true;
        }
        cdef_fb_col(cm, mi_alloc, frame, fb_info, bufs, &mut cdef_left, fbc, fbr)?;
    }
    Ok(())
}

/// Translation of `av1_cdef_frame()` on `cm->cur_frame->buf` (with the
/// buffer sizes of `av1_alloc_cdef_buffers()` for one worker): perform CDEF
/// on the frame.
pub(crate) fn av1_cdef_frame(cm: &mut Av1Common) -> Result<(), ()> {
    let num_planes = av1_num_planes(cm);
    let nvfb = (cm.mi_params.mi_rows + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;

    // num-bufs=3 represents ping-pong buffers for top linebuf,
    // followed by bottom linebuf.
    // ping-pong is to avoid top linebuf over-write by consecutive row.
    let num_bufs = 3;
    let luma_stride = ((cm.mi_params.mi_cols << MI_SIZE_LOG2) + 15) & !15;
    let mut bufs = CdefBufs {
        linebuf: [Vec::new(), Vec::new(), Vec::new()],
        colbuf: [Vec::new(), Vec::new(), Vec::new()],
        src: try_vec(CDEF_INBUF_SIZE as usize, 0u16)?,
    };
    for plane in 0..num_planes {
        let shift = if plane == AOM_PLANE_Y {
            0
        } else {
            cm.seq_params.subsampling_x
        };
        bufs.linebuf[plane] = try_vec(
            (num_bufs * (CDEF_VBORDER << 1) * (luma_stride >> shift)) as usize,
            0u16,
        )?;
        let block_height = (CDEF_BLOCKSIZE << (MI_SIZE_LOG2 - shift)) * 2 * CDEF_VBORDER;
        bufs.colbuf[plane] = try_vec((block_height * CDEF_HBORDER) as usize, 0u16)?;
    }

    let mut buf = std::mem::take(&mut cm.cur_frame_mut().buf);
    let res = {
        let mut frame = CdefFrame {
            buf: &mut buf.buffer_alloc,
            plane_buf: buf.buffers,
            plane_stride: [buf.strides[0], buf.strides[1], buf.strides[1]],
            ss_x: [0, cm.seq_params.subsampling_x, cm.seq_params.subsampling_x],
            ss_y: [0, cm.seq_params.subsampling_y, cm.seq_params.subsampling_y],
        };
        let mut fb_info = CdefBlockInfo {
            dst: 0,
            dst_stride: 0,
            top_linebuf: [0; MAX_MB_PLANE],
            bot_linebuf: [0; MAX_MB_PLANE],
            xdec: 0,
            ydec: 0,
            mi_wide_l2: 0,
            mi_high_l2: 0,
            frame_boundary: [false; 4],
            damping: 0,
            coeff_shift: 0,
            level: 0,
            sec_strength: 0,
            cdef_count: 0,
            dir: [[0; CDEF_NBLOCKS]; CDEF_NBLOCKS],
            var: [[0; CDEF_NBLOCKS]; CDEF_NBLOCKS],
            dlist: [CdefList::default(); (MI_SIZE_64X64 * MI_SIZE_64X64) as usize],
            roffset: 0,
            coffset: 0,
        };
        let cm: &Av1Common = cm;
        (0..nvfb).try_for_each(|fbr| {
            av1_cdef_fb_row(
                cm,
                &cm.mi_params.mi_alloc,
                &mut frame,
                &mut fb_info,
                &mut bufs,
                fbr,
            )
        })
    };
    cm.cur_frame_mut().buf = buf;
    res
}
