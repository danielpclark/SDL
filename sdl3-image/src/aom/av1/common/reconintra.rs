// Rust translation of the parts of av1/common/reconintra.c and
// av1/common/reconintra.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Intra prediction, without palettes (screen content tools are off). The
//! samples are 16-bit for both bit depths; the low bit depth path
//! (`build_intra_predictors()`) computes what the high bit depth one
//! (`build_intra_predictors_high()`) computes at 8 bits, except that its
//! rectangular DC predictors use their own multipliers, so the two are one
//! function here with the bit depth and whether the buffer is high bit
//! depth as arguments. The predictor function tables are matched on the
//! mode and the transform size. The prediction is made in the
//! reconstructed frame (`xd.recon`), from its neighboring samples, as
//! upstream's callers pass the same buffer as `ref` and `dst`.

use super::av1_common_int::Av1Common;
use super::blockd::*;
use super::cfl::{
    cfl_load_dc_pred, cfl_predict_block, cfl_store_dc_pred, get_cfl_pred_type, is_cfl_allowed,
};
use super::enums::*;
use crate::aom::aom_dsp::aom_dsp_common::clip_pixel_highbd;
use crate::aom::aom_dsp::intrapred::*;

const NEED_LEFT: i32 = 1 << 1;
const NEED_ABOVE: i32 = 1 << 2;
const NEED_ABOVERIGHT: i32 = 1 << 3;
const NEED_ABOVELEFT: i32 = 1 << 4;
const NEED_BOTTOMLEFT: i32 = 1 << 5;

const INTRA_EDGE_FILT: usize = 3;
const INTRA_EDGE_TAPS: usize = 5;
const MAX_UPSAMPLE_SZ: i32 = 16;
const NUM_INTRA_NEIGHBOUR_PIXELS: usize = (MAX_TX_SIZE * 2 + 32) as usize;

static EXTEND_MODES: [i32; INTRA_MODES as usize] = [
    NEED_ABOVE | NEED_LEFT,                  // DC
    NEED_ABOVE,                              // V
    NEED_LEFT,                               // H
    NEED_ABOVE | NEED_ABOVERIGHT,            // D45
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // D135
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // D113
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // D157
    NEED_LEFT | NEED_BOTTOMLEFT,             // D203
    NEED_ABOVE | NEED_ABOVERIGHT,            // D67
    NEED_LEFT | NEED_ABOVE,                  // SMOOTH
    NEED_LEFT | NEED_ABOVE,                  // SMOOTH_V
    NEED_LEFT | NEED_ABOVE,                  // SMOOTH_H
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // PAETH
];

// Tables to store if the top-right reference pixels are available. The flags
// are represented with bits, packed into 8-bit integers. E.g., for the 32x32
// blocks in a 128x128 superblock, the index of the "o" block is 10 (in raster
// order), so its flag is stored at the 3rd bit of the 2nd entry in the table,
// i.e. (table[10 / 8] >> (10 % 8)) & 1.
//       . . . .
//       . . . .
//       . . o .
//       . . . .
static HAS_TR_4X4: [u8; 128] = [
    255, 255, 255, 255, 85, 85, 85, 85, 119, 119, 119, 119, 85, 85, 85, 85, 127, 127, 127, 127, 85,
    85, 85, 85, 119, 119, 119, 119, 85, 85, 85, 85, 255, 127, 255, 127, 85, 85, 85, 85, 119, 119,
    119, 119, 85, 85, 85, 85, 127, 127, 127, 127, 85, 85, 85, 85, 119, 119, 119, 119, 85, 85, 85,
    85, 255, 255, 255, 127, 85, 85, 85, 85, 119, 119, 119, 119, 85, 85, 85, 85, 127, 127, 127, 127,
    85, 85, 85, 85, 119, 119, 119, 119, 85, 85, 85, 85, 255, 127, 255, 127, 85, 85, 85, 85, 119,
    119, 119, 119, 85, 85, 85, 85, 127, 127, 127, 127, 85, 85, 85, 85, 119, 119, 119, 119, 85, 85,
    85, 85,
];
static HAS_TR_4X8: [u8; 64] = [
    255, 255, 255, 255, 119, 119, 119, 119, 127, 127, 127, 127, 119, 119, 119, 119, 255, 127, 255,
    127, 119, 119, 119, 119, 127, 127, 127, 127, 119, 119, 119, 119, 255, 255, 255, 127, 119, 119,
    119, 119, 127, 127, 127, 127, 119, 119, 119, 119, 255, 127, 255, 127, 119, 119, 119, 119, 127,
    127, 127, 127, 119, 119, 119, 119,
];
static HAS_TR_8X4: [u8; 64] = [
    255, 255, 0, 0, 85, 85, 0, 0, 119, 119, 0, 0, 85, 85, 0, 0, 127, 127, 0, 0, 85, 85, 0, 0, 119,
    119, 0, 0, 85, 85, 0, 0, 255, 127, 0, 0, 85, 85, 0, 0, 119, 119, 0, 0, 85, 85, 0, 0, 127, 127,
    0, 0, 85, 85, 0, 0, 119, 119, 0, 0, 85, 85, 0, 0,
];
static HAS_TR_8X8: [u8; 32] = [
    255, 255, 85, 85, 119, 119, 85, 85, 127, 127, 85, 85, 119, 119, 85, 85, 255, 127, 85, 85, 119,
    119, 85, 85, 127, 127, 85, 85, 119, 119, 85, 85,
];
static HAS_TR_8X16: [u8; 16] = [
    255, 255, 119, 119, 127, 127, 119, 119, 255, 127, 119, 119, 127, 127, 119, 119,
];
static HAS_TR_16X8: [u8; 16] = [255, 0, 85, 0, 119, 0, 85, 0, 127, 0, 85, 0, 119, 0, 85, 0];
static HAS_TR_16X16: [u8; 8] = [255, 85, 119, 85, 127, 85, 119, 85];
static HAS_TR_16X32: [u8; 4] = [255, 119, 127, 119];
static HAS_TR_32X16: [u8; 4] = [15, 5, 7, 5];
static HAS_TR_32X32: [u8; 2] = [95, 87];
static HAS_TR_32X64: [u8; 1] = [127];
static HAS_TR_64X32: [u8; 1] = [19];
static HAS_TR_64X64: [u8; 1] = [7];
static HAS_TR_64X128: [u8; 1] = [3];
static HAS_TR_128X64: [u8; 1] = [1];
static HAS_TR_128X128: [u8; 1] = [1];
static HAS_TR_4X16: [u8; 32] = [
    255, 255, 255, 255, 127, 127, 127, 127, 255, 127, 255, 127, 127, 127, 127, 127, 255, 255, 255,
    127, 127, 127, 127, 127, 255, 127, 255, 127, 127, 127, 127, 127,
];
static HAS_TR_16X4: [u8; 32] = [
    255, 0, 0, 0, 85, 0, 0, 0, 119, 0, 0, 0, 85, 0, 0, 0, 127, 0, 0, 0, 85, 0, 0, 0, 119, 0, 0, 0,
    85, 0, 0, 0,
];
static HAS_TR_8X32: [u8; 8] = [255, 255, 127, 127, 255, 127, 127, 127];
static HAS_TR_32X8: [u8; 8] = [15, 0, 5, 0, 7, 0, 5, 0];
static HAS_TR_16X64: [u8; 2] = [255, 127];
static HAS_TR_64X16: [u8; 2] = [3, 1];

