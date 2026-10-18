// Rust translation of the parts of av1/encoder/encodeframe_utils.c and
// av1/encoder/encodeframe_utils.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches,
// as built without CONFIG_ENTROPY_STATS and CONFIG_INTERNAL_STATS.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Block-level helpers of the frame encoder: SSIM-tuned rate-distortion
//! multipliers, the restoring of a picked block's state, the CDF updates
//! of intra modes, and the cost update frequencies.

use super::block::*;
use super::context_tree::PickModeContext;
use super::encoder::*;
use super::encoder::{AOM_TUNE_SSIM, NO_AQ};
use super::rd::{
    av1_fill_coeff_costs, av1_fill_mode_rates, av1_fill_mv_costs, av1_set_error_per_bit,
};
use super::speed_features::*;
use crate::aom::aom_dsp::prob::{update_cdf, CDF_PROB_TOP};
use crate::aom::av1::common::alloccommon::try_vec;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cfl::is_cfl_allowed;
use crate::aom::av1::common::entropy::BR_CDF_SIZE;
use crate::aom::av1::common::entropymode::FrameContext;
use crate::aom::av1::common::entropymv::{NmvContext, CLASS0_SIZE, MV_CLASSES, MV_FP_SIZE};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::SWITCHABLE_FILTERS;
use crate::aom::av1::common::mvref_common::{av1_copy_frame_mvs, av1_ref_frame_type};
use crate::aom::av1::common::reconintra::{
    av1_allow_intrabc, av1_filter_intra_allowed, av1_is_directional_mode,
};
use crate::aom::av1::common::seg_common::MAX_SEGMENTS;
use crate::aom::av1::common::tile_common::TileInfo;

pub(crate) const SUB_PARTITIONS_SPLIT: i32 = 4;

// (encodeframe_utils.c)

pub(crate) fn av1_set_ssim_rdmult(
    cpi: &Av1Comp,
    errorperbit: &mut i32,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    rdmult: &mut i32,
) {
    let cm = &cpi.common;

    let bsize_base = BLOCK_16X16;
    let num_mi_w = MI_SIZE_WIDE[bsize_base as usize] as i32;
    let num_mi_h = MI_SIZE_HIGH[bsize_base as usize] as i32;
    let num_cols = (cm.mi_params.mi_cols + num_mi_w - 1) / num_mi_w;
    let num_rows = (cm.mi_params.mi_rows + num_mi_h - 1) / num_mi_h;
    let num_bcols = (MI_SIZE_WIDE[bsize as usize] as i32 + num_mi_w - 1) / num_mi_w;
    let num_brows = (MI_SIZE_HIGH[bsize as usize] as i32 + num_mi_h - 1) / num_mi_h;
    let mut num_of_mi = 0.0f64;
    let mut geom_mean_of_scale = 0.0f64;

    debug_assert!(cpi.oxcf.tune_cfg.tuning == AOM_TUNE_SSIM);

    let mut row = mi_row / num_mi_w;
    while row < num_rows && row < mi_row / num_mi_w + num_brows {
        let mut col = mi_col / num_mi_h;
        while col < num_cols && col < mi_col / num_mi_h + num_bcols {
            let index = (row * num_cols + col) as usize;
            debug_assert!(cpi.ssim_rdmult_scaling_factors[index] != 0.0);
            geom_mean_of_scale += cpi.ssim_rdmult_scaling_factors[index].ln();
            num_of_mi += 1.0;
            col += 1;
        }
        row += 1;
    }
    geom_mean_of_scale = (geom_mean_of_scale / num_of_mi).exp();

    *rdmult = (*rdmult as f64 * geom_mean_of_scale + 0.5) as i32;
    *rdmult = (*rdmult).max(0);
    av1_set_error_per_bit(errorperbit, *rdmult);
}

// This function will copy the best reference mode information from
// MB_MODE_INFO_EXT_FRAME to MB_MODE_INFO_EXT.
#[inline]
fn copy_mbmi_ext_frame_to_mbmi_ext(
    mbmi_ext: &mut MbModeInfoExt,
    mbmi_ext_best: &MbModeInfoExtFrame,
    ref_frame_type: u8,
) {
    mbmi_ext.ref_mv_stack[ref_frame_type as usize] = mbmi_ext_best.ref_mv_stack;
    mbmi_ext.weight[ref_frame_type as usize] = mbmi_ext_best.weight;
    mbmi_ext.mode_context[ref_frame_type as usize] = mbmi_ext_best.mode_context;
    mbmi_ext.ref_mv_count[ref_frame_type as usize] = mbmi_ext_best.ref_mv_count;
    mbmi_ext.global_mvs = mbmi_ext_best.global_mvs;
}

