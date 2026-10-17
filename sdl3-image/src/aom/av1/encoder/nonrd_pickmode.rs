// Rust translation of the parts of av1/encoder/nonrd_pickmode.c (and of
// av1/encoder/nonrd_opt.h, av1_block_error_c() from av1/encoder/rdopt.c
// and av1_copy_mbmi_ext_to_mbmi_ext_frame() from av1/encoder/rdopt.h) from
// aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the non-RD intra mode search of key frames, for high bit depth buffers.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Non-RD optimized intra mode search: each candidate luma mode's cost is
//! estimated with Hadamard transforms of the residual (chroma always uses
//! DC).

use super::av1_quantize::av1_quantize_fp;
use super::block::*;
use super::context_tree::PickModeContext;
use super::cost::AV1_PROB_COST_SHIFT;
use super::encodemb::av1_foreach_transformed_block_in_plane;
use super::encoder::Av1Comp;
use super::rd::{av1_invalid_rd_stats, rdcost};
use crate::aom::aom_dsp::aom_dsp_common::{get_msb, TranLowT};
use crate::aom::aom_dsp::avg::{aom_hadamard_16x16, aom_hadamard_8x8, aom_satd};
use crate::aom::aom_dsp::fwd_txfm::aom_fdct4x4;
use crate::aom::aom_dsp::subtract::aom_highbd_subtract_block;
use crate::aom::av1::common::av1_common_int::Av1Common;
use crate::aom::av1::common::av1_common_int::{EIGHTTAP_REGULAR, SWITCHABLE};
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mv::INVALID_MV;
use crate::aom::av1::common::mvref_common::av1_ref_frame_type;
use crate::aom::av1::common::pred_common::av1_get_skip_txfm_context;
use crate::aom::av1::common::reconintra::av1_predict_intra_block_facade;
use crate::aom::av1::common::scan::{
    get_scan, DEFAULT_SCAN_8X8_TRANSPOSE, DEFAULT_SCAN_FP_16X16_TRANSPOSE,
};

use super::enc_enums::*;

// (nonrd_opt.h)

pub(crate) const RTC_INTER_MODES: i32 = 4;
pub(crate) const RTC_INTRA_MODES: i32 = 4;
pub(crate) const RTC_MODES: usize = 4; // AOMMAX(RTC_INTER_MODES, RTC_INTRA_MODES)

pub(crate) static INTRA_MODE_LIST: [PredictionMode; 4] = [DC_PRED, V_PRED, H_PRED, SMOOTH_PRED];

pub(crate) static INTER_MODE_LIST: [PredictionMode; 4] = [NEARESTMV, NEARMV, GLOBALMV, NEWMV];

pub(crate) static MODE_IDX: [[i32; RTC_MODES]; REF_FRAMES as usize] = [
    [THR_DC, THR_V_PRED, THR_H_PRED, THR_SMOOTH],
    [THR_NEARESTMV, THR_NEARMV, THR_GLOBALMV, THR_NEWMV],
    [THR_NEARESTL2, THR_NEARL2, THR_GLOBALL2, THR_NEWL2],
    [THR_NEARESTL3, THR_NEARL3, THR_GLOBALL3, THR_NEWL3],
    [THR_NEARESTG, THR_NEARG, THR_GLOBALG, THR_NEWG],
    [THR_NEARESTB, THR_NEARB, THR_GLOBALB, THR_NEWB],
    [THR_NEARESTA2, THR_NEARA2, THR_GLOBALA2, THR_NEWA2],
    [THR_NEARESTA, THR_NEARA, THR_GLOBALA, THR_NEWA],
];

// (rdopt.c)

