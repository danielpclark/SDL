// Rust translation of the parts of av1/encoder/encodemv.c and
// av1/encoder/encodemv.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches: motion vector coding and costs.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion vector coding, statistics and cost tables.

use super::block::{Macroblock, MbModeInfoExt, MvCosts};
use super::cost::av1_cost_tokens_from_cdf;
use crate::aom::aom_dsp::aom_dsp_common::get_msb;
use crate::aom::aom_dsp::bitwriter::{aom_write_symbol, AomWriter};
use crate::aom::aom_dsp::prob::update_cdf;
use crate::aom::av1::common::blockd::{CandidateMv, MacroBlockD};
use crate::aom::av1::common::entropymv::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mv::{IntMv, Mv};
use crate::aom::av1::common::mvref_common::{av1_ref_frame_type, lower_mv_precision};

/// Translation of `av1_get_mv_joint()`.
#[inline]
pub(crate) fn av1_get_mv_joint(mv: &Mv) -> MvJointType {
    // row:  Z  col:  Z  | MV_JOINT_ZERO   (0)
    // row:  Z  col: NZ  | MV_JOINT_HNZVZ  (1)
    // row: NZ  col:  Z  | MV_JOINT_HZVNZ  (2)
    // row: NZ  col: NZ  | MV_JOINT_HNZVNZ (3)
    (mv.col != 0) as i32 | (((mv.row != 0) as i32) << 1)
}

/// Translation of `av1_mv_class_base()`.
#[inline]
fn av1_mv_class_base(c: i32) -> i32 {
    if c != 0 {
        CLASS0_SIZE << (c + 2)
    } else {
        0
    }
}

// If n != 0, returns the floor of log base 2 of n. If n == 0, returns 0.
/// Translation of `av1_log_in_base_2()`.
#[inline]
fn av1_log_in_base_2(n: u32) -> u8 {
    // get_msb() is only valid when n != 0.
    if n == 0 {
        0
    } else {
        get_msb(n) as u8
    }
}

/// Translation of `av1_get_mv_class()`.
#[inline]
pub(crate) fn av1_get_mv_class(z: i32, offset: &mut i32) -> i32 {
    debug_assert!(z >= 0);
    let c = av1_log_in_base_2((z >> 3) as u32) as i32;
    debug_assert!(c <= MV_CLASS_10);
    *offset = z - av1_mv_class_base(c);
    c
}

/// The difference of two motion vectors (`{ mv->row - ref->row,
/// mv->col - ref->col }`, stored as `int16_t`).
#[inline]
fn mv_diff(mv: &Mv, ref_: &Mv) -> Mv {
    Mv {
        row: (mv.row as i32 - ref_.row as i32) as i16,
        col: (mv.col as i32 - ref_.col as i32) as i16,
    }
}

/// Translation of `update_mv_component_stats()`.
fn update_mv_component_stats(comp: i32, mvcomp: &mut NmvComponent, precision: i32) {
    debug_assert!(comp != 0);
    let mut offset = 0;
    let sign = (comp < 0) as i32;
    let mag = if sign != 0 { -comp } else { comp };
    let mv_class = av1_get_mv_class(mag - 1, &mut offset);
    let d = offset >> 3; // int mv data
    let fr = (offset >> 1) & 3; // fractional mv data
    let hp = offset & 1; // high precision mv data

    // Sign
    update_cdf(&mut mvcomp.sign_cdf, sign as i8, 2);

    // Class
    update_cdf(&mut mvcomp.classes_cdf, mv_class as i8, MV_CLASSES);

    // Integer bits
    if mv_class == MV_CLASS_0 {
        update_cdf(&mut mvcomp.class0_cdf, d as i8, CLASS0_SIZE);
    } else {
        let n = mv_class + CLASS0_BITS - 1; // number of bits
        for i in 0..n {
            update_cdf(&mut mvcomp.bits_cdf[i as usize], ((d >> i) & 1) as i8, 2);
        }
    }
    // Fractional bits
    if precision > MV_SUBPEL_NONE {
        let fp_cdf = if mv_class == MV_CLASS_0 {
            &mut mvcomp.class0_fp_cdf[d as usize]
        } else {
            &mut mvcomp.fp_cdf
        };
        update_cdf(fp_cdf, fr as i8, MV_FP_SIZE);
    }

    // High precision bit
    if precision > MV_SUBPEL_LOW_PRECISION {
        let hp_cdf = if mv_class == MV_CLASS_0 {
            &mut mvcomp.class0_hp_cdf
        } else {
            &mut mvcomp.hp_cdf
        };
        update_cdf(hp_cdf, hp as i8, 2);
    }
}