pub(crate) fn av1_update_state(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    ctx: &PickModeContext,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    dry_run: i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let mi_params = &cm.mi_params;
    let x = &mut td.mb;
    let mi = &ctx.mic;
    let seg = &cm.seg;
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let bw = MI_SIZE_WIDE[mi.bsize as usize] as i32;
    let bh = MI_SIZE_HIGH[mi.bsize as usize] as i32;
    let mis = mi_params.mi_stride;
    let mi_width = MI_SIZE_WIDE[bsize as usize] as i32;
    let mi_height = MI_SIZE_HIGH[bsize as usize] as i32;

    debug_assert!(mi.bsize == bsize);

    *x.e_mbd.mbmi_mut() = *mi;
    copy_mbmi_ext_frame_to_mbmi_ext(
        &mut x.mbmi_ext,
        &ctx.mbmi_ext_best,
        av1_ref_frame_type(&ctx.mic.ref_frame) as u8,
    );

    let n = ctx.num_4x4_blk as usize;
    x.txfm_search_info.blk_skip[..n].copy_from_slice(&ctx.blk_skip[..n]);

    x.txfm_search_info.skip_txfm = ctx.rd_stats.skip_txfm;

    let xd = &mut x.e_mbd;
    // (xd->tx_type_map = ctx->tx_type_map: the context's map is copied into
    // the block's map)
    xd.tx_type_map_blk[..n].copy_from_slice(&ctx.tx_type_map[..n]);
    xd.tx_type_map = TxTypeMapRef::Block;
    xd.tx_type_map_stride = MI_SIZE_WIDE[bsize as usize] as i32;
    // If not dry_run, copy the transform type data into the frame level buffer.
    // Encoder will fetch tx types when writing bitstream.
    if dry_run == 0 {
        let grid_idx = get_mi_grid_idx(mi_params, mi_row, mi_col) as usize;
        let mi_stride = mi_params.mi_stride as usize;
        let blk_stride = xd.tx_type_map_stride as usize;
        for blk_row in 0..bh as usize {
            let dst = grid_idx + blk_row * mi_stride;
            let src = blk_row * blk_stride;
            let (frame, blk) = (&mut xd.tx_type_map_frame, &xd.tx_type_map_blk);
            frame[dst..dst + bw as usize].copy_from_slice(&blk[src..src + bw as usize]);
        }
        xd.tx_type_map = TxTypeMapRef::Frame(grid_idx);
        xd.tx_type_map_stride = mi_stride as i32;
    }

    // If segmentation in use
    // (segmentation and adaptive quantization are off for the saver)
    debug_assert!(seg.enabled == 0);
    debug_assert!(cpi.oxcf.q_cfg.aq_mode == NO_AQ);

    // (the coefficient buffers of the context are the planes': see
    // context_tree.rs)

    // Restore the coding context of the MB to that that was in place
    // when the mode was picked for it
    let cols = ((xd.mb_to_right_edge >> (3 + MI_SIZE_LOG2)) + mi_width).min(mi_width);
    let rows = ((xd.mb_to_bottom_edge >> (3 + MI_SIZE_LOG2)) + mi_height).min(mi_height);
    let mi_addr = xd.mi_grid_base[xd.mi];
    for y in 0..rows {
        for x_idx in 0..cols {
            let i = (xd.mi as i32 + x_idx + y * mis) as usize;
            xd.mi_grid_base[i] = mi_addr;
        }
    }

    if dry_run != 0 {
        return Ok(());
    }

    if !frame_is_intra_only(cm) {
        // (update_filter_type_count())
        return Err(());
    }

    let x_mis = bw.min(mi_params.mi_cols - mi_col);
    let y_mis = bh.min(mi_params.mi_rows - mi_row);
    if cm.seq_params.order_hint_info.enable_ref_frame_mvs != 0 {
        av1_copy_frame_mvs(cm, &mut xd.cur_frame_mvs, mi, mi_row, mi_col, x_mis, y_mis);
    }
    Ok(())
}

