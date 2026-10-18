// Rust translation of the parts of av1/encoder/tx_search.c and
// av1/encoder/tx_search.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transform size and type search of the RD intra mode search (the
//! real-time key frames' small blocks): for each transform block, the
//! transform types the speed features allow are tried, and the transform
//! sizes of the block's depths. The RD inter search's paths (the
//! variable transform partitions, the residual hash records, the skip
//! prediction) are never reached and fail. The quantized coefficients'
//! buffer of the best transform type is kept by swapping the plane's
//! `dqcoeff` buffer with the macroblock's scratch buffer, as upstream swaps
//! the pointers.

use super::block::*;
use super::encodemb::*;
use super::encoder::Av1Comp;
use super::encodetxb::block_offset;
use super::nonrd_pickmode::av1_block_error;
use super::rd::*;
use super::rdopt::av1_highbd_block_error;
use super::rdopt_utils::{bsize_to_num_blk, get_txb_dimensions};
use super::speed_features::*;
use super::txb_rdopt::av1_cost_coeffs_txb;
use crate::aom::aom_dsp::aom_dsp_common::{round_power_of_two_i64, round_power_of_two_u32};
use crate::aom::aom_dsp::sum_squares::aom_sum_squares_2d_i16;
use crate::aom::av1::common::av1_common_int::txfm_partition_context;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::entropy::EntropyContext;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::idct::{av1_get_tx_scale, av1_inverse_transform_block, TxfmParam};
use crate::aom::av1::common::pred_common::{av1_get_skip_txfm_context, get_tx_size_context};
use crate::aom::av1::common::quant_common::av1_use_qmatrix;
use crate::aom::av1::common::reconintra::av1_predict_intra_block_facade;
use crate::aom::av1::common::txb_common::{get_txb_ctx, TxbCtx};
use crate::aom::av1::encoder::av1_quantize::*;

// (tx_search.h)

// (FAST_TX_SEARCH_MODE)
pub(crate) const FTXS_NONE: i32 = 0;
pub(crate) const FTXS_DCT_AND_1D_DCT_ONLY: i32 = 1 << 0;
pub(crate) const FTXS_DISABLE_TRELLIS_OPT: i32 = 1 << 1;
pub(crate) const FTXS_USE_TRANSFORM_DOMAIN: i32 = 1 << 2;

// (TX_PRUNE_TYPE, block.h)
pub(crate) const TX_PRUNE_NONE: i32 = 0;
pub(crate) const TX_PRUNE_LARGEST: i32 = 1;
pub(crate) const TX_PRUNE_SPLIT: i32 = 2;

const MAX_TX_SCALE: i32 = 1;

/// Translation of `tx_size_cost()`.
#[inline]
pub(crate) fn tx_size_cost(x: &Macroblock, bsize: BlockSize, tx_size: TxSize) -> i32 {
    debug_assert!(bsize == x.e_mbd.mbmi().bsize);
    if x.txfm_search_params.tx_mode_search_type != TX_MODE_SELECT || !block_signals_txsize(bsize) {
        return 0;
    }

    let tx_size_cat = bsize_to_tx_size_cat(bsize) as usize;
    let depth = tx_size_to_depth(tx_size, bsize) as usize;
    let xd = &x.e_mbd;
    let tx_size_ctx = get_tx_size_context(xd) as usize;
    x.mode_costs.tx_size_cost[tx_size_cat][tx_size_ctx][depth]
}

// (tx_search.c)

/// Translation of `struct rdcost_block_args`.
struct RdcostBlockArgs<'a> {
    cpi: &'a Av1Comp,
    x: &'a mut Macroblock,
    t_above: [EntropyContext; MAX_MIB_SIZE as usize],
    t_left: [EntropyContext; MAX_MIB_SIZE as usize],
    rd_stats: RdStats,
    current_rd: i64,
    best_rd: i64,
    exit_early: bool,
    incomplete_exit: bool,
    ftxs_mode: i32,
    skip_trellis: i32,
}

// Compute the pixel domain distortion from diff on all visible 4x4s in the
// transform block.
#[inline]
fn pixel_diff_dist(
    x: &Macroblock,
    plane: usize,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_bsize: BlockSize,
    block_mse_q8: Option<&mut u32>,
) -> i64 {
    let mut visible_rows = 0;
    let mut visible_cols = 0;
    let xd = &x.e_mbd;
    get_txb_dimensions(
        xd,
        plane,
        plane_bsize,
        blk_row,
        blk_col,
        tx_bsize,
        None,
        None,
        &mut visible_cols,
        &mut visible_rows,
    );
    let diff_stride = BLOCK_SIZE_WIDE[plane_bsize as usize] as i32;
    let diff = ((blk_row * diff_stride + blk_col) << MI_SIZE_LOG2) as usize;
    let sse = aom_sum_squares_2d_i16(
        &x.plane[plane].src_diff,
        diff,
        diff_stride as usize,
        visible_cols,
        visible_rows,
    );
    if let Some(block_mse_q8) = block_mse_q8 {
        if visible_cols > 0 && visible_rows > 0 {
            *block_mse_q8 = ((256 * sse) / (visible_cols * visible_rows) as u64) as u32;
        } else {
            *block_mse_q8 = u32::MAX;
        }
    }
    sse as i64
}

#[inline]
fn inverse_transform_block_facade(
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    eob: i32,
    reduced_tx_set: i32,
) -> Result<(), ()> {
    if eob == 0 {
        return Ok(());
    }
    let Macroblock {
        plane: planes,
        e_mbd: xd,
        ..
    } = x;
    let p = &planes[plane];
    let dqcoeff = &p.dqcoeff[block_offset(block)..];
    let plane_type = get_plane_type(plane);
    let tx_size = av1_get_tx_size(plane, xd);
    let tx_type = av1_get_tx_type(xd, plane_type, blk_row, blk_col, tx_size, reduced_tx_set);

    let pd = &xd.plane[plane];
    let dst_stride = pd.dst.stride;
    let dst = pd.dst.buf + ((blk_row * dst_stride + blk_col) << MI_SIZE_LOG2) as usize;
    let lossless = xd.lossless[xd.mbmi().segment_id as usize];
    let bd = xd.bd;
    av1_inverse_transform_block(
        lossless,
        bd,
        dqcoeff,
        tx_type,
        tx_size,
        &mut xd.recon.buffer_alloc,
        dst,
        dst_stride as usize,
        eob,
        reduced_tx_set,
    )
}

