// Rust translation of the parts of av1/common/reconinter.c,
// av1/common/reconinter.h and av1/common/obmc.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Plane buffer setup and the translational inter predictions (single and
//! compound average). The warped and masked predictions are not
//! translated: the real-time encoder never picks them.

use super::av1_common_int::{get_ref_frame_buf, Av1Common};
use super::blockd::*;
use super::convolve::{av1_convolve_2d_facade, ConvolveParams};
use super::enums::*;
use super::filter::{
    av1_extract_interp_filter, av1_get_interp_filter_params_with_block_size, InterpFilterParams,
    AV1_INTRABC_FILTER_PARAMS, SUBPEL_BITS, SUBPEL_SHIFTS,
};
use super::mv::{Mv, WarpedMotionParams, DEFAULT_WARP_PARAMS, TRANSLATION};
use super::mvref_common::get_relative_dist;
use super::scale::*;
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;

/// `AOM_INTERP_EXTEND`
pub(crate) const AOM_INTERP_EXTEND: i32 = 4;
/// `AOM_BORDER_IN_PIXELS`
pub(crate) const AOM_BORDER_IN_PIXELS: i32 = 288;

// Work out how many pixels off the edge of a reference frame we're allowed
// to go when forming an inter prediction.
// The outermost row/col of each referernce frame is extended by
// (AOM_BORDER_IN_PIXELS >> subsampling) pixels, but we need to keep
// at least AOM_INTERP_EXTEND pixels within that to account for filtering.
//
// We have to break this up into two macros to keep both clang-format and
// tools/lint-hunks.py happy.
/// `AOM_LEFT_TOP_MARGIN_PX()`
#[inline]
const fn aom_left_top_margin_px(subsampling: i32) -> i32 {
    (AOM_BORDER_IN_PIXELS >> subsampling) - AOM_INTERP_EXTEND
}
/// `AOM_LEFT_TOP_MARGIN_SCALED()`
#[inline]
const fn aom_left_top_margin_scaled(subsampling: i32) -> i32 {
    aom_left_top_margin_px(subsampling) << SCALE_SUBPEL_BITS
}

/// `MAX_WEDGE_TYPES`
const MAX_WEDGE_TYPES: i32 = 16;

/// `SubpelParams`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SubpelParams {
    pub(crate) xs: i32,
    pub(crate) ys: i32,
    pub(crate) subpel_x: i32,
    pub(crate) subpel_y: i32,
    pub(crate) pos_x: i32,
    pub(crate) pos_y: i32,
}

/// `InterPredMode`
pub(crate) const TRANSLATION_PRED: i32 = 0;
pub(crate) const WARP_PRED: i32 = 1;

/// `InterCompMode`
pub(crate) const UNIFORM_SINGLE: i32 = 0;
pub(crate) const UNIFORM_COMP: i32 = 1;
pub(crate) const MASK_COMP: i32 = 2;

/// Translation of `InterPredParams` (`mask_comp` and `sb_type` are only set
/// for the masked compound predictions).
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterPredParams {
    pub(crate) mode: i32,
    pub(crate) comp_mode: i32,
    pub(crate) warp_params: WarpedMotionParams,
    pub(crate) conv_params: ConvolveParams,
    pub(crate) interp_filter_params: [&'static InterpFilterParams; 2],
    pub(crate) block_width: i32,
    pub(crate) block_height: i32,
    pub(crate) pix_row: i32,
    pub(crate) pix_col: i32,
    pub(crate) ref_frame_buf: Buf2d,
    pub(crate) subsampling_x: i32,
    pub(crate) subsampling_y: i32,
    pub(crate) scale_factors: ScaleFactors,
    pub(crate) bit_depth: i32,
    pub(crate) use_hbd_buf: bool,
    pub(crate) is_intrabc: bool,
    pub(crate) top: i32,
    pub(crate) left: i32,
}

impl Default for InterPredParams {
    fn default() -> Self {
        InterPredParams {
            mode: TRANSLATION_PRED,
            comp_mode: UNIFORM_SINGLE,
            warp_params: DEFAULT_WARP_PARAMS,
            conv_params: ConvolveParams::default(),
            interp_filter_params: [&AV1_INTRABC_FILTER_PARAMS; 2],
            block_width: 0,
            block_height: 0,
            pix_row: 0,
            pix_col: 0,
            ref_frame_buf: Buf2d::default(),
            subsampling_x: 0,
            subsampling_y: 0,
            scale_factors: ScaleFactors::default(),
            bit_depth: 0,
            use_hbd_buf: false,
            is_intrabc: false,
            top: 0,
            left: 0,
        }
    }
}

/// `WarpTypesAllowed`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WarpTypesAllowed {
    pub(crate) global_warp_allowed: bool,
    pub(crate) local_warp_allowed: bool,
}