pub(crate) fn av1_sum_intra_stats(cm: &Av1Common, xd: &mut MacroBlockD, intraonly: bool) {
    let mbmi = *xd.mbmi();
    let above_mi = xd.above_mbmi.map(|i| xd.mi_alloc[i as usize]);
    let left_mi = xd.left_mbmi.map(|i| xd.mi_alloc[i as usize]);
    let cfl_allowed = is_cfl_allowed(xd);
    let is_chroma_ref = xd.is_chroma_ref;
    let fc = &mut *xd.tile_ctx;
    let y_mode = mbmi.mode;
    let bsize = mbmi.bsize;

    if intraonly {
        update_cdf(
            get_y_mode_cdf(fc, above_mi.as_ref(), left_mi.as_ref()),
            y_mode as i8,
            INTRA_MODES,
        );
    } else {
        update_cdf(
            &mut fc.y_mode_cdf[SIZE_GROUP_LOOKUP[bsize as usize] as usize],
            y_mode as i8,
            INTRA_MODES,
        );
    }

    if av1_filter_intra_allowed(cm, &mbmi) {
        let use_filter_intra_mode = mbmi.filter_intra_mode_info.use_filter_intra;
        update_cdf(
            &mut fc.filter_intra_cdfs[mbmi.bsize as usize],
            use_filter_intra_mode as i8,
            2,
        );
        if use_filter_intra_mode != 0 {
            update_cdf(
                &mut fc.filter_intra_mode_cdf,
                mbmi.filter_intra_mode_info.filter_intra_mode as i8,
                FILTER_INTRA_MODES,
            );
        }
    }
    if av1_is_directional_mode(mbmi.mode) && av1_use_angle_delta(bsize) {
        update_cdf(
            &mut fc.angle_delta_cdf[(mbmi.mode - V_PRED) as usize],
            (mbmi.angle_delta[PLANE_TYPE_Y as usize] as i32 + MAX_ANGLE_DELTA) as i8,
            2 * MAX_ANGLE_DELTA + 1,
        );
    }

    if !is_chroma_ref {
        return;
    }

    let uv_mode = mbmi.uv_mode;
    update_cdf(
        &mut fc.uv_mode_cdf[cfl_allowed as usize][y_mode as usize],
        uv_mode as i8,
        UV_INTRA_MODES - (cfl_allowed == 0) as i32,
    );
    if uv_mode == UV_CFL_PRED {
        let joint_sign = mbmi.cfl_alpha_signs as i32;
        let idx = mbmi.cfl_alpha_idx as i32;

        update_cdf(&mut fc.cfl_sign_cdf, joint_sign as i8, CFL_JOINT_SIGNS);
        if cfl_sign_u(joint_sign) != CFL_SIGN_ZERO {
            let cdf_u = &mut fc.cfl_alpha_cdf[cfl_context_u(joint_sign) as usize];
            update_cdf(cdf_u, cfl_idx_u(idx) as i8, CFL_ALPHABET_SIZE);
        }
        if cfl_sign_v(joint_sign) != CFL_SIGN_ZERO {
            let cdf_v = &mut fc.cfl_alpha_cdf[cfl_context_v(joint_sign) as usize];
            update_cdf(cdf_v, cfl_idx_v(idx) as i8, CFL_ALPHABET_SIZE);
        }
    }
    if av1_is_directional_mode(get_uv_mode(uv_mode)) && av1_use_angle_delta(bsize) {
        update_cdf(
            &mut fc.angle_delta_cdf[(uv_mode - UV_V_PRED) as usize],
            (mbmi.angle_delta[PLANE_TYPE_UV as usize] as i32 + MAX_ANGLE_DELTA) as i8,
            2 * MAX_ANGLE_DELTA + 1,
        );
    }
    // (update_palette_cdf(): screen content tools are off, so palettes are
    // never allowed)
    debug_assert!(!av1_allow_palette(
        cm.features.allow_screen_content_tools,
        bsize
    ));
}

pub(crate) fn av1_is_leaf_split_partition(
    cm: &Av1Common,
    xd: &MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) -> bool {
    let bs = MI_SIZE_WIDE[bsize as usize] as i32;
    let hbs = bs / 2;
    debug_assert!(bsize >= BLOCK_8X8);
    let subsize = get_partition_subsize(bsize, PARTITION_SPLIT);

    for i in 0..4 {
        let x_idx = (i & 1) * hbs;
        let y_idx = (i >> 1) * hbs;
        if (mi_row + y_idx >= cm.mi_params.mi_rows) || (mi_col + x_idx >= cm.mi_params.mi_cols) {
            return false;
        }
        if get_partition(&cm.mi_params, xd, mi_row + y_idx, mi_col + x_idx, subsize)
            != PARTITION_NONE
            && subsize != BLOCK_8X8
        {
            return false;
        }
    }
    true
}

// analysis_type 0: Use mc_dep_cost and intra_cost
// analysis_type 1: Use count of best inter predictor chosen
// analysis_type 2: Use cost reduction from intra to inter for best inter
//                  predictor chosen
// (The saver's key frame has no TPL statistics, so the function always
// returns the base q index and leaves *delta_dist as it is; the
// computation from the statistics is left out.)
pub(crate) fn av1_get_q_for_deltaq_objective(
    cpi: &Av1Comp,
    delta_dist: &mut i64,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
) -> i32 {
    let cm = &cpi.common;
    let tpl_idx = cpi.gf_frame_index as i32;
    let base_qindex = cm.quant_params.base_qindex;
    let _ = (delta_dist, bsize, mi_row, mi_col);

    if tpl_idx >= MAX_TPL_FRAME_IDX {
        return base_qindex;
    }

    // (tpl_frame->is_valid is never set: the temporal dependency model only
    // runs for the frames of a group with inter frames)
    debug_assert!(!cpi.ppi.tpl_data.tpl_frame_is_valid[tpl_idx as usize]);
    base_qindex
}