/// Translation of `av1_block_error_c()`: the products are 32-bit, as
/// upstream's (which wrap as the platforms' 32-bit multiplications do).
pub(crate) fn av1_block_error(
    coeff: &[TranLowT],
    dqcoeff: &[TranLowT],
    block_size: usize,
    ssz: &mut i64,
) -> i64 {
    let mut error: i64 = 0;
    let mut sqcoeff: i64 = 0;

    for i in 0..block_size {
        let diff = coeff[i].wrapping_sub(dqcoeff[i]);
        error += diff.wrapping_mul(diff) as i64;
        sqcoeff += coeff[i].wrapping_mul(coeff[i]) as i64;
    }

    *ssz = sqcoeff;
    error
}

// (rdopt.h)

// This function will copy the best reference mode information from
// MB_MODE_INFO_EXT to MB_MODE_INFO_EXT_FRAME.
#[inline]
pub(crate) fn av1_copy_mbmi_ext_to_mbmi_ext_frame(
    mbmi_ext_best: &mut MbModeInfoExtFrame,
    mbmi_ext: &MbModeInfoExt,
    ref_frame_type: u8,
) {
    mbmi_ext_best.ref_mv_stack = mbmi_ext.ref_mv_stack[ref_frame_type as usize];
    mbmi_ext_best.weight = mbmi_ext.weight[ref_frame_type as usize];
    mbmi_ext_best.mode_context = mbmi_ext.mode_context[ref_frame_type as usize];
    mbmi_ext_best.ref_mv_count = mbmi_ext.ref_mv_count[ref_frame_type as usize];
    mbmi_ext_best.global_mvs = mbmi_ext.global_mvs;
}

// (nonrd_pickmode.c)

#[inline(always)]
fn update_yrd_loop_vars_hbd(
    x: &mut Macroblock,
    skippable: &mut i32,
    step: i32,
    ncoeffs: i32,
    coeff: &[TranLowT],
    qcoeff: &[TranLowT],
    dqcoeff: &[TranLowT],
    this_rdc: &mut RdStats,
    eob_cost: &mut i32,
    tx_blk_id: i32,
) {
    let is_txfm_skip = (ncoeffs == 0) as i32;
    *skippable &= is_txfm_skip;
    x.txfm_search_info.blk_skip[tx_blk_id as usize] = is_txfm_skip as u8;
    *eob_cost += get_msb((ncoeffs + 1) as u32);

    let mut dummy: i64 = 0;
    if ncoeffs == 1 {
        this_rdc.rate += qcoeff[0].abs();
    } else if ncoeffs > 1 {
        this_rdc.rate += aom_satd(qcoeff, step << 4);
    }

    this_rdc.dist += av1_block_error(coeff, dqcoeff, (step << 4) as usize, &mut dummy) >> 2;
}

