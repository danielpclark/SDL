// Rust translation of av1/common/tile_common.c and av1/common/tile_common.h
// from aom (https://aomedia.googlesource.com/aom, at the revision
// SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Tiles: the uniformly spaced layout the encoder uses (no superres, so
//! the tile rectangle needs no upscaling).

use super::av1_common_int::{Av1Common, CommonTileParams, SequenceHeader};
use super::enums::{MAX_TILE_COLS, MAX_TILE_ROWS, MI_SIZE_LOG2};
use crate::aom::aom_dsp::aom_dsp_common::ceil_power_of_two;

pub(crate) const DEFAULT_MAX_NUM_TG: i32 = 1;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TileInfo {
    pub(crate) mi_row_start: i32,
    pub(crate) mi_row_end: i32,
    pub(crate) mi_col_start: i32,
    pub(crate) mi_col_end: i32,
    pub(crate) tile_row: i32,
    pub(crate) tile_col: i32,
}

// Define tile maximum width and area
// There is no maximum height since height is limited by area and width limits
// The minimum tile width or height is fixed at one superblock
pub(crate) const MAX_TILE_WIDTH: i32 = 4096; // Max Tile width in pixels
pub(crate) const MAX_TILE_AREA: i32 = 4096 * 2304; // Maximum tile area in pixels

// initializes 'tile->mi_(row|col)_(start|end)' for (row, col) based on
// 'cm->log2_tile_(rows|cols)' & 'cm->mi_(rows|cols)'
pub(crate) fn av1_tile_init(tile: &mut TileInfo, cm: &Av1Common, row: i32, col: i32) {
    av1_tile_set_row(tile, cm, row);
    av1_tile_set_col(tile, cm, col);
}

// Find smallest k>=0 such that (blk_size << k) >= target
fn tile_log2(blk_size: i32, target: i32) -> i32 {
    let mut k = 0;
    while (blk_size << k) < target {
        k += 1;
    }
    k
}

pub(crate) fn av1_get_tile_limits(cm: &mut Av1Common) {
    let seq_params = &cm.seq_params;
    let tiles = &mut cm.tiles;
    let sb_cols = ceil_power_of_two(cm.mi_params.mi_cols, seq_params.mib_size_log2);
    let sb_rows = ceil_power_of_two(cm.mi_params.mi_rows, seq_params.mib_size_log2);

    let sb_size_log2 = seq_params.mib_size_log2 + MI_SIZE_LOG2;
    tiles.max_width_sb = MAX_TILE_WIDTH >> sb_size_log2;

    let max_tile_area_sb = MAX_TILE_AREA >> (2 * sb_size_log2);

    tiles.min_log2_cols = tile_log2(tiles.max_width_sb, sb_cols);
    tiles.max_log2_cols = tile_log2(1, sb_cols.min(MAX_TILE_COLS));
    tiles.max_log2_rows = tile_log2(1, sb_rows.min(MAX_TILE_ROWS));
    tiles.min_log2 = tile_log2(max_tile_area_sb, sb_cols * sb_rows);
    tiles.min_log2 = tiles.min_log2.max(tiles.min_log2_cols);
}

/// Translation of `av1_calculate_tile_cols()`, for uniform spacing (the
/// encoder sets no tile widths).
pub(crate) fn av1_calculate_tile_cols(
    seq_params: &SequenceHeader,
    cm_mi_rows: i32,
    cm_mi_cols: i32,
    tiles: &mut CommonTileParams,
) {
    let sb_cols = ceil_power_of_two(cm_mi_cols, seq_params.mib_size_log2);
    let sb_rows = ceil_power_of_two(cm_mi_rows, seq_params.mib_size_log2);
    let mut i: usize;

    // This will be overridden if there is at least two columns of tiles
    // (otherwise there is no inner tile width)
    tiles.min_inner_width = -1;

    debug_assert!(tiles.uniform_spacing != 0);
    let mut start_sb;
    let size_sb = ceil_power_of_two(sb_cols, tiles.log2_cols);
    debug_assert!(size_sb > 0);
    i = 0;
    start_sb = 0;
    while start_sb < sb_cols {
        tiles.col_start_sb[i] = start_sb;
        start_sb += size_sb;
        i += 1;
    }
    tiles.cols = i as i32;
    tiles.col_start_sb[i] = sb_cols;
    tiles.min_log2_rows = (tiles.min_log2 - tiles.log2_cols).max(0);
    tiles.max_height_sb = sb_rows >> tiles.min_log2_rows;

    tiles.width = size_sb << seq_params.mib_size_log2;
    tiles.width = tiles.width.min(cm_mi_cols);
    if tiles.cols > 1 {
        tiles.min_inner_width = tiles.width;
    }
}

