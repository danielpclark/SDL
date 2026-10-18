// Rust translation of the parts of av1/encoder/reconinter_enc.c and
// av1/common/reconinter_template.inc from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the real-time encoder
// reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encoder's inter predictions: the reference planes are those of
//! `pd->pre[]` (frames of the buffer pool), the destination is
//! `pd->dst`, which points into the reconstructed frame (`xd.recon`).

use crate::aom::av1::common::av1_common_int::Av1Common;
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::convolve::get_conv_params_no_round;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::mv::Mv;
use crate::aom::av1::common::reconinter::*;
use crate::aom::av1::common::scale::SCALE_SUBPEL_BITS;

/// Translation of `enc_calc_subpel_params()`: returns the offset of the
/// source and its stride.
#[inline]
fn enc_calc_subpel_params(
    src_mv: &Mv,
    inter_pred_params: &InterPredParams,
    subpel_params: &mut SubpelParams,
) -> (usize, usize) {
    let pre_buf = &inter_pred_params.ref_frame_buf;
    init_subpel_params(
        src_mv,
        inter_pred_params,
        subpel_params,
        pre_buf.width,
        pre_buf.height,
    );
    let pre = pre_buf.buf0 as i64
        + (subpel_params.pos_y >> SCALE_SUBPEL_BITS) as i64 * pre_buf.stride as i64
        + (subpel_params.pos_x >> SCALE_SUBPEL_BITS) as i64;
    (pre as usize, pre_buf.stride as usize)
}

/// Translation of `build_one_inter_predictor()` (`IS_DEC` 0): the
/// destination is `xd.recon` at `dst_off`.
#[inline]
fn build_one_inter_predictor(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    dst_off: usize,
    dst_stride: usize,
    src_mv: &Mv,
    inter_pred_params: &InterPredParams,
) -> Result<(), ()> {
    let mut subpel_params = SubpelParams::default();
    let (src, src_stride) = enc_calc_subpel_params(src_mv, inter_pred_params, &mut subpel_params);
    if inter_pred_params.comp_mode == UNIFORM_SINGLE || inter_pred_params.comp_mode == UNIFORM_COMP
    {
        // (xd->tmp_conv_dst: td->tmp_conv_dst, MAX_SB_SIZE * MAX_SB_SIZE
        // samples, allocated on first use)
        if xd.tmp_conv_dst.len() < MAX_SB_SQUARE as usize {
            xd.tmp_conv_dst =
                crate::aom::av1::common::alloccommon::try_vec(MAX_SB_SQUARE as usize, 0u16)?;
        }
        let ref_buf = &cm.buffer_pool.frame_bufs[inter_pred_params.ref_frame_buf.frame_buf]
            .buf
            .buffer_alloc;
        let MacroBlockD {
            recon,
            tmp_conv_dst,
            ..
        } = xd;
        av1_make_inter_predictor(
            ref_buf,
            src,
            src_stride,
            &mut recon.buffer_alloc,
            dst_off,
            dst_stride,
            inter_pred_params,
            &subpel_params,
            tmp_conv_dst,
        )
    } else {
        // (av1_make_masked_inter_predictor(): the masked compound
        // predictions are never picked)
        Err(())
    }
}

// True if the following hold:
//  1. Not intrabc and not build_for_obmc
//  2. At least one dimension is size 4 with subsampling
//  3. If sub-sampled, none of the previous blocks around the sub-sample
//     are intrabc or inter-blocks
fn is_sub8x8_inter(
    xd: &MacroBlockD,
    plane: usize,
    bsize: BlockSize,
    is_intrabc: bool,
    build_for_obmc: bool,
) -> bool {
    if is_intrabc || build_for_obmc {
        return false;
    }

    let pd = &xd.plane[plane];
    let ss_x = pd.subsampling_x;
    let ss_y = pd.subsampling_y;
    let is_sub4_x = (BLOCK_SIZE_WIDE[bsize as usize] == 4) && ss_x != 0;
    let is_sub4_y = (BLOCK_SIZE_HIGH[bsize as usize] == 4) && ss_y != 0;
    if !is_sub4_x && !is_sub4_y {
        return false;
    }

    // For sub8x8 chroma blocks, we may be covering more than one luma block's
    // worth of pixels. Thus (mi_x, mi_y) may not be the correct coordinates for
    // the top-left corner of the prediction source - the correct top-left corner
    // is at (pre_x, pre_y).
    let row_start = if is_sub4_y { -1 } else { 0 };
    let col_start = if is_sub4_x { -1 } else { 0 };

    for row in row_start..=0 {
        for col in col_start..=0 {
            let this_mbmi = &xd.mi_alloc[xd.mi_idx((row * xd.mi_stride + col) as isize) as usize];
            if !is_inter_block(this_mbmi) {
                return false;
            }
            if is_intrabc_block(this_mbmi) {
                return false;
            }
        }
    }
    true
}

