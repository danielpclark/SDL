// Rust translation of the parts of av1/common/mvref_common.c and
// av1/common/mvref_common.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Reference motion vector lists, the temporal motion field and the
//! frame-level reference setup.
//!
//! `xd->mi[off]` is the grid entry at offset `off` from the block's
//! (`MacroBlockD::mi_idx()`); the candidates the scans read are always
//! inside the tile, so never NULL.

use super::av1_common_int::*;
use super::blockd::*;
use super::enums::*;
use super::mv::*;
use super::tile_common::TileInfo;
use crate::aom::aom_dsp::aom_dsp_common::{clamp, round_power_of_two_signed};

pub(crate) const MVREF_ROW_COLS: i32 = 3;

// Set the upper limit of the motion vector component magnitude.
// This would make a motion vector fit in 26 bits. Plus 3 bits for the
// reference frame index. A tuple of motion vector can hence be stored within
// 32 bit range for efficient load/store operations.
pub(crate) const REFMVS_LIMIT: i32 = (1 << 12) - 1;

/// `POSITION`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Position {
    pub(crate) row: i32,
    pub(crate) col: i32,
}

// clamp_mv_ref
pub(crate) const MV_BORDER: i32 = 16 << 3; // Allow 16 pels in 1/8th pel units

/// Translation of `get_relative_dist()`.
#[inline]
pub(crate) fn get_relative_dist(oh: &OrderHintInfo, a: i32, b: i32) -> i32 {
    if oh.enable_order_hint == 0 {
        return 0;
    }

    let bits = oh.order_hint_bits_minus_1 + 1;

    debug_assert!(bits >= 1);
    debug_assert!(a >= 0 && a < (1 << bits));
    debug_assert!(b >= 0 && b < (1 << bits));

    let mut diff = a - b;
    let m = 1 << (bits - 1);
    diff = (diff & (m - 1)) - (diff & m);
    diff
}

/// Translation of `clamp_mv()` (mv.h).
#[inline]
pub(crate) fn clamp_mv(mv: &mut Mv, mv_limits: &SubpelMvLimits) {
    mv.col = clamp(mv.col as i32, mv_limits.col_min, mv_limits.col_max) as i16;
    mv.row = clamp(mv.row as i32, mv_limits.row_min, mv_limits.row_max) as i16;
}

/// Translation of `clamp_mv_ref()`.
#[inline]
pub(crate) fn clamp_mv_ref(mv: &mut Mv, bw: i32, bh: i32, xd: &MacroBlockD) {
    let mv_limits = SubpelMvLimits {
        col_min: xd.mb_to_left_edge - get_mv_subpel(bw) - MV_BORDER,
        col_max: xd.mb_to_right_edge + get_mv_subpel(bw) + MV_BORDER,
        row_min: xd.mb_to_top_edge - get_mv_subpel(bh) - MV_BORDER,
        row_max: xd.mb_to_bottom_edge + get_mv_subpel(bh) + MV_BORDER,
    };
    clamp_mv(mv, &mv_limits);
}

/// Translation of `get_block_mv()`.
#[inline]
fn get_block_mv(candidate: &MbModeInfo, which_mv: usize) -> IntMv {
    candidate.mv[which_mv]
}

// Checks that the given mi_row, mi_col and search point
// are inside the borders of the tile.
/// Translation of `is_inside()`.
#[inline]
pub(crate) fn is_inside(tile: &TileInfo, mi_col: i32, mi_row: i32, mi_pos: &Position) -> bool {
    !(mi_row + mi_pos.row < tile.mi_row_start
        || mi_col + mi_pos.col < tile.mi_col_start
        || mi_row + mi_pos.row >= tile.mi_row_end
        || mi_col + mi_pos.col >= tile.mi_col_end)
}

/// Translation of `find_valid_row_offset()`.
#[inline]
fn find_valid_row_offset(tile: &TileInfo, mi_row: i32, row_offset: i32) -> i32 {
    clamp(
        row_offset,
        tile.mi_row_start - mi_row,
        tile.mi_row_end - mi_row - 1,
    )
}

/// Translation of `find_valid_col_offset()`.
#[inline]
fn find_valid_col_offset(tile: &TileInfo, mi_col: i32, col_offset: i32) -> i32 {
    clamp(
        col_offset,
        tile.mi_col_start - mi_col,
        tile.mi_col_end - mi_col - 1,
    )
}

/// Translation of `lower_mv_precision()`.
#[inline]
pub(crate) fn lower_mv_precision(mv: &mut Mv, allow_hp: bool, is_integer: bool) {
    if is_integer {
        integer_mv_precision(mv);
    } else if !allow_hp {
        if mv.row & 1 != 0 {
            mv.row += if mv.row > 0 { -1 } else { 1 };
        }
        if mv.col & 1 != 0 {
            mv.col += if mv.col > 0 { -1 } else { 1 };
        }
    }
}

/// Translation of `comp_ref0()` (blockd.h).
#[inline]
fn comp_ref0(ref_idx: i32) -> MvReferenceFrame {
    const LUT: [MvReferenceFrame; TOTAL_UNIDIR_COMP_REFS as usize] = [
        LAST_FRAME,    // LAST_LAST2_FRAMES,
        LAST_FRAME,    // LAST_LAST3_FRAMES,
        LAST_FRAME,    // LAST_GOLDEN_FRAMES,
        BWDREF_FRAME,  // BWDREF_ALTREF_FRAMES,
        LAST2_FRAME,   // LAST2_LAST3_FRAMES
        LAST2_FRAME,   // LAST2_GOLDEN_FRAMES,
        LAST3_FRAME,   // LAST3_GOLDEN_FRAMES,
        BWDREF_FRAME,  // BWDREF_ALTREF2_FRAMES,
        ALTREF2_FRAME, // ALTREF2_ALTREF_FRAMES,
    ];
    LUT[ref_idx as usize]
}

/// Translation of `comp_ref1()` (blockd.h).
#[inline]
fn comp_ref1(ref_idx: i32) -> MvReferenceFrame {
    const LUT: [MvReferenceFrame; TOTAL_UNIDIR_COMP_REFS as usize] = [
        LAST2_FRAME,   // LAST_LAST2_FRAMES,
        LAST3_FRAME,   // LAST_LAST3_FRAMES,
        GOLDEN_FRAME,  // LAST_GOLDEN_FRAMES,
        ALTREF_FRAME,  // BWDREF_ALTREF_FRAMES,
        LAST3_FRAME,   // LAST2_LAST3_FRAMES
        GOLDEN_FRAME,  // LAST2_GOLDEN_FRAMES,
        GOLDEN_FRAME,  // LAST3_GOLDEN_FRAMES,
        ALTREF2_FRAME, // BWDREF_ALTREF2_FRAMES,
        ALTREF_FRAME,  // ALTREF2_ALTREF_FRAMES,
    ];
    LUT[ref_idx as usize]
}

/// Translation of `get_uni_comp_ref_idx()`.
#[inline]
fn get_uni_comp_ref_idx(rf: &[MvReferenceFrame; 2]) -> i8 {
    // Single ref pred
    if rf[1] <= INTRA_FRAME {
        return -1;
    }

    // Bi-directional comp ref pred
    if (rf[0] < BWDREF_FRAME) && (rf[1] >= BWDREF_FRAME) {
        return -1;
    }

    for ref_idx in 0..TOTAL_UNIDIR_COMP_REFS {
        if rf[0] == comp_ref0(ref_idx) && rf[1] == comp_ref1(ref_idx) {
            return ref_idx as i8;
        }
    }
    -1
}

/// Translation of `av1_ref_frame_type()`.
#[inline]
pub(crate) fn av1_ref_frame_type(rf: &[MvReferenceFrame; 2]) -> i32 {
    if rf[1] > INTRA_FRAME {
        let uni_comp_ref_idx = get_uni_comp_ref_idx(rf);
        if uni_comp_ref_idx >= 0 {
            debug_assert!(
                (REF_FRAMES + FWD_REFS * BWD_REFS + uni_comp_ref_idx as i32) < MODE_CTX_REF_FRAMES
            );
            return REF_FRAMES + FWD_REFS * BWD_REFS + uni_comp_ref_idx as i32;
        } else {
            return REF_FRAMES + (rf[0] - LAST_FRAME) + (rf[1] - BWDREF_FRAME) * FWD_REFS;
        }
    }

    rf[0]
}

// clang-format off
static REF_FRAME_MAP: [[MvReferenceFrame; 2]; TOTAL_COMP_REFS as usize] = [
    [LAST_FRAME, BWDREF_FRAME],
    [LAST2_FRAME, BWDREF_FRAME],
    [LAST3_FRAME, BWDREF_FRAME],
    [GOLDEN_FRAME, BWDREF_FRAME],
    [LAST_FRAME, ALTREF2_FRAME],
    [LAST2_FRAME, ALTREF2_FRAME],
    [LAST3_FRAME, ALTREF2_FRAME],
    [GOLDEN_FRAME, ALTREF2_FRAME],
    [LAST_FRAME, ALTREF_FRAME],
    [LAST2_FRAME, ALTREF_FRAME],
    [LAST3_FRAME, ALTREF_FRAME],
    [GOLDEN_FRAME, ALTREF_FRAME],
    [LAST_FRAME, LAST2_FRAME],
    [LAST_FRAME, LAST3_FRAME],
    [LAST_FRAME, GOLDEN_FRAME],
    [BWDREF_FRAME, ALTREF_FRAME],
    // NOTE: Following reference frame pairs are not supported to be explicitly
    //       signalled, but they are possibly chosen by the use of skip_mode,
    //       which may use the most recent one-sided reference frame pair.
    [LAST2_FRAME, LAST3_FRAME],
    [LAST2_FRAME, GOLDEN_FRAME],
    [LAST3_FRAME, GOLDEN_FRAME],
    [BWDREF_FRAME, ALTREF2_FRAME],
    [ALTREF2_FRAME, ALTREF_FRAME],
];
// clang-format on