#[inline]
fn recon_intra(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    best_tx_type: TxType,
    do_quant: bool,
    best_eob: u16,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let is_inter = is_inter_block(x.e_mbd.mbmi());
    if !is_inter
        && best_eob != 0
        && (blk_row + (TX_SIZE_HIGH_UNIT[tx_size as usize] as i32)
            < MI_SIZE_HIGH[plane_bsize as usize] as i32
            || blk_col + (TX_SIZE_WIDE_UNIT[tx_size as usize] as i32)
                < MI_SIZE_WIDE[plane_bsize as usize] as i32)
    {
        // if the quantized coefficients are stored in the dqcoeff buffer, we don't
        // need to do transform and quantization again.
        if do_quant {
            // (search_tx_type() always passes do_quant 0)
            return Err(());
        }

        let eob = x.plane[plane].eobs[block as usize] as i32;
        inverse_transform_block_facade(
            x,
            plane,
            block,
            blk_row,
            blk_col,
            eob,
            cm.features.reduced_tx_set_used as i32,
        )?;

        // This may happen because of hash collision. The eob stored in the hash
        // table is non-zero, but the real eob is zero. We need to make sure tx_type
        // is DCT_DCT in this case.
        if plane == 0 && x.plane[plane].eobs[block as usize] == 0 && best_tx_type != DCT_DCT {
            update_txk_array(&mut x.e_mbd, blk_row, blk_col, tx_size, DCT_DCT);
        }
    }
    Ok(())
}

#[inline]
fn right_signed_shift(value: i64, shift: i32) -> i64 {
    if shift >= 0 {
        value >> shift
    } else {
        value << -shift
    }
}

#[inline]
fn dist_block_tx_domain(
    x: &Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    qmatrix: Option<&[u8]>,
    out_dist: &mut i64,
    out_sse: &mut i64,
) -> Result<(), ()> {
    let p = &x.plane[plane];
    // Transform domain distortion computation is more efficient as it does
    // not involve an inverse transform, but it is less accurate.
    let buffer_length = av1_get_max_eob(tx_size) as usize;
    let mut this_sse: i64 = 0;
    // TX-domain results need to shift down to Q2/D10 to match pixel
    // domain distortion values which are in Q2^2
    let shift = (MAX_TX_SCALE - av1_get_tx_scale(tx_size)) * 2;
    let off = block_offset(block);
    let coeff = &p.coeff[off..];
    let dqcoeff = &p.dqcoeff[off..];
    let xd = &x.e_mbd;
    if is_cur_buf_hbd(xd) {
        // TODO(veluca): handle use_qm_dist_metric for HBD too.
        *out_dist = av1_highbd_block_error(coeff, dqcoeff, buffer_length, &mut this_sse, xd.bd);
    } else if qmatrix.is_none() || x.txfm_search_params.use_qm_dist_metric == 0 {
        *out_dist = av1_block_error(coeff, dqcoeff, buffer_length, &mut this_sse);
    } else {
        // (av1_block_error_qm(): quantization matrices are off)
        return Err(());
    }

    *out_dist = right_signed_shift(*out_dist, shift);
    *out_sse = right_signed_shift(this_sse, shift);
    Ok(())
}

#[inline]
fn check_bit_mask(mask: u16, val: i32) -> bool {
    mask & (1 << val) != 0
}

// (DCT_ADST_TX_MASK, txfm_common.h)
const DCT_ADST_TX_MASK: u16 = 0x000F; // Either DCT or ADST in each direction

/// Translation of `get_tx_mask()`.
fn get_tx_mask(
    cpi: &Av1Comp,
    x: &Macroblock,
    plane: usize,
    blk_row: i32,
    blk_col: i32,
    tx_size: TxSize,
    ftxs_mode: i32,
    allowed_txk_types: &mut TxType,
) -> Result<u16, ()> {
    let cm = &cpi.common;
    let xd = &x.e_mbd;
    let mbmi = xd.mbmi();
    let txfm_params = &x.txfm_search_params;
    let is_inter = is_inter_block(mbmi);
    let fast_tx_search = ftxs_mode & FTXS_DCT_AND_1D_DCT_ONLY;
    // if txk_allowed = TX_TYPES, >1 tx types are allowed, else, if txk_allowed <
    // TX_TYPES, only that specific tx type is allowed.
    let mut txk_allowed: TxType = TX_TYPES;

    // (update_type and tx_type_probs =
    // cpi->ppi->frame_probs.tx_type_probs[update_type][tx_size]: only read by
    // the pruning below, which the speed features leave off)

    if (!is_inter && txfm_params.use_default_intra_tx_type != 0)
        || (is_inter && txfm_params.default_inter_tx_type_prob_thresh == 0)
    {
        txk_allowed = get_default_tx_type(0, xd, tx_size, cpi.use_screen_content_tools != 0);
    } else if is_inter && txfm_params.default_inter_tx_type_prob_thresh != i32::MAX {
        // (the forced inter transform type by probability: the RD inter
        // search's)
        return Err(());
    } else if x.rd_model == super::enc_enums::LOW_TXFM_RD && plane == 0 {
        txk_allowed = DCT_DCT;
    }

    let tx_set_type = av1_get_ext_tx_set_type(
        tx_size,
        is_inter as i32,
        cm.features.reduced_tx_set_used as i32,
    );

    let mut uv_tx_type = DCT_DCT;
    if plane != 0 {
        // tx_type of PLANE_TYPE_UV should be the same as PLANE_TYPE_Y
        txk_allowed = av1_get_tx_type(
            xd,
            get_plane_type(plane),
            blk_row,
            blk_col,
            tx_size,
            cm.features.reduced_tx_set_used as i32,
        );
        uv_tx_type = txk_allowed;
    }
    let intra_dir = if mbmi.filter_intra_mode_info.use_filter_intra != 0 {
        FIMODE_TO_INTRADIR[mbmi.filter_intra_mode_info.filter_intra_mode as usize]
    } else {
        mbmi.mode
    };
    let mut ext_tx_used_flag = if cpi.sf.tx_sf.tx_type_search.use_reduced_intra_txset != 0
        && tx_set_type == EXT_TX_SET_DTT4_IDTX_1DDCT
    {
        AV1_REDUCED_INTRA_TX_USED_FLAG[intra_dir as usize]
    } else {
        AV1_EXT_TX_USED_FLAG[tx_set_type as usize]
    };

    if cpi.sf.tx_sf.tx_type_search.use_reduced_intra_txset == 2 {
        ext_tx_used_flag &= AV1_DERIVED_INTRA_TX_USED_FLAG[intra_dir as usize];
    }

    if xd.lossless[mbmi.segment_id as usize] != 0
        || TXSIZE_SQR_UP_MAP[tx_size as usize] > TX_32X32
        || ext_tx_used_flag == 0x0001
        || (is_inter && cpi.oxcf.txfm_cfg.use_inter_dct_only)
        || (!is_inter && cpi.oxcf.txfm_cfg.use_intra_dct_only)
    {
        txk_allowed = DCT_DCT;
    }

    if !cpi.oxcf.txfm_cfg.enable_flip_idtx {
        ext_tx_used_flag &= DCT_ADST_TX_MASK;
    }

    let mut allowed_tx_mask: u16; // 1: allow; 0: skip.
    if txk_allowed < TX_TYPES {
        allowed_tx_mask = 1 << txk_allowed;
        allowed_tx_mask &= ext_tx_used_flag;
    } else if fast_tx_search != 0 {
        allowed_tx_mask = 0x0c01; // V_DCT, H_DCT, DCT_DCT
        allowed_tx_mask &= ext_tx_used_flag;
    } else {
        debug_assert!(plane == 0);
        allowed_tx_mask = ext_tx_used_flag;
        let mut num_allowed = 0;

        if cpi.sf.tx_sf.tx_type_search.prune_tx_type_using_stats != 0 {
            // (the transform type pruning by the frame's statistics: off for
            // the saver's speed features)
            return Err(());
        }
        for i in 0..TX_TYPES {
            if allowed_tx_mask & (1 << i) != 0 {
                num_allowed += 1;
            }
        }
        debug_assert!(num_allowed > 0);

        if num_allowed > 2 && cpi.sf.tx_sf.tx_type_search.prune_tx_type_est_rd != 0 {
            // (prune_txk_type_separ() and prune_txk_type(): off for the saver's
            // speed features)
            return Err(());
        } else {
            debug_assert!(num_allowed > 0);
            let allowed_tx_count = if txfm_params.prune_2d_txfm_mode >= TX_TYPE_PRUNE_4 {
                1
            } else {
                5
            };
            // !fast_tx_search && txk_end != txk_start && plane == 0
            if txfm_params.prune_2d_txfm_mode >= TX_TYPE_PRUNE_1
                && is_inter
                && num_allowed > allowed_tx_count
            {
                // (prune_tx_2D(): the RD inter search's)
                return Err(());
            }
        }
    }

    // Need to have at least one transform type allowed.
    if allowed_tx_mask == 0 {
        txk_allowed = if plane != 0 { uv_tx_type } else { DCT_DCT };
        allowed_tx_mask = 1 << txk_allowed;
    }

    debug_assert!(txk_allowed >= TX_TYPES || allowed_tx_mask == 1 << txk_allowed);
    *allowed_txk_types = txk_allowed;
    Ok(allowed_tx_mask)
}

