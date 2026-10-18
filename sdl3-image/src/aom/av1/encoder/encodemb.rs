// Rust translation of the parts of av1/encoder/encodemb.c and
// av1/encoder/encodemb.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Coding of an intra block's transform blocks: prediction, residual,
//! forward transform, quantization (with trellis and dropout
//! optimization) and reconstruction. The source frame's samples come in as
//! `src` (the source frame's buffer; `x->plane[].src` holds offsets into
//! it), the reconstruction is `xd.recon`.

use super::av1_quantize::*;
use super::block::*;
use super::encoder::Av1Comp;
use super::encodetxb::{av1_get_txb_entropy_context, block_offset};
use super::hybrid_fwd_txfm::av1_fwd_txfm;
use super::rd::av1_get_entropy_contexts;
use super::txb_rdopt::{av1_cost_skip_txb, av1_optimize_txb};
use crate::aom::aom_dsp::aom_dsp_common::TranLowT;
use crate::aom::aom_dsp::subtract::aom_highbd_subtract_block;
use crate::aom::av1::common::av1_common_int::CommonQuantParams;
use crate::aom::av1::common::av1_common_int::{frame_is_intra_only, Av1Common};
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cfl::cfl_store_tx;
use crate::aom::av1::common::entropy::EntropyContext;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::idct::{av1_get_tx_scale, av1_inverse_transform_block, TxfmParam};
use crate::aom::av1::common::quant_common::{av1_get_iqmatrix, av1_get_qmatrix};
use crate::aom::av1::common::reconintra::av1_predict_intra_block_facade;
use crate::aom::av1::common::scan::get_scan;
use crate::aom::av1::common::txb_common::{get_txb_ctx, TxbCtx};

// (encodemb.h)

// Available optimization types to optimize the quantized coefficients.
// (OPT_TYPE)
pub(crate) const NONE_OPT: i32 = 0; // No optimization.
pub(crate) const TRELLIS_OPT: i32 = 1; // Trellis optimization. See `av1_optimize_b()`.
pub(crate) const DROPOUT_OPT: i32 = 2; // Dropout optimization. See `av1_dropout_qcoeff()`.
pub(crate) const TRELLIS_DROPOUT_OPT: i32 = 3; // Perform dropout after trellis optimization.

/// Translation of `struct encode_b_args` (without the `optimize_ctx`, which
/// the intra path leaves NULL): `skip` is `xd->mi[0]->skip_txfm`, set
/// through `x`.
pub(crate) struct EncodeBArgs<'a> {
    pub(crate) cpi: &'a Av1Comp,
    pub(crate) x: &'a mut Macroblock,
    /// The source frame's samples.
    pub(crate) src: &'a [u16],
    pub(crate) ta: [EntropyContext; MAX_MIB_SIZE as usize],
    pub(crate) tl: [EntropyContext; MAX_MIB_SIZE as usize],
    pub(crate) dry_run: i32,
    pub(crate) enable_optimize_b: i32,
}

#[inline]
pub(crate) fn av1_set_txb_context(
    x: &Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    a: &mut [EntropyContext],
    l: &mut [EntropyContext],
) {
    let ctx = x.plane[plane].txb_entropy_ctx[block as usize];
    a[..TX_SIZE_WIDE_UNIT[tx_size as usize] as usize].fill(ctx as EntropyContext);
    l[..TX_SIZE_HIGH_UNIT[tx_size as usize] as usize].fill(ctx as EntropyContext);
}

#[inline]
pub(crate) fn is_trellis_used(optimize_b: i32, dry_run: i32) -> bool {
    if optimize_b == NO_TRELLIS_OPT {
        return false;
    }
    if optimize_b == FINAL_PASS_TRELLIS_OPT && dry_run != OUTPUT_ENABLED {
        return false;
    }
    true
}

// (encodemb.c)