/// Translation of `av1_set_ref_frame()`.
#[inline]
pub(crate) fn av1_set_ref_frame(rf: &mut [MvReferenceFrame; 2], ref_frame_type: i32) {
    if ref_frame_type >= REF_FRAMES {
        rf[0] = REF_FRAME_MAP[(ref_frame_type - REF_FRAMES) as usize][0];
        rf[1] = REF_FRAME_MAP[(ref_frame_type - REF_FRAMES) as usize][1];
    } else {
        debug_assert!(ref_frame_type > NONE_FRAME);
        rf[0] = ref_frame_type;
        rf[1] = NONE_FRAME;
    }
}

static COMPOUND_MODE_CTX_MAP: [[u16; COMP_NEWMV_CTXS as usize]; 3] =
    [[0, 1, 1, 1, 1], [1, 2, 3, 4, 4], [4, 4, 5, 6, 7]];

/// Translation of `av1_mode_context_analyzer()`.
#[inline]
pub(crate) fn av1_mode_context_analyzer(mode_context: &[i16], rf: &[MvReferenceFrame; 2]) -> i16 {
    let ref_frame = av1_ref_frame_type(rf);

    if rf[1] <= INTRA_FRAME {
        return mode_context[ref_frame as usize];
    }

    let newmv_ctx = mode_context[ref_frame as usize] as i32 & NEWMV_CTX_MASK;
    let refmv_ctx = (mode_context[ref_frame as usize] as i32 >> REFMV_OFFSET) & REFMV_CTX_MASK;

    let comp_ctx = COMPOUND_MODE_CTX_MAP[(refmv_ctx >> 1) as usize]
        [newmv_ctx.min(COMP_NEWMV_CTXS - 1) as usize];
    comp_ctx as i16
}

/// Translation of `av1_drl_ctx()`.
#[inline]
pub(crate) fn av1_drl_ctx(ref_mv_weight: &[u16], ref_idx: usize) -> u8 {
    let lvl = REF_CAT_LEVEL as u16;
    if ref_mv_weight[ref_idx] >= lvl && ref_mv_weight[ref_idx + 1] >= lvl {
        return 0;
    }

    if ref_mv_weight[ref_idx] >= lvl && ref_mv_weight[ref_idx + 1] < lvl {
        return 1;
    }

    if ref_mv_weight[ref_idx] < lvl && ref_mv_weight[ref_idx + 1] < lvl {
        return 2;
    }

    0
}

/// Translation of `av1_collect_neighbors_ref_counts()`.
#[inline]
pub(crate) fn av1_collect_neighbors_ref_counts(xd: &mut MacroBlockD) {
    xd.neighbors_ref_counts = [0; REF_FRAMES as usize];

    let above_in_image = xd.up_available;
    let left_in_image = xd.left_available;

    // Above neighbor
    if above_in_image {
        if let Some(above_mbmi) = xd.mi_at(xd.above_mbmi).copied() {
            if is_inter_block(&above_mbmi) {
                xd.neighbors_ref_counts[above_mbmi.ref_frame[0] as usize] += 1;
                if has_second_ref(&above_mbmi) {
                    xd.neighbors_ref_counts[above_mbmi.ref_frame[1] as usize] += 1;
                }
            }
        }
    }

    // Left neighbor
    if left_in_image {
        if let Some(left_mbmi) = xd.mi_at(xd.left_mbmi).copied() {
            if is_inter_block(&left_mbmi) {
                xd.neighbors_ref_counts[left_mbmi.ref_frame[0] as usize] += 1;
                if has_second_ref(&left_mbmi) {
                    xd.neighbors_ref_counts[left_mbmi.ref_frame[1] as usize] += 1;
                }
            }
        }
    }
}

// Although we assign 32 bit integers, all the values are strictly under 14
// bits.
static DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];

// TODO(jingning): Consider the use of lookup table for (num / den)
// altogether.
/// Translation of `get_mv_projection()`.
#[inline]
fn get_mv_projection(output: &mut Mv, ref_: Mv, num: i32, den: i32) {
    let den = den.min(MAX_FRAME_DISTANCE);
    let num = if num > 0 {
        num.min(MAX_FRAME_DISTANCE)
    } else {
        num.max(-MAX_FRAME_DISTANCE)
    };
    let mv_row = round_power_of_two_signed(
        (ref_.row as i32)
            .wrapping_mul(num)
            .wrapping_mul(DIV_MULT[den as usize]),
        14,
    );
    let mv_col = round_power_of_two_signed(
        (ref_.col as i32)
            .wrapping_mul(num)
            .wrapping_mul(DIV_MULT[den as usize]),
        14,
    );
    let clamp_max = super::entropymv::MV_UPP - 1;
    let clamp_min = super::entropymv::MV_LOW + 1;
    output.row = clamp(mv_row, clamp_min, clamp_max) as i16;
    output.col = clamp(mv_col, clamp_min, clamp_max) as i16;
}

/// Translation of `av1_copy_frame_mvs()`: `frame_mvs` is
/// `cm->cur_frame->mvs`.
pub(crate) fn av1_copy_frame_mvs(
    cm: &Av1Common,
    frame_mvs_base: &mut [MvRef],
    mi: &MbModeInfo,
    mi_row: i32,
    mi_col: i32,
    x_mis: i32,
    y_mis: i32,
) {
    let frame_mvs_stride = (cm.mi_params.mi_cols + 1) >> 1;
    let mut frame_mvs = ((mi_row >> 1) * frame_mvs_stride + (mi_col >> 1)) as usize;
    let x_mis = (x_mis + 1) >> 1;
    let y_mis = (y_mis + 1) >> 1;

    for _h in 0..y_mis {
        let mut mv = frame_mvs;
        for _w in 0..x_mis {
            let m = &mut frame_mvs_base[mv];
            m.ref_frame = NONE_FRAME;
            m.mv.as_int = 0;

            for idx in 0..2 {
                let ref_frame = mi.ref_frame[idx];
                if ref_frame > INTRA_FRAME {
                    let ref_idx = cm.ref_frame_side[ref_frame as usize];
                    if ref_idx != 0 {
                        continue;
                    }
                    if (mi.mv[idx].as_mv().row as i32).abs() > REFMVS_LIMIT
                        || (mi.mv[idx].as_mv().col as i32).abs() > REFMVS_LIMIT
                    {
                        continue;
                    }
                    m.ref_frame = ref_frame;
                    m.mv.as_int = mi.mv[idx].as_int;
                }
            }
            mv += 1;
        }
        frame_mvs += frame_mvs_stride as usize;
    }
}

/// The reference MV list being built (the C functions' `refmv_count`,
/// `ref_mv_stack` and `ref_mv_weight` arguments).
struct RefMvList<'a> {
    refmv_count: &'a mut u8,
    ref_mv_stack: &'a mut [CandidateMv; MAX_REF_MV_STACK_SIZE as usize],
    ref_mv_weight: &'a mut [u16; MAX_REF_MV_STACK_SIZE as usize],
}

/// Translation of `add_ref_mv_candidate()`.
#[allow(clippy::too_many_arguments)]
fn add_ref_mv_candidate(
    candidate: &MbModeInfo,
    rf: &[MvReferenceFrame; 2],
    l: &mut RefMvList,
    ref_match_count: &mut u8,
    newmv_count: &mut u8,
    gm_mv_candidates: &[IntMv; 2],
    gm_params: &[WarpedMotionParams; REF_FRAMES as usize],
    weight: u16,
) {
    if !is_inter_block(candidate) {
        return;
    }
    debug_assert!(weight % 2 == 0);
    let mut index;

    if rf[1] == NONE_FRAME {
        // single reference frame
        for ref_ in 0..2 {
            if candidate.ref_frame[ref_] == rf[0] {
                let is_gm_block = is_global_mv_block(candidate, gm_params[rf[0] as usize].wmtype);
                let this_refmv = if is_gm_block {
                    gm_mv_candidates[0]
                } else {
                    get_block_mv(candidate, ref_)
                };
                index = 0;
                while index < *l.refmv_count as usize {
                    if l.ref_mv_stack[index].this_mv.as_int == this_refmv.as_int {
                        l.ref_mv_weight[index] = l.ref_mv_weight[index].wrapping_add(weight);
                        break;
                    }
                    index += 1;
                }

                // Add a new item to the list.
                if index == *l.refmv_count as usize && *l.refmv_count < MAX_REF_MV_STACK_SIZE as u8
                {
                    l.ref_mv_stack[index].this_mv = this_refmv;
                    l.ref_mv_weight[index] = weight;
                    *l.refmv_count += 1;
                }
                if have_newmv_in_inter_mode(candidate.mode) {
                    *newmv_count += 1;
                }
                *ref_match_count += 1;
            }
        }
    } else {
        // compound reference frame
        if candidate.ref_frame[0] == rf[0] && candidate.ref_frame[1] == rf[1] {
            let mut this_refmv = [IntMv::default(); 2];

            for ref_ in 0..2 {
                if is_global_mv_block(candidate, gm_params[rf[ref_] as usize].wmtype) {
                    this_refmv[ref_] = gm_mv_candidates[ref_];
                } else {
                    this_refmv[ref_] = get_block_mv(candidate, ref_);
                }
            }

            index = 0;
            while index < *l.refmv_count as usize {
                if (l.ref_mv_stack[index].this_mv.as_int == this_refmv[0].as_int)
                    && (l.ref_mv_stack[index].comp_mv.as_int == this_refmv[1].as_int)
                {
                    l.ref_mv_weight[index] = l.ref_mv_weight[index].wrapping_add(weight);
                    break;
                }
                index += 1;
            }

            // Add a new item to the list.
            if index == *l.refmv_count as usize && *l.refmv_count < MAX_REF_MV_STACK_SIZE as u8 {
                l.ref_mv_stack[index].this_mv = this_refmv[0];
                l.ref_mv_stack[index].comp_mv = this_refmv[1];
                l.ref_mv_weight[index] = weight;
                *l.refmv_count += 1;
            }
            if have_newmv_in_inter_mode(candidate.mode) {
                *newmv_count += 1;
            }
            *ref_match_count += 1;
        }
    }
}