/// Translation of `av1_update_mv_stats()`.
pub(crate) fn av1_update_mv_stats(mv: &Mv, ref_: &Mv, mvctx: &mut NmvContext, precision: i32) {
    let diff = mv_diff(mv, ref_);
    let j = av1_get_mv_joint(&diff);

    update_cdf(&mut mvctx.joints_cdf, j as i8, MV_JOINTS);

    if mv_joint_vertical(j) {
        update_mv_component_stats(diff.row as i32, &mut mvctx.comps[0], precision);
    }

    if mv_joint_horizontal(j) {
        update_mv_component_stats(diff.col as i32, &mut mvctx.comps[1], precision);
    }
}

/// Translation of `encode_mv_component()`.
fn encode_mv_component(w: &mut AomWriter, comp: i32, mvcomp: &mut NmvComponent, precision: i32) {
    debug_assert!(comp != 0);
    let mut offset = 0;
    let sign = (comp < 0) as i32;
    let mag = if sign != 0 { -comp } else { comp };
    let mv_class = av1_get_mv_class(mag - 1, &mut offset);
    let d = offset >> 3; // int mv data
    let fr = (offset >> 1) & 3; // fractional mv data
    let hp = offset & 1; // high precision mv data

    // Sign
    aom_write_symbol(w, sign, &mut mvcomp.sign_cdf, 2);

    // Class
    aom_write_symbol(w, mv_class, &mut mvcomp.classes_cdf, MV_CLASSES);

    // Integer bits
    if mv_class == MV_CLASS_0 {
        aom_write_symbol(w, d, &mut mvcomp.class0_cdf, CLASS0_SIZE);
    } else {
        let n = mv_class + CLASS0_BITS - 1; // number of bits
        for i in 0..n {
            aom_write_symbol(w, (d >> i) & 1, &mut mvcomp.bits_cdf[i as usize], 2);
        }
    }
    // Fractional bits
    if precision > MV_SUBPEL_NONE {
        let cdf = if mv_class == MV_CLASS_0 {
            &mut mvcomp.class0_fp_cdf[d as usize]
        } else {
            &mut mvcomp.fp_cdf
        };
        aom_write_symbol(w, fr, cdf, MV_FP_SIZE);
    }

    // High precision bit
    if precision > MV_SUBPEL_LOW_PRECISION {
        let cdf = if mv_class == MV_CLASS_0 {
            &mut mvcomp.class0_hp_cdf
        } else {
            &mut mvcomp.hp_cdf
        };
        aom_write_symbol(w, hp, cdf, 2);
    }
}

/* TODO(siekyleb@amazon.com): This function writes MV_VALS ints or 128 KiB. This
 *   is more than most L1D caches and is a significant chunk of L2. Write
 *   SIMD that uses streaming writes to avoid loading all of that into L1, or
 *   just don't update the larger component costs every time this called
 *   (or both).
 */
