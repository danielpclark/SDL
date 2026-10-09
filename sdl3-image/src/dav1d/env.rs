// Rust translation of src/env.h and src/ctx.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The above/left block contexts and the context derivations of the
//! symbol decoding (env.h), and the context fill helpers (ctx.h).
//!
//! ctx.h's `case_set*()` macros write a run of identical bytes through
//! 8-, 16-, 32- and 64-bit stores; [`set_ctx`] is the same fill.

use super::headers::{
    Dav1dFrameHeader, Dav1dWarpedMotionParams, DAV1D_N_SWITCHABLE_FILTERS, DAV1D_WM_TYPE_IDENTITY,
    DAV1D_WM_TYPE_ROT_ZOOM, DAV1D_WM_TYPE_TRANSLATION,
};
use super::intops::{apply_sign, imin};
use super::levels::*;
use super::refmvs::RefmvsCandidate;
use super::tables::TxfmInfo;

/// Translation of `BlockContext`.
#[derive(Clone, Copy)]
pub(crate) struct BlockContext {
    pub(crate) mode: [u8; 32],
    pub(crate) lcoef: [u8; 32],
    pub(crate) ccoef: [[u8; 32]; 2],
    pub(crate) seg_pred: [u8; 32],
    pub(crate) skip: [u8; 32],
    pub(crate) skip_mode: [u8; 32],
    pub(crate) intra: [u8; 32],
    pub(crate) comp_type: [u8; 32],
    pub(crate) ref_: [[i8; 32]; 2],   // -1 means intra
    pub(crate) filter: [[u8; 32]; 2], // 3 means unset
    pub(crate) tx_intra: [i8; 32],
    pub(crate) tx: [i8; 32],
    pub(crate) tx_lpf_y: [u8; 32],
    pub(crate) tx_lpf_uv: [u8; 32],
    pub(crate) partition: [u8; 16],
    pub(crate) uvmode: [u8; 32],
    pub(crate) pal_sz: [u8; 32],
}

impl Default for BlockContext {
    fn default() -> Self {
        BlockContext {
            mode: [0; 32],
            lcoef: [0; 32],
            ccoef: [[0; 32]; 2],
            seg_pred: [0; 32],
            skip: [0; 32],
            skip_mode: [0; 32],
            intra: [0; 32],
            comp_type: [0; 32],
            ref_: [[0; 32]; 2],
            filter: [[0; 32]; 2],
            tx_intra: [0; 32],
            tx: [0; 32],
            tx_lpf_y: [0; 32],
            tx_lpf_uv: [0; 32],
            partition: [0; 16],
            uvmode: [0; 32],
            pal_sz: [0; 32],
        }
    }
}

/// The `case_set()` family of ctx.h: fill `n` context entries starting at
/// `off` with `val`.
#[inline]
pub(crate) fn set_ctx<T: Copy>(ctx: &mut [T], off: usize, n: usize, val: T) {
    ctx[off..off + n].fill(val);
}

#[inline]
pub(crate) fn get_intra_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    if have_left {
        if have_top {
            let ctx = l.intra[yb4] as usize + a.intra[xb4] as usize;
            ctx + (ctx == 2) as usize
        } else {
            l.intra[yb4] as usize * 2
        }
    } else if have_top {
        a.intra[xb4] as usize * 2
    } else {
        0
    }
}

#[inline]
pub(crate) fn get_tx_ctx(
    a: &BlockContext,
    l: &BlockContext,
    max_tx: &TxfmInfo,
    yb4: usize,
    xb4: usize,
) -> usize {
    (l.tx_intra[yb4] as i32 >= max_tx.lh as i32) as usize
        + (a.tx_intra[xb4] as i32 >= max_tx.lw as i32) as usize
}

#[inline]
pub(crate) fn get_partition_ctx(
    a: &BlockContext,
    l: &BlockContext,
    bl: u8,
    yb8: usize,
    xb8: usize,
) -> usize {
    ((a.partition[xb8] >> (4 - bl)) & 1) as usize
        + ((((l.partition[yb8] >> (4 - bl)) & 1) as usize) << 1)
}

