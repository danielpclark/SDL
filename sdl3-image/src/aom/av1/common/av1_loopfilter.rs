// Rust translation of the parts of av1/common/av1_loopfilter.c and
// av1/common/av1_loopfilter.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The loop filter parameters and their initialization. The saver picks
//! (and signals) filter levels but never filters: libavif asks libaom to
//! skip the post-processing filters of still pictures.

use super::enums::*;
use super::seg_common::MAX_SEGMENTS;

pub(crate) const MAX_LOOP_FILTER: usize = 63;
pub(crate) const MAX_SHARPNESS: i32 = 7;

pub(crate) const SIMD_WIDTH: usize = 16;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Loopfilter {
    pub(crate) filter_level: [i32; 2],
    pub(crate) filter_level_u: i32,
    pub(crate) filter_level_v: i32,

    pub(crate) sharpness_level: i32,

    pub(crate) mode_ref_delta_enabled: u8,
    pub(crate) mode_ref_delta_update: u8,

    // 0 = Intra, Last, Last2+Last3,
    // GF, BRF, ARF2, ARF
    pub(crate) ref_deltas: [i8; REF_FRAMES as usize],

    // 0 = ZERO_MV, MV
    pub(crate) mode_deltas: [i8; MAX_MODE_LF_DELTAS as usize],
}

// Need to align this structure so when it is declared and
// passed it can be loaded into vector registers.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LoopFilterThresh {
    pub(crate) mblim: [u8; SIMD_WIDTH],
    pub(crate) lim: [u8; SIMD_WIDTH],
    pub(crate) hev_thr: [u8; SIMD_WIDTH],
}

#[derive(Debug, Clone)]
pub(crate) struct LoopFilterInfoN {
    pub(crate) lfthr: [LoopFilterThresh; MAX_LOOP_FILTER + 1],
    pub(crate) lvl: [[[[[u8; MAX_MODE_LF_DELTAS as usize]; REF_FRAMES as usize]; 2]; MAX_SEGMENTS];
        MAX_MB_PLANE],
}

impl Default for LoopFilterInfoN {
    fn default() -> Self {
        LoopFilterInfoN {
            lfthr: [LoopFilterThresh::default(); MAX_LOOP_FILTER + 1],
            lvl: [[[[[0; MAX_MODE_LF_DELTAS as usize]; REF_FRAMES as usize]; 2]; MAX_SEGMENTS];
                MAX_MB_PLANE],
        }
    }
}

pub(crate) const MAX_MB_PLANE: usize = 3;

fn update_sharpness(lfi: &mut LoopFilterInfoN, sharpness_lvl: i32) {
    // For each possible value for the loop filter fill out limits
    for lvl in 0..=MAX_LOOP_FILTER as i32 {
        // Set loop filter parameters that control sharpness.
        let mut block_inside_limit =
            lvl >> ((sharpness_lvl > 0) as i32 + (sharpness_lvl > 4) as i32);

        if sharpness_lvl > 0 {
            if block_inside_limit > (9 - sharpness_lvl) {
                block_inside_limit = 9 - sharpness_lvl;
            }
        }

        if block_inside_limit < 1 {
            block_inside_limit = 1;
        }

        lfi.lfthr[lvl as usize].lim = [block_inside_limit as u8; SIMD_WIDTH];
        lfi.lfthr[lvl as usize].mblim = [(2 * (lvl + 2) + block_inside_limit) as u8; SIMD_WIDTH];
    }
}

pub(crate) fn av1_loop_filter_init(lfi: &mut LoopFilterInfoN, lf: &Loopfilter) {
    // init limits for given sharpness
    update_sharpness(lfi, lf.sharpness_level);

    // init hev threshold const vectors
    for lvl in 0..=MAX_LOOP_FILTER {
        lfi.lfthr[lvl].hev_thr = [(lvl >> 4) as u8; SIMD_WIDTH];
    }
}

// ---------------------------------------------------------------------------
// The filtering of a frame (the "opt" line-at-a-time paths the encoder's
// lpf_opt_level selects), and thread_common.c's single-threaded row loop.
// ---------------------------------------------------------------------------

use super::av1_common_int::Av1Common;
use super::blockd::{av1_get_max_uv_txsize, is_inter_block, Buf2d, MbModeInfo, MI_NONE};
use super::seg_common::*;
use crate::aom::aom_dsp::loopfilter::aom_lpf;
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;

/// `USE_FILTER_TYPE`: the number of 4-sample filter groups along the edge.
const USE_SINGLE: i32 = 1;
const USE_DUAL: i32 = 2;
const USE_QUAD: i32 = 4;

/// `EDGE_DIR`
pub(crate) const VERT_EDGE: i32 = 0;
pub(crate) const HORZ_EDGE: i32 = 1;

const SEG_LVL_LF_LUT: [[SegLvlFeatures; 2]; MAX_MB_PLANE] = [
    [SEG_LVL_ALT_LF_Y_V, SEG_LVL_ALT_LF_Y_H],
    [SEG_LVL_ALT_LF_U, SEG_LVL_ALT_LF_U],
    [SEG_LVL_ALT_LF_V, SEG_LVL_ALT_LF_V],
];

const MODE_LF_LUT: [usize; MB_MODE_COUNT as usize] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // INTRA_MODES
    1, 1, 0, 1, // INTER_MODES (GLOBALMV == 0)
    1, 1, 1, 1, 1, 1, 0, 1, // INTER_COMPOUND_MODES (GLOBAL_GLOBALMV == 0)
];

/// Translation of `av1_get_filter_level()` (no delta loop filter: the
/// real-time encoder codes no delta q).
#[inline]
pub(crate) fn av1_get_filter_level(
    cm: &Av1Common,
    lfi_n: &LoopFilterInfoN,
    dir_idx: i32,
    plane: usize,
    mbmi: &MbModeInfo,
) -> u8 {
    let segment_id = mbmi.segment_id as usize;
    debug_assert!(cm.delta_q_info.delta_lf_present_flag == 0);
    lfi_n.lvl[plane][segment_id][dir_idx as usize][mbmi.ref_frame[0].max(0) as usize]
        [MODE_LF_LUT[mbmi.mode as usize]]
}