/// Translation of `av1_build_nmv_component_cost_table()`: `mvcost` is the
/// whole allocation, the C pointer is its middle (`MV_MAX`).
pub(crate) fn av1_build_nmv_component_cost_table(
    mvcost_alloc: &mut [i32],
    mvcomp: &NmvComponent,
    precision: i32,
) {
    let mut sign_cost = [0i32; 2];
    let mut class_cost = [0i32; MV_CLASSES as usize];
    let mut class0_cost = [0i32; CLASS0_SIZE as usize];
    let mut bits_cost = [[0i32; 2]; MV_OFFSET_BITS as usize];
    let mut class0_fp_cost = [[0i32; MV_FP_SIZE as usize]; CLASS0_SIZE as usize];
    let mut fp_cost = [0i32; MV_FP_SIZE as usize];
    let mut class0_hp_cost = [0i32; 2];
    let mut hp_cost = [0i32; 2];

    av1_cost_tokens_from_cdf(&mut sign_cost, &mvcomp.sign_cdf, None);
    av1_cost_tokens_from_cdf(&mut class_cost, &mvcomp.classes_cdf, None);
    av1_cost_tokens_from_cdf(&mut class0_cost, &mvcomp.class0_cdf, None);
    for i in 0..MV_OFFSET_BITS as usize {
        av1_cost_tokens_from_cdf(&mut bits_cost[i], &mvcomp.bits_cdf[i], None);
    }

    if precision > MV_SUBPEL_NONE {
        for i in 0..CLASS0_SIZE as usize {
            av1_cost_tokens_from_cdf(&mut class0_fp_cost[i], &mvcomp.class0_fp_cdf[i], None);
        }
        av1_cost_tokens_from_cdf(&mut fp_cost, &mvcomp.fp_cdf, None);
    }

    if precision > MV_SUBPEL_LOW_PRECISION {
        av1_cost_tokens_from_cdf(&mut class0_hp_cost, &mvcomp.class0_hp_cdf, None);
        av1_cost_tokens_from_cdf(&mut hp_cost, &mvcomp.hp_cdf, None);
    }

    let c = MV_MAX as usize;
    // mvcost[v] and mvcost[-v]
    let pos = |v: i32| c + v as usize;
    let neg = |v: i32| c - v as usize;

    // Instead of accumulating the cost of each vector component's bits
    //   individually, compute the costs based on smaller vectors. Costs for
    //   [2^exp, 2 * 2^exp - 1] are calculated based on [0, 2^exp - 1]
    //   respectively. Offsets are maintained to swap both 1) class costs when
    //   treated as a complete vector component with the highest set bit when
    //   treated as a mantissa (significand) and 2) leading zeros to account for
    //   the current exponent.

    // Cost offsets
    let mut cost_swap = [0i32; MV_OFFSET_BITS as usize];
    // Delta to convert positive vector to negative vector costs
    let negate_sign = sign_cost[1] - sign_cost[0];

    // Initialize with offsets to swap the class costs with the costs of the
    //   highest set bit.
    for i in 1..MV_OFFSET_BITS as usize {
        cost_swap[i] = bits_cost[i - 1][1];
        if i > CLASS0_BITS as usize {
            cost_swap[i] -= class_cost[i - CLASS0_BITS as usize];
        }
    }

    // Seed the fractional costs onto the output (overwritten latter).
    for o in 0..MV_FP_SIZE {
        for hp in 0..2 {
            let v = 2 * o + hp + 1;
            mvcost_alloc[pos(v)] = fp_cost[o as usize] + hp_cost[hp as usize] + sign_cost[0];
        }
    }

    mvcost_alloc[c] = 0;
    // Fill the costs for each exponent's vectors, using the costs set in the
    //   previous exponents.
    for i in 0..MV_OFFSET_BITS as usize {
        let exponent = (2 * MV_FP_SIZE) << i;

        let mut class = 0;
        if i >= CLASS0_BITS as usize {
            class = class_cost[i - CLASS0_BITS as usize + 1];
        }

        // Iterate through mantissas, keeping track of the location
        //   of the highest set bit for the mantissa.
        // To be clear: in the outer loop, the position of the highest set bit
        //   (exponent) is tracked and, in this loop, the highest set bit of the
        //   mantissa is tracked.
        let mut mantissa = 0;
        for j in 0..=i {
            while mantissa < (2 * MV_FP_SIZE) << j {
                let cost = mvcost_alloc[pos(mantissa + 1)] + class + cost_swap[j];
                let v = exponent + mantissa + 1;
                mvcost_alloc[pos(v)] = cost;
                mvcost_alloc[neg(v)] = cost + negate_sign;
                mantissa += 1;
            }
            cost_swap[j] += bits_cost[i][0];
        }
    }

    // Special case to avoid buffer overrun
    {
        let exponent = (2 * MV_FP_SIZE) << MV_OFFSET_BITS;
        let class = class_cost[MV_CLASSES as usize - 1];
        let mut mantissa = 0;
        for j in 0..MV_OFFSET_BITS as usize {
            while mantissa < (2 * MV_FP_SIZE) << j {
                let cost = mvcost_alloc[pos(mantissa + 1)] + class + cost_swap[j];
                let v = exponent + mantissa + 1;
                mvcost_alloc[pos(v)] = cost;
                mvcost_alloc[neg(v)] = cost + negate_sign;
                mantissa += 1;
            }
        }
        // At this point: mantissa = exponent >> 1

        // Manually calculate the final cost offset
        let cost_swap_hi =
            bits_cost[MV_OFFSET_BITS as usize - 1][1] - class_cost[MV_CLASSES as usize - 2];
        while mantissa < exponent - 1 {
            let cost = mvcost_alloc[pos(mantissa + 1)] + class + cost_swap_hi;
            let v = exponent + mantissa + 1;
            mvcost_alloc[pos(v)] = cost;
            mvcost_alloc[neg(v)] = cost + negate_sign;
            mantissa += 1;
        }
    }

    // Fill costs for class0 vectors, overwriting previous placeholder values
    //   used for calculating the costs of the larger vectors.
    for i in 0..CLASS0_SIZE {
        let top = i * 2 * MV_FP_SIZE;
        for o in 0..MV_FP_SIZE {
            let cost =
                class0_fp_cost[i as usize][o as usize] + class_cost[0] + class0_cost[i as usize];
            for hp in 0..2 {
                let v = top + 2 * o + hp + 1;
                mvcost_alloc[pos(v)] = cost + class0_hp_cost[hp as usize] + sign_cost[0];
                mvcost_alloc[neg(v)] = cost + class0_hp_cost[hp as usize] + sign_cost[1];
            }
        }
    }
}