/*\brief Calculates RD Cost using Hadamard transform.
*
* Calculates RD Cost using Hadamard transform. For low bit depth this function
* uses low-precision set of functions (16-bit) and 32 bit for high bit depth
* \param[in]    x              Pointer to structure holding all the data for
                               the current macroblock
* \param[in]    this_rdc       Pointer to calculated RD Cost
* \param[in]    skippable      Pointer to a flag indicating possible tx skip
* \param[in]    bsize          Current block size
* \param[in]    tx_size        Transform size
* \param[in]    is_inter_mode  Flag to indicate inter mode
*
* \remark Nothing is returned. Instead, calculated RD cost is placed to
* \c this_rdc. \c skippable flag is set if there is no non-zero quantized
* coefficients for Hadamard transform
*/
// (`src` is the source frame's samples; the buffers are always high bit
// depth, so the low bit depth paths are left out)
fn block_yrd(
    x: &mut Macroblock,
    src: &[u16],
    this_rdc: &mut RdStats,
    skippable: &mut i32,
    bsize: BlockSize,
    tx_size: TxSize,
    is_inter_mode: i32,
) {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let num_4x4_w = MI_SIZE_WIDE[bsize as usize] as i32;
    let num_4x4_h = MI_SIZE_HIGH[bsize as usize] as i32;
    let step = 1 << (tx_size << 1);
    let block_step = 1 << tx_size;
    let row_step = step * num_4x4_w >> tx_size;
    let mut block = 0;
    let xd = &x.e_mbd;
    let max_blocks_wide = num_4x4_w
        + if xd.mb_to_right_edge >= 0 {
            0
        } else {
            xd.mb_to_right_edge >> 5
        };
    let max_blocks_high = num_4x4_h
        + if xd.mb_to_bottom_edge >= 0 {
            0
        } else {
            xd.mb_to_bottom_edge >> 5
        };
    let mut eob_cost = 0;
    let bw = 4 * num_4x4_w;
    let bh = 4 * num_4x4_h;
    let use_hbd = is_cur_buf_hbd(xd);
    debug_assert!(use_hbd);
    let mut num_blk_skip_w = num_4x4_w;
    let mut sh_blk_skip = 0;
    if is_inter_mode != 0 {
        num_blk_skip_w = num_4x4_w >> 1;
        sh_blk_skip = 1;
    }

    {
        let Macroblock {
            plane: planes,
            e_mbd: xd,
            ..
        } = &mut *x;
        let p = &mut planes[0];
        let pd = &xd.plane[0];
        aom_highbd_subtract_block(
            bh,
            bw,
            &mut p.src_diff,
            0,
            bw as usize,
            src,
            p.src.buf,
            p.src.stride as usize,
            &xd.recon.buffer_alloc,
            pd.dst.buf,
            pd.dst.stride as usize,
        );
    }

    // Keep the intermediate value on the stack here. Writing directly to
    // skippable causes speed regression due to load-and-store issues in
    // update_yrd_loop_vars.
    let mut temp_skippable = 1;
    this_rdc.dist = 0;
    this_rdc.rate = 0;
    // For block sizes 8x16 or above, Hadamard txfm of two adjacent 8x8 blocks
    // can be done per function call. Hence the call of Hadamard txfm is
    // abstracted here for the specified cases.
    // (As of now, dual implementation of hadamard txfm is available for low
    // bitdepth: with the high bit depth buffers is_tx_8x8_dual_applicable is
    // 0.)

    let mut dqcoeff_buf = [0 as TranLowT; 16 * 16];
    let mut qcoeff_buf = [0 as TranLowT; 16 * 16];
    let mut coeff_buf = [0 as TranLowT; 16 * 16];
    let mut eob: u16 = 0;
    let scan_order = get_scan(tx_size, DCT_DCT);
    let diff_stride = bw;

    // Keep track of the row and column of the blocks we use so that we know
    // if we are in the unrestricted motion border.
    let mut r = 0;
    while r < max_blocks_high {
        let mut c = 0;
        let mut s = 0;
        while c < max_blocks_wide {
            let src_diff_off = ((r * diff_stride + c) << 2) as usize;
            let src_diff = &x.plane[0].src_diff;
            let p = &x.plane[0];
            match tx_size {
                TX_16X16 => {
                    aom_hadamard_16x16(
                        src_diff,
                        src_diff_off,
                        diff_stride as usize,
                        &mut coeff_buf,
                    );
                    av1_quantize_fp(
                        &coeff_buf,
                        16 * 16,
                        &p.round_fp_qtx,
                        &p.quant_fp_qtx,
                        &mut qcoeff_buf,
                        &mut dqcoeff_buf,
                        &p.dequant_qtx,
                        &mut eob,
                        // default_scan_fp_16x16_transpose and
                        // av1_default_iscan_fp_16x16_transpose have to be
                        // used together.
                        &DEFAULT_SCAN_FP_16X16_TRANSPOSE,
                    );
                }
                TX_8X8 => {
                    aom_hadamard_8x8(src_diff, src_diff_off, diff_stride as usize, &mut coeff_buf);
                    av1_quantize_fp(
                        &coeff_buf,
                        8 * 8,
                        &p.round_fp_qtx,
                        &p.quant_fp_qtx,
                        &mut qcoeff_buf,
                        &mut dqcoeff_buf,
                        &p.dequant_qtx,
                        &mut eob,
                        &DEFAULT_SCAN_8X8_TRANSPOSE,
                    );
                }
                _ => {
                    debug_assert!(tx_size == TX_4X4);
                    // In tx_size=4x4 case, aom_fdct4x4 and aom_fdct4x4_lp generate
                    // normal coefficients order, so we don't need to change the scan
                    // order here.
                    aom_fdct4x4(src_diff, src_diff_off, &mut coeff_buf, diff_stride as usize);
                    av1_quantize_fp(
                        &coeff_buf,
                        4 * 4,
                        &p.round_fp_qtx,
                        &p.quant_fp_qtx,
                        &mut qcoeff_buf,
                        &mut dqcoeff_buf,
                        &p.dequant_qtx,
                        &mut eob,
                        scan_order.scan,
                    );
                }
            }
            debug_assert!(eob <= 1024);
            update_yrd_loop_vars_hbd(
                x,
                &mut temp_skippable,
                step,
                eob as i32,
                &coeff_buf,
                &qcoeff_buf,
                &dqcoeff_buf,
                this_rdc,
                &mut eob_cost,
                (r * num_blk_skip_w + c) >> sh_blk_skip,
            );
            c += block_step;
            s += step;
        }
        let _ = s;
        block += row_step;
        r += block_step;
    }
    let _ = block;

    *skippable = temp_skippable;
    this_rdc.skip_txfm = temp_skippable as u8;
    if this_rdc.sse < i64::MAX {
        this_rdc.sse = (this_rdc.sse << 6) >> 2;
        if temp_skippable != 0 {
            this_rdc.dist = 0;
            this_rdc.dist = this_rdc.sse;
            return;
        }
    }

    // If skippable is set, rate gets clobbered later.
    this_rdc.rate <<= 2 + AV1_PROB_COST_SHIFT;
    this_rdc.rate += eob_cost << AV1_PROB_COST_SHIFT;
}