#[inline]
fn skip_trellis_opt_based_on_satd(
    coeff_opt_satd_threshold: u32,
    skip_trellis: i32,
) -> Result<i32, ()> {
    if skip_trellis != 0 || coeff_opt_satd_threshold == u32::MAX {
        return Ok(skip_trellis);
    }
    // (the SATD based trellis decision: the trellis is off for the saver's
    // real-time speed features)
    Err(())
}

// Search for the best transform type for a given transform block.
// This function can be used for both inter and intra, both luma and chroma.
fn search_tx_type(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    txb_ctx: &TxbCtx,
    ftxs_mode: i32,
    mut skip_trellis: i32,
    ref_best_rd: i64,
    best_rd_stats: &mut RdStats,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let mut best_rd = i64::MAX;
    let mut best_eob: u16 = 0;
    let mut best_tx_type = DCT_DCT;
    let mut rate_cost: i32 = 0;
    // The buffer used to swap dqcoeff in macroblockd_plane so we can keep dqcoeff
    // of the best tx_type
    // (the scratch buffer is the macroblock's; `best_is_orig` tells whether
    // best_dqcoeff points to the plane's own buffer, orig_dqcoeff)
    let mut best_dqcoeff = std::mem::take(&mut x.search_tx_dqcoeff);
    if best_dqcoeff.len() != x.plane[plane].dqcoeff.len() {
        best_dqcoeff = vec![0; x.plane[plane].dqcoeff.len()];
    }
    let mut best_is_orig = false;
    let tx_type_map_idx = if plane != 0 {
        0
    } else {
        blk_row * x.e_mbd.tx_type_map_stride + blk_col
    };
    av1_invalid_rd_stats(best_rd_stats);

    let segment_id = x.e_mbd.mbmi().segment_id as usize;
    skip_trellis |= !is_trellis_used(cpi.optimize_seg_arr[segment_id], DRY_RUN_NORMAL) as i32;

    let mut best_txb_ctx: u8 = 0;
    // txk_allowed = TX_TYPES: >1 tx types are allowed
    // txk_allowed < TX_TYPES: only that specific tx type is allowed.
    let mut txk_allowed: TxType = TX_TYPES;
    let txk_map: [TxType; TX_TYPES as usize] =
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let dequant_shift = if is_cur_buf_hbd(&x.e_mbd) {
        x.e_mbd.bd - 5
    } else {
        3
    };
    let qstep = (x.plane[plane].dequant_qtx[1] >> dequant_shift) as i32;

    let txw = TX_SIZE_WIDE[tx_size as usize];
    let txh = TX_SIZE_HIGH[tx_size as usize];
    let mut block_sse: i64;
    let mut block_mse_q8: u32 = 0;
    let dc_only_blk = false;
    let txfm_params = x.txfm_search_params;
    let predict_dc_block = txfm_params.predict_dc_level >= 1 && txw != 64 && txh != 64;
    if predict_dc_block {
        // (predict_dc_only_block(): predict_dc_level is 0 for the saver's
        // speed features)
        x.search_tx_dqcoeff = best_dqcoeff;
        return Err(());
    } else {
        block_sse = pixel_diff_dist(
            x,
            plane,
            blk_row,
            blk_col,
            plane_bsize,
            TXSIZE_TO_BSIZE[tx_size as usize],
            Some(&mut block_mse_q8),
        );
        debug_assert!(block_mse_q8 != u32::MAX);
    }

    // Bit mask to indicate which transform types are allowed in the RD search.
    // Use DCT_DCT transform for DC only block.
    let tx_mask = match get_tx_mask(
        cpi,
        x,
        plane,
        blk_row,
        blk_col,
        tx_size,
        ftxs_mode,
        &mut txk_allowed,
    ) {
        Ok(m) => m,
        Err(()) => {
            x.search_tx_dqcoeff = best_dqcoeff;
            return Err(());
        }
    };
    let allowed_tx_mask = tx_mask;

    if is_cur_buf_hbd(&x.e_mbd) {
        let bd = x.e_mbd.bd;
        block_sse = round_power_of_two_i64(block_sse, (bd - 8) * 2);
        block_mse_q8 = round_power_of_two_u32(block_mse_q8, ((bd - 8) * 2) as u32);
    }
    block_sse *= 16;
    // Use mse / qstep^2 based threshold logic to take decision of R-D
    // optimization of coeffs. For smaller residuals, coeff optimization
    // would be helpful. For larger residuals, R-D optimization may not be
    // effective.
    // TODO(any): Experiment with variance and mean based thresholds
    let perform_block_coeff_opt = (block_mse_q8 as u64)
        <= txfm_params.coeff_opt_thresholds[0] as u64 * qstep as u64 * qstep as u64;
    skip_trellis |= !perform_block_coeff_opt as i32;

    // Flag to indicate if distortion should be calculated in transform domain or
    // not during iterating through transform type candidates.
    // Transform domain distortion is accurate for higher residuals.
    // TODO(any): Experiment with variance and mean based thresholds
    let mut use_transform_domain_distortion = (txfm_params.use_transform_domain_distortion > 0)
        && (block_mse_q8 >= txfm_params.tx_domain_dist_threshold)
        // Any 64-pt transforms only preserves half the coefficients.
        // Therefore transform domain distortion is not valid for these
        // transform sizes.
        && (TXSIZE_SQR_UP_MAP[tx_size as usize] != TX_64X64)
        // Use pixel domain distortion for DC only blocks
        && !dc_only_blk;
    // Flag to indicate if an extra calculation of distortion in the pixel domain
    // should be performed at the end, after the best transform type has been
    // decided.
    let mut calc_pixel_domain_distortion_final = txfm_params.use_transform_domain_distortion == 1
        && use_transform_domain_distortion
        && x.rd_model != super::enc_enums::LOW_TXFM_RD;
    if calc_pixel_domain_distortion_final && (txk_allowed < TX_TYPES || allowed_tx_mask == 0x0001) {
        use_transform_domain_distortion = false;
        calc_pixel_domain_distortion_final = false;
    }

    let mut txfm_param = TxfmParam::default();
    let mut quant_param = QuantParam::default();
    let mut skip_trellis_based_on_satd = [0i32; TX_TYPES as usize];
    av1_setup_xform(cm, x, tx_size, DCT_DCT, &mut txfm_param);
    av1_setup_quant(
        tx_size,
        (skip_trellis == 0) as i32,
        if skip_trellis != 0 {
            if USE_B_QUANT_NO_TRELLIS != 0 {
                AV1_XFORM_QUANT_B
            } else {
                AV1_XFORM_QUANT_FP
            }
        } else {
            AV1_XFORM_QUANT_FP
        },
        cpi.oxcf.q_cfg.quant_b_adapt as i32,
        &mut quant_param,
    );

    let res = (|| -> Result<(), ()> {
        // Iterate through all transform type candidates.
        for idx in 0..TX_TYPES as usize {
            let tx_type = txk_map[idx];
            if tx_type == TX_TYPE_INVALID || !check_bit_mask(allowed_tx_mask, tx_type) {
                continue;
            }
            txfm_param.tx_type = tx_type;
            if av1_use_qmatrix(&cm.quant_params, &x.e_mbd, segment_id as i32) {
                av1_setup_qmatrix(
                    &cm.quant_params,
                    &x.e_mbd,
                    plane,
                    tx_size,
                    tx_type,
                    &mut quant_param,
                );
            }
            if plane == 0 {
                x.e_mbd
                    .tx_type_map_set(tx_type_map_idx as usize, tx_type as u8);
            }
            let mut this_rd_stats = RdStats::default();
            av1_invalid_rd_stats(&mut this_rd_stats);

            // (av1_xform_dc_only(): no DC only blocks)
            av1_xform(x, plane, block, blk_row, blk_col, plane_bsize, &txfm_param)?;

            skip_trellis_based_on_satd[tx_type as usize] =
                skip_trellis_opt_based_on_satd(txfm_params.coeff_opt_thresholds[1], skip_trellis)?;

            av1_quant(x, plane, block, &txfm_param, &quant_param)?;

            // Calculate rate cost of quantized coefficients.
            if quant_param.use_optimize_b != 0 {
                // TODO(aomedia:3209): update Trellis quantization to take into account
                // quantization matrices.
                // (the trellis is off for the saver's real-time speed features)
                return Err(());
            } else {
                rate_cost = av1_cost_coeffs_txb(
                    x,
                    plane,
                    block,
                    tx_size,
                    tx_type,
                    txb_ctx,
                    cm.features.reduced_tx_set_used as i32,
                );
            }

            // If rd cost based on coeff rate alone is already more than best_rd,
            // terminate early.
            if rdcost(x.rdmult, rate_cost, 0) > best_rd {
                continue;
            }

            // Calculate distortion.
            if x.plane[plane].eobs[block as usize] == 0 {
                // When eob is 0, pixel domain distortion is more efficient and accurate.
                this_rd_stats.dist = block_sse;
                this_rd_stats.sse = block_sse;
            } else if use_transform_domain_distortion {
                dist_block_tx_domain(
                    x,
                    plane,
                    block,
                    tx_size,
                    quant_param.qmatrix,
                    &mut this_rd_stats.dist,
                    &mut this_rd_stats.sse,
                )?;
            } else {
                // (dist_block_px_domain(): the pixel domain distortion needs
                // use_transform_domain_distortion 0, which the saver's speed
                // features never set)
                return Err(());
            }

            this_rd_stats.rate = rate_cost;

            let rd = rdcost(x.rdmult, this_rd_stats.rate, this_rd_stats.dist);

            if rd < best_rd {
                best_rd = rd;
                *best_rd_stats = this_rd_stats;
                best_tx_type = tx_type;
                best_txb_ctx = x.plane[plane].txb_entropy_ctx[block as usize];
                best_eob = x.plane[plane].eobs[block as usize];
                // Swap dqcoeff buffers
                std::mem::swap(&mut best_dqcoeff, &mut x.plane[plane].dqcoeff);
                best_is_orig = !best_is_orig;
            }

            // If the current best RD cost is much worse than the reference RD cost,
            // terminate early.
            if cpi.sf.tx_sf.adaptive_txb_search_level != 0
                && (best_rd - (best_rd >> cpi.sf.tx_sf.adaptive_txb_search_level)) > ref_best_rd
            {
                break;
            }

            // Terminate transform type search if the block has been quantized to
            // all zero.
            if cpi.sf.tx_sf.tx_type_search.skip_tx_search != 0 && best_eob == 0 {
                break;
            }
        }

        debug_assert!(best_rd != i64::MAX);

        best_rd_stats.skip_txfm = (best_eob == 0) as u8;
        if plane == 0 {
            update_txk_array(&mut x.e_mbd, blk_row, blk_col, tx_size, best_tx_type);
        }
        x.plane[plane].txb_entropy_ctx[block as usize] = best_txb_ctx;
        x.plane[plane].eobs[block as usize] = best_eob;
        let _skip_trellis = skip_trellis_based_on_satd[best_tx_type as usize];

        // Point dqcoeff to the quantized coefficients corresponding to the best
        // transform type, then we can skip transform and quantization, e.g. in the
        // final pixel domain distortion calculation and recon_intra().
        std::mem::swap(&mut best_dqcoeff, &mut x.plane[plane].dqcoeff);
        best_is_orig = !best_is_orig;

        if calc_pixel_domain_distortion_final && best_eob != 0 {
            // (dist_block_px_domain(): see above)
            return Err(());
        }

        // Intra mode needs decoded pixels such that the next transform block
        // can use them for prediction.
        recon_intra(
            cpi,
            x,
            plane,
            block,
            blk_row,
            blk_col,
            plane_bsize,
            tx_size,
            best_tx_type,
            false,
            best_eob,
        )
    })();
    // p->dqcoeff = orig_dqcoeff
    // (best_dqcoeff holds the plane's own buffer when best_is_orig)
    if best_is_orig {
        std::mem::swap(&mut best_dqcoeff, &mut x.plane[plane].dqcoeff);
    }
    x.search_tx_dqcoeff = best_dqcoeff;
    res
}

