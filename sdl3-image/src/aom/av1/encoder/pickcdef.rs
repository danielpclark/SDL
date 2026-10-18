// Rust translation of the parts of av1/encoder/pickcdef.c and pickcdef.h
// from aom (https://aomedia.googlesource.com/aom, at the revision
// SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches) the AVIF
// encoders reach: the CDEF strengths predicted from the quantizer
// (`CDEF_PICK_FROM_Q`, the method of the saver and of the real-time usage
// below 1080p) and the search of the filtered frame (the real-time usage's
// `CDEF_FAST_SEARCH_LVL5` from 1080p on).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CDEF parameter selection.

use super::cost::av1_cost_literal;
use super::encoder::*;
use super::rd::rdcost;
use super::speed_features::*;
use crate::aom::aom_dsp::aom_dsp_common::{clamp, get_msb};
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;
use crate::aom::av1::common::alloccommon::try_vec;
use crate::aom::av1::common::av1_common_int::{
    av1_num_planes, frame_is_intra_only, Av1Common, CommonModeInfoParams, CDEF_MAX_STRENGTHS,
};
use crate::aom::av1::common::blockd::{get_plane_block_size, MAX_MB_PLANE, MI_NONE};
use crate::aom::av1::common::cdef::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::quant_common::av1_ac_quant_qtx;

/// `CDEF_REFERENCE`: disable CDEF on non reference frames.
pub(crate) const CDEF_REFERENCE: i32 = 2;