static HAS_TR_TABLES: [&[u8]; BLOCK_SIZES_ALL as usize] = [
    // 4X4
    &HAS_TR_4X4,
    // 4X8,       8X4,            8X8
    &HAS_TR_4X8,
    &HAS_TR_8X4,
    &HAS_TR_8X8,
    // 8X16,      16X8,           16X16
    &HAS_TR_8X16,
    &HAS_TR_16X8,
    &HAS_TR_16X16,
    // 16X32,     32X16,          32X32
    &HAS_TR_16X32,
    &HAS_TR_32X16,
    &HAS_TR_32X32,
    // 32X64,     64X32,          64X64
    &HAS_TR_32X64,
    &HAS_TR_64X32,
    &HAS_TR_64X64,
    // 64x128,    128x64,         128x128
    &HAS_TR_64X128,
    &HAS_TR_128X64,
    &HAS_TR_128X128,
    // 4x16,      16x4,            8x32
    &HAS_TR_4X16,
    &HAS_TR_16X4,
    &HAS_TR_8X32,
    // 32x8,      16x64,           64x16
    &HAS_TR_32X8,
    &HAS_TR_16X64,
    &HAS_TR_64X16,
];

// (the _vert_* tables describe PARTITION_VERT_A and PARTITION_VERT_B, which
// the non-RD partition never uses: get_has_tr_table() and
// get_has_bl_table() fail for them)

fn get_has_tr_table(partition: PartitionType, bsize: BlockSize) -> Result<&'static [u8], ()> {
    // If this is a mixed vertical partition, look up bsize in orders_vert.
    if partition == PARTITION_VERT_A || partition == PARTITION_VERT_B {
        return Err(());
    }
    Ok(HAS_TR_TABLES[bsize as usize])
}

fn has_top_right(
    sb_size: BlockSize,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    top_available: bool,
    right_available: bool,
    partition: PartitionType,
    txsz: TxSize,
    row_off: i32,
    col_off: i32,
    ss_x: i32,
    ss_y: i32,
) -> Result<i32, ()> {
    if !top_available || !right_available {
        return Ok(0);
    }

    let bw_unit = MI_SIZE_WIDE[bsize as usize] as i32;
    let plane_bw_unit = (bw_unit >> ss_x).max(1);
    let top_right_count_unit = TX_SIZE_WIDE_UNIT[txsz as usize] as i32;

    if row_off > 0 {
        // Just need to check if enough pixels on the right.
        if BLOCK_SIZE_WIDE[bsize as usize] > BLOCK_SIZE_WIDE[BLOCK_64X64 as usize] {
            // Special case: For 128x128 blocks, the transform unit whose
            // top-right corner is at the center of the block does in fact have
            // pixels available at its top-right corner.
            if row_off == MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32 >> ss_y
                && col_off + top_right_count_unit
                    == MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32 >> ss_x
            {
                return Ok(1);
            }
            let plane_bw_unit_64 = MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32 >> ss_x;
            let col_off_64 = col_off % plane_bw_unit_64;
            return Ok((col_off_64 + top_right_count_unit < plane_bw_unit_64) as i32);
        }
        Ok((col_off + top_right_count_unit < plane_bw_unit) as i32)
    } else {
        // All top-right pixels are in the block above, which is already available.
        if col_off + top_right_count_unit < plane_bw_unit {
            return Ok(1);
        }

        let bw_in_mi_log2 = MI_SIZE_WIDE_LOG2[bsize as usize] as i32;
        let bh_in_mi_log2 = MI_SIZE_HIGH_LOG2[bsize as usize] as i32;
        let sb_mi_size = MI_SIZE_HIGH[sb_size as usize] as i32;
        let blk_row_in_sb = (mi_row & (sb_mi_size - 1)) >> bh_in_mi_log2;
        let blk_col_in_sb = (mi_col & (sb_mi_size - 1)) >> bw_in_mi_log2;

        // Top row of superblock: so top-right pixels are in the top and/or
        // top-right superblocks, both of which are already available.
        if blk_row_in_sb == 0 {
            return Ok(1);
        }

        // Rightmost column of superblock (and not the top row): so top-right pixels
        // fall in the right superblock, which is not available yet.
        if ((blk_col_in_sb + 1) << bw_in_mi_log2) >= sb_mi_size {
            return Ok(0);
        }

        // General case (neither top row nor rightmost column): check if the
        // top-right block is coded before the current block.
        let this_blk_index =
            ((blk_row_in_sb + 0) << (MAX_MIB_SIZE_LOG2 - bw_in_mi_log2)) + blk_col_in_sb + 0;
        let idx1 = (this_blk_index / 8) as usize;
        let idx2 = this_blk_index % 8;
        let has_tr_table = get_has_tr_table(partition, bsize)?;
        Ok(((has_tr_table[idx1] >> idx2) & 1) as i32)
    }
}

// Similar to the has_tr_* tables, but store if the bottom-left reference
// pixels are available.
static HAS_BL_4X4: [u8; 128] = [
    84, 85, 85, 85, 16, 17, 17, 17, 84, 85, 85, 85, 0, 1, 1, 1, 84, 85, 85, 85, 16, 17, 17, 17, 84,
    85, 85, 85, 0, 0, 1, 0, 84, 85, 85, 85, 16, 17, 17, 17, 84, 85, 85, 85, 0, 1, 1, 1, 84, 85, 85,
    85, 16, 17, 17, 17, 84, 85, 85, 85, 0, 0, 0, 0, 84, 85, 85, 85, 16, 17, 17, 17, 84, 85, 85, 85,
    0, 1, 1, 1, 84, 85, 85, 85, 16, 17, 17, 17, 84, 85, 85, 85, 0, 0, 1, 0, 84, 85, 85, 85, 16, 17,
    17, 17, 84, 85, 85, 85, 0, 1, 1, 1, 84, 85, 85, 85, 16, 17, 17, 17, 84, 85, 85, 85, 0, 0, 0, 0,
];
static HAS_BL_4X8: [u8; 64] = [
    16, 17, 17, 17, 0, 1, 1, 1, 16, 17, 17, 17, 0, 0, 1, 0, 16, 17, 17, 17, 0, 1, 1, 1, 16, 17, 17,
    17, 0, 0, 0, 0, 16, 17, 17, 17, 0, 1, 1, 1, 16, 17, 17, 17, 0, 0, 1, 0, 16, 17, 17, 17, 0, 1,
    1, 1, 16, 17, 17, 17, 0, 0, 0, 0,
];
static HAS_BL_8X4: [u8; 64] = [
    254, 255, 84, 85, 254, 255, 16, 17, 254, 255, 84, 85, 254, 255, 0, 1, 254, 255, 84, 85, 254,
    255, 16, 17, 254, 255, 84, 85, 254, 255, 0, 0, 254, 255, 84, 85, 254, 255, 16, 17, 254, 255,
    84, 85, 254, 255, 0, 1, 254, 255, 84, 85, 254, 255, 16, 17, 254, 255, 84, 85, 254, 255, 0, 0,
];
static HAS_BL_8X8: [u8; 32] = [
    84, 85, 16, 17, 84, 85, 0, 1, 84, 85, 16, 17, 84, 85, 0, 0, 84, 85, 16, 17, 84, 85, 0, 1, 84,
    85, 16, 17, 84, 85, 0, 0,
];
static HAS_BL_8X16: [u8; 16] = [16, 17, 0, 1, 16, 17, 0, 0, 16, 17, 0, 1, 16, 17, 0, 0];
static HAS_BL_16X8: [u8; 16] = [
    254, 84, 254, 16, 254, 84, 254, 0, 254, 84, 254, 16, 254, 84, 254, 0,
];
static HAS_BL_16X16: [u8; 8] = [84, 16, 84, 0, 84, 16, 84, 0];
static HAS_BL_16X32: [u8; 4] = [16, 0, 16, 0];
static HAS_BL_32X16: [u8; 4] = [78, 14, 78, 14];
static HAS_BL_32X32: [u8; 2] = [4, 4];
static HAS_BL_32X64: [u8; 1] = [0];
static HAS_BL_64X32: [u8; 1] = [34];
static HAS_BL_64X64: [u8; 1] = [0];
static HAS_BL_64X128: [u8; 1] = [0];
static HAS_BL_128X64: [u8; 1] = [0];
static HAS_BL_128X128: [u8; 1] = [0];
static HAS_BL_4X16: [u8; 32] = [
    0, 1, 1, 1, 0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 1, 1, 1, 0, 0, 0, 0,
];
static HAS_BL_16X4: [u8; 32] = [
    254, 254, 254, 84, 254, 254, 254, 16, 254, 254, 254, 84, 254, 254, 254, 0, 254, 254, 254, 84,
    254, 254, 254, 16, 254, 254, 254, 84, 254, 254, 254, 0,
];
static HAS_BL_8X32: [u8; 8] = [0, 1, 0, 0, 0, 1, 0, 0];
static HAS_BL_32X8: [u8; 8] = [238, 78, 238, 14, 238, 78, 238, 14];
static HAS_BL_16X64: [u8; 2] = [0, 0];
static HAS_BL_64X16: [u8; 2] = [42, 42];

