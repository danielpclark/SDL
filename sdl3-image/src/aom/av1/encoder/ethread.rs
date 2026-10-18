// Rust translation of the parts of av1/encoder/ethread.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Multi-threading of the encoder.
//!
//! The translation runs on one thread. What the worker count changes in
//! libaom's output is reproduced: the worker counts of each module are
//! computed as libaom computes them (they steer speed features and turn on
//! row multi-threading), and with row multi-threading the superblock rows
//! are coded one after the other with the semantics of libaom's row workers
//! (each row starts from the context its top-right superblock left, see
//! `encode_sb_row()`). libaom's row workers produce the same output however
//! the rows are distributed over them: each row's coding only reads what the
//! rows above it finished (the per-worker state a row starts from is either
//! reset or recomputed for the row). The multi-threaded loop filter and
//! CDEF filter the same pixels with the same inputs as the single-threaded
//! ones (the AV1 decoding process defines their output).

use super::encodeframe::{
    av1_accumulate_rtc_counters, av1_alloc_tile_data, av1_encode_sb_row, av1_init_rtc_counters,
    av1_init_tile_data,
};
use super::encodeframe_utils::{av1_alloc_mb_data, av1_dealloc_mb_data};
use super::encoder::*;
use super::picklpf::av1_pick_filter_level;
use super::speed_features::LPF_PICK_FROM_Q;
use crate::aom::aom_dsp::aom_dsp_common::{align_power_of_two, ceil_power_of_two};
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::cfl::cfl_init;
use crate::aom::av1::common::enums::{BLOCK_8X8, PLANE_TYPE_UV, PLANE_TYPE_Y};
use crate::aom::av1::common::tile_common::*;

/// `MULTI_THREADED_MODULES`
pub(crate) const MOD_FP: usize = 0; // First pass
pub(crate) const MOD_TF: usize = 1; // Temporal filtering
pub(crate) const MOD_TPL: usize = 2; // TPL
pub(crate) const MOD_GME: usize = 3; // Global motion estimation
pub(crate) const MOD_ENC: usize = 4; // Encode stage
pub(crate) const MOD_LPF: usize = 5; // Deblocking loop filter
pub(crate) const MOD_CDEF_SEARCH: usize = 6; // CDEF search
pub(crate) const MOD_CDEF: usize = 7; // CDEF frame
pub(crate) const MOD_LR: usize = 8; // Loop restoration filtering
pub(crate) const MOD_PACK_BS: usize = 9; // Pack bitstream
pub(crate) const MOD_FRAME_ENC: usize = 10; // Frame Parallel encode
pub(crate) const MOD_AI: usize = 11; // All intra
pub(crate) const NUM_MT_MODULES: usize = 12;

// Computes the number of workers for row multi-threading of encoding stage
/// Translation of `compute_num_enc_row_mt_workers()`.
fn compute_num_enc_row_mt_workers(cm: &Av1Common, max_threads: i32) -> i32 {
    let mut tile_info = TileInfo::default();
    let tile_cols = cm.tiles.cols;
    let tile_rows = cm.tiles.rows;
    let mut total_num_threads_row_mt = 0;
    for row in 0..tile_rows {
        for col in 0..tile_cols {
            av1_tile_init(&mut tile_info, cm, row, col);
            let num_sb_rows_in_tile = av1_get_sb_rows_in_tile(cm, &tile_info);
            let num_sb_cols_in_tile = av1_get_sb_cols_in_tile(cm, &tile_info);
            total_num_threads_row_mt += ((num_sb_cols_in_tile + 1) >> 1).min(num_sb_rows_in_tile);
        }
    }
    max_threads.min(total_num_threads_row_mt)
}

// Computes the number of workers for tile multi-threading of encoding stage
/// Translation of `compute_num_enc_tile_mt_workers()`.
fn compute_num_enc_tile_mt_workers(cm: &Av1Common, max_threads: i32) -> i32 {
    let tile_cols = cm.tiles.cols;
    let tile_rows = cm.tiles.rows;
    max_threads.min(tile_cols * tile_rows)
}

// Find max worker of all MT stages
/// Translation of `av1_get_max_num_workers()`.
pub(crate) fn av1_get_max_num_workers(cpi: &Av1Comp) -> i32 {
    let mut max_num_workers = 0;
    for i in MOD_FP..NUM_MT_MODULES {
        max_num_workers = cpi.ppi.p_mt_info.num_mod_workers[i].max(max_num_workers);
    }
    debug_assert!(max_num_workers >= 1);
    max_num_workers.min(cpi.oxcf.max_threads)
}