#[inline]
pub(crate) fn gather_left_partition_prob(r#in: &[u16], bl: u8) -> u32 {
    let i = |k: u8| r#in[k as usize] as u32;
    let mut out = i(PARTITION_H - 1).wrapping_sub(i(PARTITION_H));
    // Exploit the fact that cdfs for PARTITION_SPLIT, PARTITION_T_TOP_SPLIT,
    // PARTITION_T_BOTTOM_SPLIT and PARTITION_T_LEFT_SPLIT are neighbors.
    out = out.wrapping_add(i(PARTITION_SPLIT - 1).wrapping_sub(i(PARTITION_T_LEFT_SPLIT)));
    if bl != BL_128X128 {
        out = out.wrapping_add(i(PARTITION_H4 - 1).wrapping_sub(i(PARTITION_H4)));
    }
    out
}

#[inline]
pub(crate) fn gather_top_partition_prob(r#in: &[u16], bl: u8) -> u32 {
    let i = |k: u8| r#in[k as usize] as u32;
    // Exploit the fact that cdfs for PARTITION_V, PARTITION_SPLIT and
    // PARTITION_T_TOP_SPLIT are neighbors.
    let mut out = i(PARTITION_V - 1).wrapping_sub(i(PARTITION_T_TOP_SPLIT));
    // Exploit the facts that cdfs for PARTITION_T_LEFT_SPLIT and
    // PARTITION_T_RIGHT_SPLIT are neighbors, the probability for
    // PARTITION_V4 is always zero, and the probability for
    // PARTITION_T_RIGHT_SPLIT is zero in 128x128 blocks.
    out = out.wrapping_add(i(PARTITION_T_LEFT_SPLIT - 1));
    if bl != BL_128X128 {
        out = out.wrapping_add(i(PARTITION_V4 - 1).wrapping_sub(i(PARTITION_T_RIGHT_SPLIT)));
    }
    out
}

#[inline]
pub(crate) fn get_uv_inter_txtp(uvt_dim: &TxfmInfo, ytxtp: u8) -> u8 {
    if uvt_dim.max == TX_32X32 {
        return if ytxtp == IDTX { IDTX } else { DCT_DCT };
    }
    if uvt_dim.min == TX_16X16
        && ((1u32 << ytxtp)
            & ((1 << H_FLIPADST) | (1 << V_FLIPADST) | (1 << H_ADST) | (1 << V_ADST)))
            != 0
    {
        return DCT_DCT;
    }

    ytxtp
}

#[inline]
pub(crate) fn get_filter_ctx(
    a: &BlockContext,
    l: &BlockContext,
    comp: bool,
    dir: usize,
    r#ref: i8,
    yb4: usize,
    xb4: usize,
) -> usize {
    let n = DAV1D_N_SWITCHABLE_FILTERS as usize;
    let a_filter = if a.ref_[0][xb4] == r#ref || a.ref_[1][xb4] == r#ref {
        a.filter[dir][xb4] as usize
    } else {
        n
    };
    let l_filter = if l.ref_[0][yb4] == r#ref || l.ref_[1][yb4] == r#ref {
        l.filter[dir][yb4] as usize
    } else {
        n
    };
    let comp = comp as usize;

    if a_filter == l_filter {
        comp * 4 + a_filter
    } else if a_filter == n {
        comp * 4 + l_filter
    } else if l_filter == n {
        comp * 4 + a_filter
    } else {
        comp * 4 + n
    }
}

#[inline]
pub(crate) fn get_comp_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    if have_top {
        if have_left {
            if a.comp_type[xb4] != 0 {
                if l.comp_type[yb4] != 0 {
                    4
                } else {
                    // 4U means intra (-1) or bwd (>= 4)
                    2 + (l.ref_[0][yb4] as u32 >= 4) as usize
                }
            } else if l.comp_type[yb4] != 0 {
                // 4U means intra (-1) or bwd (>= 4)
                2 + (a.ref_[0][xb4] as u32 >= 4) as usize
            } else {
                ((l.ref_[0][yb4] >= 4) ^ (a.ref_[0][xb4] >= 4)) as usize
            }
        } else if a.comp_type[xb4] != 0 {
            3
        } else {
            (a.ref_[0][xb4] >= 4) as usize
        }
    } else if have_left {
        if l.comp_type[yb4] != 0 {
            3
        } else {
            (l.ref_[0][yb4] >= 4) as usize
        }
    } else {
        1
    }
}