/// Translation of `pick_cdef_from_qp()`.
fn pick_cdef_from_qp(
    cm: &mut Av1Common,
    skip_cdef: bool,
    is_screen_content: bool,
) -> Result<(), ()> {
    let bd = cm.seq_params.bit_depth;
    let q = av1_ac_quant_qtx(cm.quant_params.base_qindex, 0, bd) as i32 >> (bd - 8);
    let intra_only = frame_is_intra_only(cm);
    let cdef_info = &mut cm.cdef_info;
    // Check the speed feature to avoid extra signaling.
    if skip_cdef {
        cdef_info.cdef_bits = 1;
        cdef_info.nb_cdef_strengths = 2;
    } else {
        cdef_info.cdef_bits = 0;
        cdef_info.nb_cdef_strengths = 1;
    }
    cdef_info.cdef_damping = 3 + (cm.quant_params.base_qindex >> 6);

    let predicted_y_f1;
    let predicted_y_f2;
    let predicted_uv_f1;
    let predicted_uv_f2;
    if is_screen_content {
        let qd = q as f64;
        predicted_y_f1 = clamp(
            (5.88217781e-06 * qd * qd + 6.10391455e-03 * qd + 9.95043102e-02) as i32,
            0,
            15,
        );
        predicted_y_f2 = clamp(
            (-7.79934857e-06 * qd * qd + 6.58957830e-03 * qd + 8.81045025e-01) as i32,
            0,
            3,
        );
        predicted_uv_f1 = clamp(
            (-6.79500136e-06 * qd * qd + 1.02695586e-02 * qd + 1.36126802e-01) as i32,
            0,
            15,
        );
        predicted_uv_f2 = clamp(
            (-9.99613695e-08 * qd * qd - 1.79361339e-05 * qd + 1.17022324e+0) as i32,
            0,
            3,
        );
    } else {
        // (the products are `int * int * float` in C: the integer square is
        // converted to float before the multiplication)
        let qq = (q * q) as f32;
        let qf = q as f32;
        if !intra_only {
            predicted_y_f1 = clamp(
                (qq * -0.0000023593946f32 + qf * 0.0068615186f32 + 0.02709886f32).round() as i32,
                0,
                15,
            );
            predicted_y_f2 = clamp(
                (qq * -0.00000057629734f32 + qf * 0.0013993345f32 + 0.03831067f32).round() as i32,
                0,
                3,
            );
            predicted_uv_f1 = clamp(
                (qq * -0.0000007095069f32 + qf * 0.0034628846f32 + 0.00887099f32).round() as i32,
                0,
                15,
            );
            predicted_uv_f2 = clamp(
                (qq * 0.00000023874085f32 + qf * 0.00028223585f32 + 0.05576307f32).round() as i32,
                0,
                3,
            );
        } else {
            predicted_y_f1 = clamp(
                (qq * 0.0000033731974f32 + qf * 0.008070594f32 + 0.0187634f32).round() as i32,
                0,
                15,
            );
            predicted_y_f2 = clamp(
                (qq * 0.0000029167343f32 + qf * 0.0027798624f32 + 0.0079405f32).round() as i32,
                0,
                3,
            );
            predicted_uv_f1 = clamp(
                (qq * -0.0000130790995f32 + qf * 0.012892405f32 - 0.00748388f32).round() as i32,
                0,
                15,
            );
            predicted_uv_f2 = clamp(
                (qq * 0.0000032651783f32 + qf * 0.00035520183f32 + 0.00228092f32).round() as i32,
                0,
                3,
            );
        }
    }
    cdef_info.cdef_strengths[0] = predicted_y_f1 * CDEF_SEC_STRENGTHS + predicted_y_f2;
    cdef_info.cdef_uv_strengths[0] = predicted_uv_f1 * CDEF_SEC_STRENGTHS + predicted_uv_f2;

    // mbmi->cdef_strength is already set in the encoding stage. We don't need to
    // set it again here.
    if skip_cdef {
        cdef_info.cdef_strengths[1] = 0;
        cdef_info.cdef_uv_strengths[1] = 0;
        return Ok(());
    }

    let mi_params = &mut cm.mi_params;
    let nvfb = (mi_params.mi_rows + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    let nhfb = (mi_params.mi_cols + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    for r in 0..nvfb {
        for c in 0..nhfb {
            let idx = (MI_SIZE_64X64 * r * mi_params.mi_stride + MI_SIZE_64X64 * c) as usize;
            let i = *mi_params.mi_grid_base.get(idx).ok_or(())?;
            if i == MI_NONE {
                return Err(());
            }
            let current_mbmi = mi_params.mi_alloc.get_mut(i as usize).ok_or(())?;
            current_mbmi.cdef_strength = 0;
        }
    }
    Ok(())
}

/// Translation of `av1_cdef_search()`.
pub(crate) fn av1_cdef_search(cpi: &mut Av1Comp, is_screen_content: bool) -> Result<(), ()> {
    let cdef_control = cpi.oxcf.tool_cfg.cdef_control;
    let non_reference_frame = cpi.ppi.rtc_ref.non_reference_frame;
    let pick_method = cpi.sf.lpf_sf.cdef_pick_method;
    let skip_cdef_feature = cpi.sf.rt_sf.skip_cdef_sb != 0;
    let cm = &mut cpi.common;
    debug_assert!(cdef_control != CDEF_NONE);
    if cdef_control == CDEF_REFERENCE && non_reference_frame {
        let cdef_info = &mut cm.cdef_info;
        cdef_info.nb_cdef_strengths = 1;
        cdef_info.cdef_bits = 0;
        cdef_info.cdef_strengths[0] = 0;
        cdef_info.cdef_uv_strengths[0] = 0;
        return Ok(());
    }

    if pick_method == CDEF_PICK_FROM_Q {
        return pick_cdef_from_qp(cm, skip_cdef_feature, is_screen_content);
    }
    let rdmult = cpi.td.mb.rdmult;
    let cm = &cpi.common;
    let damping = 3 + (cm.quant_params.base_qindex >> 6);
    let fast = (CDEF_FAST_SEARCH_LVL1..=CDEF_FAST_SEARCH_LVL5).contains(&pick_method);
    let num_planes = av1_num_planes(cm);
    let frame = &cm.cur_frame().buf;
    let reference = &cpi.source_buf;
    // Initialize parameters related to CDEF search context.
    // Allocate CDEF search context buffers.
    // (an allocation failure is an error here)
    let mut ctx = cdef_params_init(frame, reference, cm, pick_method)?;
    // Frame level mse calculation.
    // (av1_cdef_mse_calc_frame_mt() with several workers: the same results)
    cdef_mse_calc_frame(
        &mut ctx,
        &cm.mi_params,
        &frame.buffer_alloc,
        &reference.buffer_alloc,
    )?;

    /* Search for different number of signaling bits. */
    let mut nb_strength_bits = 0;
    let mut best_rd = u64::MAX;
    let sb_count = ctx.sb_count;
    let mse: [&[[u64; TOTAL_STRENGTHS]]; 2] = [&ctx.mse[0], &ctx.mse[1]];
    /* Calculate the maximum number of bits required to signal CDEF strengths at
     * block level */
    let total_strengths = nb_cdef_strengths(pick_method)? as i32;
    let joint_strengths = if num_planes > 1 {
        total_strengths * total_strengths
    } else {
        total_strengths
    };
    let max_signaling_bits = if joint_strengths == 1 {
        0
    } else {
        get_msb((joint_strengths - 1) as u32) + 1
    };
    let mut cdef_strengths = cm.cdef_info.cdef_strengths;
    let mut cdef_uv_strengths = cm.cdef_info.cdef_uv_strengths;
    for i in 0..=3 {
        if i > max_signaling_bits {
            break;
        }
        // (best_lev0 is uninitialized in C: only its first nb_strengths
        // entries are read, after they are written)
        let mut best_lev0 = [0i32; CDEF_MAX_STRENGTHS];
        let mut best_lev1 = [0i32; CDEF_MAX_STRENGTHS];
        let nb_strengths = 1usize << i;
        let tot_mse = if num_planes > 1 {
            joint_strength_search_dual(
                &mut best_lev0,
                &mut best_lev1,
                nb_strengths,
                mse,
                sb_count,
                pick_method,
            )?
        } else {
            joint_strength_search(&mut best_lev0, nb_strengths, mse[0], sb_count, pick_method)?
        };

        let total_bits = sb_count as i32 * i
            + nb_strengths as i32 * CDEF_STRENGTH_BITS * if num_planes > 1 { 2 } else { 1 };
        let rate_cost = av1_cost_literal(total_bits);
        let dist = tot_mse.wrapping_mul(16);
        let rd = rdcost(rdmult, rate_cost, dist as i64) as u64;
        if rd < best_rd {
            best_rd = rd;
            nb_strength_bits = i;
            cdef_strengths[..nb_strengths].copy_from_slice(&best_lev0[..nb_strengths]);
            if num_planes > 1 {
                cdef_uv_strengths[..nb_strengths].copy_from_slice(&best_lev1[..nb_strengths]);
            }
        }
    }

    let nb_cdef_strengths_frame = 1usize << nb_strength_bits;
    let mut best_gis = try_vec(sb_count, 0u8)?;
    for (i, best) in best_gis.iter_mut().enumerate() {
        let mut best_mse = u64::MAX;
        let mut best_gi = 0;
        for gi in 0..nb_cdef_strengths_frame {
            let mut curr = mse[0][i][cdef_strengths[gi] as usize];
            if num_planes > 1 {
                curr = curr.wrapping_add(mse[1][i][cdef_uv_strengths[gi] as usize]);
            }
            if curr < best_mse {
                best_gi = gi;
                best_mse = curr;
            }
        }
        *best = best_gi as u8;
    }
    let sb_index = std::mem::take(&mut ctx.sb_index);
    let cm = &mut cpi.common;
    let cdef_info = &mut cm.cdef_info;
    cdef_info.cdef_strengths = cdef_strengths;
    cdef_info.cdef_uv_strengths = cdef_uv_strengths;
    cdef_info.cdef_bits = nb_strength_bits;
    cdef_info.nb_cdef_strengths = 1 << nb_strength_bits;
    for (i, &best_gi) in best_gis.iter().enumerate() {
        let mi_params = &mut cm.mi_params;
        let g = *mi_params
            .mi_grid_base
            .get(usize::try_from(sb_index[i]).map_err(|_| ())?)
            .ok_or(())?;
        if g == MI_NONE {
            return Err(());
        }
        mi_params
            .mi_alloc
            .get_mut(g as usize)
            .ok_or(())?
            .cdef_strength = best_gi as _;
    }
    if fast {
        for j in 0..cdef_info.nb_cdef_strengths as usize {
            let luma_strength = cdef_info.cdef_strengths[j];
            let chroma_strength = cdef_info.cdef_uv_strengths[j];

            cdef_info.cdef_strengths[j] = store_cdef_filter_strength(pick_method, luma_strength)?;
            cdef_info.cdef_uv_strengths[j] =
                store_cdef_filter_strength(pick_method, chroma_strength)?;
        }
    }

    cdef_info.cdef_damping = damping;
    // Deallocate CDEF search context buffers.
    Ok(())
}

// (pickcdef.h)
const REDUCED_PRI_STRENGTHS_LVL1: i32 = 8;
const REDUCED_PRI_STRENGTHS_LVL2: i32 = 5;
const REDUCED_SEC_STRENGTHS_LVL3: i32 = 2;
const REDUCED_SEC_STRENGTHS_LVL5: i32 = 1;
const REDUCED_PRI_STRENGTHS_LVL4: i32 = 2;

const REDUCED_TOTAL_STRENGTHS_LVL1: i32 = REDUCED_PRI_STRENGTHS_LVL1 * CDEF_SEC_STRENGTHS;
const REDUCED_TOTAL_STRENGTHS_LVL2: i32 = REDUCED_PRI_STRENGTHS_LVL2 * CDEF_SEC_STRENGTHS;
const REDUCED_TOTAL_STRENGTHS_LVL3: i32 = REDUCED_PRI_STRENGTHS_LVL2 * REDUCED_SEC_STRENGTHS_LVL3;
const REDUCED_TOTAL_STRENGTHS_LVL4: i32 = REDUCED_PRI_STRENGTHS_LVL4 * REDUCED_SEC_STRENGTHS_LVL3;
const REDUCED_TOTAL_STRENGTHS_LVL5: i32 = REDUCED_PRI_STRENGTHS_LVL4 * REDUCED_SEC_STRENGTHS_LVL5;
const TOTAL_STRENGTHS: usize = (CDEF_PRI_STRENGTHS * CDEF_SEC_STRENGTHS) as usize;

const PRICONV_LVL1: [i32; REDUCED_PRI_STRENGTHS_LVL1 as usize] = [0, 1, 2, 3, 5, 7, 10, 13];
const PRICONV_LVL2: [i32; REDUCED_PRI_STRENGTHS_LVL2 as usize] = [0, 2, 4, 8, 14];
const PRICONV_LVL4: [i32; REDUCED_PRI_STRENGTHS_LVL4 as usize] = [0, 11];
const PRICONV_LVL5: [i32; REDUCED_PRI_STRENGTHS_LVL4 as usize] = [0, 5];
const SECCONV_LVL3: [i32; REDUCED_SEC_STRENGTHS_LVL3 as usize] = [0, 2];
const SECCONV_LVL5: [i32; REDUCED_SEC_STRENGTHS_LVL5 as usize] = [0];
const NB_CDEF_STRENGTHS: [i32; CDEF_PICK_METHODS as usize] = [
    TOTAL_STRENGTHS as i32,
    REDUCED_TOTAL_STRENGTHS_LVL1,
    REDUCED_TOTAL_STRENGTHS_LVL2,
    REDUCED_TOTAL_STRENGTHS_LVL3,
    REDUCED_TOTAL_STRENGTHS_LVL4,
    REDUCED_TOTAL_STRENGTHS_LVL5,
    TOTAL_STRENGTHS as i32,
];

/// The number of strengths of a search method (`nb_cdef_strengths[]`).
fn nb_cdef_strengths(pick_method: i32) -> Result<usize, ()> {
    NB_CDEF_STRENGTHS
        .get(usize::try_from(pick_method).map_err(|_| ())?)
        .map(|&n| n as usize)
        .ok_or(())
}

/// Translation of `get_cdef_filter_strengths()`: the primary and secondary
/// filter strengths for the given strength index and search method.
fn get_cdef_filter_strengths(pick_method: i32, strength_idx: i32) -> Result<(i32, i32), ()> {
    let tot_sec_filter = if pick_method == CDEF_FAST_SEARCH_LVL5 {
        REDUCED_SEC_STRENGTHS_LVL5
    } else if pick_method >= CDEF_FAST_SEARCH_LVL3 {
        REDUCED_SEC_STRENGTHS_LVL3
    } else {
        CDEF_SEC_STRENGTHS
    };
    let pri_idx = strength_idx / tot_sec_filter;
    let sec_idx = strength_idx % tot_sec_filter;
    let at = |t: &[i32], i: i32| -> Result<i32, ()> {
        t.get(usize::try_from(i).map_err(|_| ())?)
            .copied()
            .ok_or(())
    };
    if pick_method == CDEF_FULL_SEARCH {
        return Ok((pri_idx, sec_idx));
    }

    match pick_method {
        CDEF_FAST_SEARCH_LVL1 => Ok((at(&PRICONV_LVL1, pri_idx)?, sec_idx)),
        CDEF_FAST_SEARCH_LVL2 => Ok((at(&PRICONV_LVL2, pri_idx)?, sec_idx)),
        CDEF_FAST_SEARCH_LVL3 => Ok((at(&PRICONV_LVL2, pri_idx)?, at(&SECCONV_LVL3, sec_idx)?)),
        CDEF_FAST_SEARCH_LVL4 => Ok((at(&PRICONV_LVL4, pri_idx)?, at(&SECCONV_LVL3, sec_idx)?)),
        CDEF_FAST_SEARCH_LVL5 => Ok((at(&PRICONV_LVL5, pri_idx)?, at(&SECCONV_LVL5, sec_idx)?)),
        // Invalid CDEF search method
        _ => Err(()),
    }
}

/// `STORE_CDEF_FILTER_STRENGTH()`: the CDEF filter strength calculated from
/// the strength index for the given search method.
fn store_cdef_filter_strength(pick_method: i32, strength_idx: i32) -> Result<i32, ()> {
    let (pri_strength, sec_strength) = get_cdef_filter_strengths(pick_method, strength_idx)?;
    Ok(pri_strength * CDEF_SEC_STRENGTHS + sec_strength)
}

/* Search for the best strength to add as an option, knowing we
already selected nb_strengths options. */
/// Translation of `search_one()`.
fn search_one(
    lev: &mut [i32; CDEF_MAX_STRENGTHS],
    nb_strengths: usize,
    mse: &[[u64; TOTAL_STRENGTHS]],
    sb_count: usize,
    pick_method: i32,
) -> Result<u64, ()> {
    let mut tot_mse = [0u64; TOTAL_STRENGTHS];
    let total_strengths = nb_cdef_strengths(pick_method)?;
    let mut best_tot_mse = 1u64 << 63;
    let mut best_id = 0;
    for m in &mse[..sb_count] {
        let mut best_mse = 1u64 << 63;
        /* Find best mse among already selected options. */
        for &l in &lev[..nb_strengths] {
            if m[l as usize] < best_mse {
                best_mse = m[l as usize];
            }
        }
        /* Find best mse when adding each possible new option. */
        for j in 0..total_strengths {
            let mut best = best_mse;
            if m[j] < best {
                best = m[j];
            }
            tot_mse[j] = tot_mse[j].wrapping_add(best);
        }
    }
    for (j, &t) in tot_mse[..total_strengths].iter().enumerate() {
        if t < best_tot_mse {
            best_tot_mse = t;
            best_id = j as i32;
        }
    }
    lev[nb_strengths] = best_id;
    Ok(best_tot_mse)
}

/* Search for the best luma+chroma strength to add as an option, knowing we
already selected nb_strengths options. */
/// Translation of `search_one_dual()`.
fn search_one_dual(
    lev0: &mut [i32; CDEF_MAX_STRENGTHS],
    lev1: &mut [i32; CDEF_MAX_STRENGTHS],
    nb_strengths: usize,
    mse: [&[[u64; TOTAL_STRENGTHS]]; 2],
    sb_count: usize,
    pick_method: i32,
) -> Result<u64, ()> {
    let mut tot_mse = [[0u64; TOTAL_STRENGTHS]; TOTAL_STRENGTHS];
    let mut best_tot_mse = 1u64 << 63;
    let mut best_id0 = 0;
    let mut best_id1 = 0;
    let total_strengths = nb_cdef_strengths(pick_method)?;
    for i in 0..sb_count {
        let (m0, m1) = (&mse[0][i], &mse[1][i]);
        let mut best_mse = 1u64 << 63;
        /* Find best mse among already selected options. */
        for gi in 0..nb_strengths {
            let mut curr = m0[lev0[gi] as usize];
            curr = curr.wrapping_add(m1[lev1[gi] as usize]);
            if curr < best_mse {
                best_mse = curr;
            }
        }
        /* Find best mse when adding each possible new option. */
        for j in 0..total_strengths {
            for k in 0..total_strengths {
                let mut best = best_mse;
                let mut curr = m0[j];
                curr = curr.wrapping_add(m1[k]);
                if curr < best {
                    best = curr;
                }
                tot_mse[j][k] = tot_mse[j][k].wrapping_add(best);
            }
        }
    }
    for j in 0..total_strengths {
        for k in 0..total_strengths {
            if tot_mse[j][k] < best_tot_mse {
                best_tot_mse = tot_mse[j][k];
                best_id0 = j as i32;
                best_id1 = k as i32;
            }
        }
    }
    lev0[nb_strengths] = best_id0;
    lev1[nb_strengths] = best_id1;
    Ok(best_tot_mse)
}

/* Search for the set of strengths that minimizes mse. */
/// Translation of `joint_strength_search()`.
fn joint_strength_search(
    best_lev: &mut [i32; CDEF_MAX_STRENGTHS],
    nb_strengths: usize,
    mse: &[[u64; TOTAL_STRENGTHS]],
    sb_count: usize,
    pick_method: i32,
) -> Result<u64, ()> {
    let fast = (CDEF_FAST_SEARCH_LVL1..=CDEF_FAST_SEARCH_LVL5).contains(&pick_method);
    let mut best_tot_mse = 1u64 << 63;
    /* Greedy search: add one strength options at a time. */
    for i in 0..nb_strengths {
        best_tot_mse = search_one(best_lev, i, mse, sb_count, pick_method)?;
    }
    /* Trying to refine the greedy search by reconsidering each
    already-selected option. */
    if !fast {
        for _ in 0..4 * nb_strengths {
            for j in 0..nb_strengths - 1 {
                best_lev[j] = best_lev[j + 1];
            }
            best_tot_mse = search_one(best_lev, nb_strengths - 1, mse, sb_count, pick_method)?;
        }
    }
    Ok(best_tot_mse)
}

/* Search for the set of luma+chroma strengths that minimizes mse. */
/// Translation of `joint_strength_search_dual()`.
fn joint_strength_search_dual(
    best_lev0: &mut [i32; CDEF_MAX_STRENGTHS],
    best_lev1: &mut [i32; CDEF_MAX_STRENGTHS],
    nb_strengths: usize,
    mse: [&[[u64; TOTAL_STRENGTHS]]; 2],
    sb_count: usize,
    pick_method: i32,
) -> Result<u64, ()> {
    let mut best_tot_mse = 1u64 << 63;
    /* Greedy search: add one strength options at a time. */
    for i in 0..nb_strengths {
        best_tot_mse = search_one_dual(best_lev0, best_lev1, i, mse, sb_count, pick_method)?;
    }
    /* Trying to refine the greedy search by reconsidering each
    already-selected option. */
    for _ in 0..4 * nb_strengths {
        for j in 0..nb_strengths - 1 {
            best_lev0[j] = best_lev0[j + 1];
            best_lev1[j] = best_lev1[j + 1];
        }
        best_tot_mse = search_one_dual(
            best_lev0,
            best_lev1,
            nb_strengths - 1,
            mse,
            sb_count,
            pick_method,
        )?;
    }
    Ok(best_tot_mse)
}

/// Translation of `init_src_params()`: `(src_stride, width, height,
/// width_log2, height_log2)`.
fn init_src_params(bsize: BlockSize) -> (i32, i32, i32, i32, i32) {
    let w = BLOCK_SIZE_WIDE[bsize as usize] as i32;
    let h = BLOCK_SIZE_HIGH[bsize as usize] as i32;
    let wl2 = MI_SIZE_LOG2 + MI_SIZE_WIDE_LOG2[bsize as usize] as i32;
    // FIXME (upstream): the height's log2 is the width's, which only
    // matters for the 4:2:2 and 4:4:0 subsamplings' blocks.
    let hl2 = MI_SIZE_LOG2 + MI_SIZE_WIDE_LOG2[bsize as usize] as i32;
    (w, w, h, wl2, hl2)
}

/// `aom_mse_wxh_16bit_highbd_c()` (and `aom_mse_wxh_16bit_c()`: the frame
/// buffers hold 16-bit samples at either bit depth here).
#[allow(clippy::too_many_arguments)]
fn aom_mse_wxh_16bit(
    dst: &[u16],
    dst_off: usize,
    dstride: i32,
    src: &[u16],
    src_off: usize,
    sstride: i32,
    w: i32,
    h: i32,
) -> Result<u64, ()> {
    let mut sum = 0u64;
    for i in 0..h as usize {
        let d = dst
            .get(dst_off + i * dstride as usize..)
            .and_then(|d| d.get(..w as usize))
            .ok_or(())?;
        let s = src
            .get(src_off + i * sstride as usize..)
            .and_then(|s| s.get(..w as usize))
            .ok_or(())?;
        for (&a, &b) in d.iter().zip(s) {
            let e = a as i32 - b as i32;
            sum += (e * e) as u64;
        }
    }
    Ok(sum)
}

/* Compute MSE only on the blocks we filtered. */
/// Translation of `compute_cdef_dist_highbd()` (the 8-bit frames'
/// `compute_cdef_dist()` is never called: `get_filt_error()` computes
/// their errors itself).
#[allow(clippy::too_many_arguments)]
fn compute_cdef_dist_highbd(
    dst: &[u16],
    dst_base: usize,
    dstride: i32,
    src: &[u16],
    dlist: &[CdefList],
    cdef_count: i32,
    bsize: BlockSize,
    coeff_shift: i32,
    row: i32,
    col: i32,
) -> Result<u64, ()> {
    debug_assert!(
        bsize == BLOCK_4X4 || bsize == BLOCK_4X8 || bsize == BLOCK_8X4 || bsize == BLOCK_8X8
    );
    let mut sum = 0u64;
    let dst_buff = dst_base + (row * dstride + col) as usize;
    let (src_stride, width, height, width_log2, height_log2) = init_src_params(bsize);
    for (bi, d) in dlist[..cdef_count as usize].iter().enumerate() {
        let by = d.by as i32;
        let bx = d.bx as i32;
        sum += aom_mse_wxh_16bit(
            dst,
            dst_buff + ((by << height_log2) * dstride + (bx << width_log2)) as usize,
            dstride,
            src,
            bi << (height_log2 + width_log2),
            src_stride,
            width,
            height,
        )?;
    }
    Ok(sum >> (2 * coeff_shift))
}

/// Translation of `aom_sse_c()`.
#[allow(clippy::too_many_arguments)]
fn aom_sse(
    a: &[u16],
    a_off: usize,
    a_stride: i32,
    b: &[u16],
    b_off: usize,
    b_stride: i32,
    width: i32,
    height: i32,
) -> Result<i64, ()> {
    let mut sse = 0i64;
    for y in 0..height as usize {
        let ra = a
            .get(a_off + y * a_stride as usize..)
            .and_then(|r| r.get(..width as usize))
            .ok_or(())?;
        let rb = b
            .get(b_off + y * b_stride as usize..)
            .and_then(|r| r.get(..width as usize))
            .ok_or(())?;
        for (&p, &q) in ra.iter().zip(rb) {
            let diff = (p as i32 - q as i32).abs();
            sse += (diff * diff) as i64;
        }
    }
    Ok(sse)
}

// Fill the boundary regions of the block with CDEF_VERY_LARGE, only if the
// region is outside frame boundary
/// Translation of `fill_borders_for_fbs_on_frame_boundary()`.
fn fill_borders_for_fbs_on_frame_boundary(
    inbuf: &mut [u16],
    hfilt_size: i32,
    vfilt_size: i32,
    is_fb_on_frm_left_boundary: bool,
    is_fb_on_frm_right_boundary: bool,
    is_fb_on_frm_top_boundary: bool,
    is_fb_on_frm_bottom_boundary: bool,
) -> Result<(), ()> {
    if !is_fb_on_frm_left_boundary
        && !is_fb_on_frm_right_boundary
        && !is_fb_on_frm_top_boundary
        && !is_fb_on_frm_bottom_boundary
    {
        return Ok(());
    }
    let (vb, hb, bs) = (CDEF_VBORDER, CDEF_HBORDER, CDEF_BSTRIDE);
    if is_fb_on_frm_bottom_boundary {
        // Fill bottom region of the block
        let buf_offset = (vfilt_size + vb) * bs + hb;
        fill_rect(
            inbuf,
            buf_offset as usize,
            bs,
            vb,
            hfilt_size,
            CDEF_VERY_LARGE,
        )?;
    }
    if is_fb_on_frm_bottom_boundary || is_fb_on_frm_left_boundary {
        let buf_offset = (vfilt_size + vb) * bs;
        // Fill bottom-left region of the block
        fill_rect(inbuf, buf_offset as usize, bs, vb, hb, CDEF_VERY_LARGE)?;
    }
    if is_fb_on_frm_bottom_boundary || is_fb_on_frm_right_boundary {
        let buf_offset = (vfilt_size + vb) * bs + hfilt_size + hb;
        // Fill bottom-right region of the block
        fill_rect(inbuf, buf_offset as usize, bs, vb, hb, CDEF_VERY_LARGE)?;
    }
    if is_fb_on_frm_top_boundary {
        // Fill top region of the block
        fill_rect(inbuf, hb as usize, bs, vb, hfilt_size, CDEF_VERY_LARGE)?;
    }
    if is_fb_on_frm_top_boundary || is_fb_on_frm_left_boundary {
        // Fill top-left region of the block
        fill_rect(inbuf, 0, bs, vb, hb, CDEF_VERY_LARGE)?;
    }
    if is_fb_on_frm_top_boundary || is_fb_on_frm_right_boundary {
        let buf_offset = hfilt_size + hb;
        // Fill top-right region of the block
        fill_rect(inbuf, buf_offset as usize, bs, vb, hb, CDEF_VERY_LARGE)?;
    }
    if is_fb_on_frm_left_boundary {
        let buf_offset = vb * bs;
        // Fill left region of the block
        fill_rect(
            inbuf,
            buf_offset as usize,
            bs,
            vfilt_size,
            hb,
            CDEF_VERY_LARGE,
        )?;
    }
    if is_fb_on_frm_right_boundary {
        let buf_offset = vb * bs;
        // Fill right region of the block
        fill_rect(
            inbuf,
            (buf_offset + hfilt_size + hb) as usize,
            bs,
            vfilt_size,
            hb,
            CDEF_VERY_LARGE,
        )?;
    }
    Ok(())
}

// Calculate the number of 8x8/4x4 filter units for which SSE can be calculated
// after CDEF filtering in single function call
/// Translation of `get_error_calc_width_in_filt_units()`.
fn get_error_calc_width_in_filt_units(
    dlist: &[CdefList],
    cdef_count: usize,
    bi: usize,
    subsampling_x: i32,
    subsampling_y: i32,
) -> i32 {
    // TODO(Ranjit): Extend the optimization for 422
    if subsampling_x != subsampling_y {
        return 1;
    }

    // Combining more blocks seems to increase encode time due to increase in
    // control code
    if bi + 3 < cdef_count
        && dlist[bi].by == dlist[bi + 3].by
        && dlist[bi].bx as i32 + 3 == dlist[bi + 3].bx as i32
    {
        /* Calculate error for four 8x8/4x4 blocks using 32x8/16x4 block specific
         * logic if y co-ordinates match and x co-ordinates are
         * separated by 3 for first and fourth 8x8/4x4 blocks in dlist[]. */
        return 4;
    }
    if bi + 1 < cdef_count
        && dlist[bi].by == dlist[bi + 1].by
        && dlist[bi].bx as i32 + 1 == dlist[bi + 1].bx as i32
    {
        /* Calculate error for two 8x8/4x4 blocks using 16x8/8x4 block specific
         * logic if their y co-ordinates match and x co-ordinates are
         * separated by 1 for first and second 8x8/4x4 blocks in dlist[]. */
        return 2;
    }
    1
}

/// The block state `av1_cdef_mse_calc_block()` shares across the planes
/// and strengths of a filter block (`dir`, `dirinit`, `var` and the list).
struct CdefSearchBlock {
    dlist: Vec<CdefList>,
    dir: [[i32; CDEF_NBLOCKS]; CDEF_NBLOCKS],
    var: [[i32; CDEF_NBLOCKS]; CDEF_NBLOCKS],
    dirinit: bool,
    cdef_count: i32,
}

/// Translation of `av1_cdef_filter_fb()` as `av1_cdef_search()` calls it
/// (`dirinit` set): to an 8-bit buffer at the blocks' positions, or to a
/// 16-bit buffer with the blocks one after another.
#[allow(clippy::too_many_arguments)]
fn cdef_filter_fb_search(
    dst8: Option<(&mut [u16], i32)>,
    dst16: Option<&mut [u16]>,
    input: &[u16],
    in_off: usize,
    xdec: i32,
    ydec: i32,
    blk: &mut CdefSearchBlock,
    pli: usize,
    level: i32,
    sec_strength: i32,
    damping: i32,
    coeff_shift: i32,
) -> Result<(), ()> {
    let pri_strength = level << coeff_shift;
    let sec_strength = sec_strength << coeff_shift;
    let damping = damping + coeff_shift - (pli != AOM_PLANE_Y) as i32;
    let bw_log2 = 3 - xdec;
    let bh_log2 = 3 - ydec;
    let cdef_count = blk.cdef_count as usize;
    let bs = CDEF_BSTRIDE;
    if pri_strength == 0 && sec_strength == 0 {
        // If we're here, both primary and secondary strengths are 0, and
        // we still haven't written anything to y[] yet, so we just copy
        // the input to y[]. This is necessary only for av1_cdef_search()
        // and only av1_cdef_search() sets dirinit.
        // (only the 16-bit buffer is filtered at zero strengths)
        let dst16 = dst16.ok_or(())?;
        for (bi, d) in blk.dlist[..cdef_count].iter().enumerate() {
            let by = d.by as i32;
            let bx = d.bx as i32;
            // TODO(stemidts/jmvalin): SIMD optimisations
            for iy in 0..1 << bh_log2 {
                let n = 1usize << bw_log2;
                let o = (bi << (bw_log2 + bh_log2)) + ((iy as usize) << bw_log2);
                let s = in_off + (((by << bh_log2) + iy) * bs + (bx << bw_log2)) as usize;
                dst16
                    .get_mut(o..o + n)
                    .ok_or(())?
                    .copy_from_slice(input.get(s..s + n).ok_or(())?);
            }
        }
        return Ok(());
    }

    if pli == 0 && !blk.dirinit {
        aom_cdef_find_dir(
            input,
            in_off,
            &blk.dlist,
            &mut blk.var,
            blk.cdef_count,
            coeff_shift,
            &mut blk.dir,
        );
        blk.dirinit = true;
    }
    if pli == 1 && xdec != ydec {
        const CONV422: [i32; 8] = [7, 0, 2, 4, 5, 6, 6, 6];
        const CONV440: [i32; 8] = [1, 2, 2, 2, 3, 4, 6, 0];
        for d in &blk.dlist[..cdef_count] {
            let (by, bx) = (d.by as usize, d.bx as usize);
            let v = blk.dir[by][bx];
            blk.dir[by][bx] = *(if xdec != 0 { &CONV422 } else { &CONV440 })
                .get(usize::try_from(v).map_err(|_| ())?)
                .ok_or(())?;
        }
    }

    let block_width = 8 >> xdec;
    let block_height = 8 >> ydec;
    /*
     * strength_index == 0 : enable_primary = 1, enable_secondary = 1
     * strength_index == 1 : enable_primary = 1, enable_secondary = 0
     * strength_index == 2 : enable_primary = 0, enable_secondary = 1
     * strength_index == 3 : enable_primary = 0, enable_secondary = 0
     */
    let (dst, is_dst8, dstride) = match (dst8, dst16) {
        (Some((d, stride)), _) => (d, true, stride),
        (None, Some(d)) => (d, false, 1 << bw_log2),
        (None, None) => return Err(()),
    };
    for (bi, d) in blk.dlist[..cdef_count].iter().enumerate() {
        let by = d.by as i32;
        let bx = d.bx as i32;
        let t = if pli != 0 {
            pri_strength
        } else {
            adjust_strength(pri_strength, blk.var[by as usize][bx as usize])
        };
        let strength_index = (sec_strength == 0) as i32 | (((t == 0) as i32) << 1);
        let dst_off = if is_dst8 {
            ((by << bh_log2) * dstride + (bx << bw_log2)) as usize
        } else {
            bi << (bw_log2 + bh_log2)
        };
        cdef_filter_block_internal(
            dst,
            dst_off,
            is_dst8,
            dstride,
            input,
            in_off + ((by * bs << bh_log2) + (bx << bw_log2)) as usize,
            t,
            sec_strength,
            if pri_strength != 0 {
                blk.dir[by as usize][bx as usize]
            } else {
                0
            },
            damping,
            damping,
            coeff_shift,
            block_width,
            block_height,
            strength_index & 2 == 0,
            strength_index & 1 == 0,
        )?;
    }
    Ok(())
}

/// The parts of `CdefSearchCtx` the search reads (the frame and the source
/// are passed along).
struct CdefSearchCtx {
    /// `plane[].dst`: the planes' offsets and strides in the frame buffer
    plane_buf: [usize; MAX_MB_PLANE],
    plane_stride: [i32; MAX_MB_PLANE],
    /// `ref`: the source planes' offsets and strides
    ref_buf: [usize; MAX_MB_PLANE],
    ref_stride: [i32; MAX_MB_PLANE],
    total_strengths: usize,
    coeff_shift: i32,
    damping: i32,
    pick_method: i32,
    num_planes: usize,
    mi_wide_l2: [i32; MAX_MB_PLANE],
    mi_high_l2: [i32; MAX_MB_PLANE],
    xdec: [i32; MAX_MB_PLANE],
    ydec: [i32; MAX_MB_PLANE],
    bsize: [BlockSize; MAX_MB_PLANE],
    nvfb: i32,
    nhfb: i32,
    mse: [Vec<[u64; TOTAL_STRENGTHS]>; 2],
    sb_index: Vec<i32>,
    sb_count: usize,
    use_highbitdepth: bool,
}

// Returns the block error after CDEF filtering for a given strength
/// Translation of `get_filt_error()`.
#[allow(clippy::too_many_arguments)]
fn get_filt_error(
    ctx: &CdefSearchCtx,
    frame: &[u16],
    reference: &[u16],
    blk: &mut CdefSearchBlock,
    input: &[u16],
    in_off: usize,
    row: i32,
    col: i32,
    pri_strength: i32,
    sec_strength: i32,
    pli: usize,
    coeff_shift: i32,
    bs: BlockSize,
    tmp: &mut [u16],
) -> Result<u64, ()> {
    let mut curr_sse = 0u64;
    let (ssx, ssy) = (ctx.xdec[pli], ctx.ydec[pli]);
    let plane_bsize = get_plane_block_size(bs, ssx, ssy);
    let bw_log2 = 3 - ssx;
    let bh_log2 = 3 - ssy;
    let ref_stride = ctx.ref_stride[pli];
    let ref_base = ctx.ref_buf[pli];
    let dst_stride = ctx.plane_stride[pli];
    let dst_base = ctx.plane_buf[pli];
    let pw = BLOCK_SIZE_WIDE[plane_bsize as usize] as i32;
    let ph = BLOCK_SIZE_HIGH[plane_bsize as usize] as i32;
    let cdef_count = blk.cdef_count;
    let tmp_stride = 1 << MAX_SB_SIZE_LOG2;
    let off = |base: usize, r: i32, c: i32, stride: i32| -> Result<usize, ()> {
        usize::try_from(base as isize + (r * stride + c) as isize).map_err(|_| ())
    };

    // TODO(Ranjit): Extend this optimization for HBD
    if !ctx.use_highbitdepth {
        // If all 8x8/4x4 blocks in CDEF block need to be filtered, calculate the
        // error at CDEF block level
        let tot_blk_count = (pw * ph) >> (bw_log2 + bh_log2);
        if cdef_count == tot_blk_count {
            // Calculate the offset in the buffer based on block position
            let buf_offset = off(ref_base, row, col, ref_stride)?;
            if pri_strength == 0 && sec_strength == 0 {
                // When CDEF strength is zero, filtering is not applied. Hence
                // error is calculated between source and unfiltered pixels
                curr_sse = aom_sse(
                    reference,
                    buf_offset,
                    ref_stride,
                    frame,
                    off(dst_base, row, col, dst_stride)?,
                    dst_stride,
                    pw,
                    ph,
                )? as u64;
            } else {
                cdef_filter_fb_search(
                    Some((&mut *tmp, tmp_stride)),
                    None,
                    input,
                    in_off,
                    ssx,
                    ssy,
                    blk,
                    pli,
                    pri_strength,
                    sec_strength + (sec_strength == 3) as i32,
                    ctx.damping,
                    coeff_shift,
                )?;
                curr_sse = aom_sse(
                    reference, buf_offset, ref_stride, tmp, 0, tmp_stride, pw, ph,
                )? as u64;
            }
        } else {
            // If few 8x8/4x4 blocks in CDEF block need to be filtered, filtering
            // functions produce 8-bit output and the error is calculated in 8-bit
            // domain
            if pri_strength == 0 && sec_strength == 0 {
                let mut bi = 0usize;
                while bi < cdef_count as usize {
                    let by = blk.dlist[bi].by as i32;
                    let bx = blk.dlist[bi].bx as i32;
                    let by_pos = (by << bh_log2) as i16 as i32;
                    let bx_pos = (bx << bw_log2) as i16 as i32;
                    // Calculate the offset in the buffer based on block position
                    let buf_offset = off(ref_base, row + by_pos, col + bx_pos, ref_stride)?;
                    let num_error_calc_filt_units = get_error_calc_width_in_filt_units(
                        &blk.dlist,
                        cdef_count as usize,
                        bi,
                        ssx,
                        ssy,
                    );
                    curr_sse += aom_sse(
                        reference,
                        buf_offset,
                        ref_stride,
                        frame,
                        off(dst_base, row + by_pos, col + bx_pos, dst_stride)?,
                        dst_stride,
                        num_error_calc_filt_units * (1 << bw_log2),
                        1 << bh_log2,
                    )? as u64;
                    bi += num_error_calc_filt_units as usize;
                }
            } else {
                cdef_filter_fb_search(
                    Some((&mut *tmp, tmp_stride)),
                    None,
                    input,
                    in_off,
                    ssx,
                    ssy,
                    blk,
                    pli,
                    pri_strength,
                    sec_strength + (sec_strength == 3) as i32,
                    ctx.damping,
                    coeff_shift,
                )?;
                let mut bi = 0usize;
                while bi < cdef_count as usize {
                    let by = blk.dlist[bi].by as i32;
                    let bx = blk.dlist[bi].bx as i32;
                    let by_pos = (by << bh_log2) as i16 as i32;
                    let bx_pos = (bx << bw_log2) as i16 as i32;
                    // Calculate the offset in the buffer based on block position
                    let buf_offset = off(ref_base, row + by_pos, col + bx_pos, ref_stride)?;
                    let tmp_buf_offset = off(0, by_pos, bx_pos, tmp_stride)?;
                    let num_error_calc_filt_units = get_error_calc_width_in_filt_units(
                        &blk.dlist,
                        cdef_count as usize,
                        bi,
                        ssx,
                        ssy,
                    );
                    curr_sse += aom_sse(
                        reference,
                        buf_offset,
                        ref_stride,
                        tmp,
                        tmp_buf_offset,
                        tmp_stride,
                        num_error_calc_filt_units * (1 << bw_log2),
                        1 << bh_log2,
                    )? as u64;
                    bi += num_error_calc_filt_units as usize;
                }
            }
        }
    } else {
        cdef_filter_fb_search(
            None,
            Some(&mut *tmp),
            input,
            in_off,
            ssx,
            ssy,
            blk,
            pli,
            pri_strength,
            sec_strength + (sec_strength == 3) as i32,
            ctx.damping,
            coeff_shift,
        )?;
        curr_sse = compute_cdef_dist_highbd(
            reference,
            ref_base,
            ref_stride,
            tmp,
            &blk.dlist,
            cdef_count,
            ctx.bsize[pli],
            coeff_shift,
            row,
            col,
        )?;
    }
    Ok(curr_sse)
}

/// Translation of `sb_all_skip()`.
fn sb_all_skip(mi_params: &CommonModeInfoParams, mi_row: i32, mi_col: i32) -> Result<bool, ()> {
    let maxr = (mi_params.mi_rows - mi_row).min(MI_SIZE_64X64);
    let maxc = (mi_params.mi_cols - mi_col).min(MI_SIZE_64X64);
    let stride = mi_params.mi_stride;
    for r in 0..maxr {
        for c in 0..maxc {
            let m = grid_mbmi(
                mi_params,
                &mi_params.mi_alloc,
                (mi_row + r) * stride + mi_col + c,
            )?
            .ok_or(())?;
            if m.skip_txfm == 0 {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

// Checks if cdef processing can be skipped for particular sb.
/// Translation of `cdef_sb_skip()`.
fn cdef_sb_skip(mi_params: &CommonModeInfoParams, fbr: i32, fbc: i32) -> Result<bool, ()> {
    let mbmi = grid_mbmi(
        mi_params,
        &mi_params.mi_alloc,
        MI_SIZE_64X64 * fbr * mi_params.mi_stride + MI_SIZE_64X64 * fbc,
    )?
    .ok_or(())?;
    // No filtering if the entire filter block is skipped.
    if sb_all_skip(mi_params, fbr * MI_SIZE_64X64, fbc * MI_SIZE_64X64)? {
        return Ok(true);
    }
    // Skip odd numbered 64x64 block rows(cols) when bsize is BLOCK_128X128,
    // BLOCK_64X128(BLOCK_128X128, BLOCK_128X64) as for such blocks CDEF filtering
    // is done at the corresponding block sizes.
    if ((fbc & 1) != 0 && (mbmi.bsize == BLOCK_128X128 || mbmi.bsize == BLOCK_128X64))
        || ((fbr & 1) != 0 && (mbmi.bsize == BLOCK_128X128 || mbmi.bsize == BLOCK_64X128))
    {
        return Ok(true);
    }
    Ok(false)
}

// Calculates MSE at block level.
/// Translation of `av1_cdef_mse_calc_block()`.
fn av1_cdef_mse_calc_block(
    ctx: &mut CdefSearchCtx,
    mi_params: &CommonModeInfoParams,
    frame: &[u16],
    reference: &[u16],
    fbr: i32,
    fbc: i32,
    sb_count: usize,
    inbuf: &mut [u16],
    tmp: &mut [u16],
) -> Result<(), ()> {
    let coeff_shift = ctx.coeff_shift;

    // Declare and initialize the temporary buffers.
    let mut blk = CdefSearchBlock {
        dlist: vec![CdefList::default(); (MI_SIZE_128X128 * MI_SIZE_128X128) as usize],
        dir: [[0; CDEF_NBLOCKS]; CDEF_NBLOCKS],
        var: [[0; CDEF_NBLOCKS]; CDEF_NBLOCKS],
        dirinit: false,
        cdef_count: 0,
    };
    let in_off = (CDEF_VBORDER * CDEF_BSTRIDE + CDEF_HBORDER) as usize;
    let mut nhb = MI_SIZE_64X64.min(mi_params.mi_cols - MI_SIZE_64X64 * fbc);
    let mut nvb = MI_SIZE_64X64.min(mi_params.mi_rows - MI_SIZE_64X64 * fbr);
    let mut hb_step = 1;
    let mut vb_step = 1;
    let bs;

    let mbmi = grid_mbmi(
        mi_params,
        &mi_params.mi_alloc,
        MI_SIZE_64X64 * fbr * mi_params.mi_stride + MI_SIZE_64X64 * fbc,
    )?
    .ok_or(())?;

    if mbmi.bsize == BLOCK_128X128 || mbmi.bsize == BLOCK_128X64 || mbmi.bsize == BLOCK_64X128 {
        bs = mbmi.bsize;
        if bs == BLOCK_128X128 || bs == BLOCK_128X64 {
            nhb = MI_SIZE_128X128.min(mi_params.mi_cols - MI_SIZE_64X64 * fbc);
            hb_step = 2;
        }
        if bs == BLOCK_128X128 || bs == BLOCK_64X128 {
            nvb = MI_SIZE_128X128.min(mi_params.mi_rows - MI_SIZE_64X64 * fbr);
            vb_step = 2;
        }
    } else {
        bs = BLOCK_64X64;
    }
    // Get number of 8x8 blocks which are not skip. Cdef processing happens for
    // 8x8 blocks which are not skip.
    blk.cdef_count = av1_cdef_compute_sb_list(
        mi_params,
        &mi_params.mi_alloc,
        fbr * MI_SIZE_64X64,
        fbc * MI_SIZE_64X64,
        &mut blk.dlist,
        bs,
    )?;
    let is_fb_on_frm_left_boundary = fbc == 0;
    let is_fb_on_frm_right_boundary = fbc + hb_step == ctx.nhfb;
    let is_fb_on_frm_top_boundary = fbr == 0;
    let is_fb_on_frm_bottom_boundary = fbr + vb_step == ctx.nvfb;
    let yoff = CDEF_VBORDER * (!is_fb_on_frm_top_boundary) as i32;
    let xoff = CDEF_HBORDER * (!is_fb_on_frm_left_boundary) as i32;
    for pli in 0..ctx.num_planes {
        /* We avoid filtering the pixels for which some of the pixels to
        average are outside the frame. We could change the filter instead,
        but it would add special cases for any future vectorization. */
        let hfilt_size = nhb << ctx.mi_wide_l2[pli];
        let vfilt_size = nvb << ctx.mi_high_l2[pli];
        let ysize = vfilt_size + CDEF_VBORDER * (!is_fb_on_frm_bottom_boundary) as i32 + yoff;
        let xsize = hfilt_size + CDEF_HBORDER * (!is_fb_on_frm_right_boundary) as i32 + xoff;
        let row = (fbr * MI_SIZE_64X64) << ctx.mi_high_l2[pli];
        let col = (fbc * MI_SIZE_64X64) << ctx.mi_wide_l2[pli];
        av1_cdef_copy_sb8_16(
            inbuf,
            usize::try_from(in_off as i32 - yoff * CDEF_BSTRIDE - xoff).map_err(|_| ())?,
            CDEF_BSTRIDE,
            frame,
            ctx.plane_buf[pli],
            row - yoff,
            col - xoff,
            ctx.plane_stride[pli],
            ysize,
            xsize,
        )?;
        fill_borders_for_fbs_on_frame_boundary(
            inbuf,
            hfilt_size,
            vfilt_size,
            is_fb_on_frm_left_boundary,
            is_fb_on_frm_right_boundary,
            is_fb_on_frm_top_boundary,
            is_fb_on_frm_bottom_boundary,
        )?;
        for gi in 0..ctx.total_strengths {
            let (pri_strength, sec_strength) =
                get_cdef_filter_strengths(ctx.pick_method, gi as i32)?;
            let curr_mse = get_filt_error(
                ctx,
                frame,
                reference,
                &mut blk,
                inbuf,
                in_off,
                row,
                col,
                pri_strength,
                sec_strength,
                pli,
                coeff_shift,
                bs,
                tmp,
            )?;
            if pli < 2 {
                ctx.mse[pli].get_mut(sb_count).ok_or(())?[gi] = curr_mse;
            } else {
                let m = &mut ctx.mse[1].get_mut(sb_count).ok_or(())?[gi];
                *m = m.wrapping_add(curr_mse);
            }
        }
    }
    *ctx.sb_index.get_mut(sb_count).ok_or(())? =
        MI_SIZE_64X64 * fbr * mi_params.mi_stride + MI_SIZE_64X64 * fbc;
    Ok(())
}

// MSE calculation at frame level.
/// Translation of `cdef_mse_calc_frame()` (with several workers,
/// `av1_cdef_mse_calc_frame_mt()` hands out the same filter blocks in the
/// same raster order, each with the next `sb_count`, and computes each
/// block's errors from the frame alone: the results are the same).
fn cdef_mse_calc_frame(
    ctx: &mut CdefSearchCtx,
    mi_params: &CommonModeInfoParams,
    frame: &[u16],
    reference: &[u16],
) -> Result<(), ()> {
    let mut inbuf = try_vec(CDEF_INBUF_SIZE as usize, 0u16)?;
    let mut tmp = try_vec(1usize << (MAX_SB_SIZE_LOG2 * 2), 0u16)?;
    // Loop over each sb.
    for fbr in 0..ctx.nvfb {
        for fbc in 0..ctx.nhfb {
            // Checks if cdef processing can be skipped for particular sb.
            if cdef_sb_skip(mi_params, fbr, fbc)? {
                continue;
            }
            // Calculate mse for each sb and store the relevant sb index.
            let sb_count = ctx.sb_count;
            av1_cdef_mse_calc_block(
                ctx, mi_params, frame, reference, fbr, fbc, sb_count, &mut inbuf, &mut tmp,
            )?;
            ctx.sb_count += 1;
        }
    }
    Ok(())
}

/// Translation of `cdef_params_init()` with `cdef_alloc_data()`.
fn cdef_params_init(
    frame: &Yv12BufferConfig,
    reference: &Yv12BufferConfig,
    cm: &Av1Common,
    pick_method: i32,
) -> Result<CdefSearchCtx, ()> {
    let mi_params = &cm.mi_params;
    let num_planes = av1_num_planes(cm);
    let nvfb = (mi_params.mi_rows + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    let nhfb = (mi_params.mi_cols + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    let mut ctx = CdefSearchCtx {
        plane_buf: frame.buffers,
        plane_stride: [frame.strides[0], frame.strides[1], frame.strides[1]],
        ref_buf: reference.buffers,
        ref_stride: [
            reference.strides[0],
            reference.strides[1],
            reference.strides[1],
        ],
        total_strengths: nb_cdef_strengths(pick_method)?,
        coeff_shift: (cm.seq_params.bit_depth - 8).max(0),
        damping: 3 + (cm.quant_params.base_qindex >> 6),
        pick_method,
        num_planes,
        mi_wide_l2: [0; MAX_MB_PLANE],
        mi_high_l2: [0; MAX_MB_PLANE],
        xdec: [0; MAX_MB_PLANE],
        ydec: [0; MAX_MB_PLANE],
        bsize: [BLOCK_8X8; MAX_MB_PLANE],
        nvfb,
        nhfb,
        // (cdef_alloc_data())
        mse: [
            try_vec((nvfb * nhfb) as usize, [0u64; TOTAL_STRENGTHS])?,
            try_vec((nvfb * nhfb) as usize, [0u64; TOTAL_STRENGTHS])?,
        ],
        sb_index: try_vec((nvfb * nhfb) as usize, 0i32)?,
        sb_count: 0,
        use_highbitdepth: cm.seq_params.use_highbitdepth != 0,
    };
    // Initialize plane wise information.
    // (av1_setup_dst_planes(): the planes' subsampling)
    for pli in 0..num_planes {
        let (ssx, ssy) = if pli == 0 {
            (0, 0)
        } else {
            (cm.seq_params.subsampling_x, cm.seq_params.subsampling_y)
        };
        ctx.xdec[pli] = ssx;
        ctx.ydec[pli] = ssy;
        ctx.bsize[pli] = if ssy != 0 {
            if ssx != 0 {
                BLOCK_4X4
            } else {
                BLOCK_8X4
            }
        } else if ssx != 0 {
            BLOCK_4X8
        } else {
            BLOCK_8X8
        };
        ctx.mi_wide_l2[pli] = MI_SIZE_LOG2 - ssx;
        ctx.mi_high_l2[pli] = MI_SIZE_LOG2 - ssy;
    }
    Ok(ctx)
}