// Update the loop filter for the current frame.
// This should be called before loop_filter_rows(),
// av1_loop_filter_frame() calls this function directly.
/// Translation of `av1_loop_filter_frame_init()`.
pub(crate) fn av1_loop_filter_frame_init(cm: &mut Av1Common, plane_start: usize, plane_end: usize) {
    let mut filt_lvl = [0i32; MAX_MB_PLANE];
    let mut filt_lvl_r = [0i32; MAX_MB_PLANE];
    // n_shift is the multiplier for lf_deltas
    // the multiplier is 1 for when filter_lvl is between 0 and 31;
    // 2 when filter_lvl is between 32 and 63
    let lf = cm.lf;
    let seg = &cm.seg;
    let lfi = &mut *cm.lf_info;

    // update sharpness limits
    update_sharpness(lfi, lf.sharpness_level);

    filt_lvl[0] = lf.filter_level[0];
    filt_lvl[1] = lf.filter_level_u;
    filt_lvl[2] = lf.filter_level_v;

    filt_lvl_r[0] = lf.filter_level[1];
    filt_lvl_r[1] = lf.filter_level_u;
    filt_lvl_r[2] = lf.filter_level_v;

    debug_assert!(plane_end <= MAX_MB_PLANE);

    for plane in plane_start..plane_end {
        if plane == 0 && filt_lvl[0] == 0 && filt_lvl_r[0] == 0 {
            break;
        } else if plane == 1 && filt_lvl[1] == 0 {
            continue;
        } else if plane == 2 && filt_lvl[2] == 0 {
            continue;
        }

        for seg_id in 0..MAX_SEGMENTS {
            for dir in 0..2 {
                let mut lvl_seg = if dir == 0 {
                    filt_lvl[plane]
                } else {
                    filt_lvl_r[plane]
                };
                let seg_lf_feature_id = SEG_LVL_LF_LUT[plane][dir];
                if segfeature_active(seg, seg_id as u8, seg_lf_feature_id) {
                    let data = get_segdata(seg, seg_id as i32, seg_lf_feature_id);
                    lvl_seg = (lvl_seg + data).clamp(0, MAX_LOOP_FILTER as i32);
                }

                if lf.mode_ref_delta_enabled == 0 {
                    // we could get rid of this if we assume that deltas are set to
                    // zero when not in use; encoder always uses deltas
                    for r in lfi.lvl[plane][seg_id][dir].iter_mut() {
                        *r = [lvl_seg as u8; MAX_MODE_LF_DELTAS as usize];
                    }
                } else {
                    let scale = 1 << (lvl_seg >> 5);
                    let intra_lvl = lvl_seg + lf.ref_deltas[INTRA_FRAME as usize] as i32 * scale;
                    lfi.lvl[plane][seg_id][dir][INTRA_FRAME as usize][0] =
                        intra_lvl.clamp(0, MAX_LOOP_FILTER as i32) as u8;

                    for r in LAST_FRAME as usize..REF_FRAMES as usize {
                        for mode in 0..MAX_MODE_LF_DELTAS as usize {
                            let inter_lvl = lvl_seg
                                + lf.ref_deltas[r] as i32 * scale
                                + lf.mode_deltas[mode] as i32 * scale;
                            lfi.lvl[plane][seg_id][dir][r][mode] =
                                inter_lvl.clamp(0, MAX_LOOP_FILTER as i32) as u8;
                        }
                    }
                }
            }
        }
    }
}

/// Translation of `get_transform_size()`.
#[inline]
fn get_transform_size(
    lossless: &[i32; MAX_SEGMENTS],
    mbmi: &MbModeInfo,
    mi_row: i32,
    mi_col: i32,
    plane: usize,
    ss_x: i32,
    ss_y: i32,
) -> TxSize {
    if lossless[mbmi.segment_id as usize] != 0 {
        return TX_4X4;
    }

    let mut tx_size = if plane == AOM_PLANE_Y as usize {
        mbmi.tx_size
    } else {
        av1_get_max_uv_txsize(mbmi.bsize, ss_x, ss_y)
    };
    debug_assert!(tx_size < TX_SIZES_ALL);
    if plane == AOM_PLANE_Y as usize && is_inter_block(mbmi) && mbmi.skip_txfm == 0 {
        let sb_type = mbmi.bsize;
        let blk_row = mi_row & (MI_SIZE_HIGH[sb_type as usize] as i32 - 1);
        let blk_col = mi_col & (MI_SIZE_WIDE[sb_type as usize] as i32 - 1);
        let mb_tx_size = mbmi.inter_tx_size[av1_get_txb_size_index(sb_type, blk_row, blk_col)];
        debug_assert!(mb_tx_size < TX_SIZES_ALL);
        tx_size = mb_tx_size;
    }

    tx_size
}

/// Translation of `av1_get_txb_size_index()` (blockd.h).
#[inline]
pub(crate) fn av1_get_txb_size_index(bsize: BlockSize, blk_row: i32, blk_col: i32) -> usize {
    const TW_W_LOG2_TABLE: [u8; BLOCK_SIZES_ALL as usize] = [
        0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 0, 1, 1, 2, 2, 3,
    ];
    const TW_H_LOG2_TABLE: [u8; BLOCK_SIZES_ALL as usize] = [
        0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 1, 0, 2, 1, 3, 2,
    ];
    const STRIDE_LOG2_TABLE: [u8; BLOCK_SIZES_ALL as usize] = [
        0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 1, 1, 1, 2, 2, 0, 1, 0, 1, 0, 1,
    ];
    let b = bsize as usize;
    let index =
        ((blk_row >> TW_H_LOG2_TABLE[b]) << STRIDE_LOG2_TABLE[b]) + (blk_col >> TW_W_LOG2_TABLE[b]);
    index as usize
}

const FL4: [u8; 19] = [4; 19];
const FL_L8V: [u8; 19] = [4, 8, 8, 8, 8, 4, 8, 8, 8, 8, 8, 8, 8, 4, 8, 8, 8, 8, 8];
const FL_L14V: [u8; 19] = [
    4, 8, 14, 14, 14, 4, 8, 8, 14, 14, 14, 14, 14, 4, 14, 8, 14, 14, 14,
];
const FL_L8H: [u8; 19] = [4, 8, 8, 8, 8, 8, 4, 8, 8, 8, 8, 8, 8, 8, 4, 8, 8, 8, 8];
const FL_L14H: [u8; 19] = [
    4, 8, 14, 14, 14, 8, 4, 14, 8, 14, 14, 14, 14, 14, 4, 14, 8, 14, 14,
];
const FL_C6V: [u8; 19] = [4, 6, 6, 6, 6, 4, 6, 6, 6, 6, 6, 6, 6, 4, 6, 6, 6, 6, 6];
const FL_C6H: [u8; 19] = [4, 6, 6, 6, 6, 6, 4, 6, 6, 6, 6, 6, 6, 6, 4, 6, 6, 6, 6];

