// Rust translation of av1/encoder/av1_fwd_txfm2d.c and
// av1/encoder/av1_fwd_txfm1d_cfg.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The 2D forward transforms of the sizes the saver reaches (all DCTs, at
//! most 32 samples a side).

use super::av1_fwd_txfm1d::*;
use crate::aom::av1::common::av1_txfm::*;
use crate::aom::av1::common::enums::*;

/// Translation of `fwd_txfm_type_to_func()` over the 1D transforms the
/// saver reaches (the DCTs of 4 to 32 points).
#[inline]
fn fwd_txfm_type_to_func(txfm_type: TxfmType) -> TxfmFunc {
    match txfm_type {
        TXFM_TYPE_DCT4 => av1_fdct4,
        TXFM_TYPE_DCT8 => av1_fdct8,
        TXFM_TYPE_DCT16 => av1_fdct16,
        _ => {
            debug_assert!(txfm_type == TXFM_TYPE_DCT32);
            av1_fdct32
        }
    }
}

pub(crate) fn av1_gen_fwd_stage_range(
    stage_range_col: &mut [i8; MAX_TXFM_STAGE_NUM],
    stage_range_row: &mut [i8; MAX_TXFM_STAGE_NUM],
    cfg: &Txfm2dFlipCfg,
    bd: i32,
) {
    // Take the shift from the larger dimension in the rectangular case.
    let shift = cfg.shift;
    // i < MAX_TXFM_STAGE_NUM will mute above array bounds warning
    let mut i = 0;
    while i < cfg.stage_num_col as usize && i < MAX_TXFM_STAGE_NUM {
        stage_range_col[i] = (cfg.stage_range_col[i] as i32 + shift[0] as i32 + bd + 1) as i8;
        i += 1;
    }

    // i < MAX_TXFM_STAGE_NUM will mute above array bounds warning
    let mut i = 0;
    while i < cfg.stage_num_row as usize && i < MAX_TXFM_STAGE_NUM {
        stage_range_row[i] =
            (cfg.stage_range_row[i] as i32 + shift[0] as i32 + shift[1] as i32 + bd + 1) as i8;
        i += 1;
    }
}

#[inline]
fn fwd_txfm2d_c(
    input: &[i16],
    output: &mut [i32],
    stride: usize,
    cfg: &Txfm2dFlipCfg,
    buf: &mut [i32],
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
    let mut stage_range_col = [0i8; MAX_TXFM_STAGE_NUM];
    let mut stage_range_row = [0i8; MAX_TXFM_STAGE_NUM];
    debug_assert!(cfg.stage_num_col as usize <= MAX_TXFM_STAGE_NUM);
    debug_assert!(cfg.stage_num_row as usize <= MAX_TXFM_STAGE_NUM);
    av1_gen_fwd_stage_range(&mut stage_range_col, &mut stage_range_row, cfg, bd);

    let cos_bit_col = cfg.cos_bit_col;
    let cos_bit_row = cfg.cos_bit_row;
    let txfm_func_col = fwd_txfm_type_to_func(cfg.txfm_type_col);
    let txfm_func_row = fwd_txfm_type_to_func(cfg.txfm_type_row);
    let txfm_size_col = txfm_size_col as usize;
    let txfm_size_row = txfm_size_row as usize;

    // use output buffer as temp buffer
    {
        let (temp_in, temp_out) = output.split_at_mut(txfm_size_row);

        // Columns
        for c in 0..txfm_size_col {
            if cfg.ud_flip == 0 {
                for r in 0..txfm_size_row {
                    temp_in[r] = input[r * stride + c] as i32;
                }
            } else {
                for r in 0..txfm_size_row {
                    // flip upside down
                    temp_in[r] = input[(txfm_size_row - r - 1) * stride + c] as i32;
                }
            }
            av1_round_shift_array(temp_in, txfm_size_row as i32, -(shift[0] as i32));
            txfm_func_col(temp_in, temp_out, cos_bit_col, &stage_range_col);
            av1_round_shift_array(temp_out, txfm_size_row as i32, -(shift[1] as i32));
            if cfg.lr_flip == 0 {
                for r in 0..txfm_size_row {
                    buf[r * txfm_size_col + c] = temp_out[r];
                }
            } else {
                for r in 0..txfm_size_row {
                    // flip from left to right
                    buf[r * txfm_size_col + (txfm_size_col - c - 1)] = temp_out[r];
                }
            }
        }
    }

    // Rows
    for r in 0..txfm_size_row {
        let o = r * txfm_size_col;
        txfm_func_row(&buf[o..], &mut output[o..], cos_bit_row, &stage_range_row);
        av1_round_shift_array(&mut output[o..], txfm_size_col as i32, -(shift[2] as i32));
        if rect_type.abs() == 1 {
            // Multiply everything by Sqrt2 if the transform is rectangular and the
            // size difference is a factor of 2.
            for c in 0..txfm_size_col {
                output[o + c] =
                    round_shift(output[o + c] as i64 * NEW_SQRT2 as i64, NEW_SQRT2_BITS);
            }
        }
    }
}