pub(crate) fn av1_subtract_block(
    bd_info: BitDepthInfo,
    rows: i32,
    cols: i32,
    diff: &mut [i16],
    diff_off: usize,
    diff_stride: usize,
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    pred: &[u16],
    pred_off: usize,
    pred_stride: usize,
) {
    debug_assert!(rows >= 4 && cols >= 4);
    // (aom_subtract_block() computes what aom_highbd_subtract_block()
    // computes)
    let _ = bd_info;
    aom_highbd_subtract_block(
        rows,
        cols,
        diff,
        diff_off,
        diff_stride,
        src,
        src_off,
        src_stride,
        pred,
        pred_off,
        pred_stride,
    );
}

pub(crate) fn av1_subtract_txb(
    x: &mut Macroblock,
    src_buf: &[u16],
    plane: usize,
    plane_bsize: BlockSize,
    blk_col: i32,
    blk_row: i32,
    tx_size: TxSize,
) {
    let Macroblock {
        plane: planes,
        e_mbd: xd,
        ..
    } = x;
    let bd_info = get_bit_depth_info(xd);
    let p = &mut planes[plane];
    let pd = &xd.plane[plane];
    let diff_stride = BLOCK_SIZE_WIDE[plane_bsize as usize] as i32;
    let src_stride = p.src.stride;
    let dst_stride = pd.dst.stride;
    let tx1d_width = TX_SIZE_WIDE[tx_size as usize] as i32;
    let tx1d_height = TX_SIZE_HIGH[tx_size as usize] as i32;
    let dst = pd.dst.buf + ((blk_row * dst_stride + blk_col) << MI_SIZE_LOG2) as usize;
    let src = p.src.buf + ((blk_row * src_stride + blk_col) << MI_SIZE_LOG2) as usize;
    let src_diff = ((blk_row * diff_stride + blk_col) << MI_SIZE_LOG2) as usize;
    av1_subtract_block(
        bd_info,
        tx1d_height,
        tx1d_width,
        &mut p.src_diff,
        src_diff,
        diff_stride as usize,
        src_buf,
        src,
        src_stride as usize,
        &xd.recon.buffer_alloc,
        dst,
        dst_stride as usize,
    );
}

pub(crate) fn av1_optimize_b(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    tx_type: TxType,
    txb_ctx: &TxbCtx,
    rate_cost: &mut i32,
) -> i32 {
    let eob = x.plane[plane].eobs[block as usize] as i32;
    let segment_id = x.e_mbd.mbmi().segment_id as usize;

    if eob == 0 || cpi.optimize_seg_arr[segment_id] == 0 || x.e_mbd.lossless[segment_id] != 0 {
        *rate_cost = av1_cost_skip_txb(&x.coeff_costs, txb_ctx, plane, tx_size);
        return eob;
    }

    av1_optimize_txb(
        cpi,
        x,
        plane,
        block,
        tx_size,
        tx_type,
        txb_ctx,
        rate_cost,
        cpi.oxcf.algo_cfg.sharpness,
    )
}

// Hyper-parameters for dropout optimization, based on following logics.
// TODO(yjshen): These settings are tuned by experiments. They may still be
// optimized for better performance.
// (1) Coefficients which are large enough will ALWAYS be kept.
const DROPOUT_COEFF_MAX: TranLowT = 2; // Max dropout-able coefficient.
                                       // (2) Continuous coefficients will ALWAYS be kept. Here rigorous continuity is
                                       //     NOT required. For example, `5 0 0 0 7` is treated as two continuous
                                       //     coefficients if three zeros do not fulfill the dropout condition.
const DROPOUT_CONTINUITY_MAX: i32 = 2; // Max dropout-able continuous coeff.
                                       // (3) Dropout operation is NOT applicable to blocks with large or small
                                       //     quantization index.