static HAS_BL_TABLES: [&[u8]; BLOCK_SIZES_ALL as usize] = [
    // 4X4
    &HAS_BL_4X4,
    // 4X8,         8X4,         8X8
    &HAS_BL_4X8,
    &HAS_BL_8X4,
    &HAS_BL_8X8,
    // 8X16,        16X8,        16X16
    &HAS_BL_8X16,
    &HAS_BL_16X8,
    &HAS_BL_16X16,
    // 16X32,       32X16,       32X32
    &HAS_BL_16X32,
    &HAS_BL_32X16,
    &HAS_BL_32X32,
    // 32X64,       64X32,       64X64
    &HAS_BL_32X64,
    &HAS_BL_64X32,
    &HAS_BL_64X64,
    // 64x128,      128x64,      128x128
    &HAS_BL_64X128,
    &HAS_BL_128X64,
    &HAS_BL_128X128,
    // 4x16,        16x4,        8x32
    &HAS_BL_4X16,
    &HAS_BL_16X4,
    &HAS_BL_8X32,
    // 32x8,        16x64,       64x16
    &HAS_BL_32X8,
    &HAS_BL_16X64,
    &HAS_BL_64X16,
];

fn get_has_bl_table(partition: PartitionType, bsize: BlockSize) -> Result<&'static [u8], ()> {
    // If this is a mixed vertical partition, look up bsize in orders_vert.
    if partition == PARTITION_VERT_A || partition == PARTITION_VERT_B {
        return Err(());
    }
    Ok(HAS_BL_TABLES[bsize as usize])
}

fn has_bottom_left(
    sb_size: BlockSize,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    bottom_available: bool,
    left_available: bool,
    partition: PartitionType,
    txsz: TxSize,
    row_off: i32,
    col_off: i32,
    ss_x: i32,
    ss_y: i32,
) -> Result<i32, ()> {
    if !bottom_available || !left_available {
        return Ok(0);
    }

    // Special case for 128x* blocks, when col_off is half the block width.
    // This is needed because 128x* superblocks are divided into 64x* blocks in
    // raster order
    if BLOCK_SIZE_WIDE[bsize as usize] > BLOCK_SIZE_WIDE[BLOCK_64X64 as usize] && col_off > 0 {
        let plane_bw_unit_64 = MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32 >> ss_x;
        let col_off_64 = col_off % plane_bw_unit_64;
        if col_off_64 == 0 {
            // We are at the left edge of top-right or bottom-right 64x* block.
            let plane_bh_unit_64 = MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32 >> ss_y;
            let row_off_64 = row_off % plane_bh_unit_64;
            let plane_bh_unit = (MI_SIZE_HIGH[bsize as usize] as i32 >> ss_y).min(plane_bh_unit_64);
            // Check if all bottom-left pixels are in the left 64x* block (which is
            // already coded).
            return Ok(
                ((row_off_64 + TX_SIZE_HIGH_UNIT[txsz as usize] as i32) < plane_bh_unit) as i32,
            );
        }
    }

    if col_off > 0 {
        // Bottom-left pixels are in the bottom-left block, which is not available.
        Ok(0)
    } else {
        let bh_unit = MI_SIZE_HIGH[bsize as usize] as i32;
        let plane_bh_unit = (bh_unit >> ss_y).max(1);
        let bottom_left_count_unit = TX_SIZE_HIGH_UNIT[txsz as usize] as i32;

        // All bottom-left pixels are in the left block, which is already available.
        if row_off + bottom_left_count_unit < plane_bh_unit {
            return Ok(1);
        }

        let bw_in_mi_log2 = MI_SIZE_WIDE_LOG2[bsize as usize] as i32;
        let bh_in_mi_log2 = MI_SIZE_HIGH_LOG2[bsize as usize] as i32;
        let sb_mi_size = MI_SIZE_HIGH[sb_size as usize] as i32;
        let blk_row_in_sb = (mi_row & (sb_mi_size - 1)) >> bh_in_mi_log2;
        let blk_col_in_sb = (mi_col & (sb_mi_size - 1)) >> bw_in_mi_log2;

        // Leftmost column of superblock: so bottom-left pixels maybe in the left
        // and/or bottom-left superblocks. But only the left superblock is
        // available, so check if all required pixels fall in that superblock.
        if blk_col_in_sb == 0 {
            let blk_start_row_off =
                blk_row_in_sb << (bh_in_mi_log2 + MI_SIZE_LOG2 - MI_SIZE_LOG2) >> ss_y;
            let row_off_in_sb = blk_start_row_off + row_off;
            let sb_height_unit = sb_mi_size >> ss_y;
            return Ok((row_off_in_sb + bottom_left_count_unit < sb_height_unit) as i32);
        }

        // Bottom row of superblock (and not the leftmost column): so bottom-left
        // pixels fall in the bottom superblock, which is not available yet.
        if ((blk_row_in_sb + 1) << bh_in_mi_log2) >= sb_mi_size {
            return Ok(0);
        }

        // General case (neither leftmost column nor bottom row): check if the
        // bottom-left block is coded before the current block.
        let this_blk_index =
            ((blk_row_in_sb + 0) << (MAX_MIB_SIZE_LOG2 - bw_in_mi_log2)) + blk_col_in_sb + 0;
        let idx1 = (this_blk_index / 8) as usize;
        let idx2 = this_blk_index % 8;
        let has_bl_table = get_has_bl_table(partition, bsize)?;
        Ok(((has_bl_table[idx1] >> idx2) & 1) as i32)
    }
}

// (reconintra.h)

#[inline]
pub(crate) fn av1_is_directional_mode(mode: PredictionMode) -> bool {
    (V_PRED..=D67_PRED).contains(&mode)
}

#[inline]
pub(crate) fn av1_is_diagonal_mode(mode: PredictionMode) -> bool {
    (D45_PRED..=D67_PRED).contains(&mode)
}

#[inline]
pub(crate) fn av1_allow_intrabc(cm: &Av1Common) -> bool {
    super::av1_common_int::frame_is_intra_only(cm)
        && cm.features.allow_screen_content_tools
        && cm.features.allow_intrabc
}

#[inline]
pub(crate) fn av1_filter_intra_allowed_bsize(cm: &Av1Common, bs: BlockSize) -> bool {
    if cm.seq_params.enable_filter_intra == 0 || bs == BLOCK_INVALID {
        return false;
    }

    BLOCK_SIZE_WIDE[bs as usize] <= 32 && BLOCK_SIZE_HIGH[bs as usize] <= 32
}

#[inline]
pub(crate) fn av1_filter_intra_allowed(cm: &Av1Common, mbmi: &MbModeInfo) -> bool {
    mbmi.mode == DC_PRED
        && mbmi.palette_mode_info.palette_size[0] == 0
        && av1_filter_intra_allowed_bsize(cm, mbmi.bsize)
}

const FILTER_INTRA_SCALE_BITS: i32 = 4;