// Pick transform type for a luma transform block of tx_size. Note this function
// is used only for inter-predicted blocks.
// (tx_type_rd(): the RD inter search's)

/// Translation of `get_search_init_depth()`.
fn get_search_init_depth(
    mi_width: i32,
    mi_height: i32,
    is_inter: bool,
    sf: &SpeedFeatures,
    tx_size_search_method: i32,
) -> i32 {
    if tx_size_search_method == USE_LARGESTALL {
        return MAX_VARTX_DEPTH;
    }

    if sf.tx_sf.tx_size_search_lgr_block != 0
        && (mi_width > MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32
            || mi_height > MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32)
    {
        return MAX_VARTX_DEPTH;
    }

    if is_inter {
        if mi_height != mi_width {
            sf.tx_sf.inter_tx_size_search_init_depth_rect
        } else {
            sf.tx_sf.inter_tx_size_search_init_depth_sqr
        }
    } else if mi_height != mi_width {
        sf.tx_sf.intra_tx_size_search_init_depth_rect
    } else {
        sf.tx_sf.intra_tx_size_search_init_depth_sqr
    }
}

fn choose_largest_tx_size(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    ref_best_rd: i64,
    bs: BlockSize,
) -> Result<(), ()> {
    let txfm_params = x.txfm_search_params;
    let mut tx_size = tx_size_from_tx_mode(bs, txfm_params.tx_mode_search_type);

    // If tx64 is not enabled, we need to go down to the next available size
    if !cpi.oxcf.txfm_cfg.enable_tx64 && cpi.oxcf.txfm_cfg.enable_rect_tx {
        static TX_SIZE_MAX_32: [TxSize; TX_SIZES_ALL as usize] = [
            TX_4X4,   // 4x4 transform
            TX_8X8,   // 8x8 transform
            TX_16X16, // 16x16 transform
            TX_32X32, // 32x32 transform
            TX_32X32, // 64x64 transform
            TX_4X8,   // 4x8 transform
            TX_8X4,   // 8x4 transform
            TX_8X16,  // 8x16 transform
            TX_16X8,  // 16x8 transform
            TX_16X32, // 16x32 transform
            TX_32X16, // 32x16 transform
            TX_32X32, // 32x64 transform
            TX_32X32, // 64x32 transform
            TX_4X16,  // 4x16 transform
            TX_16X4,  // 16x4 transform
            TX_8X32,  // 8x32 transform
            TX_32X8,  // 32x8 transform
            TX_16X32, // 16x64 transform
            TX_32X16, // 64x16 transform
        ];
        tx_size = TX_SIZE_MAX_32[tx_size as usize];
    } else if cpi.oxcf.txfm_cfg.enable_tx64 && !cpi.oxcf.txfm_cfg.enable_rect_tx {
        static TX_SIZE_MAX_SQUARE: [TxSize; TX_SIZES_ALL as usize] = [
            TX_4X4,   // 4x4 transform
            TX_8X8,   // 8x8 transform
            TX_16X16, // 16x16 transform
            TX_32X32, // 32x32 transform
            TX_64X64, // 64x64 transform
            TX_4X4,   // 4x8 transform
            TX_4X4,   // 8x4 transform
            TX_8X8,   // 8x16 transform
            TX_8X8,   // 16x8 transform
            TX_16X16, // 16x32 transform
            TX_16X16, // 32x16 transform
            TX_32X32, // 32x64 transform
            TX_32X32, // 64x32 transform
            TX_4X4,   // 4x16 transform
            TX_4X4,   // 16x4 transform
            TX_8X8,   // 8x32 transform
            TX_8X8,   // 32x8 transform
            TX_16X16, // 16x64 transform
            TX_16X16, // 64x16 transform
        ];
        tx_size = TX_SIZE_MAX_SQUARE[tx_size as usize];
    } else if !cpi.oxcf.txfm_cfg.enable_tx64 && !cpi.oxcf.txfm_cfg.enable_rect_tx {
        static TX_SIZE_MAX_32_SQUARE: [TxSize; TX_SIZES_ALL as usize] = [
            TX_4X4,   // 4x4 transform
            TX_8X8,   // 8x8 transform
            TX_16X16, // 16x16 transform
            TX_32X32, // 32x32 transform
            TX_32X32, // 64x64 transform
            TX_4X4,   // 4x8 transform
            TX_4X4,   // 8x4 transform
            TX_8X8,   // 8x16 transform
            TX_8X8,   // 16x8 transform
            TX_16X16, // 16x32 transform
            TX_16X16, // 32x16 transform
            TX_32X32, // 32x64 transform
            TX_32X32, // 64x32 transform
            TX_4X4,   // 4x16 transform
            TX_4X4,   // 16x4 transform
            TX_8X8,   // 8x32 transform
            TX_8X8,   // 32x8 transform
            TX_16X16, // 16x64 transform
            TX_16X16, // 64x16 transform
        ];
        tx_size = TX_SIZE_MAX_32_SQUARE[tx_size as usize];
    }
    x.e_mbd.mbmi_mut().tx_size = tx_size;

    let xd = &x.e_mbd;
    let skip_ctx = av1_get_skip_txfm_context(xd) as usize;
    let no_skip_txfm_rate = x.mode_costs.skip_txfm_cost[skip_ctx][0];
    let skip_txfm_rate = x.mode_costs.skip_txfm_cost[skip_ctx][1];
    // Skip RDcost is used only for Inter blocks
    let skip_txfm_rd = if is_inter_block(xd.mbmi()) {
        rdcost(x.rdmult, skip_txfm_rate, 0)
    } else {
        i64::MAX
    };
    let no_skip_txfm_rd = rdcost(x.rdmult, no_skip_txfm_rate, 0);
    let skip_trellis = 0;
    av1_txfm_rd_in_plane(
        x,
        cpi,
        rd_stats,
        ref_best_rd,
        no_skip_txfm_rd.min(skip_txfm_rd),
        AOM_PLANE_Y,
        bs,
        tx_size,
        FTXS_NONE,
        skip_trellis,
    )
}