// Computes the number of workers for encoding stage (row/tile multi-threading)
/// Translation of `av1_compute_num_enc_workers()`.
pub(crate) fn av1_compute_num_enc_workers(cpi: &Av1Comp, max_workers: i32) -> i32 {
    if max_workers <= 1 {
        return 1;
    }
    if cpi.oxcf.row_mt {
        compute_num_enc_row_mt_workers(&cpi.common, max_workers)
    } else {
        compute_num_enc_tile_mt_workers(&cpi.common, max_workers)
    }
}

/// Translation of `compute_num_mod_workers()` (with the inline
/// `compute_num_*_workers()` helpers of the modules).
fn compute_num_mod_workers(cpi: &mut Av1Comp, mod_name: usize) -> i32 {
    let max_threads = cpi.oxcf.max_threads;
    match mod_name {
        MOD_FP => {
            if cpi.oxcf.pass >= AOM_RC_SECOND_PASS {
                0
            } else {
                av1_compute_num_enc_workers(cpi, max_threads)
            }
        }
        MOD_TF => {
            // For single-pass encode, using no. of workers as per tf block size was not
            // found to improve speed. Hence the thread assignment for single-pass encode
            // is kept based on compute_num_enc_workers().
            debug_assert!(cpi.oxcf.pass < AOM_RC_SECOND_PASS);
            av1_compute_num_enc_workers(cpi, max_threads)
        }
        MOD_TPL => av1_compute_num_enc_workers(cpi, max_threads),
        MOD_GME => 1,
        MOD_ENC => av1_compute_num_enc_workers(cpi, max_threads),
        MOD_LPF => av1_compute_num_enc_workers(cpi, max_threads),
        MOD_CDEF_SEARCH => av1_compute_num_enc_workers(cpi, max_threads),
        MOD_CDEF => av1_compute_num_enc_workers(cpi, max_threads),
        MOD_LR => av1_compute_num_enc_workers(cpi, max_threads),
        MOD_PACK_BS => {
            if max_threads <= 1 {
                1
            } else {
                compute_num_enc_tile_mt_workers(&cpi.common, max_threads)
            }
        }
        MOD_FRAME_ENC => cpi.ppi.p_mt_info.num_mod_workers[MOD_FRAME_ENC],
        MOD_AI => {
            if cpi.oxcf.pass == AOM_RC_ONE_PASS {
                // Computes num_workers for all intra multi-threading.
                if max_threads <= 1 {
                    1
                } else {
                    cpi.weber_bsize = BLOCK_8X8;
                    let bsize = cpi.weber_bsize;
                    let mb_step =
                        crate::aom::av1::common::enums::MI_SIZE_WIDE[bsize as usize] as i32;
                    let num_mb_rows = cpi.common.mi_params.mi_rows / mb_step;
                    num_mb_rows.min(max_threads)
                }
            } else {
                0
            }
        }
        _ => {
            debug_assert!(false);
            0
        }
    }
}

// Computes the number of workers for each MT modules in the encoder
/// Translation of `av1_compute_num_workers_for_mt()`.
pub(crate) fn av1_compute_num_workers_for_mt(cpi: &mut Av1Comp) {
    for i in MOD_FP..NUM_MT_MODULES {
        cpi.ppi.p_mt_info.num_mod_workers[i] = compute_num_mod_workers(cpi, i);
    }
}

/// Translation of `av1_create_workers()`: the workers are only counted
/// (see the module documentation).
pub(crate) fn av1_create_workers(cpi: &mut Av1Comp, num_workers: i32) {
    for _ in 0..num_workers {
        cpi.ppi.p_mt_info.num_workers += 1;
    }
}

/// Translation of `av1_init_frame_mt()`.
pub(crate) fn av1_init_frame_mt(cpi: &mut Av1Comp) {
    cpi.mt_info.num_workers = cpi.ppi.p_mt_info.num_workers;
    for i in MOD_FP..NUM_MT_MODULES {
        cpi.mt_info.num_mod_workers[i] = cpi
            .mt_info
            .num_workers
            .min(cpi.ppi.p_mt_info.num_mod_workers[i]);
    }
}

/// Translation of `av1_get_num_mod_workers_for_alloc()`.
pub(crate) fn av1_get_num_mod_workers_for_alloc(
    p_mt_info: &PrimaryMultiThreadInfo,
    mod_name: usize,
) -> i32 {
    let mut num_mod_workers = p_mt_info.num_mod_workers[mod_name];
    if p_mt_info.num_mod_workers[MOD_FRAME_ENC] > 1 {
        // TODO(anyone): Change num_mod_workers to num_mod_workers[MOD_FRAME_ENC].
        // As frame parallel jobs will only perform multi-threading for the encode
        // stage, we can limit the allocations according to num_enc_workers per
        // frame parallel encode(a.k.a num_mod_workers[MOD_FRAME_ENC]).
        num_mod_workers = p_mt_info.num_workers;
    }
    num_mod_workers
}