#[inline]
fn has_uni_comp(edge: &BlockContext, off: usize) -> bool {
    (edge.ref_[0][off] < 4) == (edge.ref_[1][off] < 4)
}

#[inline]
pub(crate) fn get_comp_dir_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    if have_top && have_left {
        let a_intra = a.intra[xb4] != 0;
        let l_intra = l.intra[yb4] != 0;

        if a_intra && l_intra {
            return 2;
        }
        if a_intra || l_intra {
            let edge = if a_intra { l } else { a };
            let off = if a_intra { yb4 } else { xb4 };

            if edge.comp_type[off] == COMP_INTER_NONE {
                return 2;
            }
            return 1 + 2 * has_uni_comp(edge, off) as usize;
        }

        let a_comp = a.comp_type[xb4] != COMP_INTER_NONE;
        let l_comp = l.comp_type[yb4] != COMP_INTER_NONE;
        let a_ref0 = a.ref_[0][xb4];
        let l_ref0 = l.ref_[0][yb4];

        if !a_comp && !l_comp {
            1 + 2 * ((a_ref0 >= 4) == (l_ref0 >= 4)) as usize
        } else if !a_comp || !l_comp {
            let edge = if a_comp { a } else { l };
            let off = if a_comp { xb4 } else { yb4 };

            if !has_uni_comp(edge, off) {
                return 1;
            }
            3 + ((a_ref0 >= 4) == (l_ref0 >= 4)) as usize
        } else {
            let a_uni = has_uni_comp(a, xb4);
            let l_uni = has_uni_comp(l, yb4);

            if !a_uni && !l_uni {
                return 0;
            }
            if !a_uni || !l_uni {
                return 2;
            }
            3 + ((a_ref0 == 4) == (l_ref0 == 4)) as usize
        }
    } else if have_top || have_left {
        let edge = if have_left { l } else { a };
        let off = if have_left { yb4 } else { xb4 };

        if edge.intra[off] != 0 {
            return 2;
        }
        if edge.comp_type[off] == COMP_INTER_NONE {
            return 2;
        }
        4 * has_uni_comp(edge, off) as usize
    } else {
        2
    }
}

#[inline]
pub(crate) fn get_poc_diff(order_hint_n_bits: i32, poc0: i32, poc1: i32) -> i32 {
    if order_hint_n_bits == 0 {
        return 0;
    }
    let mask = 1 << (order_hint_n_bits - 1);
    let diff = poc0.wrapping_sub(poc1);
    (diff & (mask - 1)) - (diff & mask)
}

#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn get_jnt_comp_ctx(
    order_hint_n_bits: i32,
    poc: u32,
    ref0poc: u32,
    ref1poc: u32,
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
) -> usize {
    let d0 = get_poc_diff(order_hint_n_bits, ref0poc as i32, poc as i32).unsigned_abs();
    let d1 = get_poc_diff(order_hint_n_bits, poc as i32, ref1poc as i32).unsigned_abs();
    let offset = (d0 == d1) as usize;
    let a_ctx = (a.comp_type[xb4] >= COMP_INTER_AVG || a.ref_[0][xb4] == 6) as usize;
    let l_ctx = (l.comp_type[yb4] >= COMP_INTER_AVG || l.ref_[0][yb4] == 6) as usize;

    3 * offset + a_ctx + l_ctx
}

#[inline]
pub(crate) fn get_mask_comp_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
) -> usize {
    let a_ctx = if a.comp_type[xb4] >= COMP_INTER_SEG {
        1
    } else if a.ref_[0][xb4] == 6 {
        3
    } else {
        0
    };
    let l_ctx = if l.comp_type[yb4] >= COMP_INTER_SEG {
        1
    } else if l.ref_[0][yb4] == 6 {
        3
    } else {
        0
    };

    imin(a_ctx + l_ctx, 5) as usize
}

