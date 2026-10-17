// Rust translation of the parts of av1/common/reconintra.c and
// av1/common/reconintra.h from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Intra prediction, high bit depth, for the modes the saver tries (DC,
//! vertical, horizontal and smooth luma prediction, DC chroma prediction)
//! without angle deltas, palettes, filter intra or chroma from luma. Of
//! the directional predictors only the vertical (90 degree) and horizontal
//! (180 degree) ones are reached, which need neither the top-right nor the
//! bottom-left pixels, and never filter or upsample their edges. The
//! predictor function tables are matched on the mode and the transform
//! size.

use super::av1_common_int::Av1Common;
use super::blockd::*;
use super::enums::*;
use crate::aom::aom_dsp::intrapred::*;

const NEED_LEFT: i32 = 1 << 1;
const NEED_ABOVE: i32 = 1 << 2;
const NEED_ABOVERIGHT: i32 = 1 << 3;
const NEED_ABOVELEFT: i32 = 1 << 4;
const NEED_BOTTOMLEFT: i32 = 1 << 5;

static EXTEND_MODES: [i32; INTRA_MODES as usize] = [
    NEED_ABOVE | NEED_LEFT,                  // DC
    NEED_ABOVE,                              // V
    NEED_LEFT,                               // H
    NEED_ABOVE | NEED_ABOVERIGHT,            // D45
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // D135
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // D113
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // D157
    NEED_LEFT | NEED_BOTTOMLEFT,             // D203
    NEED_ABOVE | NEED_ABOVERIGHT,            // D67
    NEED_LEFT | NEED_ABOVE,                  // SMOOTH
    NEED_LEFT | NEED_ABOVE,                  // SMOOTH_V
    NEED_LEFT | NEED_ABOVE,                  // SMOOTH_H
    NEED_LEFT | NEED_ABOVE | NEED_ABOVELEFT, // PAETH
];

const NUM_INTRA_NEIGHBOUR_PIXELS: usize = (MAX_TX_SIZE * 2 + 32) as usize;

// (reconintra.h)

#[inline]
pub(crate) fn av1_is_directional_mode(mode: PredictionMode) -> bool {
    (V_PRED..=D67_PRED).contains(&mode)
}

#[inline]
pub(crate) fn av1_allow_intrabc(cm: &Av1Common) -> bool {
    super::av1_common_int::frame_is_intra_only(cm)
        && cm.features.allow_screen_content_tools
        && cm.features.allow_intrabc
}

#[inline]
pub(crate) fn av1_filter_intra_allowed_bsize(cm: &Av1Common, bs: BlockSize) -> bool {
    if cm.seq_params.enable_filter_intra == 0 || bs == BLOCK_INVALID {
        return false;
    }

    BLOCK_SIZE_WIDE[bs as usize] <= 32 && BLOCK_SIZE_HIGH[bs as usize] <= 32
}

#[inline]
pub(crate) fn av1_filter_intra_allowed(cm: &Av1Common, mbmi: &MbModeInfo) -> bool {
    mbmi.mode == DC_PRED
        && mbmi.palette_mode_info.palette_size[0] == 0
        && av1_filter_intra_allowed_bsize(cm, mbmi.bsize)
}

#[inline]
pub(crate) fn av1_use_intra_edge_upsample(bs0: i32, bs1: i32, delta: i32, ty: i32) -> bool {
    let d = delta.abs();
    let blk_wh = bs0 + bs1;
    if d == 0 || d >= 40 {
        return false;
    }
    if ty != 0 {
        blk_wh <= 8
    } else {
        blk_wh <= 16
    }
}

// (reconintra.c)

/// Translation of `highbd_dr_predictor()` for the angles the saver reaches
/// (90 and 180 degrees, `pred_high[V_PRED]` and `pred_high[H_PRED]`).
fn highbd_dr_predictor(
    dst: &mut [u16],
    dst_off: usize,
    stride: usize,
    tx_size: TxSize,
    above: &[u16],
    left: &[u16],
    angle: i32,
    bd: i32,
) {
    let bw = TX_SIZE_WIDE[tx_size as usize];
    let bh = TX_SIZE_HIGH[tx_size as usize];
    debug_assert!(angle > 0 && angle < 270);

    if angle == 90 {
        highbd_v_predictor(dst, dst_off, stride, bw, bh, above, left, bd);
    } else {
        debug_assert!(angle == 180);
        highbd_h_predictor(dst, dst_off, stride, bw, bh, above, left, bd);
    }
}

