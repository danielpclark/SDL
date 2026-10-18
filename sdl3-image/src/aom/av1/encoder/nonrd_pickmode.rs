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
        // (model_rd_for_sb_uv(): the inter frames' intra estimate)
        return Err(());
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

/// Translation of `av1_nonrd_pick_inter_mode_sb()`.
pub(crate) fn av1_nonrd_pick_inter_mode_sb(
    cpi: &Av1Comp,
    tile_data: &super::encoder::TileDataEnc,
    x: &mut Macroblock,
    rd_cost: &mut RdStats,
    bsize: BlockSize,
    ctx: &mut PickModeContext,
) -> Result<(), ()> {
    let _ = (cpi, tile_data, x, rd_cost, bsize, ctx);
    Err(())
}
