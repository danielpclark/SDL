// Rust translation of the parts of av1/encoder/av1_quantize.c and
// av1/encoder/av1_quantize.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Quantization: the frame's quantizer tables, the block quantizer setup
//! and the "fp" and "b" quantizers of high bit depth coefficients (the
//! quantizer matrices are always flat, and the adaptive "b" quantizer is
//! off). The sad-per-bit factor of motion search is not set.

use super::block::{Macroblock, MacroblockPlane};
use super::encoder::Av1Comp;
use super::rd::{av1_compute_rd_mult, av1_set_error_per_bit};
use crate::aom::aom_dsp::aom_dsp_common::{clamp64, round_power_of_two, TranLowT};
use crate::aom::aom_dsp::quantize::aom_highbd_quantize_b;
use crate::aom::av1::common::av1_common_int::{Av1Common, CommonQuantParams};
use crate::aom::av1::common::blockd::MacroBlockD;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::quant_common::*;
use crate::aom::av1::common::scan::ScanOrder;
use crate::aom::av1::common::seg_common::{segfeature_active, SEG_LVL_SKIP};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct QuantParam {
    pub(crate) log_scale: i32,
    pub(crate) tx_size: TxSize,
    pub(crate) qmatrix: Option<&'static [u8]>,
    pub(crate) iqmatrix: Option<&'static [u8]>,
    pub(crate) use_quant_b_adapt: i32,
    pub(crate) use_optimize_b: i32,
    pub(crate) xform_quant_idx: i32,
}

// (AV1_XFORM_QUANT, encodemb.h)
pub(crate) const AV1_XFORM_QUANT_FP: i32 = 0;
pub(crate) const AV1_XFORM_QUANT_B: i32 = 1;
pub(crate) const AV1_XFORM_QUANT_DC: i32 = 2;
pub(crate) const AV1_XFORM_QUANT_SKIP_QUANT: i32 = 3;
pub(crate) const AV1_XFORM_QUANT_TYPES: i32 = 4;

const QR: usize = QINDEX_RANGE as usize;

// The QUANTS structure is used only for internal quantizer setup in
// av1_quantize.c.
// All of its fields use the same coefficient shift/scaling at TX.
#[derive(Debug, Clone)]
pub(crate) struct Quants {
    // 0: dc 1: ac 2-8: ac repeated to SIMD width
    pub(crate) y_quant: [[i16; 8]; QR],
    pub(crate) y_quant_shift: [[i16; 8]; QR],
    pub(crate) y_zbin: [[i16; 8]; QR],
    pub(crate) y_round: [[i16; 8]; QR],

    // TODO(jingning): in progress of re-working the quantization. will decide
    // if we want to deprecate the current use of y_quant.
    pub(crate) y_quant_fp: [[i16; 8]; QR],
    pub(crate) u_quant_fp: [[i16; 8]; QR],
    pub(crate) v_quant_fp: [[i16; 8]; QR],
    pub(crate) y_round_fp: [[i16; 8]; QR],
    pub(crate) u_round_fp: [[i16; 8]; QR],
    pub(crate) v_round_fp: [[i16; 8]; QR],

    pub(crate) u_quant: [[i16; 8]; QR],
    pub(crate) v_quant: [[i16; 8]; QR],
    pub(crate) u_quant_shift: [[i16; 8]; QR],
    pub(crate) v_quant_shift: [[i16; 8]; QR],
    pub(crate) u_zbin: [[i16; 8]; QR],
    pub(crate) v_zbin: [[i16; 8]; QR],
    pub(crate) u_round: [[i16; 8]; QR],
    pub(crate) v_round: [[i16; 8]; QR],
}

// The Dequants structure is used only for internal quantizer setup in
// av1_quantize.c.
// Fields are suffixed according to whether or not they're expressed in
// the same coefficient shift/precision as TX or a fixed Q3 format.
#[derive(Debug, Clone)]
pub(crate) struct Dequants {
    pub(crate) y_dequant_qtx: [[i16; 8]; QR], // 8: SIMD width
    pub(crate) u_dequant_qtx: [[i16; 8]; QR], // 8: SIMD width
    pub(crate) v_dequant_qtx: [[i16; 8]; QR], // 8: SIMD width
}