// Search for the best uniform transform size and type for current coding block.
fn choose_tx_size_type_from_rd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    ref_best_rd: i64,
    bs: BlockSize,
) -> Result<(), ()> {
    av1_invalid_rd_stats(rd_stats);

    let max_rect_tx_size = MAX_TXSIZE_RECT_LOOKUP[bs as usize];
    let tx_select = x.txfm_search_params.tx_mode_search_type == TX_MODE_SELECT;
    let mut start_tx;
    // The split depth can be at most MAX_TX_DEPTH, so the init_depth controls
    // how many times of splitting is allowed during the RD search.
    let init_depth;

    if tx_select {
        start_tx = max_rect_tx_size;
        init_depth = get_search_init_depth(
            MI_SIZE_WIDE[bs as usize] as i32,
            MI_SIZE_HIGH[bs as usize] as i32,
            is_inter_block(x.e_mbd.mbmi()),
            &cpi.sf,
            x.txfm_search_params.tx_size_search_method,
        );
        if init_depth == MAX_TX_DEPTH
            && !cpi.oxcf.txfm_cfg.enable_tx64
            && TXSIZE_SQR_UP_MAP[start_tx as usize] == TX_64X64
        {
            start_tx = SUB_TX_SIZE_MAP[start_tx as usize];
        }
    } else {
        let chosen_tx_size = tx_size_from_tx_mode(bs, x.txfm_search_params.tx_mode_search_type);
        start_tx = chosen_tx_size;
        init_depth = MAX_TX_DEPTH;
    }

    let skip_trellis = 0;
    let mut best_txk_type_map = [0u8; (MAX_MIB_SIZE * MAX_MIB_SIZE) as usize];
    let mut best_blk_skip = [0u8; (MAX_MIB_SIZE * MAX_MIB_SIZE) as usize];
    let mut best_tx_size = max_rect_tx_size;
    let mut best_rd = i64::MAX;
    let num_blks = bsize_to_num_blk(bs) as usize;
    x.rd_model = super::enc_enums::FULL_TXFM_RD;
    let mut rd = [i64::MAX; (MAX_TX_DEPTH + 1) as usize];
    let mut tx_size = start_tx;
    let mut depth = init_depth;
    while depth <= MAX_TX_DEPTH {
        if (!cpi.oxcf.txfm_cfg.enable_tx64 && TXSIZE_SQR_UP_MAP[tx_size as usize] == TX_64X64)
            || (!cpi.oxcf.txfm_cfg.enable_rect_tx
                && TX_SIZE_WIDE[tx_size as usize] != TX_SIZE_HIGH[tx_size as usize])
        {
            depth += 1;
            tx_size = SUB_TX_SIZE_MAP[tx_size as usize];
            continue;
        }

        if x.txfm_search_params.nn_prune_depths_for_intra_tx == TX_PRUNE_SPLIT {
            break;
        }

        // Set the flag to enable the evaluation of NN classifier to prune transform
        // depths. As the features are based on intra residual information of
        // largest transform, the evaluation of NN model is enabled only for this
        // case.
        x.txfm_search_params.enable_nn_prune_intra_tx_depths =
            cpi.sf.tx_sf.prune_intra_tx_depths_using_nn && tx_size == start_tx;

        let mut this_rd_stats = RdStats::default();
        rd[depth as usize] = av1_uniform_txfm_yrd(
            cpi,
            x,
            &mut this_rd_stats,
            ref_best_rd,
            bs,
            tx_size,
            FTXS_NONE,
            skip_trellis,
        )?;
        if rd[depth as usize] < best_rd {
            best_blk_skip[..num_blks].copy_from_slice(&x.txfm_search_info.blk_skip[..num_blks]);
            for (i, t) in best_txk_type_map[..num_blks].iter_mut().enumerate() {
                *t = x.e_mbd.tx_type_map_get(i);
            }
            best_tx_size = tx_size;
            best_rd = rd[depth as usize];
            *rd_stats = this_rd_stats;
        }
        if tx_size == TX_4X4 {
            break;
        }
        // If we are searching three depths, prune the smallest size depending
        // on rd results for the first two depths for low contrast blocks.
        if depth > init_depth
            && depth != MAX_TX_DEPTH
            && x.source_variance < 256
            && rd[depth as usize - 1] != i64::MAX
            && rd[depth as usize] > rd[depth as usize - 1]
        {
            break;
        }
        depth += 1;
        tx_size = SUB_TX_SIZE_MAP[tx_size as usize];
    }

    if rd_stats.rate != i32::MAX {
        x.e_mbd.mbmi_mut().tx_size = best_tx_size;
        for (i, &t) in best_txk_type_map[..num_blks].iter().enumerate() {
            x.e_mbd.tx_type_map_set(i, t);
        }
        x.txfm_search_info.blk_skip[..num_blks].copy_from_slice(&best_blk_skip[..num_blks]);
    }

    // Reset the flags to avoid any unintentional evaluation of NN model and
    // consumption of prune depths.
    x.txfm_search_params.enable_nn_prune_intra_tx_depths = false;
    x.txfm_search_params.nn_prune_depths_for_intra_tx = TX_PRUNE_NONE;
    Ok(())
}

