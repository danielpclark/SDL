// Rust translation of the parts of av1/common/cfl.c and av1/common/cfl.h
// from aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Chroma from luma: the saver never picks it (its chroma mode is always
//! DC), but whether it is allowed selects the chroma mode CDF. The luma
//! buffers of `CFL_CTX` are never stored to, and are left out.

use super::av1_common_int::SequenceHeader;
use super::blockd::{get_plane_block_size, is_inter_block, MacroBlockD};
use super::enums::*;

/// Translation of `CFL_CTX` without its sample buffers.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CflCtx {
    // Whether the DC_PRED cache is enabled. The DC_PRED cache is disabled when
    // decoding.
    pub(crate) dc_pred_is_cached: [i32; CFL_PRED_PLANES as usize],
    pub(crate) use_dc_pred_cache: i32,

    // Whether the reconstructed luma pixels need to be stored
    pub(crate) are_parameters_computed: i32,

    // Chroma subsampling
    pub(crate) subsampling_x: i32,
    pub(crate) subsampling_y: i32,

    // Whether the reconstructed luma pixels need to be stored
    pub(crate) store_y: i32,
}

// Can we use CfL for the current block?
#[inline]
pub(crate) fn is_cfl_allowed(xd: &MacroBlockD) -> i32 {
    let mbmi = xd.mbmi();
    let bsize = mbmi.bsize;
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    if xd.lossless[mbmi.segment_id as usize] != 0 {
        // In lossless, CfL is available when the partition size is equal to the
        // transform size.
        let ssx = xd.plane[1].subsampling_x;
        let ssy = xd.plane[1].subsampling_y;
        let plane_bsize = get_plane_block_size(bsize, ssx, ssy);
        return (plane_bsize == BLOCK_4X4) as i32;
    }
    // Spec: CfL is available to luma partitions lesser than or equal to 32x32
    (BLOCK_SIZE_WIDE[bsize as usize] <= 32 && BLOCK_SIZE_HIGH[bsize as usize] <= 32) as i32
}

// Do we need to save the luma pixels from the current block,
// for a possible future CfL prediction?
#[inline]
pub(crate) fn store_cfl_required(seq_params: &SequenceHeader, xd: &MacroBlockD) -> i32 {
    let mbmi = xd.mbmi();

    if seq_params.monochrome != 0 {
        return CFL_DISALLOWED;
    }

    if !xd.is_chroma_ref {
        // For non-chroma-reference blocks, we should always store the luma pixels,
        // in case the corresponding chroma-reference block uses CfL.
        // Note that this can only happen for block sizes which are <8 on
        // their shortest side, as otherwise they would be chroma reference
        // blocks.
        return CFL_ALLOWED;
    }

    // If this block has chroma information, we know whether we're
    // actually going to perform a CfL prediction
    (!is_inter_block(mbmi) && mbmi.uv_mode == UV_CFL_PRED) as i32
}

// (cfl.c)

pub(crate) fn cfl_init(cfl: &mut CflCtx, seq_params: &SequenceHeader) {
    cfl.subsampling_x = seq_params.subsampling_x;
    cfl.subsampling_y = seq_params.subsampling_y;
    cfl.are_parameters_computed = 0;
    cfl.store_y = 0;
    // The DC_PRED cache is disabled by default and is only enabled in
    // cfl_rd_pick_alpha
    cfl.use_dc_pred_cache = 0;
    cfl.dc_pred_is_cached[CFL_PRED_U as usize] = 0;
    cfl.dc_pred_is_cached[CFL_PRED_V as usize] = 0;
}