/// `xd->mi[off]`
#[inline]
fn cand(xd: &MacroBlockD, off: isize) -> &MbModeInfo {
    &xd.mi_alloc[xd.mi_idx(off) as usize]
}

/// Translation of `scan_row_mbmi()`.
#[allow(clippy::too_many_arguments)]
fn scan_row_mbmi(
    cm: &Av1Common,
    xd: &MacroBlockD,
    mi_col: i32,
    rf: &[MvReferenceFrame; 2],
    row_offset: i32,
    l: &mut RefMvList,
    ref_match_count: &mut u8,
    newmv_count: &mut u8,
    gm_mv_candidates: &[IntMv; 2],
    max_row_offset: i32,
    processed_rows: &mut i32,
) {
    let mut end_mi = (xd.width as i32).min(cm.mi_params.mi_cols - mi_col);
    end_mi = end_mi.min(MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32);
    let width_8x8 = MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32;
    let width_16x16 = MI_SIZE_WIDE[BLOCK_16X16 as usize] as i32;
    let mut col_offset = 0;
    // TODO(jingning): Revisit this part after cb4x4 is stable.
    if row_offset.abs() > 1 {
        col_offset = 1;
        if (mi_col & 0x01) != 0 && (xd.width as i32) < width_8x8 {
            col_offset -= 1;
        }
    }
    let use_step_16 = xd.width >= 16;
    let candidate_mi0 = (row_offset * xd.mi_stride) as isize;

    let mut i = 0;
    while i < end_mi {
        let candidate = cand(xd, candidate_mi0 + (col_offset + i) as isize);
        let candidate_bsize = candidate.bsize;
        let n4_w = MI_SIZE_WIDE[candidate_bsize as usize] as i32;
        let mut len = (xd.width as i32).min(n4_w);
        if use_step_16 {
            len = width_16x16.max(len);
        } else if row_offset.abs() > 1 {
            len = len.max(width_8x8);
        }

        let mut weight: u16 = 2;
        if xd.width as i32 >= width_8x8 && xd.width as i32 <= n4_w {
            let inc = (-max_row_offset + row_offset + 1)
                .min(MI_SIZE_HIGH[candidate_bsize as usize] as i32) as u16;
            // Obtain range used in weight calculation.
            weight = weight.max(inc);
            // Update processed rows.
            *processed_rows = inc as i32 - row_offset - 1;
        }

        add_ref_mv_candidate(
            candidate,
            rf,
            l,
            ref_match_count,
            newmv_count,
            gm_mv_candidates,
            &cm.global_motion,
            (len as u16).wrapping_mul(weight),
        );

        i += len;
    }
}

/// Translation of `scan_col_mbmi()`.
#[allow(clippy::too_many_arguments)]
fn scan_col_mbmi(
    cm: &Av1Common,
    xd: &MacroBlockD,
    mi_row: i32,
    rf: &[MvReferenceFrame; 2],
    col_offset: i32,
    l: &mut RefMvList,
    ref_match_count: &mut u8,
    newmv_count: &mut u8,
    gm_mv_candidates: &[IntMv; 2],
    max_col_offset: i32,
    processed_cols: &mut i32,
) {
    let mut end_mi = (xd.height as i32).min(cm.mi_params.mi_rows - mi_row);
    end_mi = end_mi.min(MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32);
    let n8_h_8 = MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32;
    let n8_h_16 = MI_SIZE_HIGH[BLOCK_16X16 as usize] as i32;
    let mut row_offset = 0;
    if col_offset.abs() > 1 {
        row_offset = 1;
        if (mi_row & 0x01) != 0 && (xd.height as i32) < n8_h_8 {
            row_offset -= 1;
        }
    }
    let use_step_16 = xd.height >= 16;

    let mut i = 0;
    while i < end_mi {
        let candidate = cand(xd, ((row_offset + i) * xd.mi_stride + col_offset) as isize);
        let candidate_bsize = candidate.bsize;
        let n4_h = MI_SIZE_HIGH[candidate_bsize as usize] as i32;
        let mut len = (xd.height as i32).min(n4_h);
        if use_step_16 {
            len = n8_h_16.max(len);
        } else if col_offset.abs() > 1 {
            len = len.max(n8_h_8);
        }

        let mut weight = 2;
        if xd.height as i32 >= n8_h_8 && xd.height as i32 <= n4_h {
            let inc = (-max_col_offset + col_offset + 1)
                .min(MI_SIZE_WIDE[candidate_bsize as usize] as i32);
            // Obtain range used in weight calculation.
            weight = weight.max(inc);
            // Update processed cols.
            *processed_cols = inc - col_offset - 1;
        }

        add_ref_mv_candidate(
            candidate,
            rf,
            l,
            ref_match_count,
            newmv_count,
            gm_mv_candidates,
            &cm.global_motion,
            (len * weight) as u16,
        );

        i += len;
    }
}

/// Translation of `scan_blk_mbmi()`.
#[allow(clippy::too_many_arguments)]
fn scan_blk_mbmi(
    cm: &Av1Common,
    xd: &MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    rf: &[MvReferenceFrame; 2],
    row_offset: i32,
    col_offset: i32,
    l: &mut RefMvList,
    ref_match_count: &mut u8,
    newmv_count: &mut u8,
    gm_mv_candidates: &[IntMv; 2],
) {
    let tile = &xd.tile;
    let mi_pos = Position {
        row: row_offset,
        col: col_offset,
    };

    if is_inside(tile, mi_col, mi_row, &mi_pos) {
        let candidate = cand(xd, (mi_pos.row * xd.mi_stride + mi_pos.col) as isize);
        let len = MI_SIZE_WIDE[BLOCK_8X8 as usize] as u16;

        add_ref_mv_candidate(
            candidate,
            rf,
            l,
            ref_match_count,
            newmv_count,
            gm_mv_candidates,
            &cm.global_motion,
            2 * len,
        );
    } // Analyze a single 8x8 block motion information.
}

/// Translation of `has_top_right()`.
fn has_top_right(cm: &Av1Common, xd: &MacroBlockD, mi_row: i32, mi_col: i32, mut bs: i32) -> bool {
    let sb_mi_size = MI_SIZE_WIDE[cm.seq_params.sb_size as usize] as i32;
    let mask_row = mi_row & (sb_mi_size - 1);
    let mask_col = mi_col & (sb_mi_size - 1);

    if bs > MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32 {
        return false;
    }

    // In a split partition all apart from the bottom right has a top right
    let mut has_tr = !((mask_row & bs) != 0 && (mask_col & bs) != 0);

    // bs > 0 and bs is a power of 2
    debug_assert!(bs > 0 && (bs & (bs - 1)) == 0);

    // For each 4x4 group of blocks, when the bottom right is decoded the blocks
    // to the right have not been decoded therefore the bottom right does
    // not have a top right
    while bs < sb_mi_size {
        if (mask_col & bs) != 0 {
            if (mask_col & (2 * bs)) != 0 && (mask_row & (2 * bs)) != 0 {
                has_tr = false;
                break;
            }
        } else {
            break;
        }
        bs <<= 1;
    }

    // In a VERTICAL or VERTICAL_4 partition, all partition before the last one
    // always have a top right (as the block above will have been decoded).
    if xd.width < xd.height && !xd.is_last_vertical_rect {
        has_tr = true;
    }

    // In a HORIZONTAL or HORIZONTAL_4 partition, partitions after the first one
    // never have a top right (as the block to the right won't have been decoded).
    if xd.width > xd.height && !xd.is_first_horizontal_rect {
        has_tr = false;
    }

    // The bottom left square of a Vertical A (in the old format) does
    // not have a top right as it is decoded before the right hand
    // rectangle of the partition
    if xd.mbmi().partition == PARTITION_VERT_A && xd.width == xd.height && (mask_row & bs) != 0 {
        has_tr = false;
    }

    has_tr
}

/// Translation of `check_sb_border()`.
fn check_sb_border(mi_row: i32, mi_col: i32, row_offset: i32, col_offset: i32) -> bool {
    let sb_mi_size = MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32;
    let row = mi_row & (sb_mi_size - 1);
    let col = mi_col & (sb_mi_size - 1);

    !(row + row_offset < 0
        || row + row_offset >= sb_mi_size
        || col + col_offset < 0
        || col + col_offset >= sb_mi_size)
}

