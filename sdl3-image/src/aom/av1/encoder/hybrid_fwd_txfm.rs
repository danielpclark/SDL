// Rust translation of the parts of av1/encoder/hybrid_fwd_txfm.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The forward transform of a residual block (high bit depth: the saver's
//! frames are 10-bit), for the transform sizes the saver reaches, and the
//! lossless Walsh-Hadamard transform.

use super::av1_fwd_txfm2d::*;
use crate::aom::aom_dsp::aom_dsp_common::{TranHighT, TranLowT};
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::idct::TxfmParam;

/* 4-point reversible, orthonormal Walsh-Hadamard in 3.5 adds, 0.5 shifts per
pixel. */
pub(crate) fn av1_fwht4x4(input: &[i16], output: &mut [TranLowT], stride: usize) {
    let (mut a1, mut b1, mut c1, mut d1, mut e1): (
        TranHighT,
        TranHighT,
        TranHighT,
        TranHighT,
        TranHighT,
    );

    for i in 0..4 {
        a1 = input[i] as TranHighT;
        b1 = input[stride + i] as TranHighT;
        c1 = input[2 * stride + i] as TranHighT;
        d1 = input[3 * stride + i] as TranHighT;

        a1 += b1;
        d1 -= c1;
        e1 = (a1 - d1) >> 1;
        b1 = e1 - b1;
        c1 = e1 - c1;
        a1 -= c1;
        d1 += b1;
        output[i] = a1 as TranLowT;
        output[4 + i] = c1 as TranLowT;
        output[8 + i] = d1 as TranLowT;
        output[12 + i] = b1 as TranLowT;
    }

    for i in 0..4 {
        let ip = 4 * i;
        a1 = output[ip] as TranHighT;
        b1 = output[ip + 1] as TranHighT;
        c1 = output[ip + 2] as TranHighT;
        d1 = output[ip + 3] as TranHighT;

        a1 += b1;
        d1 -= c1;
        e1 = (a1 - d1) >> 1;
        b1 = e1 - b1;
        c1 = e1 - c1;
        a1 -= c1;
        d1 += b1;
        output[ip] = (a1 * UNIT_QUANT_FACTOR as TranHighT) as TranLowT;
        output[ip + 1] = (c1 * UNIT_QUANT_FACTOR as TranHighT) as TranLowT;
        output[ip + 2] = (d1 * UNIT_QUANT_FACTOR as TranHighT) as TranLowT;
        output[ip + 3] = (b1 * UNIT_QUANT_FACTOR as TranHighT) as TranLowT;
    }
}

pub(crate) fn av1_highbd_fwht4x4(input: &[i16], output: &mut [TranLowT], stride: usize) {
    av1_fwht4x4(input, output, stride);
}

fn highbd_fwd_txfm_4x4(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    let tx_type = txfm_param.tx_type;
    let bd = txfm_param.bd;
    if txfm_param.lossless != 0 {
        debug_assert!(tx_type == DCT_DCT);
        av1_highbd_fwht4x4(src_diff, coeff, diff_stride);
        return;
    }
    av1_fwd_txfm2d_4x4(src_diff, coeff, diff_stride, tx_type, bd);
}

fn highbd_fwd_txfm_8x16(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    let tx_type = txfm_param.tx_type;
    let bd = txfm_param.bd;
    av1_fwd_txfm2d_8x16(src_diff, coeff, diff_stride, tx_type, bd);
}

fn highbd_fwd_txfm_16x8(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    let tx_type = txfm_param.tx_type;
    let bd = txfm_param.bd;
    av1_fwd_txfm2d_16x8(src_diff, coeff, diff_stride, tx_type, bd);
}

fn highbd_fwd_txfm_16x32(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    av1_fwd_txfm2d_16x32(
        src_diff,
        coeff,
        diff_stride,
        txfm_param.tx_type,
        txfm_param.bd,
    );
}

fn highbd_fwd_txfm_32x16(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    av1_fwd_txfm2d_32x16(
        src_diff,
        coeff,
        diff_stride,
        txfm_param.tx_type,
        txfm_param.bd,
    );
}

fn highbd_fwd_txfm_8x8(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    let tx_type = txfm_param.tx_type;
    let bd = txfm_param.bd;
    av1_fwd_txfm2d_8x8(src_diff, coeff, diff_stride, tx_type, bd);
}

fn highbd_fwd_txfm_16x16(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    let tx_type = txfm_param.tx_type;
    let bd = txfm_param.bd;
    av1_fwd_txfm2d_16x16(src_diff, coeff, diff_stride, tx_type, bd);
}

fn highbd_fwd_txfm_32x32(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    let tx_type = txfm_param.tx_type;
    let bd = txfm_param.bd;
    av1_fwd_txfm2d_32x32(src_diff, coeff, diff_stride, tx_type, bd);
}

/// Translation of `av1_fwd_txfm()` (the saver's bit depth is 10, never 8).
pub(crate) fn av1_fwd_txfm(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    debug_assert!(txfm_param.bd != 8);
    av1_highbd_fwd_txfm(src_diff, coeff, diff_stride, txfm_param);
}

/// Translation of `av1_highbd_fwd_txfm()` over the transform sizes the
/// saver reaches.
pub(crate) fn av1_highbd_fwd_txfm(
    src_diff: &[i16],
    coeff: &mut [TranLowT],
    diff_stride: usize,
    txfm_param: &TxfmParam,
) {
    let tx_size = txfm_param.tx_size;
    match tx_size {
        TX_32X32 => highbd_fwd_txfm_32x32(src_diff, coeff, diff_stride, txfm_param),
        TX_16X16 => highbd_fwd_txfm_16x16(src_diff, coeff, diff_stride, txfm_param),
        TX_8X8 => highbd_fwd_txfm_8x8(src_diff, coeff, diff_stride, txfm_param),
        TX_8X16 => highbd_fwd_txfm_8x16(src_diff, coeff, diff_stride, txfm_param),
        TX_16X8 => highbd_fwd_txfm_16x8(src_diff, coeff, diff_stride, txfm_param),
        TX_16X32 => highbd_fwd_txfm_16x32(src_diff, coeff, diff_stride, txfm_param),
        TX_32X16 => highbd_fwd_txfm_32x16(src_diff, coeff, diff_stride, txfm_param),
        TX_4X4 => highbd_fwd_txfm_4x4(src_diff, coeff, diff_stride, txfm_param),
        _ => debug_assert!(false),
    }
}
