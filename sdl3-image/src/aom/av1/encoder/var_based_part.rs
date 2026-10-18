// Rust translation of the parts of av1/encoder/var_based_part.c (and the
// variance tree types of av1/encoder/encoder.h) from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the variance-based partitioning of key frames.
// Copyright (c) 2019, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Variance-based partitioning: a superblock's partition is chosen from the
//! variances of 4x4 averages of its source samples, before any mode search.

use super::encoder::{Av1Comp, ThreadData, ALLINTRA};
use crate::aom::aom_dsp::avg::{aom_avg_4x4, aom_highbd_avg_4x4};
use crate::aom::aom_scale::yv12config::YV12_FLAG_HIGHBITDEPTH;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::quant_common::av1_ac_quant_qtx;
use crate::aom::av1::common::tile_common::TileInfo;

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

/// Translation of `fill_variance_4x4avg()` for key frames (the 4x4 averages
/// are compared with 128).
#[inline]
fn fill_variance_4x4avg(
    src: &[u16],
    s: usize,
    sp: i32,
    x8_idx: i32,
    y8_idx: i32,
    vst: &mut VP8x8,
    highbd_flag: i32,
    pixels_wide: i32,
    pixels_high: i32,
    is_key_frame: bool,
    border_offset_4x4: i32,
) -> Result<(), ()> {
    for k in 0..4 {
        let x4_idx = x8_idx + ((k & 1) << 2);
        let y4_idx = y8_idx + ((k >> 1) << 2);
        let mut sse: u32 = 0;
        let mut sum: i32 = 0;
        if x4_idx < pixels_wide - border_offset_4x4 && y4_idx < pixels_high - border_offset_4x4 {
            let d_avg: i32 = 128;
            // (the inter frames' references: see choose_var_based_partitioning)
            if !is_key_frame {
                return Err(());
            }
            let s_avg = if highbd_flag & YV12_FLAG_HIGHBITDEPTH != 0 {
                aom_highbd_avg_4x4(
                    src,
                    (s as i64 + (y4_idx * sp + x4_idx) as i64) as usize,
                    sp as usize,
                ) as i32
            } else {
                aom_avg_4x4(
                    src,
                    (s as i64 + (y4_idx * sp + x4_idx) as i64) as usize,
                    sp as usize,
                ) as i32
            };
            sum = s_avg - d_avg;
            sse = sum.wrapping_mul(sum) as u32;
        }
        fill_variance(sse, sum, 0, &mut vst.split[k as usize].part_variances.none);
    }
    Ok(())
}

fn set_vbp_thresholds(cpi: &Av1Comp, thresholds: &mut [i64; 5], q: i32) {
    let cm = &cpi.common;
    let is_key_frame = frame_is_intra_only(cm);
    let threshold_multiplier = if is_key_frame { 120 } else { 1 };
    let ac_q = av1_ac_quant_qtx(q, 0, cm.seq_params.bit_depth) as i32;
    let mut threshold_base: i64 = (threshold_multiplier * ac_q) as i64;
    let threshold_left_shift = cpi.sf.rt_sf.var_part_split_threshold_shift;

    // (the saver's frames are all key frames: the thresholds of inter
    // frames, which depend on the content and noise, are left out)
    debug_assert!(is_key_frame);
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
}

pub(crate) fn av1_set_variance_partition_thresholds(cpi: &mut Av1Comp, q: i32) {
    if cpi.sf.part_sf.partition_search_type != VAR_BASED_PARTITION {
        return;
    }
    let mut thresholds = cpi.vbp_info.thresholds;
    set_vbp_thresholds(cpi, &mut thresholds, q);
    cpi.vbp_info.thresholds = thresholds;
    // The threshold below is not changed locally.
    cpi.vbp_info.threshold_minmax = 15 + (q >> 3);
}

/// `VAR_BASED_PARTITION` (speed_features.h)
pub(crate) use super::speed_features::VAR_BASED_PARTITION;

