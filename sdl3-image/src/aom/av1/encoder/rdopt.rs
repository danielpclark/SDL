// Rust translation of the parts of av1/encoder/rdopt.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The RD intra mode decision of a block of an intra frame, and the
//! transform domain distortions. (`av1_block_error_c()` is with the non-RD
//! pick mode's functions, see nonrd_pickmode.rs.)

use super::block::*;
use super::context_tree::PickModeContext;
use super::encoder::Av1Comp;
use super::intra_mode_search::{av1_rd_pick_intra_sbuv_mode, av1_rd_pick_intra_sby_mode};
use super::nonrd_pickmode::av1_copy_mbmi_ext_to_mbmi_ext_frame;
use super::rd::rdcost;
use super::rdopt_utils::{set_mode_eval_params, store_cfl_required_rdo};
use crate::aom::aom_dsp::aom_dsp_common::TranLowT;
use crate::aom::av1::common::av1_common_int::av1_num_planes;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mvref_common::av1_ref_frame_type;
use crate::aom::av1::common::pred_common::av1_get_skip_txfm_context;
use crate::aom::av1::common::reconintra::av1_allow_intrabc;

/// Translation of `av1_block_error_lp_c()`.
pub(crate) fn av1_block_error_lp(coeff: &[i16], dqcoeff: &[i16], block_size: usize) -> i64 {
    let mut error: i64 = 0;

    for i in 0..block_size {
        let diff = coeff[i] as i32 - dqcoeff[i] as i32;
        error += (diff * diff) as i64;
    }

    error
}

/// Translation of `av1_highbd_block_error_c()`.
pub(crate) fn av1_highbd_block_error(
    coeff: &[TranLowT],
    dqcoeff: &[TranLowT],
    block_size: usize,
    ssz: &mut i64,
    bd: i32,
) -> i64 {
    let mut error: i64 = 0;
    let mut sqcoeff: i64 = 0;
    let shift = 2 * (bd - 8);
    let rounding: i64 = if shift > 0 { 1 << (shift - 1) } else { 0 };

    for i in 0..block_size {
        let diff = coeff[i] as i64 - dqcoeff[i] as i64;
        error += diff * diff;
        sqcoeff += coeff[i] as i64 * coeff[i] as i64;
    }
    debug_assert!(error >= 0 && sqcoeff >= 0);
    error = (error + rounding) >> shift;
    sqcoeff = (sqcoeff + rounding) >> shift;

    *ssz = sqcoeff;
    error
}

/*\brief Search for the best intrabc predictor
 *
 * Returns the best overall rdcost (including the non-intrabc modes
 * search before this function).
 */
fn rd_pick_intrabc_mode_sb(cpi: &Av1Comp) -> Result<i64, ()> {
    let cm = &cpi.common;
    if !av1_allow_intrabc(cm)
        || !cpi.oxcf.kf_cfg.enable_intrabc
        || cpi.sf.rt_sf.use_nonrd_pick_mode != 0
    {
        return Ok(i64::MAX);
    }
    // (intra block copy needs screen content tools, which are off)
    Err(())
}

// TODO(chiyotsai@google.com): We are using struct $struct_name instead of their
// typedef here because Doxygen doesn't know about the typedefs yet. So using
// the typedef will prevent doxygen from finding this function and generating
// the callgraph. Once documents for AV1_COMP and MACROBLOCK are added to
// doxygen, we can revert back to using the typedefs.
/// Translation of `av1_rd_pick_intra_mode_sb()`.
pub(crate) fn av1_rd_pick_intra_mode_sb(
    cpi: &Av1Comp,
    x: &mut Macroblock,
    rd_cost: &mut RdStats,
    bsize: BlockSize,
    ctx: &mut PickModeContext,
    mut best_rd: i64,
) -> Result<(), ()> {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);
    let mut rate_y = 0;
    let mut rate_uv = 0;
    let mut rate_y_tokenonly = 0;
    let mut rate_uv_tokenonly = 0;
    let mut y_skip_txfm: u8 = 0;
    let mut uv_skip_txfm: u8 = 0;
    let mut dist_y: i64 = 0;
    let mut dist_uv: i64 = 0;

    ctx.rd_stats.skip_txfm = 0;
    {
        let mbmi = x.e_mbd.mbmi_mut();
        mbmi.ref_frame[0] = INTRA_FRAME;
        mbmi.ref_frame[1] = NONE_FRAME;
        mbmi.use_intrabc = 0;
        mbmi.mv[0].as_int = 0;
        mbmi.skip_mode = 0;
    }

    let intra_yrd = av1_rd_pick_intra_sby_mode(
        cpi,
        x,
        &mut rate_y,
        &mut rate_y_tokenonly,
        &mut dist_y,
        &mut y_skip_txfm,
        bsize,
        best_rd,
        ctx,
    )?;

    // Initialize default mode evaluation params
    set_mode_eval_params(cpi, x, DEFAULT_EVAL);

    if intra_yrd < best_rd {
        // Search intra modes for uv planes if needed
        if num_planes > 1 {
            // Set up the tx variables for reproducing the y predictions in case we
            // need it for chroma-from-luma.
            if x.e_mbd.is_chroma_ref && store_cfl_required_rdo(cm, x) != 0 {
                let n = ctx.num_4x4_blk as usize;
                x.txfm_search_info.blk_skip[..n].copy_from_slice(&ctx.blk_skip[..n]);
                for (i, &t) in ctx.tx_type_map[..n].iter().enumerate() {
                    x.e_mbd.tx_type_map_set(i, t);
                }
            }
            let max_uv_tx_size = av1_get_tx_size(AOM_PLANE_U, &x.e_mbd);
            av1_rd_pick_intra_sbuv_mode(
                cpi,
                x,
                &mut rate_uv,
                &mut rate_uv_tokenonly,
                &mut dist_uv,
                &mut uv_skip_txfm,
                bsize,
                max_uv_tx_size,
            )?;
        }

        // Intra block is always coded as non-skip
        rd_cost.rate = rate_y
            + rate_uv
            + x.mode_costs.skip_txfm_cost[av1_get_skip_txfm_context(&x.e_mbd) as usize][0];
        rd_cost.dist = dist_y + dist_uv;
        rd_cost.rdcost = rdcost(x.rdmult, rd_cost.rate, rd_cost.dist);
        rd_cost.skip_txfm = 0;
    } else {
        rd_cost.rate = i32::MAX;
    }

    if rd_cost.rate != i32::MAX && rd_cost.rdcost < best_rd {
        best_rd = rd_cost.rdcost;
    }
    if rd_pick_intrabc_mode_sb(cpi)? < best_rd {
        // (see rd_pick_intrabc_mode_sb())
        return Err(());
    }
    if rd_cost.rate == i32::MAX {
        return Ok(());
    }

    ctx.mic = *x.e_mbd.mbmi();
    let ref_frame_type = av1_ref_frame_type(&x.e_mbd.mbmi().ref_frame) as u8;
    av1_copy_mbmi_ext_to_mbmi_ext_frame(&mut ctx.mbmi_ext_best, &x.mbmi_ext, ref_frame_type);
    let n = ctx.num_4x4_blk as usize;
    for (i, t) in ctx.tx_type_map[..n].iter_mut().enumerate() {
        *t = x.e_mbd.tx_type_map_get(i);
    }
    Ok(())
}