#[derive(Debug, Clone)]
pub(crate) struct EncQuantDequantParams {
    /// Quantization parameters.
    pub(crate) quants: Quants,
    /// Dequantization parameters.
    pub(crate) dequants: Dequants,
}

impl Default for EncQuantDequantParams {
    fn default() -> Self {
        let z = [[0i16; 8]; QR];
        EncQuantDequantParams {
            quants: Quants {
                y_quant: z,
                y_quant_shift: z,
                y_zbin: z,
                y_round: z,
                y_quant_fp: z,
                u_quant_fp: z,
                v_quant_fp: z,
                y_round_fp: z,
                u_round_fp: z,
                v_round_fp: z,
                u_quant: z,
                v_quant: z,
                u_quant_shift: z,
                v_quant_shift: z,
                u_zbin: z,
                v_zbin: z,
                u_round: z,
                v_round: z,
            },
            dequants: Dequants {
                y_dequant_qtx: z,
                u_dequant_qtx: z,
                v_dequant_qtx: z,
            },
        }
    }
}

#[inline]
fn aomsign(x: i32) -> i32 {
    x >> 31
}

pub(crate) fn av1_quantize_skip(
    n_coeffs: i32,
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    eob_ptr: &mut u16,
) {
    qcoeff_ptr[..n_coeffs as usize].fill(0);
    dqcoeff_ptr[..n_coeffs as usize].fill(0);
    *eob_ptr = 0;
}

pub(crate) fn av1_quantize_fp_no_qmatrix(
    quant_ptr: &[i16],
    dequant_ptr: &[i16],
    round_ptr: &[i16],
    log_scale: i32,
    scan: &[i16],
    coeff_count: i32,
    coeff_ptr: &[TranLowT],
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
) -> i32 {
    qcoeff_ptr[..coeff_count as usize].fill(0);
    dqcoeff_ptr[..coeff_count as usize].fill(0);
    let rounding = [
        round_power_of_two(round_ptr[0] as i32, log_scale),
        round_power_of_two(round_ptr[1] as i32, log_scale),
    ];
    let mut eob = 0;
    for i in 0..coeff_count as usize {
        let rc = scan[i] as usize;
        let rc01 = (rc != 0) as usize;
        let thresh = dequant_ptr[rc01] as i32;
        let coeff = coeff_ptr[rc];
        let coeff_sign = aomsign(coeff);
        let mut abs_coeff: i64 = ((coeff ^ coeff_sign) - coeff_sign) as i64;
        let mut tmp32: i32 = 0;
        if (abs_coeff << (1 + log_scale)) >= thresh as i64 {
            abs_coeff = clamp64(
                abs_coeff + rounding[rc01] as i64,
                i16::MIN as i64,
                i16::MAX as i64,
            );
            tmp32 = ((abs_coeff * quant_ptr[rc01] as i64) >> (16 - log_scale)) as i32;
            if tmp32 != 0 {
                qcoeff_ptr[rc] = (tmp32 ^ coeff_sign) - coeff_sign;
                let abs_dqcoeff: TranLowT = (tmp32 * dequant_ptr[rc01] as i32) >> log_scale;
                dqcoeff_ptr[rc] = (abs_dqcoeff ^ coeff_sign) - coeff_sign;
            }
        }
        if tmp32 != 0 {
            eob = i as i32 + 1;
        }
    }
    eob
}

/// Translation of `quantize_fp_helper_c()` with flat quantizer matrices.
fn quantize_fp_helper_c(
    coeff_ptr: &[TranLowT],
    n_coeffs: i32,
    round_ptr: &[i16],
    quant_ptr: &[i16],
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    dequant_ptr: &[i16],
    eob_ptr: &mut u16,
    scan: &[i16],
    log_scale: i32,
) {
    // TODO(jingning) Decide the need of these arguments after the
    // quantization process is completed.

    qcoeff_ptr[..n_coeffs as usize].fill(0);
    dqcoeff_ptr[..n_coeffs as usize].fill(0);

    *eob_ptr = av1_quantize_fp_no_qmatrix(
        quant_ptr,
        dequant_ptr,
        round_ptr,
        log_scale,
        scan,
        n_coeffs,
        coeff_ptr,
        qcoeff_ptr,
        dqcoeff_ptr,
    ) as u16;
}