#[inline]
pub(crate) fn init_subpel_params(
    src_mv: &Mv,
    inter_pred_params: &InterPredParams,
    subpel_params: &mut SubpelParams,
    width: i32,
    height: i32,
) {
    let sf = &inter_pred_params.scale_factors;
    let ssx = inter_pred_params.subsampling_x;
    let ssy = inter_pred_params.subsampling_y;
    let mut orig_pos_y = inter_pred_params.pix_row << SUBPEL_BITS;
    orig_pos_y += src_mv.row as i32 * (1 << (1 - ssy));
    let mut orig_pos_x = inter_pred_params.pix_col << SUBPEL_BITS;
    orig_pos_x += src_mv.col as i32 * (1 << (1 - ssx));
    let is_scaled = av1_is_scaled(sf);
    let (mut pos_x, mut pos_y);
    if !is_scaled {
        pos_y = av1_unscaled_value(orig_pos_y, sf);
        pos_x = av1_unscaled_value(orig_pos_x, sf);
    } else {
        pos_y = av1_scaled_y(orig_pos_y, sf);
        pos_x = av1_scaled_x(orig_pos_x, sf);
    }

    pos_x += SCALE_EXTRA_OFF;
    pos_y += SCALE_EXTRA_OFF;

    let bottom = (height + AOM_INTERP_EXTEND) << SCALE_SUBPEL_BITS;
    let right = (width + AOM_INTERP_EXTEND) << SCALE_SUBPEL_BITS;
    pos_y = pos_y.clamp(inter_pred_params.top, bottom);
    pos_x = pos_x.clamp(inter_pred_params.left, right);

    subpel_params.pos_x = pos_x;
    subpel_params.pos_y = pos_y;
    subpel_params.subpel_x = pos_x & SCALE_SUBPEL_MASK;
    subpel_params.subpel_y = pos_y & SCALE_SUBPEL_MASK;
    subpel_params.xs = sf.x_step_q4;
    subpel_params.ys = sf.y_step_q4;
}

