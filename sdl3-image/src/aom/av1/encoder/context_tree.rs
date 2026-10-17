// Rust translation of the parts of av1/encoder/context_tree.c and
// av1/encoder/context_tree.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The pick-mode context of a coding block. The non-RD path picks the mode
//! of a block and encodes it right away, so upstream's tree of contexts
//! (`PC_TREE`, its contexts reused from superblock to superblock and reset
//! with `av1_reset_pmc()`) holds nothing a later block reads: each block
//! gets a fresh context here. The coefficient buffers the contexts point
//! at are the ones shared by all of them (`td->shared_coeff_buf`), owned
//! by the macroblock's planes (see block.rs), and so are the end-of-block
//! and entropy context arrays, which a block writes before it reads them.

use super::block::{MbModeInfoExtFrame, RdStats};
use super::rd::av1_invalid_rd_stats;
use crate::aom::av1::common::alloccommon::try_vec;
use crate::aom::av1::common::blockd::MbModeInfo;
use crate::aom::av1::common::enums::*;

/// Translation of `PICK_MODE_CONTEXT` (see the module documentation).
#[derive(Debug, Clone, Default)]
pub(crate) struct PickModeContext {
    pub(crate) mic: MbModeInfo,
    pub(crate) mbmi_ext_best: MbModeInfoExtFrame,
    pub(crate) blk_skip: Vec<u8>,
    pub(crate) tx_type_map: Vec<u8>,

    pub(crate) num_4x4_blk: i32,
    // For current partition, only if all Y, U, and V transform blocks'
    // coefficients are quantized to 0, skippable is set to 1.
    pub(crate) skippable: i32,
    pub(crate) rd_stats: RdStats,

    pub(crate) rd_mode_is_ready: i32, // Flag to indicate whether rd pick mode decision has
                                      // been made.
}

/// Translation of `av1_alloc_pmc()` (the context is zeroed, as upstream's
/// calloc() leaves it).
pub(crate) fn av1_alloc_pmc(bsize: BlockSize) -> Result<PickModeContext, ()> {
    let mut ctx = PickModeContext {
        rd_mode_is_ready: 0,
        ..Default::default()
    };

    let num_pix = BLOCK_SIZE_WIDE[bsize as usize] as i32 * BLOCK_SIZE_HIGH[bsize as usize] as i32;
    let num_blk = num_pix / 16;

    ctx.blk_skip = try_vec(num_blk as usize, 0)?;
    ctx.tx_type_map = try_vec(num_blk as usize, 0)?;
    ctx.num_4x4_blk = num_blk;

    // (no palette color index maps: screen content tools are off)

    av1_invalid_rd_stats(&mut ctx.rd_stats);

    Ok(ctx)
}