// (the rows are TX_4X4, TX_8X8, TX_16X16, TX_32X32, TX_64X64, TX_4X8,
// TX_8X4, TX_8X16, TX_16X8, TX_16X32, TX_32X16, TX_32X64, TX_64X32,
// TX_4X16, TX_16X4, TX_8X32, TX_32X8, TX_16X64, TX_64X16)
/// `vert_filter_length_luma`
const VERT_FILTER_LENGTH_LUMA: [[u8; 19]; 19] = [
    FL4, FL_L8V, FL_L14V, FL_L14V, FL_L14V, FL4, FL_L8V, FL_L8V, FL_L14V, FL_L14V, FL_L14V,
    FL_L14V, FL_L14V, FL4, FL_L14V, FL_L8V, FL_L14V, FL_L14V, FL_L14V,
];

/// `horz_filter_length_luma`
const HORZ_FILTER_LENGTH_LUMA: [[u8; 19]; 19] = [
    FL4, FL_L8H, FL_L14H, FL_L14H, FL_L14H, FL_L8H, FL4, FL_L14H, FL_L8H, FL_L14H, FL_L14H,
    FL_L14H, FL_L14H, FL_L14H, FL4, FL_L14H, FL_L8H, FL_L14H, FL_L14H,
];

/// `vert_filter_length_chroma`
const VERT_FILTER_LENGTH_CHROMA: [[u8; 19]; 19] = [
    FL4, FL_C6V, FL_C6V, FL_C6V, FL_C6V, FL4, FL_C6V, FL_C6V, FL_C6V, FL_C6V, FL_C6V, FL_C6V,
    FL_C6V, FL4, FL_C6V, FL_C6V, FL_C6V, FL_C6V, FL_C6V,
];

/// `horz_filter_length_chroma`
const HORZ_FILTER_LENGTH_CHROMA: [[u8; 19]; 19] = [
    FL4, FL_C6H, FL_C6H, FL_C6H, FL_C6H, FL_C6H, FL4, FL_C6H, FL_C6H, FL_C6H, FL_C6H, FL_C6H,
    FL_C6H, FL_C6H, FL4, FL_C6H, FL_C6H, FL_C6H, FL_C6H,
];

/// `AV1_DEBLOCKING_PARAMETERS`: `lfthr` is the index into
/// `cm->lf_info.lfthr[]`.
#[derive(Debug, Clone, Copy, Default)]
struct DeblockingParams {
    // length of the filter applied to the outer edge
    filter_length: u8,
    // deblocking limits
    lfthr: usize,
}

/// The planes being filtered: `dst` is the plane's `struct buf_2d` (an
/// offset into the frame's samples).
#[derive(Debug, Clone, Copy, Default)]
struct LfPlane {
    dst: Buf2d,
    subsampling_x: i32,
    subsampling_y: i32,
}

/// The mode info grid and loop filter state the line setup reads.
struct LfCtx<'a> {
    cm: &'a Av1Common,
    mi_alloc: &'a [MbModeInfo],
    mi_grid: &'a [u32],
    lossless: &'a [i32; MAX_SEGMENTS],
}

impl LfCtx<'_> {
    #[inline]
    fn mi(&self, grid_idx: isize) -> Option<&MbModeInfo> {
        let i = *self.mi_grid.get(grid_idx as usize)?;
        if i == MI_NONE {
            None
        } else {
            self.mi_alloc.get(i as usize)
        }
    }
}

/// Translation of `set_one_param_for_line_luma()`; `mode_step` and the
/// grid positions are in grid entries.
#[inline]
fn set_one_param_for_line_luma(
    params: &mut DeblockingParams,
    tx_size: &mut TxSize,
    c: &LfCtx,
    edge_dir: i32,
    mi_col: u32,
    mi_row: u32,
    coord: u32,
    is_first_block: bool,
    prev_tx_size: TxSize,
    mode_step: isize,
    min_dim: &mut i32,
) -> Result<(), ()> {
    let cm = c.cm;
    let is_vert = edge_dir == VERT_EDGE;
    // reset to initial values
    params.filter_length = 0;

    let mi = mi_row as isize * cm.mi_params.mi_stride as isize + mi_col as isize;
    let mbmi_idx = c.mi_grid[mi as usize];
    let mbmi = c.mi(mi).ok_or(())?;

    let ts = get_transform_size(c.lossless, mbmi, mi_row as i32, mi_col as i32, 0, 0, 0);

    // If we are not the first block, then coord is always true, so
    // !is_first_block is technically redundant. But we are keeping it here so the
    // compiler can compile away this conditional if we pass in is_first_block :=
    // false
    let mut curr_skipped = false;
    if !is_first_block || coord != 0 {
        let mi_prev_idx = c.mi_grid[(mi - mode_step) as usize];
        let mi_prev = c.mi(mi - mode_step).ok_or(())?;
        let pv_row = if is_vert {
            mi_row as i32
        } else {
            mi_row as i32 - 1
        };
        let pv_col = if is_vert {
            mi_col as i32 - 1
        } else {
            mi_col as i32
        };
        let pv_ts = if is_first_block {
            get_transform_size(c.lossless, mi_prev, pv_row, pv_col, 0, 0, 0)
        } else {
            prev_tx_size
        };
        if is_first_block {
            *min_dim = if is_vert {
                BLOCK_SIZE_HIGH[mi_prev.bsize as usize] as i32
            } else {
                BLOCK_SIZE_WIDE[mi_prev.bsize as usize] as i32
            };
        }
        let mut level = av1_get_filter_level(cm, &cm.lf_info, edge_dir, 0, mbmi);
        if level == 0 {
            level = av1_get_filter_level(cm, &cm.lf_info, edge_dir, 0, mi_prev);
        }

        let pu_edge = mi_prev_idx != mbmi_idx;

        // The quad loop filter assumes that all the transform blocks within a
        // 8x16/16x8/16x16 prediction block are of the same size.

        if !pu_edge {
            curr_skipped = mbmi.skip_txfm != 0 && is_inter_block(mbmi);
        }
        if (pu_edge || !curr_skipped) && level != 0 {
            params.filter_length = if is_vert {
                VERT_FILTER_LENGTH_LUMA[ts as usize][pv_ts as usize]
            } else {
                HORZ_FILTER_LENGTH_LUMA[ts as usize][pv_ts as usize]
            };

            // prepare common parameters
            params.lfthr = level as usize;
        }
    }
    let block_dim = if is_vert {
        BLOCK_SIZE_HIGH[mbmi.bsize as usize] as i32
    } else {
        BLOCK_SIZE_WIDE[mbmi.bsize as usize] as i32
    };
    *min_dim = (*min_dim).min(block_dim);

    *tx_size = ts;
    Ok(())
}