static DR_INTRA_DERIVATIVE: [i16; 90] = [
    // More evenly spread out angles and limited to 10-bit
    // Values that are 0 will never be used
    //                    Approx angle
    0, 0, 0, //
    1023, 0, 0, // 3, ...
    547, 0, 0, // 6, ...
    372, 0, 0, 0, 0, // 9, ...
    273, 0, 0, // 14, ...
    215, 0, 0, // 17, ...
    178, 0, 0, // 20, ...
    151, 0, 0, // 23, ... (113 & 203 are base angles)
    132, 0, 0, // 26, ...
    116, 0, 0, // 29, ...
    102, 0, 0, 0, // 32, ...
    90, 0, 0, // 36, ...
    80, 0, 0, // 39, ...
    71, 0, 0, // 42, ...
    64, 0, 0, // 45, ... (45 & 135 are base angles)
    57, 0, 0, // 48, ...
    51, 0, 0, // 51, ...
    45, 0, 0, 0, // 54, ...
    40, 0, 0, // 58, ...
    35, 0, 0, // 61, ...
    31, 0, 0, // 64, ...
    27, 0, 0, // 67, ... (67 & 157 are base angles)
    23, 0, 0, // 70, ...
    19, 0, 0, // 73, ...
    15, 0, 0, 0, 0, // 76, ...
    11, 0, 0, // 81, ...
    7, 0, 0, // 84, ...
    3, 0, 0, // 87, ...
];

// Get the shift (up-scaled by 256) in X w.r.t a unit change in Y.
// If angle > 0 && angle < 90, dx = -((int)(256 / t));
// If angle > 90 && angle < 180, dx = (int)(256 / t);
// If angle > 180 && angle < 270, dx = 1;
#[inline]
fn av1_get_dx(angle: i32) -> i32 {
    if angle > 0 && angle < 90 {
        DR_INTRA_DERIVATIVE[angle as usize] as i32
    } else if angle > 90 && angle < 180 {
        DR_INTRA_DERIVATIVE[(180 - angle) as usize] as i32
    } else {
        // In this case, we are not really going to use dx. We may return any value.
        1
    }
}

// Get the shift (up-scaled by 256) in Y w.r.t a unit change in X.
// If angle > 0 && angle < 90, dy = 1;
// If angle > 90 && angle < 180, dy = (int)(256 * t);
// If angle > 180 && angle < 270, dy = -((int)(256 * t));
#[inline]
fn av1_get_dy(angle: i32) -> i32 {
    if angle > 90 && angle < 180 {
        DR_INTRA_DERIVATIVE[(angle - 90) as usize] as i32
    } else if angle > 180 && angle < 270 {
        DR_INTRA_DERIVATIVE[(270 - angle) as usize] as i32
    } else {
        // In this case, we are not really going to use dy. We may return any value.
        1
    }
}

#[inline]
pub(crate) fn av1_use_intra_edge_upsample(bs0: i32, bs1: i32, delta: i32, ty: i32) -> bool {
    let d = delta.abs();
    let blk_wh = bs0 + bs1;
    if d == 0 || d >= 40 {
        return false;
    }
    if ty != 0 {
        blk_wh <= 8
    } else {
        blk_wh <= 16
    }
}

// (reconintra.c)

/// The neighbor sample buffers of `build_intra_predictors()`: `above_row`
/// and `left_col` are at offset 16 into them (`above_row[-1]` is
/// `data[15]`).
const AR: usize = 16;

// Directional prediction, zone 1: 0 < angle < 90
// (av1_dr_prediction_z1_c() and av1_highbd_dr_prediction_z1_c())
fn av1_dr_prediction_z1(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    upsample_above: i32,
    dx: i32,
    dy: i32,
) {
    debug_assert!(dy == 1);
    debug_assert!(dx > 0);
    let _ = dy;
    // (`above` is above_row: index AR + i is above[i])
    let ab = |i: i32| above[(AR as i32 + i) as usize];

    let max_base_x = ((bw + bh) - 1) << upsample_above;
    let frac_bits = 6 - upsample_above;
    let base_inc = 1 << upsample_above;
    let mut x = dx;
    let mut d = dst_off;
    for r in 0..bh {
        let mut base = x >> frac_bits;
        let shift = ((x << upsample_above) & 0x3F) >> 1;

        if base >= max_base_x {
            for _i in r..bh {
                dst[d..d + bw as usize].fill(ab(max_base_x));
                d += stride;
            }
            return;
        }

        for c in 0..bw as usize {
            if base < max_base_x {
                let val = ab(base) as i32 * (32 - shift) + ab(base + 1) as i32 * shift;
                dst[d + c] = ((val + 16) >> 5) as u16;
            } else {
                dst[d + c] = ab(max_base_x);
            }
            base += base_inc;
        }
        d += stride;
        x += dx;
    }
}

// Directional prediction, zone 2: 90 < angle < 180
fn av1_dr_prediction_z2(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    above: &[u16],
    left: &[u16],
    upsample_above: i32,
    upsample_left: i32,
    dx: i32,
    dy: i32,
) {
    debug_assert!(dx > 0);
    debug_assert!(dy > 0);
    let ab = |i: i32| above[(AR as i32 + i) as usize] as i32;
    let le = |i: i32| left[(AR as i32 + i) as usize] as i32;

    let min_base_x = -(1 << upsample_above);
    let min_base_y = -(1 << upsample_left);
    let _ = min_base_y;
    let frac_bits_x = 6 - upsample_above;
    let frac_bits_y = 6 - upsample_left;

    let mut d = dst_off;
    for r in 0..bh {
        for c in 0..bw {
            let mut val;
            let mut y = r + 1;
            let mut x = (c << 6) - y * dx;
            let base_x = x >> frac_bits_x;
            if base_x >= min_base_x {
                let shift = ((x * (1 << upsample_above)) & 0x3F) >> 1;
                val = ab(base_x) * (32 - shift) + ab(base_x + 1) * shift;
                val = (val + 16) >> 5;
            } else {
                x = c + 1;
                y = (r << 6) - x * dy;
                let base_y = y >> frac_bits_y;
                debug_assert!(base_y >= min_base_y);
                let shift = ((y * (1 << upsample_left)) & 0x3F) >> 1;
                val = le(base_y) * (32 - shift) + le(base_y + 1) * shift;
                val = (val + 16) >> 5;
            }
            dst[d + c as usize] = val as u16;
        }
        d += stride;
    }
}

// Directional prediction, zone 3: 180 < angle < 270
fn av1_dr_prediction_z3(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    bw: i32,
    bh: i32,
    left: &[u16],
    upsample_left: i32,
    dx: i32,
    dy: i32,
) {
    debug_assert!(dx == 1);
    debug_assert!(dy > 0);
    let _ = dx;
    let le = |i: i32| left[(AR as i32 + i) as usize];

    let max_base_y = (bw + bh - 1) << upsample_left;
    let frac_bits = 6 - upsample_left;
    let base_inc = 1 << upsample_left;
    let mut y = dy;
    for c in 0..bw as usize {
        let mut base = y >> frac_bits;
        let shift = ((y << upsample_left) & 0x3F) >> 1;

        let mut r = 0;
        while r < bh as usize {
            if base < max_base_y {
                let val = le(base) as i32 * (32 - shift) + le(base + 1) as i32 * shift;
                dst[dst_off + r * stride + c] = ((val + 16) >> 5) as u16;
            } else {
                while r < bh as usize {
                    dst[dst_off + r * stride + c] = le(max_base_y);
                    r += 1;
                }
                break;
            }
            r += 1;
            base += base_inc;
        }
        y += dy;
    }
}