/// Translation of `compute_max_sb_rows_cols()`.
fn compute_max_sb_rows_cols(cm: &Av1Common, max_sb_rows: &mut i32, max_sb_cols: &mut i32) {
    let tile_rows = cm.tiles.rows;
    let mi_cols_aligned_to_sb =
        align_power_of_two(cm.mi_params.mi_cols, cm.seq_params.mib_size_log2);
    let sb_cols_in_frame = mi_cols_aligned_to_sb >> cm.seq_params.mib_size_log2;
    let tile_cols = cm.tiles.cols;

    for row in 0..tile_rows {
        let mi_row_start = cm.tiles.row_start_sb[row as usize] << cm.seq_params.mib_size_log2;
        let mi_row_end = (cm.tiles.row_start_sb[row as usize + 1] << cm.seq_params.mib_size_log2)
            .min(cm.mi_params.mi_rows);
        let sb_rows = ceil_power_of_two(mi_row_end - mi_row_start, cm.seq_params.mib_size_log2);
        *max_sb_rows = (*max_sb_rows).max(sb_rows);
    }

    for col in 0..tile_cols {
        let mi_col_start = cm.tiles.col_start_sb[col as usize] << cm.seq_params.mib_size_log2;
        let mi_col_end = (cm.tiles.col_start_sb[col as usize + 1] << cm.seq_params.mib_size_log2)
            .min(cm.mi_params.mi_cols);
        let sb_cols = ceil_power_of_two(mi_col_end - mi_col_start, cm.seq_params.mib_size_log2);
        *max_sb_cols = (*max_sb_cols).max(sb_cols);
    }
    let _ = sb_cols_in_frame;
}

/// Translation of `lpf_pipeline_mt_init()`: decides whether the loop filter
/// runs in the encoding workers, and picks its levels then. (The pipelined
/// filtering itself runs after the rows are coded: see
/// `av1_encode_tiles_row_mt()`.)
fn lpf_pipeline_mt_init(cpi: &mut Av1Comp) {
    let cm = &cpi.common;
    let use_loopfilter = is_loopfilter_used(cm);
    let use_superres = av1_superres_scaled(cm);
    let use_cdef = is_cdef_used(cm);
    let use_restoration = is_restoration_used(cm);

    let skip_apply_postproc_filters = derive_skip_apply_postproc_filters(
        cpi,
        use_loopfilter,
        use_cdef,
        use_superres,
        use_restoration,
    );
    cpi.mt_info.pipeline_lpf_mt_with_enc = cpi.oxcf.mode == REALTIME
        && cpi.oxcf.speed >= 5
        && cpi.sf.lpf_sf.lpf_pick == LPF_PICK_FROM_Q
        && cpi.oxcf.algo_cfg.loopfilter_control != LOOPFILTER_SELECTIVELY
        && !cpi.ppi.rtc_ref.non_reference_frame
        && !cpi.common.features.allow_intrabc
        && (skip_apply_postproc_filters & SKIP_APPLY_LOOPFILTER) == 0;

    cpi.mt_info.pipelined_lpf_pending = false;
    if !cpi.mt_info.pipeline_lpf_mt_with_enc {
        return;
    }

    set_postproc_filter_default_params(&mut cpi.common);

    if !use_loopfilter {
        return;
    }

    let method = cpi.sf.lpf_sf.lpf_pick;
    debug_assert!(method == LPF_PICK_FROM_Q);
    debug_assert!(cpi.oxcf.algo_cfg.loopfilter_control != LOOPFILTER_SELECTIVELY);

    av1_pick_filter_level(cpi, method);

    let lf = &cpi.common.lf;
    // (check_planes_to_loop_filter(), av1_loop_filter_frame_init() and
    // loop_filter_frame_mt_init(): av1_loop_filter_frame() does them)
    if lf.filter_level[PLANE_TYPE_Y as usize] != 0 || lf.filter_level[PLANE_TYPE_UV as usize] != 0 {
        cpi.mt_info.pipelined_lpf_pending = true;
    }
}

