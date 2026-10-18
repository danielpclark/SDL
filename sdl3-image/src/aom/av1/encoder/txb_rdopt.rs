// Rust translation of av1/encoder/txb_rdopt.c and
// av1/encoder/txb_rdopt_utils.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2021, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Trellis-like coefficient optimization of a transform block: lowering
//! levels (and the end of block) where that lowers the rate-distortion
//! cost, and the rate of a transform block's coefficients. The quantizer
//! matrices are flat (no `QM_PSNR` distortion). The trellis optimization
//! only runs for the still image saver, whose transforms are all DCTs: only
//! the 2D transform class occurs there.

use super::block::*;
use super::cost::av1_cost_literal;
use super::encoder::Av1Comp;
use super::encodetxb::{
    av1_get_eob_pos_token, av1_get_nz_map_contexts, av1_get_txb_entropy_context,
    av1_txb_init_levels, block_offset, PLANE_RD_MULT,
};
use super::rd::rdcost;
use crate::aom::aom_dsp::aom_dsp_common::{get_msb, TranLowT};
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::entropy::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::idct::av1_get_tx_scale;
use crate::aom::av1::common::scan::get_scan;
use crate::aom::av1::common::txb_common::*;

// (txb_rdopt_utils.h)

static GOLOMB_BITS_COST: [i32; 32] = [
    0,
    512,
    512 * 3,
    512 * 3,
    512 * 5,
    512 * 5,
    512 * 5,
    512 * 5,
    512 * 7,
    512 * 7,
    512 * 7,
    512 * 7,
    512 * 7,
    512 * 7,
    512 * 7,
    512 * 7,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
    512 * 9,
];

static GOLOMB_COST_DIFF: [i32; 32] = [
    0,
    512,
    512 * 2,
    0,
    512 * 2,
    0,
    0,
    0,
    512 * 2,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    512 * 2,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
];

/// `get_dqv()` with the flat matrix.
#[inline]
fn get_dqv(dequant: &[i16], coeff_idx: i32) -> i32 {
    dequant[(coeff_idx != 0) as usize] as i32
}

/// `get_coeff_dist()` with the flat matrix.
#[inline]
fn get_coeff_dist(tcoeff: TranLowT, dqcoeff: TranLowT, shift: i32) -> i64 {
    let diff: i64 = (tcoeff.wrapping_sub(dqcoeff).wrapping_mul(1 << shift)) as i64;
    diff * diff
}

fn get_eob_cost(
    eob: i32,
    txb_eob_costs: &LvMapEobCost,
    txb_costs: &LvMapCoeffCost,
    tx_class: TxClass,
) -> i32 {
    let mut eob_extra = 0;
    let eob_pt = av1_get_eob_pos_token(eob, &mut eob_extra);
    let mut eob_cost;
    let eob_multi_ctx = if tx_class == TX_CLASS_2D { 0 } else { 1 };
    eob_cost = txb_eob_costs.eob_cost[eob_multi_ctx][(eob_pt - 1) as usize];

    if AV1_EOB_OFFSET_BITS[eob_pt as usize] > 0 {
        let eob_ctx = (eob_pt - 3) as usize;
        let eob_shift = AV1_EOB_OFFSET_BITS[eob_pt as usize] as i32 - 1;
        let bit = ((eob_extra & (1 << eob_shift)) != 0) as usize;
        eob_cost += txb_costs.eob_extra_cost[eob_ctx][bit];
        let offset_bits = AV1_EOB_OFFSET_BITS[eob_pt as usize] as i32;
        if offset_bits > 1 {
            eob_cost += av1_cost_literal(offset_bits - 1);
        }
    }
    eob_cost
}

#[inline]
fn get_golomb_cost(abs_qc: i32) -> i32 {
    if abs_qc >= 1 + NUM_BASE_LEVELS + COEFF_BASE_RANGE {
        let r = abs_qc - COEFF_BASE_RANGE - NUM_BASE_LEVELS;
        let length = get_msb(r as u32) + 1;
        return av1_cost_literal(2 * length - 1);
    }
    0
}

#[inline]
fn get_br_cost(level: TranLowT, coeff_lps: &[i32]) -> i32 {
    let base_range = (level - 1 - NUM_BASE_LEVELS).min(COEFF_BASE_RANGE);
    coeff_lps[base_range as usize] + get_golomb_cost(level)
}