/// Translation of `add_tpl_ref_mv()`.
#[allow(clippy::too_many_arguments)]
fn add_tpl_ref_mv(
    cm: &Av1Common,
    xd: &MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    ref_frame: MvReferenceFrame,
    blk_row: i32,
    blk_col: i32,
    gm_mv_candidates: &[IntMv; 2],
    l: &mut RefMvList,
    mode_context: &mut [i16],
) -> bool {
    let mi_pos = Position {
        row: if (mi_row & 0x01) != 0 {
            blk_row
        } else {
            blk_row + 1
        },
        col: if (mi_col & 0x01) != 0 {
            blk_col
        } else {
            blk_col + 1
        },
    };

    if !is_inside(&xd.tile, mi_col, mi_row, &mi_pos) {
        return false;
    }

    let prev_frame_mvs = &cm.tpl_mvs[(((mi_row + mi_pos.row) >> 1) * (cm.mi_params.mi_stride >> 1)
        + ((mi_col + mi_pos.col) >> 1)) as usize];
    if prev_frame_mvs.mfmv0.as_int == INVALID_MV {
        return false;
    }

    let mut rf = [NONE_FRAME; 2];
    av1_set_ref_frame(&mut rf, ref_frame);

    let weight_unit: u16 = 1; // mi_size_wide[BLOCK_8X8];
    let cur_frame_index = cm.cur_frame().order_hint as i32;
    let Some(buf_0) = get_ref_frame_buf(cm, rf[0]) else {
        return false;
    };
    let frame0_index = cm.buffer_pool.frame_bufs[buf_0].order_hint as i32;
    let cur_offset_0 = get_relative_dist(
        &cm.seq_params.order_hint_info,
        cur_frame_index,
        frame0_index,
    );
    let allow_high_precision_mv = cm.features.allow_high_precision_mv;
    let force_integer_mv = cm.features.cur_frame_force_integer_mv;

    let mut this_refmv = Mv::default();
    get_mv_projection(
        &mut this_refmv,
        prev_frame_mvs.mfmv0.as_mv(),
        cur_offset_0,
        prev_frame_mvs.ref_frame_offset as i32,
    );
    lower_mv_precision(&mut this_refmv, allow_high_precision_mv, force_integer_mv);
    let this_refmv = IntMv::from_mv(this_refmv);

    if rf[1] == NONE_FRAME {
        if blk_row == 0 && blk_col == 0 {
            let gm = gm_mv_candidates[0].as_mv();
            if (this_refmv.as_mv().row as i32 - gm.row as i32).abs() >= 16
                || (this_refmv.as_mv().col as i32 - gm.col as i32).abs() >= 16
            {
                mode_context[ref_frame as usize] |= 1 << GLOBALMV_OFFSET;
            }
        }

        let mut idx = 0;
        while idx < *l.refmv_count as usize {
            if this_refmv.as_int == l.ref_mv_stack[idx].this_mv.as_int {
                break;
            }
            idx += 1;
        }

        if idx < *l.refmv_count as usize {
            l.ref_mv_weight[idx] = l.ref_mv_weight[idx].wrapping_add(2 * weight_unit);
        }

        if idx == *l.refmv_count as usize && *l.refmv_count < MAX_REF_MV_STACK_SIZE as u8 {
            l.ref_mv_stack[idx].this_mv.as_int = this_refmv.as_int;
            l.ref_mv_weight[idx] = 2 * weight_unit;
            *l.refmv_count += 1;
        }
    } else {
        // Process compound inter mode
        let Some(buf_1) = get_ref_frame_buf(cm, rf[1]) else {
            return false;
        };
        let frame1_index = cm.buffer_pool.frame_bufs[buf_1].order_hint as i32;
        let cur_offset_1 = get_relative_dist(
            &cm.seq_params.order_hint_info,
            cur_frame_index,
            frame1_index,
        );
        let mut comp_refmv = Mv::default();
        get_mv_projection(
            &mut comp_refmv,
            prev_frame_mvs.mfmv0.as_mv(),
            cur_offset_1,
            prev_frame_mvs.ref_frame_offset as i32,
        );
        lower_mv_precision(&mut comp_refmv, allow_high_precision_mv, force_integer_mv);
        let comp_refmv = IntMv::from_mv(comp_refmv);

        if blk_row == 0 && blk_col == 0 {
            let gm0 = gm_mv_candidates[0].as_mv();
            let gm1 = gm_mv_candidates[1].as_mv();
            if (this_refmv.as_mv().row as i32 - gm0.row as i32).abs() >= 16
                || (this_refmv.as_mv().col as i32 - gm0.col as i32).abs() >= 16
                || (comp_refmv.as_mv().row as i32 - gm1.row as i32).abs() >= 16
                || (comp_refmv.as_mv().col as i32 - gm1.col as i32).abs() >= 16
            {
                mode_context[ref_frame as usize] |= 1 << GLOBALMV_OFFSET;
            }
        }

        let mut idx = 0;
        while idx < *l.refmv_count as usize {
            if this_refmv.as_int == l.ref_mv_stack[idx].this_mv.as_int
                && comp_refmv.as_int == l.ref_mv_stack[idx].comp_mv.as_int
            {
                break;
            }
            idx += 1;
        }

        if idx < *l.refmv_count as usize {
            l.ref_mv_weight[idx] = l.ref_mv_weight[idx].wrapping_add(2 * weight_unit);
        }

        if idx == *l.refmv_count as usize && *l.refmv_count < MAX_REF_MV_STACK_SIZE as u8 {
            l.ref_mv_stack[idx].this_mv.as_int = this_refmv.as_int;
            l.ref_mv_stack[idx].comp_mv.as_int = comp_refmv.as_int;
            l.ref_mv_weight[idx] = 2 * weight_unit;
            *l.refmv_count += 1;
        }
    }

    true
}

/// Translation of `process_compound_ref_mv_candidate()`.
fn process_compound_ref_mv_candidate(
    candidate: &MbModeInfo,
    cm: &Av1Common,
    rf: &[MvReferenceFrame; 2],
    ref_id: &mut [[IntMv; 2]; 2],
    ref_id_count: &mut [i32; 2],
    ref_diff: &mut [[IntMv; 2]; 2],
    ref_diff_count: &mut [i32; 2],
) {
    for rf_idx in 0..2 {
        let can_rf = candidate.ref_frame[rf_idx];

        for cmp_idx in 0..2 {
            if can_rf == rf[cmp_idx] && ref_id_count[cmp_idx] < 2 {
                ref_id[cmp_idx][ref_id_count[cmp_idx] as usize] = candidate.mv[rf_idx];
                ref_id_count[cmp_idx] += 1;
            } else if can_rf > INTRA_FRAME && ref_diff_count[cmp_idx] < 2 {
                let mut this_mv = candidate.mv[rf_idx];
                if cm.ref_frame_sign_bias[can_rf as usize]
                    != cm.ref_frame_sign_bias[rf[cmp_idx] as usize]
                {
                    let m = this_mv.as_mv();
                    this_mv.set_mv(Mv {
                        row: m.row.wrapping_neg(),
                        col: m.col.wrapping_neg(),
                    });
                }
                ref_diff[cmp_idx][ref_diff_count[cmp_idx] as usize] = this_mv;
                ref_diff_count[cmp_idx] += 1;
            }
        }
    }
}

/// Translation of `process_single_ref_mv_candidate()`.
fn process_single_ref_mv_candidate(
    candidate: &MbModeInfo,
    cm: &Av1Common,
    ref_frame: MvReferenceFrame,
    l: &mut RefMvList,
) {
    for rf_idx in 0..2 {
        if candidate.ref_frame[rf_idx] > INTRA_FRAME {
            let mut this_mv = candidate.mv[rf_idx];
            if cm.ref_frame_sign_bias[candidate.ref_frame[rf_idx] as usize]
                != cm.ref_frame_sign_bias[ref_frame as usize]
            {
                let m = this_mv.as_mv();
                this_mv.set_mv(Mv {
                    row: m.row.wrapping_neg(),
                    col: m.col.wrapping_neg(),
                });
            }
            let mut stack_idx = 0;
            while stack_idx < *l.refmv_count as usize {
                let stack_mv = l.ref_mv_stack[stack_idx].this_mv;
                if this_mv.as_int == stack_mv.as_int {
                    break;
                }
                stack_idx += 1;
            }

            if stack_idx == *l.refmv_count as usize {
                l.ref_mv_stack[stack_idx].this_mv = this_mv;

                // TODO(jingning): Set an arbitrary small number here. The weight
                // doesn't matter as long as it is properly initialized.
                l.ref_mv_weight[stack_idx] = 2;
                *l.refmv_count += 1;
            }
        }
    }
}

