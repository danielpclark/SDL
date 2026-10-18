// Rust translation of the parts of av1/encoder/nonrd_pickmode.c (and of
// av1/encoder/nonrd_opt.h, av1_block_error_c() from av1/encoder/rdopt.c
// and av1_copy_mbmi_ext_to_mbmi_ext_frame() from av1/encoder/rdopt.h) from
// aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the encoder reaches:
// the non-RD intra mode search of key frames and the non-RD inter mode
// search of the other frames (with find_predictors() and
// get_model_rd_flag() from av1/encoder/nonrd_opt.h,
// model_rd_with_curvfit() from av1/encoder/model_rd.h and
// av1_get_search_site_config() from
// av1/encoder/motion_search_facade.h).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Non-RD optimized mode search: each candidate mode's cost is estimated
//! from the variance of its prediction's residual or with Hadamard
//! transforms of the residual (intra chroma always uses DC in key
//! frames). Inter blocks search the motion vectors of the reference
//! frames' candidates and a new motion vector, the interpolation filters
//! and a few intra modes.

use super::av1_quantize::{av1_quantize_fp, av1_quantize_lp};
use super::block::*;
use super::context_tree::PickModeContext;
use super::cost::AV1_PROB_COST_SHIFT;
use super::encodemb::av1_foreach_transformed_block_in_plane;
use super::encoder::Av1Comp;
use super::encodetxb::block_offset;
use super::rd::{av1_invalid_rd_stats, rdcost};
use super::rdopt::av1_block_error_lp;
use crate::aom::aom_dsp::aom_dsp_common::{get_msb, TranLowT};
use crate::aom::aom_dsp::avg::{
    aom_hadamard_16x16, aom_hadamard_8x8, aom_hadamard_lp_16x16, aom_hadamard_lp_8x8,
    aom_hadamard_lp_8x8_dual, aom_satd, aom_satd_lp,
};
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
use super::encodemv::{av1_find_best_ref_mvs_from_stack, av1_get_ref_mv};
use super::encoder::{set_ref_ptrs, AOM_CBR, AOM_CONTENT_SCREEN, CYCLIC_REFRESH_AQ};
use super::mcomp::{
    av1_find_best_sub_pixel_tree_pruned_more, av1_full_pixel_search, av1_int_pro_motion_estimation,
    av1_make_default_fullpel_ms_params, av1_make_default_subpel_ms_params, av1_mv_bit_cost,
    cond_cost_list, find_fractional_mv_step, SearchSiteConfig, FULL_PEL, HALF_PEL,
    NUM_DISTINCT_SEARCH_METHODS, SS_CFG_LOOKAHEAD, SS_CFG_SRC,
};
use super::rd::{
    av1_get_intra_cost_penalty, av1_get_scaled_ref_frame, av1_get_switchable_rate,
    av1_model_rd_curvfit, av1_model_rd_from_var_lapndz, av1_mv_pred, av1_setup_pred_block,
    mode_offset, RD_THRESH_INC, RD_THRESH_MAX_FACT,
};
use super::reconinter_enc::{av1_enc_build_inter_predictor, av1_enc_build_inter_predictor_y_nonrd};
use super::speed_features::{EARLY_TERM_IDX_4, EARLY_TERM_INDICES};
use super::var_based_part::{
    av1_get_force_skip_low_temp_var, av1_get_force_skip_low_temp_var_small_sb,
};
use crate::aom::aom_dsp::variance::{aom_get_var_sse_sum_16x16_dual, aom_get_var_sse_sum_8x8_quad};
use crate::aom::av1::common::av1_common_int::{
    av1_num_planes, get_ref_frame_yv12_buf, get_ref_scale_factors_const, REFERENCE_MODE_SELECT,
};
use crate::aom::av1::common::convolve::get_conv_params;
use crate::aom::av1::common::filter::{
    av1_broadcast_interp_filter, av1_extract_interp_filter, interp_filters_of, InterpFilter,
    EIGHTTAP_SMOOTH, MULTITAP_SHARP, SWITCHABLE_FILTERS,
};
use crate::aom::av1::common::mv::{
    get_fullmv_from_mv, get_mv_from_fullmv, FullpelMv, IntMv, Mv, WarpedMotionParams, IDENTITY,
    INVALID_MV_ROW_COL,
};
use crate::aom::av1::common::mvref_common::{
    av1_drl_ctx, av1_find_mv_refs, av1_find_samples, av1_mode_context_analyzer,
};
use crate::aom::av1::common::pred_common::{
    av1_get_comp_reference_type_context, av1_get_intra_inter_context,
};
use crate::aom::av1::common::quant_common::QINDEX_BITS;
use crate::aom::av1::common::reconinter::{
    av1_count_overlappable_neighbors, init_inter_block_params, init_inter_mode_params,
    InterPredParams, SubpelParams,
};
use crate::aom::av1::common::reconintra::{
    av1_filter_intra_allowed_bsize, av1_is_directional_mode, av1_predict_intra_block,
};
use crate::aom::av1::common::scale::av1_is_scaled;
use crate::aom::av1::common::seg_common::{get_segdata, segfeature_active, SEG_LVL_REF_FRAME};

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

// The original scan order (default_scan_16x16) is modified according to the
// extra transpose in hadamard c implementation in lp case, i.e.,
// aom_hadamard_lp_16x16_c.
static DEFAULT_SCAN_LP_16X16_TRANSPOSE: [i16; 256] = [
    0, 8, 2, 4, 10, 16, 24, 18, 12, 6, 64, 14, 20, 26, 32, 40, 34, 28, 22, 72, 66, 68, 74, 80, 30,
    36, 42, 48, 56, 50, 44, 38, 88, 82, 76, 70, 128, 78, 84, 90, 96, 46, 52, 58, 1, 9, 3, 60, 54,
    104, 98, 92, 86, 136, 130, 132, 138, 144, 94, 100, 106, 112, 62, 5, 11, 17, 25, 19, 13, 7, 120,
    114, 108, 102, 152, 146, 140, 134, 192, 142, 148, 154, 160, 110, 116, 122, 65, 15, 21, 27, 33,
    41, 35, 29, 23, 73, 67, 124, 118, 168, 162, 156, 150, 200, 194, 196, 202, 208, 158, 164, 170,
    176, 126, 69, 75, 81, 31, 37, 43, 49, 57, 51, 45, 39, 89, 83, 77, 71, 184, 178, 172, 166, 216,
    210, 204, 198, 206, 212, 218, 224, 174, 180, 186, 129, 79, 85, 91, 97, 47, 53, 59, 61, 55, 105,
    99, 93, 87, 137, 131, 188, 182, 232, 226, 220, 214, 222, 228, 234, 240, 190, 133, 139, 145, 95,
    101, 107, 113, 63, 121, 115, 109, 103, 153, 147, 141, 135, 248, 242, 236, 230, 238, 244, 250,
    193, 143, 149, 155, 161, 111, 117, 123, 125, 119, 169, 163, 157, 151, 201, 195, 252, 246, 254,
    197, 203, 209, 159, 165, 171, 177, 127, 185, 179, 173, 167, 217, 211, 205, 199, 207, 213, 219,
    225, 175, 181, 187, 189, 183, 233, 227, 221, 215, 223, 229, 235, 241, 191, 249, 243, 237, 231,
    239, 245, 251, 253, 247, 255,
];

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

#[inline(always)]
fn update_yrd_loop_vars(
    x: &mut Macroblock,
    skippable: &mut i32,
    step: i32,
    ncoeffs: i32,
    low_coeff: &[i16],
    low_qcoeff: &[i16],
    low_dqcoeff: &[i16],
    this_rdc: &mut RdStats,
    eob_cost: &mut i32,
    tx_blk_id: i32,
) {
    let is_txfm_skip = (ncoeffs == 0) as i32;
    *skippable &= is_txfm_skip;
    x.txfm_search_info.blk_skip[tx_blk_id as usize] = is_txfm_skip as u8;
    *eob_cost += get_msb((ncoeffs + 1) as u32);
    if ncoeffs == 1 {
        this_rdc.rate += (low_qcoeff[0] as i32).abs();
    } else if ncoeffs > 1 {
        this_rdc.rate += aom_satd_lp(low_qcoeff, step << 4);
    }

    this_rdc.dist += av1_block_error_lp(low_coeff, low_dqcoeff, (step << 4) as usize) >> 2;
}

/// Translation of `aom_process_hadamard_lp_8x16()`: the coefficients go to
/// the macroblock's low precision coefficient buffer (upstream writes them
/// to `p->coeff` viewed as an `int16_t` array, which nothing reads as
/// `tran_low_t` before it is rewritten).
#[inline]
fn aom_process_hadamard_lp_8x16(
    x: &mut Macroblock,
    max_blocks_high: i32,
    max_blocks_wide: i32,
    num_4x4_w: i32,
    step: i32,
    block_step: i32,
) {
    let bw = 4 * num_4x4_w;
    let num_4x4 = num_4x4_w.min(max_blocks_wide);
    let mut block = 0;
    let Macroblock {
        plane: planes,
        coeff_lp,
        ..
    } = x;
    let p = &planes[0];

    let mut r = 0;
    while r < max_blocks_high {
        let mut c = 0;
        while c < num_4x4 {
            let src_diff = ((r * bw + c) << 2) as usize;
            let low_coeff = block_offset(block);
            aom_hadamard_lp_8x8_dual(
                &p.src_diff,
                src_diff,
                bw as usize,
                &mut coeff_lp[low_coeff..],
            );
            block += 2 * step;
            c += 2 * block_step;
        }
        r += block_step;
    }
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
// (`src` is the source frame's samples)
pub(crate) fn block_yrd(
    x: &mut Macroblock,
    src: &[u16],
    this_rdc: &mut RdStats,
    skippable: &mut i32,
    bsize: BlockSize,
    tx_size: TxSize,
    is_inter_mode: i32,
) -> Result<(), ()> {
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
        // (aom_subtract_block() computes what aom_highbd_subtract_block()
        // computes)
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
    let mut is_tx_8x8_dual_applicable = tx_size == TX_8X8
        && BLOCK_SIZE_WIDE[bsize as usize] >= 16
        && BLOCK_SIZE_HIGH[bsize as usize] >= 8;

    // As of now, dual implementation of hadamard txfm is available for low
    // bitdepth.
    if use_hbd {
        is_tx_8x8_dual_applicable = false;
    }

    if is_tx_8x8_dual_applicable {
        if x.coeff_lp.len() < MAX_SB_SQUARE_USIZE {
            x.coeff_lp = vec![0; MAX_SB_SQUARE_USIZE];
        }
        aom_process_hadamard_lp_8x16(
            x,
            max_blocks_high,
            max_blocks_wide,
            num_4x4_w,
            step,
            block_step,
        );
    }

    let mut dqcoeff_buf = [0 as TranLowT; 16 * 16];
    let mut qcoeff_buf = [0 as TranLowT; 16 * 16];
    let mut coeff_buf = [0 as TranLowT; 16 * 16];
    let mut low_coeff_buf = [0i16; 16 * 16];
    let mut low_qcoeff = [0i16; 16 * 16];
    let mut low_dqcoeff = [0i16; 16 * 16];
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
            // (low_coeff: the dual Hadamard transform's coefficients, or
            // low_coeff_buf)
            let mut low_coeff_dual: Option<usize> = None;
            match tx_size {
                TX_16X16 => {
                    if use_hbd {
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
                            0,
                        );
                    } else {
                        aom_hadamard_lp_16x16(
                            src_diff,
                            src_diff_off,
                            diff_stride as usize,
                            &mut low_coeff_buf,
                        );
                        av1_quantize_lp(
                            &low_coeff_buf,
                            16 * 16,
                            &p.round_fp_qtx,
                            &p.quant_fp_qtx,
                            &mut low_qcoeff,
                            &mut low_dqcoeff,
                            &p.dequant_qtx,
                            &mut eob,
                            // default_scan_lp_16x16_transpose and
                            // av1_default_iscan_lp_16x16_transpose have to be
                            // used together.
                            &DEFAULT_SCAN_LP_16X16_TRANSPOSE,
                        );
                    }
                }
                TX_8X8 => {
                    if use_hbd {
                        aom_hadamard_8x8(
                            src_diff,
                            src_diff_off,
                            diff_stride as usize,
                            &mut coeff_buf,
                        );
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
                            0,
                        );
                    } else {
                        let low_coeff: &[i16] = if is_tx_8x8_dual_applicable {
                            // The coeffs are pre-computed for the whole block, so re-assign
                            // low_coeff to the appropriate location.
                            let off = block_offset(block + s);
                            low_coeff_dual = Some(off);
                            &x.coeff_lp[off..]
                        } else {
                            aom_hadamard_lp_8x8(
                                src_diff,
                                src_diff_off,
                                diff_stride as usize,
                                &mut low_coeff_buf,
                            );
                            &low_coeff_buf
                        };
                        av1_quantize_lp(
                            low_coeff,
                            8 * 8,
                            &p.round_fp_qtx,
                            &p.quant_fp_qtx,
                            &mut low_qcoeff,
                            &mut low_dqcoeff,
                            &p.dequant_qtx,
                            &mut eob,
                            // default_scan_8x8_transpose and
                            // av1_default_iscan_8x8_transpose have to be used together.
                            &DEFAULT_SCAN_8X8_TRANSPOSE,
                        );
                    }
                }
                _ => {
                    debug_assert!(tx_size == TX_4X4);
                    // In tx_size=4x4 case, aom_fdct4x4 and aom_fdct4x4_lp generate
                    // normal coefficients order, so we don't need to change the scan
                    // order here.
                    if !use_hbd {
                        // (aom_fdct4x4_lp(): the 4x4 transforms are never
                        // reached, the variance partition stopping at 8x8)
                        return Err(());
                    }
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
                        0,
                    );
                }
            }
            debug_assert!(eob <= 1024);
            let tx_blk_id = (r * num_blk_skip_w + c) >> sh_blk_skip;
            if use_hbd {
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
                    tx_blk_id,
                );
            } else {
                let coeff_lp = std::mem::take(&mut x.coeff_lp);
                let low_coeff: &[i16] = match low_coeff_dual {
                    Some(off) => &coeff_lp[off..],
                    None => &low_coeff_buf,
                };
                update_yrd_loop_vars(
                    x,
                    &mut temp_skippable,
                    step,
                    eob as i32,
                    low_coeff,
                    &low_qcoeff,
                    &low_dqcoeff,
                    this_rdc,
                    &mut eob_cost,
                    tx_blk_id,
                );
                x.coeff_lp = coeff_lp;
            }
            c += block_step;
            s += step;
        }
        block += row_step;
        r += block_step;
    }

    *skippable = temp_skippable;
    this_rdc.skip_txfm = temp_skippable as u8;
    if this_rdc.sse < i64::MAX {
        this_rdc.sse = (this_rdc.sse << 6) >> 2;
        if temp_skippable != 0 {
            this_rdc.dist = 0;
            this_rdc.dist = this_rdc.sse;
            return Ok(());
        }
    }

    // If skippable is set, rate gets clobbered later.
    this_rdc.rate <<= 2 + AV1_PROB_COST_SHIFT;
    this_rdc.rate += eob_cost << AV1_PROB_COST_SHIFT;
    Ok(())
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
) -> Result<(), ()> {
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

    av1_predict_intra_block_facade(cm, &mut x.e_mbd, plane, col, row, tx_size)?;
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
        )?;
    } else {
        model_rd_for_sb_uv(cpi, bsize_tx, x, args.src, &mut this_rdc, plane, plane);
    }
    x.plane[plane].src.buf = src_buf_base;
    x.e_mbd.plane[plane].dst.buf = dst_buf_base;
    args.rdc.rate += this_rdc.rate;
    args.rdc.dist += this_rdc.dist;
    Ok(())
}

pub(crate) fn av1_nonrd_pick_intra_mode(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    src: &[u16],
    rd_cost: &mut RdStats,
    bsize: BlockSize,
    ctx: &mut PickModeContext,
) -> Result<(), ()> {
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
        )?;
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
    Ok(())
}

// (nonrd_pickmode.c: the inter frames' mode search)

/// `CALC_BIASED_RDCOST()`
#[inline]
fn calc_biased_rdcost(rdcost: i64) -> i64 {
    7 * rdcost >> 3
}

/// `MV_COST_WEIGHT` (rd.h)
const MV_COST_WEIGHT: i32 = 108;

// (PRED_BUFFER: the prediction buffers are only used to reuse the
// predictions, which reuse_inter_pred_nonrd never asks for: see
// av1_nonrd_pick_inter_mode_sb())

/// `BEST_PICKMODE` (without `best_pred`, a prediction buffer)
#[derive(Clone, Copy)]
struct BestPickmode {
    best_mode: PredictionMode,
    best_tx_size: TxSize,
    tx_type: TxType,
    best_ref_frame: MvReferenceFrame,
    best_second_ref_frame: MvReferenceFrame,
    best_mode_skip_txfm: u8,
    best_mode_initial_skip_flag: u8,
    best_pred_filter: u32,
    best_motion_mode: MotionMode,
    wm_params: WarpedMotionParams,
    num_proj_ref: u8,
    blk_skip: [u8; (MAX_MIB_SIZE * MAX_MIB_SIZE / 4) as usize],
    pmi: PaletteModeInfo,
    best_sse: i64,
}

/// `REF_MODE`
#[derive(Clone, Copy)]
struct RefMode {
    ref_frame: MvReferenceFrame,
    pred_mode: PredictionMode,
}

/// `COMP_REF_MODE`
#[derive(Clone, Copy)]
struct CompRefMode {
    ref_frame: [MvReferenceFrame; 2],
    pred_mode: PredictionMode,
}

/// `INTER_FILTER`
#[derive(Clone, Copy)]
struct InterFilter {
    filter_x: InterpFilter,
    filter_y: InterpFilter,
}

/*\brief Structure to store parameters and statistics used in non-rd inter mode
 * evaluation.
 */