#[inline]
fn get_br_cost_with_diff(level: TranLowT, coeff_lps: &[i32], diff: &mut i32) -> i32 {
    let base_range = (level - 1 - NUM_BASE_LEVELS).min(COEFF_BASE_RANGE);
    let mut golomb_bits = 0;
    if level <= COEFF_BASE_RANGE + 1 + NUM_BASE_LEVELS {
        *diff += coeff_lps[(base_range + COEFF_BASE_RANGE + 1) as usize];
    }

    if level >= COEFF_BASE_RANGE + 1 + NUM_BASE_LEVELS {
        let r = level - COEFF_BASE_RANGE - NUM_BASE_LEVELS;
        if r < 32 {
            golomb_bits = GOLOMB_BITS_COST[r as usize];
            *diff += GOLOMB_COST_DIFF[r as usize];
        } else {
            golomb_bits = get_golomb_cost(level);
            *diff += if (r & (r - 1)) == 0 { 1024 } else { 0 };
        }
    }

    coeff_lps[base_range as usize] + golomb_bits
}

#[inline]
fn get_two_coeff_cost_simple(
    ci: i32,
    abs_qc: TranLowT,
    coeff_ctx: i32,
    txb_costs: &LvMapCoeffCost,
    bwl: i32,
    tx_class: TxClass,
    levels: &[u8],
    lv: usize,
    cost_low: &mut i32,
) -> i32 {
    // this simple version assumes the coeff's scan_idx is not DC (scan_idx != 0)
    // and not the last (scan_idx != eob - 1)
    debug_assert!(ci > 0);
    let mut cost = txb_costs.base_cost[coeff_ctx as usize][abs_qc.min(3) as usize];
    let mut diff = 0;
    if abs_qc <= 3 {
        diff = txb_costs.base_cost[coeff_ctx as usize][(abs_qc + 4) as usize];
    }
    if abs_qc != 0 {
        cost += av1_cost_literal(1);
        if abs_qc > NUM_BASE_LEVELS {
            let br_ctx = get_br_ctx(levels, lv, ci, bwl, tx_class);
            let mut brcost_diff = 0;
            cost += get_br_cost_with_diff(
                abs_qc,
                &txb_costs.lps_cost[br_ctx as usize],
                &mut brcost_diff,
            );
            diff += brcost_diff;
        }
    }
    *cost_low = cost - diff;

    cost
}

#[inline]
fn get_coeff_cost_eob(
    ci: i32,
    abs_qc: TranLowT,
    sign: usize,
    coeff_ctx: i32,
    dc_sign_ctx: i32,
    txb_costs: &LvMapCoeffCost,
    bwl: i32,
    tx_class: TxClass,
) -> i32 {
    let mut cost = 0;
    cost += txb_costs.base_eob_cost[coeff_ctx as usize][(abs_qc.min(3) - 1) as usize];
    if abs_qc != 0 {
        if ci == 0 {
            cost += txb_costs.dc_sign_cost[dc_sign_ctx as usize][sign];
        } else {
            cost += av1_cost_literal(1);
        }
        if abs_qc > NUM_BASE_LEVELS {
            let br_ctx = get_br_ctx_eob(ci, bwl, tx_class);
            cost += get_br_cost(abs_qc, &txb_costs.lps_cost[br_ctx as usize]);
        }
    }
    cost
}

#[inline]
fn get_coeff_cost_general(
    is_last: bool,
    ci: i32,
    abs_qc: TranLowT,
    sign: usize,
    coeff_ctx: i32,
    dc_sign_ctx: i32,
    txb_costs: &LvMapCoeffCost,
    bwl: i32,
    tx_class: TxClass,
    levels: &[u8],
    lv: usize,
) -> i32 {
    let mut cost = 0;
    if is_last {
        cost += txb_costs.base_eob_cost[coeff_ctx as usize][(abs_qc.min(3) - 1) as usize];
    } else {
        cost += txb_costs.base_cost[coeff_ctx as usize][abs_qc.min(3) as usize];
    }
    if abs_qc != 0 {
        if ci == 0 {
            cost += txb_costs.dc_sign_cost[dc_sign_ctx as usize][sign];
        } else {
            cost += av1_cost_literal(1);
        }
        if abs_qc > NUM_BASE_LEVELS {
            let br_ctx = if is_last {
                get_br_ctx_eob(ci, bwl, tx_class)
            } else {
                get_br_ctx(levels, lv, ci, bwl, tx_class)
            };
            cost += get_br_cost(abs_qc, &txb_costs.lps_cost[br_ctx as usize]);
        }
    }
    cost
}