#[inline]
fn init_mbmi(
    mbmi: &mut MbModeInfo,
    pred_mode: PredictionMode,
    ref_frame0: MvReferenceFrame,
    ref_frame1: MvReferenceFrame,
    cm: &Av1Common,
) {
    let pmi = &mut mbmi.palette_mode_info;
    pmi.palette_size[0] = 0;
    pmi.palette_size[1] = 0;
    mbmi.ref_mv_idx = 0;
    mbmi.mode = pred_mode;
    mbmi.uv_mode = UV_DC_PRED;
    mbmi.ref_frame[0] = ref_frame0;
    mbmi.ref_frame[1] = ref_frame1;
    mbmi.filter_intra_mode_info.use_filter_intra = 0;
    mbmi.mv[0].as_int = 0;
    mbmi.mv[1].as_int = 0;
    mbmi.motion_mode = SIMPLE_TRANSLATION;
    mbmi.num_proj_ref = 1;
    mbmi.interintra_mode = 0;
    set_default_interp_filters(mbmi, cm.features.interp_filter);
}

/// Translation of `set_default_interp_filters()` (filter.h and
/// blockd.h): `av1_broadcast_interp_filter()` puts the filter in both
/// halves of `interp_filters`.
#[inline]
pub(crate) fn set_default_interp_filters(mbmi: &mut MbModeInfo, frame_interp_filter: i32) {
    let filter = if frame_interp_filter == SWITCHABLE {
        EIGHTTAP_REGULAR
    } else {
        frame_interp_filter
    };
    mbmi.interp_filters = (filter as u32 & 0xffff) | ((filter as u32 & 0xffff) << 16);
}

