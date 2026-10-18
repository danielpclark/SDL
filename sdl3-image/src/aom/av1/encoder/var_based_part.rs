// Rust translation of the parts of av1/encoder/var_based_part.c (and the
// variance tree types of av1/encoder/encoder.h) from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the variance-based partitioning.
// Copyright (c) 2019, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Variance-based partitioning: a superblock's partition is chosen from the
//! variances of 4x4 averages of its source samples, before any mode search.

use super::block::*;
use super::encoder::{
    set_ref_ptrs, Av1Comp, ThreadData, ALLINTRA, AOM_CONTENT_SCREEN, CYCLIC_REFRESH_AQ,
};
use super::mcomp::av1_int_pro_motion_estimation;
use super::reconinter_enc::av1_enc_build_inter_predictor;
use crate::aom::aom_dsp::avg::{
    aom_avg_4x4, aom_avg_8x8, aom_avg_8x8_quad, aom_highbd_avg_4x4, aom_highbd_avg_8x8,
};
use crate::aom::aom_scale::yv12config::YV12_FLAG_HIGHBITDEPTH;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::{av1_broadcast_interp_filter, BILINEAR};
use crate::aom::av1::common::mv::Mv;
use crate::aom::av1::common::quant_common::av1_ac_quant_qtx;
use crate::aom::av1::common::reconinter::{av1_setup_pre_planes, setup_pred_plane};
use crate::aom::av1::common::tile_common::TileInfo;

// (var_based_part.h)

// Use increased thresholds for midres for speed 9 when qindex is above
// this threshold
/// `QINDEX_LARGE_BLOCK_THR`
pub(crate) const QINDEX_LARGE_BLOCK_THR: i32 = 100;

/// `CALC_CHROMA_THRESH_FOR_ZEROMV_SKIP()`
#[inline]
pub(crate) const fn calc_chroma_thresh_for_zeromv_skip(thresh_exit_part: u32) -> u32 {
    (3 * thresh_exit_part) >> 2
}

// (aq_cyclicrefresh.h)
/// `CR_SEGMENT_ID_BASE`
pub(crate) const CR_SEGMENT_ID_BASE: i32 = 0;

// (av1_noise_estimate.h: NOISE_LEVEL)
const K_LOW: i32 = 2;
const K_MEDIUM: i32 = 3;