fn build_inter_predictors_8x8_and_bigger(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    plane: usize,
    mi: &MbModeInfo,
    build_for_obmc: bool,
    bw: i32,
    bh: i32,
    mi_x: i32,
    mi_y: i32,
) -> Result<(), ()> {
    let is_compound = has_second_ref(mi);
    let is_intrabc = is_intrabc_block(mi);
    debug_assert!(!is_intrabc || !is_compound);
    let dst_buf = xd.plane[plane].dst;
    let dst = dst_buf.buf;

    let mut is_global = [false; 2];
    for (r, g) in is_global
        .iter_mut()
        .enumerate()
        .take(1 + is_compound as usize)
    {
        let wm = &cm.global_motion[mi.ref_frame[r] as usize];
        *g = is_global_mv_block(mi, wm.wmtype);
    }

    let bsize = mi.bsize;
    let ss_x = xd.plane[plane].subsampling_x;
    let ss_y = xd.plane[plane].subsampling_y;
    let row_start = if BLOCK_SIZE_HIGH[bsize as usize] == 4 && ss_y != 0 && !build_for_obmc {
        -1
    } else {
        0
    };
    let col_start = if BLOCK_SIZE_WIDE[bsize as usize] == 4 && ss_x != 0 && !build_for_obmc {
        -1
    } else {
        0
    };
    let pre_x = (mi_x + MI_SIZE * col_start) >> ss_x;
    let pre_y = (mi_y + MI_SIZE * row_start) >> ss_y;

    for r in 0..1 + is_compound as usize {
        if is_intrabc {
            // (intrabc is only allowed with screen content tools, which are
            // off)
            return Err(());
        }
        let sf = xd.block_ref_scale_factors[r];
        let pre_buf = xd.plane[plane].pre[r];
        let mv = mi.mv[r].as_mv();
        let warp_types = WarpTypesAllowed {
            global_warp_allowed: is_global[r],
            local_warp_allowed: mi.motion_mode == WARPED_CAUSAL,
        };

        let mut inter_pred_params = InterPredParams::default();
        av1_init_inter_params(
            &mut inter_pred_params,
            bw,
            bh,
            pre_y,
            pre_x,
            ss_x,
            ss_y,
            xd.bd,
            is_cur_buf_hbd(xd),
            mi.use_intrabc != 0,
            &sf,
            &pre_buf,
            mi.interp_filters,
        );
        if is_compound {
            av1_init_comp_mode(&mut inter_pred_params);
        }
        inter_pred_params.conv_params = get_conv_params_no_round(
            r as i32,
            plane as i32,
            MAX_SB_SIZE,
            is_compound as i32,
            xd.bd,
        );

        {
            let cp = &mut inter_pred_params.conv_params;
            av1_dist_wtd_comp_weight_assign(
                cm,
                mi,
                &mut cp.fwd_offset,
                &mut cp.bck_offset,
                &mut cp.use_dist_wtd_comp_avg,
                is_compound,
            );
        }

        if !build_for_obmc {
            av1_init_warp_params(
                &mut inter_pred_params,
                &warp_types,
                r,
                xd,
                &cm.global_motion,
                mi,
            );
        }

        if is_masked_compound_type(mi.interinter_comp.type_ as i32) {
            // (the masked compound predictions are never picked)
            return Err(());
        }

        build_one_inter_predictor(
            cm,
            xd,
            dst,
            dst_buf.stride as usize,
            &mv,
            &inter_pred_params,
        )?;
    }
    Ok(())
}

