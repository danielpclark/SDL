// Rust translation of av1/common/av1_inv_txfm2d.c and
// av1/common/av1_inv_txfm1d_cfg.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The 2D inverse transforms (added to the 16-bit prediction) of the sizes
//! the saver reaches, and the lossless 4x4 inverse Walsh-Hadamard
//! transforms. Pixel buffers are a slice, the offset of the block's first
//! sample and the stride.

use super::av1_inv_txfm1d::*;
use super::av1_txfm::*;
use super::enums::*;
use crate::aom::aom_dsp::aom_dsp_common::TranLowT;

pub(crate) fn av1_highbd_iwht4x4_16_add(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    bd: i32,
) {
    /* 4-point reversible, orthonormal inverse Walsh-Hadamard in 3.5 adds,
    0.5 shifts per pixel. */
    let mut output = [0 as TranLowT; 16];
    let (mut a1, mut b1, mut c1, mut d1, mut e1): (
        TranLowT,
        TranLowT,
        TranLowT,
        TranLowT,
        TranLowT,
    );
    let mut ip = 0usize;
    let mut op = 0usize;
    let mut dest_i = dest_off;

    for _i in 0..4 {
        a1 = input[ip] >> UNIT_QUANT_SHIFT;
        c1 = input[ip + 1] >> UNIT_QUANT_SHIFT;
        d1 = input[ip + 2] >> UNIT_QUANT_SHIFT;
        b1 = input[ip + 3] >> UNIT_QUANT_SHIFT;
        a1 += c1;
        d1 -= b1;
        e1 = (a1 - d1) >> 1;
        b1 = e1 - b1;
        c1 = e1 - c1;
        a1 -= b1;
        d1 += c1;

        output[op] = a1;
        output[op + 1] = b1;
        output[op + 2] = c1;
        output[op + 3] = d1;
        ip += 4;
        op += 4;
    }

    ip = 0;
    for _i in 0..4 {
        a1 = output[ip + 4 * 0];
        c1 = output[ip + 4 * 1];
        d1 = output[ip + 4 * 2];
        b1 = output[ip + 4 * 3];
        a1 += c1;
        d1 -= b1;
        e1 = (a1 - d1) >> 1;
        b1 = e1 - b1;
        c1 = e1 - c1;
        a1 -= b1;
        d1 += c1;

        dest[dest_i + stride * 0] = highbd_clip_pixel_add(dest[dest_i + stride * 0], a1 as i64, bd);
        dest[dest_i + stride * 1] = highbd_clip_pixel_add(dest[dest_i + stride * 1], b1 as i64, bd);
        dest[dest_i + stride * 2] = highbd_clip_pixel_add(dest[dest_i + stride * 2], c1 as i64, bd);
        dest[dest_i + stride * 3] = highbd_clip_pixel_add(dest[dest_i + stride * 3], d1 as i64, bd);

        ip += 1;
        dest_i += 1;
    }
}

pub(crate) fn av1_highbd_iwht4x4_1_add(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    dest_stride: usize,
    bd: i32,
) {
    let mut a1: TranLowT;
    let mut e1: TranLowT;
    let mut tmp = [0 as TranLowT; 4];
    let mut dest_i = dest_off;

    a1 = input[0] >> UNIT_QUANT_SHIFT;
    e1 = a1 >> 1;
    a1 -= e1;
    tmp[0] = a1;
    tmp[1] = e1;
    tmp[2] = e1;
    tmp[3] = e1;

    for i in 0..4 {
        e1 = tmp[i] >> 1;
        a1 = tmp[i] - e1;
        dest[dest_i + dest_stride * 0] =
            highbd_clip_pixel_add(dest[dest_i + dest_stride * 0], a1 as i64, bd);
        dest[dest_i + dest_stride * 1] =
            highbd_clip_pixel_add(dest[dest_i + dest_stride * 1], e1 as i64, bd);
        dest[dest_i + dest_stride * 2] =
            highbd_clip_pixel_add(dest[dest_i + dest_stride * 2], e1 as i64, bd);
        dest[dest_i + dest_stride * 3] =
            highbd_clip_pixel_add(dest[dest_i + dest_stride * 3], e1 as i64, bd);
        dest_i += 1;
    }
}

/// Translation of `inv_txfm_type_to_func()` over the 1D transforms the
/// saver reaches (the DCTs of 4 to 32 points).
#[inline]
fn inv_txfm_type_to_func(txfm_type: TxfmType) -> TxfmFunc {
    match txfm_type {
        TXFM_TYPE_DCT4 => av1_idct4,
        TXFM_TYPE_DCT8 => av1_idct8,
        TXFM_TYPE_DCT16 => av1_idct16,
        _ => {
            debug_assert!(txfm_type == TXFM_TYPE_DCT32);
            av1_idct32
        }
    }
}