/// Translation of `setup_ref_mv_list()` (`mv_ref_list` is always NULL in
/// the encoder).
#[allow(clippy::too_many_arguments)]
fn setup_ref_mv_list(
    cm: &Av1Common,
    xd: &MacroBlockD,
    ref_frame: MvReferenceFrame,
    l: &mut RefMvList,
    gm_mv_candidates: &[IntMv; 2],
    mi_row: i32,
    mi_col: i32,
    mode_context: &mut [i16],
) {
    let bs = (xd.width as i32).max(xd.height as i32);
    let has_tr = has_top_right(cm, xd, mi_row, mi_col, bs);
    let mut rf = [NONE_FRAME; 2];

    let tile = &xd.tile;
    let mut max_row_offset = 0;
    let mut max_col_offset = 0;
    let row_adj = ((xd.height as i32) < MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32
        && (mi_row & 0x01) != 0) as i32;
    let col_adj = ((xd.width as i32) < MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32
        && (mi_col & 0x01) != 0) as i32;
    let mut processed_rows = 0;
    let mut processed_cols = 0;

    av1_set_ref_frame(&mut rf, ref_frame);
    mode_context[ref_frame as usize] = 0;
    *l.refmv_count = 0;

    // Find valid maximum row/col offset.
    if xd.up_available {
        max_row_offset = -(MVREF_ROW_COLS << 1) + row_adj;

        if (xd.height as i32) < MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32 {
            max_row_offset = -(2 << 1) + row_adj;
        }

        max_row_offset = find_valid_row_offset(tile, mi_row, max_row_offset);
    }

    if xd.left_available {
        max_col_offset = -(MVREF_ROW_COLS << 1) + col_adj;

        if (xd.width as i32) < MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32 {
            max_col_offset = -(2 << 1) + col_adj;
        }

        max_col_offset = find_valid_col_offset(tile, mi_col, max_col_offset);
    }

    let mut col_match_count: u8 = 0;
    let mut row_match_count: u8 = 0;
    let mut newmv_count: u8 = 0;

    // Scan the first above row mode info. row_offset = -1;
    if max_row_offset.abs() >= 1 {
        scan_row_mbmi(
            cm,
            xd,
            mi_col,
            &rf,
            -1,
            l,
            &mut row_match_count,
            &mut newmv_count,
            gm_mv_candidates,
            max_row_offset,
            &mut processed_rows,
        );
    }
    // Scan the first left column mode info. col_offset = -1;
    if max_col_offset.abs() >= 1 {
        scan_col_mbmi(
            cm,
            xd,
            mi_row,
            &rf,
            -1,
            l,
            &mut col_match_count,
            &mut newmv_count,
            gm_mv_candidates,
            max_col_offset,
            &mut processed_cols,
        );
    }
    // Check top-right boundary
    if has_tr {
        scan_blk_mbmi(
            cm,
            xd,
            mi_row,
            mi_col,
            &rf,
            -1,
            xd.width as i32,
            l,
            &mut row_match_count,
            &mut newmv_count,
            gm_mv_candidates,
        );
    }

    let nearest_match = (row_match_count > 0) as u8 + (col_match_count > 0) as u8;
    let nearest_refmv_count = *l.refmv_count;

    // TODO(yunqing): for comp_search, do it for all 3 cases.
    for idx in 0..nearest_refmv_count as usize {
        l.ref_mv_weight[idx] = l.ref_mv_weight[idx].wrapping_add(REF_CAT_LEVEL as u16);
    }

    if cm.features.allow_ref_frame_mvs {
        let mut is_available = false;
        let voffset = (MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32).max(xd.height as i32);
        let hoffset = (MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32).max(xd.width as i32);
        let blk_row_end = (xd.height as i32).min(MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32);
        let blk_col_end = (xd.width as i32).min(MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32);

        let tpl_sample_pos: [[i32; 2]; 3] =
            [[voffset, -2], [voffset, hoffset], [voffset - 2, hoffset]];
        let allow_extension = (xd.height as i32 >= MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32)
            && ((xd.height as i32) < MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32)
            && (xd.width as i32 >= MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32)
            && ((xd.width as i32) < MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32);

        let step_h = if xd.height as i32 >= MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32 {
            MI_SIZE_HIGH[BLOCK_16X16 as usize] as i32
        } else {
            MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32
        };
        let step_w = if xd.width as i32 >= MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32 {
            MI_SIZE_WIDE[BLOCK_16X16 as usize] as i32
        } else {
            MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32
        };

        let mut blk_row = 0;
        while blk_row < blk_row_end {
            let mut blk_col = 0;
            while blk_col < blk_col_end {
                let ret = add_tpl_ref_mv(
                    cm,
                    xd,
                    mi_row,
                    mi_col,
                    ref_frame,
                    blk_row,
                    blk_col,
                    gm_mv_candidates,
                    l,
                    mode_context,
                );
                if blk_row == 0 && blk_col == 0 {
                    is_available = ret;
                }
                blk_col += step_w;
            }
            blk_row += step_h;
        }

        if !is_available {
            mode_context[ref_frame as usize] |= 1 << GLOBALMV_OFFSET;
        }

        let mut i = 0;
        while i < 3 && allow_extension {
            let blk_row = tpl_sample_pos[i][0];
            let blk_col = tpl_sample_pos[i][1];

            if check_sb_border(mi_row, mi_col, blk_row, blk_col) {
                add_tpl_ref_mv(
                    cm,
                    xd,
                    mi_row,
                    mi_col,
                    ref_frame,
                    blk_row,
                    blk_col,
                    gm_mv_candidates,
                    l,
                    mode_context,
                );
            }
            i += 1;
        }
    }

    let mut dummy_newmv_count: u8 = 0;

    // Scan the second outer area.
    scan_blk_mbmi(
        cm,
        xd,
        mi_row,
        mi_col,
        &rf,
        -1,
        -1,
        l,
        &mut row_match_count,
        &mut dummy_newmv_count,
        gm_mv_candidates,
    );

    for idx in 2..=MVREF_ROW_COLS {
        let row_offset = -(idx << 1) + 1 + row_adj;
        let col_offset = -(idx << 1) + 1 + col_adj;

        if row_offset.abs() <= max_row_offset.abs() && row_offset.abs() > processed_rows {
            scan_row_mbmi(
                cm,
                xd,
                mi_col,
                &rf,
                row_offset,
                l,
                &mut row_match_count,
                &mut dummy_newmv_count,
                gm_mv_candidates,
                max_row_offset,
                &mut processed_rows,
            );
        }

        if col_offset.abs() <= max_col_offset.abs() && col_offset.abs() > processed_cols {
            scan_col_mbmi(
                cm,
                xd,
                mi_row,
                &rf,
                col_offset,
                l,
                &mut col_match_count,
                &mut dummy_newmv_count,
                gm_mv_candidates,
                max_col_offset,
                &mut processed_cols,
            );
        }
    }

    let ref_match_count = (row_match_count > 0) as u8 + (col_match_count > 0) as u8;

    let mc = &mut mode_context[ref_frame as usize];
    match nearest_match {
        0 => {
            if ref_match_count >= 1 {
                *mc |= 1;
            }
            if ref_match_count == 1 {
                *mc |= 1 << REFMV_OFFSET;
            } else if ref_match_count >= 2 {
                *mc |= 2 << REFMV_OFFSET;
            }
        }
        1 => {
            *mc |= if newmv_count > 0 { 2 } else { 3 };
            if ref_match_count == 1 {
                *mc |= 3 << REFMV_OFFSET;
            } else if ref_match_count >= 2 {
                *mc |= 4 << REFMV_OFFSET;
            }
        }
        _ => {
            if newmv_count >= 1 {
                *mc |= 4;
            } else {
                *mc |= 5;
            }

            *mc |= 5 << REFMV_OFFSET;
        }
    }

    // Rank the likelihood and assign nearest and near mvs.
    let mut len = nearest_refmv_count as usize;
    while len > 0 {
        let mut nr_len = 0;
        for idx in 1..len {
            if l.ref_mv_weight[idx - 1] < l.ref_mv_weight[idx] {
                l.ref_mv_stack.swap(idx - 1, idx);
                l.ref_mv_weight.swap(idx - 1, idx);
                nr_len = idx;
            }
        }
        len = nr_len;
    }

    len = *l.refmv_count as usize;
    while len > nearest_refmv_count as usize {
        let mut nr_len = nearest_refmv_count as usize;
        for idx in nearest_refmv_count as usize + 1..len {
            if l.ref_mv_weight[idx - 1] < l.ref_mv_weight[idx] {
                l.ref_mv_stack.swap(idx - 1, idx);
                l.ref_mv_weight.swap(idx - 1, idx);
                nr_len = idx;
            }
        }
        len = nr_len;
    }

    let mut mi_width = (MI_SIZE_WIDE[BLOCK_64X64 as usize] as i32).min(xd.width as i32);
    mi_width = mi_width.min(cm.mi_params.mi_cols - mi_col);
    let mut mi_height = (MI_SIZE_HIGH[BLOCK_64X64 as usize] as i32).min(xd.height as i32);
    mi_height = mi_height.min(cm.mi_params.mi_rows - mi_row);
    let mi_size = mi_width.min(mi_height);
    let mi_stride = xd.mi_stride;
    if rf[1] > NONE_FRAME {
        // TODO(jingning, yunqing): Refactor and consolidate the compound and
        // single reference frame modes. Reduce unnecessary redundancy.
        if (*l.refmv_count as i32) < MAX_MV_REF_CANDIDATES {
            let mut ref_id = [[IntMv::default(); 2]; 2];
            let mut ref_diff = [[IntMv::default(); 2]; 2];
            let mut ref_id_count = [0i32; 2];
            let mut ref_diff_count = [0i32; 2];

            let mut idx = 0;
            while max_row_offset.abs() >= 1 && idx < mi_size {
                let candidate = cand(xd, (-mi_stride + idx) as isize);
                process_compound_ref_mv_candidate(
                    candidate,
                    cm,
                    &rf,
                    &mut ref_id,
                    &mut ref_id_count,
                    &mut ref_diff,
                    &mut ref_diff_count,
                );
                idx += MI_SIZE_WIDE[candidate.bsize as usize] as i32;
            }

            idx = 0;
            while max_col_offset.abs() >= 1 && idx < mi_size {
                let candidate = cand(xd, (idx * mi_stride - 1) as isize);
                process_compound_ref_mv_candidate(
                    candidate,
                    cm,
                    &rf,
                    &mut ref_id,
                    &mut ref_id_count,
                    &mut ref_diff,
                    &mut ref_diff_count,
                );
                idx += MI_SIZE_HIGH[candidate.bsize as usize] as i32;
            }

            // Build up the compound mv predictor
            let mut comp_list = [[IntMv::default(); 2]; MAX_MV_REF_CANDIDATES as usize];

            for idx in 0..2 {
                let mut comp_idx = 0usize;
                let mut list_idx = 0;
                while list_idx < ref_id_count[idx] && comp_idx < MAX_MV_REF_CANDIDATES as usize {
                    comp_list[comp_idx][idx] = ref_id[idx][list_idx as usize];
                    list_idx += 1;
                    comp_idx += 1;
                }
                list_idx = 0;
                while list_idx < ref_diff_count[idx] && comp_idx < MAX_MV_REF_CANDIDATES as usize {
                    comp_list[comp_idx][idx] = ref_diff[idx][list_idx as usize];
                    list_idx += 1;
                    comp_idx += 1;
                }
                while comp_idx < MAX_MV_REF_CANDIDATES as usize {
                    comp_list[comp_idx][idx] = gm_mv_candidates[idx];
                    comp_idx += 1;
                }
            }

            if *l.refmv_count != 0 {
                debug_assert!(*l.refmv_count == 1);
                let n = *l.refmv_count as usize;
                if comp_list[0][0].as_int == l.ref_mv_stack[0].this_mv.as_int
                    && comp_list[0][1].as_int == l.ref_mv_stack[0].comp_mv.as_int
                {
                    l.ref_mv_stack[n].this_mv = comp_list[1][0];
                    l.ref_mv_stack[n].comp_mv = comp_list[1][1];
                } else {
                    l.ref_mv_stack[n].this_mv = comp_list[0][0];
                    l.ref_mv_stack[n].comp_mv = comp_list[0][1];
                }
                l.ref_mv_weight[n] = 2;
                *l.refmv_count += 1;
            } else {
                for c in &comp_list {
                    let n = *l.refmv_count as usize;
                    l.ref_mv_stack[n].this_mv = c[0];
                    l.ref_mv_stack[n].comp_mv = c[1];
                    l.ref_mv_weight[n] = 2;
                    *l.refmv_count += 1;
                }
            }
        }

        debug_assert!(*l.refmv_count >= 2);

        let (bw, bh) = (
            (xd.width as i32) << MI_SIZE_LOG2,
            (xd.height as i32) << MI_SIZE_LOG2,
        );
        for idx in 0..*l.refmv_count as usize {
            let mut m = l.ref_mv_stack[idx].this_mv.as_mv();
            clamp_mv_ref(&mut m, bw, bh, xd);
            l.ref_mv_stack[idx].this_mv.set_mv(m);
            let mut m = l.ref_mv_stack[idx].comp_mv.as_mv();
            clamp_mv_ref(&mut m, bw, bh, xd);
            l.ref_mv_stack[idx].comp_mv.set_mv(m);
        }
    } else {
        // Handle single reference frame extension
        let mut idx = 0;
        while max_row_offset.abs() >= 1
            && idx < mi_size
            && (*l.refmv_count as i32) < MAX_MV_REF_CANDIDATES
        {
            let candidate = cand(xd, (-mi_stride + idx) as isize);
            process_single_ref_mv_candidate(candidate, cm, ref_frame, l);
            idx += MI_SIZE_WIDE[candidate.bsize as usize] as i32;
        }

        idx = 0;
        while max_col_offset.abs() >= 1
            && idx < mi_size
            && (*l.refmv_count as i32) < MAX_MV_REF_CANDIDATES
        {
            let candidate = cand(xd, (idx * mi_stride - 1) as isize);
            process_single_ref_mv_candidate(candidate, cm, ref_frame, l);
            idx += MI_SIZE_HIGH[candidate.bsize as usize] as i32;
        }

        let (bw, bh) = (
            (xd.width as i32) << MI_SIZE_LOG2,
            (xd.height as i32) << MI_SIZE_LOG2,
        );
        for idx in 0..*l.refmv_count as usize {
            let mut m = l.ref_mv_stack[idx].this_mv.as_mv();
            clamp_mv_ref(&mut m, bw, bh, xd);
            l.ref_mv_stack[idx].this_mv.set_mv(m);
        }

        // (mv_ref_list is NULL)
    }
}