// Initialize interp filter required for inter prediction.
#[inline]
pub(crate) fn init_interp_filter_params(
    interp_filter_params: &mut [&'static InterpFilterParams; 2],
    filters: u32,
    block_width: i32,
    block_height: i32,
    is_intrabc: bool,
) {
    if is_intrabc {
        interp_filter_params[0] = &AV1_INTRABC_FILTER_PARAMS;
        interp_filter_params[1] = &AV1_INTRABC_FILTER_PARAMS;
    } else {
        // (InterpFilters: y_filter in the low 16 bits, x_filter in the high)
        interp_filter_params[0] = av1_get_interp_filter_params_with_block_size(
            av1_extract_interp_filter(filters, 1),
            block_width,
        );
        interp_filter_params[1] = av1_get_interp_filter_params_with_block_size(
            av1_extract_interp_filter(filters, 0),
            block_height,
        );
    }
}

// Initialize parameters for inter prediction.
#[inline]
pub(crate) fn init_inter_mode_params(
    src_mv: &Mv,
    inter_pred_params: &mut InterPredParams,
    subpel_params: &mut SubpelParams,
    sf: &ScaleFactors,
    width: i32,
    height: i32,
) {
    inter_pred_params.scale_factors = *sf;
    init_subpel_params(src_mv, inter_pred_params, subpel_params, width, height);
}

// Initialize parameters for inter prediction.
#[inline]
pub(crate) fn init_inter_block_params(
    inter_pred_params: &mut InterPredParams,
    block_width: i32,
    block_height: i32,
    pix_row: i32,
    pix_col: i32,
    subsampling_x: i32,
    subsampling_y: i32,
    bit_depth: i32,
    use_hbd_buf: bool,
    is_intrabc: bool,
) {
    inter_pred_params.block_width = block_width;
    inter_pred_params.block_height = block_height;
    inter_pred_params.pix_row = pix_row;
    inter_pred_params.pix_col = pix_col;
    inter_pred_params.subsampling_x = subsampling_x;
    inter_pred_params.subsampling_y = subsampling_y;
    inter_pred_params.bit_depth = bit_depth;
    inter_pred_params.use_hbd_buf = use_hbd_buf;
    inter_pred_params.is_intrabc = is_intrabc;
    inter_pred_params.mode = TRANSLATION_PRED;
    inter_pred_params.comp_mode = UNIFORM_SINGLE;
    inter_pred_params.top = -aom_left_top_margin_scaled(subsampling_y);
    inter_pred_params.left = -aom_left_top_margin_scaled(subsampling_x);
}

// Initialize params required for inter prediction.
#[inline]
pub(crate) fn av1_init_inter_params(
    inter_pred_params: &mut InterPredParams,
    block_width: i32,
    block_height: i32,
    pix_row: i32,
    pix_col: i32,
    subsampling_x: i32,
    subsampling_y: i32,
    bit_depth: i32,
    use_hbd_buf: bool,
    is_intrabc: bool,
    sf: &ScaleFactors,
    ref_buf: &Buf2d,
    interp_filters: u32,
) {
    init_inter_block_params(
        inter_pred_params,
        block_width,
        block_height,
        pix_row,
        pix_col,
        subsampling_x,
        subsampling_y,
        bit_depth,
        use_hbd_buf,
        is_intrabc,
    );
    init_interp_filter_params(
        &mut inter_pred_params.interp_filter_params,
        interp_filters,
        block_width,
        block_height,
        is_intrabc,
    );
    inter_pred_params.scale_factors = *sf;
    inter_pred_params.ref_frame_buf = *ref_buf;
}

#[inline]
pub(crate) fn av1_init_comp_mode(inter_pred_params: &mut InterPredParams) {
    inter_pred_params.comp_mode = UNIFORM_COMP;
}

#[inline]
fn has_scale(xs: i32, ys: i32) -> bool {
    xs != SCALE_SUBPEL_SHIFTS || ys != SCALE_SUBPEL_SHIFTS
}

#[inline]
fn revert_scale_extra_bits(sp: &mut SubpelParams) {
    sp.subpel_x >>= SCALE_EXTRA_BITS;
    sp.subpel_y >>= SCALE_EXTRA_BITS;
    sp.xs >>= SCALE_EXTRA_BITS;
    sp.ys >>= SCALE_EXTRA_BITS;
    debug_assert!(sp.subpel_x < SUBPEL_SHIFTS as i32);
    debug_assert!(sp.subpel_y < SUBPEL_SHIFTS as i32);
    debug_assert!(sp.xs <= SUBPEL_SHIFTS as i32);
    debug_assert!(sp.ys <= SUBPEL_SHIFTS as i32);
}

/// Translation of `inter_predictor()` and `highbd_inter_predictor()` (the
/// samples are kept in 16 bits: see convolve.rs).
#[inline]
fn inter_predictor(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    subpel_params: &SubpelParams,
    w: i32,
    h: i32,
    conv_params: &ConvolveParams,
    interp_filters: [&InterpFilterParams; 2],
    conv_dst: &mut [u16],
    bd: i32,
) -> Result<(), ()> {
    debug_assert!(conv_params.do_average == 0 || conv_params.do_average == 1);
    let is_scaled = has_scale(subpel_params.xs, subpel_params.ys);
    if is_scaled {
        av1_convolve_2d_facade(
            src,
            src_off,
            src_stride,
            dst,
            dst_off,
            dst_stride,
            w,
            h,
            interp_filters,
            subpel_params.subpel_x,
            subpel_params.xs,
            subpel_params.subpel_y,
            subpel_params.ys,
            true,
            conv_params,
            conv_dst,
            bd,
        )
    } else {
        let mut sp = *subpel_params;
        revert_scale_extra_bits(&mut sp);
        av1_convolve_2d_facade(
            src,
            src_off,
            src_stride,
            dst,
            dst_off,
            dst_stride,
            w,
            h,
            interp_filters,
            sp.subpel_x,
            sp.xs,
            sp.subpel_y,
            sp.ys,
            false,
            conv_params,
            conv_dst,
            bd,
        )
    }
}

// This function will determine whether or not to create a warped
// prediction.
pub(crate) fn av1_allow_warp(
    mbmi: &MbModeInfo,
    warp_types: &WarpTypesAllowed,
    gm_params: &WarpedMotionParams,
    build_for_obmc: bool,
    sf: &ScaleFactors,
    final_warp_params: Option<&mut WarpedMotionParams>,
) -> bool {
    // Note: As per the spec, we must test the fixed point scales here, which are
    // at a higher precision (1 << 14) than the xs and ys in subpel_params (that
    // have 1 << 10 precision).
    if av1_is_scaled(sf) {
        return false;
    }

    let mut final_warp_params = final_warp_params;
    if let Some(f) = final_warp_params.as_deref_mut() {
        *f = DEFAULT_WARP_PARAMS;
    }

    if build_for_obmc {
        return false;
    }

    if warp_types.local_warp_allowed && mbmi.wm_params.invalid == 0 {
        if let Some(f) = final_warp_params {
            *f = mbmi.wm_params;
        }
        return true;
    } else if warp_types.global_warp_allowed && gm_params.invalid == 0 {
        if let Some(f) = final_warp_params {
            *f = *gm_params;
        }
        return true;
    }

    false
}

pub(crate) fn av1_init_warp_params(
    inter_pred_params: &mut InterPredParams,
    warp_types: &WarpTypesAllowed,
    ref_: usize,
    xd: &MacroBlockD,
    global_motion: &[WarpedMotionParams],
    mi: &MbModeInfo,
) {
    if inter_pred_params.block_height < 8 || inter_pred_params.block_width < 8 {
        return;
    }

    if xd.cur_frame_force_integer_mv != 0 {
        return;
    }

    let sf = inter_pred_params.scale_factors;
    if av1_allow_warp(
        mi,
        warp_types,
        &global_motion[mi.ref_frame[ref_] as usize],
        false,
        &sf,
        Some(&mut inter_pred_params.warp_params),
    ) {
        inter_pred_params.mode = WARP_PRED;
    }
}

/// Translation of `av1_make_inter_predictor()`: `conv_dst` is
/// `inter_pred_params->conv_params.dst`. `Err` for the warped prediction,
/// which the saver never makes.
pub(crate) fn av1_make_inter_predictor(
    src: &[u16],
    src_off: usize,
    src_stride: usize,
    dst: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    inter_pred_params: &InterPredParams,
    subpel_params: &SubpelParams,
    conv_dst: &mut [u16],
) -> Result<(), ()> {
    if inter_pred_params.mode == TRANSLATION_PRED {
        // (inter_predictor() and highbd_inter_predictor(): the same here,
        // with the bit depth of the samples)
        let bd = if inter_pred_params.use_hbd_buf {
            inter_pred_params.bit_depth
        } else {
            8
        };
        inter_predictor(
            src,
            src_off,
            src_stride,
            dst,
            dst_off,
            dst_stride,
            subpel_params,
            inter_pred_params.block_width,
            inter_pred_params.block_height,
            &inter_pred_params.conv_params,
            inter_pred_params.interp_filter_params,
            conv_dst,
            bd,
        )
    }
    // TODO(jingning): av1_warp_plane() can be further cleaned up.
    else {
        // (av1_warp_plane(): the warped motion of WARP_PRED is never picked)
        Err(())
    }
}

/// `quant_dist_weight[4][2]`
static QUANT_DIST_WEIGHT: [[i32; 2]; 4] = [[2, 3], [2, 5], [2, 7], [1, MAX_FRAME_DISTANCE]];
/// `quant_dist_lookup_table[4][2]`
static QUANT_DIST_LOOKUP_TABLE: [[i32; 2]; 4] = [[9, 7], [11, 5], [12, 4], [13, 3]];

/// `MAX_FRAME_DISTANCE`
const MAX_FRAME_DISTANCE: i32 = (1 << 5) - 1;

pub(crate) fn av1_dist_wtd_comp_weight_assign(
    cm: &Av1Common,
    mbmi: &MbModeInfo,
    fwd_offset: &mut i32,
    bck_offset: &mut i32,
    use_dist_wtd_comp_avg: &mut i32,
    is_compound: bool,
) {
    if !is_compound || mbmi.compound_idx != 0 {
        *fwd_offset = 8;
        *bck_offset = 8;
        *use_dist_wtd_comp_avg = 0;
        return;
    }

    *use_dist_wtd_comp_avg = 1;
    let bck_buf = get_ref_frame_buf(cm, mbmi.ref_frame[0]);
    let fwd_buf = get_ref_frame_buf(cm, mbmi.ref_frame[1]);
    let cur_frame_index = cm.cur_frame().order_hint as i32;
    let mut bck_frame_index = 0;
    let mut fwd_frame_index = 0;

    if let Some(b) = bck_buf {
        bck_frame_index = cm.buffer_pool.frame_bufs[b].order_hint as i32;
    }
    if let Some(f) = fwd_buf {
        fwd_frame_index = cm.buffer_pool.frame_bufs[f].order_hint as i32;
    }

    let d0 = get_relative_dist(
        &cm.seq_params.order_hint_info,
        fwd_frame_index,
        cur_frame_index,
    )
    .abs()
    .clamp(0, MAX_FRAME_DISTANCE);
    let d1 = get_relative_dist(
        &cm.seq_params.order_hint_info,
        cur_frame_index,
        bck_frame_index,
    )
    .abs()
    .clamp(0, MAX_FRAME_DISTANCE);

    let order = (d0 <= d1) as usize;

    if d0 == 0 || d1 == 0 {
        *fwd_offset = QUANT_DIST_LOOKUP_TABLE[3][order];
        *bck_offset = QUANT_DIST_LOOKUP_TABLE[3][1 - order];
        return;
    }

    let mut i = 0;
    while i < 3 {
        let c0 = QUANT_DIST_WEIGHT[i][order];
        let c1 = QUANT_DIST_WEIGHT[i][1 - order];
        let d0_c0 = d0 * c0;
        let d1_c1 = d1 * c1;
        if (d0 > d1 && d0_c0 < d1_c1) || (d0 <= d1 && d0_c0 > d1_c1) {
            break;
        }
        i += 1;
    }

    *fwd_offset = QUANT_DIST_LOOKUP_TABLE[i][order];
    *bck_offset = QUANT_DIST_LOOKUP_TABLE[i][1 - order];
}

#[inline]
fn scaled_buffer_offset(
    x_offset: i32,
    y_offset: i32,
    stride: i32,
    sf: Option<&ScaleFactors>,
) -> i64 {
    let (x, y);
    match sf {
        None => {
            x = x_offset;
            y = y_offset;
        }
        Some(sf) if av1_is_scaled(sf) => {
            x = av1_scaled_x(x_offset, sf) >> SCALE_EXTRA_BITS;
            y = av1_scaled_y(y_offset, sf) >> SCALE_EXTRA_BITS;
        }
        Some(sf) => {
            x = av1_unscaled_value(x_offset, sf) >> SCALE_EXTRA_BITS;
            y = av1_unscaled_value(y_offset, sf) >> SCALE_EXTRA_BITS;
        }
    }
    y as i64 * stride as i64 + x as i64
}

/// Translation of `setup_pred_plane()`: `src` is the plane's offset in its
/// frame buffer.
#[inline]
pub(crate) fn setup_pred_plane(
    dst: &mut Buf2d,
    bsize: BlockSize,
    src: usize,
    width: i32,
    height: i32,
    stride: i32,
    mut mi_row: i32,
    mut mi_col: i32,
    scale: Option<&ScaleFactors>,
    subsampling_x: i32,
    subsampling_y: i32,
) {
    // Offset the buffer pointer
    if subsampling_y != 0 && (mi_row & 0x01) != 0 && (MI_SIZE_HIGH[bsize as usize] == 1) {
        mi_row -= 1;
    }
    if subsampling_x != 0 && (mi_col & 0x01) != 0 && (MI_SIZE_WIDE[bsize as usize] == 1) {
        mi_col -= 1;
    }

    let x = (MI_SIZE * mi_col) >> subsampling_x;
    let y = (MI_SIZE * mi_row) >> subsampling_y;
    dst.buf = (src as i64 + scaled_buffer_offset(x, y, stride, scale)) as usize;
    dst.buf0 = src;
    dst.width = width;
    dst.height = height;
    dst.stride = stride;
}

pub(crate) fn av1_setup_dst_planes(
    planes: &mut [MacroblockdPlane; MAX_MB_PLANE],
    bsize: BlockSize,
    src: &Yv12BufferConfig,
    mi_row: i32,
    mi_col: i32,
    plane_start: usize,
    plane_end: usize,
) {
    // We use AOMMIN(num_planes, MAX_MB_PLANE) instead of num_planes to quiet
    // the static analysis warnings.
    for i in plane_start..plane_end.min(MAX_MB_PLANE) {
        let pd = &mut planes[i];
        let is_uv = (i > 0) as usize;
        setup_pred_plane(
            &mut pd.dst,
            bsize,
            src.buffers[i],
            src.crop_widths[is_uv],
            src.crop_heights[is_uv],
            src.strides[is_uv],
            mi_row,
            mi_col,
            None,
            pd.subsampling_x,
            pd.subsampling_y,
        );
    }
}

/// Translation of `av1_setup_pre_planes()`: `src` is the reference frame,
/// at buffer pool index `frame_buf`.
pub(crate) fn av1_setup_pre_planes(
    xd: &mut MacroBlockD,
    idx: usize,
    src: Option<(&Yv12BufferConfig, usize)>,
    mi_row: i32,
    mi_col: i32,
    sf: Option<&ScaleFactors>,
    num_planes: usize,
) {
    if let Some((src, frame_buf)) = src {
        // We use AOMMIN(num_planes, MAX_MB_PLANE) instead of num_planes to quiet
        // the static analysis warnings.
        let bsize = xd.mbmi().bsize;
        for i in 0..num_planes.min(MAX_MB_PLANE) {
            let pd = &mut xd.plane[i];
            let is_uv = (i > 0) as usize;
            setup_pred_plane(
                &mut pd.pre[idx],
                bsize,
                src.buffers[i],
                src.crop_widths[is_uv],
                src.crop_heights[is_uv],
                src.strides[is_uv],
                mi_row,
                mi_col,
                sf,
                pd.subsampling_x,
                pd.subsampling_y,
            );
            pd.pre[idx].frame_buf = frame_buf;
        }
    }
}

/// Translation of `av1_count_overlappable_neighbors()` (with
/// `increment_int_ptr()` as the visitor).
pub(crate) fn av1_count_overlappable_neighbors(cm: &Av1Common, xd: &mut MacroBlockD) {
    xd.mbmi_mut().overlappable_neighbors = 0;

    if !is_motion_variation_allowed_bsize(xd.mbmi().bsize) {
        return;
    }

    let mut count = 0u8;
    foreach_overlappable_nb_above(cm, xd, i32::MAX, &mut |_| count += 1);
    xd.mbmi_mut().overlappable_neighbors = count;
    if xd.mbmi().overlappable_neighbors != 0 {
        return;
    }
    foreach_overlappable_nb_left(cm, xd, i32::MAX, &mut |_| count += 1);
    xd.mbmi_mut().overlappable_neighbors = count;
}

// (obmc.h)

/// Translation of `foreach_overlappable_nb_above()`: the visitor gets the
/// neighbor's mode info index.
pub(crate) fn foreach_overlappable_nb_above(
    cm: &Av1Common,
    xd: &MacroBlockD,
    nb_max: i32,
    fun: &mut dyn FnMut(u32),
) {
    if !xd.up_available {
        return;
    }

    let mut nb_count = 0;
    let mi_col = xd.mi_col;
    // prev_row_mi points into the mi array, starting at the beginning of the
    // previous row.
    let prev_row_mi = xd.mi as isize - mi_col as isize - xd.mi_stride as isize;
    let end_col = (mi_col + xd.width as i32).min(cm.mi_params.mi_cols);
    let mut above_mi_col = mi_col;
    while above_mi_col < end_col && nb_count < nb_max {
        let mut above_mi = xd.mi_grid_base[(prev_row_mi + above_mi_col as isize) as usize];
        let mut mi_step = (MI_SIZE_WIDE[xd.mi_alloc[above_mi as usize].bsize as usize] as i32)
            .min(MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32);
        // If we're considering a block with width 4, it should be treated as
        // half of a pair of blocks with chroma information in the second. Move
        // above_mi_col back to the start of the pair if needed, set above_mbmi
        // to point at the block with chroma information, and set mi_step to 2 to
        // step over the entire pair at the end of the iteration.
        if mi_step == 1 {
            above_mi_col &= !1;
            above_mi = xd.mi_grid_base[(prev_row_mi + above_mi_col as isize + 1) as usize];
            mi_step = 2;
        }
        if is_neighbor_overlappable(&xd.mi_alloc[above_mi as usize]) {
            nb_count += 1;
            fun(above_mi);
        }
        above_mi_col += mi_step;
    }
}

/// Translation of `foreach_overlappable_nb_left()`.
pub(crate) fn foreach_overlappable_nb_left(
    cm: &Av1Common,
    xd: &MacroBlockD,
    nb_max: i32,
    fun: &mut dyn FnMut(u32),
) {
    if !xd.left_available {
        return;
    }

    let mut nb_count = 0;
    // prev_col_mi points into the mi array, starting at the top of the
    // previous column
    let mi_row = xd.mi_row;
    let prev_col_mi = xd.mi as isize - 1 - mi_row as isize * xd.mi_stride as isize;
    let end_row = (mi_row + xd.height as i32).min(cm.mi_params.mi_rows);
    let mut left_mi_row = mi_row;
    while left_mi_row < end_row && nb_count < nb_max {
        let mut left_mi =
            xd.mi_grid_base[(prev_col_mi + left_mi_row as isize * xd.mi_stride as isize) as usize];
        let mut mi_step = (MI_SIZE_HIGH[xd.mi_alloc[left_mi as usize].bsize as usize] as i32)
            .min(MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32);
        if mi_step == 1 {
            left_mi_row &= !1;
            left_mi = xd.mi_grid_base
                [(prev_col_mi + (left_mi_row + 1) as isize * xd.mi_stride as isize) as usize];
            mi_step = 2;
        }
        if is_neighbor_overlappable(&xd.mi_alloc[left_mi as usize]) {
            nb_count += 1;
            fun(left_mi);
        }
        left_mi_row += mi_step;
    }
}

// (reconinter.h)

#[inline]
pub(crate) fn is_interinter_compound_used(type_: i32, sb_type: BlockSize) -> bool {
    let comp_allowed = is_comp_ref_allowed(sb_type);
    match type_ {
        COMPOUND_AVERAGE | COMPOUND_DISTWTD | COMPOUND_DIFFWTD => comp_allowed,
        COMPOUND_WEDGE => comp_allowed && av1_wedge_types_lookup(sb_type) > 0,
        _ => {
            debug_assert!(false);
            false
        }
    }
}

#[inline]
pub(crate) fn is_any_masked_compound_used(sb_type: BlockSize) -> bool {
    if !is_comp_ref_allowed(sb_type) {
        return false;
    }
    for i in 0..COMPOUND_TYPES {
        let comp_type = i;
        if is_masked_compound_type(comp_type) && is_interinter_compound_used(comp_type, sb_type) {
            return true;
        }
    }
    false
}

/// `av1_wedge_params_lookup[sb_type].wedge_types`
#[inline]
pub(crate) fn av1_wedge_types_lookup(sb_type: BlockSize) -> i32 {
    match sb_type {
        BLOCK_8X8 | BLOCK_8X16 | BLOCK_16X8 | BLOCK_16X16 | BLOCK_16X32 | BLOCK_32X16
        | BLOCK_32X32 | BLOCK_8X32 | BLOCK_32X8 => MAX_WEDGE_TYPES,
        _ => 0,
    }
}

#[inline]
pub(crate) fn av1_is_wedge_used(sb_type: BlockSize) -> bool {
    av1_wedge_types_lookup(sb_type) > 0
}

#[inline]
pub(crate) fn av1_is_interp_needed(xd: &MacroBlockD, global_motion: &[WarpedMotionParams]) -> bool {
    let mbmi = xd.mbmi();
    if mbmi.skip_mode != 0 {
        return false;
    }
    if mbmi.motion_mode == WARPED_CAUSAL {
        return false;
    }
    if is_nontrans_global_motion(global_motion, mbmi) {
        return false;
    }
    true
}

/// Translation of `is_nontrans_global_motion()` (blockd.h): `global_motion`
/// is `xd->global_motion`.
#[inline]
pub(crate) fn is_nontrans_global_motion(
    global_motion: &[WarpedMotionParams],
    mbmi: &MbModeInfo,
) -> bool {
    // First check if all modes are GLOBALMV
    if mbmi.mode != GLOBALMV && mbmi.mode != GLOBAL_GLOBALMV {
        return false;
    }

    if (MI_SIZE_WIDE[mbmi.bsize as usize]).min(MI_SIZE_HIGH[mbmi.bsize as usize]) < 2 {
        return false;
    }

    // Now check if all global motion is non translational
    for ref_ in 0..1 + has_second_ref(mbmi) as usize {
        if global_motion[mbmi.ref_frame[ref_] as usize].wmtype == TRANSLATION {
            return false;
        }
    }
    true
}