static INV_SHIFT_4X4: [i8; 2] = [0, -4];
static INV_SHIFT_8X8: [i8; 2] = [-1, -4];
static INV_SHIFT_16X16: [i8; 2] = [-2, -4];
static INV_SHIFT_32X32: [i8; 2] = [-2, -4];
static INV_SHIFT_64X64: [i8; 2] = [-2, -4];
static INV_SHIFT_4X8: [i8; 2] = [0, -4];
static INV_SHIFT_8X4: [i8; 2] = [0, -4];
static INV_SHIFT_8X16: [i8; 2] = [-1, -4];
static INV_SHIFT_16X8: [i8; 2] = [-1, -4];
static INV_SHIFT_16X32: [i8; 2] = [-1, -4];
static INV_SHIFT_32X16: [i8; 2] = [-1, -4];
static INV_SHIFT_32X64: [i8; 2] = [-1, -4];
static INV_SHIFT_64X32: [i8; 2] = [-1, -4];
static INV_SHIFT_4X16: [i8; 2] = [-1, -4];
static INV_SHIFT_16X4: [i8; 2] = [-1, -4];
static INV_SHIFT_8X32: [i8; 2] = [-2, -4];
static INV_SHIFT_32X8: [i8; 2] = [-2, -4];
static INV_SHIFT_16X64: [i8; 2] = [-2, -4];
static INV_SHIFT_64X16: [i8; 2] = [-2, -4];

pub(crate) static AV1_INV_TXFM_SHIFT_LS: [&[i8]; TX_SIZES_ALL as usize] = [
    &INV_SHIFT_4X4,
    &INV_SHIFT_8X8,
    &INV_SHIFT_16X16,
    &INV_SHIFT_32X32,
    &INV_SHIFT_64X64,
    &INV_SHIFT_4X8,
    &INV_SHIFT_8X4,
    &INV_SHIFT_8X16,
    &INV_SHIFT_16X8,
    &INV_SHIFT_16X32,
    &INV_SHIFT_32X16,
    &INV_SHIFT_32X64,
    &INV_SHIFT_64X32,
    &INV_SHIFT_4X16,
    &INV_SHIFT_16X4,
    &INV_SHIFT_8X32,
    &INV_SHIFT_32X8,
    &INV_SHIFT_16X64,
    &INV_SHIFT_64X16,
];

// (av1_inv_txfm1d_cfg.h)
// sum of fwd_shift_##
static INV_START_RANGE: [i8; TX_SIZES_ALL as usize] = [
    5, // 4x4 transform
    6, // 8x8 transform
    7, // 16x16 transform
    7, // 32x32 transform
    7, // 64x64 transform
    5, // 4x8 transform
    5, // 8x4 transform
    6, // 8x16 transform
    6, // 16x8 transform
    6, // 16x32 transform
    6, // 32x16 transform
    6, // 32x64 transform
    6, // 64x32 transform
    6, // 4x16 transform
    6, // 16x4 transform
    7, // 8x32 transform
    7, // 32x8 transform
    7, // 16x64 transform
    7, // 64x16 transform
];

// Values in both av1_inv_cos_bit_col and av1_inv_cos_bit_row are always 12
// for each valid row and col combination
const INV_COS_BIT: i8 = 12;

static IADST4_RANGE: [i8; 7] = [0, 1, 0, 0, 0, 0, 0];

pub(crate) fn av1_get_inv_txfm_cfg(tx_type: TxType, tx_size: TxSize, cfg: &mut Txfm2dFlipCfg) {
    cfg.tx_size = tx_size;
    cfg.stage_range_col = [0; MAX_TXFM_STAGE_NUM];
    cfg.stage_range_row = [0; MAX_TXFM_STAGE_NUM];
    set_flip_cfg(tx_type, cfg);
    let tx_type_1d_col = VTX_TAB[tx_type as usize];
    let tx_type_1d_row = HTX_TAB[tx_type as usize];
    cfg.shift = AV1_INV_TXFM_SHIFT_LS[tx_size as usize];
    let txw_idx = get_txw_idx(tx_size);
    let txh_idx = get_txh_idx(tx_size);
    cfg.cos_bit_col = INV_COS_BIT;
    cfg.cos_bit_row = INV_COS_BIT;
    cfg.txfm_type_col = AV1_TXFM_TYPE_LS[txh_idx][tx_type_1d_col as usize];
    if cfg.txfm_type_col == TXFM_TYPE_ADST4 {
        cfg.stage_range_col[..7].copy_from_slice(&IADST4_RANGE);
    }
    cfg.txfm_type_row = AV1_TXFM_TYPE_LS[txw_idx][tx_type_1d_row as usize];
    if cfg.txfm_type_row == TXFM_TYPE_ADST4 {
        cfg.stage_range_row[..7].copy_from_slice(&IADST4_RANGE);
    }
    cfg.stage_num_col = AV1_TXFM_STAGE_NUM_LIST[cfg.txfm_type_col as usize] as i32;
    cfg.stage_num_row = AV1_TXFM_STAGE_NUM_LIST[cfg.txfm_type_row as usize] as i32;
}