fn store_coding_context(x: &Macroblock, ctx: &mut PickModeContext) {
    let xd = &x.e_mbd;
    let txfm_info = &x.txfm_search_info;

    // Take a snapshot of the coding context so it can be
    // restored if we decide to encode this way
    ctx.rd_stats.skip_txfm = txfm_info.skip_txfm;
    ctx.skippable = txfm_info.skip_txfm as i32;
    ctx.mic = *xd.mbmi();
    ctx.skippable = txfm_info.skip_txfm as i32;
    av1_copy_mbmi_ext_to_mbmi_ext_frame(
        &mut ctx.mbmi_ext_best,
        &x.mbmi_ext,
        av1_ref_frame_type(&xd.mbmi().ref_frame) as u8,
    );
}

/// Translation of `struct estimate_block_intra_args`; `rdc` is the rate
/// and distortion accumulated here (upstream points it at the caller's).
struct EstimateBlockIntraArgs<'a> {
    cpi: &'a Av1Comp,
    x: &'a mut Macroblock,
    src: &'a [u16],
    mode: PredictionMode,
    skippable: i32,
    rdc: RdStats,
}

/*\brief Estimation of RD cost of an intra mode for Non-RD optimized case.
 *
 * Calculates RD Cost for an intra mode for a single TX block using Hadamard
 * transform.
 * \param[in]    plane          Color plane
 * \param[in]    block          Index of a TX block in a prediction block
 * \param[in]    row            Row of a current TX block
 * \param[in]    col            Column of a current TX block
 * \param[in]    plane_bsize    Block size of a current prediction block
 * \param[in]    tx_size        Transform size
 * \param[in]    arg            Pointer to a structure that holds parameters
 *                              for intra mode search
 *
 * \remark Nothing is returned. Instead, best mode and RD Cost of the best mode
 * are set in \c args->rdc and \c args->mode
 */
fn estimate_block_intra(
    plane: usize,
    block: i32,
    row: i32,
    col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    args: &mut EstimateBlockIntraArgs,
) {
    let cpi = args.cpi;
    let cm = &cpi.common;
    let x = &mut *args.x;
    let bsize_tx = TXSIZE_TO_BSIZE[tx_size as usize];
    let src_buf_base = x.plane[plane].src.buf;
    let dst_buf_base = x.e_mbd.plane[plane].dst.buf;
    let src_stride = x.plane[plane].src.stride as i64;
    let dst_stride = x.e_mbd.plane[plane].dst.stride as i64;
    let mut this_rdc = RdStats::default();

    let _ = block;
    let _ = plane_bsize;

    av1_predict_intra_block_facade(cm, &mut x.e_mbd, plane, col, row, tx_size);
    av1_invalid_rd_stats(&mut this_rdc);

    x.plane[plane].src.buf =
        (src_buf_base as i64 + 4 * (row as i64 * src_stride + col as i64)) as usize;
    x.e_mbd.plane[plane].dst.buf =
        (dst_buf_base as i64 + 4 * (row as i64 * dst_stride + col as i64)) as usize;

    if plane == 0 {
        // FIXME (upstream): each transform block's block_yrd() overwrites
        // args->skippable, so after the loop over the block's transform
        // blocks it only tells whether the last one is skippable.
        block_yrd(
            x,
            args.src,
            &mut this_rdc,
            &mut args.skippable,
            bsize_tx,
            tx_size.min(TX_16X16),
            0,
        );
    } else {
        // (model_rd_for_sb_uv(): the intra search only estimates luma)
        debug_assert!(plane == 0);
    }
    x.plane[plane].src.buf = src_buf_base;
    x.e_mbd.plane[plane].dst.buf = dst_buf_base;
    args.rdc.rate += this_rdc.rate;
    args.rdc.dist += this_rdc.dist;
}