/// Translation of `av1_find_mv_refs()` with a NULL `mv_ref_list`: the
/// stack and weights are the rows of `ref_frame` (`xd->ref_mv_stack` and
/// `xd->weight` in the encoder).
#[allow(clippy::too_many_arguments)]
pub(crate) fn av1_find_mv_refs(
    cm: &Av1Common,
    xd: &MacroBlockD,
    bsize: BlockSize,
    ref_frame: MvReferenceFrame,
    ref_mv_count: &mut [u8; MODE_CTX_REF_FRAMES as usize],
    ref_mv_stack: &mut [CandidateMv; MAX_REF_MV_STACK_SIZE as usize],
    ref_mv_weight: &mut [u16; MAX_REF_MV_STACK_SIZE as usize],
    global_mvs: Option<&mut [IntMv; REF_FRAMES as usize]>,
    mode_context: &mut [i16],
) {
    let mi_row = xd.mi_row;
    let mi_col = xd.mi_col;
    let mut gm_mv = [IntMv::default(); 2];

    if ref_frame == INTRA_FRAME {
        gm_mv[0].as_int = 0;
        gm_mv[1].as_int = 0;
        if let Some(g) = global_mvs {
            g[ref_frame as usize].as_int = INVALID_MV;
        }
    } else {
        let allow_high_precision_mv = cm.features.allow_high_precision_mv;
        let force_integer_mv = cm.features.cur_frame_force_integer_mv;
        if ref_frame < REF_FRAMES {
            gm_mv[0] = gm_get_motion_vector(
                &cm.global_motion[ref_frame as usize],
                allow_high_precision_mv,
                bsize,
                mi_col,
                mi_row,
                force_integer_mv,
            );
            gm_mv[1].as_int = 0;
            if let Some(g) = global_mvs {
                g[ref_frame as usize] = gm_mv[0];
            }
        } else {
            let mut rf = [NONE_FRAME; 2];
            av1_set_ref_frame(&mut rf, ref_frame);
            gm_mv[0] = gm_get_motion_vector(
                &cm.global_motion[rf[0] as usize],
                allow_high_precision_mv,
                bsize,
                mi_col,
                mi_row,
                force_integer_mv,
            );
            gm_mv[1] = gm_get_motion_vector(
                &cm.global_motion[rf[1] as usize],
                allow_high_precision_mv,
                bsize,
                mi_col,
                mi_row,
                force_integer_mv,
            );
        }
    }

    let mut l = RefMvList {
        refmv_count: &mut ref_mv_count[ref_frame as usize],
        ref_mv_stack,
        ref_mv_weight,
    };
    setup_ref_mv_list(
        cm,
        xd,
        ref_frame,
        &mut l,
        &gm_mv,
        mi_row,
        mi_col,
        mode_context,
    );
}

/// Translation of `av1_setup_frame_buf_refs()`.
pub(crate) fn av1_setup_frame_buf_refs(cm: &mut Av1Common) {
    let order_hint = cm.current_frame.order_hint;
    let display_order_hint = cm.current_frame.display_order_hint;
    let pyramid_level = cm.current_frame.pyramid_level;
    {
        let cur = cm.cur_frame_mut();
        cur.order_hint = order_hint;
        cur.display_order_hint = display_order_hint;
        cur.pyramid_level = pyramid_level;
    }
    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        if let Some(buf) = get_ref_frame_buf(cm, ref_frame) {
            let (oh, doh) = {
                let b = &cm.buffer_pool.frame_bufs[buf];
                (b.order_hint, b.display_order_hint)
            };
            let cur = cm.cur_frame_mut();
            cur.ref_order_hints[(ref_frame - LAST_FRAME) as usize] = oh;
            cur.ref_display_order_hint[(ref_frame - LAST_FRAME) as usize] = doh;
        }
    }
}

/// Translation of `av1_setup_frame_sign_bias()`.
pub(crate) fn av1_setup_frame_sign_bias(cm: &mut Av1Common) {
    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        let buf = get_ref_frame_buf(cm, ref_frame);
        if cm.seq_params.order_hint_info.enable_order_hint != 0 {
            if let Some(buf) = buf {
                let ref_order_hint = cm.buffer_pool.frame_bufs[buf].order_hint as i32;
                cm.ref_frame_sign_bias[ref_frame as usize] = if get_relative_dist(
                    &cm.seq_params.order_hint_info,
                    ref_order_hint,
                    cm.current_frame.order_hint as i32,
                ) <= 0
                {
                    0
                } else {
                    1
                };
                continue;
            }
        }
        cm.ref_frame_sign_bias[ref_frame as usize] = 0;
    }
}

const MAX_OFFSET_WIDTH: i32 = 64;
const MAX_OFFSET_HEIGHT: i32 = 0;

/// Translation of `get_block_position()`.
fn get_block_position(
    cm: &Av1Common,
    mi_r: &mut i32,
    mi_c: &mut i32,
    blk_row: i32,
    blk_col: i32,
    mv: Mv,
    sign_bias: i32,
) -> bool {
    let base_blk_row = (blk_row >> 3) << 3;
    let base_blk_col = (blk_col >> 3) << 3;

    let mv_row = mv.row as i32;
    let mv_col = mv.col as i32;
    let row_offset = if mv_row >= 0 {
        mv_row >> (4 + MI_SIZE_LOG2)
    } else {
        -((-mv_row) >> (4 + MI_SIZE_LOG2))
    };

    let col_offset = if mv_col >= 0 {
        mv_col >> (4 + MI_SIZE_LOG2)
    } else {
        -((-mv_col) >> (4 + MI_SIZE_LOG2))
    };

    let row = if sign_bias == 1 {
        blk_row - row_offset
    } else {
        blk_row + row_offset
    };
    let col = if sign_bias == 1 {
        blk_col - col_offset
    } else {
        blk_col + col_offset
    };

    if row < 0
        || row >= (cm.mi_params.mi_rows >> 1)
        || col < 0
        || col >= (cm.mi_params.mi_cols >> 1)
    {
        return false;
    }

    if row < base_blk_row - (MAX_OFFSET_HEIGHT >> 3)
        || row >= base_blk_row + 8 + (MAX_OFFSET_HEIGHT >> 3)
        || col < base_blk_col - (MAX_OFFSET_WIDTH >> 3)
        || col >= base_blk_col + 8 + (MAX_OFFSET_WIDTH >> 3)
    {
        return false;
    }

    *mi_r = row;
    *mi_c = col;

    true
}