const DROPOUT_Q_MAX: i32 = 128;
const DROPOUT_Q_MIN: i32 = 16;
// (4) Recall that dropout optimization will forcibly set some quantized
//     coefficients to zero. The key logic on determining whether a coefficient
//     should be dropped is to check the number of continuous zeros before AND
//     after this coefficient. The exact number of zeros for judgement depends
//     on block size and quantization index. More concretely, block size
//     determines the base number of zeros, while quantization index determines
//     the multiplier. Intuitively, larger block requires more zeros and larger
//     quantization index also requires more zeros (more information is lost
//     when using larger quantization index).
const DROPOUT_BEFORE_BASE_MAX: i32 = 32; // Max base number for leading zeros.
const DROPOUT_BEFORE_BASE_MIN: i32 = 16; // Min base number for leading zeros.
const DROPOUT_AFTER_BASE_MAX: i32 = 32; // Max base number for trailing zeros.
const DROPOUT_AFTER_BASE_MIN: i32 = 16; // Min base number for trailing zeros.
const DROPOUT_MULTIPLIER_MAX: i32 = 8; // Max multiplier on number of zeros.
const DROPOUT_MULTIPLIER_MIN: i32 = 2; // Min multiplier on number of zeros.
const DROPOUT_MULTIPLIER_Q_BASE: i32 = 32; // Base Q to compute multiplier.

pub(crate) fn av1_dropout_qcoeff(
    mb: &mut Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    tx_type: TxType,
    qindex: i32,
) {
    let tx_width = TX_SIZE_WIDE[tx_size as usize] as i32;
    let tx_height = TX_SIZE_HIGH[tx_size as usize] as i32;

    // Early return if `qindex` is out of range.
    if qindex > DROPOUT_Q_MAX || qindex < DROPOUT_Q_MIN {
        return;
    }

    // Compute number of zeros used for dropout judgement.
    let base_size = tx_width.max(tx_height);
    let multiplier =
        (qindex / DROPOUT_MULTIPLIER_Q_BASE).clamp(DROPOUT_MULTIPLIER_MIN, DROPOUT_MULTIPLIER_MAX);
    let dropout_num_before =
        multiplier * base_size.clamp(DROPOUT_BEFORE_BASE_MIN, DROPOUT_BEFORE_BASE_MAX);
    let dropout_num_after =
        multiplier * base_size.clamp(DROPOUT_AFTER_BASE_MIN, DROPOUT_AFTER_BASE_MAX);

    av1_dropout_qcoeff_num(
        mb,
        plane,
        block,
        tx_size,
        tx_type,
        dropout_num_before,
        dropout_num_after,
    );
}