/// `dr_predictor()` and `highbd_dr_predictor()`; `above` and `left` are the
/// neighbor buffers (`above_row` and `left_col` at offset `AR`).
fn dr_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    tx_size: TxSize,
    above: &[u16],
    left: &[u16],
    upsample_above: i32,
    upsample_left: i32,
    angle: i32,
    bd: i32,
) {
    let dx = av1_get_dx(angle);
    let dy = av1_get_dy(angle);
    let bw = TX_SIZE_WIDE[tx_size as usize];
    let bh = TX_SIZE_HIGH[tx_size as usize];
    debug_assert!(angle > 0 && angle < 270);

    if angle > 0 && angle < 90 {
        av1_dr_prediction_z1(dst, dst_off, stride, bw, bh, above, upsample_above, dx, dy);
    } else if angle > 90 && angle < 180 {
        av1_dr_prediction_z2(
            dst,
            dst_off,
            stride,
            bw,
            bh,
            above,
            left,
            upsample_above,
            upsample_left,
            dx,
            dy,
        );
    } else if angle > 180 && angle < 270 {
        av1_dr_prediction_z3(dst, dst_off, stride, bw, bh, left, upsample_left, dx, dy);
    } else if angle == 90 {
        highbd_v_predictor(dst, dst_off, stride, bw, bh, &above[AR..], &left[AR..], bd);
    } else if angle == 180 {
        highbd_h_predictor(dst, dst_off, stride, bw, bh, &above[AR..], &left[AR..], bd);
    }
}

static AV1_FILTER_INTRA_TAPS: [[[i8; 8]; 8]; FILTER_INTRA_MODES as usize] = [
    [
        [-6, 10, 0, 0, 0, 12, 0, 0],
        [-5, 2, 10, 0, 0, 9, 0, 0],
        [-3, 1, 1, 10, 0, 7, 0, 0],
        [-3, 1, 1, 2, 10, 5, 0, 0],
        [-4, 6, 0, 0, 0, 2, 12, 0],
        [-3, 2, 6, 0, 0, 2, 9, 0],
        [-3, 2, 2, 6, 0, 2, 7, 0],
        [-3, 1, 2, 2, 6, 3, 5, 0],
    ],
    [
        [-10, 16, 0, 0, 0, 10, 0, 0],
        [-6, 0, 16, 0, 0, 6, 0, 0],
        [-4, 0, 0, 16, 0, 4, 0, 0],
        [-2, 0, 0, 0, 16, 2, 0, 0],
        [-10, 16, 0, 0, 0, 0, 10, 0],
        [-6, 0, 16, 0, 0, 0, 6, 0],
        [-4, 0, 0, 16, 0, 0, 4, 0],
        [-2, 0, 0, 0, 16, 0, 2, 0],
    ],
    [
        [-8, 8, 0, 0, 0, 16, 0, 0],
        [-8, 0, 8, 0, 0, 16, 0, 0],
        [-8, 0, 0, 8, 0, 16, 0, 0],
        [-8, 0, 0, 0, 8, 16, 0, 0],
        [-4, 4, 0, 0, 0, 0, 16, 0],
        [-4, 0, 4, 0, 0, 0, 16, 0],
        [-4, 0, 0, 4, 0, 0, 16, 0],
        [-4, 0, 0, 0, 4, 0, 16, 0],
    ],
    [
        [-2, 8, 0, 0, 0, 10, 0, 0],
        [-1, 3, 8, 0, 0, 6, 0, 0],
        [-1, 2, 3, 8, 0, 4, 0, 0],
        [0, 1, 2, 3, 8, 2, 0, 0],
        [-1, 4, 0, 0, 0, 3, 10, 0],
        [-1, 3, 4, 0, 0, 4, 6, 0],
        [-1, 2, 3, 4, 0, 4, 4, 0],
        [-1, 2, 2, 3, 4, 3, 3, 0],
    ],
    [
        [-12, 14, 0, 0, 0, 14, 0, 0],
        [-10, 0, 14, 0, 0, 12, 0, 0],
        [-9, 0, 0, 14, 0, 11, 0, 0],
        [-8, 0, 0, 0, 14, 10, 0, 0],
        [-10, 12, 0, 0, 0, 0, 14, 0],
        [-9, 1, 12, 0, 0, 0, 12, 0],
        [-8, 0, 0, 12, 0, 1, 11, 0],
        [-7, 0, 0, 1, 12, 1, 9, 0],
    ],
];

/// `av1_filter_intra_predictor_c()` and `highbd_filter_intra_predictor()`
/// (`clip_pixel()` is `clip_pixel_highbd()` at 8 bits).
fn filter_intra_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    tx_size: TxSize,
    above: &[u16],
    left: &[u16],
    mode: FilterIntraMode,
    bd: i32,
) {
    let mut buffer = [[0u16; 33]; 33];
    let bw = TX_SIZE_WIDE[tx_size as usize] as usize;
    let bh = TX_SIZE_HIGH[tx_size as usize] as usize;

    debug_assert!(bw <= 32 && bh <= 32);

    for r in 0..bh {
        buffer[r + 1][0] = left[AR + r];
    }
    buffer[0][..bw + 1].copy_from_slice(&above[AR - 1..AR + bw]);

    let taps = &AV1_FILTER_INTRA_TAPS[mode as usize];
    let mut r = 1;
    while r < bh + 1 {
        let mut c = 1;
        while c < bw + 1 {
            let p0 = buffer[r - 1][c - 1] as i32;
            let p1 = buffer[r - 1][c] as i32;
            let p2 = buffer[r - 1][c + 1] as i32;
            let p3 = buffer[r - 1][c + 2] as i32;
            let p4 = buffer[r - 1][c + 3] as i32;
            let p5 = buffer[r][c - 1] as i32;
            let p6 = buffer[r + 1][c - 1] as i32;
            for k in 0..8 {
                let r_offset = k >> 2;
                let c_offset = k & 0x03;
                let t = &taps[k];
                let pr = t[0] as i32 * p0
                    + t[1] as i32 * p1
                    + t[2] as i32 * p2
                    + t[3] as i32 * p3
                    + t[4] as i32 * p4
                    + t[5] as i32 * p5
                    + t[6] as i32 * p6;
                // Section 7.11.2.3 specifies the right-hand side of the assignment as
                //   Clip1( Round2Signed( pr, INTRA_FILTER_SCALE_BITS ) ).
                // Since Clip1() clips a negative value to 0, it is safe to replace
                // Round2Signed() with Round2().
                buffer[r + r_offset][c + c_offset] = clip_pixel_highbd(
                    (pr + (1 << (FILTER_INTRA_SCALE_BITS - 1))) >> FILTER_INTRA_SCALE_BITS,
                    bd,
                );
            }
            c += 4;
        }
        r += 2;
    }

    let mut d = dst_off;
    for r in 0..bh {
        dst[d..d + bw].copy_from_slice(&buffer[r + 1][1..bw + 1]);
        d += stride;
    }
}

fn is_smooth(mbmi: &MbModeInfo, plane: usize) -> bool {
    if plane == 0 {
        let mode = mbmi.mode;
        mode == SMOOTH_PRED || mode == SMOOTH_V_PRED || mode == SMOOTH_H_PRED
    } else {
        // uv_mode is not set for inter blocks, so need to explicitly
        // detect that case.
        if is_inter_block(mbmi) {
            return false;
        }

        let uv_mode = mbmi.uv_mode;
        uv_mode == UV_SMOOTH_PRED || uv_mode == UV_SMOOTH_V_PRED || uv_mode == UV_SMOOTH_H_PRED
    }
}

fn get_intra_edge_filter_type(xd: &MacroBlockD, plane: usize) -> i32 {
    let ab_sm;
    let le_sm;

    if plane == 0 {
        let ab = xd.mi_at(xd.above_mbmi);
        let le = xd.mi_at(xd.left_mbmi);
        ab_sm = ab.is_some_and(|ab| is_smooth(ab, plane));
        le_sm = le.is_some_and(|le| is_smooth(le, plane));
    } else {
        let ab = xd.mi_at(xd.chroma_above_mbmi);
        let le = xd.mi_at(xd.chroma_left_mbmi);
        ab_sm = ab.is_some_and(|ab| is_smooth(ab, plane));
        le_sm = le.is_some_and(|le| is_smooth(le, plane));
    }

    (ab_sm || le_sm) as i32
}

