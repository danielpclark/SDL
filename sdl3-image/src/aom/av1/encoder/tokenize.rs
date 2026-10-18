// Rust translation of the parts of av1/encoder/tokenize.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the real-time encoder
// reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transform block contexts of inter blocks (variable transform sizes).

use super::encoder::{Av1Comp, ThreadData};
use super::encodetxb::{av1_record_txb_context, av1_update_and_record_txb_context, TokenizeBArgs};
use crate::aom::av1::common::av1_common_int::av1_num_planes;
use crate::aom::av1::common::av1_loopfilter::av1_get_txb_size_index;
use crate::aom::av1::common::blockd::{
    av1_get_max_uv_txsize, av1_reset_entropy_context, get_plane_block_size, get_vartx_max_txsize,
    max_block_high, max_block_wide,
};
use crate::aom::av1::common::enums::*;

fn tokenize_vartx(
    tx_size: TxSize,
    mut plane_bsize: BlockSize,
    blk_row: i32,
    blk_col: i32,
    mut block: i32,
    plane: usize,
    arg: &mut TokenizeBArgs,
) {
    let x = &arg.td.mb;
    let xd = &x.e_mbd;
    let mbmi = xd.mbmi();
    let pd = &xd.plane[plane];
    let max_blocks_high = max_block_high(xd, plane_bsize, plane);
    let max_blocks_wide = max_block_wide(xd, plane_bsize, plane);

    if blk_row >= max_blocks_high || blk_col >= max_blocks_wide {
        return;
    }

    let plane_tx_size = if plane != 0 {
        av1_get_max_uv_txsize(mbmi.bsize, pd.subsampling_x, pd.subsampling_y)
    } else {
        mbmi.inter_tx_size[av1_get_txb_size_index(plane_bsize, blk_row, blk_col)]
    };

    if tx_size == plane_tx_size || plane != 0 {
        plane_bsize = get_plane_block_size(mbmi.bsize, pd.subsampling_x, pd.subsampling_y);

        if arg.allow_update_cdf != 0 {
            av1_update_and_record_txb_context(
                plane,
                block,
                blk_row,
                blk_col,
                plane_bsize,
                tx_size,
                arg,
            );
        } else {
            av1_record_txb_context(plane, block, blk_row, blk_col, plane_bsize, tx_size, arg);
        }
    } else {
        // Half the block size in transform block unit.
        let sub_txs = SUB_TX_SIZE_MAP[tx_size as usize];
        let bsw = TX_SIZE_WIDE_UNIT[sub_txs as usize] as i32;
        let bsh = TX_SIZE_HIGH_UNIT[sub_txs as usize] as i32;
        let step = bsw * bsh;
        let row_end = (TX_SIZE_HIGH_UNIT[tx_size as usize] as i32).min(max_blocks_high - blk_row);
        let col_end = (TX_SIZE_WIDE_UNIT[tx_size as usize] as i32).min(max_blocks_wide - blk_col);

        debug_assert!(bsw > 0 && bsh > 0);

        let mut row = 0;
        while row < row_end {
            let offsetr = blk_row + row;
            let mut col = 0;
            while col < col_end {
                let offsetc = blk_col + col;

                tokenize_vartx(sub_txs, plane_bsize, offsetr, offsetc, block, plane, arg);
                block += step;
                col += bsw;
            }
            row += bsh;
        }
    }
}

/// Translation of `av1_tokenize_sb_vartx()` (with a NULL `rate`).
pub(crate) fn av1_tokenize_sb_vartx(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    dry_run: i32,
    bsize: BlockSize,
    allow_update_cdf: bool,
) -> Result<(), ()> {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let cm = &cpi.common;
    let xd = &mut td.mb.e_mbd;
    let mi_row = xd.mi_row;
    let mi_col = xd.mi_col;
    if mi_row >= cm.mi_params.mi_rows || mi_col >= cm.mi_params.mi_cols {
        return Ok(());
    }

    let num_planes = av1_num_planes(cm);
    if xd.mbmi().skip_txfm != 0 {
        av1_reset_entropy_context(xd, bsize, num_planes);
        return Ok(());
    }
    let mut arg = TokenizeBArgs {
        cpi,
        td,
        this_rate: 0,
        allow_update_cdf: allow_update_cdf as u8,
        dry_run,
    };

    for plane in 0..num_planes {
        let xd = &arg.td.mb.e_mbd;
        if plane != 0 && !xd.is_chroma_ref {
            break;
        }
        let pd = &xd.plane[plane];
        let ss_x = pd.subsampling_x;
        let ss_y = pd.subsampling_y;
        let plane_bsize = get_plane_block_size(bsize, ss_x, ss_y);
        debug_assert!(plane_bsize < BLOCK_SIZES_ALL);
        let mi_width = MI_SIZE_WIDE[plane_bsize as usize] as i32;
        let mi_height = MI_SIZE_HIGH[plane_bsize as usize] as i32;
        let max_tx_size = get_vartx_max_txsize(xd, plane_bsize, plane);
        let txb_size = TXSIZE_TO_BSIZE[max_tx_size as usize];
        let bw = MI_SIZE_WIDE[txb_size as usize] as i32;
        let bh = MI_SIZE_HIGH[txb_size as usize] as i32;
        let mut block = 0;
        let step = TX_SIZE_WIDE_UNIT[max_tx_size as usize] as i32
            * TX_SIZE_HIGH_UNIT[max_tx_size as usize] as i32;

        let max_unit_bsize = get_plane_block_size(BLOCK_64X64, ss_x, ss_y);
        let mut mu_blocks_wide = MI_SIZE_WIDE[max_unit_bsize as usize] as i32;
        let mut mu_blocks_high = MI_SIZE_HIGH[max_unit_bsize as usize] as i32;

        mu_blocks_wide = mi_width.min(mu_blocks_wide);
        mu_blocks_high = mi_height.min(mu_blocks_high);

        let mut idy = 0;
        while idy < mi_height {
            let mut idx = 0;
            while idx < mi_width {
                let unit_height = (mu_blocks_high + idy).min(mi_height);
                let unit_width = (mu_blocks_wide + idx).min(mi_width);
                let mut blk_row = idy;
                while blk_row < unit_height {
                    let mut blk_col = idx;
                    while blk_col < unit_width {
                        tokenize_vartx(
                            max_tx_size,
                            plane_bsize,
                            blk_row,
                            blk_col,
                            block,
                            plane,
                            &mut arg,
                        );
                        block += step;
                        blk_col += bw;
                    }
                    blk_row += bh;
                }
                idx += mu_blocks_wide;
            }
            idy += mu_blocks_high;
        }
    }
    Ok(())
}