/// The largest number of frames the temporal dependency model analyzes
/// (`MAX_TPL_FRAME_IDX`, tpl_model.h).
pub(crate) const MAX_TPL_FRAME_IDX: i32 = 2 * MAX_LENGTH_TPL_FRAME_STATS;
/// `MAX_LENGTH_TPL_FRAME_STATS` (tpl_model.h)
pub(crate) const MAX_LENGTH_TPL_FRAME_STATS: i32 = MAX_TOTAL_BUFFERS + REF_FRAMES + 1;
/// `MAX_TOTAL_BUFFERS` (tpl_model.h)
pub(crate) const MAX_TOTAL_BUFFERS: i32 = MAX_GF_INTERVAL + 1 + MAX_LAG_BUFFERS;
/// `MAX_GF_INTERVAL` (firstpass.h)
pub(crate) const MAX_GF_INTERVAL: i32 = 32;
/// `MAX_LAG_BUFFERS` (lookahead.h)
pub(crate) const MAX_LAG_BUFFERS: i32 = 48;

/*\brief Function to skip the cost update of symbols.
 *
 * This function contains the common code used to skip the cost update of coeff,
 * mode, mv and dv symbols.
 */
fn skip_cost_update(
    seq_params: &SequenceHeader,
    tile_info: &TileInfo,
    mi_row: i32,
    mi_col: i32,
    upd_level: i32,
) -> bool {
    if upd_level == INTERNAL_COST_UPD_SB {
        return false;
    }
    if upd_level == INTERNAL_COST_UPD_OFF {
        return true;
    }

    // upd_level is at most as frequent as each sb_row in a tile.
    if mi_col != tile_info.mi_col_start {
        return true;
    }

    if upd_level == INTERNAL_COST_UPD_SBROW_SET {
        let mib_size_log2 = seq_params.mib_size_log2;
        let sb_row = (mi_row - tile_info.mi_row_start) >> mib_size_log2;
        let sb_size = seq_params.mib_size * MI_SIZE;
        let tile_height = (tile_info.mi_row_end - tile_info.mi_row_start) * MI_SIZE;
        // When upd_level = INTERNAL_COST_UPD_SBROW_SET, the cost update happens
        // once for 2, 4 sb rows for sb size 128, sb size 64 respectively. However,
        // as the update will not be equally spaced in smaller resolutions making
        // it equally spaced by calculating (mv_num_rows_cost_update) the number of
        // rows after which the cost update should happen.
        let sb_size_update_freq_map: [i32; 2] = [2, 4];
        let update_freq_sb_rows = sb_size_update_freq_map[(sb_size != MAX_SB_SIZE) as usize];
        let update_freq_num_rows = sb_size * update_freq_sb_rows;
        // Round-up the division result to next integer.
        let num_updates_per_tile = (tile_height + update_freq_num_rows - 1) / update_freq_num_rows;
        let num_rows_update_per_tile = num_updates_per_tile * sb_size;
        // Round-up the division result to next integer.
        let num_sb_rows_per_update =
            (tile_height + num_rows_update_per_tile - 1) / num_rows_update_per_tile;
        if sb_row % num_sb_rows_per_update != 0 {
            return true;
        }
    }
    false
}

// Checks for skip status of mv cost update.
fn skip_mv_cost_update(cpi: &Av1Comp, tile_info: &TileInfo, mi_row: i32, mi_col: i32) -> bool {
    let cm = &cpi.common;
    // For intra frames, mv cdfs are not updated during the encode. Hence, the mv
    // cost calculation is skipped in this case.
    if frame_is_intra_only(cm) {
        return true;
    }

    skip_cost_update(
        &cm.seq_params,
        tile_info,
        mi_row,
        mi_col,
        cpi.sf.inter_sf.mv_cost_upd_level,
    )
}

// Checks for skip status of dv cost update.
fn skip_dv_cost_update(cpi: &Av1Comp, tile_info: &TileInfo, mi_row: i32, mi_col: i32) -> bool {
    let cm = &cpi.common;
    // Intrabc is only applicable to intra frames. So skip if intrabc is not
    // allowed.
    if !av1_allow_intrabc(cm) || is_stat_generation_stage(cpi) {
        return true;
    }

    skip_cost_update(
        &cm.seq_params,
        tile_info,
        mi_row,
        mi_col,
        cpi.sf.intra_sf.dv_cost_upd_level,
    )
}