// Similar to set_lpf_parameters, but does so one row/col at a time to reduce
// calls to \ref get_transform_size and \ref av1_get_filter_level
/// Translation of `set_lpf_parameters_for_line_luma()`.
fn set_lpf_parameters_for_line_luma(
    params_buf: &mut [DeblockingParams; MAX_MIB_SIZE as usize],
    tx_buf: &mut [TxSize; MAX_MIB_SIZE as usize],
    c: &LfCtx,
    edge_dir: i32,
    mut mi_col: u32,
    mut mi_row: u32,
    mi_range: u32,
    mode_step: isize,
    min_dim: &mut i32,
) -> Result<(), ()> {
    let is_vert = edge_dir == VERT_EDGE;

    let mut i = 0usize;
    let mut prev_tx_size: TxSize = TX_INVALID;
    let mut first = true;

    // (the first iteration of upstream's unrolled loop is the one with
    // is_first_block set)
    loop {
        let coord = if is_vert { mi_col } else { mi_row };
        if !first && coord >= mi_range {
            break;
        }
        if i >= MAX_MIB_SIZE as usize {
            return Err(());
        }
        set_one_param_for_line_luma(
            &mut params_buf[i],
            &mut tx_buf[i],
            c,
            edge_dir,
            mi_col,
            mi_row,
            coord,
            first,
            prev_tx_size,
            mode_step,
            min_dim,
        )?;
        first = false;

        // Advance
        let advance_units = if is_vert {
            TX_SIZE_WIDE_UNIT[tx_buf[i] as usize]
        } else {
            TX_SIZE_HIGH_UNIT[tx_buf[i] as usize]
        } as u32;
        prev_tx_size = tx_buf[i];
        if is_vert {
            mi_col += advance_units;
        } else {
            mi_row += advance_units;
        }
        i += advance_units as usize;
    }
    Ok(())
}

/// Translation of `set_one_param_for_line_chroma()`.
#[inline]
fn set_one_param_for_line_chroma(
    params: &mut DeblockingParams,
    tx_size: &mut TxSize,
    c: &LfCtx,
    edge_dir: i32,
    mut mi_col: u32,
    mut mi_row: u32,
    coord: u32,
    is_first_block: bool,
    prev_tx_size: TxSize,
    mode_step: isize,
    scale_horz: i32,
    scale_vert: i32,
    min_dim: &mut i32,
    plane: usize,
) -> Result<(), ()> {
    let cm = c.cm;
    let is_vert = edge_dir == VERT_EDGE;
    // reset to initial values
    params.filter_length = 0;

    // for sub8x8 block, chroma prediction mode is obtained from the
    // bottom/right mi structure of the co-located 8x8 luma block. so for chroma
    // plane, mi_row and mi_col should map to the bottom/right mi structure,
    // i.e, both mi_row and mi_col should be odd number for chroma plane.
    mi_row |= scale_vert as u32;
    mi_col |= scale_horz as u32;
    let mi = mi_row as isize * cm.mi_params.mi_stride as isize + mi_col as isize;
    let mbmi_idx = c.mi_grid[mi as usize];
    let mbmi = c.mi(mi).ok_or(())?;

    let ts = get_transform_size(
        c.lossless,
        mbmi,
        mi_row as i32,
        mi_col as i32,
        plane,
        scale_horz,
        scale_vert,
    );
    *tx_size = ts;

    // If we are not the first block, then coord is always true, so
    // !is_first_block is technically redundant. But we are keeping it here so the
    // compiler can compile away this conditional if we pass in is_first_block :=
    // false
    let mut curr_skipped = false;
    if !is_first_block || coord != 0 {
        let mi_prev_idx = c.mi_grid[(mi - mode_step) as usize];
        let mi_prev = c.mi(mi - mode_step).ok_or(())?;
        let pv_row = if is_vert {
            mi_row as i32
        } else {
            mi_row as i32 - (1 << scale_vert)
        };
        let pv_col = if is_vert {
            mi_col as i32 - (1 << scale_horz)
        } else {
            mi_col as i32
        };
        let pv_ts = if is_first_block {
            get_transform_size(
                c.lossless, mi_prev, pv_row, pv_col, plane, scale_horz, scale_vert,
            )
        } else {
            prev_tx_size
        };
        if is_first_block {
            *min_dim = if is_vert {
                TX_SIZE_HIGH[pv_ts as usize]
            } else {
                TX_SIZE_WIDE[pv_ts as usize]
            };
        }

        let mut level = av1_get_filter_level(cm, &cm.lf_info, edge_dir, plane, mbmi);
        if level == 0 {
            level = av1_get_filter_level(cm, &cm.lf_info, edge_dir, plane, mi_prev);
        }
        let pu_edge = mi_prev_idx != mbmi_idx;

        if !pu_edge {
            curr_skipped = mbmi.skip_txfm != 0 && is_inter_block(mbmi);
        }
        // For realtime mode, u and v have the same level
        if (!curr_skipped || pu_edge) && level != 0 {
            params.filter_length = if is_vert {
                VERT_FILTER_LENGTH_CHROMA[ts as usize][pv_ts as usize]
            } else {
                HORZ_FILTER_LENGTH_CHROMA[ts as usize][pv_ts as usize]
            };

            params.lfthr = level as usize;
        }
    }
    let tx_dim = if is_vert {
        TX_SIZE_HIGH[ts as usize]
    } else {
        TX_SIZE_WIDE[ts as usize]
    };
    *min_dim = (*min_dim).min(tx_dim);
    Ok(())
}

/// Translation of `set_lpf_parameters_for_line_chroma()`.
fn set_lpf_parameters_for_line_chroma(
    params_buf: &mut [DeblockingParams; MAX_MIB_SIZE as usize],
    tx_buf: &mut [TxSize; MAX_MIB_SIZE as usize],
    c: &LfCtx,
    edge_dir: i32,
    mut mi_col: u32,
    mut mi_row: u32,
    mi_range: u32,
    mode_step: isize,
    scale_horz: i32,
    scale_vert: i32,
    min_dim: &mut i32,
    plane: usize,
) -> Result<(), ()> {
    let is_vert = edge_dir == VERT_EDGE;

    let mut i = 0usize;
    let scale = if is_vert { scale_horz } else { scale_vert };
    let mut prev_tx_size: TxSize = TX_INVALID;
    let mut first = true;

    // (the first iteration of upstream's unrolled loop is the one with
    // is_first_block set)
    loop {
        let coord = if is_vert { mi_col } else { mi_row };
        if !first && coord >= mi_range {
            break;
        }
        if i >= MAX_MIB_SIZE as usize {
            return Err(());
        }
        set_one_param_for_line_chroma(
            &mut params_buf[i],
            &mut tx_buf[i],
            c,
            edge_dir,
            mi_col,
            mi_row,
            coord,
            first,
            prev_tx_size,
            mode_step,
            scale_horz,
            scale_vert,
            min_dim,
            plane,
        )?;
        first = false;

        // Advance
        let advance_units = if is_vert {
            TX_SIZE_WIDE_UNIT[tx_buf[i] as usize]
        } else {
            TX_SIZE_HIGH_UNIT[tx_buf[i] as usize]
        } as u32;
        prev_tx_size = tx_buf[i];
        if is_vert {
            mi_col += advance_units << scale;
        } else {
            mi_row += advance_units << scale;
        }
        i += advance_units as usize;
    }
    Ok(())
}