pub(crate) fn av1_dropout_qcoeff_num(
    mb: &mut Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    tx_type: TxType,
    dropout_num_before: i32,
    dropout_num_after: i32,
) {
    let p = &mut mb.plane[plane];
    let off = block_offset(block);
    let max_eob = av1_get_max_eob(tx_size);
    let scan_order = get_scan(tx_size, tx_type);
    let block = block as usize;
    let p_eob = p.eobs[block] as i32;

    // Early return if there are not enough non-zero coefficients.
    if p_eob == 0
        || p_eob <= dropout_num_before
        || max_eob <= dropout_num_before + dropout_num_after
    {
        return;
    }

    let mut count_zeros_before = 0;
    let mut count_zeros_after = 0;
    let mut count_nonzeros = 0;
    // Index of the first non-zero coefficient after sufficient number of
    // continuous zeros. If equals to `-1`, it means number of leading zeros
    // hasn't reach `dropout_num_before`.
    let mut idx: i32 = -1;
    let mut eob = 0; // New end of block.

    for i in 0..p_eob {
        let scan_idx = scan_order.scan[i as usize] as usize;
        let qc = p.qcoeff[off + scan_idx];
        if qc.abs() > DROPOUT_COEFF_MAX {
            // Keep large coefficients.
            count_zeros_before = 0;
            count_zeros_after = 0;
            idx = -1;
            eob = i + 1;
        } else if qc == 0 {
            // Count zeros.
            if idx == -1 {
                count_zeros_before += 1;
            } else {
                count_zeros_after += 1;
            }
        } else {
            // Count non-zeros.
            if count_zeros_before >= dropout_num_before {
                idx = if idx == -1 { i } else { idx };
                count_nonzeros += 1;
            } else {
                count_zeros_before = 0;
                eob = i + 1;
            }
        }

        // Handle continuity.
        if count_nonzeros > DROPOUT_CONTINUITY_MAX {
            count_zeros_before = 0;
            count_zeros_after = 0;
            count_nonzeros = 0;
            idx = -1;
            eob = i + 1;
        }

        // Handle the trailing zeros after original end of block.
        if idx != -1 && i == p_eob - 1 {
            count_zeros_after += max_eob - p_eob;
        }

        // Set redundant coefficients to zeros if needed.
        if count_zeros_after >= dropout_num_after {
            for j in idx..=i {
                let s = scan_order.scan[j as usize] as usize;
                p.qcoeff[off + s] = 0;
                p.dqcoeff[off + s] = 0;
            }
            count_zeros_before += i - idx + 1;
            count_zeros_after = 0;
            count_nonzeros = 0;
        } else if i == p_eob - 1 {
            eob = i + 1;
        }
    }

    if eob != p_eob {
        p.eobs[block] = eob as u16;
        p.txb_entropy_ctx[block] = av1_get_txb_entropy_context(&p.qcoeff[off..], &scan_order, eob);
    }
}

// Settings for optimization type. NOTE: To set optimization type for all intra
// frames, both `KEY_BLOCK_OPT_TYPE` and `INTRA_BLOCK_OPT_TYPE` should be set.
// TODO(yjshen): These settings are hard-coded and look okay for now. They
// should be made configurable later.
// Blocks of key frames ONLY.
const KEY_BLOCK_OPT_TYPE: i32 = TRELLIS_DROPOUT_OPT;
// Blocks of intra frames (key frames EXCLUSIVE).
const INTRA_BLOCK_OPT_TYPE: i32 = TRELLIS_DROPOUT_OPT;
// Blocks of inter frames. (NOTE: Dropout optimization is DISABLED by default
// if trellis optimization is on for inter frames.)
const INTER_BLOCK_OPT_TYPE: i32 = TRELLIS_DROPOUT_OPT;

// (QUANT_FUNC)
const QUANT_FUNC_LOWBD: i32 = 0;
const QUANT_FUNC_HIGHBD: i32 = 1;
const QUANT_FUNC_TYPES: i32 = 2;

pub(crate) fn av1_xform_quant(
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    txfm_param: &TxfmParam,
    qparam: &QuantParam,
) -> Result<(), ()> {
    av1_xform(x, plane, block, blk_row, blk_col, plane_bsize, txfm_param)?;
    av1_quant(x, plane, block, txfm_param, qparam)
}

pub(crate) fn av1_xform(
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    let p = &mut x.plane[plane];
    let block_offset = block_offset(block);
    let diff_stride = BLOCK_SIZE_WIDE[plane_bsize as usize] as i32;

    let src_offset = blk_row * diff_stride + blk_col;
    let src_diff = &p.src_diff[(src_offset << MI_SIZE_LOG2) as usize..];

    av1_fwd_txfm(
        src_diff,
        &mut p.coeff[block_offset..],
        diff_stride as usize,
        txfm_param,
    )
}