pub(crate) fn av1_gen_inv_stage_range(
    stage_range_col: &mut [i8; MAX_TXFM_STAGE_NUM],
    stage_range_row: &mut [i8; MAX_TXFM_STAGE_NUM],
    cfg: &Txfm2dFlipCfg,
    tx_size: TxSize,
    bd: i32,
) {
    let fwd_shift = INV_START_RANGE[tx_size as usize] as i32;
    let shift = cfg.shift;
    let opt_range_row: i8;
    let opt_range_col: i8;
    if bd == 8 {
        opt_range_row = 16;
        opt_range_col = 16;
    } else if bd == 10 {
        opt_range_row = 18;
        opt_range_col = 16;
    } else {
        debug_assert!(bd == 12);
        opt_range_row = 20;
        opt_range_col = 18;
    }
    // i < MAX_TXFM_STAGE_NUM will mute above array bounds warning
    let mut i = 0;
    while i < cfg.stage_num_row as usize && i < MAX_TXFM_STAGE_NUM {
        let real_range_row = cfg.stage_range_row[i] as i32 + fwd_shift + bd + 1;
        if cfg.txfm_type_row == TXFM_TYPE_ADST4 && i == 1 {
            // the adst4 may use 1 extra bit on top of opt_range_row at stage 1
            // so opt_range_row >= real_range_row will not hold
            stage_range_row[i] = opt_range_row;
        } else {
            debug_assert!(opt_range_row as i32 >= real_range_row);
            stage_range_row[i] = opt_range_row;
        }
        i += 1;
    }
    // i < MAX_TXFM_STAGE_NUM will mute above array bounds warning
    let mut i = 0;
    while i < cfg.stage_num_col as usize && i < MAX_TXFM_STAGE_NUM {
        let real_range_col = cfg.stage_range_col[i] as i32 + fwd_shift + shift[0] as i32 + bd + 1;
        if cfg.txfm_type_col == TXFM_TYPE_ADST4 && i == 1 {
            // the adst4 may use 1 extra bit on top of opt_range_col at stage 1
            // so opt_range_col >= real_range_col will not hold
            stage_range_col[i] = opt_range_col;
        } else {
            debug_assert!(opt_range_col as i32 >= real_range_col);
            stage_range_col[i] = opt_range_col;
        }
        i += 1;
    }
}