/// Translation of `filter_vert()` / `filter_horz()` and their chroma
/// versions (low and high bit depth, see aom_dsp/loopfilter.rs: the low
/// bit depth filters are the high bit depth ones at `bd` 8, and a quad
/// filter is two dual ones).
#[inline]
fn filter_edge(
    s: &mut [u16],
    dst: usize,
    dst_stride: i32,
    params: &DeblockingParams,
    lfi: &LoopFilterInfoN,
    bd: i32,
    vertical: bool,
    use_filter_type: i32,
) {
    let limits = &lfi.lfthr[params.lfthr];
    aom_lpf(
        s,
        dst,
        dst_stride,
        vertical,
        params.filter_length as i32,
        use_filter_type,
        limits.mblim[0],
        limits.lim[0],
        limits.hev_thr[0],
        bd,
    );
}

/// The `bd` the loop filters run at: 8 for the low bit depth functions.
#[inline]
fn lf_bd(cm: &Av1Common) -> i32 {
    if cm.seq_params.use_highbitdepth != 0 {
        cm.seq_params.bit_depth
    } else {
        8
    }
}

/// Translation of `av1_filter_block_plane_vert_opt()`.
fn av1_filter_block_plane_vert_opt(
    c: &LfCtx,
    s: &mut [u16],
    plane_ptr: &LfPlane,
    mi_row: u32,
    mi_col: u32,
    params_buf: &mut [DeblockingParams; MAX_MIB_SIZE as usize],
    tx_buf: &mut [TxSize; MAX_MIB_SIZE as usize],
    num_mis_in_lpf_unit_height_log2: i32,
) -> Result<(), ()> {
    let cm = c.cm;
    let bd = lf_bd(cm);
    let dst_ptr = plane_ptr.dst.buf;
    let dst_stride = plane_ptr.dst.stride;
    // Ensure that mi_cols/mi_rows are calculated based on frame dimension aligned
    // to MI_SIZE.
    let plane_mi_cols = (plane_ptr.dst.width + (1 << MI_SIZE_LOG2) - 1) >> MI_SIZE_LOG2;
    let plane_mi_rows = (plane_ptr.dst.height + (1 << MI_SIZE_LOG2) - 1) >> MI_SIZE_LOG2;
    // Whenever 'pipeline_lpf_mt_with_enc' is enabled, height of the unit to
    // filter (i.e., y_range) is calculated based on the size of the superblock
    // used.
    let y_range = (plane_mi_rows - mi_row as i32).min(1 << num_mis_in_lpf_unit_height_log2);
    // Width of the unit to filter (i.e., x_range) should always be calculated
    // based on maximum superblock size as this function is called for mi_col = 0,
    // MAX_MIB_SIZE, 2 * MAX_MIB_SIZE etc.
    let x_range = (plane_mi_cols - mi_col as i32).min(MAX_MIB_SIZE);
    let mode_step = 1;
    let mut y = 0;
    while y < y_range {
        let curr_y = mi_row + y as u32;
        let x_start = mi_col;
        let x_end = mi_col + x_range as u32;
        let mut min_block_height = BLOCK_SIZE_HIGH[BLOCK_128X128 as usize] as i32;
        set_lpf_parameters_for_line_luma(
            params_buf,
            tx_buf,
            c,
            VERT_EDGE,
            x_start,
            curr_y,
            x_end,
            mode_step,
            &mut min_block_height,
        )?;

        let mut i = 0usize;
        let mut use_filter_type = USE_SINGLE;

        let mut p = dst_ptr as isize + (y * MI_SIZE * dst_stride) as isize;

        if (y & 3) == 0 && (y + 3) < y_range && min_block_height >= 16 {
            // If we are on a row which is a multiple of 4, and the minimum height is
            // 16 pixels, then the current and right 3 cols must contain the same
            // prediction block. This is because dim 16 can only happen every unit of
            // 4 mi's.
            use_filter_type = USE_QUAD;
            y += 3;
        } else if (y + 1) < y_range && min_block_height >= 8 {
            use_filter_type = USE_DUAL;
            y += 1;
        }

        let mut x = 0;
        while x < x_range {
            if tx_buf[i] == TX_INVALID {
                params_buf[i].filter_length = 0;
                tx_buf[i] = TX_4X4;
            }

            filter_edge(
                s,
                p as usize,
                dst_stride,
                &params_buf[i],
                &cm.lf_info,
                bd,
                true,
                use_filter_type,
            );

            // advance the destination pointer
            let advance_units = TX_SIZE_WIDE_UNIT[tx_buf[i] as usize];
            x += advance_units;
            p += (advance_units * MI_SIZE) as isize;
            i += advance_units as usize;
        }
        y += 1;
    }
    Ok(())
}