// #define av1_get_ref_2_ctx av1_get_bwd_ref_ctx
// #define av1_get_ref_3_ctx av1_get_fwd_ref_ctx
// #define av1_get_ref_4_ctx av1_get_fwd_ref_1_ctx
// #define av1_get_ref_5_ctx av1_get_fwd_ref_2_ctx
// #define av1_get_ref_6_ctx av1_get_bwd_ref_1_ctx
// #define av1_get_uni_p_ctx av1_get_ref_ctx
// #define av1_get_uni_p2_ctx av1_get_fwd_ref_2_ctx
// (the callers use the functions these name directly)

#[inline]
fn cmp3(cnt0: i32, cnt1: i32) -> usize {
    if cnt0 == cnt1 {
        1
    } else if cnt0 < cnt1 {
        0
    } else {
        2
    }
}

#[inline]
pub(crate) fn av1_get_ref_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    let mut cnt = [0i32; 2];

    if have_top && a.intra[xb4] == 0 {
        cnt[(a.ref_[0][xb4] >= 4) as usize] += 1;
        if a.comp_type[xb4] != 0 {
            cnt[(a.ref_[1][xb4] >= 4) as usize] += 1;
        }
    }

    if have_left && l.intra[yb4] == 0 {
        cnt[(l.ref_[0][yb4] >= 4) as usize] += 1;
        if l.comp_type[yb4] != 0 {
            cnt[(l.ref_[1][yb4] >= 4) as usize] += 1;
        }
    }

    cmp3(cnt[0], cnt[1])
}

#[inline]
pub(crate) fn av1_get_fwd_ref_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    let mut cnt = [0i32; 4];

    if have_top && a.intra[xb4] == 0 {
        if a.ref_[0][xb4] < 4 {
            cnt[a.ref_[0][xb4] as usize] += 1;
        }
        if a.comp_type[xb4] != 0 && a.ref_[1][xb4] < 4 {
            cnt[a.ref_[1][xb4] as usize] += 1;
        }
    }

    if have_left && l.intra[yb4] == 0 {
        if l.ref_[0][yb4] < 4 {
            cnt[l.ref_[0][yb4] as usize] += 1;
        }
        if l.comp_type[yb4] != 0 && l.ref_[1][yb4] < 4 {
            cnt[l.ref_[1][yb4] as usize] += 1;
        }
    }

    cnt[0] += cnt[1];
    cnt[2] += cnt[3];

    cmp3(cnt[0], cnt[2])
}

#[inline]
pub(crate) fn av1_get_fwd_ref_1_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    let mut cnt = [0i32; 2];

    if have_top && a.intra[xb4] == 0 {
        if a.ref_[0][xb4] < 2 {
            cnt[a.ref_[0][xb4] as usize] += 1;
        }
        if a.comp_type[xb4] != 0 && a.ref_[1][xb4] < 2 {
            cnt[a.ref_[1][xb4] as usize] += 1;
        }
    }

    if have_left && l.intra[yb4] == 0 {
        if l.ref_[0][yb4] < 2 {
            cnt[l.ref_[0][yb4] as usize] += 1;
        }
        if l.comp_type[yb4] != 0 && l.ref_[1][yb4] < 2 {
            cnt[l.ref_[1][yb4] as usize] += 1;
        }
    }

    cmp3(cnt[0], cnt[1])
}

#[inline]
pub(crate) fn av1_get_fwd_ref_2_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    let mut cnt = [0i32; 2];

    if have_top && a.intra[xb4] == 0 {
        if (a.ref_[0][xb4] as u32 ^ 2) < 2 {
            cnt[(a.ref_[0][xb4] - 2) as usize] += 1;
        }
        if a.comp_type[xb4] != 0 && (a.ref_[1][xb4] as u32 ^ 2) < 2 {
            cnt[(a.ref_[1][xb4] - 2) as usize] += 1;
        }
    }

    if have_left && l.intra[yb4] == 0 {
        if (l.ref_[0][yb4] as u32 ^ 2) < 2 {
            cnt[(l.ref_[0][yb4] - 2) as usize] += 1;
        }
        if l.comp_type[yb4] != 0 && (l.ref_[1][yb4] as u32 ^ 2) < 2 {
            cnt[(l.ref_[1][yb4] - 2) as usize] += 1;
        }
    }

    cmp3(cnt[0], cnt[1])
}

