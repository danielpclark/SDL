// Rust translation of av1/common/idct.c and the TxfmParam of
// aom_dsp/txfm_common.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The inverse transform of a block (the high bit depth path; the saver's
//! frames are always high bit depth), for the transform sizes the saver
//! reaches.

use super::av1_inv_txfm2d::*;
use super::blockd::av1_get_ext_tx_set_type;
use super::enums::*;
use crate::aom::aom_dsp::aom_dsp_common::TranLowT;

#[derive(Clone, Copy, Default)]
pub(crate) struct TxfmParam {
    // for both forward and inverse transforms
    pub(crate) tx_type: TxType,
    pub(crate) tx_size: TxSize,
    pub(crate) lossless: i32,
    pub(crate) bd: i32,
    // are the pixel buffers octets or shorts?  This should collapse to
    // bd==8 implies !is_hbd, but that's not certain right now.
    pub(crate) is_hbd: i32,
    pub(crate) tx_set_type: TxSetType,
    // for inverse transforms only
    pub(crate) eob: i32,
}

pub(crate) fn av1_get_tx_scale(tx_size: TxSize) -> i32 {
    let pels = TX_SIZE_2D[tx_size as usize];
    // Largest possible pels is 4096 (64x64).
    (pels > 256) as i32 + (pels > 1024) as i32
}

// NOTE: The implementation of all inverses need to be aware of the fact
// that input and output could be the same buffer.

// idct
pub(crate) fn av1_highbd_iwht4x4_add(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    eob: i32,
    bd: i32,
) {
    if eob > 1 {
        av1_highbd_iwht4x4_16_add(input, dest, dest_off, stride, bd);
    } else {
        av1_highbd_iwht4x4_1_add(input, dest, dest_off, stride, bd);
    }
}

fn av1_highbd_inv_txfm_add_4x4(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    let eob = txfm_param.eob;
    let bd = txfm_param.bd;
    let lossless = txfm_param.lossless;
    let tx_type = txfm_param.tx_type;
    if lossless != 0 {
        debug_assert!(tx_type == DCT_DCT);
        av1_highbd_iwht4x4_add(input, dest, dest_off, stride, eob, bd);
        return Ok(());
    }

    av1_inv_txfm2d_add_4x4(input, dest, dest_off, stride, tx_type, bd)
}

fn av1_highbd_inv_txfm_add_16x32(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    av1_inv_txfm2d_add_16x32(
        input,
        dest,
        dest_off,
        stride,
        txfm_param.tx_type,
        txfm_param.bd,
    )
}

fn av1_highbd_inv_txfm_add_32x16(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    av1_inv_txfm2d_add_32x16(
        input,
        dest,
        dest_off,
        stride,
        txfm_param.tx_type,
        txfm_param.bd,
    )
}

fn av1_highbd_inv_txfm_add_8x8(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    let bd = txfm_param.bd;
    let tx_type = txfm_param.tx_type;

    av1_inv_txfm2d_add_8x8(input, dest, dest_off, stride, tx_type, bd)
}

fn av1_highbd_inv_txfm_add_16x16(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    let bd = txfm_param.bd;
    let tx_type = txfm_param.tx_type;

    av1_inv_txfm2d_add_16x16(input, dest, dest_off, stride, tx_type, bd)
}

fn av1_highbd_inv_txfm_add_8x16(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    av1_inv_txfm2d_add_8x16(
        input,
        dest,
        dest_off,
        stride,
        txfm_param.tx_type,
        txfm_param.bd,
    )
}

fn av1_highbd_inv_txfm_add_16x8(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    av1_inv_txfm2d_add_16x8(
        input,
        dest,
        dest_off,
        stride,
        txfm_param.tx_type,
        txfm_param.bd,
    )
}

fn av1_highbd_inv_txfm_add_32x32(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    let bd = txfm_param.bd;
    let tx_type = txfm_param.tx_type;

    av1_inv_txfm2d_add_32x32(input, dest, dest_off, stride, tx_type, bd)
}

/// Translation of `init_txfm_param()`; it takes the block's
/// `xd->lossless[segment_id]` and `xd->bd` (the destination buffer is
/// borrowed from the same state). The plane is unused upstream too.
fn init_txfm_param(
    lossless: i32,
    bd: i32,
    tx_size: TxSize,
    tx_type: TxType,
    eob: i32,
    reduced_tx_set: i32,
    txfm_param: &mut TxfmParam,
) {
    txfm_param.tx_type = tx_type;
    txfm_param.tx_size = tx_size;
    txfm_param.eob = eob;
    txfm_param.lossless = lossless;
    txfm_param.bd = bd;
    txfm_param.is_hbd = 1;
    txfm_param.tx_set_type = av1_get_ext_tx_set_type(txfm_param.tx_size, 0, reduced_tx_set);
}

/// Translation of `av1_highbd_inv_txfm_add_c()` over the transform sizes
/// the saver reaches.
pub(crate) fn av1_highbd_inv_txfm_add(
    input: &[TranLowT],
    dest: &mut [u16],
    dest_off: usize,
    stride: usize,
    txfm_param: &TxfmParam,
) -> Result<(), ()> {
    let tx_size = txfm_param.tx_size;
    match tx_size {
        TX_32X32 => av1_highbd_inv_txfm_add_32x32(input, dest, dest_off, stride, txfm_param),
        TX_16X16 => av1_highbd_inv_txfm_add_16x16(input, dest, dest_off, stride, txfm_param),
        TX_8X8 => av1_highbd_inv_txfm_add_8x8(input, dest, dest_off, stride, txfm_param),
        TX_8X16 => av1_highbd_inv_txfm_add_8x16(input, dest, dest_off, stride, txfm_param),
        TX_16X8 => av1_highbd_inv_txfm_add_16x8(input, dest, dest_off, stride, txfm_param),
        TX_16X32 => av1_highbd_inv_txfm_add_16x32(input, dest, dest_off, stride, txfm_param),
        TX_32X16 => av1_highbd_inv_txfm_add_32x16(input, dest, dest_off, stride, txfm_param),
        TX_4X4 => {
            // this is like av1_short_idct4x4 but has a special case around eob<=1
            // which is significant (not just an optimization) for the lossless
            // case.
            av1_highbd_inv_txfm_add_4x4(input, dest, dest_off, stride, txfm_param)
        }
        // (the 64-sample, 4:1 and 4-sample rectangular sizes)
        _ => Err(()),
    }
}

/// Translation of `av1_inverse_transform_block()`; see `init_txfm_param()`
/// for the `lossless` and `bd` arguments.
pub(crate) fn av1_inverse_transform_block(
    lossless: i32,
    bd: i32,
    dqcoeff: &[TranLowT],
    tx_type: TxType,
    tx_size: TxSize,
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    eob: i32,
    reduced_tx_set: i32,
) -> Result<(), ()> {
    if eob == 0 {
        return Ok(());
    }

    let mut txfm_param = TxfmParam::default();
    init_txfm_param(
        lossless,
        bd,
        tx_size,
        tx_type,
        eob,
        reduced_tx_set,
        &mut txfm_param,
    );

    av1_highbd_inv_txfm_add(dqcoeff, dst, dst_off, stride, &txfm_param)
}