/// Translation of `highbd_quantize_fp_helper_c()` with flat quantizer
/// matrices.
fn highbd_quantize_fp_helper_c(
    coeff_ptr: &[TranLowT],
    count: i32,
    round_ptr: &[i16],
    quant_ptr: &[i16],
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    dequant_ptr: &[i16],
    eob_ptr: &mut u16,
    scan: &[i16],
    log_scale: i32,
) {
    let mut eob: i32 = -1;
    let shift = 16 - log_scale;
    // TODO(jingning) Decide the need of these arguments after the
    // quantization process is completed.

    let log_scaled_round_arr = [
        round_power_of_two(round_ptr[0] as i32, log_scale),
        round_power_of_two(round_ptr[1] as i32, log_scale),
    ];
    for i in 0..count as usize {
        let rc = scan[i] as usize;
        let coeff = coeff_ptr[rc];
        let rc01 = (rc != 0) as usize;
        let coeff_sign = aomsign(coeff);
        let abs_coeff = (coeff ^ coeff_sign) - coeff_sign;
        let log_scaled_round = log_scaled_round_arr[rc01];
        if (abs_coeff << (1 + log_scale)) >= dequant_ptr[rc01] as i32 {
            let quant = quant_ptr[rc01] as i32;
            let dequant = dequant_ptr[rc01] as i32;
            let tmp: i64 = abs_coeff as i64 + log_scaled_round as i64;
            let abs_qcoeff = ((tmp * quant as i64) >> shift) as i32;
            qcoeff_ptr[rc] = (abs_qcoeff ^ coeff_sign) - coeff_sign;
            let abs_dqcoeff: TranLowT = (abs_qcoeff * dequant) >> log_scale;
            if abs_qcoeff != 0 {
                eob = i as i32;
            }
            dqcoeff_ptr[rc] = (abs_dqcoeff ^ coeff_sign) - coeff_sign;
        } else {
            qcoeff_ptr[rc] = 0;
            dqcoeff_ptr[rc] = 0;
        }
    }
    *eob_ptr = (eob + 1) as u16;
}

/// Translation of `av1_quantize_fp_c()` (zbin, quant_shift and iscan are
/// unused upstream too).
pub(crate) fn av1_quantize_fp(
    coeff_ptr: &[TranLowT],
    n_coeffs: i32,
    round_ptr: &[i16],
    quant_ptr: &[i16],
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    dequant_ptr: &[i16],
    eob_ptr: &mut u16,
    scan: &[i16],
) {
    quantize_fp_helper_c(
        coeff_ptr,
        n_coeffs,
        round_ptr,
        quant_ptr,
        qcoeff_ptr,
        dqcoeff_ptr,
        dequant_ptr,
        eob_ptr,
        scan,
        0,
    );
}

/// Translation of `av1_highbd_quantize_fp_facade()` (flat matrices).
pub(crate) fn av1_highbd_quantize_fp_facade(
    coeff_ptr: &[TranLowT],
    n_coeffs: i32,
    p: &MacroblockPlane,
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    eob_ptr: &mut u16,
    sc: &ScanOrder,
    qparam: &QuantParam,
) {
    debug_assert!(qparam.qmatrix.is_none() && qparam.iqmatrix.is_none());
    av1_highbd_quantize_fp(
        coeff_ptr,
        n_coeffs,
        &p.round_fp_qtx,
        &p.quant_fp_qtx,
        qcoeff_ptr,
        dqcoeff_ptr,
        &p.dequant_qtx,
        eob_ptr,
        sc.scan,
        qparam.log_scale,
    );
}