/// Translation of `av1_calculate_tile_rows()`, for uniform spacing.
pub(crate) fn av1_calculate_tile_rows(
    seq_params: &SequenceHeader,
    cm_mi_rows: i32,
    tiles: &mut CommonTileParams,
) {
    let sb_rows = ceil_power_of_two(cm_mi_rows, seq_params.mib_size_log2);
    let mut start_sb;
    let size_sb;
    let mut i: usize;

    debug_assert!(tiles.uniform_spacing != 0);
    size_sb = ceil_power_of_two(sb_rows, tiles.log2_rows);
    debug_assert!(size_sb > 0);
    i = 0;
    start_sb = 0;
    while start_sb < sb_rows {
        tiles.row_start_sb[i] = start_sb;
        start_sb += size_sb;
        i += 1;
    }
    tiles.rows = i as i32;
    tiles.row_start_sb[i] = sb_rows;

    tiles.height = size_sb << seq_params.mib_size_log2;
    tiles.height = tiles.height.min(cm_mi_rows);
}

pub(crate) fn av1_tile_set_row(tile: &mut TileInfo, cm: &Av1Common, row: i32) {
    debug_assert!(row < cm.tiles.rows);
    let mi_row_start = cm.tiles.row_start_sb[row as usize] << cm.seq_params.mib_size_log2;
    let mi_row_end = cm.tiles.row_start_sb[row as usize + 1] << cm.seq_params.mib_size_log2;
    tile.tile_row = row;
    tile.mi_row_start = mi_row_start;
    tile.mi_row_end = mi_row_end.min(cm.mi_params.mi_rows);
    debug_assert!(tile.mi_row_end > tile.mi_row_start);
}

pub(crate) fn av1_tile_set_col(tile: &mut TileInfo, cm: &Av1Common, col: i32) {
    debug_assert!(col < cm.tiles.cols);
    let mi_col_start = cm.tiles.col_start_sb[col as usize] << cm.seq_params.mib_size_log2;
    let mi_col_end = cm.tiles.col_start_sb[col as usize + 1] << cm.seq_params.mib_size_log2;
    tile.tile_col = col;
    tile.mi_col_start = mi_col_start;
    tile.mi_col_end = mi_col_end.min(cm.mi_params.mi_cols);
    debug_assert!(tile.mi_col_end > tile.mi_col_start);
}

pub(crate) fn av1_get_sb_rows_in_tile(cm: &Av1Common, tile: &TileInfo) -> i32 {
    ceil_power_of_two(
        tile.mi_row_end - tile.mi_row_start,
        cm.seq_params.mib_size_log2,
    )
}

pub(crate) fn av1_get_sb_cols_in_tile(cm: &Av1Common, tile: &TileInfo) -> i32 {
    ceil_power_of_two(
        tile.mi_col_end - tile.mi_col_start,
        cm.seq_params.mib_size_log2,
    )
}

pub(crate) fn av1_get_uniform_tile_size(cm: &Av1Common, w: &mut i32, h: &mut i32) {
    let tiles = &cm.tiles;
    debug_assert!(tiles.uniform_spacing != 0);
    *w = tiles.width;
    *h = tiles.height;
}

// Checks if the minimum tile_width requirement is satisfied
pub(crate) fn av1_is_min_tile_width_satisfied(cm: &Av1Common) -> i32 {
    // Disable check if there is a single tile col in the frame
    if cm.tiles.cols == 1 {
        return 1;
    }

    // (no superres: av1_superres_scaled() is 0)
    ((cm.tiles.min_inner_width << MI_SIZE_LOG2) >= 64) as i32
}