/// Translation of `av1_filter_block_plane_vert_opt_chroma()` (with
/// `joint_filter_chroma`, `plane_ptr` holds U and V).
fn av1_filter_block_plane_vert_opt_chroma(
    c: &LfCtx,
    s: &mut [u16],
    plane_ptr: &[LfPlane],
    mi_row: u32,
    mi_col: u32,
    params_buf: &mut [DeblockingParams; MAX_MIB_SIZE as usize],
    tx_buf: &mut [TxSize; MAX_MIB_SIZE as usize],
    plane: usize,
    joint_filter_chroma: bool,
    num_mis_in_lpf_unit_height_log2: i32,
) -> Result<(), ()> {
    let cm = c.cm;
    let bd = lf_bd(cm);
    let scale_horz = plane_ptr[0].subsampling_x;
    let scale_vert = plane_ptr[0].subsampling_y;
    let dst_stride = plane_ptr[0].dst.stride;
    // Ensure that mi_cols/mi_rows are calculated based on frame dimension aligned
    // to MI_SIZE.
    let mi_cols = ((plane_ptr[0].dst.width << scale_horz) + MI_SIZE - 1) >> MI_SIZE_LOG2;
    let mi_rows = ((plane_ptr[0].dst.height << scale_vert) + MI_SIZE - 1) >> MI_SIZE_LOG2;
    let plane_mi_rows = (mi_rows + ((1 << scale_vert) >> 1)) >> scale_vert;
    let plane_mi_cols = (mi_cols + ((1 << scale_horz) >> 1)) >> scale_horz;
    let y_range = (plane_mi_rows - (mi_row as i32 >> scale_vert))
        .min((1 << num_mis_in_lpf_unit_height_log2) >> scale_vert);
    let x_range = (plane_mi_cols - (mi_col as i32 >> scale_horz)).min(MAX_MIB_SIZE >> scale_horz);
    let mode_step = 1isize << scale_horz;

    let mut y = 0;
    while y < y_range {
        let curr_y = mi_row + ((y as u32) << scale_vert);
        let x_start = mi_col;
        let x_end = mi_col + ((x_range as u32) << scale_horz);
        let mut min_height = TX_SIZE_HIGH[TX_64X64 as usize];
        set_lpf_parameters_for_line_chroma(
            params_buf,
            tx_buf,
            c,
            VERT_EDGE,
            x_start,
            curr_y,
            x_end,
            mode_step,
            scale_horz,
            scale_vert,
            &mut min_height,
            plane,
        )?;

        let mut i = 0usize;
        let mut use_filter_type = USE_SINGLE;
        let mut y_inc = 0;

        if (y & 3) == 0 && (y + 3) < y_range && min_height >= 16 {
            // If we are on a row which is a multiple of 4, and the minimum height is
            // 16 pixels, then the current and below 3 rows must contain the same tx
            // block. This is because dim 16 can only happen every unit of 4 mi's.
            use_filter_type = USE_QUAD;
            y_inc = 3;
        } else if y % 2 == 0 && (y + 1) < y_range && min_height >= 8 {
            // If we are on an even row, and the minimum height is 8 pixels, then the
            // current and below rows must contain the same tx block. This is because
            // dim 4 can only happen every unit of 2**0, and 8 every unit of 2**1,
            // etc.
            use_filter_type = USE_DUAL;
            y_inc = 1;
        }

        let mut x = 0;
        while x < x_range {
            // inner loop always filter vertical edges in a MI block. If MI size
            // is 8x8, it will filter the vertical edge aligned with a 8x8 block.
            // If 4x4 transform is used, it will then filter the internal edge
            // aligned with a 4x4 block
            if tx_buf[i] == TX_INVALID {
                params_buf[i].filter_length = 0;
                tx_buf[i] = TX_4X4;
            }

            let offset = (y * MI_SIZE * dst_stride + x * MI_SIZE) as isize;
            let lfi = &cm.lf_info;
            if joint_filter_chroma {
                let u_dst = (plane_ptr[0].dst.buf as isize + offset) as usize;
                let v_dst = (plane_ptr[1].dst.buf as isize + offset) as usize;
                filter_edge(
                    s,
                    u_dst,
                    dst_stride,
                    &params_buf[i],
                    lfi,
                    bd,
                    true,
                    use_filter_type,
                );
                filter_edge(
                    s,
                    v_dst,
                    dst_stride,
                    &params_buf[i],
                    lfi,
                    bd,
                    true,
                    use_filter_type,
                );
            } else {
                let dst_ptr = (plane_ptr[0].dst.buf as isize + offset) as usize;
                filter_edge(
                    s,
                    dst_ptr,
                    dst_stride,
                    &params_buf[i],
                    lfi,
                    bd,
                    true,
                    use_filter_type,
                );
            }

            // advance the destination pointer
            let advance_units = TX_SIZE_WIDE_UNIT[tx_buf[i] as usize];
            x += advance_units;
            i += advance_units as usize;
        }
        y += y_inc;
        y += 1;
    }
    Ok(())
}

/// Translation of `av1_filter_block_plane_horz_opt()`.
fn av1_filter_block_plane_horz_opt(
    c: &LfCtx,
    s: &mut [u16],
    plane_ptr: &LfPlane,
    mi_row: u32,
    mi_col: u32,
    params_buf: &mut [DeblockingParams; MAX_MIB_SIZE as usize],
    tx_buf: &mut [TxSize; MAX_MIB_SIZE as usize],
    num_mis_in_lpf_unit_height_log2: i32,
) -> Result<(), ()> {
    let cm = c.cm;
    let bd = lf_bd(cm);
    let dst_ptr = plane_ptr.dst.buf;
    let dst_stride = plane_ptr.dst.stride;
    // Ensure that mi_cols/mi_rows are calculated based on frame dimension aligned
    // to MI_SIZE.
    let plane_mi_cols = (plane_ptr.dst.width + (1 << MI_SIZE_LOG2) - 1) >> MI_SIZE_LOG2;
    let plane_mi_rows = (plane_ptr.dst.height + (1 << MI_SIZE_LOG2) - 1) >> MI_SIZE_LOG2;
    let y_range = (plane_mi_rows - mi_row as i32).min(1 << num_mis_in_lpf_unit_height_log2);
    let x_range = (plane_mi_cols - mi_col as i32).min(MAX_MIB_SIZE);

    let mode_step = cm.mi_params.mi_stride as isize;
    let mut x = 0;
    while x < x_range {
        let curr_x = mi_col + x as u32;
        let y_start = mi_row;
        let y_end = mi_row + y_range as u32;
        let mut min_block_width = BLOCK_SIZE_HIGH[BLOCK_128X128 as usize] as i32;
        set_lpf_parameters_for_line_luma(
            params_buf,
            tx_buf,
            c,
            HORZ_EDGE,
            curr_x,
            y_start,
            y_end,
            mode_step,
            &mut min_block_width,
        )?;

        let mut i = 0usize;
        let mut filter_type = USE_SINGLE;

        let mut p = dst_ptr as isize + (x * MI_SIZE) as isize;

        if (x & 3) == 0 && (x + 3) < x_range && min_block_width >= 16 {
            // If we are on a col which is a multiple of 4, and the minimum width is
            // 16 pixels, then the current and right 3 cols must contain the same
            // prediction block. This is because dim 16 can only happen every unit of
            // 4 mi's.
            filter_type = USE_QUAD;
            x += 3;
        } else if (x + 1) < x_range && min_block_width >= 8 {
            filter_type = USE_DUAL;
            x += 1;
        }

        let mut y = 0;
        while y < y_range {
            if tx_buf[i] == TX_INVALID {
                params_buf[i].filter_length = 0;
                tx_buf[i] = TX_4X4;
            }

            filter_edge(
                s,
                p as usize,
                dst_stride,
                &params_buf[i],
                &cm.lf_info,
                bd,
                false,
                filter_type,
            );

            // advance the destination pointer
            let advance_units = TX_SIZE_HIGH_UNIT[tx_buf[i] as usize];
            y += advance_units;
            p += (advance_units * dst_stride * MI_SIZE) as isize;
            i += advance_units as usize;
        }
        x += 1;
    }
    Ok(())
}

