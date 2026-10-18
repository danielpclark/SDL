// Rust translation of the parts of av1/common/cfl.c and av1/common/cfl.h
// from aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Chroma from luma prediction. The still image saver never picks it (its
//! chroma mode is always DC), but whether it is allowed selects the chroma
//! mode CDF; the animation saver's real-time key frames search it. The
//! saver's chroma is never subsampled (4:4:4), so the 4:4:4 luma
//! "subsampling" is the only one reached. The pixel buffers hold the
//! samples as 16-bit values for both bit depths, which is how the low bit
//! depth functions' results compare too (the DC prediction cache of the low
//! bit depth path copies bytes in and out of a 16-bit array, which keeps
//! the values).

use super::av1_common_int::{get_tx_size, SequenceHeader};
use super::blockd::{
    get_plane_block_size, is_cur_buf_hbd, is_inter_block, max_block_high, max_block_wide,
    MacroBlockD,
};
use super::enums::*;
use crate::aom::aom_dsp::aom_dsp_common::{clip_pixel_highbd, round_power_of_two_signed};

/// `CFL_BUF_LINE`
pub(crate) const CFL_BUF_LINE: usize = 32;
/// `CFL_BUF_SQUARE`
pub(crate) const CFL_BUF_SQUARE: usize = CFL_BUF_LINE * CFL_BUF_LINE;

/// Translation of `CFL_CTX`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CflCtx {
    // Q3 reconstructed luma pixels (only Q2 is required, but Q3 is used to avoid
    // shifts)
    pub(crate) recon_buf_q3: [u16; CFL_BUF_SQUARE],
    // Q3 AC contributions (reconstructed luma pixels - tx block avg)
    pub(crate) ac_buf_q3: [i16; CFL_BUF_SQUARE],

    // Cache the DC_PRED when performing RDO, so it does not have to be recomputed
    // for every scaling parameter
    pub(crate) dc_pred_is_cached: [i32; CFL_PRED_PLANES as usize],
    // Whether the DC_PRED cache is enabled. The DC_PRED cache is disabled when
    // decoding.
    pub(crate) use_dc_pred_cache: i32,
    // Only cache the first row of the DC_PRED
    pub(crate) dc_pred_cache: [[i16; CFL_BUF_LINE]; CFL_PRED_PLANES as usize],

    // Height and width currently used in the CfL prediction buffer.
    pub(crate) buf_height: i32,
    pub(crate) buf_width: i32,

    pub(crate) are_parameters_computed: i32,

    // Chroma subsampling
    pub(crate) subsampling_x: i32,
    pub(crate) subsampling_y: i32,

    // Whether the reconstructed luma pixels need to be stored
    pub(crate) store_y: i32,
}