#[inline]
fn get_qc_dqc_low(
    abs_qc: TranLowT,
    sign: i32,
    dqv: i32,
    shift: i32,
    qc_low: &mut TranLowT,
    dqc_low: &mut TranLowT,
) {
    let abs_qc_low = abs_qc - 1;
    *qc_low = (-sign ^ abs_qc_low) + sign;
    debug_assert!((if sign != 0 { -abs_qc_low } else { abs_qc_low }) == *qc_low);
    let abs_dqc_low = (abs_qc_low * dqv) >> shift;
    *dqc_low = (-sign ^ abs_dqc_low) + sign;
    debug_assert!((if sign != 0 { -abs_dqc_low } else { abs_dqc_low }) == *dqc_low);
}

// (txb_rdopt.c)

/// The coefficient arrays of the transform block `av1_optimize_txb()`
/// works on.
struct TxbCoeffs<'a> {
    tcoeff: &'a [TranLowT],
    qcoeff: &'a mut [TranLowT],
    dqcoeff: &'a mut [TranLowT],
    levels: &'a mut [u8],
    lv: usize,
}

#[inline]
fn update_coeff_general(
    accu_rate: &mut i32,
    accu_dist: &mut i64,
    si: i32,
    eob: i32,
    tx_size: TxSize,
    tx_class: TxClass,
    bwl: i32,
    height: i32,
    rdmult: i64,
    shift: i32,
    dc_sign_ctx: i32,
    dequant: &[i16],
    scan: &[i16],
    txb_costs: &LvMapCoeffCost,
    c: &mut TxbCoeffs<'_>,
) {
    let dqv = get_dqv(dequant, scan[si as usize] as i32);
    let ci = scan[si as usize] as i32;
    let ciu = ci as usize;
    let qc = c.qcoeff[ciu];
    let is_last = si == eob - 1;
    let coeff_ctx = get_lower_levels_ctx_general(
        is_last, si, bwl, height, c.levels, c.lv, ci, tx_size, tx_class,
    );
    if qc == 0 {
        *accu_rate += txb_costs.base_cost[coeff_ctx as usize][0];
    } else {
        let sign = (qc < 0) as i32;
        let abs_qc = qc.abs();
        let tqc = c.tcoeff[ciu];
        let dqc = c.dqcoeff[ciu];
        let dist = get_coeff_dist(tqc, dqc, shift);
        let dist0 = get_coeff_dist(tqc, 0, shift);
        let rate = get_coeff_cost_general(
            is_last,
            ci,
            abs_qc,
            sign as usize,
            coeff_ctx,
            dc_sign_ctx,
            txb_costs,
            bwl,
            tx_class,
            c.levels,
            c.lv,
        );
        let rd = rdcost(rdmult, rate, dist);

        let mut qc_low: TranLowT = 0;
        let mut dqc_low: TranLowT = 0;
        let abs_qc_low: TranLowT;
        let dist_low: i64;
        let rd_low: i64;
        let rate_low: i32;
        if abs_qc == 1 {
            abs_qc_low = 0;
            qc_low = 0;
            dqc_low = 0;
            dist_low = dist0;
            rate_low = txb_costs.base_cost[coeff_ctx as usize][0];
        } else {
            get_qc_dqc_low(abs_qc, sign, dqv, shift, &mut qc_low, &mut dqc_low);
            abs_qc_low = abs_qc - 1;
            dist_low = get_coeff_dist(tqc, dqc_low, shift);
            rate_low = get_coeff_cost_general(
                is_last,
                ci,
                abs_qc_low,
                sign as usize,
                coeff_ctx,
                dc_sign_ctx,
                txb_costs,
                bwl,
                tx_class,
                c.levels,
                c.lv,
            );
        }

        rd_low = rdcost(rdmult, rate_low, dist_low);
        if rd_low < rd {
            c.qcoeff[ciu] = qc_low;
            c.dqcoeff[ciu] = dqc_low;
            c.levels[c.lv + get_padded_idx(ci, bwl) as usize] =
                abs_qc_low.min(i8::MAX as i32) as u8;
            *accu_rate += rate_low;
            *accu_dist += dist_low - dist0;
        } else {
            *accu_rate += rate;
            *accu_dist += dist - dist0;
        }
    }
}