/// Translation of `av1_encode_mv()`: `cur_frame_force_integer_mv` and
/// `auto_mv_step_size` are the frame feature and the speed feature,
/// `max_mv_magnitude` is `td->max_mv_magnitude`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn av1_encode_mv(
    cur_frame_force_integer_mv: bool,
    auto_mv_step_size: bool,
    w: &mut AomWriter,
    max_mv_magnitude: &mut i32,
    mv: &Mv,
    ref_: &Mv,
    mvctx: &mut NmvContext,
    mut usehp: i32,
) {
    let diff = mv_diff(mv, ref_);
    let j = av1_get_mv_joint(&diff);
    // If the mv_diff is zero, then we should have used near or nearest instead.
    debug_assert!(j != MV_JOINT_ZERO);
    if cur_frame_force_integer_mv {
        usehp = MV_SUBPEL_NONE;
    }
    aom_write_symbol(w, j, &mut mvctx.joints_cdf, MV_JOINTS);
    if mv_joint_vertical(j) {
        encode_mv_component(w, diff.row as i32, &mut mvctx.comps[0], usehp);
    }

    if mv_joint_horizontal(j) {
        encode_mv_component(w, diff.col as i32, &mut mvctx.comps[1], usehp);
    }

    // If auto_mv_step_size is enabled then keep track of the largest
    // motion vector component used.
    if auto_mv_step_size {
        let maxv = (mv.row as i32).abs().max((mv.col as i32).abs()) >> 3;
        *max_mv_magnitude = maxv.max(*max_mv_magnitude);
    }
}