fn intra_edge_filter_strength(bs0: i32, bs1: i32, delta: i32, ty: i32) -> i32 {
    let d = delta.abs();
    let mut strength = 0;

    let blk_wh = bs0 + bs1;
    if ty == 0 {
        if blk_wh <= 8 {
            if d >= 56 {
                strength = 1;
            }
        } else if blk_wh <= 12 {
            if d >= 40 {
                strength = 1;
            }
        } else if blk_wh <= 16 {
            if d >= 40 {
                strength = 1;
            }
        } else if blk_wh <= 24 {
            if d >= 8 {
                strength = 1;
            }
            if d >= 16 {
                strength = 2;
            }
            if d >= 32 {
                strength = 3;
            }
        } else if blk_wh <= 32 {
            if d >= 1 {
                strength = 1;
            }
            if d >= 4 {
                strength = 2;
            }
            if d >= 32 {
                strength = 3;
            }
        } else if d >= 1 {
            strength = 3;
        }
    } else if blk_wh <= 8 {
        if d >= 40 {
            strength = 1;
        }
        if d >= 64 {
            strength = 2;
        }
    } else if blk_wh <= 16 {
        if d >= 20 {
            strength = 1;
        }
        if d >= 48 {
            strength = 2;
        }
    } else if blk_wh <= 24 {
        if d >= 4 {
            strength = 3;
        }
    } else if d >= 1 {
        strength = 3;
    }
    strength
}

/// `av1_filter_intra_edge_c()` and `av1_filter_intra_edge_high_c()`: the
/// `sz` samples of `buf` from `p` on.
fn av1_filter_intra_edge(buf: &mut [u16], p: usize, sz: i32, strength: i32) {
    if strength == 0 {
        return;
    }

    const KERNEL: [[i32; INTRA_EDGE_TAPS]; INTRA_EDGE_FILT] =
        [[0, 4, 8, 4, 0], [0, 5, 6, 5, 0], [2, 4, 4, 4, 2]];
    let filt = (strength - 1) as usize;
    let mut edge = [0u16; 129];

    edge[..sz as usize].copy_from_slice(&buf[p..p + sz as usize]);
    for i in 1..sz {
        let mut s = 0;
        for j in 0..INTRA_EDGE_TAPS as i32 {
            let mut k = i - 2 + j;
            k = if k < 0 { 0 } else { k };
            k = if k > sz - 1 { sz - 1 } else { k };
            s += edge[k as usize] as i32 * KERNEL[filt][j as usize];
        }
        s = (s + 8) >> 4;
        buf[p + i as usize] = s as u16;
    }
}

/// `filter_intra_edge_corner()` and `filter_intra_edge_corner_high()`.
fn filter_intra_edge_corner(above: &mut [u16], left: &mut [u16]) {
    const KERNEL: [i32; 3] = [5, 6, 5];

    let mut s = (left[AR] as i32 * KERNEL[0])
        + (above[AR - 1] as i32 * KERNEL[1])
        + (above[AR] as i32 * KERNEL[2]);
    s = (s + 8) >> 4;
    above[AR - 1] = s as u16;
    left[AR - 1] = s as u16;
}

/// `av1_upsample_intra_edge_c()` and `av1_upsample_intra_edge_high_c()`
/// (`clip_pixel()` is `clip_pixel_highbd()` at 8 bits): `p` is the edge's
/// offset in `buf`.
fn av1_upsample_intra_edge(buf: &mut [u16], p: usize, sz: i32, bd: i32) {
    // interpolate half-sample positions
    debug_assert!(sz <= MAX_UPSAMPLE_SZ);

    let mut inp = [0u16; (MAX_UPSAMPLE_SZ + 3) as usize];
    // copy p[-1..(sz-1)] and extend first and last samples
    inp[0] = buf[p - 1];
    inp[1] = buf[p - 1];
    for i in 0..sz as usize {
        inp[i + 2] = buf[p + i];
    }
    inp[sz as usize + 2] = buf[p + sz as usize - 1];

    // interpolate half-sample edge positions
    buf[p - 2] = inp[0];
    for i in 0..sz as usize {
        let mut s = -(inp[i] as i32) + (9 * inp[i + 1] as i32) + (9 * inp[i + 2] as i32)
            - inp[i + 3] as i32;
        s = (s + 8) >> 4;
        buf[p + 2 * i - 1] = clip_pixel_highbd(s, bd);
        buf[p + 2 * i] = inp[i + 2];
    }
}