#[inline]
fn update_coeff_simple(
    accu_rate: &mut i32,
    si: i32,
    eob: i32,
    tx_size: TxSize,
    tx_class: TxClass,
    bwl: i32,
    rdmult: i64,
    shift: i32,
    dequant: &[i16],
    scan: &[i16],
    txb_costs: &LvMapCoeffCost,
    c: &mut TxbCoeffs<'_>,
) {
    let dqv = get_dqv(dequant, scan[si as usize] as i32);
    // this simple version assumes the coeff's scan_idx is not DC (scan_idx != 0)
    // and not the last (scan_idx != eob - 1)
    debug_assert!(si != eob - 1);
    debug_assert!(si > 0);
    let ci = scan[si as usize] as i32;
    let ciu = ci as usize;
    let qc = c.qcoeff[ciu];
    let coeff_ctx = get_lower_levels_ctx(c.levels, c.lv, ci, bwl, tx_size, tx_class);
    if qc == 0 {
        *accu_rate += txb_costs.base_cost[coeff_ctx as usize][0];
    } else {
        let abs_qc = qc.abs();
        let abs_tqc = c.tcoeff[ciu].abs();
        let abs_dqc = c.dqcoeff[ciu].abs();
        let mut rate_low = 0;
        let rate = get_two_coeff_cost_simple(
            ci,
            abs_qc,
            coeff_ctx,
            txb_costs,
            bwl,
            tx_class,
            c.levels,
            c.lv,
            &mut rate_low,
        );
        if abs_dqc < abs_tqc {
            *accu_rate += rate;
            return;
        }

        let dist = get_coeff_dist(abs_tqc, abs_dqc, shift);
        let rd = rdcost(rdmult, rate, dist);

        let abs_qc_low = abs_qc - 1;
        let abs_dqc_low = (abs_qc_low * dqv) >> shift;
        let dist_low = get_coeff_dist(abs_tqc, abs_dqc_low, shift);
        let rd_low = rdcost(rdmult, rate_low, dist_low);

        if rd_low < rd {
            let sign = (qc < 0) as i32;
            c.qcoeff[ciu] = (-sign ^ abs_qc_low) + sign;
            c.dqcoeff[ciu] = (-sign ^ abs_dqc_low) + sign;
            c.levels[c.lv + get_padded_idx(ci, bwl) as usize] =
                abs_qc_low.min(i8::MAX as i32) as u8;
            *accu_rate += rate_low;
        } else {
            *accu_rate += rate;
        }
    }
}

#[inline]
fn update_coeff_eob(
    accu_rate: &mut i32,
    accu_dist: &mut i64,
    eob: &mut i32,
    nz_num: &mut i32,
    nz_ci: &mut [i32; 3],
    si: i32,
    tx_size: TxSize,
    tx_class: TxClass,
    bwl: i32,
    height: i32,
    dc_sign_ctx: i32,
    rdmult: i64,
    shift: i32,
    dequant: &[i16],
    scan: &[i16],
    txb_eob_costs: &LvMapEobCost,
    txb_costs: &LvMapCoeffCost,
    c: &mut TxbCoeffs<'_>,
    sharpness: i32,
) {
    let dqv = get_dqv(dequant, scan[si as usize] as i32);
    debug_assert!(si != *eob - 1);
    let ci = scan[si as usize] as i32;
    let ciu = ci as usize;
    let qc = c.qcoeff[ciu];
    let coeff_ctx = get_lower_levels_ctx(c.levels, c.lv, ci, bwl, tx_size, tx_class);
    if qc == 0 {
        *accu_rate += txb_costs.base_cost[coeff_ctx as usize][0];
    } else {
        let mut lower_level = false;
        let abs_qc = qc.abs();
        let tqc = c.tcoeff[ciu];
        let dqc = c.dqcoeff[ciu];
        let sign = (qc < 0) as i32;
        let dist0 = get_coeff_dist(tqc, 0, shift);
        let mut dist = get_coeff_dist(tqc, dqc, shift) - dist0;
        let mut rate = get_coeff_cost_general(
            false,
            ci,
            abs_qc,
            sign as usize,
            coeff_ctx,
            dc_sign_ctx,
            txb_costs,
            bwl,
            tx_class,
            c.levels,
            c.lv,
        );
        let mut rd = rdcost(rdmult, *accu_rate + rate, *accu_dist + dist);

        let mut qc_low: TranLowT = 0;
        let mut dqc_low: TranLowT = 0;
        let abs_qc_low: TranLowT;
        let dist_low: i64;
        let rd_low: i64;
        let rate_low: i32;

        if abs_qc == 1 {
            abs_qc_low = 0;
            dqc_low = 0;
            qc_low = 0;
            dist_low = 0;
            rate_low = txb_costs.base_cost[coeff_ctx as usize][0];
            rd_low = rdcost(rdmult, *accu_rate + rate_low, *accu_dist);
        } else {
            get_qc_dqc_low(abs_qc, sign, dqv, shift, &mut qc_low, &mut dqc_low);
            abs_qc_low = abs_qc - 1;
            dist_low = get_coeff_dist(tqc, dqc_low, shift) - dist0;
            rate_low = get_coeff_cost_general(
                false,
                ci,
                abs_qc_low,
                sign as usize,
                coeff_ctx,
                dc_sign_ctx,
                txb_costs,
                bwl,
                tx_class,
                c.levels,
                c.lv,
            );
            rd_low = rdcost(rdmult, *accu_rate + rate_low, *accu_dist + dist_low);
        }

        let mut lower_level_new_eob = false;
        let new_eob = si + 1;
        let coeff_ctx_new_eob = get_lower_levels_ctx_eob(bwl, height, si);
        let new_eob_cost = get_eob_cost(new_eob, txb_eob_costs, txb_costs, tx_class);
        let mut rate_coeff_eob = new_eob_cost
            + get_coeff_cost_eob(
                ci,
                abs_qc,
                sign as usize,
                coeff_ctx_new_eob,
                dc_sign_ctx,
                txb_costs,
                bwl,
                tx_class,
            );
        let mut dist_new_eob = dist;
        let mut rd_new_eob = rdcost(rdmult, rate_coeff_eob, dist_new_eob);

        if abs_qc_low > 0 {
            let rate_coeff_eob_low = new_eob_cost
                + get_coeff_cost_eob(
                    ci,
                    abs_qc_low,
                    sign as usize,
                    coeff_ctx_new_eob,
                    dc_sign_ctx,
                    txb_costs,
                    bwl,
                    tx_class,
                );
            let dist_new_eob_low = dist_low;
            let rd_new_eob_low = rdcost(rdmult, rate_coeff_eob_low, dist_new_eob_low);
            if rd_new_eob_low < rd_new_eob {
                lower_level_new_eob = true;
                rd_new_eob = rd_new_eob_low;
                rate_coeff_eob = rate_coeff_eob_low;
                dist_new_eob = dist_new_eob_low;
            }
        }

        if sharpness == 0 || abs_qc > 1 {
            if rd_low < rd {
                lower_level = true;
                rd = rd_low;
                rate = rate_low;
                dist = dist_low;
            }
        }

        if sharpness == 0 && rd_new_eob < rd {
            for ni in 0..*nz_num as usize {
                let last_ci = nz_ci[ni];
                c.levels[c.lv + get_padded_idx(last_ci, bwl) as usize] = 0;
                c.qcoeff[last_ci as usize] = 0;
                c.dqcoeff[last_ci as usize] = 0;
            }
            *eob = new_eob;
            *nz_num = 0;
            *accu_rate = rate_coeff_eob;
            *accu_dist = dist_new_eob;
            lower_level = lower_level_new_eob;
        } else {
            *accu_rate += rate;
            *accu_dist += dist;
        }

        if lower_level {
            c.qcoeff[ciu] = qc_low;
            c.dqcoeff[ciu] = dqc_low;
            c.levels[c.lv + get_padded_idx(ci, bwl) as usize] =
                abs_qc_low.min(i8::MAX as i32) as u8;
        }
        if c.qcoeff[ciu] != 0 {
            nz_ci[*nz_num as usize] = ci;
            *nz_num += 1;
        }
    }
}