#[inline]
pub(crate) fn av1_get_bwd_ref_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    let mut cnt = [0i32; 3];

    if have_top && a.intra[xb4] == 0 {
        if a.ref_[0][xb4] >= 4 {
            cnt[(a.ref_[0][xb4] - 4) as usize] += 1;
        }
        if a.comp_type[xb4] != 0 && a.ref_[1][xb4] >= 4 {
            cnt[(a.ref_[1][xb4] - 4) as usize] += 1;
        }
    }

    if have_left && l.intra[yb4] == 0 {
        if l.ref_[0][yb4] >= 4 {
            cnt[(l.ref_[0][yb4] - 4) as usize] += 1;
        }
        if l.comp_type[yb4] != 0 && l.ref_[1][yb4] >= 4 {
            cnt[(l.ref_[1][yb4] - 4) as usize] += 1;
        }
    }

    cnt[1] += cnt[0];

    cmp3(cnt[1], cnt[2])
}

#[inline]
pub(crate) fn av1_get_bwd_ref_1_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    let mut cnt = [0i32; 3];

    if have_top && a.intra[xb4] == 0 {
        if a.ref_[0][xb4] >= 4 {
            cnt[(a.ref_[0][xb4] - 4) as usize] += 1;
        }
        if a.comp_type[xb4] != 0 && a.ref_[1][xb4] >= 4 {
            cnt[(a.ref_[1][xb4] - 4) as usize] += 1;
        }
    }

    if have_left && l.intra[yb4] == 0 {
        if l.ref_[0][yb4] >= 4 {
            cnt[(l.ref_[0][yb4] - 4) as usize] += 1;
        }
        if l.comp_type[yb4] != 0 && l.ref_[1][yb4] >= 4 {
            cnt[(l.ref_[1][yb4] - 4) as usize] += 1;
        }
    }

    cmp3(cnt[0], cnt[1])
}

#[inline]
pub(crate) fn av1_get_uni_p1_ctx(
    a: &BlockContext,
    l: &BlockContext,
    yb4: usize,
    xb4: usize,
    have_top: bool,
    have_left: bool,
) -> usize {
    let mut cnt = [0i32; 3];

    if have_top && a.intra[xb4] == 0 {
        if (a.ref_[0][xb4] as u32).wrapping_sub(1) < 3 {
            cnt[(a.ref_[0][xb4] - 1) as usize] += 1;
        }
        if a.comp_type[xb4] != 0 && (a.ref_[1][xb4] as u32).wrapping_sub(1) < 3 {
            cnt[(a.ref_[1][xb4] - 1) as usize] += 1;
        }
    }

    if have_left && l.intra[yb4] == 0 {
        if (l.ref_[0][yb4] as u32).wrapping_sub(1) < 3 {
            cnt[(l.ref_[0][yb4] - 1) as usize] += 1;
        }
        if l.comp_type[yb4] != 0 && (l.ref_[1][yb4] as u32).wrapping_sub(1) < 3 {
            cnt[(l.ref_[1][yb4] - 1) as usize] += 1;
        }
    }

    cnt[1] += cnt[2];

    cmp3(cnt[0], cnt[1])
}

#[inline]
pub(crate) fn get_drl_context(ref_mv_stack: &[RefmvsCandidate], ref_idx: usize) -> usize {
    if ref_mv_stack[ref_idx].weight >= 640 {
        return (ref_mv_stack[ref_idx + 1].weight < 640) as usize;
    }

    if ref_mv_stack[ref_idx + 1].weight < 640 {
        2
    } else {
        0
    }
}