pub(crate) fn av1_quant(
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    txfm_param: &TxfmParam,
    qparam: &QuantParam,
) -> Result<(), ()> {
    let seg_skip_block = x.seg_skip_block;
    let p = &mut x.plane[plane];
    let scan_order = get_scan(txfm_param.tx_size, txfm_param.tx_type);
    let block_offset = block_offset(block);
    let block = block as usize;

    if qparam.xform_quant_idx != AV1_XFORM_QUANT_SKIP_QUANT {
        let n_coeffs = av1_get_max_eob(txfm_param.tx_size);
        if seg_skip_block == 0 {
            // (quant_func_list[qparam->xform_quant_idx][txfm_param->is_hbd]:
            // the quantizer is "fp" or "b"; the coefficient buffers are taken
            // out of the plane for the call, which reads the plane's
            // quantizers)
            type Facade = fn(
                &[TranLowT],
                i32,
                &MacroblockPlane,
                &mut [TranLowT],
                &mut [TranLowT],
                &mut u16,
                &crate::aom::av1::common::scan::ScanOrder,
                &QuantParam,
            ) -> Result<(), ()>;
            let facade: Facade = match (qparam.xform_quant_idx, txfm_param.is_hbd != 0) {
                (AV1_XFORM_QUANT_FP, false) => av1_quantize_fp_facade,
                (AV1_XFORM_QUANT_B, false) => av1_quantize_b_facade,
                (AV1_XFORM_QUANT_FP, true) => av1_highbd_quantize_fp_facade,
                (AV1_XFORM_QUANT_B, true) => av1_highbd_quantize_b_facade,
                // (the "dc" quantizers: never used)
                _ => return Err(()),
            };
            let coeff = std::mem::take(&mut p.coeff);
            let mut qcoeff = std::mem::take(&mut p.qcoeff);
            let mut dqcoeff = std::mem::take(&mut p.dqcoeff);
            let mut eob = p.eobs[block];
            let res = facade(
                &coeff[block_offset..],
                n_coeffs,
                p,
                &mut qcoeff[block_offset..],
                &mut dqcoeff[block_offset..],
                &mut eob,
                &scan_order,
                qparam,
            );
            p.eobs[block] = eob;
            p.coeff = coeff;
            p.qcoeff = qcoeff;
            p.dqcoeff = dqcoeff;
            res?;
        } else {
            av1_quantize_skip(
                n_coeffs,
                &mut p.qcoeff[block_offset..],
                &mut p.dqcoeff[block_offset..],
                &mut p.eobs[block],
            );
        }
    }
    // use_optimize_b is true means av1_optimze_b will be called,
    // thus cannot update entropy ctx now (performed in optimize_b)
    if qparam.use_optimize_b != 0 {
        p.txb_entropy_ctx[block] = 0;
    } else {
        p.txb_entropy_ctx[block] = av1_get_txb_entropy_context(
            &p.qcoeff[block_offset..],
            &scan_order,
            p.eobs[block] as i32,
        );
    }
    Ok(())
}

pub(crate) fn av1_setup_xform(
    cm: &Av1Common,
    x: &Macroblock,
    tx_size: TxSize,
    tx_type: TxType,
    txfm_param: &mut TxfmParam,
) {
    let xd = &x.e_mbd;
    let mbmi = xd.mbmi();

    txfm_param.tx_type = tx_type;
    txfm_param.tx_size = tx_size;
    txfm_param.lossless = xd.lossless[mbmi.segment_id as usize];
    txfm_param.tx_set_type = av1_get_ext_tx_set_type(
        tx_size,
        is_inter_block(mbmi) as i32,
        cm.features.reduced_tx_set_used as i32,
    );

    txfm_param.bd = xd.bd;
    txfm_param.is_hbd = is_cur_buf_hbd(xd) as i32;
}

pub(crate) fn av1_setup_quant(
    tx_size: TxSize,
    use_optimize_b: i32,
    xform_quant_idx: i32,
    use_quant_b_adapt: i32,
    qparam: &mut QuantParam,
) {
    qparam.log_scale = av1_get_tx_scale(tx_size);
    qparam.tx_size = tx_size;

    qparam.use_quant_b_adapt = use_quant_b_adapt;

    // TODO(bohanli): optimize_b and quantization idx has relationship,
    // but is kind of buried and complicated in different encoding stages.
    // Should have a unified function to derive quant_idx, rather than
    // determine and pass in the quant_idx
    qparam.use_optimize_b = use_optimize_b;
    qparam.xform_quant_idx = xform_quant_idx;

    qparam.qmatrix = None;
    qparam.iqmatrix = None;
}