/// Translation of `av1_filter_block_plane_horz_opt_chroma()`.
fn av1_filter_block_plane_horz_opt_chroma(
    c: &LfCtx,
    s: &mut [u16],
    plane_ptr: &[LfPlane],
    mi_row: u32,
    mi_col: u32,
    params_buf: &mut [DeblockingParams; MAX_MIB_SIZE as usize],
    tx_buf: &mut [TxSize; MAX_MIB_SIZE as usize],
    plane: usize,
    joint_filter_chroma: bool,
    num_mis_in_lpf_unit_height_log2: i32,
) -> Result<(), ()> {
    let cm = c.cm;
    let bd = lf_bd(cm);
    let scale_horz = plane_ptr[0].subsampling_x;
    let scale_vert = plane_ptr[0].subsampling_y;
    let dst_stride = plane_ptr[0].dst.stride;
    // Ensure that mi_cols/mi_rows are calculated based on frame dimension aligned
    // to MI_SIZE.
    let mi_cols = ((plane_ptr[0].dst.width << scale_horz) + MI_SIZE - 1) >> MI_SIZE_LOG2;
    let mi_rows = ((plane_ptr[0].dst.height << scale_vert) + MI_SIZE - 1) >> MI_SIZE_LOG2;
    let plane_mi_rows = (mi_rows + ((1 << scale_vert) >> 1)) >> scale_vert;
    let plane_mi_cols = (mi_cols + ((1 << scale_horz) >> 1)) >> scale_horz;
    let y_range = (plane_mi_rows - (mi_row as i32 >> scale_vert))
        .min((1 << num_mis_in_lpf_unit_height_log2) >> scale_vert);
    let x_range = (plane_mi_cols - (mi_col as i32 >> scale_horz)).min(MAX_MIB_SIZE >> scale_horz);
    let mode_step = (cm.mi_params.mi_stride as isize) << scale_vert;
    let mut x = 0;
    while x < x_range {
        let y_start = mi_row;
        let curr_x = mi_col + ((x as u32) << scale_horz);
        let y_end = mi_row + ((y_range as u32) << scale_vert);
        let mut min_width = TX_SIZE_WIDE[TX_64X64 as usize];
        set_lpf_parameters_for_line_chroma(
            params_buf,
            tx_buf,
            c,
            HORZ_EDGE,
            curr_x,
            y_start,
            y_end,
            mode_step,
            scale_horz,
            scale_vert,
            &mut min_width,
            plane,
        )?;

        let mut i = 0usize;
        let mut use_filter_type = USE_SINGLE;
        let mut x_inc = 0;

        if (x & 3) == 0 && (x + 3) < x_range && min_width >= 16 {
            // If we are on a col which is a multiple of 4, and the minimum width is
            // 16 pixels, then the current and right 3 cols must contain the same tx
            // block. This is because dim 16 can only happen every unit of 4 mi's.
            use_filter_type = USE_QUAD;
            x_inc = 3;
        } else if x % 2 == 0 && (x + 1) < x_range && min_width >= 8 {
            // If we are on an even col, and the minimum width is 8 pixels, then the
            // current and left cols must contain the same tx block. This is because
            // dim 4 can only happen every unit of 2**0, and 8 every unit of 2**1,
            // etc.
            use_filter_type = USE_DUAL;
            x_inc = 1;
        }

        let mut y = 0;
        while y < y_range {
            // inner loop always filter vertical edges in a MI block. If MI size
            // is 8x8, it will first filter the vertical edge aligned with a 8x8
            // block. If 4x4 transform is used, it will then filter the internal
            // edge aligned with a 4x4 block
            if tx_buf[i] == TX_INVALID {
                params_buf[i].filter_length = 0;
                tx_buf[i] = TX_4X4;
            }

            let offset = (y * MI_SIZE * dst_stride + x * MI_SIZE) as isize;
            let lfi = &cm.lf_info;
            if joint_filter_chroma {
                let u_dst = (plane_ptr[0].dst.buf as isize + offset) as usize;
                let v_dst = (plane_ptr[1].dst.buf as isize + offset) as usize;
                filter_edge(
                    s,
                    u_dst,
                    dst_stride,
                    &params_buf[i],
                    lfi,
                    bd,
                    false,
                    use_filter_type,
                );
                filter_edge(
                    s,
                    v_dst,
                    dst_stride,
                    &params_buf[i],
                    lfi,
                    bd,
                    false,
                    use_filter_type,
                );
            } else {
                let dst_ptr = (plane_ptr[0].dst.buf as isize + offset) as usize;
                filter_edge(
                    s,
                    dst_ptr,
                    dst_stride,
                    &params_buf[i],
                    lfi,
                    bd,
                    false,
                    use_filter_type,
                );
            }

            // advance the destination pointer
            let advance_units = TX_SIZE_HIGH_UNIT[tx_buf[i] as usize];
            y += advance_units;
            i += advance_units as usize;
        }
        x += x_inc;
        x += 1;
    }
    Ok(())
}

/// Translation of `check_planes_to_loop_filter()` (thread_common.h).
#[inline]
fn check_planes_to_loop_filter(
    lf: &Loopfilter,
    planes_to_lf: &mut [bool; 3],
    plane_start: usize,
    plane_end: usize,
) -> bool {
    // For each luma and chroma plane, whether to filter it or not.
    planes_to_lf[0] =
        (lf.filter_level[0] != 0 || lf.filter_level[1] != 0) && plane_start == 0 && 0 < plane_end;
    planes_to_lf[1] = lf.filter_level_u != 0 && plane_start <= 1 && 1 < plane_end;
    planes_to_lf[2] = lf.filter_level_v != 0 && plane_start <= 2 && 2 < plane_end;
    // If the luma plane is purposely not filtered, neither are the chroma
    // planes.
    if !planes_to_lf[0] && plane_start == 0 && 0 < plane_end {
        return false;
    }
    // Early exit.
    if !planes_to_lf[0] && !planes_to_lf[1] && !planes_to_lf[2] {
        return false;
    }
    true
}

/// Translation of `skip_loop_filter_plane()` (thread_common.h).
#[inline]
fn skip_loop_filter_plane(planes_to_lf: &[bool; 3], plane: usize, lpf_opt_level: i32) -> bool {
    // If LPF_PICK_METHOD is LPF_PICK_FROM_Q, we have the option to filter both
    // chroma planes together
    if lpf_opt_level == 2 {
        if plane == AOM_PLANE_Y as usize {
            return !planes_to_lf[plane];
        }
        if plane == AOM_PLANE_U as usize {
            // U and V are handled together
            return !planes_to_lf[1] && !planes_to_lf[2];
        }
        // V is handled when u is filtered
        return true;
    }

    // Normal operation mode
    !planes_to_lf[plane]
}