// Update the rate costs of some symbols according to the frequency directed
// by speed features
pub(crate) fn av1_set_cost_upd_freq(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    tile_info: &TileInfo,
    mi_row: i32,
    mi_col: i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);
    let x = &mut td.mb;

    if cm.features.disable_cdf_update {
        return Ok(());
    }

    match cpi.sf.inter_sf.coeff_cost_upd_level {
        INTERNAL_COST_UPD_OFF | INTERNAL_COST_UPD_TILE => {} // Tile level
        // SB row set level in tile, SB row level in tile, SB level
        INTERNAL_COST_UPD_SBROW_SET | INTERNAL_COST_UPD_SBROW | INTERNAL_COST_UPD_SB => {
            if !skip_cost_update(
                &cm.seq_params,
                tile_info,
                mi_row,
                mi_col,
                cpi.sf.inter_sf.coeff_cost_upd_level,
            ) {
                av1_fill_coeff_costs(&mut x.coeff_costs, &x.e_mbd.tile_ctx, num_planes);
            }
        }
        _ => debug_assert!(false),
    }

    match cpi.sf.inter_sf.mode_cost_upd_level {
        INTERNAL_COST_UPD_OFF | INTERNAL_COST_UPD_TILE => {} // Tile level
        // SB row set level in tile, SB row level in tile, SB level
        INTERNAL_COST_UPD_SBROW_SET | INTERNAL_COST_UPD_SBROW | INTERNAL_COST_UPD_SB => {
            if !skip_cost_update(
                &cm.seq_params,
                tile_info,
                mi_row,
                mi_col,
                cpi.sf.inter_sf.mode_cost_upd_level,
            ) {
                av1_fill_mode_rates(cm, &mut x.mode_costs, &x.e_mbd.tile_ctx);
            }
        }
        _ => debug_assert!(false),
    }

    match cpi.sf.inter_sf.mv_cost_upd_level {
        INTERNAL_COST_UPD_OFF | INTERNAL_COST_UPD_TILE => {} // Tile level
        // SB row set level in tile, SB row level in tile, SB level
        INTERNAL_COST_UPD_SBROW_SET | INTERNAL_COST_UPD_SBROW | INTERNAL_COST_UPD_SB => {
            // Checks for skip status of mv cost update.
            if !skip_mv_cost_update(cpi, tile_info, mi_row, mi_col) {
                av1_fill_mv_costs(
                    &x.e_mbd.tile_ctx.nmvc,
                    cm.features.cur_frame_force_integer_mv,
                    cm.features.allow_high_precision_mv,
                    x.mv_costs.as_deref_mut(),
                );
            }
        }
        _ => debug_assert!(false),
    }

    match cpi.sf.intra_sf.dv_cost_upd_level {
        INTERNAL_COST_UPD_OFF | INTERNAL_COST_UPD_TILE => {} // Tile level
        // SB row set level in tile, SB row level in tile, SB level
        INTERNAL_COST_UPD_SBROW_SET | INTERNAL_COST_UPD_SBROW | INTERNAL_COST_UPD_SB => {
            // Checks for skip status of dv cost update.
            if !skip_dv_cost_update(cpi, tile_info, mi_row, mi_col) {
                // (av1_fill_dv_costs(): intra block copy needs screen
                // content tools, which are off)
                return Err(());
            }
        }
        _ => debug_assert!(false),
    }
    Ok(())
}

/// The CDF arrays of a frame context, for `AVG_CDF_STRIDE()`: visits the
/// innermost arrays (one CDF each, of `CDF_SIZE()` entries) of two arrays
/// of the same shape in step.
trait CdfArrays {
    fn each_cdf(&mut self, other: &Self, f: &mut dyn FnMut(&mut [u16], &[u16]));
}

impl<const N: usize> CdfArrays for [u16; N] {
    fn each_cdf(&mut self, other: &Self, f: &mut dyn FnMut(&mut [u16], &[u16])) {
        f(self, other);
    }
}

impl<T: CdfArrays, const N: usize> CdfArrays for [T; N] {
    fn each_cdf(&mut self, other: &Self, f: &mut dyn FnMut(&mut [u16], &[u16])) {
        for (a, b) in self.iter_mut().zip(other.iter()) {
            a.each_cdf(b, f);
        }
    }
}

fn avg_cdf_symbol(
    cdf_ptr_left: &mut [u16],
    cdf_ptr_tr: &[u16],
    nsymbs: i32,
    wt_left: i32,
    wt_tr: i32,
) {
    for j in 0..=nsymbs as usize {
        cdf_ptr_left[j] = ((cdf_ptr_left[j] as i32 * wt_left
            + cdf_ptr_tr[j] as i32 * wt_tr
            + ((wt_left + wt_tr) / 2))
            / (wt_left + wt_tr)) as u16;
        debug_assert!((cdf_ptr_left[j] as u32) < CDF_PROB_TOP);
    }
}

/// `AVG_CDF_STRIDE()` (and `AVERAGE_CDF()`: the stride, the size of the
/// innermost arrays, is always `CDF_SIZE()` of the arrays' type).
fn avg_cdf_stride<T: CdfArrays>(
    cname_left: &mut T,
    cname_tr: &T,
    nsymbs: i32,
    wt_left: i32,
    wt_tr: i32,
) {
    cname_left.each_cdf(cname_tr, &mut |l, t| {
        avg_cdf_symbol(l, t, nsymbs, wt_left, wt_tr)
    });
}

