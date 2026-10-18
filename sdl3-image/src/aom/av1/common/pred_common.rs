// Rust translation of the parts of av1/common/pred_common.c and
// av1/common/pred_common.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches (the segmentation and palette contexts are
// left out: both tools are off).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The symbol contexts derived from the neighboring blocks.

use super::av1_common_int::{get_ref_frame_buf, Av1Common};
use super::blockd::{has_second_ref, has_uni_comp_refs, is_inter_block, MacroBlockD, MbModeInfo};
use super::enums::*;
use super::filter::{av1_extract_interp_filter, InterpFilter, SWITCHABLE_FILTERS};
use super::mvref_common::get_relative_dist;

/// `INTER_FILTER_COMP_OFFSET` (filter.h)
pub(crate) const INTER_FILTER_COMP_OFFSET: i32 = SWITCHABLE_FILTERS as i32 + 1;
/// `INTER_FILTER_DIR_OFFSET` (filter.h)
pub(crate) const INTER_FILTER_DIR_OFFSET: i32 = (SWITCHABLE_FILTERS as i32 + 1) * 2;

#[inline]
pub(crate) fn get_comp_index_context(cm: &Av1Common, xd: &MacroBlockD) -> i32 {
    let mbmi = xd.mbmi();
    let bck_buf = get_ref_frame_buf(cm, mbmi.ref_frame[0]);
    let fwd_buf = get_ref_frame_buf(cm, mbmi.ref_frame[1]);
    let mut bck_frame_index = 0;
    let mut fwd_frame_index = 0;
    let cur_frame_index = cm.cur_frame().order_hint as i32;

    if let Some(b) = bck_buf {
        bck_frame_index = cm.buffer_pool.frame_bufs[b].order_hint as i32;
    }
    if let Some(f) = fwd_buf {
        fwd_frame_index = cm.buffer_pool.frame_bufs[f].order_hint as i32;
    }

    let fwd = get_relative_dist(
        &cm.seq_params.order_hint_info,
        fwd_frame_index,
        cur_frame_index,
    )
    .abs();
    let bck = get_relative_dist(
        &cm.seq_params.order_hint_info,
        cur_frame_index,
        bck_frame_index,
    )
    .abs();

    let above_mi = xd.mi_at(xd.above_mbmi);
    let left_mi = xd.mi_at(xd.left_mbmi);

    let mut above_ctx = 0;
    let mut left_ctx = 0;
    let offset = (fwd == bck) as i32;

    if let Some(above_mi) = above_mi {
        if has_second_ref(above_mi) {
            above_ctx = above_mi.compound_idx as i32;
        } else if above_mi.ref_frame[0] == ALTREF_FRAME {
            above_ctx = 1;
        }
    }

    if let Some(left_mi) = left_mi {
        if has_second_ref(left_mi) {
            left_ctx = left_mi.compound_idx as i32;
        } else if left_mi.ref_frame[0] == ALTREF_FRAME {
            left_ctx = 1;
        }
    }

    above_ctx + left_ctx + 3 * offset
}

#[inline]
pub(crate) fn get_comp_group_idx_context(xd: &MacroBlockD) -> i32 {
    let above_mi = xd.mi_at(xd.above_mbmi);
    let left_mi = xd.mi_at(xd.left_mbmi);
    let mut above_ctx = 0;
    let mut left_ctx = 0;

    if let Some(above_mi) = above_mi {
        if has_second_ref(above_mi) {
            above_ctx = above_mi.comp_group_idx as i32;
        } else if above_mi.ref_frame[0] == ALTREF_FRAME {
            above_ctx = 3;
        }
    }
    if let Some(left_mi) = left_mi {
        if has_second_ref(left_mi) {
            left_ctx = left_mi.comp_group_idx as i32;
        } else if left_mi.ref_frame[0] == ALTREF_FRAME {
            left_ctx = 3;
        }
    }

    5.min(above_ctx + left_ctx)
}