fn is_smooth(mbmi: &MbModeInfo, plane: usize) -> bool {
    if plane == 0 {
        let mode = mbmi.mode;
        mode == SMOOTH_PRED || mode == SMOOTH_V_PRED || mode == SMOOTH_H_PRED
    } else {
        // uv_mode is not set for inter blocks, so need to explicitly
        // detect that case.
        if is_inter_block(mbmi) {
            return false;
        }

        let uv_mode = mbmi.uv_mode;
        uv_mode == UV_SMOOTH_PRED || uv_mode == UV_SMOOTH_V_PRED || uv_mode == UV_SMOOTH_H_PRED
    }
}

fn get_intra_edge_filter_type(xd: &MacroBlockD, plane: usize) -> i32 {
    let ab_sm;
    let le_sm;

    if plane == 0 {
        let ab = xd.mi_at(xd.above_mbmi);
        let le = xd.mi_at(xd.left_mbmi);
        ab_sm = ab.is_some_and(|ab| is_smooth(ab, plane));
        le_sm = le.is_some_and(|le| is_smooth(le, plane));
    } else {
        let ab = xd.mi_at(xd.chroma_above_mbmi);
        let le = xd.mi_at(xd.chroma_left_mbmi);
        ab_sm = ab.is_some_and(|ab| is_smooth(ab, plane));
        le_sm = le.is_some_and(|le| is_smooth(le, plane));
    }

    (ab_sm || le_sm) as i32
}