impl Default for CflCtx {
    fn default() -> Self {
        CflCtx {
            recon_buf_q3: [0; CFL_BUF_SQUARE],
            ac_buf_q3: [0; CFL_BUF_SQUARE],
            dc_pred_is_cached: [0; CFL_PRED_PLANES as usize],
            use_dc_pred_cache: 0,
            dc_pred_cache: [[0; CFL_BUF_LINE]; CFL_PRED_PLANES as usize],
            buf_height: 0,
            buf_width: 0,
            are_parameters_computed: 0,
            subsampling_x: 0,
            subsampling_y: 0,
            store_y: 0,
        }
    }
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

#[inline]
fn get_scaled_luma_q0(alpha_q3: i32, pred_buf_q3: i16) -> i32 {
    let scaled_luma_q6 = alpha_q3 * pred_buf_q3 as i32;
    round_power_of_two_signed(scaled_luma_q6, 6)
}

#[inline]
pub(crate) fn get_cfl_pred_type(plane: usize) -> usize {
    debug_assert!(plane > 0);
    plane - 1
}

// (cfl.c)

pub(crate) fn cfl_init(cfl: &mut CflCtx, seq_params: &SequenceHeader) {
    cfl.recon_buf_q3 = [0; CFL_BUF_SQUARE];
    cfl.ac_buf_q3 = [0; CFL_BUF_SQUARE];
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

/// Translation of `cfl_store_dc_pred()`: the first row of the prediction in
/// `xd.recon` at `input`.
pub(crate) fn cfl_store_dc_pred(xd: &mut MacroBlockD, input: usize, pred_plane: usize, width: i32) {
    debug_assert!(pred_plane < CFL_PRED_PLANES as usize);
    debug_assert!(width as usize <= CFL_BUF_LINE);

    let w = width as usize;
    for i in 0..w {
        xd.cfl.dc_pred_cache[pred_plane][i] = xd.recon.buffer_alloc[input + i] as i16;
    }
}

/// Translation of `cfl_load_dc_pred()` (`cfl_load_dc_pred_lbd()` and
/// `cfl_load_dc_pred_hbd()` copy the same values).
pub(crate) fn cfl_load_dc_pred(
    xd: &mut MacroBlockD,
    dst: usize,
    dst_stride: usize,
    tx_size: TxSize,
    pred_plane: usize,
) {
    let width = TX_SIZE_WIDE[tx_size as usize] as usize;
    let height = TX_SIZE_HIGH[tx_size as usize] as usize;
    debug_assert!(pred_plane < CFL_PRED_PLANES as usize);
    debug_assert!(width <= CFL_BUF_LINE);
    debug_assert!(height <= CFL_BUF_LINE);
    let mut d = dst;
    for _j in 0..height {
        for i in 0..width {
            xd.recon.buffer_alloc[d + i] = xd.cfl.dc_pred_cache[pred_plane][i] as u16;
        }
        d += dst_stride;
    }
}

// Due to frame boundary issues, it is possible that the total area covered by
// chroma exceeds that of luma. When this happens, we fill the missing pixels by
// repeating the last columns and/or rows.
#[inline]
fn cfl_pad(cfl: &mut CflCtx, width: i32, height: i32) {
    let diff_width = width - cfl.buf_width;
    let diff_height = height - cfl.buf_height;

    if diff_width > 0 {
        let min_height = height - diff_height;
        let mut recon_buf_q3 = (width - diff_width) as usize;
        for _j in 0..min_height {
            let last_pixel = cfl.recon_buf_q3[recon_buf_q3 - 1];
            debug_assert!(recon_buf_q3 + diff_width as usize <= CFL_BUF_SQUARE);
            for i in 0..diff_width as usize {
                cfl.recon_buf_q3[recon_buf_q3 + i] = last_pixel;
            }
            recon_buf_q3 += CFL_BUF_LINE;
        }
        cfl.buf_width = width;
    }
    if diff_height > 0 {
        let mut recon_buf_q3 = (height - diff_height) as usize * CFL_BUF_LINE;
        for _j in 0..diff_height {
            let last_row_q3 = recon_buf_q3 - CFL_BUF_LINE;
            debug_assert!(recon_buf_q3 + width as usize <= CFL_BUF_SQUARE);
            for i in 0..width as usize {
                cfl.recon_buf_q3[recon_buf_q3 + i] = cfl.recon_buf_q3[last_row_q3 + i];
            }
            recon_buf_q3 += CFL_BUF_LINE;
        }
        cfl.buf_height = height;
    }
}

fn subtract_average_c(
    cfl: &mut CflCtx,
    width: i32,
    height: i32,
    round_offset: i32,
    num_pel_log2: i32,
) {
    let mut sum = round_offset;
    let mut recon = 0;
    for _j in 0..height {
        for i in 0..width as usize {
            sum += cfl.recon_buf_q3[recon + i] as i32;
        }
        recon += CFL_BUF_LINE;
    }
    let avg = sum >> num_pel_log2;
    let mut s = 0;
    for _j in 0..height {
        for i in 0..width as usize {
            cfl.ac_buf_q3[s + i] = (cfl.recon_buf_q3[s + i] as i32 - avg) as i16;
        }
        s += CFL_BUF_LINE;
    }
}

/// `cfl_get_subtract_average_fn(tx_size)` applied (the sizes' round offsets
/// and pel counts of `CFL_SUB_AVG_FN`).
fn cfl_subtract_average(cfl: &mut CflCtx, tx_size: TxSize) -> Result<(), ()> {
    let (w, h, round_offset, num_pel_log2) = match tx_size {
        TX_4X4 => (4, 4, 8, 4),
        TX_4X8 => (4, 8, 16, 5),
        TX_4X16 => (4, 16, 32, 6),
        TX_8X4 => (8, 4, 16, 5),
        TX_8X8 => (8, 8, 32, 6),
        TX_8X16 => (8, 16, 64, 7),
        TX_8X32 => (8, 32, 128, 8),
        TX_16X4 => (16, 4, 32, 6),
        TX_16X8 => (16, 8, 64, 7),
        TX_16X16 => (16, 16, 128, 8),
        TX_16X32 => (16, 32, 256, 9),
        TX_32X8 => (32, 8, 128, 8),
        TX_32X16 => (32, 16, 256, 9),
        TX_32X32 => (32, 32, 512, 10),
        // (invalid CFL size: a NULL function)
        _ => return Err(()),
    };
    subtract_average_c(cfl, w, h, round_offset, num_pel_log2);
    Ok(())
}

#[inline]
fn cfl_idx_to_alpha(alpha_idx: u8, joint_sign: i8, pred_type: usize) -> i32 {
    let alpha_sign = if pred_type == CFL_PRED_U as usize {
        cfl_sign_u(joint_sign as i32)
    } else {
        cfl_sign_v(joint_sign as i32)
    };
    if alpha_sign == CFL_SIGN_ZERO {
        return 0;
    }
    let abs_alpha_q3 = if pred_type == CFL_PRED_U as usize {
        cfl_idx_u(alpha_idx as i32)
    } else {
        cfl_idx_v(alpha_idx as i32)
    };
    if alpha_sign == CFL_SIGN_POS {
        abs_alpha_q3 + 1
    } else {
        -abs_alpha_q3 - 1
    }
}

// (the CFL_SIGN_U, CFL_SIGN_V, CFL_IDX_U and CFL_IDX_V macros of enums.h)
#[inline]
fn cfl_sign_u(js: i32) -> i32 {
    ((js + 1) * 11) >> 5
}
#[inline]
fn cfl_sign_v(js: i32) -> i32 {
    (js + 1) - CFL_SIGNS * cfl_sign_u(js)
}
#[inline]
fn cfl_idx_u(idx: i32) -> i32 {
    idx >> CFL_ALPHABET_SIZE_LOG2
}
#[inline]
fn cfl_idx_v(idx: i32) -> i32 {
    idx & (CFL_ALPHABET_SIZE - 1)
}

/// `cfl_predict_lbd_c()` and `cfl_predict_hbd_c()` (`clip_pixel()` is
/// `clip_pixel_highbd()` at 8 bits).
fn cfl_predict(
    ac_buf_q3: &[i16],
    dst_buf: &mut [u16],
    dst: usize,
    dst_stride: usize,
    alpha_q3: i32,
    bit_depth: i32,
    width: i32,
    height: i32,
) {
    let mut d = dst;
    let mut a = 0;
    for _j in 0..height {
        for i in 0..width as usize {
            dst_buf[d + i] = clip_pixel_highbd(
                get_scaled_luma_q0(alpha_q3, ac_buf_q3[a + i]) + dst_buf[d + i] as i32,
                bit_depth,
            );
        }
        d += dst_stride;
        a += CFL_BUF_LINE;
    }
}

fn cfl_compute_parameters(xd: &mut MacroBlockD, tx_size: TxSize) -> Result<(), ()> {
    let cfl = &mut xd.cfl;
    // Do not call cfl_compute_parameters multiple time on the same values.
    debug_assert!(cfl.are_parameters_computed == 0);

    cfl_pad(
        cfl,
        TX_SIZE_WIDE[tx_size as usize] as i32,
        TX_SIZE_HIGH[tx_size as usize] as i32,
    );
    cfl_subtract_average(cfl, tx_size)?;
    cfl.are_parameters_computed = 1;
    Ok(())
}

/// Translation of `cfl_predict_block()`, predicting into `xd.recon` at
/// `dst`.
pub(crate) fn cfl_predict_block(
    xd: &mut MacroBlockD,
    dst: usize,
    dst_stride: usize,
    tx_size: TxSize,
    plane: usize,
) -> Result<(), ()> {
    debug_assert!(is_cfl_allowed(xd) != 0);

    if xd.cfl.are_parameters_computed == 0 {
        cfl_compute_parameters(xd, tx_size)?;
    }

    let mbmi = *xd.mbmi();
    let alpha_q3 = cfl_idx_to_alpha(mbmi.cfl_alpha_idx, mbmi.cfl_alpha_signs, plane - 1);
    debug_assert!(
        (TX_SIZE_HIGH[tx_size as usize] as usize - 1) * CFL_BUF_LINE
            + TX_SIZE_WIDE[tx_size as usize] as usize
            <= CFL_BUF_SQUARE
    );
    // (cfl_get_predict_hbd_fn(tx_size) and cfl_get_predict_lbd_fn(tx_size):
    // NULL for the sizes cfl_subtract_average() rejects)
    let bd = if is_cur_buf_hbd(xd) { xd.bd } else { 8 };
    let MacroBlockD { cfl, recon, .. } = xd;
    cfl_predict(
        &cfl.ac_buf_q3,
        &mut recon.buffer_alloc,
        dst,
        dst_stride,
        alpha_q3,
        bd,
        TX_SIZE_WIDE[tx_size as usize] as i32,
        TX_SIZE_HIGH[tx_size as usize] as i32,
    );
    Ok(())
}

/// `cfl_luma_subsampling_444_lbd_c()` and `cfl_luma_subsampling_444_hbd_c()`
/// (the same for both bit depths).
fn cfl_luma_subsampling_444(
    input_buf: &[u16],
    input: usize,
    input_stride: usize,
    output_q3: &mut [u16],
    out: usize,
    width: i32,
    height: i32,
) {
    debug_assert!((height as usize - 1) * CFL_BUF_LINE + width as usize <= CFL_BUF_SQUARE);
    let mut i_off = input;
    let mut o_off = out;
    for _j in 0..height {
        for i in 0..width as usize {
            output_q3[o_off + i] = input_buf[i_off + i] << 3;
        }
        i_off += input_stride;
        o_off += CFL_BUF_LINE;
    }
}

fn cfl_store(
    cfl: &mut CflCtx,
    input_buf: &[u16],
    input: usize,
    input_stride: usize,
    row: i32,
    col: i32,
    tx_size: TxSize,
) -> Result<(), ()> {
    let width = TX_SIZE_WIDE[tx_size as usize] as i32;
    let height = TX_SIZE_HIGH[tx_size as usize] as i32;
    let tx_off_log2 = MI_SIZE_LOG2;
    let sub_x = cfl.subsampling_x;
    let sub_y = cfl.subsampling_y;
    let store_row = row << (tx_off_log2 - sub_y);
    let store_col = col << (tx_off_log2 - sub_x);
    let store_height = height >> sub_y;
    let store_width = width >> sub_x;

    // Invalidate current parameters
    cfl.are_parameters_computed = 0;

    // Store the surface of the pixel buffer that was written to, this way we
    // can manage chroma overrun (e.g. when the chroma surfaces goes beyond the
    // frame boundary)
    if col == 0 && row == 0 {
        cfl.buf_width = store_width;
        cfl.buf_height = store_height;
    } else {
        cfl.buf_width = (store_col + store_width).max(cfl.buf_width);
        cfl.buf_height = (store_row + store_height).max(cfl.buf_height);
    }

    // Check that we will remain inside the pixel buffer.
    debug_assert!(store_row + store_height <= CFL_BUF_LINE as i32);
    debug_assert!(store_col + store_width <= CFL_BUF_LINE as i32);

    // Store the input into the CfL pixel buffer
    let recon_buf_q3 = store_row as usize * CFL_BUF_LINE + store_col as usize;
    // (cfl_subsampling_hbd() and cfl_subsampling_lbd(): the saver's chroma is
    // 4:4:4; the size's function is NULL for the 64-sample sizes)
    if sub_x != 0 || sub_y != 0 || width > 32 || height > 32 {
        return Err(());
    }
    cfl_luma_subsampling_444(
        input_buf,
        input,
        input_stride,
        &mut cfl.recon_buf_q3,
        recon_buf_q3,
        width,
        height,
    );
    Ok(())
}

// Adjust the row and column of blocks smaller than 8X8, as chroma-referenced
// and non-chroma-referenced blocks are stored together in the CfL buffer.
#[inline]
fn sub8x8_adjust_offset(
    cfl: &CflCtx,
    mi_row: i32,
    mi_col: i32,
    row_out: &mut i32,
    col_out: &mut i32,
) {
    // Increment row index for bottom: 8x4, 16x4 or both bottom 4x4s.
    if (mi_row & 0x01) != 0 && cfl.subsampling_y != 0 {
        debug_assert!(*row_out == 0);
        *row_out += 1;
    }

    // Increment col index for right: 4x8, 4x16 or both right 4x4s.
    if (mi_col & 0x01) != 0 && cfl.subsampling_x != 0 {
        debug_assert!(*col_out == 0);
        *col_out += 1;
    }
}

#[inline]
fn max_intra_block_width(
    xd: &MacroBlockD,
    plane_bsize: BlockSize,
    plane: usize,
    tx_size: TxSize,
) -> i32 {
    let max_blocks_wide = max_block_wide(xd, plane_bsize, plane) << MI_SIZE_LOG2;
    let n = TX_SIZE_WIDE_LOG2[tx_size as usize] as i32;
    (max_blocks_wide + ((1 << n) - 1)) & !((1 << n) - 1)
}

#[inline]
fn max_intra_block_height(
    xd: &MacroBlockD,
    plane_bsize: BlockSize,
    plane: usize,
    tx_size: TxSize,
) -> i32 {
    let max_blocks_high = max_block_high(xd, plane_bsize, plane) << MI_SIZE_LOG2;
    let n = TX_SIZE_HIGH_LOG2[tx_size as usize] as i32;
    (max_blocks_high + ((1 << n) - 1)) & !((1 << n) - 1)
}

/// Translation of `cfl_store_block()`.
pub(crate) fn cfl_store_block(
    xd: &mut MacroBlockD,
    bsize: BlockSize,
    tx_size: TxSize,
) -> Result<(), ()> {
    let mut row = 0;
    let mut col = 0;

    if BLOCK_SIZE_HIGH[bsize as usize] == 4 || BLOCK_SIZE_WIDE[bsize as usize] == 4 {
        sub8x8_adjust_offset(&xd.cfl, xd.mi_row, xd.mi_col, &mut row, &mut col);
    }
    let width = max_intra_block_width(xd, bsize, AOM_PLANE_Y, tx_size);
    let height = max_intra_block_height(xd, bsize, AOM_PLANE_Y, tx_size);
    let tx_size = get_tx_size(width, height);
    let pd = &xd.plane[AOM_PLANE_Y];
    let (dst, stride) = (pd.dst.buf, pd.dst.stride);
    let MacroBlockD { cfl, recon, .. } = xd;
    cfl_store(
        cfl,
        &recon.buffer_alloc,
        dst,
        stride as usize,
        row,
        col,
        tx_size,
    )
}

/// Translation of `cfl_store_tx()`.
pub(crate) fn cfl_store_tx(
    xd: &mut MacroBlockD,
    mut row: i32,
    mut col: i32,
    tx_size: TxSize,
    bsize: BlockSize,
) -> Result<(), ()> {
    let pd = &xd.plane[AOM_PLANE_Y];
    let stride = pd.dst.stride;
    let dst = pd.dst.buf + ((row * stride + col) << MI_SIZE_LOG2) as usize;

    if BLOCK_SIZE_HIGH[bsize as usize] == 4 || BLOCK_SIZE_WIDE[bsize as usize] == 4 {
        // Only dimensions of size 4 can have an odd offset.
        debug_assert!(!((col & 1) != 0 && TX_SIZE_WIDE[tx_size as usize] != 4));
        debug_assert!(!((row & 1) != 0 && TX_SIZE_HIGH[tx_size as usize] != 4));
        sub8x8_adjust_offset(&xd.cfl, xd.mi_row, xd.mi_col, &mut row, &mut col);
    }
    let MacroBlockD { cfl, recon, .. } = xd;
    cfl_store(
        cfl,
        &recon.buffer_alloc,
        dst,
        stride as usize,
        row,
        col,
        tx_size,
    )
}