#[inline]
fn inv_txfm2d_add_c(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    cfg: &Txfm2dFlipCfg,
    txfm_buf: &mut [i32],
    tx_size: TxSize,
    bd: i32,
) {
    // Note when assigning txfm_size_col, we use the txfm_size from the
    // row configuration and vice versa. This is intentionally done to
    // accurately perform rectangular transforms. When the transform is
    // rectangular, the number of columns will be the same as the
    // txfm_size stored in the row cfg struct. It will make no difference
    // for square transforms.
    let txfm_size_col = TX_SIZE_WIDE[cfg.tx_size as usize];
    let txfm_size_row = TX_SIZE_HIGH[cfg.tx_size as usize];
    // Take the shift from the larger dimension in the rectangular case.
    let shift = cfg.shift;
    let rect_type = get_rect_tx_log_ratio(txfm_size_col, txfm_size_row);
    let mut stage_range_row = [0i8; MAX_TXFM_STAGE_NUM];
    let mut stage_range_col = [0i8; MAX_TXFM_STAGE_NUM];
    debug_assert!(cfg.stage_num_row as usize <= MAX_TXFM_STAGE_NUM);
    debug_assert!(cfg.stage_num_col as usize <= MAX_TXFM_STAGE_NUM);
    av1_gen_inv_stage_range(&mut stage_range_col, &mut stage_range_row, cfg, tx_size, bd);

    let cos_bit_col = cfg.cos_bit_col;
    let cos_bit_row = cfg.cos_bit_row;
    let txfm_func_col = inv_txfm_type_to_func(cfg.txfm_type_col);
    let txfm_func_row = inv_txfm_type_to_func(cfg.txfm_type_row);

    // txfm_buf's length is  txfm_size_row * txfm_size_col + 2 *
    // AOMMAX(txfm_size_row, txfm_size_col)
    // it is used for intermediate data buffering
    let buf_offset = txfm_size_row.max(txfm_size_col) as usize;
    let (temp_in, rest) = txfm_buf.split_at_mut(buf_offset);
    let (temp_out, buf) = rest.split_at_mut(buf_offset);
    let txfm_size_col = txfm_size_col as usize;
    let txfm_size_row = txfm_size_row as usize;
    let mut input_i = 0usize;
    let mut buf_ptr = 0usize;

    // Rows
    for _r in 0..txfm_size_row {
        if rect_type.abs() == 1 {
            for c in 0..txfm_size_col {
                temp_in[c] = round_shift(
                    input[input_i + c] as i64 * NEW_INV_SQRT2 as i64,
                    NEW_SQRT2_BITS,
                );
            }
            clamp_buf(temp_in, txfm_size_col as i32, (bd + 8) as i8);
            txfm_func_row(temp_in, &mut buf[buf_ptr..], cos_bit_row, &stage_range_row);
        } else {
            for c in 0..txfm_size_col {
                temp_in[c] = input[input_i + c];
            }
            clamp_buf(temp_in, txfm_size_col as i32, (bd + 8) as i8);
            txfm_func_row(temp_in, &mut buf[buf_ptr..], cos_bit_row, &stage_range_row);
        }
        av1_round_shift_array(
            &mut buf[buf_ptr..],
            txfm_size_col as i32,
            -(shift[0] as i32),
        );
        input_i += txfm_size_col;
        buf_ptr += txfm_size_col;
    }

    // Columns
    for c in 0..txfm_size_col {
        if cfg.lr_flip == 0 {
            for r in 0..txfm_size_row {
                temp_in[r] = buf[r * txfm_size_col + c];
            }
        } else {
            // flip left right
            for r in 0..txfm_size_row {
                temp_in[r] = buf[r * txfm_size_col + (txfm_size_col - c - 1)];
            }
        }
        clamp_buf(temp_in, txfm_size_row as i32, (bd + 6).max(16) as i8);
        txfm_func_col(temp_in, temp_out, cos_bit_col, &stage_range_col);
        av1_round_shift_array(temp_out, txfm_size_row as i32, -(shift[1] as i32));
        if cfg.ud_flip == 0 {
            for r in 0..txfm_size_row {
                let o = out_off + r * stride + c;
                output[o] = highbd_clip_pixel_add(output[o], temp_out[r] as i64, bd);
            }
        } else {
            // flip upside down
            for r in 0..txfm_size_row {
                let o = out_off + r * stride + c;
                output[o] =
                    highbd_clip_pixel_add(output[o], temp_out[txfm_size_row - r - 1] as i64, bd);
            }
        }
    }
}

#[inline]
fn inv_txfm2d_add_facade(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    txfm_buf: &mut [i32],
    tx_type: TxType,
    tx_size: TxSize,
    bd: i32,
) {
    let mut cfg = Txfm2dFlipCfg::default();
    av1_get_inv_txfm_cfg(tx_type, tx_size, &mut cfg);
    // Forward shift sum uses larger square size, to be consistent with what
    // av1_gen_inv_stage_range() does for inverse shifts.
    inv_txfm2d_add_c(input, output, out_off, stride, &cfg, txfm_buf, tx_size, bd);
}

pub(crate) fn av1_inv_txfm2d_add_8x16(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 8 * 16 + 16 + 16];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_8X16,
        bd,
    );
}

pub(crate) fn av1_inv_txfm2d_add_16x8(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 16 * 8 + 16 + 16];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_16X8,
        bd,
    );
}

pub(crate) fn av1_inv_txfm2d_add_16x32(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 16 * 32 + 32 + 32];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_16X32,
        bd,
    );
}

pub(crate) fn av1_inv_txfm2d_add_32x16(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 32 * 16 + 32 + 32];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_32X16,
        bd,
    );
}

pub(crate) fn av1_inv_txfm2d_add_4x4(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 4 * 4 + 4 + 4];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_4X4,
        bd,
    );
}

pub(crate) fn av1_inv_txfm2d_add_8x8(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 8 * 8 + 8 + 8];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_8X8,
        bd,
    );
}

pub(crate) fn av1_inv_txfm2d_add_16x16(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 16 * 16 + 16 + 16];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_16X16,
        bd,
    );
}

pub(crate) fn av1_inv_txfm2d_add_32x32(
    input: &[i32],
    output: &mut [u16],
    out_off: usize,
    stride: usize,
    tx_type: TxType,
    bd: i32,
) {
    let mut txfm_buf = [0i32; 32 * 32 + 32 + 32];
    inv_txfm2d_add_facade(
        input,
        output,
        out_off,
        stride,
        &mut txfm_buf,
        tx_type,
        TX_32X32,
        bd,
    );
}