#[inline]
pub(crate) fn av1_get_skip_mode_context(xd: &MacroBlockD) -> i32 {
    let above_mi = xd.mi_at(xd.above_mbmi);
    let left_mi = xd.mi_at(xd.left_mbmi);
    let above_skip_mode = above_mi.map_or(0, |m| m.skip_mode as i32);
    let left_skip_mode = left_mi.map_or(0, |m| m.skip_mode as i32);
    above_skip_mode + left_skip_mode
}

// Returns a context number for the given MB prediction signal
#[inline]
fn get_ref_filter_type(
    ref_mbmi: &MbModeInfo,
    dir: i32,
    ref_frame: MvReferenceFrame,
) -> InterpFilter {
    if ref_mbmi.ref_frame[0] == ref_frame || ref_mbmi.ref_frame[1] == ref_frame {
        av1_extract_interp_filter(ref_mbmi.interp_filters, dir & 0x01)
    } else {
        SWITCHABLE_FILTERS
    }
}

pub(crate) fn av1_get_pred_context_switchable_interp(xd: &MacroBlockD, dir: i32) -> i32 {
    let mbmi = xd.mbmi();
    let ctx_offset = (mbmi.ref_frame[1] > INTRA_FRAME) as i32 * INTER_FILTER_COMP_OFFSET;
    debug_assert!(dir == 0 || dir == 1);
    let ref_frame = mbmi.ref_frame[0];
    // Note:
    // The mode info data structure has a one element border above and to the
    // left of the entries corresponding to real macroblocks.
    // The prediction flags in these dummy entries are initialized to 0.
    let mut filter_type_ctx = ctx_offset + (dir & 0x01) * INTER_FILTER_DIR_OFFSET;
    let mut left_type = SWITCHABLE_FILTERS;
    let mut above_type = SWITCHABLE_FILTERS;

    if xd.left_available {
        left_type = get_ref_filter_type(&xd.mi_alloc[xd.mi_idx(-1) as usize], dir, ref_frame);
    }

    if xd.up_available {
        above_type = get_ref_filter_type(
            &xd.mi_alloc[xd.mi_idx(-(xd.mi_stride as isize)) as usize],
            dir,
            ref_frame,
        );
    }

    if left_type == above_type {
        filter_type_ctx += left_type as i32;
    } else if left_type == SWITCHABLE_FILTERS {
        debug_assert!(above_type != SWITCHABLE_FILTERS);
        filter_type_ctx += above_type as i32;
    } else if above_type == SWITCHABLE_FILTERS {
        debug_assert!(left_type != SWITCHABLE_FILTERS);
        filter_type_ctx += left_type as i32;
    } else {
        filter_type_ctx += SWITCHABLE_FILTERS as i32;
    }

    filter_type_ctx
}

/// The above and left neighbors, where they are available.
#[inline]
fn edges(xd: &MacroBlockD) -> (Option<&MbModeInfo>, Option<&MbModeInfo>) {
    (
        if xd.up_available {
            xd.mi_at(xd.above_mbmi)
        } else {
            None
        },
        if xd.left_available {
            xd.mi_at(xd.left_mbmi)
        } else {
            None
        },
    )
}

// The mode info data structure has a one element border above and to the
// left of the entries corresponding to real macroblocks.
// The prediction flags in these dummy entries are initialized to 0.
// 0 - inter/inter, inter/--, --/inter, --/--
// 1 - intra/inter, inter/intra
// 2 - intra/--, --/intra
// 3 - intra/intra
pub(crate) fn av1_get_intra_inter_context(xd: &MacroBlockD) -> i32 {
    match edges(xd) {
        // both edges available
        (Some(above_mbmi), Some(left_mbmi)) => {
            let above_intra = !is_inter_block(above_mbmi);
            let left_intra = !is_inter_block(left_mbmi);
            if left_intra && above_intra {
                3
            } else {
                (left_intra || above_intra) as i32
            }
        }
        // one edge available
        (Some(edge), None) | (None, Some(edge)) => 2 * (!is_inter_block(edge)) as i32,
        (None, None) => 0,
    }
}