#[inline]
fn update_skip(
    accu_rate: &mut i32,
    accu_dist: i64,
    eob: &mut i32,
    nz_num: i32,
    nz_ci: &[i32; 3],
    rdmult: i64,
    skip_cost: i32,
    non_skip_cost: i32,
    qcoeff: &mut [TranLowT],
    dqcoeff: &mut [TranLowT],
) {
    let rd = rdcost(rdmult, *accu_rate + non_skip_cost, accu_dist);
    let rd_new_eob = rdcost(rdmult, skip_cost, 0);
    if rd_new_eob < rd {
        for i in 0..nz_num as usize {
            let ci = nz_ci[i] as usize;
            qcoeff[ci] = 0;
            dqcoeff[ci] = 0;
            // no need to set up levels because this is the last step
            // levels[get_padded_idx(ci, bwl)] = 0;
        }
        *accu_rate = 0;
        *eob = 0;
    }
}

// TODO(angiebird): use this function whenever it's possible
fn get_tx_type_cost(
    mode_costs: &ModeCosts,
    xd: &MacroBlockD,
    plane: usize,
    tx_size: TxSize,
    tx_type: TxType,
    reduced_tx_set_used: i32,
) -> i32 {
    if plane > 0 {
        return 0;
    }

    let square_tx_size = TXSIZE_SQR_MAP[tx_size as usize];

    let mbmi = xd.mbmi();
    let is_inter = is_inter_block(mbmi) as i32;
    if get_ext_tx_types(tx_size, is_inter, reduced_tx_set_used) > 1
        && xd.lossless[mbmi.segment_id as usize] == 0
    {
        let ext_tx_set = get_ext_tx_set(tx_size, is_inter, reduced_tx_set_used);
        if is_inter != 0 {
            if ext_tx_set > 0 {
                return mode_costs.inter_tx_type_costs[ext_tx_set as usize]
                    [square_tx_size as usize][tx_type as usize];
            }
        } else if ext_tx_set > 0 {
            let intra_dir = if mbmi.filter_intra_mode_info.use_filter_intra != 0 {
                FIMODE_TO_INTRADIR[mbmi.filter_intra_mode_info.filter_intra_mode as usize]
            } else {
                mbmi.mode
            };
            return mode_costs.intra_tx_type_costs[ext_tx_set as usize][square_tx_size as usize]
                [intra_dir as usize][tx_type as usize];
        }
    }
    0
}