fn avg_nmv(nmv_left: &mut NmvContext, nmv_tr: &NmvContext, wt_left: i32, wt_tr: i32) {
    avg_cdf_stride(
        &mut nmv_left.joints_cdf,
        &nmv_tr.joints_cdf,
        4,
        wt_left,
        wt_tr,
    );
    for i in 0..2 {
        let (l, t) = (&mut nmv_left.comps[i], &nmv_tr.comps[i]);
        avg_cdf_stride(
            &mut l.classes_cdf,
            &t.classes_cdf,
            MV_CLASSES,
            wt_left,
            wt_tr,
        );
        avg_cdf_stride(
            &mut l.class0_fp_cdf,
            &t.class0_fp_cdf,
            MV_FP_SIZE,
            wt_left,
            wt_tr,
        );
        avg_cdf_stride(&mut l.fp_cdf, &t.fp_cdf, MV_FP_SIZE, wt_left, wt_tr);
        avg_cdf_stride(&mut l.sign_cdf, &t.sign_cdf, 2, wt_left, wt_tr);
        avg_cdf_stride(&mut l.class0_hp_cdf, &t.class0_hp_cdf, 2, wt_left, wt_tr);
        avg_cdf_stride(&mut l.hp_cdf, &t.hp_cdf, 2, wt_left, wt_tr);
        avg_cdf_stride(
            &mut l.class0_cdf,
            &t.class0_cdf,
            CLASS0_SIZE,
            wt_left,
            wt_tr,
        );
        avg_cdf_stride(&mut l.bits_cdf, &t.bits_cdf, 2, wt_left, wt_tr);
    }
}

// In case of row-based multi-threading of encoder, since we always
// keep a top - right sync, we can average the top - right SB's CDFs and
// the left SB's CDFs and use the same for current SB's encoding to
// improve the performance. This function facilitates the averaging
// of CDF and used only when row-mt is enabled in encoder.
/// Translation of `av1_avg_cdf_symbols()`.
pub(crate) fn av1_avg_cdf_symbols(
    ctx_left: &mut FrameContext,
    ctx_tr: &FrameContext,
    wt_left: i32,
    wt_tr: i32,
) {
    let (l, t) = (ctx_left, ctx_tr);
    macro_rules! average_cdf {
        ($f:ident, $n:expr) => {
            avg_cdf_stride(&mut l.$f, &t.$f, $n as i32, wt_left, wt_tr)
        };
        ($f:ident [$i:expr], $n:expr) => {
            avg_cdf_stride(&mut l.$f[$i], &t.$f[$i], $n as i32, wt_left, wt_tr)
        };
    }
    average_cdf!(txb_skip_cdf, 2);
    average_cdf!(eob_extra_cdf, 2);
    average_cdf!(dc_sign_cdf, 2);
    average_cdf!(eob_flag_cdf16, 5);
    average_cdf!(eob_flag_cdf32, 6);
    average_cdf!(eob_flag_cdf64, 7);
    average_cdf!(eob_flag_cdf128, 8);
    average_cdf!(eob_flag_cdf256, 9);
    average_cdf!(eob_flag_cdf512, 10);
    average_cdf!(eob_flag_cdf1024, 11);
    average_cdf!(coeff_base_eob_cdf, 3);
    average_cdf!(coeff_base_cdf, 4);
    average_cdf!(coeff_br_cdf, BR_CDF_SIZE);
    average_cdf!(newmv_cdf, 2);
    average_cdf!(zeromv_cdf, 2);
    average_cdf!(refmv_cdf, 2);
    average_cdf!(drl_cdf, 2);
    average_cdf!(inter_compound_mode_cdf, INTER_COMPOUND_MODES);
    average_cdf!(compound_type_cdf, MASKED_COMPOUND_TYPES);
    average_cdf!(wedge_idx_cdf, 16);
    average_cdf!(interintra_cdf, 2);
    average_cdf!(wedge_interintra_cdf, 2);
    average_cdf!(interintra_mode_cdf, INTERINTRA_MODES);
    average_cdf!(motion_mode_cdf, MOTION_MODES);
    average_cdf!(obmc_cdf, 2);
    average_cdf!(palette_y_size_cdf, PALETTE_SIZES);
    average_cdf!(palette_uv_size_cdf, PALETTE_SIZES);
    for j in 0..PALETTE_SIZES as usize {
        let nsymbs = j as i32 + PALETTE_MIN_SIZE;
        average_cdf!(palette_y_color_index_cdf[j], nsymbs);
        average_cdf!(palette_uv_color_index_cdf[j], nsymbs);
    }
    average_cdf!(palette_y_mode_cdf, 2);
    average_cdf!(palette_uv_mode_cdf, 2);
    average_cdf!(comp_inter_cdf, 2);
    average_cdf!(single_ref_cdf, 2);
    average_cdf!(comp_ref_type_cdf, 2);
    average_cdf!(uni_comp_ref_cdf, 2);
    average_cdf!(comp_ref_cdf, 2);
    average_cdf!(comp_bwdref_cdf, 2);
    average_cdf!(txfm_partition_cdf, 2);
    average_cdf!(compound_index_cdf, 2);
    average_cdf!(comp_group_idx_cdf, 2);
    average_cdf!(skip_mode_cdfs, 2);
    average_cdf!(skip_txfm_cdfs, 2);
    average_cdf!(intra_inter_cdf, 2);
    avg_nmv(&mut l.nmvc, &t.nmvc, wt_left, wt_tr);
    avg_nmv(&mut l.ndvc, &t.ndvc, wt_left, wt_tr);
    average_cdf!(intrabc_cdf, 2);
    avg_cdf_stride(&mut l.seg.pred_cdf, &t.seg.pred_cdf, 2, wt_left, wt_tr);
    avg_cdf_stride(
        &mut l.seg.spatial_pred_seg_cdf,
        &t.seg.spatial_pred_seg_cdf,
        MAX_SEGMENTS as i32,
        wt_left,
        wt_tr,
    );
    average_cdf!(filter_intra_cdfs, 2);
    average_cdf!(filter_intra_mode_cdf, FILTER_INTRA_MODES);
    average_cdf!(switchable_restore_cdf, RESTORE_SWITCHABLE_TYPES);
    average_cdf!(wiener_restore_cdf, 2);
    average_cdf!(sgrproj_restore_cdf, 2);
    average_cdf!(y_mode_cdf, INTRA_MODES);
    average_cdf!(uv_mode_cdf[0], UV_INTRA_MODES - 1);
    average_cdf!(uv_mode_cdf[1], UV_INTRA_MODES);
    for i in 0..PARTITION_CONTEXTS as usize {
        if i < 4 {
            average_cdf!(partition_cdf[i], 4);
        } else if i < 16 {
            average_cdf!(partition_cdf[i], 10);
        } else {
            average_cdf!(partition_cdf[i], 8);
        }
    }
    average_cdf!(switchable_interp_cdf, SWITCHABLE_FILTERS);
    average_cdf!(kf_y_cdf, INTRA_MODES);
    average_cdf!(angle_delta_cdf, 2 * MAX_ANGLE_DELTA + 1);
    average_cdf!(tx_size_cdf[0], MAX_TX_DEPTH);
    average_cdf!(tx_size_cdf[1], MAX_TX_DEPTH + 1);
    average_cdf!(tx_size_cdf[2], MAX_TX_DEPTH + 1);
    average_cdf!(tx_size_cdf[3], MAX_TX_DEPTH + 1);
    average_cdf!(delta_q_cdf, DELTA_Q_PROBS + 1);
    average_cdf!(delta_lf_cdf, DELTA_LF_PROBS + 1);
    for i in 0..FRAME_LF_COUNT as usize {
        average_cdf!(delta_lf_multi_cdf[i], DELTA_LF_PROBS + 1);
    }
    average_cdf!(intra_ext_tx_cdf[1], 7);
    average_cdf!(intra_ext_tx_cdf[2], 5);
    average_cdf!(inter_ext_tx_cdf[1], 16);
    average_cdf!(inter_ext_tx_cdf[2], 12);
    average_cdf!(inter_ext_tx_cdf[3], 2);
    average_cdf!(cfl_sign_cdf, CFL_JOINT_SIGNS);
    average_cdf!(cfl_alpha_cdf, CFL_ALPHABET_SIZE);
}