/// Translation of `av1_highbd_quantize_b_facade()` (flat matrices, no
/// adaptive quantization). The "b" quantizer only quantizes the lossless
/// 4x4 blocks (lossy blocks use trellis and the "fp" quantizer), so the
/// log scale is always 0.
pub(crate) fn av1_highbd_quantize_b_facade(
    coeff_ptr: &[TranLowT],
    n_coeffs: i32,
    p: &MacroblockPlane,
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    eob_ptr: &mut u16,
    sc: &ScanOrder,
    qparam: &QuantParam,
) {
    debug_assert!(qparam.use_quant_b_adapt == 0);
    debug_assert!(qparam.qmatrix.is_none() && qparam.iqmatrix.is_none());
    match qparam.log_scale {
        0 => aom_highbd_quantize_b(
            coeff_ptr,
            n_coeffs as isize,
            &p.zbin_qtx,
            &p.round_qtx,
            &p.quant_qtx,
            &p.quant_shift_qtx,
            qcoeff_ptr,
            dqcoeff_ptr,
            &p.dequant_qtx,
            eob_ptr,
            sc.scan,
            sc.iscan,
        ),
        _ => debug_assert!(false),
    }
}

/// Translation of `av1_highbd_quantize_fp_c()`.
pub(crate) fn av1_highbd_quantize_fp(
    coeff_ptr: &[TranLowT],
    count: i32,
    round_ptr: &[i16],
    quant_ptr: &[i16],
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    dequant_ptr: &[i16],
    eob_ptr: &mut u16,
    scan: &[i16],
    log_scale: i32,
) {
    highbd_quantize_fp_helper_c(
        coeff_ptr,
        count,
        round_ptr,
        quant_ptr,
        qcoeff_ptr,
        dqcoeff_ptr,
        dequant_ptr,
        eob_ptr,
        scan,
        log_scale,
    );
}

fn invert_quant(quant: &mut i16, shift: &mut i16, d: i32) {
    let mut t: u32 = d as u32;
    let mut l: i32 = 0;
    while t > 1 {
        t >>= 1;
        l += 1;
    }
    let m = 1 + (1i32 << (16 + l)) / d;
    *quant = (m - (1 << 16)) as i16;
    *shift = (1i32 << (16 - l)) as i16;
}

fn get_qzbin_factor(q: i32, bit_depth: i32) -> i32 {
    let quant = av1_dc_quant_qtx(q, 0, bit_depth) as i32;
    match bit_depth {
        8 => {
            if q == 0 {
                64
            } else if quant < 148 {
                84
            } else {
                80
            }
        }
        10 => {
            if q == 0 {
                64
            } else if quant < 592 {
                84
            } else {
                80
            }
        }
        12 => {
            if q == 0 {
                64
            } else if quant < 2368 {
                84
            } else {
                80
            }
        }
        _ => {
            debug_assert!(
                false,
                "bit_depth should be AOM_BITS_8, AOM_BITS_10 or AOM_BITS_12"
            );
            -1
        }
    }
}