macro_rules! fwd_txfm2d_fn {
    ($name:ident, $w:expr, $h:expr, $tx:expr) => {
        pub(crate) fn $name(
            input: &[i16],
            output: &mut [i32],
            stride: usize,
            tx_type: TxType,
            bd: i32,
        ) {
            let mut txfm_buf = [0i32; $w * $h];
            let mut cfg = Txfm2dFlipCfg::default();
            av1_get_fwd_txfm_cfg(tx_type, $tx, &mut cfg);
            fwd_txfm2d_c(input, output, stride, &cfg, &mut txfm_buf, bd);
        }
    };
}

fwd_txfm2d_fn!(av1_fwd_txfm2d_8x16, 8, 16, TX_8X16);
fwd_txfm2d_fn!(av1_fwd_txfm2d_16x8, 16, 8, TX_16X8);
fwd_txfm2d_fn!(av1_fwd_txfm2d_16x32, 16, 32, TX_16X32);
fwd_txfm2d_fn!(av1_fwd_txfm2d_32x16, 32, 16, TX_32X16);
fwd_txfm2d_fn!(av1_fwd_txfm2d_4x4, 4, 4, TX_4X4);
fwd_txfm2d_fn!(av1_fwd_txfm2d_8x8, 8, 8, TX_8X8);
fwd_txfm2d_fn!(av1_fwd_txfm2d_16x16, 16, 16, TX_16X16);
fwd_txfm2d_fn!(av1_fwd_txfm2d_32x32, 32, 32, TX_32X32);

static FWD_SHIFT_4X4: [i8; 3] = [2, 0, 0];
static FWD_SHIFT_8X8: [i8; 3] = [2, -1, 0];
static FWD_SHIFT_16X16: [i8; 3] = [2, -2, 0];
static FWD_SHIFT_32X32: [i8; 3] = [2, -4, 0];
static FWD_SHIFT_64X64: [i8; 3] = [0, -2, -2];
static FWD_SHIFT_4X8: [i8; 3] = [2, -1, 0];
static FWD_SHIFT_8X4: [i8; 3] = [2, -1, 0];
static FWD_SHIFT_8X16: [i8; 3] = [2, -2, 0];
static FWD_SHIFT_16X8: [i8; 3] = [2, -2, 0];
static FWD_SHIFT_16X32: [i8; 3] = [2, -4, 0];
static FWD_SHIFT_32X16: [i8; 3] = [2, -4, 0];
static FWD_SHIFT_32X64: [i8; 3] = [0, -2, -2];
static FWD_SHIFT_64X32: [i8; 3] = [2, -4, -2];
static FWD_SHIFT_4X16: [i8; 3] = [2, -1, 0];
static FWD_SHIFT_16X4: [i8; 3] = [2, -1, 0];
static FWD_SHIFT_8X32: [i8; 3] = [2, -2, 0];
static FWD_SHIFT_32X8: [i8; 3] = [2, -2, 0];
static FWD_SHIFT_16X64: [i8; 3] = [0, -2, 0];
static FWD_SHIFT_64X16: [i8; 3] = [2, -4, 0];

pub(crate) static AV1_FWD_TXFM_SHIFT_LS: [&[i8]; TX_SIZES_ALL as usize] = [
    &FWD_SHIFT_4X4,
    &FWD_SHIFT_8X8,
    &FWD_SHIFT_16X16,
    &FWD_SHIFT_32X32,
    &FWD_SHIFT_64X64,
    &FWD_SHIFT_4X8,
    &FWD_SHIFT_8X4,
    &FWD_SHIFT_8X16,
    &FWD_SHIFT_16X8,
    &FWD_SHIFT_16X32,
    &FWD_SHIFT_32X16,
    &FWD_SHIFT_32X64,
    &FWD_SHIFT_64X32,
    &FWD_SHIFT_4X16,
    &FWD_SHIFT_16X4,
    &FWD_SHIFT_8X32,
    &FWD_SHIFT_32X8,
    &FWD_SHIFT_16X64,
    &FWD_SHIFT_64X16,
];

pub(crate) static AV1_FWD_COS_BIT_COL: [[i8; MAX_TXWH_IDX /*txh_idx*/]; MAX_TXWH_IDX /*txw_idx*/] = [
    [13, 13, 13, 0, 0],
    [13, 13, 13, 12, 0],
    [13, 13, 13, 12, 13],
    [0, 13, 13, 12, 13],
    [0, 0, 13, 12, 13],
];