// Search for the best transform type for the given transform block in the
// given plane/channel, and calculate the corresponding RD cost.
fn block_rd_txfm(
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    args: &mut RdcostBlockArgs,
) -> Result<(), ()> {
    if args.exit_early {
        args.incomplete_exit = true;
        return Ok(());
    }

    let cpi = args.cpi;
    let x = &mut *args.x;
    let is_inter = is_inter_block(x.e_mbd.mbmi());
    let cm = &cpi.common;
    let mut this_rd_stats = RdStats::default();
    av1_init_rd_stats(&mut this_rd_stats);

    if !is_inter {
        av1_predict_intra_block_facade(cm, &mut x.e_mbd, plane, blk_col, blk_row, tx_size)?;
        with_source(x, |x, src| {
            av1_subtract_txb(x, src, plane, plane_bsize, blk_col, blk_row, tx_size)
        });
        if x.txfm_search_params.enable_nn_prune_intra_tx_depths {
            // (ml_predict_intra_tx_depth_prune(): off for the saver's speed
            // features)
            return Err(());
        }
    }

    let mut txb_ctx = TxbCtx::default();
    get_txb_ctx(
        plane_bsize,
        tx_size,
        plane,
        &args.t_above[blk_col as usize..],
        &args.t_left[blk_row as usize..],
        &mut txb_ctx,
    );
    search_tx_type(
        cpi,
        x,
        plane,
        block,
        blk_row,
        blk_col,
        plane_bsize,
        tx_size,
        &txb_ctx,
        args.ftxs_mode,
        args.skip_trellis,
        args.best_rd - args.current_rd,
        &mut this_rd_stats,
    )?;

    if plane == AOM_PLANE_Y && x.e_mbd.cfl.store_y != 0 {
        debug_assert!(!is_inter || plane_bsize < BLOCK_8X8);
        crate::aom::av1::common::cfl::cfl_store_tx(
            &mut x.e_mbd,
            blk_row,
            blk_col,
            tx_size,
            plane_bsize,
        )?;
    }

    av1_set_txb_context(
        x,
        plane,
        block,
        tx_size,
        &mut args.t_above[blk_col as usize..],
        &mut args.t_left[blk_row as usize..],
    );

    let blk_idx = (blk_row * (BLOCK_SIZE_WIDE[plane_bsize as usize] as i32 >> MI_SIZE_LOG2)
        + blk_col) as usize;
    let txfm_info = &mut x.txfm_search_info;
    if plane == 0 {
        set_blk_skip(
            &mut txfm_info.blk_skip,
            plane,
            blk_idx,
            x.plane[plane].eobs[block as usize] == 0,
        );
    } else {
        set_blk_skip(&mut txfm_info.blk_skip, plane, blk_idx, false);
    }

    let rd;
    if is_inter {
        let no_skip_txfm_rd = rdcost(x.rdmult, this_rd_stats.rate, this_rd_stats.dist);
        let skip_txfm_rd = rdcost(x.rdmult, 0, this_rd_stats.sse);
        rd = no_skip_txfm_rd.min(skip_txfm_rd);
        this_rd_stats.skip_txfm &= (x.plane[plane].eobs[block as usize] == 0) as u8;
    } else {
        // Signal non-skip_txfm for Intra blocks
        rd = rdcost(x.rdmult, this_rd_stats.rate, this_rd_stats.dist);
        this_rd_stats.skip_txfm = 0;
    }

    av1_merge_rd_stats(&mut args.rd_stats, &this_rd_stats);

    args.current_rd += rd;
    if args.current_rd > args.best_rd {
        args.exit_early = true;
    }
    Ok(())
}