/// Translation of `av1_encode_tiles_row_mt()`, `prepare_enc_workers()`,
/// `enc_row_mt_worker_hook()` and `accumulate_counters_enc_workers()`, on
/// one thread: the superblock rows are coded in order, each with the state
/// libaom's row workers give it (see the module documentation). `td` is
/// `&cpi->td`, the first worker's data, moved out of `cpi` as for
/// `encode_tiles()`.
pub(crate) fn av1_encode_tiles_row_mt(cpi: &mut Av1Comp, td: &mut ThreadData) -> Result<(), ()> {
    let tile_cols = cpi.common.tiles.cols;
    let tile_rows = cpi.common.tiles.rows;
    let mut max_sb_rows_in_tile = 0;
    let mut max_sb_cols_in_tile = 0;
    let mut num_workers = cpi.mt_info.num_mod_workers[MOD_ENC];

    compute_max_sb_rows_cols(
        &cpi.common,
        &mut max_sb_rows_in_tile,
        &mut max_sb_cols_in_tile,
    );
    let alloc_tile_data = cpi.allocated_tiles < tile_cols * tile_rows;

    if alloc_tile_data {
        av1_alloc_tile_data(cpi);
    }

    // (row_mt_mem_alloc(): the row synchronization data is not needed; the
    // row contexts are allocated when cdf_update_mode is set. Their values
    // are always written before they are read.)
    let num_row_ctx = 1.max(max_sb_cols_in_tile - 1) as usize;
    if cpi.oxcf.algo_cfg.cdf_update_mode != 0 {
        for tile in cpi.tile_data.iter_mut() {
            if tile.row_ctx.len() != num_row_ctx {
                tile.row_ctx = vec![(*cpi.common.fc).clone(); num_row_ctx];
            }
        }
    }

    num_workers = num_workers.min(cpi.mt_info.num_workers);
    let _ = num_workers;
    lpf_pipeline_mt_init(cpi);

    av1_init_tile_data(cpi);

    for tile_row in 0..tile_rows {
        for tile_col in 0..tile_cols {
            let tile_index = (tile_row * tile_cols + tile_col) as usize;
            let tile_info = cpi.tile_data[tile_index].tile_info;
            // (av1_inter_mode_data_init(): the inter mode RD models only serve
            // the RD pick mode)
            av1_zero_above_context(
                &cpi.common,
                &mut td.mb.e_mbd.above_contexts,
                tile_info.mi_col_start,
                tile_info.mi_col_end,
                tile_row,
            );
        }
    }

    // (prepare_enc_workers())
    td.intrabc_used = 0;
    td.deltaq_used = 0;
    td.abs_sum_level = 0;
    td.rd_counts.seg_tmp_pred_cost = [0; 2];
    av1_alloc_mb_data(cpi, &mut td.mb)?;
    av1_init_rtc_counters(&mut td.mb);

    // (enc_row_mt_worker_hook(): td->rt_pc_root, the partition tree, is
    // implicit, see context_tree.rs)
    let mut res = Ok(());
    'tiles: for tile_index in 0..(tile_cols * tile_rows) as usize {
        let tile_info = cpi.tile_data[tile_index].tile_info;
        let allow_update_cdf = cpi.tile_data[tile_index].allow_update_cdf;
        td.mb.row_ctx = std::mem::take(&mut cpi.tile_data[tile_index].row_ctx);
        let mut current_mi_row = tile_info.mi_row_start;
        while current_mi_row < tile_info.mi_row_end {
            // (td->mb.e_mbd.tile_ctx = td->tctx, the worker's context)
            td.abs_sum_level = 0;

            if allow_update_cdf {
                if current_mi_row == tile_info.mi_row_start {
                    *td.mb.e_mbd.tile_ctx = (*cpi.tile_data[tile_index].tctx).clone();
                }
            } else {
                *td.mb.e_mbd.tile_ctx = (*cpi.tile_data[tile_index].tctx).clone();
            }

            av1_init_above_context(tile_info.tile_row, &mut td.mb.e_mbd);

            cfl_init(&mut td.mb.e_mbd.cfl, &cpi.common.seq_params);

            res = av1_encode_sb_row(cpi, td, &cpi.tile_data[tile_index], current_mi_row);
            if res.is_err() {
                cpi.tile_data[tile_index].row_ctx = std::mem::take(&mut td.mb.row_ctx);
                break 'tiles;
            }
            cpi.tile_data[tile_index].abs_sum_level += td.abs_sum_level;
            current_mi_row += cpi.common.seq_params.mib_size;
        }
        cpi.tile_data[tile_index].row_ctx = std::mem::take(&mut td.mb.row_ctx);
    }
    res?;

    // (update_delta_lf_for_row_mt(): delta loop filter levels are never
    // present)
    debug_assert!(cpi.common.delta_q_info.delta_lf_present_flag == 0);

    // (accumulate_counters_enc_workers())
    cpi.intrabc_used |= td.intrabc_used;
    cpi.deltaq_used |= td.deltaq_used;
    if !frame_is_intra_only(&cpi.common) {
        av1_accumulate_rtc_counters(cpi, &td.mb);
    }
    av1_dealloc_mb_data(&mut td.mb);
    Ok(())
}