// (encoder.h)

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VPartVar {
    // TODO(kyslov): consider changing to 64bit

    // This struct is used for computing variance in choose_partitioning(), where
    // the max number of samples within a superblock is 32x32 (with 4x4 avg).
    // With 8bit bitdepth, uint32_t is enough for sum_square_error (2^8 * 2^8 * 32
    // * 32 = 2^26). For high bitdepth we need to consider changing this to 64 bit
    pub(crate) sum_square_error: u32,
    pub(crate) sum_error: i32,
    pub(crate) log2_count: i32,
    pub(crate) variance: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VPVariance {
    pub(crate) none: VPartVar,
    pub(crate) horz: [VPartVar; 2],
    pub(crate) vert: [VPartVar; 2],
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VP4x4 {
    pub(crate) part_variances: VPVariance,
    pub(crate) split: [VPartVar; 4],
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VP8x8 {
    pub(crate) part_variances: VPVariance,
    pub(crate) split: [VP4x4; 4],
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VP16x16 {
    pub(crate) part_variances: VPVariance,
    pub(crate) split: [VP8x8; 4],
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VP32x32 {
    pub(crate) part_variances: VPVariance,
    pub(crate) split: [VP16x16; 4],
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VP64x64 {
    pub(crate) part_variances: VPVariance,
    pub(crate) split: [VP32x32; 4],
}

/// Translation of `VP128x128`: `split` is `td->vt64x64`, borrowed.
pub(crate) struct VP128x128<'a> {
    pub(crate) part_variances: VPVariance,
    pub(crate) split: &'a mut [VP64x64; 4],
}

// (var_based_part.c)

// Possible values for the force_split variable while evaluating variance based
// partitioning.
// (PART_EVAL_STATUS)
// Evaluate all partition types
const PART_EVAL_ALL: i32 = 0;
// Force PARTITION_SPLIT
const PART_EVAL_ONLY_SPLIT: i32 = 1;
// Force PARTITION_NONE
const PART_EVAL_ONLY_NONE: i32 = 2;

/// `tree_to_node()`: the node of a variance tree of any size (the
/// `variance_node` of upstream's `void *data`).
trait VarTree {
    fn part_variances(&mut self) -> &mut VPVariance;
    fn split_none(&self, i: usize) -> VPartVar;
}

macro_rules! var_tree {
    ($t:ty) => {
        impl VarTree for $t {
            fn part_variances(&mut self) -> &mut VPVariance {
                &mut self.part_variances
            }
            fn split_none(&self, i: usize) -> VPartVar {
                self.split[i].part_variances.none
            }
        }
    };
}
var_tree!(VP8x8);
var_tree!(VP16x16);
var_tree!(VP32x32);
var_tree!(VP64x64);
var_tree!(VP128x128<'_>);

impl VarTree for VP4x4 {
    fn part_variances(&mut self) -> &mut VPVariance {
        &mut self.part_variances
    }
    fn split_none(&self, i: usize) -> VPartVar {
        self.split[i]
    }
}

// Set variance values given sum square error, sum error, count.
#[inline]
fn fill_variance(s2: u32, s: i32, c: i32, v: &mut VPartVar) {
    v.sum_square_error = s2;
    v.sum_error = s;
    v.log2_count = c;
}

#[inline]
fn get_variance(v: &mut VPartVar) {
    // (the unsigned arithmetic of upstream's expression wraps)
    let mean_sq = (((v.sum_error as i64) * (v.sum_error as i64)) >> v.log2_count) as u32;
    v.variance =
        (256u32.wrapping_mul(v.sum_square_error.wrapping_sub(mean_sq)) >> v.log2_count) as i32;
}

#[inline]
fn sum_2_variances(a: &VPartVar, b: &VPartVar, r: &mut VPartVar) {
    debug_assert!(a.log2_count == b.log2_count);
    fill_variance(
        a.sum_square_error.wrapping_add(b.sum_square_error),
        a.sum_error.wrapping_add(b.sum_error),
        a.log2_count + 1,
        r,
    );
}

#[inline]
fn fill_variance_tree<T: VarTree>(data: &mut T) {
    let split = [
        data.split_none(0),
        data.split_none(1),
        data.split_none(2),
        data.split_none(3),
    ];
    let pv = data.part_variances();
    sum_2_variances(&split[0], &split[1], &mut pv.horz[0]);
    sum_2_variances(&split[2], &split[3], &mut pv.horz[1]);
    sum_2_variances(&split[0], &split[2], &mut pv.vert[0]);
    sum_2_variances(&split[1], &split[3], &mut pv.vert[1]);
    let (v0, v1) = (pv.vert[0], pv.vert[1]);
    sum_2_variances(&v0, &v1, &mut pv.none);
}

/// Translation of `set_block_size()`: the mode info grid is the one moved
/// into `xd`.
#[inline]
fn set_block_size(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) {
    if cm.mi_params.mi_cols > mi_col && cm.mi_params.mi_rows > mi_row {
        let mi_params = &cm.mi_params;
        let mi_grid_idx = get_mi_grid_idx(mi_params, mi_row, mi_col) as usize;
        let mi_alloc_idx = get_alloc_mi_idx(mi_params, mi_row, mi_col) as usize;
        xd.mi_grid_base[mi_grid_idx] = mi_alloc_idx as u32;
        xd.mi_alloc[mi_alloc_idx].bsize = bsize;
    }
}

fn set_vt_partitioning<T: VarTree>(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    tile: &TileInfo,
    vt: &mut T,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    threshold: i64,
    bsize_min: BlockSize,
    force_split: i32,
) -> bool {
    let block_width = MI_SIZE_WIDE[bsize as usize] as i32;
    let block_height = MI_SIZE_HIGH[bsize as usize] as i32;
    let mut bs_width_check = block_width;
    let mut bs_height_check = block_height;
    let mut bs_width_vert_check = block_width >> 1;
    let mut bs_height_horiz_check = block_height >> 1;
    // On the right and bottom boundary we only need to check
    // if half the bsize fits, because boundary is extended
    // up to 64. So do this check only for sb_size = 64X64.
    if cm.seq_params.sb_size == BLOCK_64X64 {
        if tile.mi_col_end == cm.mi_params.mi_cols {
            bs_width_check = (block_width >> 1) + 1;
            bs_width_vert_check = (block_width >> 2) + 1;
        }
        if tile.mi_row_end == cm.mi_params.mi_rows {
            bs_height_check = (block_height >> 1) + 1;
            bs_height_horiz_check = (block_height >> 2) + 1;
        }
    }

    debug_assert!(block_height == block_width);

    if mi_col + bs_width_check <= tile.mi_col_end
        && mi_row + bs_height_check <= tile.mi_row_end
        && force_split == PART_EVAL_ONLY_NONE
    {
        set_block_size(cm, xd, mi_row, mi_col, bsize);
        return true;
    }
    if force_split == PART_EVAL_ONLY_SPLIT {
        return false;
    }

    // For bsize=bsize_min (16x16/8x8 for 8x8/4x4 downsampling), select if
    // variance is below threshold, otherwise split will be selected.
    // No check for vert/horiz split as too few samples for variance.
    let pv = vt.part_variances();
    if bsize == bsize_min {
        // Variance already computed to set the force_split.
        if frame_is_intra_only(cm) {
            get_variance(&mut pv.none);
        }
        if mi_col + bs_width_check <= tile.mi_col_end
            && mi_row + bs_height_check <= tile.mi_row_end
            && (pv.none.variance as i64) < threshold
        {
            set_block_size(cm, xd, mi_row, mi_col, bsize);
            return true;
        }
        return false;
    } else if bsize > bsize_min {
        // Variance already computed to set the force_split.
        if frame_is_intra_only(cm) {
            get_variance(&mut pv.none);
        }
        // For key frame: take split for bsize above 32X32 or very high variance.
        if frame_is_intra_only(cm)
            && (bsize > BLOCK_32X32 || pv.none.variance as i64 > (threshold << 4))
        {
            return false;
        }
        // If variance is low, take the bsize (no split).
        if mi_col + bs_width_check <= tile.mi_col_end
            && mi_row + bs_height_check <= tile.mi_row_end
            && (pv.none.variance as i64) < threshold
        {
            set_block_size(cm, xd, mi_row, mi_col, bsize);
            return true;
        }
        // Check vertical split.
        if mi_row + bs_height_check <= tile.mi_row_end
            && mi_col + bs_width_vert_check <= tile.mi_col_end
        {
            let subsize = get_partition_subsize(bsize, PARTITION_VERT);
            get_variance(&mut pv.vert[0]);
            get_variance(&mut pv.vert[1]);
            if (pv.vert[0].variance as i64) < threshold
                && (pv.vert[1].variance as i64) < threshold
                && get_plane_block_size(
                    subsize,
                    xd.plane[1].subsampling_x,
                    xd.plane[1].subsampling_y,
                ) < BLOCK_INVALID
            {
                set_block_size(cm, xd, mi_row, mi_col, subsize);
                set_block_size(cm, xd, mi_row, mi_col + block_width / 2, subsize);
                return true;
            }
        }
        // Check horizontal split.
        if mi_col + bs_width_check <= tile.mi_col_end
            && mi_row + bs_height_horiz_check <= tile.mi_row_end
        {
            let subsize = get_partition_subsize(bsize, PARTITION_HORZ);
            get_variance(&mut pv.horz[0]);
            get_variance(&mut pv.horz[1]);
            if (pv.horz[0].variance as i64) < threshold
                && (pv.horz[1].variance as i64) < threshold
                && get_plane_block_size(
                    subsize,
                    xd.plane[1].subsampling_x,
                    xd.plane[1].subsampling_y,
                ) < BLOCK_INVALID
            {
                set_block_size(cm, xd, mi_row, mi_col, subsize);
                set_block_size(cm, xd, mi_row + block_height / 2, mi_col, subsize);
                return true;
            }
        }
        return false;
    }
    false
}

/// A block's samples in a frame buffer (upstream's pointer and stride
/// pairs): the buffer, the offset of the block's first sample and the
/// stride.
#[derive(Clone, Copy)]
struct Samples<'a> {
    buf: &'a [u16],
    off: usize,
    stride: i32,
}

impl Samples<'_> {
    /// The offset of the sample at (`x`, `y`) of the block.
    #[inline]
    fn at(&self, x: i32, y: i32) -> usize {
        (self.off as i64 + (y as i64 * self.stride as i64) + x as i64) as usize
    }
}

#[inline]
fn all_blks_inside(x16_idx: i32, y16_idx: i32, pixels_wide: i32, pixels_high: i32) -> bool {
    let mut all_inside = true;
    for k in 0..4 {
        all_inside &= (x16_idx + ((k & 1) << 3)) < pixels_wide;
        all_inside &= (y16_idx + ((k >> 1) << 3)) < pixels_high;
    }
    all_inside
}

// TODO(yunqingwang): Perform average of four 8x8 blocks similar to lowbd
#[inline]
fn fill_variance_8x8avg_highbd(
    s: Samples,
    d: Samples,
    x16_idx: i32,
    y16_idx: i32,
    vst: &mut VP16x16,
    pixels_wide: i32,
    pixels_high: i32,
    is_key_frame: bool,
) {
    for k in 0..4 {
        let x8_idx = x16_idx + ((k & 1) << 3);
        let y8_idx = y16_idx + ((k >> 1) << 3);
        let mut sse: u32 = 0;
        let mut sum: i32 = 0;
        if x8_idx < pixels_wide && y8_idx < pixels_high {
            let mut d_avg: i32 = 128;
            let s_avg = aom_highbd_avg_8x8(s.buf, s.at(x8_idx, y8_idx), s.stride as usize) as i32;
            if !is_key_frame {
                d_avg = aom_highbd_avg_8x8(d.buf, d.at(x8_idx, y8_idx), d.stride as usize) as i32;
            }

            sum = s_avg - d_avg;
            sse = sum.wrapping_mul(sum) as u32;
        }
        fill_variance(sse, sum, 0, &mut vst.split[k as usize].part_variances.none);
    }
}

#[inline]
fn fill_variance_8x8avg_lowbd(
    s: Samples,
    d: Samples,
    x16_idx: i32,
    y16_idx: i32,
    vst: &mut VP16x16,
    pixels_wide: i32,
    pixels_high: i32,
    is_key_frame: bool,
) {
    let mut sse = [0u32; 4];
    let mut sum = [0i32; 4];
    let mut d_avg = [128i32; 4];
    let mut s_avg = [0i32; 4];

    if all_blks_inside(x16_idx, y16_idx, pixels_wide, pixels_high) {
        aom_avg_8x8_quad(
            s.buf,
            s.off,
            s.stride as usize,
            x16_idx,
            y16_idx,
            &mut s_avg,
        );
        if !is_key_frame {
            aom_avg_8x8_quad(
                d.buf,
                d.off,
                d.stride as usize,
                x16_idx,
                y16_idx,
                &mut d_avg,
            );
        }
        for k in 0..4 {
            sum[k] = s_avg[k] - d_avg[k];
            sse[k] = sum[k].wrapping_mul(sum[k]) as u32;
        }
    } else {
        for k in 0..4 {
            let x8_idx = x16_idx + ((k as i32 & 1) << 3);
            let y8_idx = y16_idx + ((k as i32 >> 1) << 3);
            if x8_idx < pixels_wide && y8_idx < pixels_high {
                s_avg[k] = aom_avg_8x8(s.buf, s.at(x8_idx, y8_idx), s.stride as usize) as i32;
                if !is_key_frame {
                    d_avg[k] = aom_avg_8x8(d.buf, d.at(x8_idx, y8_idx), d.stride as usize) as i32;
                }
                sum[k] = s_avg[k] - d_avg[k];
                sse[k] = sum[k].wrapping_mul(sum[k]) as u32;
            }
        }
    }

    for k in 0..4 {
        fill_variance(sse[k], sum[k], 0, &mut vst.split[k].part_variances.none);
    }
}

// Obtain parameters required to calculate variance (such as sum, sse, etc,.)
// at 8x8 sub-block level for a given 16x16 block.
#[inline]
fn fill_variance_8x8avg(
    s: Samples,
    d: Samples,
    x16_idx: i32,
    y16_idx: i32,
    vst: &mut VP16x16,
    highbd_flag: bool,
    pixels_wide: i32,
    pixels_high: i32,
    is_key_frame: bool,
) {
    if highbd_flag {
        fill_variance_8x8avg_highbd(
            s,
            d,
            x16_idx,
            y16_idx,
            vst,
            pixels_wide,
            pixels_high,
            is_key_frame,
        );
        return;
    }
    fill_variance_8x8avg_lowbd(
        s,
        d,
        x16_idx,
        y16_idx,
        vst,
        pixels_wide,
        pixels_high,
        is_key_frame,
    );
}

#[inline]
fn fill_variance_4x4avg(
    s: Samples,
    d: Samples,
    x8_idx: i32,
    y8_idx: i32,
    vst: &mut VP8x8,
    highbd_flag: i32,
    pixels_wide: i32,
    pixels_high: i32,
    is_key_frame: bool,
    border_offset_4x4: i32,
) {
    for k in 0..4 {
        let x4_idx = x8_idx + ((k & 1) << 2);
        let y4_idx = y8_idx + ((k >> 1) << 2);
        let mut sse: u32 = 0;
        let mut sum: i32 = 0;
        if x4_idx < pixels_wide - border_offset_4x4 && y4_idx < pixels_high - border_offset_4x4 {
            let s_avg: i32;
            let mut d_avg: i32 = 128;
            if highbd_flag & YV12_FLAG_HIGHBITDEPTH != 0 {
                s_avg = aom_highbd_avg_4x4(s.buf, s.at(x4_idx, y4_idx), s.stride as usize) as i32;
                if !is_key_frame {
                    d_avg =
                        aom_highbd_avg_4x4(d.buf, d.at(x4_idx, y4_idx), d.stride as usize) as i32;
                }
            } else {
                s_avg = aom_avg_4x4(s.buf, s.at(x4_idx, y4_idx), s.stride as usize) as i32;
                if !is_key_frame {
                    d_avg = aom_avg_4x4(d.buf, d.at(x4_idx, y4_idx), d.stride as usize) as i32;
                }
            }

            sum = s_avg - d_avg;
            sse = sum.wrapping_mul(sum) as u32;
        }
        fill_variance(sse, sum, 0, &mut vst.split[k as usize].part_variances.none);
    }
}

// TODO(kyslov) Bring back threshold adjustment based on content state
fn scale_part_thresh_content(
    threshold_base: i64,
    speed: i32,
    _width: i32,
    _height: i32,
    non_reference_frame: bool,
) -> i64 {
    let mut threshold = threshold_base;
    if non_reference_frame {
        threshold = (3 * threshold) >> 1;
    }
    if speed >= 8 {
        return (5 * threshold) >> 2;
    }
    threshold
}

#[inline]
fn tune_thresh_based_on_qindex_window(
    qindex: i32,
    th: i32,
    win: i32,
    fac: i32,
    thresholds: &mut [i64; 5],
) {
    let weight: f64 = if qindex < th - win {
        1.0
    } else if qindex > th + win {
        0.0
    } else {
        // (integer division, as upstream's)
        1.0 - ((qindex - th + win) / (2 * win)) as f64
    };
    thresholds[1] = ((1.0 - weight) * (thresholds[1] << 1) as f64 + weight * thresholds[1] as f64)
        as i32 as i64;
    thresholds[2] = ((1.0 - weight) * (thresholds[2] << 1) as f64 + weight * thresholds[2] as f64)
        as i32 as i64;
    thresholds[3] = ((1.0 - weight) * (thresholds[3] << fac) as f64 + weight * thresholds[3] as f64)
        as i32 as i64;
}

/// Translation of `set_vbp_thresholds()` (the noise estimate is never
/// enabled: `av1_update_noise_estimate()` is not reached, so
/// `content_lowsumdiff` does not matter).
fn set_vbp_thresholds(
    cpi: &Av1Comp,
    thresholds: &mut [i64; 5],
    q: i32,
    _content_lowsumdiff: i32,
    source_sad_nonrd: i32,
    source_sad_rd: i32,
    segment_id: i32,
    blk_sad: u64,
    lighting_change: i32,
) {
    let cm = &cpi.common;
    let is_key_frame = frame_is_intra_only(cm);
    let threshold_multiplier = if is_key_frame { 120 } else { 1 };
    let ac_q = av1_ac_quant_qtx(q, 0, cm.seq_params.bit_depth) as i32;
    let mut threshold_base: i64 = (threshold_multiplier * ac_q) as i64;
    let current_qindex = cm.quant_params.base_qindex;
    let threshold_left_shift = cpi.sf.rt_sf.var_part_split_threshold_shift;

    if is_key_frame {
        if cpi.sf.rt_sf.force_large_partition_blocks_intra != 0 {
            let shift_steps = threshold_left_shift - if cpi.oxcf.mode == ALLINTRA { 7 } else { 8 };
            debug_assert!(shift_steps >= 0);
            threshold_base <<= shift_steps;
        }
        thresholds[0] = threshold_base;
        thresholds[1] = threshold_base;
        if cm.width * cm.height < 1280 * 720 {
            thresholds[2] = threshold_base / 3;
            thresholds[3] = threshold_base >> 1;
        } else {
            let mut shift_val = 2;
            if cpi.sf.rt_sf.force_large_partition_blocks_intra != 0 {
                shift_val = 0;
            }

            thresholds[2] = threshold_base >> shift_val;
            thresholds[3] = threshold_base >> shift_val;
        }
        thresholds[4] = threshold_base << 2;
        return;
    }

    // Increase partition thresholds for noisy content. Apply it only for
    // superblocks where sumdiff is low, as we assume the sumdiff of superblock
    // whose only change is due to noise will be low (i.e, noise will average
    // out over large block).
    // (cpi->noise_estimate.enabled is 0)
    // TODO(kyslov) Enable var based partition adjusment on temporal denoising
    // Increase base variance threshold based on content_state/sum_diff level.
    threshold_base = scale_part_thresh_content(
        threshold_base,
        cpi.oxcf.speed,
        cm.width,
        cm.height,
        cpi.ppi.rtc_ref.non_reference_frame,
    );
    thresholds[0] = threshold_base >> 1;
    thresholds[1] = threshold_base;
    thresholds[3] = threshold_base << threshold_left_shift;
    if cm.width >= 1280 && cm.height >= 720 {
        thresholds[3] <<= 1;
    }
    if cm.width * cm.height <= 352 * 288 {
        const QINDEX_THR: [[i32; 2]; 5] =
            [[200, 220], [140, 170], [120, 150], [200, 210], [170, 220]];
        let mut th_idx = 0usize;
        if cpi.sf.rt_sf.var_part_based_on_qidx >= 1 {
            th_idx = if source_sad_rd <= K_LOW_SAD {
                cpi.sf.rt_sf.var_part_based_on_qidx as usize
            } else {
                0
            };
        }
        if cpi.sf.rt_sf.var_part_based_on_qidx >= 3 {
            th_idx = cpi.sf.rt_sf.var_part_based_on_qidx as usize;
        }
        let qindex_low_thr = QINDEX_THR[th_idx][0];
        let qindex_high_thr = QINDEX_THR[th_idx][1];
        if current_qindex >= qindex_high_thr {
            threshold_base = (5 * threshold_base) >> 1;
            thresholds[1] = threshold_base >> 3;
            thresholds[2] = threshold_base << 2;
            thresholds[3] = threshold_base << 5;
        } else if current_qindex < qindex_low_thr {
            thresholds[1] = threshold_base >> 3;
            thresholds[2] = threshold_base >> 1;
            thresholds[3] = threshold_base << 3;
        } else {
            let qi_diff_low = (current_qindex - qindex_low_thr) as i64;
            let qi_diff_high = (qindex_high_thr - current_qindex) as i64;
            let mut threshold_diff = (qindex_high_thr - qindex_low_thr) as i64;
            let threshold_base_high = (5 * threshold_base) >> 1;

            threshold_diff = if threshold_diff > 0 {
                threshold_diff
            } else {
                1
            };
            threshold_base = (qi_diff_low * threshold_base_high + qi_diff_high * threshold_base)
                / threshold_diff;
            thresholds[1] = threshold_base >> 3;
            thresholds[2] = ((qi_diff_low * threshold_base) + qi_diff_high * (threshold_base >> 1))
                / threshold_diff;
            thresholds[3] = ((qi_diff_low * (threshold_base << 5))
                + qi_diff_high * (threshold_base << 3))
                / threshold_diff;
        }
    } else if cm.width < 1280 && cm.height < 720 {
        thresholds[2] = (5 * threshold_base) >> 2;
    } else if cm.width < 1920 && cm.height < 1080 {
        thresholds[2] = threshold_base << 1;
    } else if cm.width < 2560 && cm.height < 1440 {
        thresholds[2] = (5 * threshold_base) >> 1;
    } else {
        thresholds[2] = (7 * threshold_base) >> 1;
    }
    // Tune thresholds less or more aggressively to prefer larger partitions
    if cpi.sf.rt_sf.prefer_large_partition_blocks >= 3 {
        let win = 20;
        let weight: f64 = if current_qindex < QINDEX_LARGE_BLOCK_THR - win {
            1.0
        } else if current_qindex > QINDEX_LARGE_BLOCK_THR + win {
            0.0
        } else {
            // (integer division, as upstream's)
            1.0 - ((current_qindex - QINDEX_LARGE_BLOCK_THR + win) / (2 * win)) as f64
        };
        if cm.width * cm.height > 640 * 480 {
            for t in thresholds.iter_mut().take(4) {
                *t <<= 1;
            }
        }
        if cm.width * cm.height <= 352 * 288 {
            thresholds[3] = i64::MAX;
            if segment_id == 0 {
                thresholds[1] <<= 2;
                thresholds[2] <<= if source_sad_nonrd <= K_LOW_SAD { 5 } else { 4 };
            } else {
                thresholds[1] <<= 1;
                thresholds[2] <<= 3;
            }
            // Allow for split to 8x8 for superblocks where part of it has
            // moving boundary. So allow for sb with source_sad above threshold,
            // and avoid very large source_sad or high source content, to avoid
            // too many 8x8 within superblock.
            if segment_id == 0
                && cpi.rc.avg_source_sad < 25000
                && blk_sad > 25000
                && blk_sad < 50000
                && lighting_change == 0
            {
                thresholds[2] = (3 * thresholds[2]) >> 2;
                thresholds[3] = thresholds[2] << 3;
            }
            // Condition the increase of partition thresholds on the segment
            // and the content. Avoid the increase for superblocks which have
            // high source sad, unless the whole frame has very high motion
            // (i.e, cpi->rc.avg_source_sad is very large, in which case all blocks
            // have high source sad).
        } else if cm.width * cm.height > 640 * 480
            && segment_id == 0
            && (source_sad_nonrd != K_HIGH_SAD || cpi.rc.avg_source_sad > 50000)
        {
            thresholds[0] = (3 * thresholds[0]) >> 1;
            thresholds[3] = i64::MAX;
            if current_qindex > QINDEX_LARGE_BLOCK_THR {
                thresholds[1] = ((1.0 - weight) * (thresholds[1] << 1) as f64
                    + weight * thresholds[1] as f64) as i32 as i64;
                thresholds[2] = ((1.0 - weight) * (thresholds[2] << 1) as f64
                    + weight * thresholds[2] as f64) as i32 as i64;
            }
        } else if current_qindex > QINDEX_LARGE_BLOCK_THR
            && segment_id == 0
            && (source_sad_nonrd != K_HIGH_SAD || cpi.rc.avg_source_sad > 50000)
        {
            thresholds[1] = ((1.0 - weight) * (thresholds[1] << 2) as f64
                + weight * thresholds[1] as f64) as i32 as i64;
            thresholds[2] = ((1.0 - weight) * (thresholds[2] << 4) as f64
                + weight * thresholds[2] as f64) as i32 as i64;
            thresholds[3] = i64::MAX;
        }
    } else if cpi.sf.rt_sf.prefer_large_partition_blocks >= 2 {
        thresholds[1] <<= if source_sad_nonrd <= K_LOW_SAD { 2 } else { 0 };
        thresholds[2] = if source_sad_nonrd <= K_LOW_SAD {
            3 * thresholds[2]
        } else {
            thresholds[2]
        };
    } else if cpi.sf.rt_sf.prefer_large_partition_blocks >= 1 {
        let fac = if source_sad_nonrd <= K_LOW_SAD { 2 } else { 1 };
        tune_thresh_based_on_qindex_window(
            current_qindex,
            QINDEX_LARGE_BLOCK_THR,
            45,
            fac,
            thresholds,
        );
    }
    if cpi.sf.part_sf.disable_8x8_part_based_on_qidx != 0 && current_qindex < 128 {
        thresholds[3] = i64::MAX;
    }
}

// Set temporal variance low flag for superblock 64x64.
// Only first 25 in the array are used in this case.
#[inline]
fn set_low_temp_var_flag_64x64(
    mi_params: &CommonModeInfoParams,
    part_info: &mut PartitionSearchInfo,
    xd: &MacroBlockD,
    vt: &VP64x64,
    thresholds: &[i64; 5],
    mi_col: i32,
    mi_row: i32,
) {
    let bsize = xd.mbmi().bsize;
    if bsize == BLOCK_64X64 {
        if ((vt.part_variances).none.variance as i64) < (thresholds[0] >> 1) {
            part_info.variance_low[0] = 1;
        }
    } else if bsize == BLOCK_64X32 {
        for i in 0..2 {
            if (vt.part_variances.horz[i].variance as i64) < (thresholds[0] >> 2) {
                part_info.variance_low[i + 1] = 1;
            }
        }
    } else if bsize == BLOCK_32X64 {
        for i in 0..2 {
            if (vt.part_variances.vert[i].variance as i64) < (thresholds[0] >> 2) {
                part_info.variance_low[i + 3] = 1;
            }
        }
    } else {
        const IDX: [[i32; 2]; 4] = [[0, 0], [0, 8], [8, 0], [8, 8]];
        for i in 0..4 {
            if mi_params.mi_cols <= mi_col + IDX[i][1] || mi_params.mi_rows <= mi_row + IDX[i][0] {
                continue;
            }
            let idx_str = mi_params.mi_stride * (mi_row + IDX[i][0]) + mi_col + IDX[i][1];
            let Some(this_mi) = xd.mi_at_grid(idx_str as usize) else {
                continue;
            };

            if this_mi.bsize == BLOCK_32X32 {
                let threshold_32x32 = (5 * thresholds[1]) >> 3;
                if (vt.split[i].part_variances.none.variance as i64) < threshold_32x32 {
                    part_info.variance_low[i + 5] = 1;
                }
            } else {
                // For 32x16 and 16x32 blocks, the flag is set on each 16x16 block
                // inside.
                if this_mi.bsize == BLOCK_16X16
                    || this_mi.bsize == BLOCK_32X16
                    || this_mi.bsize == BLOCK_16X32
                {
                    for j in 0..4 {
                        if (vt.split[i].split[j].part_variances.none.variance as i64)
                            < (thresholds[2] >> 8)
                        {
                            part_info.variance_low[(i << 2) + j + 9] = 1;
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn set_low_temp_var_flag_128x128(
    mi_params: &CommonModeInfoParams,
    part_info: &mut PartitionSearchInfo,
    xd: &MacroBlockD,
    vt: &VP128x128,
    thresholds: &[i64; 5],
    mi_col: i32,
    mi_row: i32,
) {
    let bsize = xd.mbmi().bsize;
    if bsize == BLOCK_128X128 {
        if (vt.part_variances.none.variance as i64) < (thresholds[0] >> 1) {
            part_info.variance_low[0] = 1;
        }
    } else if bsize == BLOCK_128X64 {
        for i in 0..2 {
            if (vt.part_variances.horz[i].variance as i64) < (thresholds[0] >> 2) {
                part_info.variance_low[i + 1] = 1;
            }
        }
    } else if bsize == BLOCK_64X128 {
        for i in 0..2 {
            if (vt.part_variances.vert[i].variance as i64) < (thresholds[0] >> 2) {
                part_info.variance_low[i + 3] = 1;
            }
        }
    } else {
        const IDX64: [[i32; 2]; 4] = [[0, 0], [0, 16], [16, 0], [16, 16]];
        const IDX32: [[i32; 2]; 4] = [[0, 0], [0, 8], [8, 0], [8, 8]];
        for i in 0..4 {
            let idx_str = mi_params.mi_stride * (mi_row + IDX64[i][0]) + mi_col + IDX64[i][1];
            // (upstream reads the grid before checking that the block is
            // inside the frame; the grid entry is only read here once it is)
            if mi_params.mi_cols <= mi_col + IDX64[i][1]
                || mi_params.mi_rows <= mi_row + IDX64[i][0]
            {
                continue;
            }
            let Some(mi_64) = xd.mi_at_grid(idx_str as usize) else {
                continue;
            };
            let threshold_64x64 = (5 * thresholds[1]) >> 3;
            if mi_64.bsize == BLOCK_64X64 {
                if (vt.split[i].part_variances.none.variance as i64) < threshold_64x64 {
                    part_info.variance_low[5 + i] = 1;
                }
            } else if mi_64.bsize == BLOCK_64X32 {
                for j in 0..2 {
                    if (vt.split[i].part_variances.horz[j].variance as i64) < (threshold_64x64 >> 1)
                    {
                        part_info.variance_low[9 + (i << 1) + j] = 1;
                    }
                }
            } else if mi_64.bsize == BLOCK_32X64 {
                for j in 0..2 {
                    if (vt.split[i].part_variances.vert[j].variance as i64) < (threshold_64x64 >> 1)
                    {
                        part_info.variance_low[17 + (i << 1) + j] = 1;
                    }
                }
            } else {
                for k in 0..4 {
                    if mi_params.mi_cols <= mi_col + IDX64[i][1] + IDX32[k][1]
                        || mi_params.mi_rows <= mi_row + IDX64[i][0] + IDX32[k][0]
                    {
                        continue;
                    }
                    let idx_str1 = mi_params.mi_stride * IDX32[k][0] + IDX32[k][1];
                    let Some(mi_32) = xd.mi_at_grid((idx_str + idx_str1) as usize) else {
                        continue;
                    };

                    let threshold_32x32 = (5 * thresholds[2]) >> 3;
                    if mi_32.bsize == BLOCK_32X32 {
                        if (vt.split[i].split[k].part_variances.none.variance as i64)
                            < threshold_32x32
                        {
                            part_info.variance_low[25 + (i << 2) + k] = 1;
                        }
                    } else {
                        // For 32x16 and 16x32 blocks, the flag is set on each 16x16 block
                        // inside.
                        if mi_32.bsize == BLOCK_16X16
                            || mi_32.bsize == BLOCK_32X16
                            || mi_32.bsize == BLOCK_16X32
                        {
                            for j in 0..4 {
                                if (vt.split[i].split[k].split[j].part_variances.none.variance
                                    as i64)
                                    < (thresholds[3] >> 8)
                                {
                                    part_info.variance_low[41 + (i << 4) + (k << 2) + j] = 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn set_low_temp_var_flag(
    cpi: &Av1Comp,
    part_info: &mut PartitionSearchInfo,
    xd: &MacroBlockD,
    vt: &VP128x128,
    thresholds: &[i64; 5],
    ref_frame_partition: MvReferenceFrame,
    mi_col: i32,
    mi_row: i32,
) {
    let cm = &cpi.common;
    // Check temporal variance for bsize >= 16x16, if LAST_FRAME was selected.
    // If the temporal variance is small set the flag
    // variance_low for the block. The variance threshold can be adjusted, the
    // higher the more aggressive.
    if ref_frame_partition == LAST_FRAME {
        let is_small_sb = cm.seq_params.sb_size == BLOCK_64X64;
        if is_small_sb {
            set_low_temp_var_flag_64x64(
                &cm.mi_params,
                part_info,
                xd,
                &vt.split[0],
                thresholds,
                mi_col,
                mi_row,
            );
        } else {
            set_low_temp_var_flag_128x128(
                &cm.mi_params,
                part_info,
                xd,
                vt,
                thresholds,
                mi_col,
                mi_row,
            );
        }
    }
}

const POS_SHIFT_16X16: [[usize; 4]; 4] = [
    [9, 10, 13, 14],
    [11, 12, 15, 16],
    [17, 18, 21, 22],
    [19, 20, 23, 24],
];

pub(crate) fn av1_get_force_skip_low_temp_var_small_sb(
    variance_low: &[u8; 105],
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) -> i32 {
    // Relative indices of MB inside the superblock.
    let mi_x = mi_row & 0xF;
    let mi_y = mi_col & 0xF;
    // Relative indices of 16x16 block inside the superblock.
    let i = (mi_x >> 2) as usize;
    let j = (mi_y >> 2) as usize;
    let mut force_skip_low_temp_var = 0;
    // Set force_skip_low_temp_var based on the block size and block offset.
    match bsize {
        BLOCK_64X64 => force_skip_low_temp_var = variance_low[0],
        BLOCK_64X32 => {
            if mi_y == 0 && mi_x == 0 {
                force_skip_low_temp_var = variance_low[1];
            } else if mi_y == 0 && mi_x != 0 {
                force_skip_low_temp_var = variance_low[2];
            }
        }
        BLOCK_32X64 => {
            if mi_y == 0 && mi_x == 0 {
                force_skip_low_temp_var = variance_low[3];
            } else if mi_y != 0 && mi_x == 0 {
                force_skip_low_temp_var = variance_low[4];
            }
        }
        BLOCK_32X32 => {
            if mi_y == 0 && mi_x == 0 {
                force_skip_low_temp_var = variance_low[5];
            } else if mi_y != 0 && mi_x == 0 {
                force_skip_low_temp_var = variance_low[6];
            } else if mi_y == 0 && mi_x != 0 {
                force_skip_low_temp_var = variance_low[7];
            } else if mi_y != 0 && mi_x != 0 {
                force_skip_low_temp_var = variance_low[8];
            }
        }
        BLOCK_32X16 | BLOCK_16X32 | BLOCK_16X16 => {
            force_skip_low_temp_var = variance_low[POS_SHIFT_16X16[i][j]];
        }
        _ => {}
    }

    force_skip_low_temp_var as i32
}

pub(crate) fn av1_get_force_skip_low_temp_var(
    variance_low: &[u8; 105],
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) -> i32 {
    let mut force_skip_low_temp_var = 0;
    let mut x = (mi_col & 0x1F) >> 4;
    // y = (mi_row & 0x1F) >> 4;
    // const int idx64 = (y << 1) + x;
    let mut y = (mi_row & 0x17) >> 3;
    let idx64 = (y + x) as usize;

    x = (mi_col & 0xF) >> 3;
    // y = (mi_row & 0xF) >> 3;
    // const int idx32 = (y << 1) + x;
    y = (mi_row & 0xB) >> 2;
    let idx32 = (y + x) as usize;

    x = (mi_col & 0x7) >> 2;
    // y = (mi_row & 0x7) >> 2;
    // const int idx16 = (y << 1) + x;
    y = (mi_row & 0x5) >> 1;
    let idx16 = (y + x) as usize;
    // Set force_skip_low_temp_var based on the block size and block offset.
    match bsize {
        BLOCK_128X128 => force_skip_low_temp_var = variance_low[0],
        BLOCK_128X64 => {
            debug_assert!((mi_col & 0x1F) == 0);
            force_skip_low_temp_var = variance_low[1 + ((mi_row & 0x1F) != 0) as usize];
        }
        BLOCK_64X128 => {
            debug_assert!((mi_row & 0x1F) == 0);
            force_skip_low_temp_var = variance_low[3 + ((mi_col & 0x1F) != 0) as usize];
        }
        BLOCK_64X64 => {
            // Location of this 64x64 block inside the 128x128 superblock
            force_skip_low_temp_var = variance_low[5 + idx64];
        }
        BLOCK_64X32 => {
            x = (mi_col & 0x1F) >> 4;
            y = (mi_row & 0x1F) >> 3;
            /*
            .---------------.---------------.
            | x=0,y=0,idx=0 | x=0,y=0,idx=2 |
            :---------------+---------------:
            | x=0,y=1,idx=1 | x=1,y=1,idx=3 |
            :---------------+---------------:
            | x=0,y=2,idx=4 | x=1,y=2,idx=6 |
            :---------------+---------------:
            | x=0,y=3,idx=5 | x=1,y=3,idx=7 |
            '---------------'---------------'
            */
            let idx64x32 = ((x << 1) + (y % 2) + ((y >> 1) << 2)) as usize;
            force_skip_low_temp_var = variance_low[9 + idx64x32];
        }
        BLOCK_32X64 => {
            x = (mi_col & 0x1F) >> 3;
            y = (mi_row & 0x1F) >> 4;
            let idx32x64 = ((y << 2) + x) as usize;
            force_skip_low_temp_var = variance_low[17 + idx32x64];
        }
        BLOCK_32X32 => {
            force_skip_low_temp_var = variance_low[25 + (idx64 << 2) + idx32];
        }
        BLOCK_32X16 | BLOCK_16X32 | BLOCK_16X16 => {
            force_skip_low_temp_var = variance_low[41 + (idx64 << 4) + (idx32 << 2) + idx16];
        }
        _ => {}
    }
    force_skip_low_temp_var as i32
}

pub(crate) fn av1_set_variance_partition_thresholds(
    cpi: &mut Av1Comp,
    q: i32,
    content_lowsumdiff: i32,
) {
    let sf = &cpi.sf;
    if sf.part_sf.partition_search_type != VAR_BASED_PARTITION {
        return;
    }
    let mut thresholds = cpi.vbp_info.thresholds;
    set_vbp_thresholds(cpi, &mut thresholds, q, content_lowsumdiff, 0, 0, 0, 0, 0);
    cpi.vbp_info.thresholds = thresholds;
    // The threshold below is not changed locally.
    cpi.vbp_info.threshold_minmax = 15 + (q >> 3);
}

/// `VAR_BASED_PARTITION` (speed_features.h)
pub(crate) use super::speed_features::VAR_BASED_PARTITION;

#[inline]
fn chroma_check(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    y_sad: u32,
    y_sad_g: u32,
    is_key_frame: bool,
    zero_motion: bool,
    uv_sad: &mut [u32; 2],
) {
    let mut shift = 3;
    if is_key_frame || cpi.oxcf.tool_cfg.enable_monochrome {
        return;
    }

    if cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN && cpi.rc.high_source_sad != 0 {
        shift = 5;
    }

    let cm = &cpi.common;
    let xd = &x.e_mbd;
    let mi = xd.mbmi();
    let yv12 = get_ref_frame_yv12_buf(cm, LAST_FRAME);
    let yv12_g = get_ref_frame_yv12_buf(cm, GOLDEN_FRAME);
    let sf = get_ref_scale_factors_const(cm, LAST_FRAME);
    let mut dst = Buf2d::default();
    let mut uv_sad_g: u32 = 0;
    let src_vec = std::mem::take(&mut x.source_alloc);
    let src = &src_vec;
    let fn_ptr = &cpi.ppi.fn_ptr;
    let pool = &cm.buffer_pool.frame_bufs;

    for i in 1..=2usize {
        let p = &x.plane[i];
        let pd = &xd.plane[i];
        let bs = get_plane_block_size(bsize, pd.subsampling_x, pd.subsampling_y);

        if bs != BLOCK_INVALID {
            // For last:
            if zero_motion {
                if mi.ref_frame[0] == LAST_FRAME {
                    uv_sad[i - 1] = fn_ptr.sdf(
                        bs,
                        src,
                        p.src.buf,
                        p.src.stride as usize,
                        &pool[pd.pre[0].frame_buf].buf.buffer_alloc,
                        pd.pre[0].buf,
                        pd.pre[0].stride as usize,
                    );
                } else if let Some((yv12, _)) = yv12 {
                    setup_pred_plane(
                        &mut dst,
                        mi.bsize,
                        yv12.buffers[i],
                        yv12.crop_widths[1],
                        yv12.crop_heights[1],
                        yv12.strides[1],
                        xd.mi_row,
                        xd.mi_col,
                        sf,
                        xd.plane[i].subsampling_x,
                        xd.plane[i].subsampling_y,
                    );

                    uv_sad[i - 1] = fn_ptr.sdf(
                        bs,
                        src,
                        p.src.buf,
                        p.src.stride as usize,
                        &yv12.buffer_alloc,
                        dst.buf,
                        dst.stride as usize,
                    );
                }
            } else {
                uv_sad[i - 1] = fn_ptr.sdf(
                    bs,
                    src,
                    p.src.buf,
                    p.src.stride as usize,
                    &xd.recon.buffer_alloc,
                    pd.dst.buf,
                    pd.dst.stride as usize,
                );
            }

            // For golden:
            if y_sad_g != u32::MAX {
                if let Some((yv12_g, _)) = yv12_g {
                    setup_pred_plane(
                        &mut dst,
                        mi.bsize,
                        yv12_g.buffers[i],
                        yv12_g.crop_widths[1],
                        yv12_g.crop_heights[1],
                        yv12_g.strides[1],
                        xd.mi_row,
                        xd.mi_col,
                        sf,
                        xd.plane[i].subsampling_x,
                        xd.plane[i].subsampling_y,
                    );
                    uv_sad_g = fn_ptr.sdf(
                        bs,
                        src,
                        p.src.buf,
                        p.src.stride as usize,
                        &yv12_g.buffer_alloc,
                        dst.buf,
                        dst.stride as usize,
                    );
                }
            }
        }

        if uv_sad[i - 1] > (y_sad >> 1) {
            x.color_sensitivity_sb[i - 1] = 1;
        } else if uv_sad[i - 1] < (y_sad >> shift) {
            x.color_sensitivity_sb[i - 1] = 0;
        }
        // Borderline case: to be refined at coding block level in nonrd_pickmode,
        // for coding block size < sb_size.
        else {
            x.color_sensitivity_sb[i - 1] = 2;
        }

        x.color_sensitivity_sb_g[i - 1] = (uv_sad_g > y_sad_g / 6) as u8;
    }
    x.source_alloc = src_vec;
}

fn fill_variance_tree_leaves(
    cpi: &Av1Comp,
    xd: &MacroBlockD,
    vt: &mut VP128x128,
    force_split: &mut [i32; 85],
    avg_16x16: &mut [[i32; 4]; 4],
    maxvar_16x16: &mut [[i32; 4]; 4],
    minvar_16x16: &mut [[i32; 4]; 4],
    variance4x4downsample: &mut [i32; 64],
    thresholds: &[i64; 5],
    src: Samples,
    dst: Samples,
    is_key_frame: bool,
) {
    let cm = &cpi.common;
    let is_small_sb = cm.seq_params.sb_size == BLOCK_64X64;
    let num_64x64_blocks = if is_small_sb { 1 } else { 4 };
    // TODO(kyslov) Bring back compute_minmax_variance with content type detection
    // (compute_minmax_variance is 0)
    let mut pixels_wide = 128;
    let mut pixels_high = 128;
    let mut border_offset_4x4 = 0;
    let temporal_denoising = cpi.sf.rt_sf.use_rtc_tf;
    if is_small_sb {
        pixels_wide = 64;
        pixels_high = 64;
    }
    if xd.mb_to_right_edge < 0 {
        pixels_wide += xd.mb_to_right_edge >> 3;
    }
    if xd.mb_to_bottom_edge < 0 {
        pixels_high += xd.mb_to_bottom_edge >> 3;
    }
    // For temporal filtering or temporal denoiser enabled: since the source
    // is modified we need to avoid 4x4 avg along superblock boundary, since
    // simd code will load 8 pixels for 4x4 avg and so can access source
    // data outside superblock (while its being modified by temporal filter).
    // Temporal filtering is never done on key frames.
    if !is_key_frame && temporal_denoising != 0 {
        border_offset_4x4 = 4;
    }
    for m in 0..num_64x64_blocks {
        let x64_idx = (m as i32 & 1) << 6;
        let y64_idx = (m as i32 >> 1) << 6;
        let m2 = m << 2;
        force_split[m + 1] = PART_EVAL_ALL;

        for i in 0..4usize {
            let x32_idx = x64_idx + ((i as i32 & 1) << 5);
            let y32_idx = y64_idx + ((i as i32 >> 1) << 5);
            let i2 = (m2 + i) << 2;
            force_split[5 + m2 + i] = PART_EVAL_ALL;
            avg_16x16[m][i] = 0;
            maxvar_16x16[m][i] = 0;
            minvar_16x16[m][i] = i32::MAX;
            for j in 0..4usize {
                let x16_idx = x32_idx + ((j as i32 & 1) << 4);
                let y16_idx = y32_idx + ((j as i32 >> 1) << 4);
                let split_index = 21 + i2 + j;
                let vst = &mut vt.split[m].split[i].split[j];
                force_split[split_index] = PART_EVAL_ALL;
                variance4x4downsample[i2 + j] = 0;
                if !is_key_frame {
                    fill_variance_8x8avg(
                        src,
                        dst,
                        x16_idx,
                        y16_idx,
                        vst,
                        is_cur_buf_hbd(xd),
                        pixels_wide,
                        pixels_high,
                        is_key_frame,
                    );

                    fill_variance_tree(vst);
                    get_variance(&mut vst.part_variances.none);
                    let var = vst.part_variances.none.variance;
                    avg_16x16[m][i] = avg_16x16[m][i].wrapping_add(var);
                    if var < minvar_16x16[m][i] {
                        minvar_16x16[m][i] = var;
                    }
                    if var > maxvar_16x16[m][i] {
                        maxvar_16x16[m][i] = var;
                    }
                    if var as i64 > thresholds[3] {
                        // 16X16 variance is above threshold for split, so force split to
                        // 8x8 for this 16x16 block (this also forces splits for upper
                        // levels).
                        force_split[split_index] = PART_EVAL_ONLY_SPLIT;
                        force_split[5 + m2 + i] = PART_EVAL_ONLY_SPLIT;
                        force_split[m + 1] = PART_EVAL_ONLY_SPLIT;
                        force_split[0] = PART_EVAL_ONLY_SPLIT;
                    }
                    // (the minmax split check: compute_minmax_variance is 0)
                }
                if is_key_frame {
                    force_split[split_index] = PART_EVAL_ALL;
                    // Go down to 4x4 down-sampling for variance.
                    variance4x4downsample[i2 + j] = 1;
                    for k in 0..4usize {
                        let x8_idx = x16_idx + ((k as i32 & 1) << 3);
                        let y8_idx = y16_idx + ((k as i32 >> 1) << 3);
                        // (vt2 is never allocated: on key frames, the tree's
                        // own nodes are used)
                        let vst2 = &mut vst.split[k];
                        fill_variance_4x4avg(
                            src,
                            dst,
                            x8_idx,
                            y8_idx,
                            vst2,
                            xd.cur_buf_flags,
                            pixels_wide,
                            pixels_high,
                            is_key_frame,
                            border_offset_4x4,
                        );
                    }
                }
            }
        }
    }
}

/// Translation of `setup_planes()`: returns whether the predictor was not
/// needed (a zero motion vector) or has been built.
fn setup_planes(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    y_sad: &mut u32,
    y_sad_g: &mut u32,
    y_sad_alt: &mut u32,
    y_sad_last: &mut u32,
    ref_frame_partition: &mut MvReferenceFrame,
    mi_row: i32,
    mi_col: i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);
    let is_small_sb = cm.seq_params.sb_size == BLOCK_64X64;
    let bsize = if is_small_sb {
        BLOCK_64X64
    } else {
        BLOCK_128X128
    };
    let yv12 = get_ref_frame_yv12_buf(cm, LAST_FRAME);
    debug_assert!(yv12.is_some());
    let mut yv12_g = None;
    let mut yv12_alt = None;
    // Check if LAST is a reference. For spatial layers always use it as
    // reference scaling (golden or altref being lower resolution) is not
    // handled/check here.
    let use_last_ref =
        (cpi.ref_frame_flags & AOM_LAST_FLAG) != 0 || cpi.svc.number_spatial_layers > 1;
    let use_golden_ref = cpi.ref_frame_flags & AOM_GOLD_FLAG != 0;
    let use_alt_ref =
        cpi.ppi.rtc_ref.set_ref_frame_config || cpi.sf.rt_sf.use_nonrd_altref_frame != 0;
    let fn_ptr = &cpi.ppi.fn_ptr;
    let pool = &cm.buffer_pool.frame_bufs;
    let pre_sad = |x: &Macroblock| {
        let pre = &x.e_mbd.plane[0].pre[0];
        fn_ptr.sdf(
            bsize,
            &x.source_alloc,
            x.plane[0].src.buf,
            x.plane[0].src.stride as usize,
            &pool[pre.frame_buf].buf.buffer_alloc,
            pre.buf,
            pre.stride as usize,
        )
    };

    // For 1 spatial layer: GOLDEN is another temporal reference.
    // Check if it should be used as reference for partitioning.
    if cpi.svc.number_spatial_layers == 1
        && use_golden_ref
        && (x.content_state_sb.source_sad_nonrd != K_ZERO_SAD || !use_last_ref)
    {
        yv12_g = get_ref_frame_yv12_buf(cm, GOLDEN_FRAME);
        if let Some(g) = yv12_g {
            if yv12.map(|l| l.1) != Some(g.1) {
                av1_setup_pre_planes(
                    &mut x.e_mbd,
                    0,
                    Some(g),
                    mi_row,
                    mi_col,
                    get_ref_scale_factors_const(cm, GOLDEN_FRAME),
                    num_planes,
                );
                *y_sad_g = pre_sad(x);
            }
        }
    }

    // For 1 spatial layer: ALTREF is another temporal reference.
    // Check if it should be used as reference for partitioning.
    if cpi.svc.number_spatial_layers == 1
        && use_alt_ref
        && (cpi.ref_frame_flags & AOM_ALT_FLAG) != 0
        && (x.content_state_sb.source_sad_nonrd != K_ZERO_SAD || !use_last_ref)
    {
        yv12_alt = get_ref_frame_yv12_buf(cm, ALTREF_FRAME);
        if let Some(a) = yv12_alt {
            if yv12.map(|l| l.1) != Some(a.1) {
                av1_setup_pre_planes(
                    &mut x.e_mbd,
                    0,
                    Some(a),
                    mi_row,
                    mi_col,
                    get_ref_scale_factors_const(cm, ALTREF_FRAME),
                    num_planes,
                );
                *y_sad_alt = pre_sad(x);
            }
        }
    }

    if use_last_ref {
        av1_setup_pre_planes(
            &mut x.e_mbd,
            0,
            yv12,
            mi_row,
            mi_col,
            get_ref_scale_factors_const(cm, LAST_FRAME),
            num_planes,
        );
        let mi = x.e_mbd.mbmi_mut();
        mi.ref_frame[0] = LAST_FRAME;
        mi.ref_frame[1] = NONE_FRAME;
        mi.bsize = cm.seq_params.sb_size;
        mi.mv[0].as_int = 0;
        mi.interp_filters = av1_broadcast_interp_filter(BILINEAR);
        if cpi.sf.rt_sf.estimate_motion_for_var_based_partition != 0
            && x.e_mbd.mb_to_right_edge >= 0
            && x.e_mbd.mb_to_bottom_edge >= 0
        {
            let dummy_mv = Mv { row: 0, col: 0 };
            *y_sad = av1_int_pro_motion_estimation(
                cpi,
                x,
                cm.seq_params.sb_size,
                mi_row,
                mi_col,
                &dummy_mv,
            )?;
        }
        if *y_sad == u32::MAX {
            *y_sad = pre_sad(x);
        }
        *y_sad_last = *y_sad;
    }

    // Pick the ref frame for partitioning, use golden or altref frame only if
    // its lower sad, bias to LAST with factor 0.9.
    if (*y_sad_g as f64) < 0.9 * *y_sad as f64 && *y_sad_g < *y_sad_alt {
        av1_setup_pre_planes(
            &mut x.e_mbd,
            0,
            yv12_g,
            mi_row,
            mi_col,
            get_ref_scale_factors_const(cm, GOLDEN_FRAME),
            num_planes,
        );
        let mi = x.e_mbd.mbmi_mut();
        mi.ref_frame[0] = GOLDEN_FRAME;
        mi.mv[0].as_int = 0;
        *y_sad = *y_sad_g;
        *ref_frame_partition = GOLDEN_FRAME;
        x.nonrd_prune_ref_frame_search = 0;
    } else if (*y_sad_alt as f64) < 0.9 * *y_sad as f64 && *y_sad_alt < *y_sad_g {
        av1_setup_pre_planes(
            &mut x.e_mbd,
            0,
            yv12_alt,
            mi_row,
            mi_col,
            get_ref_scale_factors_const(cm, ALTREF_FRAME),
            num_planes,
        );
        let mi = x.e_mbd.mbmi_mut();
        mi.ref_frame[0] = ALTREF_FRAME;
        mi.mv[0].as_int = 0;
        *y_sad = *y_sad_alt;
        *ref_frame_partition = ALTREF_FRAME;
        x.nonrd_prune_ref_frame_search = 0;
    } else {
        *ref_frame_partition = LAST_FRAME;
        x.nonrd_prune_ref_frame_search = cpi.sf.rt_sf.nonrd_prune_ref_frame_search;
    }

    // Only calculate the predictor for non-zero MV.
    if x.e_mbd.mbmi().mv[0].as_int != 0 {
        let (rf0, rf1) = (x.e_mbd.mbmi().ref_frame[0], x.e_mbd.mbmi().ref_frame[1]);
        set_ref_ptrs(cm, &mut x.e_mbd, rf0, rf1)?;
        av1_enc_build_inter_predictor(
            cm,
            &mut x.e_mbd,
            mi_row,
            mi_col,
            None,
            cm.seq_params.sb_size,
            AOM_PLANE_Y,
            num_planes - 1,
        )?;
    }
    Ok(())
}

// Decides whether to split or merge a 16x16 partition block in variance based
// partitioning based on the 8x8 sub-block variances.
#[inline]
fn get_part_eval_based_on_sub_blk_var(var_16x16_info: &mut VP16x16, threshold16: i64) -> i32 {
    let mut max_8x8_var = 0;
    let mut min_8x8_var = i32::MAX;
    for k in 0..4 {
        get_variance(&mut var_16x16_info.split[k].part_variances.none);
        let this_8x8_var = var_16x16_info.split[k].part_variances.none.variance;
        max_8x8_var = this_8x8_var.max(max_8x8_var);
        min_8x8_var = this_8x8_var.min(min_8x8_var);
    }
    // If the difference between maximum and minimum sub-block variances is high,
    // then only evaluate PARTITION_SPLIT for the 16x16 block. Otherwise, evaluate
    // only PARTITION_NONE. The shift factor for threshold16 has been derived
    // empirically.
    if (max_8x8_var.wrapping_sub(min_8x8_var)) as i64 > (threshold16 << 2) {
        PART_EVAL_ONLY_SPLIT
    } else {
        PART_EVAL_ONLY_NONE
    }
}

#[inline]
fn is_set_force_zeromv_skip_based_on_src_sad(
    set_zeromv_skip_based_on_source_sad: i32,
    source_sad_nonrd: i32,
) -> bool {
    if set_zeromv_skip_based_on_source_sad == 0 {
        return false;
    }

    if set_zeromv_skip_based_on_source_sad >= 2 {
        return source_sad_nonrd <= K_VERY_LOW_SAD;
    } else if set_zeromv_skip_based_on_source_sad >= 1 {
        return source_sad_nonrd == K_ZERO_SAD;
    }

    false
}

/// Translation of `av1_choose_var_based_partitioning()`: writes the
/// partition into the mode info grid (moved into `td.mb.e_mbd`).
pub(crate) fn av1_choose_var_based_partitioning(
    cpi: &Av1Comp,
    tile: &TileInfo,
    td: &mut ThreadData,
    mi_row: i32,
    mi_col: i32,
) -> Result<i32, ()> {
    let cm = &cpi.common;
    let vbp_thresholds = &cpi.vbp_info.thresholds;
    let mut force_split = [0i32; 85];
    let mut avg_64x64: i32;
    let mut max_var_32x32 = [0i32; 4];
    let mut min_var_32x32 = [0i32; 4];
    let mut var_32x32: i32;
    let mut var_64x64: i32;
    let mut min_var_64x64 = i32::MAX;
    let mut max_var_64x64 = 0;
    let mut avg_16x16 = [[0i32; 4]; 4];
    let mut maxvar_16x16 = [[0i32; 4]; 4];
    let mut minvar_16x16 = [[0i32; 4]; 4];
    let mut uv_sad = [0u32; 2];
    // (noise_level stays kLow: the noise estimate is never enabled)
    let noise_level = K_LOW;
    let mut zero_motion = true;

    let mut is_key_frame = frame_is_intra_only(cm) || {
        // (no SVC)
        debug_assert!(!cpi.ppi.use_svc);
        false
    };

    debug_assert!(cm.seq_params.sb_size == BLOCK_64X64 || cm.seq_params.sb_size == BLOCK_128X128);
    let is_small_sb = cm.seq_params.sb_size == BLOCK_64X64;
    let num_64x64_blocks = if is_small_sb { 1 } else { 4 };

    let mut y_sad = u32::MAX;
    let mut y_sad_g = u32::MAX;
    let mut y_sad_alt = u32::MAX;
    let mut y_sad_last = u32::MAX;
    let bsize = if is_small_sb {
        BLOCK_64X64
    } else {
        BLOCK_128X128
    };

    // Ref frame used in partitioning.
    let mut ref_frame_partition = LAST_FRAME;

    let ThreadData { mb: x, vt64x64, .. } = td;
    // (upstream mallocs vt without clearing it; its part_variances are only
    // read after they are filled)
    let mut vt = VP128x128 {
        part_variances: VPVariance::default(),
        split: vt64x64,
    };

    let mut thresholds: [i64; 5] = [
        vbp_thresholds[0],
        vbp_thresholds[1],
        vbp_thresholds[2],
        vbp_thresholds[3],
        vbp_thresholds[4],
    ];

    let mut variance4x4downsample = [0i32; 64];
    let segment_id = x.e_mbd.mbmi().segment_id as i32;
    let mut blk_sad: u64 = 0;
    if !cpi.src_sad_blk_64x64.is_empty() && !cpi.ppi.use_svc {
        let sb_size_by_mb = if cm.seq_params.sb_size == BLOCK_128X128 {
            cm.seq_params.mib_size >> 1
        } else {
            cm.seq_params.mib_size
        };
        let sb_cols = (cm.mi_params.mi_cols + sb_size_by_mb - 1) / sb_size_by_mb;
        let sbi_col = mi_col / sb_size_by_mb;
        let sbi_row = mi_row / sb_size_by_mb;
        blk_sad = cpi.src_sad_blk_64x64[(sbi_col + sbi_row * sb_cols) as usize];
    }

    // (the adaptive quantization of the cyclic refresh is off: no boosted
    // segments)
    debug_assert!(cpi.oxcf.q_cfg.aq_mode != CYCLIC_REFRESH_AQ || cm.seg.enabled == 0);
    set_vbp_thresholds(
        cpi,
        &mut thresholds,
        cm.quant_params.base_qindex,
        x.content_state_sb.low_sumdiff,
        x.content_state_sb.source_sad_nonrd,
        x.content_state_sb.source_sad_rd,
        0,
        blk_sad,
        x.content_state_sb.lighting_change,
    );

    // For non keyframes, disable 4x4 average for low resolution when speed = 8
    // (threshold_4x4avg is INT64_MAX: vt2 is never allocated)

    let s = x.plane[0].src.buf;
    let sp = x.plane[0].src.stride;

    // Index for force_split: 0 for 64x64, 1-4 for 32x32 blocks,
    // 5-20 for the 16x16 blocks.
    force_split[0] = PART_EVAL_ALL;
    x.part_search_info.variance_low = [0; 105];

    // Check if LAST frame is NULL or if the resolution of LAST is
    // different than the current frame resolution, and if so, treat this frame
    // as a key frame, for the purpose of the superblock partitioning.
    // LAST == NULL can happen in cases where enhancement spatial layers are
    // enabled dyanmically and the only reference is the spatial(GOLDEN).
    // TODO(marpan): Check se of scaled references for the different resoln.
    if !frame_is_intra_only(cm) {
        match get_ref_frame_yv12_buf(cm, LAST_FRAME) {
            Some((r, _)) if r.y_crop_height() == cm.height && r.y_crop_width() == cm.width => {}
            _ => is_key_frame = true,
        }
    }

    let (d_frame, dp): (Option<usize>, i32);
    let d_off: usize;
    if !is_key_frame {
        setup_planes(
            cpi,
            x,
            &mut y_sad,
            &mut y_sad_g,
            &mut y_sad_alt,
            &mut y_sad_last,
            &mut ref_frame_partition,
            mi_row,
            mi_col,
        )?;

        let xd = &x.e_mbd;
        // Use reference SB directly for zero mv.
        if xd.mbmi().mv[0].as_int != 0 {
            d_frame = None;
            d_off = xd.plane[0].dst.buf;
            dp = xd.plane[0].dst.stride;
            zero_motion = false;
        } else {
            d_frame = Some(xd.plane[0].pre[0].frame_buf);
            d_off = xd.plane[0].pre[0].buf;
            dp = xd.plane[0].pre[0].stride;
        }
    } else {
        // (d is AV1_VAR_OFFS with a stride of 0: never read, the key frames'
        // averages are compared with 128)
        d_frame = None;
        d_off = 0;
        dp = 0;
    }

    chroma_check(
        cpi,
        x,
        bsize,
        y_sad_last,
        y_sad_g,
        is_key_frame,
        zero_motion,
        &mut uv_sad,
    );

    x.force_zeromv_skip_for_sb = 0;
    let is_set_force_zeromv_skip = is_set_force_zeromv_skip_based_on_src_sad(
        cpi.sf.rt_sf.set_zeromv_skip_based_on_source_sad,
        x.content_state_sb.source_sad_nonrd,
    );

    // If the superblock is completely static (zero source sad) and
    // the y_sad (relative to LAST ref) is very small, take the sb_size partition
    // and exit, and force zeromv_last skip mode for nonrd_pickmode.
    // Only do this on the base segment (so the QP-boosted segment, if applied,
    // can still continue cleaning/ramping up the quality).
    // Condition on color uv_sad is also added.
    if !is_key_frame
        && cpi.sf.rt_sf.part_early_exit_zeromv != 0
        && cpi.rc.frames_since_key > 30
        && segment_id == CR_SEGMENT_ID_BASE
        && is_set_force_zeromv_skip
        && ref_frame_partition == LAST_FRAME
        && x.e_mbd.mbmi().mv[0].as_int == 0
    {
        let block_width = MI_SIZE_WIDE[cm.seq_params.sb_size as usize] as i32;
        let block_height = MI_SIZE_HIGH[cm.seq_params.sb_size as usize] as i32;
        let thresh_exit_part_y = cpi.zeromv_skip_thresh_exit_part[bsize as usize];
        let thresh_exit_part_uv = calc_chroma_thresh_for_zeromv_skip(thresh_exit_part_y);
        if mi_col + block_width <= tile.mi_col_end
            && mi_row + block_height <= tile.mi_row_end
            && y_sad < thresh_exit_part_y
            && uv_sad[0] < thresh_exit_part_uv
            && uv_sad[1] < thresh_exit_part_uv
        {
            set_block_size(cm, &mut x.e_mbd, mi_row, mi_col, bsize);
            x.force_zeromv_skip_for_sb = 1;
            return Ok(0);
        } else if x.content_state_sb.source_sad_nonrd == K_ZERO_SAD
            && cpi.sf.rt_sf.part_early_exit_zeromv >= 2
        {
            x.force_zeromv_skip_for_sb = 2;
        }
    }

    // (cpi->noise_estimate.enabled is 0)

    // Fill in the entire tree of 8x8 (or 4x4 under some conditions) variances
    // for splits.
    {
        let xd = &x.e_mbd;
        let src = Samples {
            buf: &x.source_alloc,
            off: s,
            stride: sp,
        };
        let dst = Samples {
            buf: match d_frame {
                Some(f) => &cm.buffer_pool.frame_bufs[f].buf.buffer_alloc,
                None if !is_key_frame => &xd.recon.buffer_alloc,
                None => &[],
            },
            off: d_off,
            stride: dp,
        };
        fill_variance_tree_leaves(
            cpi,
            xd,
            &mut vt,
            &mut force_split,
            &mut avg_16x16,
            &mut maxvar_16x16,
            &mut minvar_16x16,
            &mut variance4x4downsample,
            &thresholds,
            src,
            dst,
            is_key_frame,
        );
    }

    avg_64x64 = 0;
    for m in 0..num_64x64_blocks {
        max_var_32x32[m] = 0;
        min_var_32x32[m] = i32::MAX;
        let m2 = m << 2;
        for i in 0..4 {
            let i2 = (m2 + i) << 2;
            for j in 0..4 {
                let split_index = 21 + i2 + j;
                if variance4x4downsample[i2 + j] == 1 {
                    // (vt2 is never allocated: variance4x4downsample[] is only
                    // set on key frames)
                    let vtemp = &mut vt.split[m].split[i].split[j];
                    for k in 0..4 {
                        fill_variance_tree(&mut vtemp.split[k]);
                    }
                    fill_variance_tree(vtemp);
                    // If variance of this 16x16 block is above the threshold, force block
                    // to split. This also forces a split on the upper levels.
                    get_variance(&mut vtemp.part_variances.none);
                    if vtemp.part_variances.none.variance as i64 > thresholds[3] {
                        force_split[split_index] =
                            if cpi.sf.rt_sf.vbp_prune_16x16_split_using_min_max_sub_blk_var {
                                get_part_eval_based_on_sub_blk_var(vtemp, thresholds[3])
                            } else {
                                PART_EVAL_ONLY_SPLIT
                            };
                        force_split[5 + m2 + i] = PART_EVAL_ONLY_SPLIT;
                        force_split[m + 1] = PART_EVAL_ONLY_SPLIT;
                        force_split[0] = PART_EVAL_ONLY_SPLIT;
                    }
                }
            }
            fill_variance_tree(&mut vt.split[m].split[i]);
            // If variance of this 32x32 block is above the threshold, or if its above
            // (some threshold of) the average variance over the sub-16x16 blocks,
            // then force this block to split. This also forces a split on the upper
            // (64x64) level.
            let mut frame_sad_thresh: u64 = 20000;
            if cpi.svc.number_temporal_layers > 2 && cpi.svc.temporal_layer_id == 0 {
                frame_sad_thresh <<= 1;
            }
            if force_split[5 + m2 + i] == PART_EVAL_ALL {
                get_variance(&mut vt.split[m].split[i].part_variances.none);
                var_32x32 = vt.split[m].split[i].part_variances.none.variance;
                max_var_32x32[m] = var_32x32.max(max_var_32x32[m]);
                min_var_32x32[m] = var_32x32.min(min_var_32x32[m]);
                let v = vt.split[m].split[i].part_variances.none.variance as i64;
                if v > thresholds[2]
                    || (!is_key_frame
                        && v > (thresholds[2] >> 1)
                        && v > (avg_16x16[m][i] >> 1) as i64)
                {
                    force_split[5 + m2 + i] = PART_EVAL_ONLY_SPLIT;
                    force_split[m + 1] = PART_EVAL_ONLY_SPLIT;
                    force_split[0] = PART_EVAL_ONLY_SPLIT;
                } else if !is_key_frame
                    && (cm.width * cm.height <= 640 * 360)
                    && ((maxvar_16x16[m][i].wrapping_sub(minvar_16x16[m][i]) as i64
                        > (thresholds[2] >> 1)
                        && maxvar_16x16[m][i] as i64 > thresholds[2])
                        || (cpi.sf.rt_sf.prefer_large_partition_blocks != 0
                            && x.content_state_sb.source_sad_nonrd > K_LOW_SAD
                            && cpi.rc.frame_source_sad < frame_sad_thresh
                            && maxvar_16x16[m][i] as i64 > (thresholds[2] >> 4)
                            && maxvar_16x16[m][i] > (minvar_16x16[m][i].wrapping_shl(2))))
                {
                    force_split[5 + m2 + i] = PART_EVAL_ONLY_SPLIT;
                    force_split[m + 1] = PART_EVAL_ONLY_SPLIT;
                    force_split[0] = PART_EVAL_ONLY_SPLIT;
                }
            }
        }
        if force_split[1 + m] == PART_EVAL_ALL {
            fill_variance_tree(&mut vt.split[m]);
            get_variance(&mut vt.split[m].part_variances.none);
            var_64x64 = vt.split[m].part_variances.none.variance;
            max_var_64x64 = var_64x64.max(max_var_64x64);
            min_var_64x64 = var_64x64.min(min_var_64x64);
            // If the difference of the max-min variances of sub-blocks or max
            // variance of a sub-block is above some threshold of then force this
            // block to split. Only checking this for noise level >= medium, if
            // encoder is in SVC or if we already forced large blocks.

            if !is_key_frame
                && (max_var_32x32[m].wrapping_sub(min_var_32x32[m])) as i64
                    > 3 * (thresholds[1] >> 3)
                && max_var_32x32[m] as i64 > thresholds[1] >> 1
                && (noise_level >= K_MEDIUM
                    || cpi.ppi.use_svc
                    || cpi.sf.rt_sf.prefer_large_partition_blocks != 0)
            {
                force_split[1 + m] = PART_EVAL_ONLY_SPLIT;
                force_split[0] = PART_EVAL_ONLY_SPLIT;
            }
            avg_64x64 = avg_64x64.wrapping_add(var_64x64);
        }
        if is_small_sb {
            force_split[0] = PART_EVAL_ONLY_SPLIT;
        }
    }

    if force_split[0] == PART_EVAL_ALL {
        fill_variance_tree(&mut vt);
        get_variance(&mut vt.part_variances.none);
        if !is_key_frame && vt.part_variances.none.variance as i64 > ((9 * avg_64x64 as i64) >> 5) {
            force_split[0] = PART_EVAL_ONLY_SPLIT;
        }

        if !is_key_frame
            && (max_var_64x64.wrapping_sub(min_var_64x64)) as i64 > 3 * (thresholds[0] >> 3)
            && max_var_64x64 as i64 > thresholds[0] >> 1
        {
            force_split[0] = PART_EVAL_ONLY_SPLIT;
        }
    }

    let xd = &mut x.e_mbd;
    if mi_col + 32 > tile.mi_col_end
        || mi_row + 32 > tile.mi_row_end
        || !set_vt_partitioning(
            cm,
            xd,
            tile,
            &mut vt,
            BLOCK_128X128,
            mi_row,
            mi_col,
            thresholds[0],
            BLOCK_16X16,
            force_split[0],
        )
    {
        for m in 0..num_64x64_blocks {
            let x64_idx = (m as i32 & 1) << 4;
            let y64_idx = (m as i32 >> 1) << 4;
            let m2 = m << 2;

            // Now go through the entire structure, splitting every block size until
            // we get to one that's got a variance lower than our threshold.
            if !set_vt_partitioning(
                cm,
                xd,
                tile,
                &mut vt.split[m],
                BLOCK_64X64,
                mi_row + y64_idx,
                mi_col + x64_idx,
                thresholds[1],
                BLOCK_16X16,
                force_split[1 + m],
            ) {
                for i in 0..4 {
                    let x32_idx = (i as i32 & 1) << 3;
                    let y32_idx = (i as i32 >> 1) << 3;
                    let i2 = (m2 + i) << 2;
                    if !set_vt_partitioning(
                        cm,
                        xd,
                        tile,
                        &mut vt.split[m].split[i],
                        BLOCK_32X32,
                        mi_row + y64_idx + y32_idx,
                        mi_col + x64_idx + x32_idx,
                        thresholds[2],
                        BLOCK_16X16,
                        force_split[5 + m2 + i],
                    ) {
                        for j in 0..4 {
                            let x16_idx = (j as i32 & 1) << 2;
                            let y16_idx = (j as i32 >> 1) << 2;
                            let split_index = 21 + i2 + j;
                            // For inter frames: if variance4x4downsample[] == 1 for this
                            // 16x16 block, then the variance is based on 4x4 down-sampling,
                            // so use vt2 in set_vt_partioning(), otherwise use vt.
                            // (vt2 is never allocated)
                            let vtemp = &mut vt.split[m].split[i].split[j];
                            if !set_vt_partitioning(
                                cm,
                                xd,
                                tile,
                                vtemp,
                                BLOCK_16X16,
                                mi_row + y64_idx + y32_idx + y16_idx,
                                mi_col + x64_idx + x32_idx + x16_idx,
                                thresholds[3],
                                BLOCK_8X8,
                                force_split[split_index],
                            ) {
                                for k in 0..4 {
                                    let x8_idx = (k & 1) << 1;
                                    let y8_idx = (k >> 1) << 1;
                                    set_block_size(
                                        cm,
                                        xd,
                                        mi_row + y64_idx + y32_idx + y16_idx + y8_idx,
                                        mi_col + x64_idx + x32_idx + x16_idx + x8_idx,
                                        BLOCK_8X8,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if cpi.sf.rt_sf.short_circuit_low_temp_var != 0 {
        set_low_temp_var_flag(
            cpi,
            &mut x.part_search_info,
            &x.e_mbd,
            &vt,
            &thresholds,
            ref_frame_partition,
            mi_col,
            mi_row,
        );
    }

    Ok(0)
}