/// Translation of `av1_uniform_txfm_yrd()`.
pub(crate) fn av1_uniform_txfm_yrd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    ref_best_rd: i64,
    bs: BlockSize,
    tx_size: TxSize,
    ftxs_mode: i32,
    skip_trellis: i32,
) -> Result<i64, ()> {
    let xd = &x.e_mbd;
    let mbmi = *xd.mbmi();
    let txfm_params = &x.txfm_search_params;
    let mode_costs = &x.mode_costs;
    let is_inter = is_inter_block(&mbmi);
    let tx_select =
        txfm_params.tx_mode_search_type == TX_MODE_SELECT && block_signals_txsize(mbmi.bsize);
    let mut tx_size_rate = 0;
    if tx_select {
        let ctx = txfm_partition_context(
            xd.above_txfm()[xd.above_txfm_context],
            xd.left_txfm_context_buffer[xd.left_txfm_context],
            mbmi.bsize,
            tx_size,
        ) as usize;
        tx_size_rate = if is_inter {
            mode_costs.txfm_partition_cost[ctx][0]
        } else {
            tx_size_cost(x, bs, tx_size)
        };
    }
    let skip_ctx = av1_get_skip_txfm_context(xd) as usize;
    let no_skip_txfm_rate = mode_costs.skip_txfm_cost[skip_ctx][0];
    let skip_txfm_rate = mode_costs.skip_txfm_cost[skip_ctx][1];
    let skip_txfm_rd = if is_inter {
        rdcost(x.rdmult, skip_txfm_rate, 0)
    } else {
        i64::MAX
    };
    let no_this_rd = rdcost(x.rdmult, no_skip_txfm_rate + tx_size_rate, 0);

    x.e_mbd.mbmi_mut().tx_size = tx_size;
    av1_txfm_rd_in_plane(
        x,
        cpi,
        rd_stats,
        ref_best_rd,
        no_this_rd.min(skip_txfm_rd),
        AOM_PLANE_Y,
        bs,
        tx_size,
        ftxs_mode,
        skip_trellis,
    )?;
    if rd_stats.rate == i32::MAX {
        return Ok(i64::MAX);
    }

    let mut rd;
    // rdstats->rate should include all the rate except skip/non-skip cost as the
    // same is accounted in the caller functions after rd evaluation of all
    // planes. However the decisions should be done after considering the
    // skip/non-skip header cost
    if rd_stats.skip_txfm != 0 && is_inter {
        rd = rdcost(x.rdmult, skip_txfm_rate, rd_stats.sse);
    } else {
        // Intra blocks are always signalled as non-skip
        rd = rdcost(
            x.rdmult,
            rd_stats.rate + no_skip_txfm_rate + tx_size_rate,
            rd_stats.dist,
        );
        rd_stats.rate += tx_size_rate;
    }
    // Check if forcing the block to skip transform leads to smaller RD cost.
    if is_inter && rd_stats.skip_txfm == 0 && x.e_mbd.lossless[mbmi.segment_id as usize] == 0 {
        let temp_skip_txfm_rd = rdcost(x.rdmult, skip_txfm_rate, rd_stats.sse);
        if temp_skip_txfm_rd <= rd {
            rd = temp_skip_txfm_rd;
            rd_stats.rate = 0;
            rd_stats.dist = rd_stats.sse;
            rd_stats.skip_txfm = 1;
        }
    }

    Ok(rd)
}