/// Translation of `get_cur_frame_segid()`: returns (the predicted
/// segment id, `seg_ctx`).
#[inline]
pub(crate) fn get_cur_frame_segid(
    by: usize,
    bx: usize,
    have_top: bool,
    have_left: bool,
    cur_seg_map: &[u8],
    stride: usize,
) -> (u32, usize) {
    let off = bx + by * stride;
    if have_left && have_top {
        let l = cur_seg_map[off - 1];
        let a = cur_seg_map[off - stride];
        let al = cur_seg_map[off - (stride + 1)];

        let seg_ctx = if l == a && al == l {
            2
        } else if l == a || al == l || a == al {
            1
        } else {
            0
        };
        (if a == al { a } else { l } as u32, seg_ctx)
    } else {
        (
            if have_left {
                cur_seg_map[off - 1]
            } else if have_top {
                cur_seg_map[off - stride]
            } else {
                0
            } as u32,
            0,
        )
    }
}

#[inline]
pub(crate) fn fix_int_mv_precision(mv: &mut Mv) {
    let x = mv.x as i32;
    let y = mv.y as i32;
    mv.x = ((x - (x >> 15) + 3) as u32 & !7u32) as i16;
    mv.y = ((y - (y >> 15) + 3) as u32 & !7u32) as i16;
}

#[inline]
pub(crate) fn fix_mv_precision(hdr: &Dav1dFrameHeader, mv: &mut Mv) {
    if hdr.force_integer_mv != 0 {
        fix_int_mv_precision(mv);
    } else if hdr.hp == 0 {
        let x = mv.x as i32;
        let y = mv.y as i32;
        mv.x = ((x - (x >> 15)) as u32 & !1u32) as i16;
        mv.y = ((y - (y >> 15)) as u32 & !1u32) as i16;
    }
}

#[inline]
pub(crate) fn get_gmv_2d(
    gmv: &Dav1dWarpedMotionParams,
    bx4: i32,
    by4: i32,
    bw4: i32,
    bh4: i32,
    hdr: &Dav1dFrameHeader,
) -> Mv {
    match gmv.type_ {
        DAV1D_WM_TYPE_TRANSLATION => {
            let mut res = Mv {
                y: (gmv.matrix[0] >> 13) as i16,
                x: (gmv.matrix[1] >> 13) as i16,
            };
            if hdr.force_integer_mv != 0 {
                fix_int_mv_precision(&mut res);
            }
            res
        }
        DAV1D_WM_TYPE_IDENTITY => Mv { x: 0, y: 0 },
        // DAV1D_WM_TYPE_ROT_ZOOM falls through to the DAV1D_WM_TYPE_AFFINE
        // (and default) case
        _ => {
            if gmv.type_ == DAV1D_WM_TYPE_ROT_ZOOM {
                debug_assert!(gmv.matrix[5] == gmv.matrix[2]);
                debug_assert!(gmv.matrix[4] == -gmv.matrix[3]);
            }
            let x = bx4 * 4 + bw4 * 2 - 1;
            let y = by4 * 4 + bh4 * 2 - 1;
            let xc = (gmv.matrix[2] - (1 << 16))
                .wrapping_mul(x)
                .wrapping_add(gmv.matrix[3].wrapping_mul(y))
                .wrapping_add(gmv.matrix[0]);
            let yc = (gmv.matrix[5] - (1 << 16))
                .wrapping_mul(y)
                .wrapping_add(gmv.matrix[4].wrapping_mul(x))
                .wrapping_add(gmv.matrix[1]);
            let hp = (hdr.hp == 0) as i32;
            let shift = 16 - (3 - hp);
            let round = (1 << shift) >> 1;
            let mut res = Mv {
                y: apply_sign(((yc.wrapping_abs().wrapping_add(round)) >> shift) << hp, yc) as i16,
                x: apply_sign(((xc.wrapping_abs().wrapping_add(round)) >> shift) << hp, xc) as i16,
            };
            if hdr.force_integer_mv != 0 {
                fix_int_mv_precision(&mut res);
            }
            res
        }
    }
}