// (encodeframe_utils.h)

pub(crate) fn av1_dealloc_mb_data(mb: &mut Macroblock) {
    for plane in 0..MAX_MB_PLANE {
        mb.plane[plane].src_diff = Vec::new();
    }
}

/// Translation of `av1_alloc_mb_data()`: besides the residual buffers,
/// the coefficient buffers the pick-mode contexts share
/// (`av1_setup_shared_coeff_buffer()`, context_tree.c) and the block's
/// transform type map are allocated here (see context_tree.rs).
pub(crate) fn av1_alloc_mb_data(cpi: &Av1Comp, mb: &mut Macroblock) -> Result<(), ()> {
    let cm = &cpi.common;
    // (use_nonrd_pick_mode: no RD hash records or inter mode buffers; the
    // segment mask is only used by compound prediction)
    debug_assert!(cpi.sf.rt_sf.use_nonrd_pick_mode != 0);
    let num_planes = av1_num_planes(cm);
    for plane in 0..num_planes {
        let subsampling_xy = if plane != 0 {
            cm.seq_params.subsampling_x + cm.seq_params.subsampling_y
        } else {
            0
        };
        let sb_size = (MAX_SB_SQUARE >> subsampling_xy) as usize;
        mb.plane[plane].src_diff = try_vec(sb_size, 0)?;
    }

    // (av1_setup_shared_coeff_buffer())
    let max_sb_square_y = 1usize << NUM_PELS_LOG2_LOOKUP[cm.seq_params.sb_size as usize];
    let max_sb_square_uv =
        max_sb_square_y >> (cm.seq_params.subsampling_x + cm.seq_params.subsampling_y);
    for i in 0..num_planes {
        let max_num_pix = if i == AOM_PLANE_Y {
            max_sb_square_y
        } else {
            max_sb_square_uv
        };
        let p = &mut mb.plane[i];
        p.coeff = try_vec(max_num_pix, 0)?;
        p.qcoeff = try_vec(max_num_pix, 0)?;
        p.dqcoeff = try_vec(max_num_pix, 0)?;
        // (the contexts' end-of-block and entropy context arrays, for the
        // largest block)
        p.eobs = try_vec(max_num_pix / 16, 0)?;
        p.txb_entropy_ctx = try_vec(max_num_pix / 16, 0)?;
    }
    mb.e_mbd.tx_type_map_blk = try_vec((MAX_MIB_SIZE * MAX_MIB_SIZE) as usize, 0)?;

    // (allocate_winner_mode_stats(): not required with the non-RD pick mode)
    Ok(())
}