/// Translation of `build_intra_predictors_high()`; `ref` and `dst` are the
/// same plane of `buf` (as `av1_predict_intra_block_facade()` passes them),
/// at the offsets `dst_off`, with stride `dst_stride`.
fn build_intra_predictors_high(
    buf: &mut [u16],
    dst_off: usize,
    dst_stride: usize,
    mode: PredictionMode,
    p_angle: i32,
    filter_intra_mode: FilterIntraMode,
    tx_size: TxSize,
    disable_edge_filter: bool,
    n_top_px: i32,
    n_topright_px: i32,
    n_left_px: i32,
    n_bottomleft_px: i32,
    intra_edge_filter_type: i32,
    bit_depth: i32,
) {
    let mut left_data = [0u16; NUM_INTRA_NEIGHBOUR_PIXELS];
    let mut above_data = [0u16; NUM_INTRA_NEIGHBOUR_PIXELS];
    // above_row = above_data + 16, left_col = left_data + 16
    const AR: usize = 16;
    const LC: usize = 16;
    let txwpx = TX_SIZE_WIDE[tx_size as usize];
    let txhpx = TX_SIZE_HIGH[tx_size as usize];
    let mut need_left = EXTEND_MODES[mode as usize] & NEED_LEFT != 0;
    let mut need_above = EXTEND_MODES[mode as usize] & NEED_ABOVE != 0;
    let mut need_above_left = EXTEND_MODES[mode as usize] & NEED_ABOVELEFT != 0;
    let above_ref = dst_off - dst_stride;
    let left_ref = dst_off - 1;
    let is_dr_mode = av1_is_directional_mode(mode);
    let use_filter_intra = filter_intra_mode != FILTER_INTRA_MODES;
    let base: i32 = 128 << (bit_depth - 8);
    // The left_data, above_data buffers must be zeroed to fix some intermittent
    // valgrind errors. Uninitialized reads in intra pred modules (e.g. width = 4
    // path in av1_highbd_dr_prediction_z2_avx2()) from left_data, above_data are
    // seen to be the potential reason for this issue.
    left_data.fill((base + 1) as u16);
    above_data.fill((base - 1) as u16);

    // The default values if ref pixels are not available:
    // base   base-1 base-1 .. base-1 base-1 base-1 base-1 base-1 base-1
    // base+1   A      B  ..     Y      Z
    // base+1   C      D  ..     W      X
    // base+1   E      F  ..     U      V
    // base+1   G      H  ..     S      T      T      T      T      T

    if is_dr_mode {
        if p_angle <= 90 {
            need_above = true;
            need_left = false;
            need_above_left = true;
        } else if p_angle < 180 {
            need_above = true;
            need_left = true;
            need_above_left = true;
        } else {
            need_above = false;
            need_left = true;
            need_above_left = true;
        }
    }
    if use_filter_intra {
        need_left = true;
        need_above = true;
        need_above_left = true;
    }

    debug_assert!(n_top_px >= 0);
    debug_assert!(n_topright_px >= -1);
    debug_assert!(n_left_px >= 0);
    debug_assert!(n_bottomleft_px >= -1);

    if (!need_above && n_left_px == 0) || (!need_left && n_top_px == 0) {
        let val: i32 = if need_left {
            if n_top_px > 0 {
                buf[above_ref] as i32
            } else {
                base + 1
            }
        } else if n_left_px > 0 {
            buf[left_ref] as i32
        } else {
            base - 1
        };
        let mut d = dst_off;
        for _i in 0..txhpx {
            buf[d..d + txwpx as usize].fill(val as u16);
            d += dst_stride;
        }
        return;
    }

    // NEED_LEFT
    if need_left {
        let num_left_pixels_needed = txhpx + if n_bottomleft_px >= 0 { txwpx } else { 0 };
        let mut i: i32 = 0;
        if n_left_px > 0 {
            while i < n_left_px {
                left_data[LC + i as usize] = buf[left_ref + i as usize * dst_stride];
                i += 1;
            }
            // (n_bottomleft_px > 0 needs a top-right or bottom-left mode)
            debug_assert!(n_bottomleft_px <= 0);
            if i < num_left_pixels_needed {
                let v = left_data[LC + i as usize - 1];
                left_data[LC + i as usize..LC + num_left_pixels_needed as usize].fill(v);
            }
        } else if n_top_px > 0 {
            let v = buf[above_ref];
            left_data[LC..LC + num_left_pixels_needed as usize].fill(v);
        }
    }

    // NEED_ABOVE
    if need_above {
        let num_top_pixels_needed = txwpx + if n_topright_px >= 0 { txhpx } else { 0 };
        if n_top_px > 0 {
            above_data[AR..AR + n_top_px as usize]
                .copy_from_slice(&buf[above_ref..above_ref + n_top_px as usize]);
            let i = n_top_px;
            // (n_topright_px > 0 needs a top-right or bottom-left mode)
            debug_assert!(n_topright_px <= 0);
            if i < num_top_pixels_needed {
                let v = above_data[AR + i as usize - 1];
                above_data[AR + i as usize..AR + num_top_pixels_needed as usize].fill(v);
            }
        } else if n_left_px > 0 {
            let v = buf[left_ref];
            above_data[AR..AR + num_top_pixels_needed as usize].fill(v);
        }
    }

    if need_above_left {
        if n_top_px > 0 && n_left_px > 0 {
            above_data[AR - 1] = buf[above_ref - 1];
        } else if n_top_px > 0 {
            above_data[AR - 1] = buf[above_ref];
        } else if n_left_px > 0 {
            above_data[AR - 1] = buf[left_ref];
        } else {
            above_data[AR - 1] = base as u16;
        }
        left_data[LC - 1] = above_data[AR - 1];
    }

    // (filter intra is never used)
    debug_assert!(!use_filter_intra);

    if is_dr_mode {
        if !disable_edge_filter {
            // (p_angle is 90 or 180: no edge filtering, and no upsampling)
            debug_assert!(p_angle == 90 || p_angle == 180);
            debug_assert!(!av1_use_intra_edge_upsample(
                txwpx,
                txhpx,
                p_angle - 90,
                intra_edge_filter_type
            ));
            debug_assert!(!av1_use_intra_edge_upsample(
                txhpx,
                txwpx,
                p_angle - 180,
                intra_edge_filter_type
            ));
        }
        highbd_dr_predictor(
            buf,
            dst_off,
            dst_stride,
            tx_size,
            &above_data[AR..],
            &left_data[LC..],
            p_angle,
            bit_depth,
        );
        return;
    }

    // predict
    let above_row = &above_data[AR..];
    let left_col = &left_data[LC..];
    if mode == DC_PRED {
        match (n_left_px > 0, n_top_px > 0) {
            (false, false) => highbd_dc_128_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bit_depth,
            ),
            (false, true) => highbd_dc_top_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bit_depth,
            ),
            (true, false) => highbd_dc_left_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bit_depth,
            ),
            (true, true) => highbd_dc_predictor(
                buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bit_depth,
            ),
        }
    } else {
        // (SMOOTH_PRED: the only non-DC, non-directional mode the saver tries)
        debug_assert!(mode == SMOOTH_PRED);
        highbd_smooth_predictor(
            buf, dst_off, dst_stride, txwpx, txhpx, above_row, left_col, bit_depth,
        );
    }
}