pub(crate) fn av1_optimize_txb(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    tx_type: TxType,
    txb_ctx: &TxbCtx,
    rate_cost: &mut i32,
    sharpness: i32,
) -> i32 {
    let Macroblock {
        plane: planes,
        e_mbd: xd,
        coeff_costs,
        mode_costs,
        rdmult: x_rdmult,
        ..
    } = x;
    let p = &mut planes[plane];
    let scan_order = get_scan(tx_size, tx_type);
    let scan = scan_order.scan;
    let shift = av1_get_tx_scale(tx_size);
    let mut eob = p.eobs[block as usize] as i32;
    let dequant = p.dequant_qtx;
    // (flat quantizer matrices, no QM_PSNR distortion metric)
    let block_offset = (block << 4) as usize;
    let coeff_costs: &CoeffCosts = coeff_costs;

    // This function is not called if eob = 0.
    debug_assert!(eob > 0);

    let cm = &cpi.common;
    let plane_type = get_plane_type(plane);
    let txs_ctx = get_txsize_entropy_ctx(tx_size);
    let tx_class = TX_TYPE_TO_CLASS[tx_type as usize];
    let mbmi = xd.mbmi();
    let bwl = get_txb_bwl(tx_size);
    let width = get_txb_wide(tx_size);
    let height = get_txb_high(tx_size);
    debug_assert!(width == (1 << bwl));
    let is_inter = is_inter_block(mbmi) as usize;
    let txb_costs = &coeff_costs.coeff_costs[txs_ctx as usize][plane_type as usize];
    let eob_multi_size = TXSIZE_LOG2_MINUS4[tx_size as usize] as usize;
    let txb_eob_costs = &coeff_costs.eob_costs[eob_multi_size][plane_type as usize];

    let rshift = 2;

    let rdmult: i64 = ((*x_rdmult as i64
        * ((PLANE_RD_MULT[is_inter][plane_type as usize] << (2 * (xd.bd - 8))) as i64))
        + 2)
        >> rshift;

    let mut levels_buf = [0u8; TX_PAD_2D as usize];
    let lv = set_levels(width);

    let (tcoeff_all, qcoeff_all, dqcoeff_all) = (&p.coeff, &mut p.qcoeff, &mut p.dqcoeff);
    let tcoeff = &tcoeff_all[block_offset..];
    let qcoeff = &mut qcoeff_all[block_offset..];
    let dqcoeff = &mut dqcoeff_all[block_offset..];

    if eob > 1 {
        av1_txb_init_levels(qcoeff, width, height, &mut levels_buf, lv);
    }

    // TODO(angirbird): check iqmatrix

    let non_skip_cost = txb_costs.txb_skip_cost[txb_ctx.txb_skip_ctx as usize][0];
    let skip_cost = txb_costs.txb_skip_cost[txb_ctx.txb_skip_ctx as usize][1];
    let eob_cost = get_eob_cost(eob, txb_eob_costs, txb_costs, tx_class);
    let mut accu_rate = eob_cost;
    let mut accu_dist: i64 = 0;
    let mut si = eob - 1;
    let ci = scan[si as usize] as i32;
    let qc = qcoeff[ci as usize];
    let abs_qc = qc.abs();
    let sign = (qc < 0) as usize;
    let max_nz_num = 2;
    let mut nz_num = 1;
    let mut nz_ci = [ci, 0, 0];
    let mut c = TxbCoeffs {
        tcoeff,
        qcoeff,
        dqcoeff,
        levels: &mut levels_buf,
        lv,
    };
    if abs_qc >= 2 {
        update_coeff_general(
            &mut accu_rate,
            &mut accu_dist,
            si,
            eob,
            tx_size,
            tx_class,
            bwl,
            height,
            rdmult,
            shift,
            txb_ctx.dc_sign_ctx,
            &dequant,
            scan,
            txb_costs,
            &mut c,
        );
        si -= 1;
    } else {
        debug_assert!(abs_qc == 1);
        let coeff_ctx = get_lower_levels_ctx_eob(bwl, height, si);
        accu_rate += get_coeff_cost_eob(
            ci,
            abs_qc,
            sign,
            coeff_ctx,
            txb_ctx.dc_sign_ctx,
            txb_costs,
            bwl,
            tx_class,
        );
        let tqc = c.tcoeff[ci as usize];
        let dqc = c.dqcoeff[ci as usize];
        let dist = get_coeff_dist(tqc, dqc, shift);
        let dist0 = get_coeff_dist(tqc, 0, shift);
        accu_dist += dist - dist0;
        si -= 1;
    }

    // (UPDATE_COEFF_EOB_CASE: the transform class is always 2D)
    debug_assert!(tx_class == TX_CLASS_2D);
    while si >= 0 && nz_num <= max_nz_num {
        update_coeff_eob(
            &mut accu_rate,
            &mut accu_dist,
            &mut eob,
            &mut nz_num,
            &mut nz_ci,
            si,
            tx_size,
            TX_CLASS_2D,
            bwl,
            height,
            txb_ctx.dc_sign_ctx,
            rdmult,
            shift,
            &dequant,
            scan,
            txb_eob_costs,
            txb_costs,
            &mut c,
            sharpness,
        );
        si -= 1;
    }
    if si == -1 && nz_num <= max_nz_num && sharpness == 0 {
        update_skip(
            &mut accu_rate,
            accu_dist,
            &mut eob,
            nz_num,
            &nz_ci,
            rdmult,
            skip_cost,
            non_skip_cost,
            c.qcoeff,
            c.dqcoeff,
        );
    }

    // (UPDATE_COEFF_SIMPLE_CASE: the transform class is always 2D)
    while si >= 1 {
        update_coeff_simple(
            &mut accu_rate,
            si,
            eob,
            tx_size,
            TX_CLASS_2D,
            bwl,
            rdmult,
            shift,
            &dequant,
            scan,
            txb_costs,
            &mut c,
        );
        si -= 1;
    }
    if si == 0 {
        // no need to update accu_dist because it's not used after this point
        let mut dummy_dist: i64 = 0;
        update_coeff_general(
            &mut accu_rate,
            &mut dummy_dist,
            si,
            eob,
            tx_size,
            tx_class,
            bwl,
            height,
            rdmult,
            shift,
            txb_ctx.dc_sign_ctx,
            &dequant,
            scan,
            txb_costs,
            &mut c,
        );
    }

    let tx_type_cost = get_tx_type_cost(
        mode_costs,
        xd,
        plane,
        tx_size,
        tx_type,
        cm.features.reduced_tx_set_used as i32,
    );
    if eob == 0 {
        accu_rate += skip_cost;
    } else {
        accu_rate += non_skip_cost + tx_type_cost;
    }

    p.eobs[block as usize] = eob as u16;
    p.txb_entropy_ctx[block as usize] = av1_get_txb_entropy_context(
        &p.qcoeff[block_offset..],
        &scan_order,
        p.eobs[block as usize] as i32,
    );

    *rate_cost = accu_rate;
    eob
}