/// Translation of `build_intra_predictors()` and
/// `build_intra_predictors_high()` (see the module documentation); `ref`
/// and `dst` are the same plane of `buf`, at the offset `dst_off`, with
/// stride `dst_stride`.
fn build_intra_predictors(
    buf: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    mode: PredictionMode,
    p_angle: i32,
    filter_intra_mode: FilterIntraMode,
    tx_size: TxSize,
    disable_edge_filter: bool,
    n_top_px: i32,
    n_topright_px: i32,
    n_left_px: i32,
    n_bottomleft_px: i32,
    intra_edge_filter_type: i32,
    bit_depth: i32,
    is_hbd: bool,
) {
    let mut left_data = [0u16; NUM_INTRA_NEIGHBOUR_PIXELS];
    let mut above_data = [0u16; NUM_INTRA_NEIGHBOUR_PIXELS];
    // above_row = above_data + 16, left_col = left_data + 16
    let txwpx = TX_SIZE_WIDE[tx_size as usize];
    let txhpx = TX_SIZE_HIGH[tx_size as usize];
    let mut need_left = EXTEND_MODES[mode as usize] & NEED_LEFT != 0;
    let mut need_above = EXTEND_MODES[mode as usize] & NEED_ABOVE != 0;
    let mut need_above_left = EXTEND_MODES[mode as usize] & NEED_ABOVELEFT != 0;
    let above_ref = dst_off - dst_stride;
    let left_ref = dst_off - 1;
    let is_dr_mode = av1_is_directional_mode(mode);
    let use_filter_intra = filter_intra_mode != FILTER_INTRA_MODES;
    let base: i32 = 128 << (bit_depth - 8);
    // The left_data, above_data buffers must be zeroed to fix some intermittent
    // valgrind errors. Uninitialized reads in intra pred modules (e.g. width = 4
    // path in av1_highbd_dr_prediction_z2_avx2()) from left_data, above_data are
    // seen to be the potential reason for this issue.
    left_data.fill((base + 1) as u16);
    above_data.fill((base - 1) as u16);

    // The default values if ref pixels are not available:
    // base   base-1 base-1 .. base-1 base-1 base-1 base-1 base-1 base-1
    // base+1   A      B  ..     Y      Z
    // base+1   C      D  ..     W      X
    // base+1   E      F  ..     U      V
    // base+1   G      H  ..     S      T      T      T      T      T

    if is_dr_mode {
        if p_angle <= 90 {
            need_above = true;
            need_left = false;
            need_above_left = true;
        } else if p_angle < 180 {
            need_above = true;
            need_left = true;
            need_above_left = true;
        } else {
            need_above = false;
            need_left = true;
            need_above_left = true;
        }
    }
    if use_filter_intra {
        need_left = true;
        need_above = true;
        need_above_left = true;
    }

    debug_assert!(n_top_px >= 0);
    debug_assert!(n_topright_px >= -1);
    debug_assert!(n_left_px >= 0);
    debug_assert!(n_bottomleft_px >= -1);

    if (!need_above && n_left_px == 0) || (!need_left && n_top_px == 0) {
        let val: i32 = if need_left {
            if n_top_px > 0 {
                buf[above_ref] as i32
            } else {
                base + 1
            }
        } else if n_left_px > 0 {
            buf[left_ref] as i32
        } else {
            base - 1
        };
        let mut d = dst_off;
        for _i in 0..txhpx {
            buf[d..d + txwpx as usize].fill(val as u16);
            d += dst_stride;
        }
        return;
    }

    // NEED_LEFT
    if need_left {
        let num_left_pixels_needed = txhpx + if n_bottomleft_px >= 0 { txwpx } else { 0 };
        let mut i: i32 = 0;
        if n_left_px > 0 {
            while i < n_left_px {
                left_data[AR + i as usize] = buf[left_ref + i as usize * dst_stride];
                i += 1;
            }
            if n_bottomleft_px > 0 {
                debug_assert!(i == txhpx);
                while i < txhpx + n_bottomleft_px {
                    left_data[AR + i as usize] = buf[left_ref + i as usize * dst_stride];
                    i += 1;
                }
            }
            if i < num_left_pixels_needed {
                let v = left_data[AR + i as usize - 1];
                left_data[AR + i as usize..AR + num_left_pixels_needed as usize].fill(v);
            }
        } else if n_top_px > 0 {
            let v = buf[above_ref];
            left_data[AR..AR + num_left_pixels_needed as usize].fill(v);
        }
    }

    // NEED_ABOVE
    if need_above {
        let num_top_pixels_needed = txwpx + if n_topright_px >= 0 { txhpx } else { 0 };
        if n_top_px > 0 {
            above_data[AR..AR + n_top_px as usize]
                .copy_from_slice(&buf[above_ref..above_ref + n_top_px as usize]);
            let mut i = n_top_px;
            if n_topright_px > 0 {
                debug_assert!(n_top_px == txwpx);
                let (s, n) = ((above_ref + txwpx as usize), n_topright_px as usize);
                above_data[AR + txwpx as usize..AR + txwpx as usize + n]
                    .copy_from_slice(&buf[s..s + n]);
                i += n_topright_px;
            }
            if i < num_top_pixels_needed {
                let v = above_data[AR + i as usize - 1];
                above_data[AR + i as usize..AR + num_top_pixels_needed as usize].fill(v);
            }
        } else if n_left_px > 0 {
            let v = buf[left_ref];
            above_data[AR..AR + num_top_pixels_needed as usize].fill(v);
        }
    }

    if need_above_left {
        if n_top_px > 0 && n_left_px > 0 {
            above_data[AR - 1] = buf[above_ref - 1];
        } else if n_top_px > 0 {
            above_data[AR - 1] = buf[above_ref];
        } else if n_left_px > 0 {
            above_data[AR - 1] = buf[left_ref];
        } else {
            above_data[AR - 1] = base as u16;
        }
        left_data[AR - 1] = above_data[AR - 1];
    }

    if use_filter_intra {
        filter_intra_predictor(
            buf,
            dst_off,
            dst_stride,
            tx_size,
            &above_data,
            &left_data,
            filter_intra_mode,
            bit_depth,
        );
        return;
    }

    if is_dr_mode {
        let mut upsample_above = 0;
        let mut upsample_left = 0;
        if !disable_edge_filter {
            let need_right = p_angle < 90;
            let need_bottom = p_angle > 180;
            if p_angle != 90 && p_angle != 180 {
                let ab_le = if need_above_left { 1 } else { 0 };
                if need_above && need_left && (txwpx + txhpx >= 24) {
                    filter_intra_edge_corner(&mut above_data, &mut left_data);
                }
                if need_above && n_top_px > 0 {
                    let strength = intra_edge_filter_strength(
                        txwpx,
                        txhpx,
                        p_angle - 90,
                        intra_edge_filter_type,
                    );
                    let n_px = n_top_px + ab_le + if need_right { txhpx } else { 0 };
                    av1_filter_intra_edge(&mut above_data, AR - ab_le as usize, n_px, strength);
                }
                if need_left && n_left_px > 0 {
                    let strength = intra_edge_filter_strength(
                        txhpx,
                        txwpx,
                        p_angle - 180,
                        intra_edge_filter_type,
                    );
                    let n_px = n_left_px + ab_le + if need_bottom { txwpx } else { 0 };
                    av1_filter_intra_edge(&mut left_data, AR - ab_le as usize, n_px, strength);
                }
            }
            upsample_above =
                av1_use_intra_edge_upsample(txwpx, txhpx, p_angle - 90, intra_edge_filter_type)
                    as i32;
            if need_above && upsample_above != 0 {
                let n_px = txwpx + if need_right { txhpx } else { 0 };
                av1_upsample_intra_edge(&mut above_data, AR, n_px, bit_depth);
            }
            upsample_left =
                av1_use_intra_edge_upsample(txhpx, txwpx, p_angle - 180, intra_edge_filter_type)
                    as i32;
            if need_left && upsample_left != 0 {
                let n_px = txhpx + if need_bottom { txwpx } else { 0 };
                av1_upsample_intra_edge(&mut left_data, AR, n_px, bit_depth);
            }
        }
        dr_predictor(
            buf,
            dst_off,
            dst_stride,
            tx_size,
            &above_data,
            &left_data,
            upsample_above,
            upsample_left,
            p_angle,
            bit_depth,
        );
        return;
    }

    // predict
    let above_row = &above_data[AR..];
    let left_col = &left_data[AR..];
    let bd = bit_depth;
    if mode == DC_PRED {
        match (n_left_px > 0, n_top_px > 0) {
            (false, false) => highbd_dc_128_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bd,
            ),
            (false, true) => highbd_dc_top_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bd,
            ),
            (true, false) => highbd_dc_left_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bd,
            ),
            (true, true) => {
                if is_hbd {
                    highbd_dc_predictor(
                        buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bd,
                    )
                } else {
                    dc_predictor(buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col)
                }
            }
        }
    } else {
        match mode {
            SMOOTH_PRED => highbd_smooth_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bd,
            ),
            SMOOTH_V_PRED => highbd_smooth_v_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bd,
            ),
            SMOOTH_H_PRED => highbd_smooth_h_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bd,
            ),
            _ => {
                // (PAETH_PRED: the remaining non-directional mode)
                debug_assert!(mode == PAETH_PRED);
                highbd_paeth_predictor(
                    buf,
                    dst_off,
                    dst_stride,
                    txwpx,
                    txhpx,
                    &above_data[AR - 1..],
                    left_col,
                    bd,
                )
            }
        }
    }
}

#[inline]
fn scale_chroma_bsize(bsize: BlockSize, subsampling_x: i32, subsampling_y: i32) -> BlockSize {
    debug_assert!((0..2).contains(&subsampling_x));
    debug_assert!((0..2).contains(&subsampling_y));
    let mut bs = bsize;
    match bsize {
        BLOCK_4X4 => {
            if subsampling_x == 1 && subsampling_y == 1 {
                bs = BLOCK_8X8;
            } else if subsampling_x == 1 {
                bs = BLOCK_8X4;
            } else if subsampling_y == 1 {
                bs = BLOCK_4X8;
            }
        }
        BLOCK_4X8 => {
            if subsampling_x == 1 && subsampling_y == 1 {
                bs = BLOCK_8X8;
            } else if subsampling_x == 1 {
                bs = BLOCK_8X8;
            } else if subsampling_y == 1 {
                bs = BLOCK_4X8;
            }
        }
        BLOCK_8X4 => {
            if subsampling_x == 1 && subsampling_y == 1 {
                bs = BLOCK_8X8;
            } else if subsampling_x == 1 {
                bs = BLOCK_8X4;
            } else if subsampling_y == 1 {
                bs = BLOCK_8X8;
            }
        }
        BLOCK_4X16 => {
            if subsampling_x == 1 && subsampling_y == 1 {
                bs = BLOCK_8X16;
            } else if subsampling_x == 1 {
                bs = BLOCK_8X16;
            } else if subsampling_y == 1 {
                bs = BLOCK_4X16;
            }
        }
        BLOCK_16X4 => {
            if subsampling_x == 1 && subsampling_y == 1 {
                bs = BLOCK_16X8;
            } else if subsampling_x == 1 {
                bs = BLOCK_16X4;
            } else if subsampling_y == 1 {
                bs = BLOCK_16X8;
            }
        }
        _ => {}
    }
    bs
}