// (CHECK_BACKWARD_REFS() / IS_BACKWARD_REF_FRAME())
#[inline]
fn is_backward_ref_frame(ref_frame: MvReferenceFrame) -> bool {
    (BWDREF_FRAME..=ALTREF_FRAME).contains(&ref_frame)
}

pub(crate) fn av1_get_reference_mode_context(xd: &MacroBlockD) -> i32 {
    // Note:
    // The mode info data structure has a one element border above and to the
    // left of the entries corresponding to real macroblocks.
    // The prediction flags in these dummy entries are initialized to 0.
    let ctx = match edges(xd) {
        // both edges available
        (Some(above_mbmi), Some(left_mbmi)) => {
            if !has_second_ref(above_mbmi) && !has_second_ref(left_mbmi) {
                // neither edge uses comp pred (0/1)
                (is_backward_ref_frame(above_mbmi.ref_frame[0])
                    ^ is_backward_ref_frame(left_mbmi.ref_frame[0])) as i32
            } else if !has_second_ref(above_mbmi) {
                // one of two edges uses comp pred (2/3)
                2 + (is_backward_ref_frame(above_mbmi.ref_frame[0]) || !is_inter_block(above_mbmi))
                    as i32
            } else if !has_second_ref(left_mbmi) {
                // one of two edges uses comp pred (2/3)
                2 + (is_backward_ref_frame(left_mbmi.ref_frame[0]) || !is_inter_block(left_mbmi))
                    as i32
            } else {
                // both edges use comp pred (4)
                4
            }
        }
        // one edge available
        (Some(edge_mbmi), None) | (None, Some(edge_mbmi)) => {
            if !has_second_ref(edge_mbmi) {
                // edge does not use comp pred (0/1)
                is_backward_ref_frame(edge_mbmi.ref_frame[0]) as i32
            } else {
                // edge uses comp pred (3)
                3
            }
        }
        // no edges available (1)
        (None, None) => 1,
    };
    debug_assert!(ctx >= 0 && ctx < COMP_INTER_CONTEXTS as i32);
    ctx
}

pub(crate) fn av1_get_comp_reference_type_context(xd: &MacroBlockD) -> i32 {
    let pred_context = match edges(xd) {
        // both edges available
        (Some(above_mbmi), Some(left_mbmi)) => {
            let above_intra = !is_inter_block(above_mbmi);
            let left_intra = !is_inter_block(left_mbmi);

            if above_intra && left_intra {
                // intra/intra
                2
            } else if above_intra || left_intra {
                // intra/inter
                let inter_mbmi = if above_intra { left_mbmi } else { above_mbmi };

                if !has_second_ref(inter_mbmi) {
                    // single pred
                    2
                } else {
                    // comp pred
                    1 + 2 * has_uni_comp_refs(inter_mbmi) as i32
                }
            } else {
                // inter/inter
                let a_sg = !has_second_ref(above_mbmi);
                let l_sg = !has_second_ref(left_mbmi);
                let frfa = above_mbmi.ref_frame[0];
                let frfl = left_mbmi.ref_frame[0];

                if a_sg && l_sg {
                    // single/single
                    1 + 2 * (!(is_backward_ref_frame(frfa) ^ is_backward_ref_frame(frfl))) as i32
                } else if l_sg || a_sg {
                    // single/comp
                    let uni_rfc = if a_sg {
                        has_uni_comp_refs(left_mbmi)
                    } else {
                        has_uni_comp_refs(above_mbmi)
                    };

                    if !uni_rfc {
                        // comp bidir
                        1
                    } else {
                        // comp unidir
                        3 + (!(is_backward_ref_frame(frfa) ^ is_backward_ref_frame(frfl))) as i32
                    }
                } else {
                    // comp/comp
                    let a_uni_rfc = has_uni_comp_refs(above_mbmi);
                    let l_uni_rfc = has_uni_comp_refs(left_mbmi);

                    if !a_uni_rfc && !l_uni_rfc {
                        // bidir/bidir
                        0
                    } else if !a_uni_rfc || !l_uni_rfc {
                        // unidir/bidir
                        2
                    } else {
                        // unidir/unidir
                        3 + (!((frfa == BWDREF_FRAME) ^ (frfl == BWDREF_FRAME))) as i32
                    }
                }
            }
        }
        // one edge available
        (Some(edge_mbmi), None) | (None, Some(edge_mbmi)) => {
            if !is_inter_block(edge_mbmi) {
                // intra
                2
            } else {
                // inter
                if !has_second_ref(edge_mbmi) {
                    // single pred
                    2
                } else {
                    // comp pred
                    4 * has_uni_comp_refs(edge_mbmi) as i32
                }
            }
        }
        // no edges available
        (None, None) => 2,
    };

    debug_assert!(pred_context >= 0 && pred_context < COMP_REF_TYPE_CONTEXTS as i32);
    pred_context
}