pub(crate) fn av1_setup_qmatrix(
    quant_params: &CommonQuantParams,
    xd: &MacroBlockD,
    plane: usize,
    tx_size: TxSize,
    tx_type: TxType,
    qparam: &mut QuantParam,
) {
    // (the flat matrices: quantization matrices are off)
    let _ = (quant_params, xd, plane, tx_size, tx_type);
    qparam.qmatrix = av1_get_qmatrix();
    qparam.iqmatrix = av1_get_iqmatrix();
}

/// Translation of `av1_foreach_transformed_block_in_plane()`: `visit` gets
/// `arg` (the visitor's `void *arg`) and the block's position; `xd` finds
/// the block's `MACROBLOCKD` in `arg`, read before the first visit. A
/// visitor's error stops the walk.
pub(crate) fn av1_foreach_transformed_block_in_plane<T>(
    arg: &mut T,
    xd: impl Fn(&T) -> &MacroBlockD,
    plane_bsize: BlockSize,
    plane: usize,
    mut visit: impl FnMut(&mut T, usize, i32, i32, i32, BlockSize, TxSize) -> Result<(), ()>,
) -> Result<(), ()> {
    let xd = xd(arg);
    let pd = &xd.plane[plane];
    // block and transform sizes, in number of 4x4 blocks log 2 ("*_b")
    // 4x4=0, 8x8=2, 16x16=4, 32x32=6, 64x64=8
    // transform size varies per plane, look it up in a common way.
    let tx_size = av1_get_tx_size(plane, xd);
    let txw_unit = TX_SIZE_WIDE_UNIT[tx_size as usize] as i32;
    let txh_unit = TX_SIZE_HIGH_UNIT[tx_size as usize] as i32;
    let step = txw_unit * txh_unit;

    // If mb_to_right_edge is < 0 we are in a situation in which
    // the current block size extends into the UMV and we won't
    // visit the sub blocks that are wholly within the UMV.
    let max_blocks_wide = max_block_wide(xd, plane_bsize, plane);
    let max_blocks_high = max_block_high(xd, plane_bsize, plane);
    let max_unit_bsize = get_plane_block_size(BLOCK_64X64, pd.subsampling_x, pd.subsampling_y);
    let mu_blocks_wide = (MI_SIZE_WIDE[max_unit_bsize as usize] as i32).min(max_blocks_wide);
    let mu_blocks_high = (MI_SIZE_HIGH[max_unit_bsize as usize] as i32).min(max_blocks_high);

    // Keep track of the row and column of the blocks we use so that we know
    // if we are in the unrestricted motion border.
    let mut i = 0;
    let mut r = 0;
    while r < max_blocks_high {
        let unit_height = (mu_blocks_high + r).min(max_blocks_high);
        // Skip visiting the sub blocks that are wholly within the UMV.
        let mut c = 0;
        while c < max_blocks_wide {
            let unit_width = (mu_blocks_wide + c).min(max_blocks_wide);
            let mut blk_row = r;
            while blk_row < unit_height {
                let mut blk_col = c;
                while blk_col < unit_width {
                    visit(arg, plane, i, blk_row, blk_col, plane_bsize, tx_size)?;
                    i += step;
                    blk_col += txw_unit;
                }
                blk_row += txh_unit;
            }
            c += mu_blocks_wide;
        }
        r += mu_blocks_high;
    }
    Ok(())
}