pub(crate) fn av1_nonrd_pick_intra_mode(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    src: &[u16],
    rd_cost: &mut RdStats,
    bsize: BlockSize,
    ctx: &mut PickModeContext,
) {
    let cm = &cpi.common;
    let mut this_rdc = RdStats::default();
    let mut best_rdc = RdStats::default();
    let txfm_params = &x.txfm_search_params;
    let intra_tx_size = MAX_TXSIZE_LOOKUP[bsize as usize]
        .min(TX_MODE_TO_BIGGEST_TX_SIZE[txfm_params.tx_mode_search_type as usize]);
    let mut best_mode = DC_PRED;
    let xd = &x.e_mbd;
    let above_mi = xd.mi_at(xd.above_mbmi);
    let left_mi = xd.mi_at(xd.left_mbmi);
    let a = av1_above_block_mode(above_mi);
    let l = av1_left_block_mode(left_mi);
    let above_ctx = INTRA_MODE_CONTEXT[a as usize] as usize;
    let left_ctx = INTRA_MODE_CONTEXT[l as usize] as usize;
    let bmode_costs = x.mode_costs.y_mode_costs[above_ctx][left_ctx];

    av1_invalid_rd_stats(&mut best_rdc);
    av1_invalid_rd_stats(&mut this_rdc);

    {
        let mi = x.e_mbd.mbmi_mut();
        init_mbmi(mi, DC_PRED, INTRA_FRAME, NONE_FRAME, cm);
        mi.mv[0].as_int = INVALID_MV;
        mi.mv[1].as_int = INVALID_MV;
    }

    let mut args = EstimateBlockIntraArgs {
        cpi,
        x,
        src,
        mode: DC_PRED,
        skippable: 1,
        rdc: RdStats::default(),
    };

    // Change the limit of this loop to add other intra prediction
    // mode tests.
    for i in 0..4 {
        let this_mode = INTRA_MODE_LIST[i];

        // As per the statistics generated for intra mode evaluation in the nonrd
        // path, it is found that the probability of H_PRED mode being the winner is
        // very less when the best mode so far is V_PRED (out of DC_PRED and
        // V_PRED). If V_PRED is the winner mode out of DC_PRED and V_PRED, it could
        // imply the presence of a vertically dominant pattern. Hence, H_PRED mode
        // is not evaluated.
        if cpi.sf.rt_sf.prune_h_pred_using_best_mode_so_far
            && this_mode == H_PRED
            && best_mode == V_PRED
        {
            continue;
        }

        this_rdc.dist = 0;
        this_rdc.rate = 0;
        args.mode = this_mode;
        args.skippable = 1;
        // (args.rdc = &this_rdc)
        args.rdc = this_rdc;
        {
            let mi = args.x.e_mbd.mbmi_mut();
            mi.tx_size = intra_tx_size;
            mi.mode = this_mode;
        }
        av1_foreach_transformed_block_in_plane(
            &mut args,
            |a| &a.x.e_mbd,
            bsize,
            0,
            |a, plane, block, row, col, plane_bsize, tx_size| {
                estimate_block_intra(plane, block, row, col, plane_bsize, tx_size, a)
            },
        );
        this_rdc = args.rdc;
        let x = &*args.x;
        let skip_ctx = av1_get_skip_txfm_context(&x.e_mbd) as usize;
        if args.skippable != 0 {
            this_rdc.rate = x.mode_costs.skip_txfm_cost[skip_ctx][1];
        } else {
            this_rdc.rate += x.mode_costs.skip_txfm_cost[skip_ctx][0];
        }
        this_rdc.rate += bmode_costs[this_mode as usize];
        this_rdc.rdcost = rdcost(x.rdmult, this_rdc.rate, this_rdc.dist);

        if this_rdc.rdcost < best_rdc.rdcost {
            best_rdc = this_rdc;
            best_mode = this_mode;
            if this_rdc.skip_txfm == 0 {
                let n = ctx.num_4x4_blk as usize;
                ctx.blk_skip[..n].fill(0);
            }
        }
    }

    let x = args.x;
    {
        let mi = x.e_mbd.mbmi_mut();
        mi.mode = best_mode;
        // Keep DC for UV since mode test is based on Y channel only.
        mi.uv_mode = UV_DC_PRED;
    }
    *rd_cost = best_rdc;
    store_coding_context(x, ctx);
}