/// Translation of `av1_build_nmv_cost_table()` on the tables of
/// `mv_costs` the `mv_cost_stack` points to.
pub(crate) fn av1_build_nmv_cost_table(mv_costs: &mut MvCosts, ctx: &NmvContext, precision: i32) {
    av1_cost_tokens_from_cdf(&mut mv_costs.nmv_joint_cost, &ctx.joints_cdf, None);
    let tables = if mv_costs.mv_cost_stack_hp {
        &mut mv_costs.nmv_cost_hp_alloc
    } else {
        &mut mv_costs.nmv_cost_alloc
    };
    av1_build_nmv_component_cost_table(&mut tables[0], &ctx.comps[0], precision);
    av1_build_nmv_component_cost_table(&mut tables[1], &ctx.comps[1], precision);
}

/// Translation of `av1_get_ref_mv_from_stack()`.
pub(crate) fn av1_get_ref_mv_from_stack(
    ref_idx: i32,
    ref_frame: &[MvReferenceFrame; 2],
    ref_mv_idx: i32,
    mbmi_ext: &MbModeInfoExt,
) -> IntMv {
    let ref_frame_type = av1_ref_frame_type(ref_frame) as usize;
    let curr_ref_mv_stack: &[CandidateMv] = &mbmi_ext.ref_mv_stack[ref_frame_type];

    if ref_frame[1] > INTRA_FRAME {
        debug_assert!(ref_idx == 0 || ref_idx == 1);
        return if ref_idx != 0 {
            curr_ref_mv_stack[ref_mv_idx as usize].comp_mv
        } else {
            curr_ref_mv_stack[ref_mv_idx as usize].this_mv
        };
    }

    debug_assert!(ref_idx == 0);
    if ref_mv_idx < mbmi_ext.ref_mv_count[ref_frame_type] as i32 {
        curr_ref_mv_stack[ref_mv_idx as usize].this_mv
    } else {
        mbmi_ext.global_mvs[ref_frame_type]
    }
}

/// Translation of `av1_get_ref_mv()`.
pub(crate) fn av1_get_ref_mv(x: &Macroblock, ref_idx: i32) -> IntMv {
    let xd: &MacroBlockD = &x.e_mbd;
    let mbmi = &xd.mi_alloc[xd.mi_grid_base[xd.mi] as usize];
    let mut ref_mv_idx = mbmi.ref_mv_idx as i32;
    if mbmi.mode == NEAR_NEWMV || mbmi.mode == NEW_NEARMV {
        debug_assert!(mbmi.ref_frame[1] > INTRA_FRAME);
        ref_mv_idx += 1;
    }
    av1_get_ref_mv_from_stack(ref_idx, &mbmi.ref_frame, ref_mv_idx, &x.mbmi_ext)
}

/// Translation of `av1_find_best_ref_mvs_from_stack()`.
pub(crate) fn av1_find_best_ref_mvs_from_stack(
    allow_hp: bool,
    mbmi_ext: &MbModeInfoExt,
    ref_frame: MvReferenceFrame,
    nearest_mv: &mut IntMv,
    near_mv: &mut IntMv,
    is_integer: bool,
) {
    let ref_idx = 0;
    let ref_frames = [ref_frame, NONE_FRAME];
    *nearest_mv = av1_get_ref_mv_from_stack(ref_idx, &ref_frames, 0, mbmi_ext);
    let mut m = nearest_mv.as_mv();
    lower_mv_precision(&mut m, allow_hp, is_integer);
    nearest_mv.set_mv(m);
    *near_mv = av1_get_ref_mv_from_stack(ref_idx, &ref_frames, 1, mbmi_ext);
    let mut m = near_mv.as_mv();
    lower_mv_precision(&mut m, allow_hp, is_integer);
    near_mv.set_mv(m);
}