fn fill_variance_tree_leaves(
    cpi: &Av1Comp,
    xd: &MacroBlockD,
    vt: &mut VP128x128,
    force_split: &mut [i32; 85],
    avg_16x16: &mut [[i32; 4]; 4],
    maxvar_16x16: &mut [[i32; 4]; 4],
    minvar_16x16: &mut [[i32; 4]; 4],
    variance4x4downsample: &mut [i32; 64],
    src: &[u16],
    s: usize,
    src_stride: i32,
    is_key_frame: bool,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let is_small_sb = cm.seq_params.sb_size == BLOCK_64X64;
    let num_64x64_blocks = if is_small_sb { 1 } else { 4 };
    // TODO(kyslov) Bring back compute_minmax_variance with content type detection
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
    // (the saver's frames are all key frames: the 8x8 averages of inter
    // frames are left out)
    debug_assert!(is_key_frame);
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
                if is_key_frame {
                    force_split[split_index] = PART_EVAL_ALL;
                    // Go down to 4x4 down-sampling for variance.
                    variance4x4downsample[i2 + j] = 1;
                    for k in 0..4usize {
                        let x8_idx = x16_idx + ((k as i32 & 1) << 3);
                        let y8_idx = y16_idx + ((k as i32 >> 1) << 3);
                        let vst2 = &mut vst.split[k];
                        fill_variance_4x4avg(
                            src,
                            s,
                            src_stride,
                            x8_idx,
                            y8_idx,
                            vst2,
                            xd.cur_buf_flags,
                            pixels_wide,
                            pixels_high,
                            is_key_frame,
                            border_offset_4x4,
                        )?;
                    }
                }
            }
        }
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

/// Translation of `av1_choose_var_based_partitioning()` for key frames:
/// writes the partition into the mode info grid (moved into `td.mb.e_mbd`).
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

    let is_key_frame = frame_is_intra_only(cm);
    // (the saver's frames are all key frames: the inter frame searches of
    // the partitioning are left out)
    debug_assert!(is_key_frame);
    debug_assert!(cm.seq_params.sb_size == BLOCK_64X64 || cm.seq_params.sb_size == BLOCK_128X128);
    let is_small_sb = cm.seq_params.sb_size == BLOCK_64X64;
    let num_64x64_blocks = if is_small_sb { 1 } else { 4 };

    let ThreadData { mb: x, vt64x64, .. } = td;
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
    // (src_sad_blk_64x64 is a real-time feature; the adaptive quantization of
    // the cyclic refresh is off)
    set_vbp_thresholds(cpi, &mut thresholds, cm.quant_params.base_qindex);

    // For non keyframes, disable 4x4 average for low resolution when speed = 8

    let s = x.plane[0].src.buf;
    let sp = x.plane[0].src.stride;

    // Index for force_split: 0 for 64x64, 1-4 for 32x32 blocks,
    // 5-20 for the 16x16 blocks.
    force_split[0] = PART_EVAL_ALL;
    // (x->part_search_info.variance_low: only read by inter mode searches)

    // (chroma_check() returns at once for key frames)

    x.force_zeromv_skip_for_sb = 0;

    // Fill in the entire tree of 8x8 (or 4x4 under some conditions) variances
    // for splits.
    fill_variance_tree_leaves(
        cpi,
        &x.e_mbd,
        &mut vt,
        &mut force_split,
        &mut avg_16x16,
        &mut maxvar_16x16,
        &mut minvar_16x16,
        &mut variance4x4downsample,
        &cpi.source().buffer_alloc,
        s,
        sp,
        is_key_frame,
    )?;

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
            // (frame_sad_thresh is only used for inter frames)
            if force_split[5 + m2 + i] == PART_EVAL_ALL {
                get_variance(&mut vt.split[m].split[i].part_variances.none);
                var_32x32 = vt.split[m].split[i].part_variances.none.variance;
                max_var_32x32[m] = var_32x32.max(max_var_32x32[m]);
                min_var_32x32[m] = var_32x32.min(min_var_32x32[m]);
                if vt.split[m].split[i].part_variances.none.variance as i64 > thresholds[2] {
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
            // (inter frames only)
            avg_64x64 = avg_64x64.wrapping_add(var_64x64);
        }
        if is_small_sb {
            force_split[0] = PART_EVAL_ONLY_SPLIT;
        }
    }
    let _ = (
        avg_64x64,
        max_var_64x64,
        min_var_64x64,
        avg_16x16,
        maxvar_16x16,
        minvar_16x16,
    );

    if force_split[0] == PART_EVAL_ALL {
        fill_variance_tree(&mut vt);
        get_variance(&mut vt.part_variances.none);
        // (inter frames only)
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

    // (short_circuit_low_temp_var: inter frames only)
    Ok(0)
}