pub(crate) fn av1_build_quantizer(
    bit_depth: i32,
    y_dc_delta_q: i32,
    u_dc_delta_q: i32,
    u_ac_delta_q: i32,
    v_dc_delta_q: i32,
    v_ac_delta_q: i32,
    quants: &mut Quants,
    deq: &mut Dequants,
) {
    for q in 0..QINDEX_RANGE {
        let qu = q as usize;
        let qzbin_factor = get_qzbin_factor(q, bit_depth);
        let qrounding_factor = if q == 0 { 64 } else { 48 };

        for i in 0..2 {
            let qrounding_factor_fp = 64;
            // y quantizer with TX scale
            let mut quant_qtx = if i == 0 {
                av1_dc_quant_qtx(q, y_dc_delta_q, bit_depth)
            } else {
                av1_ac_quant_qtx(q, 0, bit_depth)
            } as i32;
            invert_quant(
                &mut quants.y_quant[qu][i],
                &mut quants.y_quant_shift[qu][i],
                quant_qtx,
            );
            quants.y_quant_fp[qu][i] = ((1 << 16) / quant_qtx) as i16;
            quants.y_round_fp[qu][i] = ((qrounding_factor_fp * quant_qtx) >> 7) as i16;
            quants.y_zbin[qu][i] = round_power_of_two(qzbin_factor * quant_qtx, 7) as i16;
            quants.y_round[qu][i] = ((qrounding_factor * quant_qtx) >> 7) as i16;
            deq.y_dequant_qtx[qu][i] = quant_qtx as i16;

            // u quantizer with TX scale
            quant_qtx = if i == 0 {
                av1_dc_quant_qtx(q, u_dc_delta_q, bit_depth)
            } else {
                av1_ac_quant_qtx(q, u_ac_delta_q, bit_depth)
            } as i32;
            invert_quant(
                &mut quants.u_quant[qu][i],
                &mut quants.u_quant_shift[qu][i],
                quant_qtx,
            );
            quants.u_quant_fp[qu][i] = ((1 << 16) / quant_qtx) as i16;
            quants.u_round_fp[qu][i] = ((qrounding_factor_fp * quant_qtx) >> 7) as i16;
            quants.u_zbin[qu][i] = round_power_of_two(qzbin_factor * quant_qtx, 7) as i16;
            quants.u_round[qu][i] = ((qrounding_factor * quant_qtx) >> 7) as i16;
            deq.u_dequant_qtx[qu][i] = quant_qtx as i16;

            // v quantizer with TX scale
            quant_qtx = if i == 0 {
                av1_dc_quant_qtx(q, v_dc_delta_q, bit_depth)
            } else {
                av1_ac_quant_qtx(q, v_ac_delta_q, bit_depth)
            } as i32;
            invert_quant(
                &mut quants.v_quant[qu][i],
                &mut quants.v_quant_shift[qu][i],
                quant_qtx,
            );
            quants.v_quant_fp[qu][i] = ((1 << 16) / quant_qtx) as i16;
            quants.v_round_fp[qu][i] = ((qrounding_factor_fp * quant_qtx) >> 7) as i16;
            quants.v_zbin[qu][i] = round_power_of_two(qzbin_factor * quant_qtx, 7) as i16;
            quants.v_round[qu][i] = ((qrounding_factor * quant_qtx) >> 7) as i16;
            deq.v_dequant_qtx[qu][i] = quant_qtx as i16;
        }

        for i in 2..8 {
            // 8: SIMD width
            quants.y_quant[qu][i] = quants.y_quant[qu][1];
            quants.y_quant_fp[qu][i] = quants.y_quant_fp[qu][1];
            quants.y_round_fp[qu][i] = quants.y_round_fp[qu][1];
            quants.y_quant_shift[qu][i] = quants.y_quant_shift[qu][1];
            quants.y_zbin[qu][i] = quants.y_zbin[qu][1];
            quants.y_round[qu][i] = quants.y_round[qu][1];
            deq.y_dequant_qtx[qu][i] = deq.y_dequant_qtx[qu][1];

            quants.u_quant[qu][i] = quants.u_quant[qu][1];
            quants.u_quant_fp[qu][i] = quants.u_quant_fp[qu][1];
            quants.u_round_fp[qu][i] = quants.u_round_fp[qu][1];
            quants.u_quant_shift[qu][i] = quants.u_quant_shift[qu][1];
            quants.u_zbin[qu][i] = quants.u_zbin[qu][1];
            quants.u_round[qu][i] = quants.u_round[qu][1];
            deq.u_dequant_qtx[qu][i] = deq.u_dequant_qtx[qu][1];

            quants.v_quant[qu][i] = quants.v_quant[qu][1];
            quants.v_quant_fp[qu][i] = quants.v_quant_fp[qu][1];
            quants.v_round_fp[qu][i] = quants.v_round_fp[qu][1];
            quants.v_quant_shift[qu][i] = quants.v_quant_shift[qu][1];
            quants.v_zbin[qu][i] = quants.v_zbin[qu][1];
            quants.v_round[qu][i] = quants.v_round[qu][1];
            deq.v_dequant_qtx[qu][i] = deq.v_dequant_qtx[qu][1];
        }
    }
}

