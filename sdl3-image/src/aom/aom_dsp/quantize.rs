// Rust translation of aom_dsp/quantize.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The high bit depth "B" quantizer, without quantization matrices (the
//! encoder doesn't enable them).

use super::aom_dsp_common::{round_power_of_two, QmValT, TranLowT, AOM_QM_BITS};

#[inline]
fn aomsign(x: i32) -> i32 {
    if x < 0 {
        -1
    } else {
        0
    }
}

/// Translation of `aom_highbd_quantize_b_helper_c()`.
pub(crate) fn aom_highbd_quantize_b_helper(
    coeff_ptr: &[TranLowT],
    n_coeffs: isize,
    zbin_ptr: &[i16],
    round_ptr: &[i16],
    quant_ptr: &[i16],
    quant_shift_ptr: &[i16],
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    dequant_ptr: &[i16],
    eob_ptr: &mut u16,
    scan: &[i16],
    _iscan: &[i16],
    qm_ptr: Option<&[QmValT]>,
    iqm_ptr: Option<&[QmValT]>,
    log_scale: i32,
) {
    let mut eob: i32 = -1;
    let zbins = [
        round_power_of_two(zbin_ptr[0] as i32, log_scale),
        round_power_of_two(zbin_ptr[1] as i32, log_scale),
    ];
    let nzbins = [zbins[0] * -1, zbins[1] * -1];
    let mut dequant: i32;
    let mut idx_arr = [0i32; 4096];
    let mut idx: usize = 0;
    let n = n_coeffs as usize;

    qcoeff_ptr[..n].fill(0);
    dqcoeff_ptr[..n].fill(0);

    // Pre-scan pass
    for i in 0..n {
        let rc = scan[i] as usize;
        let wt: QmValT = match qm_ptr {
            Some(qm) => qm[rc],
            None => 1 << AOM_QM_BITS,
        };
        let coeff = coeff_ptr[rc] * wt as i32;

        // If the coefficient is out of the base ZBIN range, keep it for
        // quantization.
        if coeff >= (zbins[(rc != 0) as usize] * (1 << AOM_QM_BITS))
            || coeff <= (nzbins[(rc != 0) as usize] * (1 << AOM_QM_BITS))
        {
            idx_arr[idx] = i as i32;
            idx += 1;
        }
    }

    // Quantization pass: only process the coefficients selected in
    // pre-scan pass. Note: idx can be zero.
    for i in 0..idx {
        let rc = scan[idx_arr[i] as usize] as usize;
        let coeff = coeff_ptr[rc];
        let coeff_sign = aomsign(coeff);
        let wt: QmValT = match qm_ptr {
            Some(qm) => qm[rc],
            None => 1 << AOM_QM_BITS,
        };
        let iwt: QmValT = match iqm_ptr {
            Some(iqm) => iqm[rc],
            None => 1 << AOM_QM_BITS,
        };
        let abs_coeff = (coeff ^ coeff_sign) - coeff_sign;
        let tmp1: i64 = abs_coeff as i64
            + round_power_of_two(round_ptr[(rc != 0) as usize] as i32, log_scale) as i64;
        let tmpw: i64 = tmp1 * wt as i64;
        let tmp2: i64 = ((tmpw * quant_ptr[(rc != 0) as usize] as i64) >> 16) + tmpw;
        let abs_qcoeff = ((tmp2 * quant_shift_ptr[(rc != 0) as usize] as i64)
            >> (16 - log_scale + AOM_QM_BITS)) as i32;
        qcoeff_ptr[rc] = (abs_qcoeff ^ coeff_sign) - coeff_sign;
        dequant = (dequant_ptr[(rc != 0) as usize] as i32 * iwt as i32 + (1 << (AOM_QM_BITS - 1)))
            >> AOM_QM_BITS;
        let abs_dqcoeff: TranLowT = (abs_qcoeff * dequant) >> log_scale;
        dqcoeff_ptr[rc] = (abs_dqcoeff ^ coeff_sign) - coeff_sign;
        if abs_qcoeff != 0 {
            eob = idx_arr[i];
        }
    }
    *eob_ptr = (eob + 1) as u16;
}

/// Translation of `aom_highbd_quantize_b_c()`.
pub(crate) fn aom_highbd_quantize_b(
    coeff_ptr: &[TranLowT],
    n_coeffs: isize,
    zbin_ptr: &[i16],
    round_ptr: &[i16],
    quant_ptr: &[i16],
    quant_shift_ptr: &[i16],
    qcoeff_ptr: &mut [TranLowT],
    dqcoeff_ptr: &mut [TranLowT],
    dequant_ptr: &[i16],
    eob_ptr: &mut u16,
    scan: &[i16],
    iscan: &[i16],
) {
    aom_highbd_quantize_b_helper(
        coeff_ptr,
        n_coeffs,
        zbin_ptr,
        round_ptr,
        quant_ptr,
        quant_shift_ptr,
        qcoeff_ptr,
        dqcoeff_ptr,
        dequant_ptr,
        eob_ptr,
        scan,
        iscan,
        None,
        None,
        0,
    );
}