/// Translation of `av1_predict_intra_block()` (no palette), predicting into
/// the reconstructed frame `xd.recon` at `dst_off` from its neighbors.
pub(crate) fn av1_predict_intra_block(
    xd: &mut MacroBlockD,
    sb_size: BlockSize,
    enable_intra_edge_filter: bool,
    wpx: i32,
    hpx: i32,
    tx_size: TxSize,
    mode: PredictionMode,
    angle_delta: i32,
    use_palette: bool,
    filter_intra_mode: FilterIntraMode,
    dst_off: usize,
    dst_stride: usize,
    col_off: i32,
    row_off: i32,
    plane: usize,
) -> Result<(), ()> {
    let mbmi = *xd.mbmi();
    let txwpx = TX_SIZE_WIDE[tx_size as usize];
    let txhpx = TX_SIZE_HIGH[tx_size as usize];
    let x = col_off << MI_SIZE_LOG2;
    let y = row_off << MI_SIZE_LOG2;

    // (palettes need screen content tools, which are off)
    if use_palette {
        return Err(());
    }

    let pd = &xd.plane[plane];
    let txw = TX_SIZE_WIDE_UNIT[tx_size as usize] as i32;
    let txh = TX_SIZE_HIGH_UNIT[tx_size as usize] as i32;
    let ss_x = pd.subsampling_x;
    let ss_y = pd.subsampling_y;
    let have_top = row_off != 0
        || if ss_y != 0 {
            xd.chroma_up_available
        } else {
            xd.up_available
        };
    let have_left = col_off != 0
        || if ss_x != 0 {
            xd.chroma_left_available
        } else {
            xd.left_available
        };
    let mi_row = -xd.mb_to_top_edge >> (3 + MI_SIZE_LOG2);
    let mi_col = -xd.mb_to_left_edge >> (3 + MI_SIZE_LOG2);

    // Distance between the right edge of this prediction block to
    // the frame right edge
    let xr = (xd.mb_to_right_edge >> (3 + ss_x)) + wpx - x - txwpx;
    // Distance between the bottom edge of this prediction block to
    // the frame bottom edge
    let yd = (xd.mb_to_bottom_edge >> (3 + ss_y)) + hpx - y - txhpx;
    let right_available = mi_col + ((col_off + txw) << ss_x) < xd.tile.mi_col_end;
    let bottom_available = (yd > 0) && (mi_row + ((row_off + txh) << ss_y) < xd.tile.mi_row_end);

    let partition = mbmi.partition;

    let mut bsize = mbmi.bsize;
    // force 4x4 chroma component block size.
    if ss_x != 0 || ss_y != 0 {
        bsize = scale_chroma_bsize(bsize, ss_x, ss_y);
    }

    let is_dr_mode = av1_is_directional_mode(mode);
    let use_filter_intra = filter_intra_mode != FILTER_INTRA_MODES;
    let mut p_angle = 0;
    let mut need_top_right = EXTEND_MODES[mode as usize] & NEED_ABOVERIGHT != 0;
    let mut need_bottom_left = EXTEND_MODES[mode as usize] & NEED_BOTTOMLEFT != 0;

    if use_filter_intra {
        need_top_right = false;
        need_bottom_left = false;
    }
    if is_dr_mode {
        p_angle = MODE_TO_ANGLE_MAP[mode as usize] as i32 + angle_delta;
        need_top_right = p_angle < 90;
        need_bottom_left = p_angle > 180;
    }

    // Possible states for have_top_right(TR) and have_bottom_left(BL)
    // -1 : TR and BL are not needed
    //  0 : TR and BL are needed but not available
    // > 0 : TR and BL are needed and pixels are available
    let have_top_right = if need_top_right {
        has_top_right(
            sb_size,
            bsize,
            mi_row,
            mi_col,
            have_top,
            right_available,
            partition,
            tx_size,
            row_off,
            col_off,
            ss_x,
            ss_y,
        )?
    } else {
        -1
    };
    let have_bottom_left = if need_bottom_left {
        has_bottom_left(
            sb_size,
            bsize,
            mi_row,
            mi_col,
            bottom_available,
            have_left,
            partition,
            tx_size,
            row_off,
            col_off,
            ss_x,
            ss_y,
        )?
    } else {
        -1
    };

    let disable_edge_filter = !enable_intra_edge_filter;
    let intra_edge_filter_type = get_intra_edge_filter_type(xd, plane);
    let is_hbd = is_cur_buf_hbd(xd);
    let bd = if is_hbd { xd.bd } else { 8 };
    build_intra_predictors(
        &mut xd.recon.buffer_alloc,
        dst_off,
        dst_stride,
        mode,
        p_angle,
        filter_intra_mode,
        tx_size,
        disable_edge_filter,
        if have_top { txwpx.min(xr + txwpx) } else { 0 },
        if have_top_right > 0 {
            txwpx.min(xr)
        } else {
            have_top_right
        },
        if have_left { txhpx.min(yd + txhpx) } else { 0 },
        if have_bottom_left > 0 {
            txhpx.min(yd)
        } else {
            have_bottom_left
        },
        intra_edge_filter_type,
        bd,
        is_hbd,
    );
    Ok(())
}

pub(crate) fn av1_predict_intra_block_facade(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    plane: usize,
    blk_col: i32,
    blk_row: i32,
    tx_size: TxSize,
) -> Result<(), ()> {
    let mbmi = *xd.mbmi();
    let pd = &xd.plane[plane];
    let dst_stride = pd.dst.stride;
    let dst = pd.dst.buf + ((blk_row * dst_stride + blk_col) << MI_SIZE_LOG2) as usize;
    let mode = if plane == AOM_PLANE_Y {
        mbmi.mode
    } else {
        get_uv_mode(mbmi.uv_mode)
    };
    let use_palette = mbmi.palette_mode_info.palette_size[(plane != 0) as usize] > 0;
    let filter_intra_mode =
        if plane == AOM_PLANE_Y && mbmi.filter_intra_mode_info.use_filter_intra != 0 {
            mbmi.filter_intra_mode_info.filter_intra_mode
        } else {
            FILTER_INTRA_MODES
        };
    let angle_delta = mbmi.angle_delta[(plane != AOM_PLANE_Y) as usize] as i32 * ANGLE_STEP;
    let seq_params = &cm.seq_params;
    let (pw, ph) = (pd.width as i32, pd.height as i32);

    if plane != AOM_PLANE_Y && mbmi.uv_mode == UV_CFL_PRED {
        debug_assert!(is_cfl_allowed(xd) != 0);
        let pred_plane = get_cfl_pred_type(plane);
        if xd.cfl.dc_pred_is_cached[pred_plane] == 0 {
            av1_predict_intra_block(
                xd,
                seq_params.sb_size,
                seq_params.enable_intra_edge_filter != 0,
                pw,
                ph,
                tx_size,
                mode,
                angle_delta,
                use_palette,
                filter_intra_mode,
                dst,
                dst_stride as usize,
                blk_col,
                blk_row,
                plane,
            )?;
            if xd.cfl.use_dc_pred_cache != 0 {
                cfl_store_dc_pred(xd, dst, pred_plane, TX_SIZE_WIDE[tx_size as usize]);
                xd.cfl.dc_pred_is_cached[pred_plane] = 1;
            }
        } else {
            cfl_load_dc_pred(xd, dst, dst_stride as usize, tx_size, pred_plane);
        }
        return cfl_predict_block(xd, dst, dst_stride as usize, tx_size, plane);
    }
    av1_predict_intra_block(
        xd,
        seq_params.sb_size,
        seq_params.enable_intra_edge_filter != 0,
        pw,
        ph,
        tx_size,
        mode,
        angle_delta,
        use_palette,
        filter_intra_mode,
        dst,
        dst_stride as usize,
        blk_col,
        blk_row,
        plane,
    )
}