// Note: motion_filed_projection finds motion vectors of current frame's
// reference frame, and projects them to current frame. To make it clear,
// let's call current frame's reference frame as start frame.
// Call Start frame's reference frames as reference frames.
// Call ref_offset as frame distances between start frame and its reference
// frames.
/// Translation of `motion_field_projection()`.
fn motion_field_projection(cm: &mut Av1Common, start_frame: MvReferenceFrame, dir: i32) -> bool {
    let mut ref_offset = [0i32; REF_FRAMES as usize];

    let Some(start_frame_buf) = get_ref_frame_buf(cm, start_frame) else {
        return false;
    };
    let sfb = &cm.buffer_pool.frame_bufs[start_frame_buf];

    if sfb.frame_type == KEY_FRAME || sfb.frame_type == INTRA_ONLY_FRAME {
        return false;
    }

    if sfb.mi_rows != cm.mi_params.mi_rows || sfb.mi_cols != cm.mi_params.mi_cols {
        return false;
    }

    let start_frame_order_hint = sfb.order_hint as i32;
    let ref_order_hints = sfb.ref_order_hints;
    let cur_order_hint = cm.cur_frame().order_hint as i32;
    let mut start_to_current_frame_offset = get_relative_dist(
        &cm.seq_params.order_hint_info,
        start_frame_order_hint,
        cur_order_hint,
    );

    for rf in LAST_FRAME..=INTER_REFS_PER_FRAME {
        ref_offset[rf as usize] = get_relative_dist(
            &cm.seq_params.order_hint_info,
            start_frame_order_hint,
            ref_order_hints[(rf - LAST_FRAME) as usize] as i32,
        );
    }

    if dir == 2 {
        start_to_current_frame_offset = -start_to_current_frame_offset;
    }

    let mvs_rows = (cm.mi_params.mi_rows + 1) >> 1;
    let mvs_cols = (cm.mi_params.mi_cols + 1) >> 1;
    let tpl_stride = cm.mi_params.mi_stride >> 1;
    let mut tpl_mvs = std::mem::take(&mut cm.tpl_mvs);
    {
        let mv_ref_base = &cm.buffer_pool.frame_bufs[start_frame_buf].mvs;
        for blk_row in 0..mvs_rows {
            for blk_col in 0..mvs_cols {
                let mv_ref = &mv_ref_base[(blk_row * mvs_cols + blk_col) as usize];
                let fwd_mv = mv_ref.mv.as_mv();

                if mv_ref.ref_frame > INTRA_FRAME {
                    let mut this_mv = Mv::default();
                    let mut mi_r = 0;
                    let mut mi_c = 0;
                    let ref_frame_offset = ref_offset[mv_ref.ref_frame as usize];

                    let mut pos_valid = ref_frame_offset.abs() <= MAX_FRAME_DISTANCE
                        && ref_frame_offset > 0
                        && start_to_current_frame_offset.abs() <= MAX_FRAME_DISTANCE;

                    if pos_valid {
                        get_mv_projection(
                            &mut this_mv,
                            fwd_mv,
                            start_to_current_frame_offset,
                            ref_frame_offset,
                        );
                        pos_valid = get_block_position(
                            cm,
                            &mut mi_r,
                            &mut mi_c,
                            blk_row,
                            blk_col,
                            this_mv,
                            dir >> 1,
                        );
                    }

                    if pos_valid {
                        let mi_offset = (mi_r * tpl_stride + mi_c) as usize;

                        tpl_mvs[mi_offset].mfmv0.set_mv(Mv {
                            row: fwd_mv.row,
                            col: fwd_mv.col,
                        });
                        tpl_mvs[mi_offset].ref_frame_offset = ref_frame_offset as u8;
                    }
                }
            }
        }
    }
    cm.tpl_mvs = tpl_mvs;

    true
}

// cm->ref_frame_side is calculated here, and will be used in
// av1_copy_frame_mvs() to affect how mvs are copied.
/// Translation of `av1_calculate_ref_frame_side()`.
pub(crate) fn av1_calculate_ref_frame_side(cm: &mut Av1Common) {
    cm.ref_frame_side = [0; REF_FRAMES as usize];
    if cm.seq_params.order_hint_info.enable_order_hint == 0 {
        return;
    }

    let cur_order_hint = cm.cur_frame().order_hint as i32;

    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        let buf = get_ref_frame_buf(cm, ref_frame);
        let mut order_hint = 0;

        if let Some(buf) = buf {
            order_hint = cm.buffer_pool.frame_bufs[buf].order_hint as i32;
        }

        if get_relative_dist(&cm.seq_params.order_hint_info, order_hint, cur_order_hint) > 0 {
            cm.ref_frame_side[ref_frame as usize] = 1;
        } else if order_hint == cur_order_hint {
            cm.ref_frame_side[ref_frame as usize] = -1;
        }
    }
}

/// Translation of `av1_setup_motion_field()`.
pub(crate) fn av1_setup_motion_field(cm: &mut Av1Common) {
    if cm.seq_params.order_hint_info.enable_order_hint == 0 {
        return;
    }

    let size = ((cm.mi_params.mi_rows + MAX_MIB_SIZE) >> 1) * (cm.mi_params.mi_stride >> 1);
    for t in cm.tpl_mvs[..size as usize].iter_mut() {
        t.mfmv0.as_int = INVALID_MV;
        t.ref_frame_offset = 0;
    }

    let cur_order_hint = cm.cur_frame().order_hint as i32;
    let mut ref_buf: [Option<usize>; INTER_REFS_PER_FRAME as usize] =
        [None; INTER_REFS_PER_FRAME as usize];
    let mut ref_order_hint = [0i32; INTER_REFS_PER_FRAME as usize];

    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        let ref_idx = (ref_frame - LAST_FRAME) as usize;
        let buf = get_ref_frame_buf(cm, ref_frame);
        let mut order_hint = 0;

        if let Some(b) = buf {
            order_hint = cm.buffer_pool.frame_bufs[b].order_hint as i32;
        }

        ref_buf[ref_idx] = buf;
        ref_order_hint[ref_idx] = order_hint;
    }

    let mut ref_stamp = MFMV_STACK_SIZE - 1;

    if let Some(lst) = ref_buf[(LAST_FRAME - LAST_FRAME) as usize] {
        let alt_of_lst_order_hint = cm.buffer_pool.frame_bufs[lst].ref_order_hints
            [(ALTREF_FRAME - LAST_FRAME) as usize] as i32;

        let is_lst_overlay =
            alt_of_lst_order_hint == ref_order_hint[(GOLDEN_FRAME - LAST_FRAME) as usize];
        if !is_lst_overlay {
            motion_field_projection(cm, LAST_FRAME, 2);
        }
        ref_stamp -= 1;
    }

    let ohi = cm.seq_params.order_hint_info;
    if get_relative_dist(
        &ohi,
        ref_order_hint[(BWDREF_FRAME - LAST_FRAME) as usize],
        cur_order_hint,
    ) > 0
        && motion_field_projection(cm, BWDREF_FRAME, 0)
    {
        ref_stamp -= 1;
    }

    if get_relative_dist(
        &ohi,
        ref_order_hint[(ALTREF2_FRAME - LAST_FRAME) as usize],
        cur_order_hint,
    ) > 0
        && motion_field_projection(cm, ALTREF2_FRAME, 0)
    {
        ref_stamp -= 1;
    }

    if get_relative_dist(
        &ohi,
        ref_order_hint[(ALTREF_FRAME - LAST_FRAME) as usize],
        cur_order_hint,
    ) > 0
        && ref_stamp >= 0
        && motion_field_projection(cm, ALTREF_FRAME, 0)
    {
        ref_stamp -= 1;
    }

    if ref_stamp >= 0 {
        motion_field_projection(cm, LAST2_FRAME, 2);
    }
}

/// Translation of `record_samples()`.
#[inline]
fn record_samples(
    mbmi: &MbModeInfo,
    pts: &mut [i32],
    pts_inref: &mut [i32],
    row_offset: i32,
    sign_r: i32,
    col_offset: i32,
    sign_c: i32,
) {
    let bw = BLOCK_SIZE_WIDE[mbmi.bsize as usize] as i32;
    let bh = BLOCK_SIZE_HIGH[mbmi.bsize as usize] as i32;
    let x = col_offset * MI_SIZE + sign_c * bw / 2 - 1;
    let y = row_offset * MI_SIZE + sign_r * bh / 2 - 1;

    pts[0] = get_mv_subpel(x);
    pts[1] = get_mv_subpel(y);
    pts_inref[0] = pts[0] + mbmi.mv[0].as_mv().col as i32;
    pts_inref[1] = pts[1] + mbmi.mv[0].as_mv().row as i32;
}

pub(crate) const LEAST_SQUARES_SAMPLES_MAX_BITS: i32 = 3;
pub(crate) const LEAST_SQUARES_SAMPLES_MAX: usize = 1 << LEAST_SQUARES_SAMPLES_MAX_BITS;