// This function will compute the number of reference frames to be disabled
// based on selective_ref_frame speed feature.
/// Translation of `get_num_refs_to_disable()`.
fn get_num_refs_to_disable(
    cpi: &Av1Comp,
    ref_frame_flags: i32,
    ref_display_order_hint: &[u32],
    cur_frame_display_index: u32,
) -> u32 {
    let mut num_refs_to_disable = 0;
    if cpi.sf.inter_sf.selective_ref_frame >= 3 {
        num_refs_to_disable += 1;
        if cpi.sf.inter_sf.selective_ref_frame >= 6 {
            // Disable LAST2_FRAME  and ALTREF2_FRAME
            num_refs_to_disable += 2;
        } else if cpi.sf.inter_sf.selective_ref_frame == 5
            && ref_frame_flags & AV1_REF_FRAME_FLAG_LIST[LAST2_FRAME as usize] != 0
        {
            let last2_frame_dist = av1_encoder_get_relative_dist(
                ref_display_order_hint[(LAST2_FRAME - LAST_FRAME) as usize] as i32,
                cur_frame_display_index as i32,
            );
            // Disable LAST2_FRAME if it is a temporally distant frame
            if last2_frame_dist.abs() > 2 {
                num_refs_to_disable += 1;
            } else {
                // (the coded error of the first pass statistics: one pass)
                debug_assert!(!is_stat_consumption_stage_twopass(cpi));
            }
        }
    }
    num_refs_to_disable
}

/// Translation of `get_max_allowed_ref_frames()`.
#[inline]
fn get_max_allowed_ref_frames(
    cpi: &Av1Comp,
    ref_frame_flags: i32,
    ref_display_order_hint: &[u32],
    cur_frame_display_index: u32,
) -> i32 {
    let max_reference_frames = cpi.oxcf.ref_frm_cfg.max_reference_frames as u32;
    let num_refs_to_disable = get_num_refs_to_disable(
        cpi,
        ref_frame_flags,
        ref_display_order_hint,
        cur_frame_display_index,
    );
    let max_allowed_refs_for_given_speed =
        (INTER_REFS_PER_FRAME as u32).wrapping_sub(num_refs_to_disable);
    max_allowed_refs_for_given_speed.min(max_reference_frames) as i32
}

// Enforce the number of references for each arbitrary frame based on user
// options and speed.
/// Translation of `enforce_max_ref_frames()` on `cpi->ref_frame_flags`
/// with the current frame's display order hints.
pub(crate) fn enforce_max_ref_frames(cpi: &mut Av1Comp) {
    let mut ref_frame_flags = cpi.ref_frame_flags;
    let ref_display_order_hint = cpi.common.cur_frame().ref_display_order_hint;
    let cur_frame_display_index = cpi.common.current_frame.display_order_hint;
    let mut total_valid_refs = 0;

    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        if ref_frame_flags & AV1_REF_FRAME_FLAG_LIST[ref_frame as usize] != 0 {
            total_valid_refs += 1;
        }
    }

    let max_allowed_refs = get_max_allowed_ref_frames(
        cpi,
        ref_frame_flags,
        &ref_display_order_hint,
        cur_frame_display_index,
    );

    let mut i = 0;
    while i < 4 && total_valid_refs > max_allowed_refs {
        let ref_frame_to_disable = DISABLE_ORDER[i];
        i += 1;

        if ref_frame_flags & AV1_REF_FRAME_FLAG_LIST[ref_frame_to_disable as usize] == 0 {
            continue;
        }

        match ref_frame_to_disable {
            LAST3_FRAME => ref_frame_flags &= !AOM_LAST3_FLAG,
            LAST2_FRAME => ref_frame_flags &= !AOM_LAST2_FLAG,
            ALTREF2_FRAME => ref_frame_flags &= !AOM_ALT2_FLAG,
            // FIXME (upstream): disabling BWDREF_FRAME clears the GOLDEN_FRAME
            // flag (kept: it is what libaom does).
            BWDREF_FRAME => ref_frame_flags &= !AOM_GOLD_FLAG,
            _ => debug_assert!(false),
        }
        total_valid_refs -= 1;
    }
    debug_assert!(total_valid_refs <= max_allowed_refs);
    cpi.ref_frame_flags = ref_frame_flags;
}