/// The context of a vote between two counts of neighboring references.
#[inline]
fn vote(a: i32, b: i32) -> i32 {
    if a == b {
        1
    } else if a < b {
        0
    } else {
        2
    }
}

/// `xd->neighbors_ref_counts[ref_frame]`
#[inline]
fn count(xd: &MacroBlockD, ref_frame: MvReferenceFrame) -> i32 {
    xd.neighbors_ref_counts[ref_frame as usize] as i32
}

// Returns a context number for the given MB prediction signal
//
// Signal the uni-directional compound reference frame pair as either
// (BWDREF, ALTREF), or (LAST, LAST2) / (LAST, LAST3) / (LAST, GOLDEN),
// conditioning on the pair is known as uni-directional.
//
// 3 contexts: Voting is used to compare the count of forward references with
//             that of backward references from the spatial neighbors.
pub(crate) fn av1_get_pred_context_uni_comp_ref_p(xd: &MacroBlockD) -> i32 {
    // Count of forward references (L, L2, L3, or G)
    let frf_count = count(xd, LAST_FRAME)
        + count(xd, LAST2_FRAME)
        + count(xd, LAST3_FRAME)
        + count(xd, GOLDEN_FRAME);
    // Count of backward references (B or A)
    let brf_count = count(xd, BWDREF_FRAME) + count(xd, ALTREF2_FRAME) + count(xd, ALTREF_FRAME);

    vote(frf_count, brf_count)
}

// Returns a context number for the given MB prediction signal
//
// Signal the uni-directional compound reference frame pair as
// either (LAST, LAST2), or (LAST, LAST3) / (LAST, GOLDEN),
// conditioning on the pair is known as one of the above three.
//
// 3 contexts: Voting is used to compare the count of LAST2_FRAME with the
//             total count of LAST3/GOLDEN from the spatial neighbors.
pub(crate) fn av1_get_pred_context_uni_comp_ref_p1(xd: &MacroBlockD) -> i32 {
    // Count of LAST2
    let last2_count = count(xd, LAST2_FRAME);
    // Count of LAST3 or GOLDEN
    let last3_or_gld_count = count(xd, LAST3_FRAME) + count(xd, GOLDEN_FRAME);

    vote(last2_count, last3_or_gld_count)
}

// Returns a context number for the given MB prediction signal
//
// Signal the uni-directional compound reference frame pair as
// either (LAST, LAST3) or (LAST, GOLDEN),
// conditioning on the pair is known as one of the above two.
//
// 3 contexts: Voting is used to compare the count of LAST3_FRAME with the
//             total count of GOLDEN_FRAME from the spatial neighbors.
pub(crate) fn av1_get_pred_context_uni_comp_ref_p2(xd: &MacroBlockD) -> i32 {
    // Count of LAST3
    let last3_count = count(xd, LAST3_FRAME);
    // Count of GOLDEN
    let gld_count = count(xd, GOLDEN_FRAME);

    vote(last3_count, gld_count)
}

// == Common context functions for both comp and single ref ==
//
// Obtain contexts to signal a reference frame to be either LAST/LAST2 or
// LAST3/GOLDEN.
fn get_pred_context_ll2_or_l3gld(xd: &MacroBlockD) -> i32 {
    // Count of LAST + LAST2
    let last_last2_count = count(xd, LAST_FRAME) + count(xd, LAST2_FRAME);
    // Count of LAST3 + GOLDEN
    let last3_gld_count = count(xd, LAST3_FRAME) + count(xd, GOLDEN_FRAME);

    vote(last_last2_count, last3_gld_count)
}

