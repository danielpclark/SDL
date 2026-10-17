// Rust translation of the parts of av1/common/alloccommon.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The mode info and above context allocations. Frees are `Drop` (or a
//! reset to the empty value); allocation failures return `Err(())`. CDEF
//! is off, so its buffers are never allocated (only the sizes are kept).

use super::av1_common_int::*;
use super::blockd::{MbModeInfo, MAX_MB_PLANE, MI_NONE};
use super::enums::*;
use crate::aom::aom_dsp::aom_dsp_common::{align_power_of_two, round_power_of_two};
use crate::aom::av1::encoder::encoder_utils::enc_set_mb_mi;

/// A zeroed vector, or `Err(())` if the allocation fails.
pub(crate) fn try_vec<T: Clone>(n: usize, v: T) -> Result<Vec<T>, ()> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| ())?;
    out.resize(n, v);
    Ok(out)
}

#[inline]
pub(crate) fn calc_mi_size(len: i32) -> i32 {
    // len is in mi units. Align to a multiple of SBs.
    align_power_of_two(len, MAX_MIB_SIZE_LOG2)
}

pub(crate) fn av1_get_mbs(width: i32, height: i32) -> i32 {
    let aligned_width = align_power_of_two(width, 3);
    let aligned_height = align_power_of_two(height, 3);
    let mi_cols = aligned_width >> MI_SIZE_LOG2;
    let mi_rows = aligned_height >> MI_SIZE_LOG2;

    let mb_cols = round_power_of_two(mi_cols, 2);
    let mb_rows = round_power_of_two(mi_rows, 2);
    mb_rows * mb_cols
}

pub(crate) fn av1_free_ref_frame_buffers(pool: &mut BufferPool) {
    for i in 0..FRAME_BUFFERS as usize {
        pool.frame_bufs[i].mvs = Vec::new();
        pool.frame_bufs[i].seg_map = Vec::new();
        pool.frame_bufs[i].buf = Default::default();
    }
}

/// Translation of `av1_alloc_cdef_buffers()` with CDEF disabled (no
/// buffers are allocated; the configuration is recorded).
pub(crate) fn av1_alloc_cdef_buffers(cm: &mut Av1Common, num_workers: i32) {
    let cdef_info = &mut cm.cdef_info;
    // Check for configuration change
    let num_mi_rows = (cm.mi_params.mi_rows + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    debug_assert!(cm.seq_params.enable_cdef == 0 || cm.tiles.large_scale != 0);

    // Store configuration to check change in configuration
    cdef_info.allocated_mi_rows = num_mi_rows;
    cdef_info.allocated_num_workers = num_workers;
}

pub(crate) fn av1_free_above_context_buffers(above_contexts: &mut CommonContexts) {
    *above_contexts = CommonContexts::default();
}

pub(crate) fn av1_free_context_buffers(cm: &mut Av1Common) {
    crate::aom::av1::encoder::encoder_utils::enc_free_mi(&mut cm.mi_params);

    av1_free_above_context_buffers(&mut cm.above_contexts);
}

pub(crate) fn av1_alloc_above_context_buffers(
    above_contexts: &mut CommonContexts,
    num_tile_rows: i32,
    num_mi_cols: i32,
    num_planes: i32,
) -> Result<(), ()> {
    let aligned_mi_cols = align_power_of_two(num_mi_cols, MAX_MIB_SIZE_LOG2);

    // Allocate above context buffers
    above_contexts.num_tile_rows = num_tile_rows;
    above_contexts.num_mi_cols = aligned_mi_cols;
    above_contexts.num_planes = num_planes;
    for plane_idx in 0..num_planes as usize {
        above_contexts.entropy[plane_idx] = Vec::new();
        above_contexts.entropy[plane_idx]
            .try_reserve_exact(num_tile_rows as usize)
            .map_err(|_| ())?;
    }

    above_contexts.partition = Vec::new();
    above_contexts
        .partition
        .try_reserve_exact(num_tile_rows as usize)
        .map_err(|_| ())?;

    above_contexts.txfm = Vec::new();
    above_contexts
        .txfm
        .try_reserve_exact(num_tile_rows as usize)
        .map_err(|_| ())?;

    for _tile_row in 0..num_tile_rows {
        for plane_idx in 0..num_planes as usize {
            let v = try_vec(aligned_mi_cols as usize, 0)?;
            above_contexts.entropy[plane_idx].push(v);
        }

        let v = try_vec(aligned_mi_cols as usize, 0)?;
        above_contexts.partition.push(v);

        let v = try_vec(aligned_mi_cols as usize, 0)?;
        above_contexts.txfm.push(v);
    }

    Ok(())
}

// Allocate the dynamically allocated arrays in 'mi_params' assuming
// 'mi_params->set_mb_mi()' was already called earlier to initialize the rest of
// the struct members.
fn alloc_mi(mi_params: &mut CommonModeInfoParams) -> Result<(), ()> {
    let aligned_mi_rows = calc_mi_size(mi_params.mi_rows);
    let mi_grid_size = mi_params.mi_stride * aligned_mi_rows;
    let alloc_size_1d = MI_SIZE_WIDE[mi_params.mi_alloc_bsize as usize] as i32;
    let alloc_mi_size = mi_params.mi_alloc_stride * (aligned_mi_rows / alloc_size_1d);

    if mi_params.mi_alloc_size < alloc_mi_size || mi_params.mi_grid_size < mi_grid_size {
        crate::aom::av1::encoder::encoder_utils::enc_free_mi(mi_params);

        mi_params.mi_alloc = try_vec(alloc_mi_size as usize, MbModeInfo::default())?;
        mi_params.mi_alloc_size = alloc_mi_size;

        mi_params.mi_grid_base = try_vec(mi_grid_size as usize, MI_NONE)?;
        mi_params.mi_grid_size = mi_grid_size;

        mi_params.tx_type_map = try_vec(mi_grid_size as usize, 0)?;
    }

    Ok(())
}

pub(crate) fn av1_alloc_context_buffers(
    cm: &mut Av1Common,
    width: i32,
    height: i32,
    min_partition_size: BlockSize,
) -> Result<(), ()> {
    let mi_params = &mut cm.mi_params;
    enc_set_mb_mi(mi_params, width, height, min_partition_size);
    if alloc_mi(mi_params).is_err() {
        // fail:
        // clear the mi_* values to force a realloc on resync
        enc_set_mb_mi(mi_params, 0, 0, BLOCK_4X4);
        av1_free_context_buffers(cm);
        return Err(());
    }
    Ok(())
}

/// The planes of the above contexts (`MAX_MB_PLANE`).
pub(crate) const ABOVE_CONTEXT_PLANES: usize = MAX_MB_PLANE;