fn encode_block_intra_and_set_context(
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    args: &mut EncodeBArgs,
) -> Result<(), ()> {
    av1_encode_block_intra(plane, block, blk_row, blk_col, plane_bsize, tx_size, args)?;

    let EncodeBArgs { x, ta, tl, .. } = args;
    av1_set_txb_context(
        x,
        plane,
        block,
        tx_size,
        &mut ta[blk_col as usize..],
        &mut tl[blk_row as usize..],
    );
    Ok(())
}

pub(crate) fn av1_encode_block_intra(
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    args: &mut EncodeBArgs,
) -> Result<(), ()> {
    let cpi = args.cpi;
    let cm = &cpi.common;
    let off = block_offset(block);
    let plane_type = get_plane_type(plane);
    let mut dummy_rate_cost = 0;

    av1_predict_intra_block_facade(cm, &mut args.x.e_mbd, plane, blk_col, blk_row, tx_size)?;

    let mut tx_type = DCT_DCT;
    let bw = MI_SIZE_WIDE[plane_bsize as usize] as i32;
    if plane == 0
        && is_blk_skip(
            &args.x.txfm_search_info.blk_skip,
            plane,
            (blk_row * bw + blk_col) as usize,
        )
    {
        args.x.plane[plane].eobs[block as usize] = 0;
    } else {
        av1_subtract_txb(
            args.x,
            args.src,
            plane,
            plane_bsize,
            blk_col,
            blk_row,
            tx_size,
        );

        let x = &mut *args.x;
        tx_type = av1_get_tx_type(
            &x.e_mbd,
            plane_type,
            blk_row,
            blk_col,
            tx_size,
            cm.features.reduced_tx_set_used as i32,
        );
        let mut txfm_param = TxfmParam::default();
        let mut quant_param = QuantParam::default();
        let use_trellis = is_trellis_used(args.enable_optimize_b, args.dry_run);
        let quant_idx = if use_trellis {
            AV1_XFORM_QUANT_FP
        } else if USE_B_QUANT_NO_TRELLIS != 0 {
            AV1_XFORM_QUANT_B
        } else {
            AV1_XFORM_QUANT_FP
        };
        av1_setup_xform(cm, x, tx_size, tx_type, &mut txfm_param);
        av1_setup_quant(
            tx_size,
            use_trellis as i32,
            quant_idx,
            cpi.oxcf.q_cfg.quant_b_adapt as i32,
            &mut quant_param,
        );
        av1_setup_qmatrix(
            &cm.quant_params,
            &x.e_mbd,
            plane,
            tx_size,
            tx_type,
            &mut quant_param,
        );

        av1_xform_quant(
            x,
            plane,
            block,
            blk_row,
            blk_col,
            plane_bsize,
            &txfm_param,
            &quant_param,
        )?;

        // Whether trellis or dropout optimization is required for key frames and
        // intra frames.
        let do_trellis = (frame_is_intra_only(cm)
            && (KEY_BLOCK_OPT_TYPE == TRELLIS_OPT || KEY_BLOCK_OPT_TYPE == TRELLIS_DROPOUT_OPT))
            || (!frame_is_intra_only(cm)
                && (INTRA_BLOCK_OPT_TYPE == TRELLIS_OPT
                    || INTRA_BLOCK_OPT_TYPE == TRELLIS_DROPOUT_OPT));
        let do_dropout = (frame_is_intra_only(cm)
            && (KEY_BLOCK_OPT_TYPE == DROPOUT_OPT || KEY_BLOCK_OPT_TYPE == TRELLIS_DROPOUT_OPT))
            || (!frame_is_intra_only(cm)
                && (INTRA_BLOCK_OPT_TYPE == DROPOUT_OPT
                    || INTRA_BLOCK_OPT_TYPE == TRELLIS_DROPOUT_OPT));

        if quant_param.use_optimize_b != 0 && do_trellis {
            let mut txb_ctx = TxbCtx::default();
            get_txb_ctx(
                plane_bsize,
                tx_size,
                plane,
                &args.ta[blk_col as usize..],
                &args.tl[blk_row as usize..],
                &mut txb_ctx,
            );
            av1_optimize_b(
                cpi,
                x,
                plane,
                block,
                tx_size,
                tx_type,
                &txb_ctx,
                &mut dummy_rate_cost,
            );
        }
        if do_dropout {
            av1_dropout_qcoeff(
                x,
                plane,
                block,
                tx_size,
                tx_type,
                cm.quant_params.base_qindex,
            );
        }
    }

    let x = &mut *args.x;
    let eob = x.plane[plane].eobs[block as usize];
    if eob != 0 {
        let Macroblock {
            plane: planes,
            e_mbd: xd,
            ..
        } = x;
        let pd = &xd.plane[plane];
        let dst_stride = pd.dst.stride;
        let dst = pd.dst.buf + ((blk_row * dst_stride + blk_col) << MI_SIZE_LOG2) as usize;
        // (av1_inverse_transform_block(xd, ...): the block's lossless flag
        // and bit depth)
        let lossless = xd.lossless[xd.mbmi().segment_id as usize];
        let bd = xd.bd;
        av1_inverse_transform_block(
            lossless,
            bd,
            &planes[plane].dqcoeff[off..],
            tx_type,
            tx_size,
            &mut xd.recon.buffer_alloc,
            dst,
            dst_stride as usize,
            eob as i32,
            cm.features.reduced_tx_set_used as i32,
        )?;
    }

    // TODO(jingning): Temporarily disable txk_type check for eob=0 case.
    // It is possible that certain collision in hash index would cause
    // the assertion failure. To further optimize the rate-distortion
    // performance, we need to re-visit this part and enable this assert
    // again.
    if eob == 0 && plane == 0 {
        update_txk_array(&mut x.e_mbd, blk_row, blk_col, tx_size, DCT_DCT);
    }

    // For intra mode, skipped blocks are so rare that transmitting skip=1 is
    // very expensive.
    x.e_mbd.mbmi_mut().skip_txfm = 0;

    if plane == AOM_PLANE_Y && x.e_mbd.cfl.store_y != 0 {
        cfl_store_tx(&mut x.e_mbd, blk_row, blk_col, tx_size, plane_bsize)?;
    }
    Ok(())
}