// Obtain contexts to signal a reference frame to be either LAST or LAST2.
fn get_pred_context_last_or_last2(xd: &MacroBlockD) -> i32 {
    // Count of LAST
    let last_count = count(xd, LAST_FRAME);
    // Count of LAST2
    let last2_count = count(xd, LAST2_FRAME);

    vote(last_count, last2_count)
}

// Obtain contexts to signal a reference frame to be either LAST3 or GOLDEN.
fn get_pred_context_last3_or_gld(xd: &MacroBlockD) -> i32 {
    // Count of LAST3
    let last3_count = count(xd, LAST3_FRAME);
    // Count of GOLDEN
    let gld_count = count(xd, GOLDEN_FRAME);

    vote(last3_count, gld_count)
}

// Obtain contexts to signal a reference frame be either BWDREF/ALTREF2, or
// ALTREF.
fn get_pred_context_brfarf2_or_arf(xd: &MacroBlockD) -> i32 {
    // Counts of BWDREF, ALTREF2, or ALTREF frames (B, A2, or A)
    let brfarf2_count = count(xd, BWDREF_FRAME) + count(xd, ALTREF2_FRAME);
    let arf_count = count(xd, ALTREF_FRAME);

    vote(brfarf2_count, arf_count)
}

// Obtain contexts to signal a reference frame be either BWDREF or ALTREF2.
fn get_pred_context_brf_or_arf2(xd: &MacroBlockD) -> i32 {
    // Count of BWDREF frames (B)
    let brf_count = count(xd, BWDREF_FRAME);
    // Count of ALTREF2 frames (A2)
    let arf2_count = count(xd, ALTREF2_FRAME);

    vote(brf_count, arf2_count)
}

// == Context functions for comp ref ==
//
// Returns a context number for the given MB prediction signal
// Signal the first reference frame for a compound mode be either
// GOLDEN/LAST3, or LAST/LAST2.
pub(crate) fn av1_get_pred_context_comp_ref_p(xd: &MacroBlockD) -> i32 {
    get_pred_context_ll2_or_l3gld(xd)
}

// Returns a context number for the given MB prediction signal
// Signal the first reference frame for a compound mode be LAST,
// conditioning on that it is known either LAST/LAST2.
pub(crate) fn av1_get_pred_context_comp_ref_p1(xd: &MacroBlockD) -> i32 {
    get_pred_context_last_or_last2(xd)
}

// Returns a context number for the given MB prediction signal
// Signal the first reference frame for a compound mode be GOLDEN,
// conditioning on that it is known either GOLDEN or LAST3.
pub(crate) fn av1_get_pred_context_comp_ref_p2(xd: &MacroBlockD) -> i32 {
    get_pred_context_last3_or_gld(xd)
}

// Signal the 2nd reference frame for a compound mode be either
// ALTREF, or ALTREF2/BWDREF.
pub(crate) fn av1_get_pred_context_comp_bwdref_p(xd: &MacroBlockD) -> i32 {
    get_pred_context_brfarf2_or_arf(xd)
}

// Signal the 2nd reference frame for a compound mode be either
// ALTREF2 or BWDREF.
pub(crate) fn av1_get_pred_context_comp_bwdref_p1(xd: &MacroBlockD) -> i32 {
    get_pred_context_brf_or_arf2(xd)
}

// == Context functions for single ref ==
//
// For the bit to signal whether the single reference is a forward reference
// frame or a backward reference frame.
pub(crate) fn av1_get_pred_context_single_ref_p1(xd: &MacroBlockD) -> i32 {
    // Count of forward reference frames
    let fwd_count = count(xd, LAST_FRAME)
        + count(xd, LAST2_FRAME)
        + count(xd, LAST3_FRAME)
        + count(xd, GOLDEN_FRAME);
    // Count of backward reference frames
    let bwd_count = count(xd, BWDREF_FRAME) + count(xd, ALTREF2_FRAME) + count(xd, ALTREF_FRAME);

    vote(fwd_count, bwd_count)
}