/// Translation of `av1_predict_intra_block()` (high bit depth, no palette),
/// predicting into the reconstructed frame `xd.recon` at `dst_off`.
pub(crate) fn av1_predict_intra_block(
    xd: &mut MacroBlockD,
    _sb_size: BlockSize,
    enable_intra_edge_filter: bool,
    wpx: i32,
    hpx: i32,
    tx_size: TxSize,
    mode: PredictionMode,
    angle_delta: i32,
    use_palette: bool,
    filter_intra_mode: FilterIntraMode,
    dst_off: usize,
    dst_stride: usize,
    col_off: i32,
    row_off: i32,
    plane: usize,
) {
    let txwpx = TX_SIZE_WIDE[tx_size as usize];
    let txhpx = TX_SIZE_HIGH[tx_size as usize];
    let x = col_off << MI_SIZE_LOG2;
    let y = row_off << MI_SIZE_LOG2;

    // (palettes are never used)
    debug_assert!(!use_palette);

    let pd = &xd.plane[plane];
    let ss_x = pd.subsampling_x;
    let ss_y = pd.subsampling_y;
    let have_top = row_off != 0
        || if ss_y != 0 {
            xd.chroma_up_available
        } else {
            xd.up_available
        };
    let have_left = col_off != 0
        || if ss_x != 0 {
            xd.chroma_left_available
        } else {
            xd.left_available
        };

    // Distance between the right edge of this prediction block to
    // the frame right edge
    let xr = (xd.mb_to_right_edge >> (3 + ss_x)) + wpx - x - txwpx;
    // Distance between the bottom edge of this prediction block to
    // the frame bottom edge
    let yd = (xd.mb_to_bottom_edge >> (3 + ss_y)) + hpx - y - txhpx;

    // (the saver's chroma is never subsampled: 4:4:4 color, monochrome
    // alpha)
    debug_assert!(ss_x == 0 && ss_y == 0);
    let is_dr_mode = av1_is_directional_mode(mode);
    let use_filter_intra = filter_intra_mode != FILTER_INTRA_MODES;
    let mut p_angle = 0;
    let mut need_top_right = EXTEND_MODES[mode as usize] & NEED_ABOVERIGHT != 0;
    let mut need_bottom_left = EXTEND_MODES[mode as usize] & NEED_BOTTOMLEFT != 0;

    if use_filter_intra {
        need_top_right = false;
        need_bottom_left = false;
    }
    if is_dr_mode {
        p_angle = MODE_TO_ANGLE_MAP[mode as usize] as i32 + angle_delta;
        need_top_right = p_angle < 90;
        need_bottom_left = p_angle > 180;
    }

    // Possible states for have_top_right(TR) and have_bottom_left(BL)
    // -1 : TR and BL are not needed
    //  0 : TR and BL are needed but not available
    // > 0 : TR and BL are needed and pixels are available
    // (the saver's modes need neither)
    debug_assert!(!need_top_right && !need_bottom_left);
    let have_top_right: i32 = -1;
    let have_bottom_left: i32 = -1;

    let disable_edge_filter = !enable_intra_edge_filter;
    let intra_edge_filter_type = get_intra_edge_filter_type(xd, plane);
    let bd = xd.bd;
    // (high bit depth)
    build_intra_predictors_high(
        &mut xd.recon.buffer_alloc,
        dst_off,
        dst_stride,
        mode,
        p_angle,
        filter_intra_mode,
        tx_size,
        disable_edge_filter,
        if have_top { txwpx.min(xr + txwpx) } else { 0 },
        if have_top_right > 0 {
            txwpx.min(xr)
        } else {
            have_top_right
        },
        if have_left { txhpx.min(yd + txhpx) } else { 0 },
        if have_bottom_left > 0 {
            txhpx.min(yd)
        } else {
            have_bottom_left
        },
        intra_edge_filter_type,
        bd,
    );
}

pub(crate) fn av1_predict_intra_block_facade(
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    plane: usize,
    blk_col: i32,
    blk_row: i32,
    tx_size: TxSize,
) {
    let mbmi = *xd.mbmi();
    let pd = &xd.plane[plane];
    let dst_stride = pd.dst.stride;
    let dst = pd.dst.buf + ((blk_row * dst_stride + blk_col) << MI_SIZE_LOG2) as usize;
    let mode = if plane == 0 {
        mbmi.mode
    } else {
        get_uv_mode(mbmi.uv_mode)
    };
    let use_palette = mbmi.palette_mode_info.palette_size[(plane != 0) as usize] > 0;
    let filter_intra_mode = if plane == 0 && mbmi.filter_intra_mode_info.use_filter_intra != 0 {
        mbmi.filter_intra_mode_info.filter_intra_mode
    } else {
        FILTER_INTRA_MODES
    };
    let angle_delta = mbmi.angle_delta[(plane != 0) as usize] as i32 * ANGLE_STEP;
    let seq_params = &cm.seq_params;

    // (chroma from luma is never used)
    debug_assert!(plane == 0 || mbmi.uv_mode != UV_CFL_PRED);
    let (pw, ph) = (pd.width as i32, pd.height as i32);
    av1_predict_intra_block(
        xd,
        seq_params.sb_size,
        seq_params.enable_intra_edge_filter != 0,
        pw,
        ph,
        tx_size,
        mode,
        angle_delta,
        use_palette,
        filter_intra_mode,
        dst,
        dst_stride as usize,
        blk_col,
        blk_row,
        plane,
    );
}