/// Translation of `av1_cost_skip_txb()` (encodetxb.h).
#[inline]
pub(crate) fn av1_cost_skip_txb(
    coeff_costs: &CoeffCosts,
    txb_ctx: &TxbCtx,
    plane: usize,
    tx_size: TxSize,
) -> i32 {
    let txs_ctx = get_txsize_entropy_ctx(tx_size);
    let plane_type = get_plane_type(plane);
    let coeff_costs_ = &coeff_costs.coeff_costs[txs_ctx as usize][plane_type as usize];
    coeff_costs_.txb_skip_cost[txb_ctx.txb_skip_ctx as usize][1]
}

#[inline]
fn aomsign(x: i32) -> i32 {
    x >> 31
}

/// Translation of `warehouse_efficients_txb()`.
fn warehouse_efficients_txb(
    x: &Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    txb_ctx: &TxbCtx,
    eob: i32,
    plane_type: i32,
    coeff_costs: &LvMapCoeffCost,
    tx_type: TxType,
    tx_class: TxClass,
    reduced_tx_set_used: i32,
) -> i32 {
    let p = &x.plane[plane];
    let xd = &x.e_mbd;
    let qcoeff = &p.qcoeff[block_offset(block)..];
    let txb_skip_ctx = txb_ctx.txb_skip_ctx as usize;
    let bwl = get_txb_bwl(tx_size);
    let width = get_txb_wide(tx_size);
    let height = get_txb_high(tx_size);
    let scan_order = get_scan(tx_size, tx_type);
    let scan = scan_order.scan;
    let mut levels_buf = [0u8; TX_PAD_2D as usize];
    let levels = set_levels(width);
    let mut coeff_contexts = [0i8; MAX_TX_SQUARE as usize];
    let eob_multi_size = TXSIZE_LOG2_MINUS4[tx_size as usize] as usize;
    let eob_costs = &x.coeff_costs.eob_costs[eob_multi_size][plane_type as usize];
    let mut cost = coeff_costs.txb_skip_cost[txb_skip_ctx][0];

    av1_txb_init_levels(qcoeff, width, height, &mut levels_buf, levels);

    cost += get_tx_type_cost(
        &x.mode_costs,
        xd,
        plane,
        tx_size,
        tx_type,
        reduced_tx_set_used,
    );

    cost += get_eob_cost(eob, eob_costs, coeff_costs, tx_class);

    av1_get_nz_map_contexts(
        &levels_buf,
        levels,
        scan,
        eob as u16,
        tx_size,
        tx_class,
        &mut coeff_contexts,
    );

    let lps_cost = &coeff_costs.lps_cost;
    let mut c = eob - 1;
    {
        let pos = scan[c as usize] as usize;
        let v = qcoeff[pos];
        let sign = aomsign(v);
        let level = (v ^ sign) - sign;
        let coeff_ctx = coeff_contexts[pos] as usize;
        cost += coeff_costs.base_eob_cost[coeff_ctx][(level.min(3) - 1) as usize];

        if v != 0 {
            // sign bit cost
            if level > NUM_BASE_LEVELS {
                let ctx = get_br_ctx_eob(pos as i32, bwl, tx_class) as usize;
                cost += get_br_cost(level, &lps_cost[ctx]);
            }
            if c != 0 {
                cost += av1_cost_literal(1);
            } else {
                let sign01 = ((sign ^ sign) - sign) as usize;
                let dc_sign_ctx = txb_ctx.dc_sign_ctx as usize;
                cost += coeff_costs.dc_sign_cost[dc_sign_ctx][sign01];
                return cost;
            }
        }
    }
    let base_cost = &coeff_costs.base_cost;
    c = eob - 2;
    while c >= 1 {
        let pos = scan[c as usize] as usize;
        let coeff_ctx = coeff_contexts[pos] as usize;
        let v = qcoeff[pos];
        let level = v.abs();
        cost += base_cost[coeff_ctx][level.min(3) as usize];
        if v != 0 {
            // sign bit cost
            cost += av1_cost_literal(1);
            if level > NUM_BASE_LEVELS {
                let ctx = get_br_ctx(&levels_buf, levels, pos as i32, bwl, tx_class) as usize;
                cost += get_br_cost(level, &lps_cost[ctx]);
            }
        }
        c -= 1;
    }
    // c == 0 after previous loop
    {
        let pos = scan[0] as usize;
        let v = qcoeff[pos];
        let coeff_ctx = coeff_contexts[pos] as usize;
        let sign = aomsign(v);
        let level = (v ^ sign) - sign;
        cost += base_cost[coeff_ctx][level.min(3) as usize];

        if v != 0 {
            // sign bit cost
            let sign01 = ((sign ^ sign) - sign) as usize;
            let dc_sign_ctx = txb_ctx.dc_sign_ctx as usize;
            cost += coeff_costs.dc_sign_cost[dc_sign_ctx][sign01];
            if level > NUM_BASE_LEVELS {
                let ctx = get_br_ctx(&levels_buf, levels, pos as i32, bwl, tx_class) as usize;
                cost += get_br_cost(level, &lps_cost[ctx]);
            }
        }
    }
    cost
}

/// Translation of `av1_cost_coeffs_txb()`.
pub(crate) fn av1_cost_coeffs_txb(
    x: &Macroblock,
    plane: usize,
    block: i32,
    tx_size: TxSize,
    tx_type: TxType,
    txb_ctx: &TxbCtx,
    reduced_tx_set_used: i32,
) -> i32 {
    let p = &x.plane[plane];
    let eob = p.eobs[block as usize] as i32;
    let txs_ctx = get_txsize_entropy_ctx(tx_size) as usize;
    let plane_type = get_plane_type(plane);
    let coeff_costs = &x.coeff_costs.coeff_costs[txs_ctx][plane_type as usize];
    if eob == 0 {
        return coeff_costs.txb_skip_cost[txb_ctx.txb_skip_ctx as usize][1];
    }

    let tx_class = TX_TYPE_TO_CLASS[tx_type as usize];

    warehouse_efficients_txb(
        x,
        plane,
        block,
        tx_size,
        txb_ctx,
        eob,
        plane_type,
        coeff_costs,
        tx_type,
        tx_class,
        reduced_tx_set_used,
    )
}