pub(crate) fn av1_init_quantizer(
    enc_quant_dequant_params: &mut EncQuantDequantParams,
    quant_params: &CommonQuantParams,
    bit_depth: i32,
) {
    let EncQuantDequantParams { quants, dequants } = enc_quant_dequant_params;
    av1_build_quantizer(
        bit_depth,
        quant_params.y_dc_delta_q,
        quant_params.u_dc_delta_q,
        quant_params.u_ac_delta_q,
        quant_params.v_dc_delta_q,
        quant_params.v_ac_delta_q,
        quants,
        dequants,
    );
}

pub(crate) fn av1_set_q_index(
    enc_quant_dequant_params: &EncQuantDequantParams,
    qindex: i32,
    x: &mut Macroblock,
) {
    let quants = &enc_quant_dequant_params.quants;
    let dequants = &enc_quant_dequant_params.dequants;
    let q = qindex as usize;
    x.qindex = qindex;
    x.seg_skip_block = 0; // TODO(angiebird): Find a proper place to init this variable.

    // Y
    x.plane[0].quant_qtx = quants.y_quant[q];
    x.plane[0].quant_fp_qtx = quants.y_quant_fp[q];
    x.plane[0].round_fp_qtx = quants.y_round_fp[q];
    x.plane[0].quant_shift_qtx = quants.y_quant_shift[q];
    x.plane[0].zbin_qtx = quants.y_zbin[q];
    x.plane[0].round_qtx = quants.y_round[q];
    x.plane[0].dequant_qtx = dequants.y_dequant_qtx[q];

    // U
    x.plane[1].quant_qtx = quants.u_quant[q];
    x.plane[1].quant_fp_qtx = quants.u_quant_fp[q];
    x.plane[1].round_fp_qtx = quants.u_round_fp[q];
    x.plane[1].quant_shift_qtx = quants.u_quant_shift[q];
    x.plane[1].zbin_qtx = quants.u_zbin[q];
    x.plane[1].round_qtx = quants.u_round[q];
    x.plane[1].dequant_qtx = dequants.u_dequant_qtx[q];

    // V
    x.plane[2].quant_qtx = quants.v_quant[q];
    x.plane[2].quant_fp_qtx = quants.v_quant_fp[q];
    x.plane[2].round_fp_qtx = quants.v_round_fp[q];
    x.plane[2].quant_shift_qtx = quants.v_quant_shift[q];
    x.plane[2].zbin_qtx = quants.v_zbin[q];
    x.plane[2].round_qtx = quants.v_round[q];
    x.plane[2].dequant_qtx = dequants.v_dequant_qtx[q];
}

/// Translation of `av1_set_qmatrix()`: every level the saver uses is the
/// flat one, so the plane matrices stay NULL (they are not stored).
pub(crate) fn av1_set_qmatrix(quant_params: &CommonQuantParams, segment_id: i32, xd: &MacroBlockD) {
    let use_qmatrix = av1_use_qmatrix(quant_params, xd, segment_id);
    debug_assert!(!use_qmatrix);
}