/// `InterModeSearchStateNonrd`. Upstream's lives on the stack, unset but
/// for `use_ref_frame_mask`; the motion vectors and planes of the
/// references that are not searched are never read, and are zero here.
struct InterModeSearchStateNonrd {
    best_pickmode: BestPickmode,
    this_rdc: RdStats,
    best_rdc: RdStats,
    uv_dist: [[i64; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    yv12_mb: [[Buf2d; MAX_MB_PLANE]; REF_FRAMES as usize],
    vars: [[u32; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    ref_costs_single: [u32; REF_FRAMES as usize],
    frame_mv: [[IntMv; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    frame_mv_best: [[IntMv; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    single_inter_mode_costs: [[i32; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    use_ref_frame_mask: [i32; REF_FRAMES as usize],
    mode_checked: [[u8; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
}

const NUM_COMP_INTER_MODES_RT: usize = 6;
const NUM_INTER_MODES: usize = 12;

const fn rm(ref_frame: MvReferenceFrame, pred_mode: PredictionMode) -> RefMode {
    RefMode {
        ref_frame,
        pred_mode,
    }
}

// GLOBALMV in the set below is in fact ZEROMV as we don't do global ME in RT
// mode
static REF_MODE_SET: [RefMode; NUM_INTER_MODES] = [
    rm(LAST_FRAME, NEARESTMV),
    rm(LAST_FRAME, NEARMV),
    rm(LAST_FRAME, GLOBALMV),
    rm(LAST_FRAME, NEWMV),
    rm(GOLDEN_FRAME, NEARESTMV),
    rm(GOLDEN_FRAME, NEARMV),
    rm(GOLDEN_FRAME, GLOBALMV),
    rm(GOLDEN_FRAME, NEWMV),
    rm(ALTREF_FRAME, NEARESTMV),
    rm(ALTREF_FRAME, NEARMV),
    rm(ALTREF_FRAME, GLOBALMV),
    rm(ALTREF_FRAME, NEWMV),
];

const fn crm(r0: MvReferenceFrame, r1: MvReferenceFrame, pred_mode: PredictionMode) -> CompRefMode {
    CompRefMode {
        ref_frame: [r0, r1],
        pred_mode,
    }
}

static COMP_REF_MODE_SET: [CompRefMode; NUM_COMP_INTER_MODES_RT] = [
    crm(LAST_FRAME, GOLDEN_FRAME, GLOBAL_GLOBALMV),
    crm(LAST_FRAME, GOLDEN_FRAME, NEAREST_NEARESTMV),
    crm(LAST_FRAME, LAST2_FRAME, GLOBAL_GLOBALMV),
    crm(LAST_FRAME, LAST2_FRAME, NEAREST_NEARESTMV),
    crm(LAST_FRAME, ALTREF_FRAME, GLOBAL_GLOBALMV),
    crm(LAST_FRAME, ALTREF_FRAME, NEAREST_NEARESTMV),
];

const fn inf(filter_x: InterpFilter, filter_y: InterpFilter) -> InterFilter {
    InterFilter { filter_x, filter_y }
}

static FILTERS_REF_SET: [InterFilter; 9] = [
    inf(EIGHTTAP_REGULAR, EIGHTTAP_REGULAR),
    inf(EIGHTTAP_SMOOTH, EIGHTTAP_SMOOTH),
    inf(EIGHTTAP_REGULAR, EIGHTTAP_SMOOTH),
    inf(EIGHTTAP_SMOOTH, EIGHTTAP_REGULAR),
    inf(MULTITAP_SHARP, MULTITAP_SHARP),
    inf(EIGHTTAP_REGULAR, MULTITAP_SHARP),
    inf(MULTITAP_SHARP, EIGHTTAP_REGULAR),
    inf(EIGHTTAP_SMOOTH, MULTITAP_SHARP),
    inf(MULTITAP_SHARP, EIGHTTAP_SMOOTH),
];

/// `INTER_OFFSET()` (entropymode.h)
#[inline]
fn inter_offset(mode: PredictionMode) -> usize {
    (mode - NEARESTMV) as usize
}

/// `INTER_COMPOUND_OFFSET()` (entropymode.h)
#[inline]
fn inter_compound_offset(mode: PredictionMode) -> usize {
    (mode - NEAREST_NEARESTMV) as usize
}

/// Translation of `cyclic_refresh_segment_id_boosted()` (aq_cyclicrefresh.h).
#[inline]
fn cyclic_refresh_segment_id_boosted(segment_id: i32) -> bool {
    segment_id == 1 /* CR_SEGMENT_ID_BOOST1 */ || segment_id == 2 /* CR_SEGMENT_ID_BOOST2 */
}

/// Translation of `is_mode_intra()` (blockd.h).
#[inline]
fn is_mode_intra(mode: PredictionMode) -> bool {
    mode < INTRA_MODE_END
}

/// Translation of `rd_less_than_thresh()` (rd.h).
#[inline]
fn rd_less_than_thresh(best_rd: i64, thresh: i64, thresh_fact: i32) -> bool {
    best_rd < (thresh.wrapping_mul(thresh_fact as i64) >> 5) || thresh == i32::MAX as i64
}

/// Translation of `get_chessboard_index()` (encoder.h).
#[inline]
fn get_chessboard_index(frame_index: u32) -> i32 {
    (frame_index & 0x1) as i32
}

/// The cost of `mv` against `ref_mv` (`av1_mv_bit_cost()` with
/// `x->mv_costs`' tables and `MV_COST_WEIGHT`).
#[inline]
fn mv_bit_cost(x: &Macroblock, mv: &Mv, ref_mv: &Mv) -> Result<i32, ()> {
    let Some(c) = x.mv_costs.as_deref() else {
        return Err(());
    };
    Ok(av1_mv_bit_cost(
        mv,
        ref_mv,
        &c.nmv_joint_cost,
        &[c.mv_cost_stack(0), c.mv_cost_stack(1)],
        MV_COST_WEIGHT,
    ))
}

#[inline]
fn early_term_inter_search_with_sse(
    early_term_idx: i32,
    bsize: BlockSize,
    this_sse: i64,
    best_sse: i64,
    this_mode: PredictionMode,
) -> bool {
    // Aggressiveness to terminate inter mode search early is adjusted based on
    // speed and block size.
    static EARLY_TERM_THRESH: [[f64; 4]; 4] = [
        [0.65, 0.65, 0.65, 0.7],
        [0.6, 0.65, 0.85, 0.9],
        [0.5, 0.5, 0.55, 0.6],
        [0.6, 0.75, 0.85, 0.85],
    ];
    static EARLY_TERM_THRESH_NEWMV_NEARESTMV: [f64; 4] = [0.3, 0.3, 0.3, 0.3];

    let size_group = SIZE_GROUP_LOOKUP[bsize as usize] as usize;
    debug_assert!(size_group < 4);
    debug_assert!((early_term_idx > 0) && (early_term_idx < EARLY_TERM_INDICES));
    let threshold =
        if (early_term_idx == EARLY_TERM_IDX_4) && (this_mode == NEWMV || this_mode == NEARESTMV) {
            EARLY_TERM_THRESH_NEWMV_NEARESTMV[size_group]
        } else {
            EARLY_TERM_THRESH[(early_term_idx - 1) as usize][size_group]
        };

    // Terminate inter mode search early based on best sse so far.
    if (early_term_idx > 0) && (threshold * this_sse as f64 > best_sse as f64) {
        return true;
    }
    false
}

#[inline]
fn init_best_pickmode() -> BestPickmode {
    BestPickmode {
        best_sse: i64::MAX,
        best_mode: NEARESTMV,
        best_ref_frame: LAST_FRAME,
        best_second_ref_frame: NONE_FRAME,
        best_tx_size: TX_8X8,
        tx_type: DCT_DCT,
        best_pred_filter: av1_broadcast_interp_filter(EIGHTTAP_REGULAR),
        best_mode_skip_txfm: 0,
        best_mode_initial_skip_flag: 0,
        best_motion_mode: SIMPLE_TRANSLATION,
        num_proj_ref: 0,
        wm_params: WarpedMotionParams {
            wmmat: [0; 6],
            alpha: 0,
            beta: 0,
            gamma: 0,
            delta: 0,
            wmtype: IDENTITY,
            invalid: 0,
        },
        blk_skip: [0; (MAX_MIB_SIZE * MAX_MIB_SIZE / 4) as usize],
        pmi: PaletteModeInfo::default(),
    }
}

#[inline]
fn subpel_select(
    cpi: &Av1Comp,
    x: &Macroblock,
    bsize: BlockSize,
    mv: &IntMv,
    ref_mv: Mv,
    start_mv: FullpelMv,
    fullpel_performed_well: bool,
) -> i32 {
    let frame_lowmotion = cpi.rc.avg_frame_low_motion;
    let fullmv = mv.as_fullmv();
    // Reduce MV precision for higher int MV value & frame-level motion
    if cpi.sf.rt_sf.reduce_mv_pel_precision_highmotion >= 3 {
        let mut mv_thresh;
        let is_low_resoln = cpi.common.width * cpi.common.height <= 320 * 240;
        mv_thresh = if bsize > BLOCK_32X32 {
            2
        } else if bsize > BLOCK_16X16 {
            4
        } else {
            6
        };
        if frame_lowmotion > 0 && frame_lowmotion < 40 {
            mv_thresh = 12;
        }
        mv_thresh = if is_low_resoln {
            mv_thresh >> 1
        } else {
            mv_thresh
        };
        if (fullmv.row as i32).abs() >= mv_thresh || (fullmv.col as i32).abs() >= mv_thresh {
            return HALF_PEL;
        }
    } else if cpi.sf.rt_sf.reduce_mv_pel_precision_highmotion >= 1 {
        let th_vals: [[i32; 3]; 2] = [[4, 8, 10], [4, 6, 8]];
        let th_idx = (cpi.sf.rt_sf.reduce_mv_pel_precision_highmotion - 1) as usize;
        debug_assert!(th_idx < 2);
        let mv_thresh = if frame_lowmotion > 0 && frame_lowmotion < 40 {
            12
        } else if bsize >= BLOCK_32X32 {
            th_vals[th_idx][0]
        } else if bsize >= BLOCK_16X16 {
            th_vals[th_idx][1]
        } else {
            th_vals[th_idx][2]
        };
        if (fullmv.row as i32).abs() >= (mv_thresh << 1)
            || (fullmv.col as i32).abs() >= (mv_thresh << 1)
        {
            return FULL_PEL;
        } else if (fullmv.row as i32).abs() >= mv_thresh || (fullmv.col as i32).abs() >= mv_thresh {
            return HALF_PEL;
        }
    }
    // Reduce MV precision for relatively static (e.g. background), low-complex
    // large areas
    if cpi.sf.rt_sf.reduce_mv_pel_precision_lowcomplex >= 2 {
        let qband = x.qindex >> (QINDEX_BITS - 2);
        debug_assert!(qband < 4);
        if x.content_state_sb.source_sad_nonrd <= K_VERY_LOW_SAD
            && bsize > BLOCK_16X16
            && qband != 0
        {
            if x.source_variance < 500 {
                return FULL_PEL;
            } else if x.source_variance < 5000 {
                return HALF_PEL;
            }
        }
    } else if cpi.sf.rt_sf.reduce_mv_pel_precision_lowcomplex >= 1 {
        if fullpel_performed_well
            && ref_mv.row == 0
            && ref_mv.col == 0
            && start_mv.row == 0
            && start_mv.col == 0
        {
            return HALF_PEL;
        }
    }
    cpi.sf.mv_sf.subpel_force_stop
}

fn use_aggressive_subpel_search_method(
    x: &Macroblock,
    use_adaptive_subpel_search: bool,
    fullpel_performed_well: bool,
) -> bool {
    if !use_adaptive_subpel_search {
        return false;
    }
    let qband = x.qindex >> (QINDEX_BITS - 2);
    debug_assert!(qband < 4);
    if (qband > 0)
        && (fullpel_performed_well
            || (x.content_state_sb.source_sad_nonrd <= K_LOW_SAD)
            || (x.source_variance < 100))
    {
        return true;
    }
    false
}

/// Translation of `av1_get_search_site_config()` (motion_search_facade.h).
/// `Err` when neither of the compressor's caches has the reference's
/// stride (`x->search_site_cfg_buf`: the frames all have the same size).
#[inline]
fn av1_get_search_site_config<'a>(
    cpi: &'a Av1Comp,
    x: &Macroblock,
    search_method: i32,
) -> Result<&'a [SearchSiteConfig; NUM_DISTINCT_SEARCH_METHODS as usize], ()> {
    let ref_stride = x.e_mbd.plane[0].pre[0].stride;

    // AV1_COMP::mv_search_params.search_site_config is a compressor level cache
    // that's shared by multiple threads. In most cases where all frames have the
    // same resolution, the cache contains the search site config that we need.
    let mv_search_params = &cpi.mv_search_params;
    if ref_stride == mv_search_params.search_site_cfg[SS_CFG_SRC][0].stride {
        return Ok(&mv_search_params.search_site_cfg[SS_CFG_SRC]);
    } else if ref_stride == mv_search_params.search_site_cfg[SS_CFG_LOOKAHEAD][0].stride {
        return Ok(&mv_search_params.search_site_cfg[SS_CFG_LOOKAHEAD]);
    }

    // If the cache does not contain the correct stride, then we will need to rely
    // on the thread level config MACROBLOCK::search_site_cfg_buf. If even the
    // thread level config doesn't match, then we need to update it.
    let _ = search_method;
    Err(())
}

/*\brief Runs Motion Estimation for a specific block and specific ref frame.
 *
 * \ingroup nonrd_mode_search
 * \callgraph
 * \callergraph
 * Finds the best Motion Vector by running Motion Estimation for a specific
 * block and a specific reference frame. Exits early if RDCost of Full Pel part
 * exceeds best RD Cost fund so far
 * \param[in]    cpi                      Top-level encoder structure
 * \param[in]    x                        Pointer to structure holding all the
 *                                        data for the current macroblock
 * \param[in]    bsize                    Current block size
 * \param[in]    mi_row                   Row index in 4x4 units
 * \param[in]    mi_col                   Column index in 4x4 units
 * \param[in]    tmp_mv                   Pointer to best found New MV
 * \param[in]    rate_mv                  Pointer to Rate of the best new MV
 * \param[in]    best_rd_sofar            RD Cost of the best mode found so far
 * \param[in]    use_base_mv              Flag, indicating that tmp_mv holds
 *                                        specific MV to start the search with
 *
 * \return Returns 0 if ME was terminated after Full Pel Search because too
 * high RD Cost. Otherwise returns 1. Best New MV is placed into \c tmp_mv.
 * Rate estimation for this vector is placed to \c rate_mv
 */
fn combined_motion_search(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    tmp_mv: &mut IntMv,
    rate_mv: &mut i32,
    best_rd_sofar: i64,
    use_base_mv: bool,
) -> Result<bool, ()> {
    let cm = &cpi.common;
    let sf = &cpi.sf;
    let mi = *x.e_mbd.mbmi();
    let step_param = if sf.rt_sf.fullpel_search_step_param != 0 {
        sf.rt_sf.fullpel_search_step_param
    } else {
        cpi.mv_search_params.mv_step_param
    };
    let ref_ = mi.ref_frame[0];
    let ref_mv = av1_get_ref_mv(x, mi.ref_mv_idx as i32).as_mv();
    let mut dis = 0;
    let mut rv;
    let mut cost_list = [0i32; 5];
    let search_subpel = true;
    let scaled_ref_frame = av1_get_scaled_ref_frame(cpi, ref_);

    if scaled_ref_frame.is_some() {
        // Swap out the reference frame for a version that's been scaled to
        // match the resolution of the current frame, allowing the existing
        // motion search code to be used without additional modifications.
        // (the frames all have the same size)
        let _ = (mi_row, mi_col);
        return Err(());
    }

    let start_mv = get_fullmv_from_mv(&ref_mv);

    let center_mv = if !use_base_mv { ref_mv } else { tmp_mv.as_mv() };

    let search_method = sf.mv_sf.search_method;
    let src_search_sites = av1_get_search_site_config(cpi, x, search_method)?;
    let full_var_rd = {
        let full_ms_params = av1_make_default_fullpel_ms_params(
            cpi,
            x,
            bsize,
            &center_mv,
            src_search_sites,
            /*fine_search_interval=*/ 0,
        )?;

        let mut best = tmp_mv.as_fullmv();
        let r = av1_full_pixel_search(
            start_mv,
            &full_ms_params,
            step_param,
            if cond_cost_list(cpi) {
                Some(&mut cost_list)
            } else {
                None
            },
            &mut best,
            None,
        )? as u32;
        tmp_mv.set_fullmv(best);
        r
    };

    // calculate the bit cost on motion vector
    let mvp_full = get_mv_from_fullmv(&tmp_mv.as_fullmv());

    *rate_mv = mv_bit_cost(x, &mvp_full, &ref_mv)?;

    // TODO(kyslov) Account for Rate Mode!
    rv = !(rdcost(x.rdmult, *rate_mv, 0) > best_rd_sofar);

    if rv && search_subpel {
        let (best, sse) = {
            let mut ms_params =
                av1_make_default_subpel_ms_params(cpi, x, bsize, &ref_mv, &cost_list);
            let fullpel_performed_well = (bsize == BLOCK_64X64
                && full_var_rd.wrapping_mul(40) < 62267 * 7)
                || (bsize == BLOCK_32X32 && full_var_rd.wrapping_mul(8) < 42380)
                || (bsize == BLOCK_16X16 && full_var_rd.wrapping_mul(8) < 10127);
            if sf.rt_sf.reduce_mv_pel_precision_highmotion != 0
                || sf.rt_sf.reduce_mv_pel_precision_lowcomplex != 0
            {
                ms_params.forced_stop = subpel_select(
                    cpi,
                    x,
                    bsize,
                    tmp_mv,
                    ref_mv,
                    start_mv,
                    fullpel_performed_well,
                );
            }

            let subpel_start_mv = get_mv_from_fullmv(&tmp_mv.as_fullmv());
            let mut best = tmp_mv.as_mv();
            let mut sse = x.pred_sse[ref_ as usize];
            // adaptively downgrade subpel search method based on block properties
            if use_aggressive_subpel_search_method(
                x,
                sf.rt_sf.use_adaptive_subpel_search,
                fullpel_performed_well,
            ) {
                av1_find_best_sub_pixel_tree_pruned_more(
                    &x.e_mbd,
                    cm,
                    &ms_params,
                    subpel_start_mv,
                    &mut best,
                    &mut dis,
                    &mut sse,
                )?;
            } else {
                find_fractional_mv_step(
                    cpi.mv_search_params.find_fractional_mv_step,
                    &x.e_mbd,
                    cm,
                    &ms_params,
                    subpel_start_mv,
                    &mut best,
                    &mut dis,
                    &mut sse,
                )?;
            }
            (best, sse)
        };
        x.pred_sse[ref_ as usize] = sse;
        tmp_mv.set_mv(best);
        *rate_mv = mv_bit_cost(x, &tmp_mv.as_mv(), &ref_mv)?;
    }

    // The final MV can not be equal to the reference MV as this will trigger an
    // assert later. This can happen if both NEAREST and NEAR modes were skipped.
    let m = tmp_mv.as_mv();
    rv = m.col != ref_mv.col || m.row != ref_mv.row;
    Ok(rv)
}

/*\brief Searches for the best New Motion Vector.
 *
 * \ingroup nonrd_mode_search
 * \callgraph
 * \callergraph
 * Finds the best Motion Vector by doing Motion Estimation. Uses reduced
 * complexity ME for non-LAST frames or calls \c combined_motion_search
 * for LAST reference frame
 * \param[in]    cpi                      Top-level encoder structure
 * \param[in]    x                        Pointer to structure holding all the
 *                                        data for the current macroblock
 * \param[in]    frame_mv                 Array that holds MVs for all modes
 *                                        and ref frames
 * \param[in]    ref_frame                Reference frame for which to find
 *                                        the best New MVs
 * \param[in]    gf_temporal_ref          Flag, indicating temporal reference
 *                                        for GOLDEN frame
 * \param[in]    bsize                    Current block size
 * \param[in]    mi_row                   Row index in 4x4 units
 * \param[in]    mi_col                   Column index in 4x4 units
 * \param[in]    rate_mv                  Pointer to Rate of the best new MV
 * \param[in]    best_rdc                 Pointer to the RD Cost for the best
 *                                        mode found so far
 *
 * \return Returns -1 if the search was not done, otherwise returns 0.
 * Best New MV is placed into \c frame_mv array, Rate estimation for this
 * vector is placed to \c rate_mv
 */
// (`Ok(true)` for -1)
fn search_new_mv(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    frame_mv: &mut [[IntMv; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    ref_frame: MvReferenceFrame,
    gf_temporal_ref: bool,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    rate_mv: &mut i32,
    best_rdc: &RdStats,
) -> Result<bool, ()> {
    let cm = &cpi.common;
    let rf = ref_frame as usize;
    let newmv = NEWMV as usize;
    if ref_frame > LAST_FRAME && cpi.oxcf.rc_cfg.mode == AOM_CBR && gf_temporal_ref {
        let tmp_sad;
        let mut dis = 0;

        if bsize < BLOCK_16X16 {
            return Ok(true);
        }

        let r = x.mbmi_ext.ref_mv_stack[rf][0].this_mv.as_mv();
        tmp_sad = av1_int_pro_motion_estimation(cpi, x, bsize, mi_row, mi_col, &r)? as i32;

        if tmp_sad > x.pred_mv_sad[LAST_FRAME as usize] {
            return Ok(true);
        }

        let mi_mv = x.e_mbd.mbmi().mv[0];
        frame_mv[newmv][rf].as_int = mi_mv.as_int;
        let mut best_mv = mi_mv;
        let m = best_mv.as_mv();
        best_mv.set_mv(Mv {
            row: m.row >> 3,
            col: m.col >> 3,
        });
        let ref_mv = av1_get_ref_mv(x, 0).as_mv();
        let m = frame_mv[newmv][rf].as_mv();
        frame_mv[newmv][rf].set_mv(Mv {
            row: m.row >> 3,
            col: m.col >> 3,
        });

        let (best, sse) = {
            // (a NULL cost list)
            let mut ms_params = av1_make_default_subpel_ms_params(cpi, x, bsize, &ref_mv, &[0; 5]);
            ms_params.cost_list = None;
            if cpi.sf.rt_sf.reduce_mv_pel_precision_highmotion != 0
                || cpi.sf.rt_sf.reduce_mv_pel_precision_lowcomplex != 0
            {
                let start_mv = FullpelMv { row: 0, col: 0 };
                ms_params.forced_stop =
                    subpel_select(cpi, x, bsize, &best_mv, ref_mv, start_mv, false);
            }
            let start_mv = get_mv_from_fullmv(&best_mv.as_fullmv());
            let mut best = best_mv.as_mv();
            let mut sse = x.pred_sse[rf];
            find_fractional_mv_step(
                cpi.mv_search_params.find_fractional_mv_step,
                &x.e_mbd,
                cm,
                &ms_params,
                start_mv,
                &mut best,
                &mut dis,
                &mut sse,
            )?;
            (best, sse)
        };
        x.pred_sse[rf] = sse;
        best_mv.set_mv(best);
        frame_mv[newmv][rf].as_int = best_mv.as_int;

        // When NEWMV is same as ref_mv from the drl, it is preferred to code the
        // MV as NEARESTMV or NEARMV. In this case, NEWMV needs to be skipped to
        // avoid an assert failure at a later stage. The scenario can occur if
        // NEARESTMV was not evaluated for ALTREF.
        let m = frame_mv[newmv][rf].as_mv();
        if m.col == ref_mv.col && m.row == ref_mv.row {
            return Ok(true);
        }

        *rate_mv = mv_bit_cost(x, &m, &ref_mv)?;
    } else if !combined_motion_search(
        cpi,
        x,
        bsize,
        mi_row,
        mi_col,
        &mut frame_mv[newmv][rf],
        rate_mv,
        best_rdc.rdcost,
        false,
    )? {
        return Ok(true);
    }

    Ok(false)
}

fn estimate_single_ref_frame_costs(
    cm: &Av1Common,
    xd: &MacroBlockD,
    mode_costs: &ModeCosts,
    segment_id: u8,
    bsize: BlockSize,
    ref_costs_single: &mut [u32; REF_FRAMES as usize],
) {
    let seg_ref_active = segfeature_active(&cm.seg, segment_id, SEG_LVL_REF_FRAME);
    if seg_ref_active {
        ref_costs_single.fill(0);
    } else {
        let intra_inter_ctx = av1_get_intra_inter_context(xd) as usize;
        ref_costs_single[INTRA_FRAME as usize] =
            mode_costs.intra_inter_cost[intra_inter_ctx][0] as u32;
        let mut base_cost = mode_costs.intra_inter_cost[intra_inter_ctx][1] as u32;
        if cm.current_frame.reference_mode == REFERENCE_MODE_SELECT && is_comp_ref_allowed(bsize) {
            let comp_ref_type_ctx = av1_get_comp_reference_type_context(xd) as usize;
            base_cost =
                base_cost.wrapping_add(mode_costs.comp_ref_type_cost[comp_ref_type_ctx][1] as u32);
        }
        ref_costs_single[LAST_FRAME as usize] = base_cost;
        ref_costs_single[GOLDEN_FRAME as usize] = base_cost;
        ref_costs_single[ALTREF_FRAME as usize] = base_cost;
        // add cost for last, golden, altref
        let c = |v: i32| v as u32;
        let r = ref_costs_single;
        r[LAST_FRAME as usize] =
            r[LAST_FRAME as usize].wrapping_add(c(mode_costs.single_ref_cost[0][0][0]));
        r[GOLDEN_FRAME as usize] =
            r[GOLDEN_FRAME as usize].wrapping_add(c(mode_costs.single_ref_cost[0][0][1]));
        r[GOLDEN_FRAME as usize] =
            r[GOLDEN_FRAME as usize].wrapping_add(c(mode_costs.single_ref_cost[0][1][0]));
        r[ALTREF_FRAME as usize] =
            r[ALTREF_FRAME as usize].wrapping_add(c(mode_costs.single_ref_cost[0][0][1]));
        r[ALTREF_FRAME as usize] =
            r[ALTREF_FRAME as usize].wrapping_add(c(mode_costs.single_ref_cost[0][2][0]));
    }
}

#[inline]
fn set_force_skip_flag(cpi: &Av1Comp, x: &Macroblock, sse: u32, force_skip: &mut i32) {
    if x.txfm_search_params.tx_mode_search_type == TX_MODE_SELECT
        && cpi.sf.rt_sf.tx_size_level_based_on_qstep != 0
        && cpi.sf.rt_sf.tx_size_level_based_on_qstep >= 2
    {
        let qstep = (x.plane[0].dequant_qtx[1] as i32) >> (x.e_mbd.bd - 5);
        let qstep_sq = qstep.wrapping_mul(qstep) as u32;
        // If the sse is low for low source variance blocks, mark those as
        // transform skip.
        // Note: Though qstep_sq is based on ac qstep, the threshold is kept
        // low so that reliable early estimate of tx skip can be obtained
        // through its comparison with sse.
        if sse < qstep_sq
            && x.source_variance < qstep_sq
            && x.color_sensitivity[0] == 0
            && x.color_sensitivity[1] == 0
        {
            *force_skip = 1;
        }
    }
}

/// `CAP_TX_SIZE_FOR_BSIZE_GT32()`
#[inline]
fn cap_tx_size_for_bsize_gt32(tx_mode_search_type: TxMode, bsize: BlockSize) -> bool {
    tx_mode_search_type != ONLY_4X4 && bsize > BLOCK_32X32
}
/// `TX_SIZE_FOR_BSIZE_GT32`
const TX_SIZE_FOR_BSIZE_GT32: TxSize = TX_16X16;

fn calculate_tx_size(
    cpi: &Av1Comp,
    bsize: BlockSize,
    x: &Macroblock,
    var: u32,
    sse: u32,
    force_skip: &mut i32,
) -> TxSize {
    let xd = &x.e_mbd;
    let mut tx_size;
    let txfm_params = &x.txfm_search_params;
    if txfm_params.tx_mode_search_type == TX_MODE_SELECT {
        let mut multiplier: i32 = 8;
        let mut var_thresh: u32 = 0;
        let mut is_high_var = true;
        // Use quantizer based thresholds to determine transform size.
        if cpi.sf.rt_sf.tx_size_level_based_on_qstep != 0 {
            let qband = x.qindex >> (QINDEX_BITS - 2);
            let mult: [i32; 4] = [8, 7, 6, 5];
            debug_assert!(qband < 4);
            multiplier = mult[qband as usize];
            let qstep = (x.plane[0].dequant_qtx[1] as i32) >> (xd.bd - 5);
            let qstep_sq = qstep.wrapping_mul(qstep) as u32;
            var_thresh = qstep_sq.wrapping_mul(2);
            if cpi.sf.rt_sf.tx_size_level_based_on_qstep >= 2 {
                // If the sse is low for low source variance blocks, mark those as
                // transform skip.
                // Note: Though qstep_sq is based on ac qstep, the threshold is kept
                // low so that reliable early estimate of tx skip can be obtained
                // through its comparison with sse.
                if sse < qstep_sq
                    && x.source_variance < qstep_sq
                    && x.color_sensitivity[0] == 0
                    && x.color_sensitivity[1] == 0
                {
                    *force_skip = 1;
                }
                // Further lower transform size based on aq mode only if residual
                // variance is high.
                is_high_var = var >= var_thresh;
            }
        }
        // Choose larger transform size for blocks where dc component is dominant or
        // the ac component is low.
        if sse > (var.wrapping_mul(multiplier as u32) >> 2) || (var < var_thresh) {
            tx_size = MAX_TXSIZE_LOOKUP[bsize as usize]
                .min(TX_MODE_TO_BIGGEST_TX_SIZE[txfm_params.tx_mode_search_type as usize]);
        } else {
            tx_size = TX_8X8;
        }

        if cpi.oxcf.q_cfg.aq_mode == CYCLIC_REFRESH_AQ
            && cyclic_refresh_segment_id_boosted(xd.mbmi().segment_id as i32)
            && is_high_var
        {
            tx_size = TX_8X8;
        } else if tx_size > TX_16X16 {
            tx_size = TX_16X16;
        }
    } else {
        tx_size = MAX_TXSIZE_LOOKUP[bsize as usize]
            .min(TX_MODE_TO_BIGGEST_TX_SIZE[txfm_params.tx_mode_search_type as usize]);
    }

    if cap_tx_size_for_bsize_gt32(txfm_params.tx_mode_search_type, bsize) {
        tx_size = TX_SIZE_FOR_BSIZE_GT32;
    }

    tx_size.min(TX_16X16)
}

static B_WIDTH_LOG2_LOOKUP: [u8; BLOCK_SIZES as usize] =
    [0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5];
static B_HEIGHT_LOG2_LOOKUP: [u8; BLOCK_SIZES as usize] =
    [0, 1, 0, 1, 2, 1, 2, 3, 2, 3, 4, 3, 4, 5, 4, 5];

fn block_variance(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    ref_: &[u16],
    ref_off: usize,
    ref_stride: usize,
    w: i32,
    h: i32,
    sse: &mut u32,
    sum: &mut i32,
    block_size: i32,
    sse8x8: &mut [u32],
    sum8x8: &mut [i32],
    var8x8: &mut [u32],
) {
    let mut k = 0;
    *sse = 0;
    *sum = 0;

    // This function is called for block sizes >= BLOCK_32x32. As per the design
    // the aom_get_var_sse_sum_8x8_quad() processes four 8x8 blocks (in a 8x32)
    // per call. Hence the width and height of the block need to be at least 8 and
    // 32 samples respectively.
    debug_assert!(w >= 32);
    debug_assert!(h >= 8);
    let mut i = 0;
    while i < h {
        let mut j = 0;
        while j < w {
            aom_get_var_sse_sum_8x8_quad(
                src,
                src_off + src_stride * i as usize + j as usize,
                src_stride,
                ref_,
                ref_off + ref_stride * i as usize + j as usize,
                ref_stride,
                &mut sse8x8[k..],
                &mut sum8x8[k..],
                sse,
                sum,
                &mut var8x8[k..],
            );
            k += 4;
            j += 32;
        }
        i += block_size;
    }
}

fn block_variance_16x16_dual(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    ref_: &[u16],
    ref_off: usize,
    ref_stride: usize,
    w: i32,
    h: i32,
    sse: &mut u32,
    sum: &mut i32,
    block_size: i32,
    sse16x16: &mut [u32],
    var16x16: &mut [u32],
) {
    let mut k = 0;
    *sse = 0;
    *sum = 0;
    // This function is called for block sizes >= BLOCK_32x32. As per the design
    // the aom_get_var_sse_sum_16x16_dual() processes four 16x16 blocks (in a
    // 16x32) per call. Hence the width and height of the block need to be at
    // least 16 and 32 samples respectively.
    debug_assert!(w >= 32);
    debug_assert!(h >= 16);
    let mut i = 0;
    while i < h {
        let mut j = 0;
        while j < w {
            aom_get_var_sse_sum_16x16_dual(
                src,
                src_off + src_stride * i as usize + j as usize,
                src_stride,
                ref_,
                ref_off + ref_stride * i as usize + j as usize,
                ref_stride,
                &mut sse16x16[k..],
                sse,
                sum,
                &mut var16x16[k..],
            );
            k += 2;
            j += 32;
        }
        i += block_size;
    }
}

fn calculate_variance(
    bw: i32,
    bh: i32,
    tx_size: TxSize,
    sse_i: &[u32],
    sum_i: &[i32],
    var_o: &mut [u32],
    sse_o: &mut [u32],
    sum_o: &mut [i32],
) {
    let unit_size = TXSIZE_TO_BSIZE[tx_size as usize] as usize;
    let nw = 1 << (bw - B_WIDTH_LOG2_LOOKUP[unit_size] as i32);
    let nh = 1 << (bh - B_HEIGHT_LOG2_LOOKUP[unit_size] as i32);
    let mut k = 0;

    let mut i = 0;
    while i < nh {
        let mut j = 0;
        while j < nw {
            let a = (i * nw + j) as usize;
            let b = ((i + 1) * nw + j) as usize;
            sse_o[k] = sse_i[a]
                .wrapping_add(sse_i[a + 1])
                .wrapping_add(sse_i[b])
                .wrapping_add(sse_i[b + 1]);
            sum_o[k] = sum_i[a]
                .wrapping_add(sum_i[a + 1])
                .wrapping_add(sum_i[b])
                .wrapping_add(sum_i[b + 1]);
            var_o[k] = sse_o[k].wrapping_sub(
                (((sum_o[k] as i64) * sum_o[k] as i64)
                    >> (B_WIDTH_LOG2_LOOKUP[unit_size] as i32
                        + B_HEIGHT_LOG2_LOOKUP[unit_size] as i32
                        + 6)) as u32,
            );
            k += 1;
            j += 2;
        }
        i += 2;
    }
}

// Adjust the ac_thr according to speed, width, height and normalized sum
fn ac_thr_factor(speed: i32, width: i32, height: i32, norm_sum: i32) -> i32 {
    if speed >= 8 && norm_sum < 5 {
        if width <= 640 && height <= 480 {
            return 4;
        } else {
            return 2;
        }
    }
    1
}

// Sets early_term flag based on chroma planes prediction
#[inline]
fn set_early_term_based_on_uv_plane(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    early_term: &mut i32,
    num_blk: u32,
    sse_tx: &[u32],
    var_tx: &[u32],
    sum: i32,
    var: u32,
    sse: u32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let p = &x.plane[0];
    let dc_quant = p.dequant_qtx[0] as i32 as u32;
    let ac_quant = p.dequant_qtx[1] as i32 as u32;
    let dc_thr = (dc_quant.wrapping_mul(dc_quant) >> 6) as i64;
    let mut ac_thr = (ac_quant.wrapping_mul(ac_quant) >> 6) as i64;
    let bw = B_WIDTH_LOG2_LOOKUP[bsize as usize] as i32;
    let bh = B_HEIGHT_LOG2_LOOKUP[bsize as usize] as i32;
    let mut ac_test = 1;
    let mut dc_test = 1;
    let norm_sum = sum.wrapping_abs() >> (bw + bh);

    // (CONFIG_AV1_TEMPORAL_DENOISING is off)
    ac_thr *= ac_thr_factor(cpi.oxcf.speed, cm.width, cm.height, norm_sum) as i64;

    for k in 0..num_blk as usize {
        // Check if all ac coefficients can be quantized to zero.
        if !((var_tx[k] as i64) < ac_thr || var == 0) {
            ac_test = 0;
            break;
        }
        // Check if dc coefficient can be quantized to zero.
        if !((sse_tx[k].wrapping_sub(var_tx[k]) as i64) < dc_thr || sse == var) {
            dc_test = 0;
            break;
        }
    }

    // Check if chroma can be skipped based on ac and dc test flags.
    if ac_test != 0 && dc_test != 0 {
        let mut skip_uv = [0i32; 2];
        let mut var_uv = [0u32; 2];
        let mut sse_uv = [0u32; 2];
        // Transform skipping test in UV planes.
        for i in 1..=2usize {
            let j = i - 1;
            skip_uv[j] = 1;
            if x.color_sensitivity[j] != 0 {
                skip_uv[j] = 0;
                let (ssx, ssy) = (
                    x.e_mbd.plane[i].subsampling_x,
                    x.e_mbd.plane[i].subsampling_y,
                );
                let uv_bsize = get_plane_block_size(bsize, ssx, ssy);
                // Adjust these thresholds for UV.
                let dq = x.plane[i].dequant_qtx;
                let uv_dc_thr = (((dq[0] as i32) * (dq[0] as i32)) >> 3) as i64;
                let uv_ac_thr = (((dq[1] as i32) * (dq[1] as i32)) >> 3) as i64;
                av1_enc_build_inter_predictor(cm, &mut x.e_mbd, mi_row, mi_col, None, bsize, i, i)?;
                let puv = &x.plane[i];
                let puvd = &x.e_mbd.plane[i];
                var_uv[j] = cpi.ppi.fn_ptr.vf(
                    uv_bsize,
                    &x.source_alloc,
                    puv.src.buf,
                    puv.src.stride as usize,
                    &x.e_mbd.recon.buffer_alloc,
                    puvd.dst.buf,
                    puvd.dst.stride as usize,
                    &mut sse_uv[j],
                );
                if ((var_uv[j] as i64) < uv_ac_thr || var_uv[j] == 0)
                    && ((sse_uv[j].wrapping_sub(var_uv[j]) as i64) < uv_dc_thr
                        || sse_uv[j] == var_uv[j])
                {
                    skip_uv[j] = 1;
                } else {
                    break;
                }
            }
        }
        if (skip_uv[0] & skip_uv[1]) != 0 {
            *early_term = 1;
        }
    }
    Ok(())
}

#[inline]
fn calc_rate_dist_block_param(
    cpi: &Av1Comp,
    x: &Macroblock,
    rd_stats: &mut RdStats,
    calculate_rd: i32,
    early_term: &i32,
    bsize: BlockSize,
    sse: u32,
) {
    if calculate_rd != 0 {
        if *early_term == 0 {
            let bw = BLOCK_SIZE_WIDE[bsize as usize] as i32;
            let bh = BLOCK_SIZE_HIGH[bsize as usize] as i32;

            let s = rd_stats.sse;
            model_rd_with_curvfit(
                cpi,
                x,
                bsize,
                AOM_PLANE_Y,
                s,
                bw * bh,
                &mut rd_stats.rate,
                &mut rd_stats.dist,
            );
        }

        if *early_term != 0 {
            rd_stats.rate = 0;
            // (sse << 4 in 32 bits)
            rd_stats.dist = (sse << 4) as i64;
        }
    }
}

fn model_skip_for_sb_y_large_64(
    cpi: &Av1Comp,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    early_term: &mut i32,
    calculate_rd: i32,
    best_sse: i64,
    var_output: Option<&mut u32>,
    var_prune_threshold: u32,
) -> Result<(), ()> {
    // Note our transform coeffs are 8 times an orthogonal transform.
    // Hence quantizer step is also 8 times. To get effective quantizer
    // we need to divide by 8 before sending to modeling function.
    let mut sse = 0u32;
    let mut test_skip = 1;
    let var;
    let mut sum = 0i32;
    let bw = B_WIDTH_LOG2_LOOKUP[bsize as usize] as i32;
    let bh = B_HEIGHT_LOG2_LOOKUP[bsize as usize] as i32;
    let mut sse16x16 = [0u32; 64];
    let mut var16x16 = [0u32; 64];
    debug_assert!(x.e_mbd.mbmi().tx_size == TX_16X16);
    debug_assert!(bsize > BLOCK_32X32);

    // Calculate variance for whole partition, and also save 16x16 blocks'
    // variance to be used in following transform skipping test.
    {
        let p = &x.plane[0];
        let pd = &x.e_mbd.plane[0];
        block_variance_16x16_dual(
            &x.source_alloc,
            p.src.buf,
            p.src.stride as usize,
            &x.e_mbd.recon.buffer_alloc,
            pd.dst.buf,
            pd.dst.stride as usize,
            4 << bw,
            4 << bh,
            &mut sse,
            &mut sum,
            16,
            &mut sse16x16,
            &mut var16x16,
        );
    }

    var = sse.wrapping_sub((((sum as i64) * sum as i64) >> (bw + bh + 4)) as u32);
    if let Some(v) = var_output {
        *v = var;
        if *v > var_prune_threshold {
            return Ok(());
        }
    }

    rd_stats.sse = sse as i64;
    // Skipping test
    *early_term = 0;
    set_force_skip_flag(cpi, x, sse, early_term);
    // The code below for setting skip flag assumes transform size of at least
    // 8x8, so force this lower limit on transform.
    let mode = x.e_mbd.mbmi().mode;
    if calculate_rd == 0
        && cpi.sf.rt_sf.sse_early_term_inter_search != 0
        && early_term_inter_search_with_sse(
            cpi.sf.rt_sf.sse_early_term_inter_search,
            bsize,
            sse as i64,
            best_sse,
            mode,
        )
    {
        test_skip = 0;
    }

    if *early_term != 0 {
        test_skip = 0;
    }

    // Evaluate if the partition block is a skippable block in Y plane.
    if test_skip != 0 {
        let num_block = (1u32 << (bw + bh - 2)) >> 2;
        set_early_term_based_on_uv_plane(
            cpi, x, bsize, mi_row, mi_col, early_term, num_block, &sse16x16, &var16x16, sum, var,
            sse,
        )?;
    }
    calc_rate_dist_block_param(cpi, x, rd_stats, calculate_rd, early_term, bsize, sse);
    Ok(())
}

fn model_skip_for_sb_y_large(
    cpi: &Av1Comp,
    bsize: BlockSize,
    mi_row: i32,
    mi_col: i32,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    early_term: &mut i32,
    calculate_rd: i32,
    best_sse: i64,
    var_output: Option<&mut u32>,
    var_prune_threshold: u32,
) -> Result<(), ()> {
    if x.force_zeromv_skip_for_blk != 0 {
        *early_term = 1;
        rd_stats.rate = 0;
        rd_stats.dist = 0;
        rd_stats.sse = 0;
        return Ok(());
    }

    // For block sizes greater than 32x32, the transform size is always 16x16.
    // This function avoids calling calculate_variance() for tx_size 16x16 cases
    // by directly populating variance at tx_size level from
    // block_variance_16x16_dual() function.
    if cap_tx_size_for_bsize_gt32(x.txfm_search_params.tx_mode_search_type, bsize) {
        x.e_mbd.mbmi_mut().tx_size = TX_SIZE_FOR_BSIZE_GT32;
        return model_skip_for_sb_y_large_64(
            cpi,
            bsize,
            mi_row,
            mi_col,
            x,
            rd_stats,
            early_term,
            calculate_rd,
            best_sse,
            var_output,
            var_prune_threshold,
        );
    }

    // Note our transform coeffs are 8 times an orthogonal transform.
    // Hence quantizer step is also 8 times. To get effective quantizer
    // we need to divide by 8 before sending to modeling function.
    let mut sse = 0u32;
    let mut test_skip = 1;
    let var;
    let mut sum = 0i32;

    let bw = B_WIDTH_LOG2_LOOKUP[bsize as usize] as i32;
    let bh = B_HEIGHT_LOG2_LOOKUP[bsize as usize] as i32;
    let mut sse8x8 = [0u32; 256];
    let mut sum8x8 = [0i32; 256];
    let mut var8x8 = [0u32; 256];
    let mut tx_size;

    // Calculate variance for whole partition, and also save 8x8 blocks' variance
    // to be used in following transform skipping test.
    {
        let p = &x.plane[0];
        let pd = &x.e_mbd.plane[0];
        block_variance(
            &x.source_alloc,
            p.src.buf,
            p.src.stride as usize,
            &x.e_mbd.recon.buffer_alloc,
            pd.dst.buf,
            pd.dst.stride as usize,
            4 << bw,
            4 << bh,
            &mut sse,
            &mut sum,
            8,
            &mut sse8x8,
            &mut sum8x8,
            &mut var8x8,
        );
    }
    var = sse.wrapping_sub((((sum as i64) * sum as i64) >> (bw + bh + 4)) as u32);
    if let Some(v) = var_output {
        *v = var;
        if *v > var_prune_threshold {
            return Ok(());
        }
    }

    rd_stats.sse = sse as i64;
    // Skipping test
    *early_term = 0;
    tx_size = calculate_tx_size(cpi, bsize, x, var, sse, early_term);
    debug_assert!(tx_size <= TX_16X16);
    // The code below for setting skip flag assumes transform size of at least
    // 8x8, so force this lower limit on transform.
    if tx_size < TX_8X8 {
        tx_size = TX_8X8;
    }
    x.e_mbd.mbmi_mut().tx_size = tx_size;

    let mode = x.e_mbd.mbmi().mode;
    if calculate_rd == 0
        && cpi.sf.rt_sf.sse_early_term_inter_search != 0
        && early_term_inter_search_with_sse(
            cpi.sf.rt_sf.sse_early_term_inter_search,
            bsize,
            sse as i64,
            best_sse,
            mode,
        )
    {
        test_skip = 0;
    }

    if *early_term != 0 {
        test_skip = 0;
    }

    // Evaluate if the partition block is a skippable block in Y plane.
    if test_skip != 0 {
        let mut sse16x16 = [0u32; 64];
        let mut sum16x16 = [0i32; 64];
        let mut var16x16 = [0u32; 64];
        let mut num_blks = 1u32 << (bw + bh - 2);

        if tx_size >= TX_16X16 {
            calculate_variance(
                bw,
                bh,
                TX_8X8,
                &sse8x8,
                &sum8x8,
                &mut var16x16,
                &mut sse16x16,
                &mut sum16x16,
            );
            num_blks >>= 2;
            set_early_term_based_on_uv_plane(
                cpi, x, bsize, mi_row, mi_col, early_term, num_blks, &sse16x16, &var16x16, sum,
                var, sse,
            )?;
        } else {
            set_early_term_based_on_uv_plane(
                cpi, x, bsize, mi_row, mi_col, early_term, num_blks, &sse8x8, &var8x8, sum, var,
                sse,
            )?;
        }
    }
    calc_rate_dist_block_param(cpi, x, rd_stats, calculate_rd, early_term, bsize, sse);
    Ok(())
}

fn model_rd_for_sb_y(
    cpi: &Av1Comp,
    bsize: BlockSize,
    x: &mut Macroblock,
    rd_stats: &mut RdStats,
    var_out: Option<&mut u32>,
    calculate_rd: i32,
    early_term: Option<&mut i32>,
) {
    if x.force_zeromv_skip_for_blk != 0 {
        if let Some(e) = early_term {
            *e = 1;
            rd_stats.rate = 0;
            rd_stats.dist = 0;
            rd_stats.sse = 0;
        }
    }

    // Note our transform coeffs are 8 times an orthogonal transform.
    // Hence quantizer step is also 8 times. To get effective quantizer
    // we need to divide by 8 before sending to modeling function.
    let ref_ = x.e_mbd.mbmi().ref_frame[0];

    debug_assert!(bsize < BLOCK_SIZES_ALL);

    let mut sse = 0u32;
    let mut rate = 0i32;
    let mut dist = 0i64;

    let var = {
        let p = &x.plane[0];
        let pd = &x.e_mbd.plane[0];
        cpi.ppi.fn_ptr.vf(
            bsize,
            &x.source_alloc,
            p.src.buf,
            p.src.stride as usize,
            &x.e_mbd.recon.buffer_alloc,
            pd.dst.buf,
            pd.dst.stride as usize,
            &mut sse,
        )
    };
    let mut force_skip = 0;
    let tx_size = calculate_tx_size(cpi, bsize, x, var, sse, &mut force_skip);
    x.e_mbd.mbmi_mut().tx_size = tx_size;
    if let Some(v) = var_out {
        *v = var;
    }

    if calculate_rd != 0 && (force_skip == 0 || ref_ == INTRA_FRAME) {
        let bwide = BLOCK_SIZE_WIDE[bsize as usize] as i32;
        let bhigh = BLOCK_SIZE_HIGH[bsize as usize] as i32;
        model_rd_with_curvfit(
            cpi,
            x,
            bsize,
            AOM_PLANE_Y,
            sse as i64,
            bwide * bhigh,
            &mut rate,
            &mut dist,
        );
    } else {
        rate = i32::MAX; // this will be overwritten later with block_yrd
        dist = i32::MAX as i64;
    }
    rd_stats.sse = sse as i64;
    x.pred_sse[ref_ as usize] = sse;

    if force_skip != 0 && ref_ > INTRA_FRAME {
        rate = 0;
        dist = (sse as i64) << 4;
    }

    debug_assert!(rate >= 0);

    rd_stats.skip_txfm = (rate == 0) as u8;
    rd_stats.rate = rate;
    rd_stats.dist = dist;
}

/// `block_yrd()` on the source samples (see `with_source()`).
#[inline]
fn block_yrd_src(
    x: &mut Macroblock,
    this_rdc: &mut RdStats,
    skippable: &mut i32,
    bsize: BlockSize,
    tx_size: TxSize,
    is_inter_mode: i32,
) -> Result<(), ()> {
    with_source(x, |x, src| {
        block_yrd(x, src, this_rdc, skippable, bsize, tx_size, is_inter_mode)
    })
}

#[inline]
fn get_drl_cost(
    this_mode: PredictionMode,
    ref_mv_idx: i32,
    mbmi_ext: &MbModeInfoExt,
    drl_mode_cost0: &[[i32; 2]],
    ref_frame_type: i8,
) -> i32 {
    let mut cost = 0;
    let rft = ref_frame_type as usize;
    if this_mode == NEWMV || this_mode == NEW_NEWMV {
        for idx in 0..2 {
            if mbmi_ext.ref_mv_count[rft] as i32 > idx + 1 {
                let drl_ctx = av1_drl_ctx(&mbmi_ext.weight[rft], idx as usize) as usize;
                cost += drl_mode_cost0[drl_ctx][(ref_mv_idx != idx) as usize];
                if ref_mv_idx == idx {
                    return cost;
                }
            }
        }
        return cost;
    }

    if have_nearmv_in_inter_mode(this_mode) {
        for idx in 1..3 {
            if mbmi_ext.ref_mv_count[rft] as i32 > idx + 1 {
                let drl_ctx = av1_drl_ctx(&mbmi_ext.weight[rft], idx as usize) as usize;
                cost += drl_mode_cost0[drl_ctx][(ref_mv_idx != (idx - 1)) as usize];
                if ref_mv_idx == (idx - 1) {
                    return cost;
                }
            }
        }
        return cost;
    }
    cost
}

fn cost_mv_ref(mode_costs: &ModeCosts, mode: PredictionMode, mode_context: i16) -> i32 {
    if is_inter_compound_mode(mode) {
        return mode_costs.inter_compound_mode_cost[mode_context as usize]
            [inter_compound_offset(mode)];
    }

    let mode_cost;
    let mut mode_ctx = (mode_context & NEWMV_CTX_MASK as i16) as usize;

    debug_assert!(is_inter_mode(mode));

    if mode == NEWMV {
        mode_cost = mode_costs.newmv_mode_cost[mode_ctx][0];
        mode_cost
    } else {
        let mut mode_cost = mode_costs.newmv_mode_cost[mode_ctx][1];
        mode_ctx = ((mode_context >> GLOBALMV_OFFSET) & GLOBALMV_CTX_MASK as i16) as usize;

        if mode == GLOBALMV {
            mode_cost += mode_costs.zeromv_mode_cost[mode_ctx][0];
            mode_cost
        } else {
            mode_cost += mode_costs.zeromv_mode_cost[mode_ctx][1];
            mode_ctx = ((mode_context >> REFMV_OFFSET) & REFMV_CTX_MASK as i16) as usize;
            mode_cost += mode_costs.refmv_mode_cost[mode_ctx][(mode != NEARESTMV) as usize];
            mode_cost
        }
    }
}

fn newmv_diff_bias(
    xd: &MacroBlockD,
    this_mode: PredictionMode,
    this_rdc: &mut RdStats,
    bsize: BlockSize,
    mv_row: i32,
    mv_col: i32,
    speed: i32,
    spatial_variance: u32,
    content_state_sb: ContentStateSb,
) {
    // Bias against MVs associated with NEWMV mode that are very different from
    // top/left neighbors.
    if this_mode == NEWMV {
        let al_mv_average_row;
        let al_mv_average_col;
        let row_diff;
        let col_diff;
        let mut above_mv_valid = false;
        let mut left_mv_valid = false;
        let mut above_row = INVALID_MV_ROW_COL;
        let mut above_col = INVALID_MV_ROW_COL;
        let mut left_row = INVALID_MV_ROW_COL;
        let mut left_col = INVALID_MV_ROW_COL;
        if bsize >= BLOCK_64X64
            && content_state_sb.source_sad_nonrd != K_HIGH_SAD
            && spatial_variance < 300
            && (mv_row > 16 || mv_row < -16 || mv_col > 16 || mv_col < -16)
        {
            this_rdc.rdcost <<= 2;
            return;
        }
        if let Some(a) = xd.mi_at(xd.above_mbmi) {
            above_mv_valid = a.mv[0].as_int != INVALID_MV;
            above_row = a.mv[0].as_mv().row as i32;
            above_col = a.mv[0].as_mv().col as i32;
        }
        if let Some(l) = xd.mi_at(xd.left_mbmi) {
            left_mv_valid = l.mv[0].as_int != INVALID_MV;
            left_row = l.mv[0].as_mv().row as i32;
            left_col = l.mv[0].as_mv().col as i32;
        }
        if above_mv_valid && left_mv_valid {
            al_mv_average_row = (above_row + left_row + 1) >> 1;
            al_mv_average_col = (above_col + left_col + 1) >> 1;
        } else if above_mv_valid {
            al_mv_average_row = above_row;
            al_mv_average_col = above_col;
        } else if left_mv_valid {
            al_mv_average_row = left_row;
            al_mv_average_col = left_col;
        } else {
            al_mv_average_row = 0;
            al_mv_average_col = 0;
        }
        row_diff = al_mv_average_row - mv_row;
        col_diff = al_mv_average_col - mv_col;
        if row_diff > 80 || row_diff < -80 || col_diff > 80 || col_diff < -80 {
            if bsize >= BLOCK_32X32 {
                this_rdc.rdcost <<= 1;
            } else {
                this_rdc.rdcost = 5 * this_rdc.rdcost >> 2;
            }
        }
    } else {
        // Bias for speed >= 8 for low spatial variance.
        if speed >= 8
            && spatial_variance < 150
            && (mv_row > 64 || mv_row < -64 || mv_col > 64 || mv_col < -64)
        {
            this_rdc.rdcost = 5 * this_rdc.rdcost >> 2;
        }
    }
}

// (`src` is the source frame's samples)
fn model_rd_for_sb_uv(
    cpi: &Av1Comp,
    plane_bsize: BlockSize,
    x: &Macroblock,
    src: &[u16],
    this_rdc: &mut RdStats,
    start_plane: usize,
    stop_plane: usize,
) -> i64 {
    // Note our transform coeffs are 8 times an orthogonal transform.
    // Hence quantizer step is also 8 times. To get effective quantizer
    // we need to divide by 8 before sending to modeling function.
    let mut sse = 0u32;
    let mut rate = 0i32;
    let mut dist = 0i64;
    let mut tot_sse: i64 = 0;
    let xd = &x.e_mbd;

    this_rdc.rate = 0;
    this_rdc.dist = 0;
    this_rdc.skip_txfm = 0;

    for i in start_plane..=stop_plane {
        let p = &x.plane[i];
        let pd = &xd.plane[i];
        let dc_quant = p.dequant_qtx[0] as i32 as u32;
        let ac_quant = p.dequant_qtx[1] as i32 as u32;
        let bs = plane_bsize;
        if x.color_sensitivity[i - 1] == 0 {
            continue;
        }

        let var = cpi.ppi.fn_ptr.vf(
            bs,
            src,
            p.src.buf,
            p.src.stride as usize,
            &xd.recon.buffer_alloc,
            pd.dst.buf,
            pd.dst.stride as usize,
            &mut sse,
        );
        debug_assert!(sse >= var);
        tot_sse += sse as i64;

        av1_model_rd_from_var_lapndz(
            sse.wrapping_sub(var) as i64,
            NUM_PELS_LOG2_LOOKUP[bs as usize] as u32,
            dc_quant >> 3,
            &mut rate,
            &mut dist,
        );

        this_rdc.rate += rate >> 1;
        this_rdc.dist += dist << 3;

        av1_model_rd_from_var_lapndz(
            var as i64,
            NUM_PELS_LOG2_LOOKUP[bs as usize] as u32,
            ac_quant >> 3,
            &mut rate,
            &mut dist,
        );

        this_rdc.rate += rate;
        this_rdc.dist += dist << 4;
    }

    if this_rdc.rate == 0 {
        this_rdc.skip_txfm = 1;
    }

    if rdcost(x.rdmult, this_rdc.rate, this_rdc.dist) >= rdcost(x.rdmult, 0, tot_sse << 4) {
        this_rdc.rate = 0;
        this_rdc.dist = tot_sse << 4;
        this_rdc.skip_txfm = 1;
    }

    tot_sse
}

#[inline]
fn update_thresh_freq_fact(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    ref_frame: MvReferenceFrame,
    best_mode_idx: i32,
    mode: PredictionMode,
) -> Result<(), ()> {
    let thr_mode_idx = MODE_IDX[ref_frame as usize][mode_offset(mode)?];
    let min_size = (bsize as i32 - 3).max(BLOCK_4X4 as i32);
    let max_size = (bsize as i32 + 6).min(BLOCK_128X128 as i32);
    let mut bs = min_size;
    while bs <= max_size {
        let freq_fact = &mut x.thresh_freq_fact[bs as usize][thr_mode_idx as usize];
        if thr_mode_idx == best_mode_idx {
            *freq_fact -= *freq_fact >> 4;
        } else {
            *freq_fact = (*freq_fact + RD_THRESH_INC)
                .min(cpi.sf.inter_sf.adaptive_rd_thresh * RD_THRESH_MAX_FACT);
        }
        bs += 3;
    }
    Ok(())
}

// (CONFIG_AV1_TEMPORAL_DENOISING is off: av1_pickmode_ctx_den_update() and
// recheck_zeromv_after_denoising() are left out)

const FILTER_SEARCH_SIZE: usize = 2;

/*\brief Searches for the best interpolation filter
*
* \ingroup nonrd_mode_search
* \callgraph
* \callergraph
* Iterates through subset of possible interpolation filters (EIGHTTAP_REGULAR,
* EIGTHTAP_SMOOTH, MULTITAP_SHARP, depending on FILTER_SEARCH_SIZE) and selects
* the one that gives lowest RD cost. RD cost is calculated using curvfit model.
* Support for dual filters (different filters in the x & y directions) is
* allowed if sf.interp_sf.disable_dual_filter = 0.
*
* \param[in]    cpi                  Top-level encoder structure
* \param[in]    x                    Pointer to structure holding all the
*                                    data for the current macroblock
* \param[in]    this_rdc             Pointer to calculated RD Cost
* \param[in]    inter_pred_params_sr Pointer to structure holding parameters of
                                     inter prediction for single reference
* \param[in]    mi_row               Row index in 4x4 units
* \param[in]    mi_col               Column index in 4x4 units
* \param[in]    tmp_buffer           Pointer to a temporary buffer for
*                                    prediction re-use
* \param[in]    bsize                Current block size
* \param[in]    reuse_inter_pred     Flag, indicating prediction re-use
* \param[out]   this_mode_pred       Pointer to store prediction buffer
*                                    for prediction re-use
* \param[out]   this_early_term      Flag, indicating that transform can be
*                                    skipped
* \param[out]   var                  The residue variance of the current
*                                    predictor.
* \param[in]    use_model_yrd_large  Flag, indicating special logic to handle
*                                    large blocks
* \param[in]    best_sse             Best sse so far.
* \param[in]    comp_pred            Flag, indicating compound mode.
*
* \remark Nothing is returned. Instead, calculated RD cost is placed to
* \c this_rdc and best filter is placed to \c mi->interp_filters. In case
* \c reuse_inter_pred flag is set, this function also outputs
* \c this_mode_pred. Also \c this_early_temp is set if transform can be
* skipped
*/
// (reuse_inter_pred is 0, and tmp_buffer and this_mode_pred are left out)
fn search_filter_ref(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    this_rdc: &mut RdStats,
    inter_pred_params_sr: &mut InterPredParams,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    this_early_term: &mut i32,
    var: &mut u32,
    use_model_yrd_large: bool,
    best_sse: i64,
    comp_pred: bool,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let dim_factor = if cpi.sf.interp_sf.disable_dual_filter == 0 {
        FILTER_SEARCH_SIZE
    } else {
        1
    };
    let mut pf_rd_stats = [RdStats::default(); FILTER_SEARCH_SIZE * FILTER_SEARCH_SIZE];
    let mut pf_tx_size = [0 as TxSize; FILTER_SEARCH_SIZE * FILTER_SEARCH_SIZE];
    let mut best_skip = 0u8;
    let mut best_early_term = 0;
    let mut best_cost = i64::MAX;
    let mut best_filter_index: i32 = -1;

    let mut subpel_params = SubpelParams::default();
    // Initialize inter prediction params at mode level for single reference
    // mode.
    if !comp_pred {
        let xd = &x.e_mbd;
        let pd = &xd.plane[0];
        init_inter_mode_params(
            &xd.mbmi().mv[0].as_mv(),
            inter_pred_params_sr,
            &mut subpel_params,
            &xd.block_ref_scale_factors[0],
            pd.pre[0].width,
            pd.pre[0].height,
        );
    }
    for i in 0..FILTER_SEARCH_SIZE * FILTER_SEARCH_SIZE {
        let cost;
        if cpi.sf.interp_sf.disable_dual_filter != 0
            && FILTERS_REF_SET[i].filter_x != FILTERS_REF_SET[i].filter_y
        {
            continue;
        }
        x.e_mbd.mbmi_mut().interp_filters =
            interp_filters_of(FILTERS_REF_SET[i].filter_y, FILTERS_REF_SET[i].filter_x);
        if !comp_pred {
            av1_enc_build_inter_predictor_y_nonrd(
                cm,
                &mut x.e_mbd,
                inter_pred_params_sr,
                &subpel_params,
            )?;
        } else {
            av1_enc_build_inter_predictor(cm, &mut x.e_mbd, mi_row, mi_col, None, bsize, 0, 0)?;
        }
        let mut curr_var = u32::MAX;
        if use_model_yrd_large {
            model_skip_for_sb_y_large(
                cpi,
                bsize,
                mi_row,
                mi_col,
                x,
                &mut pf_rd_stats[i],
                this_early_term,
                1,
                best_sse,
                Some(&mut curr_var),
                u32::MAX,
            )?;
        } else {
            model_rd_for_sb_y(
                cpi,
                bsize,
                x,
                &mut pf_rd_stats[i],
                Some(&mut curr_var),
                1,
                None,
            );
        }
        pf_rd_stats[i].rate += av1_get_switchable_rate(
            x,
            &x.e_mbd,
            cm.features.interp_filter,
            cm.seq_params.enable_dual_filter != 0,
        );
        cost = rdcost(x.rdmult, pf_rd_stats[i].rate, pf_rd_stats[i].dist);
        pf_tx_size[i] = x.e_mbd.mbmi().tx_size;
        if cost < best_cost {
            *var = curr_var;
            best_filter_index = i as i32;
            best_cost = cost;
            best_skip = pf_rd_stats[i].skip_txfm;
            best_early_term = *this_early_term;
        }
    }
    debug_assert!(
        best_filter_index >= 0 && best_filter_index < (dim_factor * FILTER_SEARCH_SIZE) as i32
    );
    if best_filter_index < 0 {
        return Err(());
    }
    let b = best_filter_index as usize;

    {
        let mi = x.e_mbd.mbmi_mut();
        mi.interp_filters =
            interp_filters_of(FILTERS_REF_SET[b].filter_y, FILTERS_REF_SET[b].filter_x);
        mi.tx_size = pf_tx_size[b];
    }
    this_rdc.rate = pf_rd_stats[b].rate;
    this_rdc.dist = pf_rd_stats[b].dist;
    this_rdc.sse = pf_rd_stats[b].sse;
    this_rdc.skip_txfm = (best_skip != 0 || best_early_term != 0) as u8;
    *this_early_term = best_early_term;
    if b < dim_factor * FILTER_SEARCH_SIZE - 1 {
        if !comp_pred {
            av1_enc_build_inter_predictor_y_nonrd(
                cm,
                &mut x.e_mbd,
                inter_pred_params_sr,
                &subpel_params,
            )?;
        } else {
            av1_enc_build_inter_predictor(cm, &mut x.e_mbd, mi_row, mi_col, None, bsize, 0, 0)?;
        }
    }
    Ok(())
}

const MOTION_MODE_SEARCH_SIZE: usize = 2;

#[inline]
fn is_warped_mode_allowed(cpi: &Av1Comp, x: &Macroblock, mbmi: &MbModeInfo) -> bool {
    let features = &cpi.common.features;
    let xd = &x.e_mbd;

    if cpi.sf.inter_sf.extra_prune_warped != 0 {
        return false;
    }
    if has_second_ref(mbmi) {
        return false;
    }
    let mut last_motion_mode_allowed = SIMPLE_TRANSLATION;

    if features.switchable_motion_mode {
        // Determine which motion modes to search if more than SIMPLE_TRANSLATION
        // is allowed.
        last_motion_mode_allowed = motion_mode_allowed(
            &cpi.common.global_motion,
            xd,
            mbmi,
            features.allow_warped_motion,
        );
    }

    if last_motion_mode_allowed == WARPED_CAUSAL {
        return true;
    }

    false
}

fn calc_num_proj_ref(cpi: &Av1Comp, x: &mut Macroblock) {
    let cm = &cpi.common;
    let features = &cm.features;
    let Macroblock {
        e_mbd: xd,
        warp_sample_info,
        ..
    } = x;

    xd.mbmi_mut().num_proj_ref = 1;
    let mi = *xd.mbmi();
    let warp_sample_info = &mut warp_sample_info[mi.ref_frame[0] as usize];
    let mut last_motion_mode_allowed = SIMPLE_TRANSLATION;

    if features.switchable_motion_mode {
        // Determine which motion modes to search if more than SIMPLE_TRANSLATION
        // is allowed.
        last_motion_mode_allowed =
            motion_mode_allowed(&cm.global_motion, xd, &mi, features.allow_warped_motion);
    }

    if last_motion_mode_allowed == WARPED_CAUSAL {
        if warp_sample_info.num < 0 {
            warp_sample_info.num = av1_find_samples(
                cm,
                xd,
                &mut warp_sample_info.pts,
                &mut warp_sample_info.pts_inref,
            ) as i32;
        }
        xd.mbmi_mut().num_proj_ref = warp_sample_info.num as u8;
    }
}

// (`Err` for WARPED_CAUSAL, which extra_prune_warped never lets it search)
fn search_motion_mode(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    this_rdc: &mut RdStats,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    this_early_term: &mut i32,
    use_model_yrd_large: bool,
    rate_mv: &mut i32,
    best_sse: i64,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let mut pf_rd_stats = [RdStats::default(); MOTION_MODE_SEARCH_SIZE];
    let mut best_skip = 0u8;
    let mut best_early_term = 0;
    let mut best_cost = i64::MAX;
    let mut best_mode_index: i32 = -1;

    let motion_modes: [MotionMode; MOTION_MODE_SEARCH_SIZE] = [SIMPLE_TRANSLATION, WARPED_CAUSAL];
    let mut mode_search_size = if is_warped_mode_allowed(cpi, x, x.e_mbd.mbmi()) {
        2
    } else {
        1
    };

    let total_samples = x.e_mbd.mbmi().num_proj_ref;
    if total_samples == 0 {
        // Do not search WARPED_CAUSAL if there are no samples to use to determine
        // warped parameters.
        mode_search_size = 1;
    }

    let base_mbmi = *x.e_mbd.mbmi();
    let mut best_mbmi = base_mbmi;
    let _ = rate_mv;

    for i in 0..mode_search_size {
        let mut cost = i64::MAX;
        let motion_mode = motion_modes[i];
        {
            let mi = x.e_mbd.mbmi_mut();
            *mi = base_mbmi;
            mi.motion_mode = motion_mode;
        }
        if motion_mode == SIMPLE_TRANSLATION {
            x.e_mbd.mbmi_mut().interp_filters = av1_broadcast_interp_filter(EIGHTTAP_REGULAR);

            av1_enc_build_inter_predictor(cm, &mut x.e_mbd, mi_row, mi_col, None, bsize, 0, 0)?;
            if use_model_yrd_large {
                model_skip_for_sb_y_large(
                    cpi,
                    bsize,
                    mi_row,
                    mi_col,
                    x,
                    &mut pf_rd_stats[i],
                    this_early_term,
                    1,
                    best_sse,
                    None,
                    u32::MAX,
                )?;
            } else {
                model_rd_for_sb_y(cpi, bsize, x, &mut pf_rd_stats[i], None, 1, None);
            }
            pf_rd_stats[i].rate += av1_get_switchable_rate(
                x,
                &x.e_mbd,
                cm.features.interp_filter,
                cm.seq_params.enable_dual_filter != 0,
            );
            cost = rdcost(x.rdmult, pf_rd_stats[i].rate, pf_rd_stats[i].dist);
        } else if motion_mode == WARPED_CAUSAL {
            // (av1_selectSamples(), av1_find_projection() and
            // av1_refine_warped_mv())
            return Err(());
        }
        if cost < best_cost {
            best_mode_index = i as i32;
            best_cost = cost;
            best_skip = pf_rd_stats[i].skip_txfm;
            best_early_term = *this_early_term;
            best_mbmi = *x.e_mbd.mbmi();
        }
    }
    debug_assert!(best_mode_index >= 0 && best_mode_index < FILTER_SEARCH_SIZE as i32);
    if best_mode_index < 0 {
        return Err(());
    }
    let b = best_mode_index as usize;

    *x.e_mbd.mbmi_mut() = best_mbmi;
    this_rdc.rate = pf_rd_stats[b].rate;
    this_rdc.dist = pf_rd_stats[b].dist;
    this_rdc.sse = pf_rd_stats[b].sse;
    this_rdc.skip_txfm = (best_skip != 0 || best_early_term != 0) as u8;
    *this_early_term = best_early_term;
    if b < FILTER_SEARCH_SIZE - 1 {
        av1_enc_build_inter_predictor(cm, &mut x.e_mbd, mi_row, mi_col, None, bsize, 0, 0)?;
    }
    Ok(())
}

// (COLLECT_PICK_MODE_STAT is 0)

fn compute_intra_yprediction(
    cm: &Av1Common,
    mode: PredictionMode,
    bsize: BlockSize,
    x: &mut Macroblock,
) -> Result<(), ()> {
    let seq_params = &cm.seq_params;
    let src_buf_base = x.plane[0].src.buf;
    let dst_buf_base = x.e_mbd.plane[0].dst.buf;
    let src_stride = x.plane[0].src.stride;
    let dst_stride = x.e_mbd.plane[0].dst.stride;
    let plane = 0;
    // block and transform sizes, in number of 4x4 blocks log 2 ("*_b")
    // 4x4=0, 8x8=2, 16x16=4, 32x32=6, 64x64=8
    // transform size varies per plane, look it up in a common way.
    let tx_size = MAX_TXSIZE_LOOKUP[bsize as usize];
    let plane_bsize = get_plane_block_size(
        bsize,
        x.e_mbd.plane[0].subsampling_x,
        x.e_mbd.plane[0].subsampling_y,
    );
    // If mb_to_right_edge is < 0 we are in a situation in which
    // the current block size extends into the UMV and we won't
    // visit the sub blocks that are wholly within the UMV.
    let max_blocks_wide = max_block_wide(&x.e_mbd, plane_bsize, plane);
    let max_blocks_high = max_block_high(&x.e_mbd, plane_bsize, plane);
    // Keep track of the row and column of the blocks we use so that we know
    // if we are in the unrestricted motion border.
    let mut row = 0;
    while row < max_blocks_high {
        // Skip visiting the sub blocks that are wholly within the UMV.
        let mut col = 0;
        while col < max_blocks_wide {
            x.plane[0].src.buf =
                (src_buf_base as i64 + 4 * (row as i64 * src_stride as i64 + col as i64)) as usize;
            let dst =
                (dst_buf_base as i64 + 4 * (row as i64 * dst_stride as i64 + col as i64)) as usize;
            x.e_mbd.plane[0].dst.buf = dst;
            av1_predict_intra_block(
                &mut x.e_mbd,
                seq_params.sb_size,
                seq_params.enable_intra_edge_filter != 0,
                BLOCK_SIZE_WIDE[bsize as usize] as i32,
                BLOCK_SIZE_HIGH[bsize as usize] as i32,
                tx_size,
                mode,
                0,
                false,
                FILTER_INTRA_MODES,
                dst,
                dst_stride as usize,
                0,
                0,
                plane,
            )?;
            col += 1 << tx_size;
        }
        row += 1 << tx_size;
    }
    x.plane[0].src.buf = src_buf_base;
    x.e_mbd.plane[0].dst.buf = dst_buf_base;
    Ok(())
}

#[inline]
fn is_same_gf_and_last_scale(cm: &Av1Common) -> bool {
    let sf_last = get_ref_scale_factors_const(cm, LAST_FRAME);
    let sf_golden = get_ref_scale_factors_const(cm, GOLDEN_FRAME);
    match (sf_last, sf_golden) {
        (Some(l), Some(g)) => (l.x_scale_fp == g.x_scale_fp) && (l.y_scale_fp == g.y_scale_fp),
        _ => false,
    }
}

fn get_ref_frame_use_mask(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    gf_temporal_ref: bool,
    use_ref_frame: &mut [i32; REF_FRAMES as usize],
    force_skip_low_temp_var: &mut i32,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let seg = &cm.seg;
    let is_small_sb = cm.seq_params.sb_size == BLOCK_64X64;
    let segment_id = x.e_mbd.mbmi().segment_id;

    // When the ref_frame_config is used to set the reference frame structure
    // then the usage of alt_ref is determined by the ref_frame_flags
    // (and not the speed feature use_nonrd_altref_frame).
    let mut use_alt_ref_frame =
        cpi.ppi.rtc_ref.set_ref_frame_config || cpi.sf.rt_sf.use_nonrd_altref_frame != 0;

    let mut use_golden_ref_frame = true;
    let mut use_last_ref_frame = true;

    // When the ref_frame_config is used to set the reference frame structure:
    // check if LAST is used as a reference. And only remove golden and altref
    // references below if last is used as a reference.
    if cpi.ppi.rtc_ref.set_ref_frame_config {
        use_last_ref_frame = if cpi.ref_frame_flags & AOM_LAST_FLAG != 0 {
            use_last_ref_frame
        } else {
            false
        };
    }

    // frame_since_golden is not used when user sets the referene structure.
    if !cpi.ppi.rtc_ref.set_ref_frame_config
        && use_last_ref_frame
        && cpi.rc.frames_since_golden == 0
        && gf_temporal_ref
    {
        use_golden_ref_frame = false;
    }

    if use_last_ref_frame
        && cpi.sf.rt_sf.short_circuit_low_temp_var != 0
        && x.nonrd_prune_ref_frame_search != 0
    {
        if is_small_sb {
            *force_skip_low_temp_var = av1_get_force_skip_low_temp_var_small_sb(
                &x.part_search_info.variance_low,
                mi_row,
                mi_col,
                bsize,
            );
        } else {
            *force_skip_low_temp_var = av1_get_force_skip_low_temp_var(
                &x.part_search_info.variance_low,
                mi_row,
                mi_col,
                bsize,
            );
        }
        // If force_skip_low_temp_var is set, skip golden reference.
        if *force_skip_low_temp_var != 0 {
            use_golden_ref_frame = false;
            use_alt_ref_frame = false;
        }
    }

    if use_last_ref_frame
        && (x.nonrd_prune_ref_frame_search > 2
            || x.force_zeromv_skip_for_blk != 0
            || (x.nonrd_prune_ref_frame_search > 1 && bsize > BLOCK_64X64))
    {
        use_golden_ref_frame = false;
        use_alt_ref_frame = false;
    }

    if segfeature_active(seg, segment_id, SEG_LVL_REF_FRAME)
        && get_segdata(seg, segment_id as i32, SEG_LVL_REF_FRAME) == GOLDEN_FRAME as i32
    {
        use_golden_ref_frame = true;
        use_alt_ref_frame = false;
    }

    // Skip golden reference if color is set, on flat blocks with motion.
    // For screen: always skip golden (if color_sensitivity_sb_g is set)
    // except when x->nonrd_prune_ref_frame_search = 0. This latter flag
    // may be set in the variance partition when golden is a much better
    // reference than last, in which case it may not be worth skipping
    // golden completely.
    if ((cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN && x.nonrd_prune_ref_frame_search != 0)
        || (x.source_variance < 500 && x.content_state_sb.source_sad_nonrd > K_LOW_SAD))
        && (x.color_sensitivity_sb_g[0] == 1 || x.color_sensitivity_sb_g[1] == 1)
    {
        use_golden_ref_frame = false;
    }

    // For non-screen: if golden and altref are not being selected as references
    // (use_golden_ref_frame/use_alt_ref_frame = 0) check to allow golden back
    // based on the sad of nearest/nearmv of LAST ref. If this block sad is large,
    // keep golden as reference. Only do this for the agrressive pruning mode and
    // avoid it when color is set for golden reference.
    if cpi.oxcf.tune_cfg.content != AOM_CONTENT_SCREEN
        && (cpi.ref_frame_flags & AOM_LAST_FLAG) != 0
        && !use_golden_ref_frame
        && !use_alt_ref_frame
        && x.pred_mv_sad[LAST_FRAME as usize] != i32::MAX
        && x.nonrd_prune_ref_frame_search > 2
        && x.color_sensitivity_sb_g[0] == 0
        && x.color_sensitivity_sb_g[1] == 0
    {
        let thr = if cm.width * cm.height >= 640 * 360 {
            100
        } else {
            150
        };
        let pred = x.pred_mv_sad[LAST_FRAME as usize]
            >> (B_WIDTH_LOG2_LOOKUP[bsize as usize] as i32
                + B_HEIGHT_LOG2_LOOKUP[bsize as usize] as i32);
        if pred > thr {
            use_golden_ref_frame = true;
        }
    }

    use_alt_ref_frame = if cpi.ref_frame_flags & AOM_ALT_FLAG != 0 {
        use_alt_ref_frame
    } else {
        false
    };
    use_golden_ref_frame = if cpi.ref_frame_flags & AOM_GOLD_FLAG != 0 {
        use_golden_ref_frame
    } else {
        false
    };

    // For spatial layers: enable golden ref if it is set by user and
    // corresponds to the lower spatial layer.
    if cpi.svc.spatial_layer_id > 0
        && (cpi.ref_frame_flags & AOM_GOLD_FLAG) != 0
        && x.content_state_sb.source_sad_nonrd < K_HIGH_SAD
    {
        // (spatial layers are never coded)
        return Err(());
    }

    use_ref_frame[ALTREF_FRAME as usize] = use_alt_ref_frame as i32;
    use_ref_frame[GOLDEN_FRAME as usize] = use_golden_ref_frame as i32;
    use_ref_frame[LAST_FRAME as usize] = use_last_ref_frame as i32;
    // For now keep this assert on, but we should remove it for svc mode,
    // as the user may want to generate an intra-only frame (no inter-modes).
    // Remove this assert in subsequent CL when nonrd_pickmode is tested for the
    // case of intra-only frame (no references enabled).
    debug_assert!(use_last_ref_frame || use_golden_ref_frame || use_alt_ref_frame);
    Ok(())
}

// Checks whether Intra mode needs to be pruned based on
// 'intra_y_mode_bsize_mask_nrd' and 'prune_hv_pred_modes_using_blksad'
// speed features.
#[inline]
fn is_prune_intra_mode(
    cpi: &Av1Comp,
    mode_index: usize,
    force_intra_check: i32,
    bsize: BlockSize,
    segment_id: u8,
    source_sad_nonrd: i32,
    color_sensitivity: &[u8; 2],
) -> bool {
    let this_mode = INTRA_MODE_LIST[mode_index];
    if mode_index > 2 || force_intra_check == 0 {
        if ((1 << this_mode) & cpi.sf.rt_sf.intra_y_mode_bsize_mask_nrd[bsize as usize]) == 0 {
            return true;
        }

        if this_mode == DC_PRED {
            return false;
        }

        if !cpi.sf.rt_sf.prune_hv_pred_modes_using_src_sad {
            return false;
        }

        let has_color_sensitivity = color_sensitivity[0] != 0 && color_sensitivity[1] != 0;
        if has_color_sensitivity
            && (cpi.rc.frame_source_sad as f64 > 1.1 * cpi.rc.avg_source_sad as f64
                || cyclic_refresh_segment_id_boosted(segment_id as i32)
                || source_sad_nonrd > K_MED_SAD)
        {
            return false;
        }

        return true;
    }
    false
}

/*\brief Estimates best intra mode for inter mode search
 *
 * \ingroup nonrd_mode_search
 * \callgraph
 * \callergraph
 *
 * Using heuristics based on best inter mode, block size, and other decides
 * whether to check intra modes. If so, estimates and selects best intra mode
 * from the reduced set of intra modes (max 4 intra modes checked)
 *
 * \param[in]    cpi                      Top-level encoder structure
 * \param[in]    x                        Pointer to structure holding all the
 *                                        data for the current macroblock
 * \param[in]    bsize                    Current block size
 * \param[in]    best_early_term          Flag, indicating that TX for the
 *                                        best inter mode was skipped
 * \param[in]    ref_cost_intra           Cost of signalling intra mode
 * \param[in]    reuse_prediction         Flag, indicating prediction re-use
 * \param[in]    orig_dst                 Original destination buffer
 * \param[in]    tmp_buffers              Pointer to a temporary buffers for
 *                                        prediction re-use
 * \param[out]   this_mode_pred           Pointer to store prediction buffer
 *                                        for prediction re-use
 * \param[in]    best_rdc                 Pointer to RD cost for the best
 *                                        selected intra mode
 * \param[in]    best_pickmode            Pointer to a structure containing
 *                                        best mode picked so far
 * \param[in]    ctx                      Pointer to structure holding coding
 *                                        contexts and modes for the block
 *
 * \remark Nothing is returned. Instead, calculated RD cost is placed to
 * \c best_rdc and best selected mode is placed to \c best_pickmode
 */
// (`src` is the source frame's samples; reuse_prediction is 0, and
// tmp_buffers and this_mode_pred are left out)
fn estimate_intra_mode(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    src: &[u16],
    bsize: BlockSize,
    mut best_early_term: i32,
    ref_cost_intra: u32,
    orig_dst: &Buf2d,
    best_rdc: &mut RdStats,
    best_pickmode: &mut BestPickmode,
    ctx: &mut PickModeContext,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let txfm_params = &x.txfm_search_params;
    let segment_id = x.e_mbd.mbmi().segment_id;
    let rd_threshes = &cpi.rd.threshes[segment_id as usize][bsize as usize];
    let is_screen_content = cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN;

    let quant_params = &cm.quant_params;

    let mut this_rdc = RdStats::default();

    let mut intra_cost_penalty = av1_get_intra_cost_penalty(
        quant_params.base_qindex,
        quant_params.y_dc_delta_q,
        cm.seq_params.bit_depth,
    );
    let mut inter_mode_thresh = rdcost(
        x.rdmult,
        (ref_cost_intra as i32).wrapping_add(intra_cost_penalty),
        0,
    );
    let mut perform_intra_pred = cpi.sf.rt_sf.check_intra_pred_nonrd;
    let mut force_intra_check = 0;
    // For spatial enhancement layer: turn off intra prediction if the
    // previous spatial layer as golden ref is not chosen as best reference.
    // only do this for temporal enhancement layer and on non-key frames.
    if cpi.svc.spatial_layer_id > 0 {
        // (spatial layers are never coded)
        return Err(());
    }

    let mut do_early_exit_rdthresh = true;

    let mut spatial_var_thresh: u32 = 50;
    let mut motion_thresh = 32;
    // Adjust thresholds to make intra mode likely tested if the other
    // references (golden, alt) are skipped/not checked. For now always
    // adjust for svc mode.
    if cpi.ppi.use_svc
        || (cpi.sf.rt_sf.use_nonrd_altref_frame == 0
            && cpi.sf.rt_sf.nonrd_prune_ref_frame_search > 0)
    {
        spatial_var_thresh = 150;
        motion_thresh = 0;
    }

    // Some adjustments to checking intra mode based on source variance.
    if x.source_variance < spatial_var_thresh {
        // If the best inter mode is large motion or non-LAST ref reduce intra cost
        // penalty, so intra mode is more likely tested.
        let mv0 = x.e_mbd.mbmi().mv[0].as_mv();
        if best_rdc.rdcost != i64::MAX
            && (best_pickmode.best_ref_frame != LAST_FRAME
                || (mv0.row as i32).abs() >= motion_thresh
                || (mv0.col as i32).abs() >= motion_thresh)
        {
            intra_cost_penalty >>= 2;
            inter_mode_thresh = rdcost(
                x.rdmult,
                (ref_cost_intra as i32).wrapping_add(intra_cost_penalty),
                0,
            );
            do_early_exit_rdthresh = false;
        }
        if (x.source_variance < (spatial_var_thresh >> 1).max(50)
            && x.content_state_sb.source_sad_nonrd >= K_HIGH_SAD)
            || (is_screen_content
                && x.source_variance < 50
                && ((bsize >= BLOCK_32X32 && x.content_state_sb.source_sad_nonrd != K_ZERO_SAD)
                    || x.color_sensitivity[0] == 1
                    || x.color_sensitivity[1] == 1))
        {
            force_intra_check = 1;
        }
        // For big blocks worth checking intra (since only DC will be checked),
        // even if best_early_term is set.
        if bsize >= BLOCK_32X32 {
            best_early_term = 0;
        }
    } else if cpi.sf.rt_sf.source_metrics_sb_nonrd != 0
        && x.content_state_sb.source_sad_nonrd <= K_LOW_SAD
    {
        perform_intra_pred = 0;
    }

    if best_rdc.skip_txfm != 0 && best_pickmode.best_mode_initial_skip_flag != 0 {
        if cpi.sf.rt_sf.skip_intra_pred == 1 && best_pickmode.best_mode != NEWMV {
            perform_intra_pred = 0;
        } else if cpi.sf.rt_sf.skip_intra_pred == 2 {
            perform_intra_pred = 0;
        }
    }

    if !(best_rdc.rdcost == i64::MAX
        || force_intra_check != 0
        || (perform_intra_pred != 0
            && best_early_term == 0
            && bsize <= cpi.sf.part_sf.max_intra_bsize))
    {
        return Ok(());
    }

    // Early exit based on RD cost calculated using known rate. When
    // is_screen_content is true, more bias is given to intra modes. Hence,
    // considered conservative threshold in early exit for the same.
    let known_rd = if is_screen_content {
        calc_biased_rdcost(inter_mode_thresh)
    } else {
        inter_mode_thresh
    };
    if known_rd > best_rdc.rdcost {
        return Ok(());
    }

    let mut intra_tx_size = MAX_TXSIZE_LOOKUP[bsize as usize]
        .min(TX_MODE_TO_BIGGEST_TX_SIZE[txfm_params.tx_mode_search_type as usize])
        .min(TX_16X16);
    if is_screen_content
        && cpi.rc.high_source_sad != 0
        && x.source_variance > spatial_var_thresh
        && bsize <= BLOCK_16X16
    {
        intra_tx_size = TX_4X4;
    }

    // (reuse_prediction is 0)
    x.e_mbd.plane[0].dst = *orig_dst;

    let mut args = EstimateBlockIntraArgs {
        cpi,
        x,
        src,
        mode: DC_PRED,
        skippable: 1,
        rdc: RdStats::default(),
    };

    for i in 0..4 {
        let this_mode = INTRA_MODE_LIST[i];
        let mode_index = MODE_IDX[INTRA_FRAME as usize][mode_offset(this_mode)?];
        let mode_rd_thresh = rd_threshes[mode_index as usize] as i64;

        if is_prune_intra_mode(
            cpi,
            i,
            force_intra_check,
            bsize,
            segment_id,
            args.x.content_state_sb.source_sad_nonrd,
            &args.x.color_sensitivity,
        ) {
            continue;
        }

        if is_screen_content && cpi.sf.rt_sf.source_metrics_sb_nonrd != 0 {
            // For spatially flat blocks with zero motion only check
            // DC mode.
            if args.x.content_state_sb.source_sad_nonrd == K_ZERO_SAD
                && args.x.source_variance == 0
                && this_mode != DC_PRED
            {
                continue;
            }
            // Only test Intra for big blocks if spatial_variance is small.
            else if bsize > BLOCK_32X32 && args.x.source_variance > 50 {
                continue;
            }
        }

        if rd_less_than_thresh(
            best_rdc.rdcost,
            mode_rd_thresh,
            args.x.thresh_freq_fact[bsize as usize][mode_index as usize],
        ) && (do_early_exit_rdthresh || this_mode == SMOOTH_PRED)
        {
            continue;
        }
        let uv_bsize = get_plane_block_size(
            bsize,
            args.x.e_mbd.plane[1].subsampling_x,
            args.x.e_mbd.plane[1].subsampling_y,
        );

        {
            let mi = args.x.e_mbd.mbmi_mut();
            mi.mode = this_mode;
            mi.ref_frame[0] = INTRA_FRAME;
            mi.ref_frame[1] = NONE_FRAME;
        }

        av1_invalid_rd_stats(&mut this_rdc);
        args.mode = this_mode;
        args.skippable = 1;
        // (args.rdc = &this_rdc)
        args.rdc = this_rdc;
        args.x.e_mbd.mbmi_mut().tx_size = intra_tx_size;
        compute_intra_yprediction(cm, this_mode, bsize, args.x)?;
        // Look into selecting tx_size here, based on prediction residual.
        let tx_size = args.x.e_mbd.mbmi().tx_size;
        let mut rdc = args.rdc;
        block_yrd(
            args.x,
            src,
            &mut rdc,
            &mut args.skippable,
            bsize,
            tx_size,
            0,
        )?;
        args.rdc = rdc;
        // TODO(kyslov@) Need to account for skippable
        if args.x.color_sensitivity[0] != 0 {
            av1_foreach_transformed_block_in_plane(
                &mut args,
                |a| &a.x.e_mbd,
                uv_bsize,
                1,
                |a, plane, block, row, col, plane_bsize, tx_size| {
                    estimate_block_intra(plane, block, row, col, plane_bsize, tx_size, a)
                },
            )?;
        }
        if args.x.color_sensitivity[1] != 0 {
            av1_foreach_transformed_block_in_plane(
                &mut args,
                |a| &a.x.e_mbd,
                uv_bsize,
                2,
                |a, plane, block, row, col, plane_bsize, tx_size| {
                    estimate_block_intra(plane, block, row, col, plane_bsize, tx_size, a)
                },
            )?;
        }
        this_rdc = args.rdc;

        let x = &mut *args.x;
        let mut mode_cost = 0;
        if av1_is_directional_mode(this_mode) && av1_use_angle_delta(bsize) {
            mode_cost += x.mode_costs.angle_delta_cost[(this_mode - V_PRED) as usize]
                [(MAX_ANGLE_DELTA + x.e_mbd.mbmi().angle_delta[PLANE_TYPE_Y as usize] as i32)
                    as usize];
        }
        if this_mode == DC_PRED && av1_filter_intra_allowed_bsize(cm, bsize) {
            mode_cost += x.mode_costs.filter_intra_cost[bsize as usize][0];
        }
        this_rdc.rate = this_rdc.rate.wrapping_add(ref_cost_intra as i32);
        this_rdc.rate += intra_cost_penalty;
        this_rdc.rate += mode_cost;
        this_rdc.rdcost = rdcost(x.rdmult, this_rdc.rate, this_rdc.dist);

        if is_screen_content && cpi.sf.rt_sf.source_metrics_sb_nonrd != 0 {
            // For blocks with low spatial variance and color sad,
            // favor the intra-modes, only on scene/slide change.
            if cpi.rc.high_source_sad != 0
                && x.source_variance < 800
                && (x.color_sensitivity[0] != 0 || x.color_sensitivity[1] != 0)
            {
                this_rdc.rdcost = calc_biased_rdcost(this_rdc.rdcost);
            }
            // Otherwise bias against intra for blocks with zero
            // motion and no color, on non-scene/slide changes.
            else if cpi.rc.high_source_sad == 0
                && x.source_variance > 0
                && x.content_state_sb.source_sad_nonrd == K_ZERO_SAD
                && x.color_sensitivity[0] == 0
                && x.color_sensitivity[1] == 0
            {
                this_rdc.rdcost = (3 * this_rdc.rdcost) >> 1;
            }
        }

        if this_rdc.rdcost < best_rdc.rdcost {
            *best_rdc = this_rdc;
            best_pickmode.best_mode = this_mode;
            best_pickmode.best_tx_size = x.e_mbd.mbmi().tx_size;
            best_pickmode.best_ref_frame = INTRA_FRAME;
            best_pickmode.best_second_ref_frame = NONE_FRAME;
            best_pickmode.best_mode_skip_txfm = this_rdc.skip_txfm;
            if this_rdc.skip_txfm == 0 {
                let n = ctx.num_4x4_blk as usize;
                ctx.blk_skip[..n].copy_from_slice(&x.txfm_search_info.blk_skip[..n]);
            }
            let mi = x.e_mbd.mbmi_mut();
            mi.uv_mode = this_mode as UvPredictionMode;
            mi.mv[0].as_int = INVALID_MV;
            mi.mv[1].as_int = INVALID_MV;
        }
    }
    args.x.e_mbd.mbmi_mut().tx_size = best_pickmode.best_tx_size;
    Ok(())
}

#[inline]
fn is_filter_search_enabled_blk(
    cpi: &Av1Comp,
    x: &Macroblock,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    segment_id: u8,
    cb_pred_filter_search: i32,
    filt_select: &mut InterpFilter,
) -> i32 {
    let cm = &cpi.common;
    // filt search disabled
    if cpi.sf.rt_sf.use_nonrd_filter_search == 0 {
        return 0;
    }
    // filt search purely based on mode properties
    if cb_pred_filter_search == 0 {
        return 1;
    }
    let xd = &x.e_mbd;
    let enable_interp_search;
    let left = xd.mi_at(xd.left_mbmi);
    let above = xd.mi_at(xd.above_mbmi);
    match (left, above) {
        (Some(left), Some(above)) => {
            if !(is_inter_block(left) && is_inter_block(above)) {
                // neighbor is INTRA
                enable_interp_search = 2;
            } else if left.interp_filters != above.interp_filters {
                // filters are different
                enable_interp_search = 2;
            } else if (cb_pred_filter_search == 1)
                && (av1_extract_interp_filter(left.interp_filters, 1) != EIGHTTAP_REGULAR)
            {
                // not regular
                enable_interp_search = 2;
            } else {
                // enable prediction based on chessboard pattern
                if av1_extract_interp_filter(left.interp_filters, 1) == EIGHTTAP_SMOOTH {
                    *filt_select = EIGHTTAP_SMOOTH;
                }
                let bsl = MI_SIZE_WIDE_LOG2[bsize as usize] as i32;
                let mut e = ((((mi_row + mi_col) >> bsl)
                    + get_chessboard_index(cm.current_frame.frame_number))
                    & 0x1)
                    != 0;
                if cyclic_refresh_segment_id_boosted(segment_id as i32) {
                    e = true;
                }
                enable_interp_search = e as i32;
            }
        }
        _ => {
            // neighbors info unavailable
            enable_interp_search = 2;
        }
    }
    enable_interp_search
}

#[inline]
fn skip_mode_by_threshold(
    mode: PredictionMode,
    ref_frame: MvReferenceFrame,
    mv: IntMv,
    frames_since_golden: i32,
    rd_threshes: &[i32],
    rd_thresh_freq_fact: &[i32],
    best_cost: i64,
    best_skip: u8,
    extra_shift: i32,
) -> bool {
    let mut skip_this_mode = false;
    let mode_index = MODE_IDX[ref_frame as usize][inter_offset(mode)] as usize;
    let mut mode_rd_thresh: i64 = if best_skip != 0 {
        (rd_threshes[mode_index] as i64) << (extra_shift + 1)
    } else {
        (rd_threshes[mode_index] as i64) << extra_shift
    };

    // Increase mode_rd_thresh value for non-LAST for improved encoding
    // speed
    if ref_frame != LAST_FRAME {
        mode_rd_thresh <<= 1;
        if ref_frame == GOLDEN_FRAME && frames_since_golden > 4 {
            mode_rd_thresh <<= extra_shift + 1;
        }
    }

    if rd_less_than_thresh(best_cost, mode_rd_thresh, rd_thresh_freq_fact[mode_index])
        && mv.as_int != 0
    {
        skip_this_mode = true;
    }

    skip_this_mode
}

#[inline]
fn skip_mode_by_low_temp(
    mode: PredictionMode,
    ref_frame: MvReferenceFrame,
    bsize: BlockSize,
    content_state_sb: ContentStateSb,
    mv: IntMv,
    force_skip_low_temp_var: i32,
) -> bool {
    // Skip non-zeromv mode search for non-LAST frame if force_skip_low_temp_var
    // is set. If nearestmv for golden frame is 0, zeromv mode will be skipped
    // later.
    if force_skip_low_temp_var != 0 && ref_frame != LAST_FRAME && mv.as_int != 0 {
        return true;
    }

    if content_state_sb.source_sad_nonrd != K_HIGH_SAD
        && bsize >= BLOCK_64X64
        && force_skip_low_temp_var != 0
        && mode == NEWMV
    {
        return true;
    }
    false
}

#[inline]
fn skip_mode_by_bsize_and_ref_frame(
    mode: PredictionMode,
    ref_frame: MvReferenceFrame,
    bsize: BlockSize,
    extra_prune: i32,
    sse_zeromv_norm: u32,
    more_prune: i32,
) -> bool {
    let thresh_skip_golden: u32 = 500;

    if ref_frame != LAST_FRAME && sse_zeromv_norm < thresh_skip_golden && mode == NEWMV {
        return true;
    }

    if bsize == BLOCK_128X128 && mode == NEWMV {
        return true;
    }

    // Skip testing non-LAST if this flag is set.
    if extra_prune != 0 {
        if extra_prune > 1 && ref_frame != LAST_FRAME && (bsize > BLOCK_16X16 && mode == NEWMV) {
            return true;
        }

        if ref_frame != LAST_FRAME && mode == NEARMV {
            return true;
        }

        if more_prune != 0 && bsize >= BLOCK_32X32 && mode == NEARMV {
            return true;
        }
    }
    false
}

fn set_color_sensitivity(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    y_sad: i32,
    source_variance: u32,
    yv12_mb: &[Buf2d; MAX_MB_PLANE],
) {
    let subsampling_x = cpi.common.seq_params.subsampling_x;
    let subsampling_y = cpi.common.seq_params.subsampling_y;
    let mut factor = if bsize >= BLOCK_32X32 { 2 } else { 3 };
    let mut shift = 3;
    if cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN && cpi.rc.high_source_sad != 0 {
        factor = 1;
        shift = 6;
    }
    // (the noise estimate is never enabled: kLow)
    let noise_level_is_low = true;
    let norm_sad = y_sad
        >> (B_WIDTH_LOG2_LOOKUP[bsize as usize] as i32
            + B_HEIGHT_LOG2_LOOKUP[bsize as usize] as i32);
    let thresh_spatial: u32 = if cpi.common.width > 1920 { 5000 } else { 1000 };
    // If the spatial source variance is high and the normalized y_sad
    // is low, then y-channel is likely good for mode estimation, so keep
    // color_sensitivity off. For low noise content for now, since there is
    // some bdrate regression for noisy color clip.
    if noise_level_is_low
        && source_variance > thresh_spatial
        && cpi.oxcf.tune_cfg.content != AOM_CONTENT_SCREEN
        && norm_sad < 50
    {
        x.color_sensitivity[0] = 0;
        x.color_sensitivity[1] = 0;
        return;
    }
    let num_planes = av1_num_planes(&cpi.common);
    for i in 1..num_planes {
        if x.color_sensitivity[i - 1] == 2 || source_variance < 50 {
            let p = &x.plane[i];
            let bs = get_plane_block_size(bsize, subsampling_x, subsampling_y);

            let uv_sad = cpi.ppi.fn_ptr.sdf(
                bs,
                &x.source_alloc,
                p.src.buf,
                p.src.stride as usize,
                &cpi.common.buffer_pool.frame_bufs[yv12_mb[i].frame_buf]
                    .buf
                    .buffer_alloc,
                yv12_mb[i].buf,
                yv12_mb[i].stride as usize,
            ) as i32;

            let norm_uv_sad = uv_sad
                >> (B_WIDTH_LOG2_LOOKUP[bs as usize] as i32
                    + B_HEIGHT_LOG2_LOOKUP[bs as usize] as i32);
            x.color_sensitivity[i - 1] =
                (uv_sad > (factor * (y_sad >> shift)) && norm_uv_sad > 40) as u8;
            if source_variance < 50 && norm_uv_sad > 100 {
                x.color_sensitivity[i - 1] = 1;
            }
        }
    }
}

fn setup_compound_prediction(
    cm: &Av1Common,
    x: &mut Macroblock,
    yv12_mb: &mut [[Buf2d; MAX_MB_PLANE]; REF_FRAMES as usize],
    use_ref_frame_mask: &[i32; REF_FRAMES as usize],
    rf: &[MvReferenceFrame; 2],
    ref_mv_idx: &mut i32,
) {
    if use_ref_frame_mask[rf[1] as usize] == 0 {
        // Need to setup pred_block, if it hasn't been done in find_predictors.
        let num_planes = av1_num_planes(cm);
        if let Some((yv12, yv12_idx)) = get_ref_frame_yv12_buf(cm, rf[1]) {
            let sf = get_ref_scale_factors_const(cm, rf[1]);
            av1_setup_pred_block(
                &x.e_mbd,
                &mut yv12_mb[rf[1] as usize],
                yv12,
                yv12_idx,
                sf,
                sf,
                num_planes,
            );
        }
    }
    let ref_frame_comp = av1_ref_frame_type(rf);
    let rfc = ref_frame_comp as usize;
    x.mbmi_ext.mode_context[rfc] = 0;
    x.mbmi_ext.ref_mv_count[rfc] = u8::MAX;
    find_mv_refs(cm, x, ref_frame_comp);
    av1_copy_usable_ref_mv_stack_and_weight(&x.e_mbd, &mut x.mbmi_ext, ref_frame_comp);
    *ref_mv_idx = x.e_mbd.mbmi().ref_mv_idx as i32 + 1;
}

fn set_compound_mode(
    x: &mut Macroblock,
    ref_frame: MvReferenceFrame,
    ref_frame2: MvReferenceFrame,
    ref_mv_idx: i32,
    frame_mv: &mut [[IntMv; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    this_mode: PredictionMode,
) {
    let xd = &mut x.e_mbd;
    let ref_frame_comp;
    {
        let mi = xd.mbmi_mut();
        mi.ref_frame[0] = ref_frame;
        mi.ref_frame[1] = ref_frame2;
        mi.compound_idx = 1;
        mi.comp_group_idx = 0;
        mi.interinter_comp.type_ = COMPOUND_AVERAGE;
        ref_frame_comp = av1_ref_frame_type(&mi.ref_frame) as usize;
    }
    let (m, r, r2) = (this_mode as usize, ref_frame as usize, ref_frame2 as usize);
    if this_mode == GLOBAL_GLOBALMV {
        frame_mv[m][r].as_int = 0;
        frame_mv[m][r2].as_int = 0;
    } else if this_mode == NEAREST_NEARESTMV {
        frame_mv[m][r].as_int = xd.ref_mv_stack[ref_frame_comp][0].this_mv.as_int;
        frame_mv[m][r2].as_int = xd.ref_mv_stack[ref_frame_comp][0].comp_mv.as_int;
    } else if this_mode == NEAR_NEARMV {
        let i = ref_mv_idx as usize;
        frame_mv[m][r].as_int = xd.ref_mv_stack[ref_frame_comp][i].this_mv.as_int;
        frame_mv[m][r2].as_int = xd.ref_mv_stack[ref_frame_comp][i].comp_mv.as_int;
    }
}

// Prune compound mode if the single mode variance is lower than a fixed
// percentage of the median value.
fn skip_comp_based_on_var(
    single_vars: &[[u32; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    bsize: BlockSize,
) -> bool {
    let mut best_var = u32::MAX;
    for cur_mode_idx in 0..RTC_INTER_MODES as usize {
        for ref_idx in 0..REF_FRAMES as usize {
            best_var = best_var.min(single_vars[cur_mode_idx][ref_idx]);
        }
    }
    let thresh_64 = (0.57356805f32 * 8659.0f32) as u32;
    let thresh_32 = (0.23964763f32 * 4281.0f32) as u32;

    // Currently, the thresh for 128 and 16 are not well-tuned. We are using the
    // results from 64 and 32 as an heuristic.
    match bsize {
        BLOCK_128X128 => best_var < 4 * thresh_64,
        BLOCK_64X64 => best_var < thresh_64,
        BLOCK_32X32 => best_var < thresh_32,
        BLOCK_16X16 => best_var < thresh_32 / 4,
        _ => false,
    }
}

#[inline]
fn fill_single_inter_mode_costs(
    single_inter_mode_costs: &mut [[i32; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    num_inter_modes: usize,
    reference_mode_set: &[RefMode],
    mode_costs: &ModeCosts,
    mode_context: &[i16],
) {
    let mut ref_frame_used = [false; REF_FRAMES as usize];
    for idx in 0..num_inter_modes {
        ref_frame_used[reference_mode_set[idx].ref_frame as usize] = true;
    }

    for this_ref_frame in LAST_FRAME..REF_FRAMES as MvReferenceFrame {
        if !ref_frame_used[this_ref_frame as usize] {
            continue;
        }

        let rf: [MvReferenceFrame; 2] = [this_ref_frame, NONE_FRAME];
        let mode_ctx = av1_mode_context_analyzer(mode_context, &rf);
        for this_mode in NEARESTMV..=NEWMV {
            single_inter_mode_costs[inter_offset(this_mode)][this_ref_frame as usize] =
                cost_mv_ref(mode_costs, this_mode, mode_ctx);
        }
    }
}

#[inline]
fn is_globalmv_better(
    this_mode: PredictionMode,
    ref_frame: MvReferenceFrame,
    rate_mv: i32,
    mode_costs: &ModeCosts,
    single_inter_mode_costs: &[[i32; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    mbmi_ext: &MbModeInfoExt,
) -> bool {
    let globalmv_mode_cost = single_inter_mode_costs[inter_offset(GLOBALMV)][ref_frame as usize];
    let mut this_mode_cost =
        rate_mv + single_inter_mode_costs[inter_offset(this_mode)][ref_frame as usize];
    if this_mode == NEWMV || this_mode == NEARMV {
        let rf: [MvReferenceFrame; 2] = [ref_frame, NONE_FRAME];
        this_mode_cost += get_drl_cost(
            NEWMV,
            0,
            mbmi_ext,
            &mode_costs.drl_mode_cost0,
            av1_ref_frame_type(&rf) as i8,
        );
    }
    this_mode_cost > globalmv_mode_cost
}

// Set up the mv/ref_frames etc based on the comp_index. Returns 1 if it
// succeeds, 0 if it fails.
#[inline]
fn setup_compound_params_from_comp_idx(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    yv12_mb: &mut [[Buf2d; MAX_MB_PLANE]; REF_FRAMES as usize],
    this_mode: &mut PredictionMode,
    ref_frame: &mut MvReferenceFrame,
    ref_frame2: &mut MvReferenceFrame,
    frame_mv: &mut [[IntMv; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    use_ref_frame_mask: &[i32; REF_FRAMES as usize],
    comp_index: usize,
    comp_use_zero_zeromv_only: bool,
    last_comp_ref_frame: &mut MvReferenceFrame,
) -> bool {
    let rf = COMP_REF_MODE_SET[comp_index].ref_frame;
    *this_mode = COMP_REF_MODE_SET[comp_index].pred_mode;
    *ref_frame = rf[0];
    *ref_frame2 = rf[1];
    debug_assert!(*ref_frame == LAST_FRAME);
    debug_assert!(*this_mode == GLOBAL_GLOBALMV || *this_mode == NEAREST_NEARESTMV);
    if comp_use_zero_zeromv_only && *this_mode != GLOBAL_GLOBALMV {
        return false;
    }
    if *ref_frame2 == GOLDEN_FRAME
        && (cpi.sf.rt_sf.ref_frame_comp_nonrd[0] == 0 || (cpi.ref_frame_flags & AOM_GOLD_FLAG) == 0)
    {
        return false;
    } else if *ref_frame2 == LAST2_FRAME
        && (cpi.sf.rt_sf.ref_frame_comp_nonrd[1] == 0
            || (cpi.ref_frame_flags & AOM_LAST2_FLAG) == 0)
    {
        return false;
    } else if *ref_frame2 == ALTREF_FRAME
        && (cpi.sf.rt_sf.ref_frame_comp_nonrd[2] == 0 || (cpi.ref_frame_flags & AOM_ALT_FLAG) == 0)
    {
        return false;
    }
    let mut ref_mv_idx = 0;
    if *last_comp_ref_frame != rf[1] {
        // Only needs to be done once per reference pair.
        setup_compound_prediction(
            &cpi.common,
            x,
            yv12_mb,
            use_ref_frame_mask,
            &rf,
            &mut ref_mv_idx,
        );
        *last_comp_ref_frame = rf[1];
    }
    set_compound_mode(x, *ref_frame, *ref_frame2, ref_mv_idx, frame_mv, *this_mode);
    if *this_mode != GLOBAL_GLOBALMV
        && frame_mv[*this_mode as usize][*ref_frame as usize].as_int == 0
        && frame_mv[*this_mode as usize][*ref_frame2 as usize].as_int == 0
    {
        return false;
    }

    true
}

#[inline]
fn previous_mode_performed_poorly(
    mode: PredictionMode,
    ref_frame: MvReferenceFrame,
    vars: &[[u32; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    uv_dist: &[[i64; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
) -> bool {
    let rf = ref_frame as usize;
    let mut best_var = u32::MAX;
    let mut best_uv_dist = i64::MAX;
    for midx in 0..RTC_INTER_MODES as usize {
        best_var = best_var.min(vars[midx][rf]);
        best_uv_dist = best_uv_dist.min(uv_dist[midx][rf]);
    }
    debug_assert!(best_var != u32::MAX, "Invalid variance data.");
    let mult = 1.125f32;
    let mut var_bad = mult * (best_var as f32) < vars[inter_offset(mode)][rf] as f32;
    if uv_dist[inter_offset(mode)][rf] < i64::MAX && best_uv_dist != uv_dist[inter_offset(mode)][rf]
    {
        // If we have chroma info, then take it into account
        var_bad &= mult * (best_uv_dist as f32) < uv_dist[inter_offset(mode)][rf] as f32;
    }
    var_bad
}

#[inline]
fn prune_compoundmode_with_singlemode_var(
    compound_mode: PredictionMode,
    ref_frame: MvReferenceFrame,
    ref_frame2: MvReferenceFrame,
    frame_mv: &[[IntMv; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    mode_checked: &[[u8; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    vars: &[[u32; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
    uv_dist: &[[i64; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
) -> bool {
    let single_mode0 = compound_ref0_mode(compound_mode);
    let single_mode1 = compound_ref1_mode(compound_mode);
    let (m0, m1, cm_) = (
        single_mode0 as usize,
        single_mode1 as usize,
        compound_mode as usize,
    );
    let (r, r2) = (ref_frame as usize, ref_frame2 as usize);

    let mut first_ref_valid = false;
    let mut second_ref_valid = false;
    let mut first_ref_bad = false;
    let mut second_ref_bad = false;
    if mode_checked[m0][r] != 0
        && frame_mv[m0][r].as_int == frame_mv[cm_][r].as_int
        && vars[inter_offset(single_mode0)][r] < u32::MAX
    {
        first_ref_valid = true;
        first_ref_bad = previous_mode_performed_poorly(single_mode0, ref_frame, vars, uv_dist);
    }
    if mode_checked[m1][r2] != 0
        && frame_mv[m1][r2].as_int == frame_mv[cm_][r2].as_int
        && vars[inter_offset(single_mode1)][r2] < u32::MAX
    {
        second_ref_valid = true;
        second_ref_bad = previous_mode_performed_poorly(single_mode1, ref_frame2, vars, uv_dist);
    }
    if first_ref_valid && second_ref_valid {
        return first_ref_bad && second_ref_bad;
    } else if first_ref_valid || second_ref_valid {
        return first_ref_bad || second_ref_bad;
    }
    false
}

/// `av1_find_mv_refs()` of the block for `ref_frame` into `xd->ref_mv_stack`,
/// `xd->weight` and `x->mbmi_ext`.
#[inline]
fn find_mv_refs(cm: &Av1Common, x: &mut Macroblock, ref_frame: MvReferenceFrame) {
    let rf = ref_frame as usize;
    let Macroblock {
        e_mbd: xd,
        mbmi_ext,
        ..
    } = x;
    let bsize = xd.mbmi().bsize;
    let mut stack = xd.ref_mv_stack[rf];
    let mut weight = xd.weight[rf];
    av1_find_mv_refs(
        cm,
        xd,
        bsize,
        ref_frame,
        &mut mbmi_ext.ref_mv_count,
        &mut stack,
        &mut weight,
        Some(&mut mbmi_ext.global_mvs),
        &mut mbmi_ext.mode_context,
    );
    xd.ref_mv_stack[rf] = stack;
    xd.weight[rf] = weight;
}

/// Translation of `av1_copy_usable_ref_mv_stack_and_weight()` (rdopt_utils.h).
#[inline]
fn av1_copy_usable_ref_mv_stack_and_weight(
    xd: &MacroBlockD,
    mbmi_ext: &mut MbModeInfoExt,
    ref_frame: MvReferenceFrame,
) {
    let rf = ref_frame as usize;
    let n = USABLE_REF_MV_STACK_SIZE as usize;
    mbmi_ext.weight[rf].copy_from_slice(&xd.weight[rf][..n]);
    mbmi_ext.ref_mv_stack[rf].copy_from_slice(&xd.ref_mv_stack[rf][..n]);
}

// (nonrd_opt.h)

// Indicates the blocks for which RD model should be based on special logic
#[inline]
fn get_model_rd_flag(cpi: &Av1Comp, xd: &MacroBlockD, bsize: BlockSize) -> bool {
    let large_block = bsize >= BLOCK_32X32;
    let cm = &cpi.common;
    cpi.oxcf.rc_cfg.mode == AOM_CBR
        && large_block
        && !cyclic_refresh_segment_id_boosted(xd.mbmi().segment_id as i32)
        && cm.quant_params.base_qindex != 0
        && cm.seq_params.bit_depth == AOM_BITS_8
}

/*\brief Finds predicted motion vectors for a block.
 *
 * \ingroup nonrd_mode_search
 * \callgraph
 * \callergraph
 * Finds predicted motion vectors for a block from a certain reference frame.
 * First, it fills reference MV stack, then picks the test from the stack and
 * predicts the final MV for a block for each mode.
 * \param[in]    cpi                      Top-level encoder structure
 * \param[in]    x                        Pointer to structure holding all the
 *                                        data for the current macroblock
 * \param[in]    ref_frame                Reference frame for which to find
 *                                        ref MVs
 * \param[in]    frame_mv                 Predicted MVs for a block
 * \param[in]    tile_data                Pointer to struct holding adaptive
 *                                        data/contexts/models for the tile
 *                                        during encoding
 * \param[in]    yv12_mb                  Buffer to hold predicted block
 * \param[in]    bsize                    Current block size
 * \param[in]    force_skip_low_temp_var  Flag indicating possible mode search
 *                                        prune for low temporal variance block
 * \param[in]    skip_pred_mv             Flag indicating to skip av1_mv_pred
 *
 * \remark Nothing is returned. Instead, predicted MVs are placed into
 * \c frame_mv array
 */
#[inline]
fn find_predictors(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    ref_frame: MvReferenceFrame,
    frame_mv: &mut [[IntMv; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    yv12_mb: &mut [[Buf2d; MAX_MB_PLANE]; REF_FRAMES as usize],
    bsize: BlockSize,
    force_skip_low_temp_var: bool,
    skip_pred_mv: bool,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let rf = ref_frame as usize;
    let yv12 = get_ref_frame_yv12_buf(cm, ref_frame);
    let num_planes = av1_num_planes(cm);

    x.pred_mv_sad[rf] = i32::MAX;
    x.pred_mv0_sad[rf] = i32::MAX;
    x.pred_mv1_sad[rf] = i32::MAX;
    frame_mv[NEWMV as usize][rf].as_int = INVALID_MV;
    // TODO(kyslov) this needs various further optimizations. to be continued..
    debug_assert!(yv12.is_some());
    if let Some((yv12, yv12_idx)) = yv12 {
        let sf = get_ref_scale_factors_const(cm, ref_frame);
        av1_setup_pred_block(
            &x.e_mbd,
            &mut yv12_mb[rf],
            yv12,
            yv12_idx,
            sf,
            sf,
            num_planes,
        );
        find_mv_refs(cm, x, ref_frame);
        // TODO(Ravi): Populate mbmi_ext->ref_mv_stack[ref_frame][4] and
        // mbmi_ext->weight[ref_frame][4] inside av1_find_mv_refs.
        av1_copy_usable_ref_mv_stack_and_weight(&x.e_mbd, &mut x.mbmi_ext, ref_frame);
        let mut nearest = frame_mv[NEARESTMV as usize][rf];
        let mut near = frame_mv[NEARMV as usize][rf];
        av1_find_best_ref_mvs_from_stack(
            cm.features.allow_high_precision_mv,
            &x.mbmi_ext,
            ref_frame,
            &mut nearest,
            &mut near,
            false,
        );
        frame_mv[NEARESTMV as usize][rf] = nearest;
        frame_mv[NEARMV as usize][rf] = near;
        frame_mv[GLOBALMV as usize][rf] = x.mbmi_ext.global_mvs[rf];
        // Early exit for non-LAST frame if force_skip_low_temp_var is set.
        let Some(sf) = sf else {
            return Err(());
        };
        if !av1_is_scaled(sf)
            && bsize >= BLOCK_8X8
            && !skip_pred_mv
            && !(force_skip_low_temp_var && ref_frame != LAST_FRAME)
        {
            let pre = yv12_mb[rf][0];
            av1_mv_pred(
                cpi,
                x,
                (&yv12.buffer_alloc, pre.buf),
                yv12.y_stride(),
                ref_frame,
                bsize,
            );
        }
    }
    if cm.features.switchable_motion_mode {
        av1_count_overlappable_neighbors(cm, &mut x.e_mbd);
    }
    x.e_mbd.mbmi_mut().num_proj_ref = 1;
    Ok(())
}

// Function to setup parameters used for inter mode evaluation.
#[inline]
fn set_params_nonrd_pick_inter_mode(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    search_state: &mut InterModeSearchStateNonrd,
    rd_cost: &mut RdStats,
    force_skip_low_temp_var: &mut i32,
    skip_pred_mv: &mut bool,
    mi_row: i32,
    mi_col: i32,
    gf_temporal_ref: bool,
    segment_id: u8,
    bsize: BlockSize,
) -> Result<(), ()> {
    let cm = &cpi.common;

    for idx in 0..RTC_INTER_MODES as usize {
        for ref_ in 0..REF_FRAMES as usize {
            search_state.vars[idx][ref_] = u32::MAX;
            search_state.uv_dist[idx][ref_] = i64::MAX;
        }
    }

    x.color_sensitivity[0] = x.color_sensitivity_sb[0];
    x.color_sensitivity[1] = x.color_sensitivity_sb[1];
    search_state.best_pickmode = init_best_pickmode();

    estimate_single_ref_frame_costs(
        cm,
        &x.e_mbd,
        &x.mode_costs,
        segment_id,
        bsize,
        &mut search_state.ref_costs_single,
    );

    search_state.mode_checked = [[0; REF_FRAMES as usize]; MB_MODE_COUNT as usize];

    x.txfm_search_info.skip_txfm = 0;

    // initialize mode decisions
    av1_invalid_rd_stats(&mut search_state.best_rdc);
    av1_invalid_rd_stats(&mut search_state.this_rdc);
    av1_invalid_rd_stats(rd_cost);
    for i in 0..REF_FRAMES as usize {
        x.warp_sample_info[i].num = -1;
    }

    {
        let mi = x.e_mbd.mbmi_mut();
        mi.bsize = bsize;
        mi.ref_frame[0] = NONE_FRAME;
        mi.ref_frame[1] = NONE_FRAME;
    }

    // (CONFIG_AV1_TEMPORAL_DENOISING is off)

    if cpi.ref_frame_flags & AOM_LAST_FLAG != 0 {
        let skip = x.force_zeromv_skip_for_blk != 0;
        find_predictors(
            cpi,
            x,
            LAST_FRAME,
            &mut search_state.frame_mv,
            &mut search_state.yv12_mb,
            bsize,
            *force_skip_low_temp_var != 0,
            skip,
        )?;
    }

    get_ref_frame_use_mask(
        cpi,
        x,
        mi_row,
        mi_col,
        bsize,
        gf_temporal_ref,
        &mut search_state.use_ref_frame_mask,
        force_skip_low_temp_var,
    )?;

    *skip_pred_mv = x.force_zeromv_skip_for_blk != 0
        || (x.nonrd_prune_ref_frame_search > 2
            && x.color_sensitivity[0] != 2
            && x.color_sensitivity[1] != 2);

    // Start at LAST_FRAME + 1.
    for ref_frame_iter in LAST_FRAME + 1..=ALTREF_FRAME {
        if search_state.use_ref_frame_mask[ref_frame_iter as usize] != 0 {
            find_predictors(
                cpi,
                x,
                ref_frame_iter,
                &mut search_state.frame_mv,
                &mut search_state.yv12_mb,
                bsize,
                *force_skip_low_temp_var != 0,
                *skip_pred_mv,
            )?;
        }
    }
    Ok(())
}

// Function to check the inter mode can be skipped based on mode statistics and
// speed features settings.
#[inline]
fn skip_inter_mode_nonrd(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    search_state: &mut InterModeSearchStateNonrd,
    thresh_sad_pred: &mut i64,
    force_mv_inter_layer: &mut i32,
    comp_pred: &mut bool,
    this_mode: &mut PredictionMode,
    last_comp_ref_frame: &mut MvReferenceFrame,
    ref_frame: &mut MvReferenceFrame,
    ref_frame2: &mut MvReferenceFrame,
    idx: usize,
    force_skip_low_temp_var: i32,
    sse_zeromv_norm: u32,
    num_inter_modes: usize,
    segment_id: u8,
    bsize: BlockSize,
    comp_use_zero_zeromv_only: bool,
    check_globalmv: bool,
) -> Result<bool, ()> {
    let cm = &cpi.common;
    let seg = &cm.seg;

    if idx >= num_inter_modes {
        let comp_index = idx - num_inter_modes;
        let use_ref_frame_mask = search_state.use_ref_frame_mask;
        if !setup_compound_params_from_comp_idx(
            cpi,
            x,
            &mut search_state.yv12_mb,
            this_mode,
            ref_frame,
            ref_frame2,
            &mut search_state.frame_mv,
            &use_ref_frame_mask,
            comp_index,
            comp_use_zero_zeromv_only,
            last_comp_ref_frame,
        ) {
            return Ok(true);
        }
        *comp_pred = true;
    } else {
        *this_mode = REF_MODE_SET[idx].pred_mode;
        *ref_frame = REF_MODE_SET[idx].ref_frame;
        *ref_frame2 = NONE_FRAME;
    }
    let (m, r) = (*this_mode as usize, *ref_frame as usize);

    if !*comp_pred && search_state.mode_checked[m][r] != 0 {
        return Ok(true);
    }

    if !check_globalmv && *this_mode == GLOBALMV {
        return Ok(true);
    }

    // (COLLECT_PICK_MODE_STAT is 0)
    {
        let mi = x.e_mbd.mbmi_mut();
        mi.mode = *this_mode;
        mi.ref_frame[0] = *ref_frame;
        mi.ref_frame[1] = *ref_frame2;
    }

    if search_state.use_ref_frame_mask[r] == 0 {
        return Ok(true);
    }

    if x.force_zeromv_skip_for_blk != 0
        && ((!(*this_mode == NEARESTMV && search_state.frame_mv[m][r].as_int == 0)
            && *this_mode != GLOBALMV)
            || *ref_frame != LAST_FRAME)
    {
        return Ok(true);
    }

    if cpi.sf.rt_sf.prune_compoundmode_with_singlemode_var
        && *comp_pred
        && prune_compoundmode_with_singlemode_var(
            *this_mode,
            *ref_frame,
            *ref_frame2,
            &search_state.frame_mv,
            &search_state.mode_checked,
            &search_state.vars,
            &search_state.uv_dist,
        )
    {
        return Ok(true);
    }

    *force_mv_inter_layer = 0;
    if cpi.ppi.use_svc && cpi.svc.spatial_layer_id > 0 {
        // (spatial layers are never coded)
        return Err(());
    }

    // If the segment reference frame feature is enabled then do nothing if the
    // current ref frame is not allowed.
    if segfeature_active(seg, segment_id, SEG_LVL_REF_FRAME)
        && get_segdata(seg, segment_id as i32, SEG_LVL_REF_FRAME) != *ref_frame as i32
    {
        return Ok(true);
    }

    // For screen content: for base spatial layer only for now.
    if cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN && cpi.svc.spatial_layer_id == 0 {
        // If source_sad is computed: skip non-zero motion
        // check for stationary (super)blocks. Otherwise if superblock
        // has motion skip the modes with zero motion for flat blocks,
        // and color is not set.
        // For the latter condition: the same condition should apply
        // to newmv if (0, 0), so this latter condition is repeated
        // below after search_new_mv.
        if cpi.sf.rt_sf.source_metrics_sb_nonrd != 0 {
            if (search_state.frame_mv[m][r].as_int != 0
                && x.content_state_sb.source_sad_nonrd == K_ZERO_SAD)
                || (search_state.frame_mv[m][r].as_int == 0
                    && x.content_state_sb.source_sad_nonrd != K_ZERO_SAD
                    && ((x.color_sensitivity[0] == 0 && x.color_sensitivity[1] == 0)
                        || cpi.rc.high_source_sad != 0)
                    && x.source_variance == 0)
            {
                return Ok(true);
            }
        }
        // Skip NEWMV search for flat blocks.
        if *this_mode == NEWMV && x.source_variance < 100 {
            return Ok(true);
        }
        // Skip non-LAST for color on flat blocks.
        if *ref_frame > LAST_FRAME
            && x.source_variance == 0
            && (x.color_sensitivity[0] == 1 || x.color_sensitivity[1] == 1)
        {
            return Ok(true);
        }
    }

    if skip_mode_by_bsize_and_ref_frame(
        *this_mode,
        *ref_frame,
        bsize,
        x.nonrd_prune_ref_frame_search,
        sse_zeromv_norm,
        cpi.sf.rt_sf.nonrd_aggressive_skip,
    ) {
        return Ok(true);
    }

    if skip_mode_by_low_temp(
        *this_mode,
        *ref_frame,
        bsize,
        x.content_state_sb,
        search_state.frame_mv[m][r],
        force_skip_low_temp_var,
    ) {
        return Ok(true);
    }

    // Disable this drop out case if the ref frame segment level feature is
    // enabled for this segment. This is to prevent the possibility that we
    // end up unable to pick any mode.
    if !segfeature_active(seg, segment_id, SEG_LVL_REF_FRAME) {
        // Check for skipping GOLDEN and ALTREF based pred_mv_sad.
        if cpi.sf.rt_sf.nonrd_prune_ref_frame_search > 0
            && x.pred_mv_sad[r] != i32::MAX
            && *ref_frame != LAST_FRAME
        {
            if (x.pred_mv_sad[r] as i64) > *thresh_sad_pred {
                return Ok(true);
            }
        }
    }

    // Check for skipping NEARMV based on pred_mv_sad.
    if *this_mode == NEARMV
        && x.pred_mv1_sad[r] != i32::MAX
        && x.pred_mv1_sad[r] > (x.pred_mv0_sad[r] << 1)
    {
        return Ok(true);
    }

    if !*comp_pred {
        if skip_mode_by_threshold(
            *this_mode,
            *ref_frame,
            search_state.frame_mv[m][r],
            cpi.rc.frames_since_golden,
            &cpi.rd.threshes[segment_id as usize][bsize as usize],
            &x.thresh_freq_fact[bsize as usize],
            search_state.best_rdc.rdcost,
            search_state.best_pickmode.best_mode_skip_txfm,
            if cpi.sf.rt_sf.nonrd_aggressive_skip != 0 {
                1
            } else {
                0
            },
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Translation of `av1_nonrd_pick_inter_mode_sb()`. `Err` for the
/// prediction reuse (`reuse_inter_pred_nonrd`), the screen content tools
/// (identity transforms and palettes) and the spatial layers, none of
/// which the real-time configuration reaches.
pub(crate) fn av1_nonrd_pick_inter_mode_sb(
    cpi: &Av1Comp,
    tile_data: &super::encoder::TileDataEnc,
    x: &mut Macroblock,
    rd_cost: &mut RdStats,
    bsize: BlockSize,
    ctx: &mut PickModeContext,
) -> Result<(), ()> {
    let _ = tile_data;
    let cm = &cpi.common;
    let filter_ref = cm.features.interp_filter;
    let default_interp_filter = EIGHTTAP_REGULAR;
    let mut ref_frame: MvReferenceFrame = NONE_FRAME;
    let mut ref_frame2: MvReferenceFrame = NONE_FRAME;
    let segment_id = x.e_mbd.mbmi().segment_id;
    let mut best_early_term = 0;
    let mut force_skip_low_temp_var = 0;
    let mut sse_zeromv_norm = u32::MAX;
    let mut skip_pred_mv = false;
    let num_inter_modes = NUM_INTER_MODES;
    let mut check_globalmv = cpi.sf.rt_sf.check_globalmv_on_single_ref;
    let reuse_inter_pred =
        cpi.sf.rt_sf.reuse_inter_pred_nonrd != 0 && cm.seq_params.bit_depth == AOM_BITS_8;
    let mut search_state = InterModeSearchStateNonrd {
        best_pickmode: init_best_pickmode(),
        this_rdc: RdStats::default(),
        best_rdc: RdStats::default(),
        uv_dist: [[0; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
        yv12_mb: [[Buf2d::default(); MAX_MB_PLANE]; REF_FRAMES as usize],
        vars: [[0; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
        ref_costs_single: [0; REF_FRAMES as usize],
        frame_mv: [[IntMv::default(); REF_FRAMES as usize]; MB_MODE_COUNT as usize],
        frame_mv_best: [[IntMv::default(); REF_FRAMES as usize]; MB_MODE_COUNT as usize],
        single_inter_mode_costs: [[0; REF_FRAMES as usize]; RTC_INTER_MODES as usize],
        use_ref_frame_mask: [0; REF_FRAMES as usize],
        mode_checked: [[0; REF_FRAMES as usize]; MB_MODE_COUNT as usize],
    };

    let num_8x8_blocks = (ctx.num_4x4_blk / 4) as usize;
    let orig_dst = x.e_mbd.plane[0].dst;
    let mut thresh_sad_pred = i64::MAX;
    let mi_row = x.e_mbd.mi_row;
    let mi_col = x.e_mbd.mi_col;
    let mut force_mv_inter_layer = 0;
    let mut comp_use_zero_zeromv_only = false;
    let mut tot_num_comp_modes = NUM_COMP_INTER_MODES_RT;
    // (CONFIG_AV1_TEMPORAL_DENOISING is off)

    if reuse_inter_pred {
        // (the prediction buffers)
        return Err(());
    }

    let gf_temporal_ref = is_same_gf_and_last_scale(cm);

    // If the lower spatial layer uses an averaging filter for downsampling
    // (phase = 8), the target decimated pixel is shifted by (1/2, 1/2) relative
    // to source, so use subpel motion vector to compensate. The nonzero motion
    // is half pixel shifted to left and top, so (-4, -4). This has more effect
    // on higher resolutions, so condition it on that for now.
    if cpi.ppi.use_svc && cpi.svc.spatial_layer_id > 0 {
        // (spatial layers are never coded)
        return Err(());
    }

    // Setup parameters used for inter mode evaluation.
    set_params_nonrd_pick_inter_mode(
        cpi,
        x,
        &mut search_state,
        rd_cost,
        &mut force_skip_low_temp_var,
        &mut skip_pred_mv,
        mi_row,
        mi_col,
        gf_temporal_ref,
        segment_id,
        bsize,
    )?;

    if cpi.sf.rt_sf.use_comp_ref_nonrd != 0 && is_comp_ref_allowed(bsize) {
        // Only search compound if bsize \gt BLOCK_16X16.
        if bsize > BLOCK_16X16 {
            comp_use_zero_zeromv_only = cpi.sf.rt_sf.check_only_zero_zeromv_on_large_blocks;
        } else {
            tot_num_comp_modes = 0;
        }
    } else {
        tot_num_comp_modes = 0;
    }

    if x.pred_mv_sad[LAST_FRAME as usize] != i32::MAX {
        thresh_sad_pred = (x.pred_mv_sad[LAST_FRAME as usize] as i64) << 1;
        // Increase threshold for less aggressive pruning.
        if cpi.sf.rt_sf.nonrd_prune_ref_frame_search == 1 {
            thresh_sad_pred += (x.pred_mv_sad[LAST_FRAME as usize] >> 2) as i64;
        }
    }

    let use_model_yrd_large = get_model_rd_flag(cpi, &x.e_mbd, bsize);

    // decide block-level interp filter search flags:
    // filter_search_enabled_blk:
    // 0: disabled
    // 1: filter search depends on mode properties
    // 2: filter search forced since prediction is unreliable
    // cb_pred_filter_search 0: disabled cb prediction
    let mut filt_select = EIGHTTAP_REGULAR;
    let cb_pred_filter_search = if x.content_state_sb.source_sad_nonrd > K_VERY_LOW_SAD {
        cpi.sf.interp_sf.cb_pred_filter_search
    } else {
        0
    };
    let filter_search_enabled_blk = is_filter_search_enabled_blk(
        cpi,
        x,
        mi_row,
        mi_col,
        bsize,
        segment_id,
        cb_pred_filter_search,
        &mut filt_select,
    );

    // (COLLECT_PICK_MODE_STAT is 0)
    init_mbmi(x.e_mbd.mbmi_mut(), DC_PRED, NONE_FRAME, NONE_FRAME, cm);
    x.e_mbd.mbmi_mut().tx_size = MAX_TXSIZE_LOOKUP[bsize as usize]
        .min(TX_MODE_TO_BIGGEST_TX_SIZE[x.txfm_search_params.tx_mode_search_type as usize])
        .min(TX_16X16);

    fill_single_inter_mode_costs(
        &mut search_state.single_inter_mode_costs,
        num_inter_modes,
        &REF_MODE_SET,
        &x.mode_costs,
        &x.mbmi_ext.mode_context,
    );

    let mut last_comp_ref_frame = NONE_FRAME;

    // Initialize inter prediction params at block level for single reference
    // mode.
    let mut inter_pred_params_sr = InterPredParams::default();
    {
        let xd = &x.e_mbd;
        let pd = &xd.plane[0];
        init_inter_block_params(
            &mut inter_pred_params_sr,
            pd.width as i32,
            pd.height as i32,
            mi_row * MI_SIZE,
            mi_col * MI_SIZE,
            pd.subsampling_x,
            pd.subsampling_y,
            xd.bd,
            is_cur_buf_hbd(xd),
            /*is_intrabc=*/ false,
        );
        inter_pred_params_sr.conv_params =
            get_conv_params(/*do_average=*/ 0, AOM_PLANE_Y as i32, xd.bd);
    }

    for idx in 0..num_inter_modes + tot_num_comp_modes {
        // If we are at the first compound mode, and the single modes already
        // perform well, then end the search.
        if cpi.sf.rt_sf.skip_compound_based_on_var
            && idx == num_inter_modes
            && skip_comp_based_on_var(&search_state.vars, bsize)
        {
            break;
        }

        let mut rate_mv = 0;
        let mut is_skippable = 0;
        let mut this_early_term = 0;
        let mut skip_this_mv = false;
        let mut comp_pred = false;
        let mut var = u32::MAX;
        let mut this_mode: PredictionMode = DC_PRED;
        let mut nonskip_rdc = RdStats::default();
        av1_invalid_rd_stats(&mut nonskip_rdc);
        x.txfm_search_info.blk_skip[..num_8x8_blocks].fill(0);

        // Check the inter mode can be skipped based on mode statistics and speed
        // features settings.
        if skip_inter_mode_nonrd(
            cpi,
            x,
            &mut search_state,
            &mut thresh_sad_pred,
            &mut force_mv_inter_layer,
            &mut comp_pred,
            &mut this_mode,
            &mut last_comp_ref_frame,
            &mut ref_frame,
            &mut ref_frame2,
            idx,
            force_skip_low_temp_var,
            sse_zeromv_norm,
            num_inter_modes,
            segment_id,
            bsize,
            comp_use_zero_zeromv_only,
            check_globalmv,
        )? {
            continue;
        }
        let (m, r) = (this_mode as usize, ref_frame as usize);

        // Select prediction reference frames.
        for i in 0..MAX_MB_PLANE {
            x.e_mbd.plane[i].pre[0] = search_state.yv12_mb[r][i];
            if comp_pred {
                x.e_mbd.plane[i].pre[1] = search_state.yv12_mb[ref_frame2 as usize][i];
            }
        }

        {
            let mi = x.e_mbd.mbmi_mut();
            mi.ref_frame[0] = ref_frame;
            mi.ref_frame[1] = ref_frame2;
        }
        set_ref_ptrs(cm, &mut x.e_mbd, ref_frame, ref_frame2)?;

        if this_mode == NEWMV && force_mv_inter_layer == 0 {
            // (COLLECT_PICK_MODE_STAT is 0)
            let best_rdc = search_state.best_rdc;
            let skip_newmv = search_new_mv(
                cpi,
                x,
                &mut search_state.frame_mv,
                ref_frame,
                gf_temporal_ref,
                bsize,
                mi_row,
                mi_col,
                &mut rate_mv,
                &best_rdc,
            )?;
            if skip_newmv {
                continue;
            }
        }

        for inter_mv_mode in NEARESTMV..=NEWMV {
            if inter_mv_mode == this_mode {
                continue;
            }
            if !comp_pred
                && search_state.mode_checked[inter_mv_mode as usize][r] != 0
                && search_state.frame_mv[m][r].as_int
                    == search_state.frame_mv[inter_mv_mode as usize][r].as_int
            {
                skip_this_mv = true;
                break;
            }
        }

        if skip_this_mv && !comp_pred {
            continue;
        }

        // For screen: for spatially flat blocks with non-zero motion,
        // skip newmv if the motion vector is (0, 0), and color is not set.
        if this_mode == NEWMV
            && cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN
            && cpi.svc.spatial_layer_id == 0
            && cpi.sf.rt_sf.source_metrics_sb_nonrd != 0
        {
            if search_state.frame_mv[m][r].as_int == 0
                && x.content_state_sb.source_sad_nonrd != K_ZERO_SAD
                && ((x.color_sensitivity[0] == 0 && x.color_sensitivity[1] == 0)
                    || cpi.rc.high_source_sad != 0)
                && x.source_variance == 0
            {
                continue;
            }
        }

        {
            let mv0 = search_state.frame_mv[m][r];
            let mv1 = if comp_pred {
                search_state.frame_mv[m][ref_frame2 as usize]
            } else {
                IntMv { as_int: 0 }
            };
            let mi = x.e_mbd.mbmi_mut();
            mi.mode = this_mode;
            mi.mv[0].as_int = mv0.as_int;
            mi.mv[1].as_int = 0;
            if comp_pred {
                mi.mv[1].as_int = mv1.as_int;
            }
        }

        // (reuse_inter_pred is 0)

        if idx == 0 && !skip_pred_mv {
            // Set color sensitivity on first tested mode only.
            // Use y-sad already computed in find_predictors: take the sad with motion
            // vector closest to 0; the uv-sad computed below in set_color_sensitivity
            // is for zeromv.
            // For screen: first check if golden reference is being used, if so,
            // force color_sensitivity on if the color sensitivity for sb_g is on.
            if cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN
                && search_state.use_ref_frame_mask[GOLDEN_FRAME as usize] != 0
            {
                if x.color_sensitivity_sb_g[0] == 1 {
                    x.color_sensitivity[0] = 1;
                }
                if x.color_sensitivity_sb_g[1] == 1 {
                    x.color_sensitivity[1] = 1;
                }
            } else {
                let lf = LAST_FRAME as usize;
                let mut y_sad = x.pred_mv0_sad[lf];
                let near = search_state.frame_mv[NEARMV as usize][lf].as_mv();
                let nearest = search_state.frame_mv[NEARESTMV as usize][lf].as_mv();
                if x.pred_mv1_sad[lf] != i32::MAX
                    && ((near.col as i32).abs() + (near.row as i32).abs())
                        < ((nearest.col as i32).abs() + (nearest.row as i32).abs())
                {
                    y_sad = x.pred_mv1_sad[lf];
                }
                let source_variance = x.source_variance;
                let yv12_mb = search_state.yv12_mb[lf];
                set_color_sensitivity(cpi, x, bsize, y_sad, source_variance, &yv12_mb);
            }
        }
        x.e_mbd.mbmi_mut().motion_mode = SIMPLE_TRANSLATION;
        if cpi.oxcf.motion_mode_cfg.allow_warped_motion {
            calc_num_proj_ref(cpi, x);
        }
        // set variance threshold for compound more pruning
        let mut var_threshold = u32::MAX;
        if cpi.sf.rt_sf.prune_compoundmode_with_singlecompound_var
            && comp_pred
            && use_model_yrd_large
        {
            let single_mode0 = compound_ref0_mode(this_mode);
            let single_mode1 = compound_ref1_mode(this_mode);
            var_threshold = var_threshold.min(search_state.vars[inter_offset(single_mode0)][r]);
            var_threshold = var_threshold
                .min(search_state.vars[inter_offset(single_mode1)][ref_frame2 as usize]);
        }
        // decide interpolation filter, build prediction signal, get sse
        let mv0 = x.e_mbd.mbmi().mv[0].as_mv();
        let is_mv_subpel = (mv0.row & 0x07) != 0 || (mv0.col & 0x07) != 0;
        let enable_filt_search_this_mode = if filter_search_enabled_blk == 2 {
            true
        } else {
            filter_search_enabled_blk != 0
                && force_mv_inter_layer == 0
                && !comp_pred
                && (ref_frame == LAST_FRAME || x.nonrd_prune_ref_frame_search == 0)
        };
        if is_mv_subpel && enable_filt_search_this_mode {
            // (COLLECT_PICK_MODE_STAT is 0)
            let best_sse = search_state.best_pickmode.best_sse;
            search_filter_ref(
                cpi,
                x,
                &mut search_state.this_rdc,
                &mut inter_pred_params_sr,
                mi_row,
                mi_col,
                bsize,
                &mut this_early_term,
                &mut var,
                use_model_yrd_large,
                best_sse,
                comp_pred,
            )?;
        } else if cpi.oxcf.motion_mode_cfg.allow_warped_motion && this_mode == NEWMV {
            let best_sse = search_state.best_pickmode.best_sse;
            search_motion_mode(
                cpi,
                x,
                &mut search_state.this_rdc,
                mi_row,
                mi_col,
                bsize,
                &mut this_early_term,
                use_model_yrd_large,
                &mut rate_mv,
                best_sse,
            )?;
            if this_mode == NEWMV {
                search_state.frame_mv[m][r] = x.e_mbd.mbmi().mv[0];
            }
        } else {
            {
                let mi = x.e_mbd.mbmi_mut();
                mi.interp_filters = if filter_ref == SWITCHABLE {
                    av1_broadcast_interp_filter(default_interp_filter)
                } else {
                    av1_broadcast_interp_filter(filter_ref)
                };
                if force_mv_inter_layer != 0 {
                    mi.interp_filters = av1_broadcast_interp_filter(EIGHTTAP_REGULAR);
                }

                // If it is sub-pel motion and cb_pred_filter_search is enabled, select
                // the pre-decided filter
                if is_mv_subpel && cb_pred_filter_search != 0 {
                    mi.interp_filters = av1_broadcast_interp_filter(filt_select);
                }
            }

            // (COLLECT_PICK_MODE_STAT is 0)
            if !comp_pred {
                let mut subpel_params = SubpelParams::default();
                // Initialize inter mode level params for single reference mode.
                {
                    let xd = &x.e_mbd;
                    let pd = &xd.plane[0];
                    init_inter_mode_params(
                        &xd.mbmi().mv[0].as_mv(),
                        &mut inter_pred_params_sr,
                        &mut subpel_params,
                        &xd.block_ref_scale_factors[0],
                        pd.pre[0].width,
                        pd.pre[0].height,
                    );
                }
                av1_enc_build_inter_predictor_y_nonrd(
                    cm,
                    &mut x.e_mbd,
                    &mut inter_pred_params_sr,
                    &subpel_params,
                )?;
            } else {
                av1_enc_build_inter_predictor(cm, &mut x.e_mbd, mi_row, mi_col, None, bsize, 0, 0)?;
            }

            if use_model_yrd_large {
                let best_sse = search_state.best_pickmode.best_sse;
                model_skip_for_sb_y_large(
                    cpi,
                    bsize,
                    mi_row,
                    mi_col,
                    x,
                    &mut search_state.this_rdc,
                    &mut this_early_term,
                    0,
                    best_sse,
                    Some(&mut var),
                    var_threshold,
                )?;
            } else {
                model_rd_for_sb_y(
                    cpi,
                    bsize,
                    x,
                    &mut search_state.this_rdc,
                    Some(&mut var),
                    0,
                    Some(&mut this_early_term),
                );
            }
            // (COLLECT_PICK_MODE_STAT is 0)
        }
        // update variance for single mode
        if !comp_pred {
            search_state.vars[inter_offset(this_mode)][r] = var;
            if search_state.frame_mv[m][r].as_int == 0 {
                search_state.vars[inter_offset(GLOBALMV)][r] = var;
            }
        }
        // prune compound mode based on single mode var threshold
        if comp_pred && var > var_threshold {
            continue;
        }

        if ref_frame == LAST_FRAME && search_state.frame_mv[m][r].as_int == 0 {
            sse_zeromv_norm = (search_state.this_rdc.sse
                >> (B_WIDTH_LOG2_LOOKUP[bsize as usize] as i32
                    + B_HEIGHT_LOG2_LOOKUP[bsize as usize] as i32))
                as u32;
        }

        if cpi.sf.rt_sf.sse_early_term_inter_search != 0
            && early_term_inter_search_with_sse(
                cpi.sf.rt_sf.sse_early_term_inter_search,
                bsize,
                search_state.this_rdc.sse,
                search_state.best_pickmode.best_sse,
                this_mode,
            )
        {
            continue;
        }

        // (COLLECT_PICK_MODE_STAT is 0)

        let skip_ctx = av1_get_skip_txfm_context(&x.e_mbd) as usize;
        let skip_txfm_cost = x.mode_costs.skip_txfm_cost[skip_ctx][1];
        let no_skip_txfm_cost = x.mode_costs.skip_txfm_cost[skip_ctx][0];
        let sse_y = search_state.this_rdc.sse;
        if this_early_term != 0 {
            search_state.this_rdc.skip_txfm = 1;
            search_state.this_rdc.rate = skip_txfm_cost;
            search_state.this_rdc.dist = search_state.this_rdc.sse << 4;
        } else {
            // (COLLECT_PICK_MODE_STAT is 0)
            let tx_size = x.e_mbd.mbmi().tx_size;
            block_yrd_src(
                x,
                &mut search_state.this_rdc,
                &mut is_skippable,
                bsize,
                tx_size,
                1,
            )?;
            if search_state.this_rdc.skip_txfm != 0
                || rdcost(
                    x.rdmult,
                    search_state.this_rdc.rate,
                    search_state.this_rdc.dist,
                ) >= rdcost(x.rdmult, 0, search_state.this_rdc.sse)
            {
                if search_state.this_rdc.skip_txfm == 0 {
                    // Need to store "real" rdc for possible future use if UV rdc
                    // disallows tx skip
                    nonskip_rdc = search_state.this_rdc;
                    nonskip_rdc.rate += no_skip_txfm_cost;
                }
                search_state.this_rdc.rate = skip_txfm_cost;
                search_state.this_rdc.skip_txfm = 1;
                search_state.this_rdc.dist = search_state.this_rdc.sse;
            } else {
                search_state.this_rdc.rate += no_skip_txfm_cost;
            }
            if x.color_sensitivity[0] != 0 || x.color_sensitivity[1] != 0 {
                let mut rdc_uv = RdStats::default();
                let uv_bsize = get_plane_block_size(
                    bsize,
                    x.e_mbd.plane[1].subsampling_x,
                    x.e_mbd.plane[1].subsampling_y,
                );
                if x.color_sensitivity[0] != 0 {
                    av1_enc_build_inter_predictor(
                        cm,
                        &mut x.e_mbd,
                        mi_row,
                        mi_col,
                        None,
                        bsize,
                        AOM_PLANE_U,
                        AOM_PLANE_U,
                    )?;
                }
                if x.color_sensitivity[1] != 0 {
                    av1_enc_build_inter_predictor(
                        cm,
                        &mut x.e_mbd,
                        mi_row,
                        mi_col,
                        None,
                        bsize,
                        AOM_PLANE_V,
                        AOM_PLANE_V,
                    )?;
                }
                let sse_uv =
                    model_rd_for_sb_uv(cpi, uv_bsize, x, &x.source_alloc, &mut rdc_uv, 1, 2);
                search_state.this_rdc.sse += sse_uv;
                // Restore Y rdc if UV rdc disallows txfm skip
                if search_state.this_rdc.skip_txfm != 0
                    && rdc_uv.skip_txfm == 0
                    && nonskip_rdc.rate != i32::MAX
                {
                    search_state.this_rdc = nonskip_rdc;
                }
                if !comp_pred {
                    search_state.uv_dist[inter_offset(this_mode)][r] = rdc_uv.dist;
                }
                search_state.this_rdc.rate += rdc_uv.rate;
                search_state.this_rdc.dist += rdc_uv.dist;
                search_state.this_rdc.skip_txfm =
                    (search_state.this_rdc.skip_txfm != 0 && rdc_uv.skip_txfm != 0) as u8;
            }
            // (COLLECT_PICK_MODE_STAT is 0)
        }
        let mut this_best_mode = this_mode;

        // TODO(kyslov) account for UV prediction cost
        search_state.this_rdc.rate += rate_mv;
        if comp_pred {
            let mode_ctx =
                av1_mode_context_analyzer(&x.mbmi_ext.mode_context, &x.e_mbd.mbmi().ref_frame);
            search_state.this_rdc.rate += cost_mv_ref(&x.mode_costs, this_mode, mode_ctx);
        } else {
            // If the current mode has zeromv but is not GLOBALMV, compare the rate
            // cost. If GLOBALMV is cheaper, use GLOBALMV instead.
            if this_mode != GLOBALMV
                && search_state.frame_mv[m][r].as_int
                    == search_state.frame_mv[GLOBALMV as usize][r].as_int
            {
                if is_globalmv_better(
                    this_mode,
                    ref_frame,
                    rate_mv,
                    &x.mode_costs,
                    &search_state.single_inter_mode_costs,
                    &x.mbmi_ext,
                ) {
                    this_best_mode = GLOBALMV;
                }
            }

            search_state.this_rdc.rate +=
                search_state.single_inter_mode_costs[inter_offset(this_best_mode)][r];
        }

        if !comp_pred && search_state.frame_mv[m][r].as_int == 0 && var < u32::MAX {
            search_state.vars[inter_offset(GLOBALMV)][r] = var;
        }

        search_state.this_rdc.rate = search_state
            .this_rdc
            .rate
            .wrapping_add(search_state.ref_costs_single[r] as i32);

        search_state.this_rdc.rdcost = rdcost(
            x.rdmult,
            search_state.this_rdc.rate,
            search_state.this_rdc.dist,
        );
        let bm = this_best_mode as usize;
        if cpi.oxcf.rc_cfg.mode == AOM_CBR && !comp_pred {
            let mv = search_state.frame_mv[bm][r].as_mv();
            newmv_diff_bias(
                &x.e_mbd,
                this_best_mode,
                &mut search_state.this_rdc,
                bsize,
                mv.row as i32,
                mv.col as i32,
                cpi.speed,
                x.source_variance,
                x.content_state_sb,
            );
        }
        // (CONFIG_AV1_TEMPORAL_DENOISING is off)
        let _ = sse_y;

        search_state.mode_checked[m][r] = 1;
        search_state.mode_checked[bm][r] = 1;

        if check_globalmv {
            let mv = search_state.frame_mv[bm][r].as_mv();
            let abs_mv = (mv.row as i32).abs() + (mv.col as i32).abs();
            // Early exit check: if the magnitude of this_best_mode's mv is small
            // enough, we skip GLOBALMV check in the next loop iteration.
            if abs_mv < 2 {
                check_globalmv = false;
            }
        }
        // (COLLECT_PICK_MODE_STAT is 0)
        if search_state.this_rdc.rdcost < search_state.best_rdc.rdcost {
            search_state.best_rdc = search_state.this_rdc;
            best_early_term = this_early_term;
            let mi = *x.e_mbd.mbmi();
            let bp = &mut search_state.best_pickmode;
            bp.best_sse = sse_y;
            bp.best_mode = this_best_mode;
            bp.best_motion_mode = mi.motion_mode;
            bp.wm_params = mi.wm_params;
            bp.num_proj_ref = mi.num_proj_ref;
            bp.best_pred_filter = mi.interp_filters;
            bp.best_tx_size = mi.tx_size;
            bp.best_ref_frame = ref_frame;
            bp.best_second_ref_frame = ref_frame2;
            bp.best_mode_skip_txfm = search_state.this_rdc.skip_txfm;
            bp.best_mode_initial_skip_flag =
                (nonskip_rdc.rate == i32::MAX && search_state.this_rdc.skip_txfm != 0) as u8;
            if bp.best_mode_skip_txfm == 0 {
                bp.blk_skip[..num_8x8_blocks]
                    .copy_from_slice(&x.txfm_search_info.blk_skip[..num_8x8_blocks]);
            }

            // This is needed for the compound modes.
            search_state.frame_mv_best[bm][r].as_int = search_state.frame_mv[bm][r].as_int;
            if ref_frame2 > NONE_FRAME {
                let r2 = ref_frame2 as usize;
                search_state.frame_mv_best[bm][r2].as_int = search_state.frame_mv[bm][r2].as_int;
            }

            // (reuse_inter_pred is 0)
        }
        if best_early_term != 0 && (idx > 0 || cpi.sf.rt_sf.nonrd_aggressive_skip != 0) {
            x.txfm_search_info.skip_txfm = 1;
            break;
        }
    }

    {
        let bp = &search_state.best_pickmode;
        let mv0 = search_state.frame_mv_best[bp.best_mode as usize][bp.best_ref_frame as usize];
        let mv1 = if bp.best_second_ref_frame > INTRA_FRAME {
            Some(
                search_state.frame_mv_best[bp.best_mode as usize]
                    [bp.best_second_ref_frame as usize],
            )
        } else {
            None
        };
        let mi = x.e_mbd.mbmi_mut();
        mi.mode = bp.best_mode;
        mi.motion_mode = bp.best_motion_mode;
        mi.wm_params = bp.wm_params;
        mi.num_proj_ref = bp.num_proj_ref;
        mi.interp_filters = bp.best_pred_filter;
        mi.tx_size = bp.best_tx_size;
        mi.inter_tx_size.fill(mi.tx_size);
        mi.ref_frame[0] = bp.best_ref_frame;
        mi.mv[0].as_int = mv0.as_int;
        mi.mv[1].as_int = 0;
        if let Some(mv1) = mv1 {
            mi.ref_frame[1] = bp.best_second_ref_frame;
            mi.mv[1].as_int = mv1.as_int;
        }
        // Perform intra prediction search, if the best SAD is above a certain
        // threshold.
        mi.angle_delta[PLANE_TYPE_Y as usize] = 0;
        mi.angle_delta[PLANE_TYPE_UV as usize] = 0;
        mi.filter_intra_mode_info.use_filter_intra = 0;
    }

    // (COLLECT_PICK_MODE_STAT is 0)

    if x.force_zeromv_skip_for_blk == 0 {
        let ref_cost_intra = search_state.ref_costs_single[INTRA_FRAME as usize];
        let InterModeSearchStateNonrd {
            best_rdc,
            best_pickmode,
            ..
        } = &mut search_state;
        with_source(x, |x, src| {
            estimate_intra_mode(
                cpi,
                x,
                src,
                bsize,
                best_early_term,
                ref_cost_intra,
                &orig_dst,
                best_rdc,
                best_pickmode,
                ctx,
            )
        })?;
    }

    let skip_idtx_palette = (x.color_sensitivity[0] != 0 || x.color_sensitivity[1] != 0)
        && x.content_state_sb.source_sad_nonrd != K_ZERO_SAD
        && cpi.rc.high_source_sad == 0;

    // Check for IDTX: based only on Y channel, so avoid when color_sensitivity
    // is set.
    if cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN
        && !skip_idtx_palette
        && !cpi.oxcf.txfm_cfg.use_inter_dct_only
        && x.force_zeromv_skip_for_blk == 0
        && is_inter_mode(search_state.best_pickmode.best_mode)
        && (cpi.sf.rt_sf.prune_idtx_nonrd == 0
            || (cpi.sf.rt_sf.prune_idtx_nonrd != 0
                && bsize <= BLOCK_32X32
                && search_state.best_pickmode.best_mode_skip_txfm != 1
                && x.source_variance > 200))
    {
        // (block_yrd_idtx(): the screen content tools)
        return Err(());
    }

    let mut try_palette = !skip_idtx_palette
        && cpi.oxcf.tool_cfg.enable_palette
        && av1_allow_palette(
            cpi.common.features.allow_screen_content_tools,
            x.e_mbd.mbmi().bsize,
        );
    try_palette = try_palette
        && is_mode_intra(search_state.best_pickmode.best_mode)
        && x.source_variance > 0
        && x.force_zeromv_skip_for_blk == 0
        && (cpi.rc.high_source_sad != 0 || x.source_variance > 500);

    if try_palette {
        // (av1_search_palette_mode_luma(): the screen content tools)
        return Err(());
    }

    // (COLLECT_PICK_MODE_STAT is 0)

    x.e_mbd.plane[0].dst = orig_dst;
    {
        let bp = &search_state.best_pickmode;
        let mi = x.e_mbd.mbmi_mut();
        mi.mode = bp.best_mode;
        mi.ref_frame[0] = bp.best_ref_frame;
        mi.ref_frame[1] = bp.best_second_ref_frame;
    }
    x.txfm_search_info.skip_txfm = search_state.best_pickmode.best_mode_skip_txfm;
    if x.txfm_search_info.skip_txfm == 0 {
        // For inter modes: copy blk_skip from best_pickmode, which is
        // defined for 8x8 blocks. If palette or intra mode was selected
        // as best then blk_skip is already copied into the ctx.
        if search_state.best_pickmode.best_mode >= INTRA_MODE_END {
            ctx.blk_skip[..num_8x8_blocks]
                .copy_from_slice(&search_state.best_pickmode.blk_skip[..num_8x8_blocks]);
        }
    }
    {
        let mi = x.e_mbd.mbmi_mut();
        if has_second_ref(mi) {
            mi.comp_group_idx = 0;
            mi.compound_idx = 1;
            mi.interinter_comp.type_ = COMPOUND_AVERAGE;
        }

        if !is_inter_block(mi) {
            mi.interp_filters = av1_broadcast_interp_filter(SWITCHABLE_FILTERS);
        }
    }

    // (reuse_inter_pred is 0)

    // (CONFIG_AV1_TEMPORAL_DENOISING is off)

    if cpi.sf.inter_sf.adaptive_rd_thresh != 0 && !has_second_ref(x.e_mbd.mbmi()) {
        let best_ref_frame = search_state.best_pickmode.best_ref_frame;
        let best_mode_idx = MODE_IDX[best_ref_frame as usize][mode_offset(x.e_mbd.mbmi().mode)?];
        if best_ref_frame == INTRA_FRAME {
            // Only consider the modes that are included in the intra_mode_list.
            let intra_modes = INTRA_MODE_LIST.len();
            for i in 0..intra_modes {
                update_thresh_freq_fact(
                    cpi,
                    x,
                    bsize,
                    INTRA_FRAME,
                    best_mode_idx,
                    INTRA_MODE_LIST[i],
                )?;
            }
        } else {
            for this_mode in NEARESTMV..=NEWMV {
                update_thresh_freq_fact(cpi, x, bsize, best_ref_frame, best_mode_idx, this_mode)?;
            }
        }
    }

    store_coding_context(x, ctx);

    // (COLLECT_PICK_MODE_STAT is 0)

    *rd_cost = search_state.best_rdc;
    Ok(())
}

// (model_rd.h)

// Fits a curve for rate and distortion using as feature:
// log2(sse_norm/qstep^2)
#[inline]
fn model_rd_with_curvfit(
    cpi: &Av1Comp,
    x: &Macroblock,
    plane_bsize: BlockSize,
    plane: usize,
    sse: i64,
    num_samples: i32,
    rate: &mut i32,
    dist: &mut i64,
) {
    let _ = cpi;
    let xd = &x.e_mbd;
    let p = &x.plane[plane];
    let dequant_shift = if is_cur_buf_hbd(xd) { xd.bd - 5 } else { 3 };
    let qstep = ((p.dequant_qtx[1] as i32) >> dequant_shift).max(1);

    if sse == 0 {
        *rate = 0;
        *dist = 0;
        return;
    }
    let sse_norm = sse as f64 / num_samples as f64;
    let qstepsqr = qstep as f64 * qstep as f64;
    let xqr = (sse_norm / qstepsqr).log2();
    let mut rate_f = 0.0;
    let mut dist_by_sse_norm_f = 0.0;
    av1_model_rd_curvfit(
        plane_bsize,
        sse_norm,
        xqr,
        &mut rate_f,
        &mut dist_by_sse_norm_f,
    );

    let dist_f = dist_by_sse_norm_f * sse_norm;
    // (AOMMAX(0.0, v))
    let max0 = |v: f64| if 0.0 > v { 0.0 } else { v };
    let mut rate_i = (max0(rate_f * num_samples as f64) + 0.5) as i32;
    let mut dist_i = (max0(dist_f * num_samples as f64) + 0.5) as i64;

    // Check if skip is better
    if rate_i == 0 {
        dist_i = sse << 4;
    } else if rdcost(x.rdmult, rate_i, dist_i) >= rdcost(x.rdmult, 0, sse << 4) {
        rate_i = 0;
        dist_i = sse << 4;
    }

    *rate = rate_i;
    *dist = dist_i;
}