/// Translation of `av1_thread_loop_filter_rows()` (no row
/// synchronization).
fn av1_thread_loop_filter_rows(
    c: &LfCtx,
    frame_buffer: &mut Yv12BufferConfig,
    planes: &mut [LfPlane; MAX_MB_PLANE],
    mi_row: i32,
    plane: usize,
    dir: i32,
    lpf_opt_level: i32,
    params_buf: &mut [DeblockingParams; MAX_MIB_SIZE as usize],
    tx_buf: &mut [TxSize; MAX_MIB_SIZE as usize],
    num_mis_in_lpf_unit_height_log2: i32,
) -> Result<(), ()> {
    let cm = c.cm;
    let joint_filter_chroma = lpf_opt_level == 2 && plane > AOM_PLANE_Y as usize;
    let num_planes = if joint_filter_chroma { 2 } else { 1 };

    // (the non-"opt" filters of lpf_opt_level 0 are not reached: the
    // real-time inter transform search depth is always 1)
    if lpf_opt_level == 0 {
        return Err(());
    }

    let sb_size = cm.seq_params.sb_size;
    let mut mi_col = 0;
    while mi_col < cm.mi_params.mi_cols {
        setup_lf_planes(
            planes,
            sb_size,
            frame_buffer,
            mi_row,
            mi_col,
            plane,
            plane + num_planes,
        );
        let s = &mut frame_buffer.buffer_alloc;
        if dir == 0 {
            if plane == AOM_PLANE_Y as usize {
                av1_filter_block_plane_vert_opt(
                    c,
                    s,
                    &planes[plane],
                    mi_row as u32,
                    mi_col as u32,
                    params_buf,
                    tx_buf,
                    num_mis_in_lpf_unit_height_log2,
                )?;
            } else {
                av1_filter_block_plane_vert_opt_chroma(
                    c,
                    s,
                    &planes[plane..],
                    mi_row as u32,
                    mi_col as u32,
                    params_buf,
                    tx_buf,
                    plane,
                    joint_filter_chroma,
                    num_mis_in_lpf_unit_height_log2,
                )?;
            }
        } else if plane == AOM_PLANE_Y as usize {
            av1_filter_block_plane_horz_opt(
                c,
                s,
                &planes[plane],
                mi_row as u32,
                mi_col as u32,
                params_buf,
                tx_buf,
                num_mis_in_lpf_unit_height_log2,
            )?;
        } else {
            av1_filter_block_plane_horz_opt_chroma(
                c,
                s,
                &planes[plane..],
                mi_row as u32,
                mi_col as u32,
                params_buf,
                tx_buf,
                plane,
                joint_filter_chroma,
                num_mis_in_lpf_unit_height_log2,
            )?;
        }
        mi_col += MAX_MIB_SIZE;
    }
    Ok(())
}

/// `av1_setup_dst_planes()` for the loop filter's planes.
fn setup_lf_planes(
    planes: &mut [LfPlane; MAX_MB_PLANE],
    bsize: BlockSize,
    src: &Yv12BufferConfig,
    mi_row: i32,
    mi_col: i32,
    plane_start: usize,
    plane_end: usize,
) {
    for i in plane_start..plane_end.min(MAX_MB_PLANE) {
        let pd = &mut planes[i];
        let is_uv = (i > 0) as usize;
        super::reconinter::setup_pred_plane(
            &mut pd.dst,
            bsize,
            src.buffers[i],
            src.crop_widths[is_uv],
            src.crop_heights[is_uv],
            src.strides[is_uv],
            mi_row,
            mi_col,
            None,
            pd.subsampling_x,
            pd.subsampling_y,
        );
    }
}

/// Translation of `av1_loop_filter_frame_mt()` with one worker, which
/// filters the rows in the main thread (`loop_filter_rows()`): the frame
/// is `cm->cur_frame->buf`, the mode info grid is the one the frame was
/// coded with. (The multi-threaded row filtering filters the same edges,
/// each after the edges its samples depend on, so it makes the same
/// frame.)
pub(crate) fn av1_loop_filter_frame(
    cm: &mut Av1Common,
    lossless: &[i32; MAX_SEGMENTS],
    plane_start: usize,
    plane_end: usize,
    partial_frame: bool,
    lpf_opt_level: i32,
) -> Result<(), ()> {
    let mut planes_to_lf = [false; 3];
    if !check_planes_to_loop_filter(&cm.lf, &mut planes_to_lf, plane_start, plane_end) {
        return Ok(());
    }

    let start_mi_row = 0;
    let mi_rows_to_filter = cm.mi_params.mi_rows;
    if partial_frame && cm.mi_params.mi_rows > 8 {
        // (partial frames are only filtered by the RD filter level search)
        return Err(());
    }
    let end_mi_row = start_mi_row + mi_rows_to_filter;
    av1_loop_filter_frame_init(cm, plane_start, plane_end);

    // Directly filter in the main thread.
    let mut frame = std::mem::take(&mut cm.cur_frame_mut().buf);
    let mut planes = [LfPlane::default(); MAX_MB_PLANE];
    for (i, p) in planes.iter_mut().enumerate() {
        let is_uv = i > 0;
        p.subsampling_x = if is_uv {
            cm.seq_params.subsampling_x
        } else {
            0
        };
        p.subsampling_y = if is_uv {
            cm.seq_params.subsampling_y
        } else {
            0
        };
    }
    let res = {
        let c = LfCtx {
            cm,
            mi_alloc: &cm.mi_params.mi_alloc,
            mi_grid: &cm.mi_params.mi_grid_base,
            lossless,
        };
        loop_filter_rows(
            &c,
            &mut frame,
            &mut planes,
            start_mi_row,
            end_mi_row,
            &planes_to_lf,
            lpf_opt_level,
        )
    };
    cm.cur_frame_mut().buf = frame;
    res
}

/// Translation of `loop_filter_rows()`.
fn loop_filter_rows(
    c: &LfCtx,
    frame: &mut Yv12BufferConfig,
    planes: &mut [LfPlane; MAX_MB_PLANE],
    start: i32,
    stop: i32,
    planes_to_lf: &[bool; 3],
    lpf_opt_level: i32,
) -> Result<(), ()> {
    // Filter top rows of all planes first, in case the output can be partially
    // reconstructed row by row.
    let mut params_buf = [DeblockingParams::default(); MAX_MIB_SIZE as usize];
    let mut tx_buf = [0 as TxSize; MAX_MIB_SIZE as usize];
    let mut mi_row = start;
    while mi_row < stop {
        for plane in 0..3 {
            if skip_loop_filter_plane(planes_to_lf, plane, lpf_opt_level) {
                continue;
            }

            for dir in 0..2 {
                av1_thread_loop_filter_rows(
                    c,
                    frame,
                    planes,
                    mi_row,
                    plane,
                    dir,
                    lpf_opt_level,
                    &mut params_buf,
                    &mut tx_buf,
                    MAX_MIB_SIZE_LOG2,
                )?;
            }
        }
        mi_row += MAX_MIB_SIZE;
    }
    Ok(())
}