pub(crate) fn av1_encode_intra_block_plane(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    src: &[u16],
    bsize: BlockSize,
    plane: usize,
    dry_run: i32,
    enable_optimize_b: i32,
) -> Result<(), ()> {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let xd = &x.e_mbd;
    if plane != 0 && !xd.is_chroma_ref {
        return Ok(());
    }

    let pd = &xd.plane[plane];
    let ss_x = pd.subsampling_x;
    let ss_y = pd.subsampling_y;
    let mut ta = [0 as EntropyContext; MAX_MIB_SIZE as usize];
    let mut tl = [0 as EntropyContext; MAX_MIB_SIZE as usize];
    let plane_bsize = get_plane_block_size(bsize, ss_x, ss_y);
    if enable_optimize_b != 0 {
        av1_get_entropy_contexts(plane_bsize, xd, plane, &mut ta, &mut tl);
    }
    let mut arg = EncodeBArgs {
        cpi,
        x,
        src,
        ta,
        tl,
        dry_run,
        enable_optimize_b,
    };
    av1_foreach_transformed_block_in_plane(
        &mut arg,
        |a| &a.x.e_mbd,
        plane_bsize,
        plane,
        |a, plane, block, blk_row, blk_col, plane_bsize, tx_size| {
            encode_block_intra_and_set_context(
                plane,
                block,
                blk_row,
                blk_col,
                plane_bsize,
                tx_size,
                a,
            )
        },
    )
}

/// Translation of `av1_encode_sb()`.
pub(crate) fn av1_encode_sb(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    bsize: BlockSize,
    dry_run: i32,
) -> Result<(), ()> {
    let _ = (cpi, x, bsize, dry_run);
    Err(())
}