// For the bit to signal whether the single reference is ALTREF_FRAME or
// non-ALTREF backward reference frame, knowing that it shall be either of
// these 2 choices.
pub(crate) fn av1_get_pred_context_single_ref_p2(xd: &MacroBlockD) -> i32 {
    get_pred_context_brfarf2_or_arf(xd)
}

// For the bit to signal whether the single reference is LAST3/GOLDEN or
// LAST2/LAST, knowing that it shall be either of these 2 choices.
pub(crate) fn av1_get_pred_context_single_ref_p3(xd: &MacroBlockD) -> i32 {
    get_pred_context_ll2_or_l3gld(xd)
}

// For the bit to signal whether the single reference is LAST2_FRAME or
// LAST_FRAME, knowing that it shall be either of these 2 choices.
pub(crate) fn av1_get_pred_context_single_ref_p4(xd: &MacroBlockD) -> i32 {
    get_pred_context_last_or_last2(xd)
}

// For the bit to signal whether the single reference is GOLDEN_FRAME or
// LAST3_FRAME, knowing that it shall be either of these 2 choices.
pub(crate) fn av1_get_pred_context_single_ref_p5(xd: &MacroBlockD) -> i32 {
    get_pred_context_last3_or_gld(xd)
}

// For the bit to signal whether the single reference is ALTREF2_FRAME or
// BWDREF_FRAME, knowing that it shall be either of these 2 choices.
pub(crate) fn av1_get_pred_context_single_ref_p6(xd: &MacroBlockD) -> i32 {
    get_pred_context_brf_or_arf2(xd)
}

#[inline]
pub(crate) fn av1_get_skip_txfm_context(xd: &MacroBlockD) -> i32 {
    let above_mi = xd.mi_at(xd.above_mbmi);
    let left_mi = xd.mi_at(xd.left_mbmi);
    let above_skip_txfm = above_mi.map_or(0, |m| m.skip_txfm as i32);
    let left_skip_txfm = left_mi.map_or(0, |m| m.skip_txfm as i32);
    above_skip_txfm + left_skip_txfm
}

// Returns a context number for the given MB prediction signal
// The mode info data structure has a one element border above and to the
// left of the entries corresponding to real blocks.
// The prediction flags in these dummy entries are initialized to 0.
#[inline]
pub(crate) fn get_tx_size_context(xd: &MacroBlockD) -> i32 {
    let mbmi = xd.mbmi();
    let above_mbmi = xd.mi_at(xd.above_mbmi);
    let left_mbmi = xd.mi_at(xd.left_mbmi);
    let max_tx_size = MAX_TXSIZE_RECT_LOOKUP[mbmi.bsize as usize];
    let max_tx_wide = TX_SIZE_WIDE[max_tx_size as usize];
    let max_tx_high = TX_SIZE_HIGH[max_tx_size as usize];
    let has_above = xd.up_available;
    let has_left = xd.left_available;

    let mut above = (xd.above_txfm()[xd.above_txfm_context] as i32 >= max_tx_wide) as i32;
    let mut left = (xd.left_txfm_context_buffer[xd.left_txfm_context] as i32 >= max_tx_high) as i32;

    if has_above {
        if let Some(above_mbmi) = above_mbmi {
            if is_inter_block(above_mbmi) {
                above = (BLOCK_SIZE_WIDE[above_mbmi.bsize as usize] as i32 >= max_tx_wide) as i32;
            }
        }
    }
    if has_left {
        if let Some(left_mbmi) = left_mbmi {
            if is_inter_block(left_mbmi) {
                left = (BLOCK_SIZE_HIGH[left_mbmi.bsize as usize] as i32 >= max_tx_high) as i32;
            }
        }
    }
    if has_above && has_left {
        above + left
    } else if has_above {
        above
    } else if has_left {
        left
    } else {
        0
    }
}