pub(crate) static AV1_FWD_COS_BIT_ROW: [[i8; MAX_TXWH_IDX /*txh_idx*/]; MAX_TXWH_IDX /*txw_idx*/] = [
    [13, 13, 12, 0, 0],
    [13, 13, 13, 12, 0],
    [13, 13, 12, 13, 12],
    [0, 12, 13, 12, 11],
    [0, 0, 12, 11, 10],
];

static FDCT4_RANGE_MULT2: [i8; 4] = [0, 2, 3, 3];
static FDCT8_RANGE_MULT2: [i8; 6] = [0, 2, 4, 5, 5, 5];
static FDCT16_RANGE_MULT2: [i8; 8] = [0, 2, 4, 6, 7, 7, 7, 7];
static FDCT32_RANGE_MULT2: [i8; 10] = [0, 2, 4, 6, 8, 9, 9, 9, 9, 9];
static FDCT64_RANGE_MULT2: [i8; 12] = [0, 2, 4, 6, 8, 10, 11, 11, 11, 11, 11, 11];

static FADST4_RANGE_MULT2: [i8; 7] = [0, 2, 4, 3, 3, 3, 3];
static FADST8_RANGE_MULT2: [i8; 8] = [0, 0, 1, 3, 3, 5, 5, 5];
static FADST16_RANGE_MULT2: [i8; 10] = [0, 0, 1, 3, 3, 5, 5, 7, 7, 7];

static FIDTX4_RANGE_MULT2: [i8; 1] = [1];
static FIDTX8_RANGE_MULT2: [i8; 1] = [2];
static FIDTX16_RANGE_MULT2: [i8; 1] = [3];
static FIDTX32_RANGE_MULT2: [i8; 1] = [4];

static FWD_TXFM_RANGE_MULT2_LIST: [&[i8]; TXFM_TYPES as usize] = [
    &FDCT4_RANGE_MULT2,
    &FDCT8_RANGE_MULT2,
    &FDCT16_RANGE_MULT2,
    &FDCT32_RANGE_MULT2,
    &FDCT64_RANGE_MULT2,
    &FADST4_RANGE_MULT2,
    &FADST8_RANGE_MULT2,
    &FADST16_RANGE_MULT2,
    &FIDTX4_RANGE_MULT2,
    &FIDTX8_RANGE_MULT2,
    &FIDTX16_RANGE_MULT2,
    &FIDTX32_RANGE_MULT2,
];

#[inline]
fn set_fwd_txfm_non_scale_range(cfg: &mut Txfm2dFlipCfg) {
    cfg.stage_range_col = [0; MAX_TXFM_STAGE_NUM];
    cfg.stage_range_row = [0; MAX_TXFM_STAGE_NUM];

    let range_mult2_col = FWD_TXFM_RANGE_MULT2_LIST[cfg.txfm_type_col as usize];
    if cfg.txfm_type_col != TXFM_TYPE_INVALID {
        let stage_num_col = cfg.stage_num_col as usize;
        for i in 0..stage_num_col {
            cfg.stage_range_col[i] = (range_mult2_col[i] + 1) >> 1;
        }
    }

    if cfg.txfm_type_row != TXFM_TYPE_INVALID {
        let stage_num_row = cfg.stage_num_row as usize;
        let range_mult2_row = FWD_TXFM_RANGE_MULT2_LIST[cfg.txfm_type_row as usize];
        for i in 0..stage_num_row {
            cfg.stage_range_row[i] =
                (range_mult2_col[cfg.stage_num_col as usize - 1] + range_mult2_row[i] + 1) >> 1;
        }
    }
}

pub(crate) fn av1_get_fwd_txfm_cfg(tx_type: TxType, tx_size: TxSize, cfg: &mut Txfm2dFlipCfg) {
    cfg.tx_size = tx_size;
    set_flip_cfg(tx_type, cfg);
    let tx_type_1d_col = VTX_TAB[tx_type as usize];
    let tx_type_1d_row = HTX_TAB[tx_type as usize];
    let txw_idx = get_txw_idx(tx_size);
    let txh_idx = get_txh_idx(tx_size);
    cfg.shift = AV1_FWD_TXFM_SHIFT_LS[tx_size as usize];
    cfg.cos_bit_col = AV1_FWD_COS_BIT_COL[txw_idx][txh_idx];
    cfg.cos_bit_row = AV1_FWD_COS_BIT_ROW[txw_idx][txh_idx];
    cfg.txfm_type_col = AV1_TXFM_TYPE_LS[txh_idx][tx_type_1d_col as usize];
    cfg.txfm_type_row = AV1_TXFM_TYPE_LS[txw_idx][tx_type_1d_row as usize];
    cfg.stage_num_col = AV1_TXFM_STAGE_NUM_LIST[cfg.txfm_type_col as usize] as i32;
    cfg.stage_num_row = AV1_TXFM_STAGE_NUM_LIST[cfg.txfm_type_row as usize] as i32;
    set_fwd_txfm_non_scale_range(cfg);
}