pub(crate) fn av1_init_plane_quantizers(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    segment_id: i32,
    do_update: bool,
) {
    let cm = &cpi.common;
    let quant_params = &cm.quant_params;
    let gf_group = &cpi.ppi.gf_group;
    let boost_index = 15.min(cpi.ppi.p_rc.gfu_boost / 100);
    let layer_depth = (gf_group.layer_depth[cpi.gf_frame_index as usize] as i32).min(6);
    let frame_type = cm.current_frame.frame_type;
    let qindex_rd;

    let current_qindex = 0.max((QINDEX_RANGE - 1).min(
        if cm.delta_q_info.delta_q_present_flag != 0 {
            quant_params.base_qindex + x.delta_qindex
        } else {
            quant_params.base_qindex
        },
    ));
    let qindex = av1_get_qindex(&cm.seg, segment_id, current_qindex);

    if cpi.oxcf.sb_qp_sweep {
        let current_rd_qindex = 0.max((QINDEX_RANGE - 1).min(
            if cm.delta_q_info.delta_q_present_flag != 0 {
                quant_params.base_qindex + x.rdmult_delta_qindex
            } else {
                quant_params.base_qindex
            },
        ));
        qindex_rd = av1_get_qindex(&cm.seg, segment_id, current_rd_qindex);
    } else {
        qindex_rd = qindex;
    }

    let qindex_rdmult = qindex_rd + quant_params.y_dc_delta_q;
    let rdmult = av1_compute_rd_mult(
        qindex_rdmult,
        cm.seq_params.bit_depth,
        cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize] as i32,
        layer_depth,
        boost_index,
        frame_type,
        cpi.oxcf.q_cfg.use_fixed_qp_offsets,
        super::encoder::is_stat_consumption_stage(cpi),
    );

    let qindex_change = x.qindex != qindex;
    if qindex_change || do_update {
        av1_set_q_index(&cpi.enc_quant_dequant_params, qindex, x);
    }

    if segment_id != x.prev_segment_id || av1_use_qmatrix(quant_params, &x.e_mbd, segment_id) {
        av1_set_qmatrix(quant_params, segment_id, &x.e_mbd);
    }

    x.seg_skip_block = segfeature_active(&cm.seg, segment_id as u8, SEG_LVL_SKIP) as i32;

    av1_set_error_per_bit(&mut x.errorperbit, rdmult);

    x.prev_segment_id = segment_id;
}

pub(crate) fn av1_frame_init_quantizer(cpi: &Av1Comp, x: &mut Macroblock) {
    x.prev_segment_id = -1;
    let segment_id = x.e_mbd.mbmi().segment_id as i32;
    av1_init_plane_quantizers(cpi, x, segment_id, true);
}

pub(crate) fn av1_set_quantizer(
    cm: &mut Av1Common,
    min_qmlevel: i32,
    max_qmlevel: i32,
    q: i32,
    enable_chroma_deltaq: i32,
    enable_hdr_deltaq: i32,
) {
    // quantizer has to be reinitialized with av1_init_quantizer() if any
    // delta_q changes.
    let delta_q_present_flag = cm.delta_q_info.delta_q_present_flag;
    let separate_uv_delta_q = cm.seq_params.separate_uv_delta_q;
    let quant_params = &mut cm.quant_params;
    quant_params.base_qindex = delta_q_present_flag.max(q);
    quant_params.y_dc_delta_q = 0;

    // (chroma delta q and HDR delta q are off for the saver)
    debug_assert!(enable_chroma_deltaq == 0 && enable_hdr_deltaq == 0);
    quant_params.u_dc_delta_q = 0;
    quant_params.u_ac_delta_q = 0;
    quant_params.v_dc_delta_q = 0;
    quant_params.v_ac_delta_q = 0;

    quant_params.qmatrix_level_y =
        aom_get_qmlevel(quant_params.base_qindex, min_qmlevel, max_qmlevel);
    quant_params.qmatrix_level_u = aom_get_qmlevel(
        quant_params.base_qindex + quant_params.u_ac_delta_q,
        min_qmlevel,
        max_qmlevel,
    );

    if separate_uv_delta_q == 0 {
        quant_params.qmatrix_level_v = quant_params.qmatrix_level_u;
    } else {
        quant_params.qmatrix_level_v = aom_get_qmlevel(
            quant_params.base_qindex + quant_params.v_ac_delta_q,
            min_qmlevel,
            max_qmlevel,
        );
    }
}

static QUANTIZER_TO_QINDEX: [i32; 64] = [
    0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60, 64, 68, 72, 76, 80, 84, 88, 92,
    96, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144, 148, 152, 156, 160, 164, 168,
    172, 176, 180, 184, 188, 192, 196, 200, 204, 208, 212, 216, 220, 224, 228, 232, 236, 240, 244,
    249, 255,
];

pub(crate) fn av1_quantizer_to_qindex(quantizer: i32) -> i32 {
    QUANTIZER_TO_QINDEX[quantizer as usize]
}