// Note: Samples returned are at 1/8-pel precision
// Sample are the neighbor block center point's coordinates relative to the
// left-top pixel of current block.
/// Translation of `av1_findSamples()`: `pts` and `pts_inref` hold
/// `2 * LEAST_SQUARES_SAMPLES_MAX` values.
pub(crate) fn av1_find_samples(
    cm: &Av1Common,
    xd: &MacroBlockD,
    pts: &mut [i32],
    pts_inref: &mut [i32],
) -> u8 {
    let mbmi0 = xd.mbmi();
    let ref_frame = mbmi0.ref_frame[0];
    let up_available = xd.up_available;
    let left_available = xd.left_available;
    let mut np: u8 = 0;
    let mut do_tl = true;
    let mut do_tr = true;
    let mi_stride = xd.mi_stride;
    let mi_row = xd.mi_row;
    let mi_col = xd.mi_col;
    let mut p = 0usize;
    let max = LEAST_SQUARES_SAMPLES_MAX as u8;

    // scan the nearest above rows
    if up_available {
        let mi_row_offset = -1;
        let mut mbmi = cand(xd, (mi_row_offset * mi_stride) as isize);
        let mut superblock_width = MI_SIZE_WIDE[mbmi.bsize as usize];

        if xd.width <= superblock_width {
            // Handle "current block width <= above block width" case.
            let col_offset = -mi_col % superblock_width as i32;

            if col_offset < 0 {
                do_tl = false;
            }
            if col_offset + superblock_width as i32 > xd.width as i32 {
                do_tr = false;
            }

            if mbmi.ref_frame[0] == ref_frame && mbmi.ref_frame[1] == NONE_FRAME {
                record_samples(
                    mbmi,
                    &mut pts[p..],
                    &mut pts_inref[p..],
                    0,
                    -1,
                    col_offset,
                    1,
                );
                p += 2;
                np += 1;
                if np >= max {
                    return max;
                }
            }
        } else {
            // Handle "current block width > above block width" case.
            let mut i = 0;
            while i < (xd.width as i32).min(cm.mi_params.mi_cols - mi_col) {
                mbmi = cand(xd, (i + mi_row_offset * mi_stride) as isize);
                superblock_width = MI_SIZE_WIDE[mbmi.bsize as usize];

                if mbmi.ref_frame[0] == ref_frame && mbmi.ref_frame[1] == NONE_FRAME {
                    record_samples(mbmi, &mut pts[p..], &mut pts_inref[p..], 0, -1, i, 1);
                    p += 2;
                    np += 1;
                    if np >= max {
                        return max;
                    }
                }
                i += superblock_width as i32;
            }
        }
    }
    debug_assert!(np <= max);

    // scan the nearest left columns
    if left_available {
        let mi_col_offset = -1;
        let mut mbmi = cand(xd, mi_col_offset);
        let mut superblock_height = MI_SIZE_HIGH[mbmi.bsize as usize];

        if xd.height <= superblock_height {
            // Handle "current block height <= above block height" case.
            let row_offset = -mi_row % superblock_height as i32;

            if row_offset < 0 {
                do_tl = false;
            }

            if mbmi.ref_frame[0] == ref_frame && mbmi.ref_frame[1] == NONE_FRAME {
                record_samples(
                    mbmi,
                    &mut pts[p..],
                    &mut pts_inref[p..],
                    row_offset,
                    1,
                    0,
                    -1,
                );
                p += 2;
                np += 1;
                if np >= max {
                    return max;
                }
            }
        } else {
            // Handle "current block height > above block height" case.
            let mut i = 0;
            while i < (xd.height as i32).min(cm.mi_params.mi_rows - mi_row) {
                mbmi = cand(xd, (mi_col_offset as i32 + i * mi_stride) as isize);
                superblock_height = MI_SIZE_HIGH[mbmi.bsize as usize];

                if mbmi.ref_frame[0] == ref_frame && mbmi.ref_frame[1] == NONE_FRAME {
                    record_samples(mbmi, &mut pts[p..], &mut pts_inref[p..], i, 1, 0, -1);
                    p += 2;
                    np += 1;
                    if np >= max {
                        return max;
                    }
                }
                i += superblock_height as i32;
            }
        }
    }
    debug_assert!(np <= max);

    // Top-left block
    if do_tl && left_available && up_available {
        let mi_row_offset = -1;
        let mi_col_offset = -1;
        let mbmi = cand(xd, (mi_col_offset + mi_row_offset * mi_stride) as isize);

        if mbmi.ref_frame[0] == ref_frame && mbmi.ref_frame[1] == NONE_FRAME {
            record_samples(mbmi, &mut pts[p..], &mut pts_inref[p..], 0, -1, 0, -1);
            p += 2;
            np += 1;
            if np >= max {
                return max;
            }
        }
    }
    debug_assert!(np <= max);

    // Top-right block
    if do_tr
        && has_top_right(
            cm,
            xd,
            mi_row,
            mi_col,
            (xd.width as i32).max(xd.height as i32),
        )
    {
        let trb_pos = Position {
            row: -1,
            col: xd.width as i32,
        };
        let tile = &xd.tile;
        if is_inside(tile, mi_col, mi_row, &trb_pos) {
            let mi_row_offset = -1;
            let mi_col_offset = xd.width as i32;
            let mbmi = cand(xd, (mi_col_offset + mi_row_offset * mi_stride) as isize);

            if mbmi.ref_frame[0] == ref_frame && mbmi.ref_frame[1] == NONE_FRAME {
                record_samples(
                    mbmi,
                    &mut pts[p..],
                    &mut pts_inref[p..],
                    0,
                    -1,
                    xd.width as i32,
                    1,
                );
                np += 1;
                if np >= max {
                    return max;
                }
            }
        }
    }
    debug_assert!(np <= max);

    np
}

/// Translation of `av1_setup_skip_mode_allowed()`.
pub(crate) fn av1_setup_skip_mode_allowed(cm: &mut Av1Common) {
    let order_hint_info = cm.seq_params.order_hint_info;

    {
        let skip_mode_info = &mut cm.current_frame.skip_mode_info;
        skip_mode_info.skip_mode_allowed = 0;
        skip_mode_info.ref_frame_idx_0 = INVALID_IDX;
        skip_mode_info.ref_frame_idx_1 = INVALID_IDX;
    }

    if order_hint_info.enable_order_hint == 0
        || frame_is_intra_only(cm)
        || cm.current_frame.reference_mode == SINGLE_REFERENCE
    {
        return;
    }

    let cur_order_hint = cm.current_frame.order_hint as i32;
    let mut ref_order_hints = [-1i32, i32::MAX];
    let mut ref_idx = [INVALID_IDX, INVALID_IDX];

    // Identify the nearest forward and backward references.
    for i in 0..INTER_REFS_PER_FRAME {
        let Some(buf) = get_ref_frame_buf(cm, LAST_FRAME + i) else {
            continue;
        };

        let ref_order_hint = cm.buffer_pool.frame_bufs[buf].order_hint as i32;
        if get_relative_dist(&order_hint_info, ref_order_hint, cur_order_hint) < 0 {
            // Forward reference
            if ref_order_hints[0] == -1
                || get_relative_dist(&order_hint_info, ref_order_hint, ref_order_hints[0]) > 0
            {
                ref_order_hints[0] = ref_order_hint;
                ref_idx[0] = i;
            }
        } else if get_relative_dist(&order_hint_info, ref_order_hint, cur_order_hint) > 0 {
            // Backward reference
            if ref_order_hints[1] == i32::MAX
                || get_relative_dist(&order_hint_info, ref_order_hint, ref_order_hints[1]) < 0
            {
                ref_order_hints[1] = ref_order_hint;
                ref_idx[1] = i;
            }
        }
    }

    let skip_mode_info = &mut cm.current_frame.skip_mode_info;
    if ref_idx[0] != INVALID_IDX && ref_idx[1] != INVALID_IDX {
        // == Bi-directional prediction ==
        skip_mode_info.skip_mode_allowed = 1;
        skip_mode_info.ref_frame_idx_0 = ref_idx[0].min(ref_idx[1]);
        skip_mode_info.ref_frame_idx_1 = ref_idx[0].max(ref_idx[1]);
    } else if ref_idx[0] != INVALID_IDX && ref_idx[1] == INVALID_IDX {
        // == Forward prediction only ==
        // Identify the second nearest forward reference.
        ref_order_hints[1] = -1;
        for i in 0..INTER_REFS_PER_FRAME {
            let Some(buf) = get_ref_frame_buf(cm, LAST_FRAME + i) else {
                continue;
            };

            let ref_order_hint = cm.buffer_pool.frame_bufs[buf].order_hint as i32;
            if (ref_order_hints[0] != -1
                && get_relative_dist(&order_hint_info, ref_order_hint, ref_order_hints[0]) < 0)
                && (ref_order_hints[1] == -1
                    || get_relative_dist(&order_hint_info, ref_order_hint, ref_order_hints[1]) > 0)
            {
                // Second closest forward reference
                ref_order_hints[1] = ref_order_hint;
                ref_idx[1] = i;
            }
        }
        if ref_order_hints[1] != -1 {
            let skip_mode_info = &mut cm.current_frame.skip_mode_info;
            skip_mode_info.skip_mode_allowed = 1;
            skip_mode_info.ref_frame_idx_0 = ref_idx[0].min(ref_idx[1]);
            skip_mode_info.ref_frame_idx_1 = ref_idx[0].max(ref_idx[1]);
        }
    }
}