/// Translation of `av1_pick_uniform_tx_size_type_yrd()`.
pub(crate) fn av1_pick_uniform_tx_size_type_yrd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    bs: BlockSize,
    ref_best_rd: i64,
) -> Result<(), ()> {
    let mbmi = *x.e_mbd.mbmi();
    let tx_params = x.txfm_search_params;
    debug_assert!(bs == mbmi.bsize);
    let is_inter = is_inter_block(&mbmi);

    av1_init_rd_stats(rd_stats);

    // Hashing based speed feature for inter blocks. If the hash of the residue
    // block is found in the table, use previously saved search results and
    // terminate early.
    if is_inter && cpi.sf.rd_sf.use_mb_rd_hash != 0 {
        // (the residual hash records: the RD inter search's)
        return Err(());
    }

    // If we predict that skip is the optimal RD decision - set the respective
    // context and terminate early.
    if tx_params.skip_txfm_level != 0 && is_inter {
        // (predict_skip_txfm(): the RD inter search's)
        return Err(());
    }

    if x.e_mbd.lossless[mbmi.segment_id as usize] != 0 {
        // Lossless mode can only pick the smallest (4x4) transform size.
        // (choose_smallest_tx_size(): the 4x4 lossless blocks never take the
        // RD search, which only codes blocks smaller than 16x16, as small as
        // the 8x8 of the variance partition)
        return Err(());
    } else if tx_params.tx_size_search_method == USE_LARGESTALL {
        choose_largest_tx_size(cpi, x, rd_stats, ref_best_rd, bs)?;
    } else {
        choose_tx_size_type_from_rd(cpi, x, rd_stats, ref_best_rd, bs)?;
    }

    // Save the RD search results into mb_rd_record for possible reuse in future.
    // (no mb_rd_record: see above)
    Ok(())
}

/// Translation of `av1_txfm_uvrd()`.
pub(crate) fn av1_txfm_uvrd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    bsize: BlockSize,
    ref_best_rd: i64,
) -> Result<bool, ()> {
    av1_init_rd_stats(rd_stats);
    if ref_best_rd < 0 {
        return Ok(false);
    }
    if !x.e_mbd.is_chroma_ref {
        return Ok(true);
    }

    let xd = &x.e_mbd;
    let pd = &xd.plane[AOM_PLANE_U];
    let is_inter = is_inter_block(xd.mbmi());
    let mut this_rd: i64 = 0;
    let mut skip_txfm_rd: i64 = 0;
    let plane_bsize = get_plane_block_size(bsize, pd.subsampling_x, pd.subsampling_y);

    if is_inter {
        // (av1_subtract_plane(): the RD inter search's)
        return Err(());
    }

    let skip_trellis = 0;
    let uv_tx_size = av1_get_tx_size(AOM_PLANE_U, xd);
    let mut is_cost_valid = true;
    for plane in 1..MAX_MB_PLANE {
        let mut this_rd_stats = RdStats::default();
        let mut chroma_ref_best_rd = ref_best_rd;
        // For inter blocks, refined ref_best_rd is used for early exit
        // For intra blocks, even though current rd crosses ref_best_rd, early
        // exit is not recommended as current rd is used for gating subsequent
        // modes as well (say, for angular modes)
        // TODO(any): Extend the early exit mechanism for intra modes as well
        if cpi.sf.inter_sf.perform_best_rd_based_gating_for_chroma != 0
            && is_inter
            && chroma_ref_best_rd != i64::MAX
        {
            chroma_ref_best_rd = ref_best_rd - this_rd.min(skip_txfm_rd);
        }
        av1_txfm_rd_in_plane(
            x,
            cpi,
            &mut this_rd_stats,
            chroma_ref_best_rd,
            0,
            plane,
            plane_bsize,
            uv_tx_size,
            FTXS_NONE,
            skip_trellis,
        )?;
        if this_rd_stats.rate == i32::MAX {
            is_cost_valid = false;
            break;
        }
        av1_merge_rd_stats(rd_stats, &this_rd_stats);
        this_rd = rdcost(x.rdmult, rd_stats.rate, rd_stats.dist);
        skip_txfm_rd = rdcost(x.rdmult, 0, rd_stats.sse);
        if this_rd.min(skip_txfm_rd) > ref_best_rd {
            is_cost_valid = false;
            break;
        }
    }

    if !is_cost_valid {
        // reset cost value
        av1_invalid_rd_stats(rd_stats);
    }

    Ok(is_cost_valid)
}

/// Translation of `av1_txfm_rd_in_plane()`.
pub(crate) fn av1_txfm_rd_in_plane(
    x: &mut Macroblock,
    cpi: &Av1Comp,
    rd_stats: &mut RdStats,
    ref_best_rd: i64,
    current_rd: i64,
    plane: usize,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    ftxs_mode: i32,
    skip_trellis: i32,
) -> Result<(), ()> {
    debug_assert!(plane != 0 || x.e_mbd.mbmi().tx_size == tx_size);

    if !cpi.oxcf.txfm_cfg.enable_tx64 && TXSIZE_SQR_UP_MAP[tx_size as usize] == TX_64X64 {
        av1_invalid_rd_stats(rd_stats);
        return Ok(());
    }

    if current_rd > ref_best_rd {
        av1_invalid_rd_stats(rd_stats);
        return Ok(());
    }

    let mut args = RdcostBlockArgs {
        cpi,
        x,
        t_above: [0; MAX_MIB_SIZE as usize],
        t_left: [0; MAX_MIB_SIZE as usize],
        rd_stats: RdStats::default(),
        current_rd,
        best_rd: ref_best_rd,
        exit_early: false,
        incomplete_exit: false,
        ftxs_mode,
        skip_trellis,
    };
    av1_init_rd_stats(&mut args.rd_stats);

    av1_get_entropy_contexts(
        plane_bsize,
        &args.x.e_mbd,
        plane,
        &mut args.t_above,
        &mut args.t_left,
    );
    av1_foreach_transformed_block_in_plane(
        &mut args,
        |a| &a.x.e_mbd,
        plane_bsize,
        plane,
        |a, plane, block, blk_row, blk_col, plane_bsize, tx_size| {
            block_rd_txfm(plane, block, blk_row, blk_col, plane_bsize, tx_size, a)
        },
    )?;

    let is_inter = is_inter_block(args.x.e_mbd.mbmi());
    let invalid_rd = if is_inter {
        args.incomplete_exit
    } else {
        args.exit_early
    };

    if invalid_rd {
        av1_invalid_rd_stats(rd_stats);
    } else {
        *rd_stats = args.rd_stats;
    }
    Ok(())
}
