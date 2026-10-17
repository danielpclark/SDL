// Rust translation of the parts of av1/common/pred_common.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The symbol contexts derived from the neighboring blocks that intra
//! frames use.

use super::blockd::{is_inter_block, MacroBlockD};
use super::enums::*;

#[inline]
pub(crate) fn av1_get_skip_txfm_context(xd: &MacroBlockD) -> i32 {
    let above_mi = xd.mi_at(xd.above_mbmi);
    let left_mi = xd.mi_at(xd.left_mbmi);
    let above_skip_txfm = above_mi.map_or(0, |m| m.skip_txfm as i32);
    let left_skip_txfm = left_mi.map_or(0, |m| m.skip_txfm as i32);
    above_skip_txfm + left_skip_txfm
}

// Returns a context number for the given MB prediction signal
// The mode info data structure has a one element border above and to the
// left of the entries corresponding to real blocks.
// The prediction flags in these dummy entries are initialized to 0.
#[inline]
pub(crate) fn get_tx_size_context(xd: &MacroBlockD) -> i32 {
    let mbmi = xd.mbmi();
    let above_mbmi = xd.mi_at(xd.above_mbmi);
    let left_mbmi = xd.mi_at(xd.left_mbmi);
    let max_tx_size = MAX_TXSIZE_RECT_LOOKUP[mbmi.bsize as usize];
    let max_tx_wide = TX_SIZE_WIDE[max_tx_size as usize];
    let max_tx_high = TX_SIZE_HIGH[max_tx_size as usize];
    let has_above = xd.up_available;
    let has_left = xd.left_available;

    let mut above = (xd.above_txfm()[xd.above_txfm_context] as i32 >= max_tx_wide) as i32;
    let mut left = (xd.left_txfm_context_buffer[xd.left_txfm_context] as i32 >= max_tx_high) as i32;

    if has_above {
        if let Some(above_mbmi) = above_mbmi {
            if is_inter_block(above_mbmi) {
                above = (BLOCK_SIZE_WIDE[above_mbmi.bsize as usize] as i32 >= max_tx_wide) as i32;
            }
        }
    }
    if has_left {
        if let Some(left_mbmi) = left_mbmi {
            if is_inter_block(left_mbmi) {
                left = (BLOCK_SIZE_HIGH[left_mbmi.bsize as usize] as i32 >= max_tx_high) as i32;
            }
        }
    }
    if has_above && has_left {
        above + left
    } else if has_above {
        above
    } else if has_left {
        left
    } else {
        0
    }
}