/// Translation of `build_inter_predictors()` (`IS_DEC` 0). `Err` for the
/// sub8x8 chroma predictions (4xN blocks are never coded by the real-time
/// partitioning).
fn build_inter_predictors(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    plane: usize,
    mi: &MbModeInfo,
    build_for_obmc: bool,
    bw: i32,
    bh: i32,
    mi_x: i32,
    mi_y: i32,
) -> Result<(), ()> {
    if is_sub8x8_inter(xd, plane, mi.bsize, is_intrabc_block(mi), build_for_obmc) {
        debug_assert!(bw < 8 || bh < 8);
        // (build_inter_predictors_sub8x8())
        Err(())
    } else {
        build_inter_predictors_8x8_and_bigger(cm, xd, plane, mi, build_for_obmc, bw, bh, mi_x, mi_y)
    }
}

fn enc_build_inter_predictors(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    plane: usize,
    mi: &MbModeInfo,
    bw: i32,
    bh: i32,
    mi_x: i32,
    mi_y: i32,
) -> Result<(), ()> {
    build_inter_predictors(
        cm, xd, plane, mi, /*build_for_obmc=*/ false, bw, bh, mi_x, mi_y,
    )
}

/// Translation of `av1_enc_build_inter_predictor_y_nonrd()`.
pub(crate) fn av1_enc_build_inter_predictor_y_nonrd(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    inter_pred_params: &mut InterPredParams,
    subpel_params: &SubpelParams,
) -> Result<(), ()> {
    let pd = &xd.plane[AOM_PLANE_Y];

    let mbmi = xd.mbmi();
    let dst_buf = pd.dst;
    let pre_buf = pd.pre[0];
    let src = (pre_buf.buf0 as i64
        + (subpel_params.pos_y >> SCALE_SUBPEL_BITS) as i64 * pre_buf.stride as i64
        + (subpel_params.pos_x >> SCALE_SUBPEL_BITS) as i64) as usize;
    let dst = dst_buf.buf;
    let src_stride = pre_buf.stride as usize;
    let dst_stride = dst_buf.stride as usize;
    inter_pred_params.ref_frame_buf = pre_buf;

    // Initialize interp filter for single reference mode.
    init_interp_filter_params(
        &mut inter_pred_params.interp_filter_params,
        mbmi.interp_filters,
        pd.width as i32,
        pd.height as i32,
        /*is_intrabc=*/ false,
    );

    let ref_buf = &cm.buffer_pool.frame_bufs[pre_buf.frame_buf]
        .buf
        .buffer_alloc;
    let MacroBlockD {
        recon,
        tmp_conv_dst,
        ..
    } = xd;
    av1_make_inter_predictor(
        ref_buf,
        src,
        src_stride,
        &mut recon.buffer_alloc,
        dst,
        dst_stride,
        inter_pred_params,
        subpel_params,
        tmp_conv_dst,
    )
}

/// Translation of `av1_enc_build_inter_predictor()`: `ctx` (the buffers of
/// the inter-intra prediction) is always NULL in the real-time encoder.
pub(crate) fn av1_enc_build_inter_predictor(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    _ctx: Option<()>,
    _bsize: BlockSize,
    plane_from: usize,
    plane_to: usize,
) -> Result<(), ()> {
    for plane in plane_from..=plane_to {
        if plane != 0 && !xd.is_chroma_ref {
            break;
        }
        let mi_x = mi_col * MI_SIZE;
        let mi_y = mi_row * MI_SIZE;
        let mi = *xd.mbmi();
        let (w, h) = (xd.plane[plane].width as i32, xd.plane[plane].height as i32);
        enc_build_inter_predictors(cm, xd, plane, &mi, w, h, mi_x, mi_y)?;

        if is_interintra_pred(&mi) {
            // (av1_build_interintra_predictor(): inter-intra is never picked)
            return Err(());
        }
    }
    Ok(())
}
