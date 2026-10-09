// Rust translation of src/refmvs.c and src/refmvs.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins), the plain-C functions.
// Copyright © 2020, VideoLAN and dav1d authors
// Copyright © 2020, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion vector prediction: the spatial and temporal candidate lists of
//! each block, and the per-frame motion field (projected from the
//! reference frames' saved vectors).
//!
//! The row pointers of `refmvs_tile` are offsets into the frame's
//! `refmvs_block` buffer (`NO_ROW` for the two `NULL` rows); the frame's
//! own temporal vectors (`rp`) are owned by `RefmvsFrame` while the frame
//! is decoded, the references' are shared. The tile/frame threading
//! variants are gone: one tile thread and one frame thread.

use std::sync::Arc;

use super::env::{fix_mv_precision, get_gmv_2d, get_poc_diff};
use super::headers::{Dav1dFrameHeader, Dav1dSequenceHeader, DAV1D_WM_TYPE_TRANSLATION};
use super::intops::{apply_sign, iclip, imax, imin};
use super::intra_edge::EDGE_I444_TOP_HAS_RIGHT;
use super::levels::Mv;
use super::tables::BLOCK_DIMENSIONS;

pub(crate) const INVALID_MV: u32 = 0x80008000;

/// `mv.n = INVALID_MV`.
pub(crate) const INVALID_MV_MV: Mv = Mv {
    y: i16::MIN,
    x: i16::MIN,
};

/// Translation of `refmvs_temporal_block`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RefmvsTemporalBlock {
    pub(crate) mv: Mv,
    pub(crate) ref_: i8,
}

/// Translation of `refmvs_refpair` (`ref[2]`; the `pair` view is the
/// comparison of both).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RefmvsRefpair {
    pub(crate) ref_: [i8; 2], // [0] = 0: intra=1, [1] = -1: comp=0
}

/// Translation of `refmvs_mvpair` (`mv[2]`; the `n` view is the
/// comparison of both).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RefmvsMvpair {
    pub(crate) mv: [Mv; 2],
}

/// Translation of `refmvs_block`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RefmvsBlock {
    pub(crate) mv: RefmvsMvpair,
    pub(crate) ref_: RefmvsRefpair,
    pub(crate) bs: u8,
    pub(crate) mf: u8, // 1 = globalmv+affine, 2 = newmv
}

/// Translation of `refmvs_frame`.
#[derive(Default)]
pub(crate) struct RefmvsFrame {
    pub(crate) frm_hdr: Option<Arc<Dav1dFrameHeader>>,
    pub(crate) iw4: i32,
    pub(crate) ih4: i32,
    pub(crate) iw8: i32,
    pub(crate) ih8: i32,
    pub(crate) sbsz: i32,
    pub(crate) use_ref_frame_mvs: i32,
    pub(crate) sign_bias: [u8; 7],
    pub(crate) mfmv_sign: [u8; 7],
    pub(crate) pocdiff: [i8; 7],
    pub(crate) mfmv_ref: [u8; 3],
    pub(crate) mfmv_ref2cur: [i32; 3],
    pub(crate) mfmv_ref2ref: [[i32; 7]; 3],
    pub(crate) n_mfmvs: i32,

    /// The frame's own temporal vectors (`f->mvs`).
    pub(crate) rp: Vec<RefmvsTemporalBlock>,
    pub(crate) rp_ref: [Option<Arc<Vec<RefmvsTemporalBlock>>>; 7],
    pub(crate) rp_proj: Vec<RefmvsTemporalBlock>,
    pub(crate) rp_stride: usize,

    pub(crate) r: Vec<RefmvsBlock>, // 35 x r_stride memory
    pub(crate) r_stride: usize,
    pub(crate) n_tile_rows: i32,
}

/// The row offset of `refmvs_tile.r[]`'s `NULL` entries.
pub(crate) const NO_ROW: usize = usize::MAX;

/// Translation of `refmvs_tile`.
#[derive(Clone, Copy)]
pub(crate) struct RefmvsTile {
    pub(crate) r: [usize; 32 + 5],
    pub(crate) rp_proj: usize,
    pub(crate) tile_col_start: i32,
    pub(crate) tile_col_end: i32,
    pub(crate) tile_row_start: i32,
    pub(crate) tile_row_end: i32,
}

impl Default for RefmvsTile {
    fn default() -> Self {
        RefmvsTile {
            r: [NO_ROW; 32 + 5],
            rp_proj: 0,
            tile_col_start: 0,
            tile_col_end: 0,
            tile_row_start: 0,
            tile_row_end: 0,
        }
    }
}

/// Translation of `refmvs_candidate`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RefmvsCandidate {
    pub(crate) mv: RefmvsMvpair,
    pub(crate) weight: i32,
}

#[allow(clippy::too_many_arguments)]
fn add_spatial_candidate(
    mvstack: &mut [RefmvsCandidate; 8],
    cnt: &mut usize,
    weight: i32,
    b: &RefmvsBlock,
    r#ref: RefmvsRefpair,
    gmv: &[Mv; 2],
    have_newmv_match: &mut i32,
    have_refmv_match: &mut i32,
) {
    if b.mv.mv[0].n() == INVALID_MV {
        return; // intra block, no intrabc
    }

    if r#ref.ref_[1] == -1 {
        for n in 0..2 {
            if b.ref_.ref_[n] == r#ref.ref_[0] {
                let cand_mv = if (b.mf & 1) != 0 && gmv[0].n() != INVALID_MV {
                    gmv[0]
                } else {
                    b.mv.mv[n]
                };

                *have_refmv_match = 1;
                *have_newmv_match |= (b.mf >> 1) as i32;

                let last = *cnt;
                for m in 0..last {
                    if mvstack[m].mv.mv[0] == cand_mv {
                        mvstack[m].weight += weight;
                        return;
                    }
                }

                if last < 8 {
                    mvstack[last].mv.mv[0] = cand_mv;
                    mvstack[last].weight = weight;
                    *cnt = last + 1;
                }
                return;
            }
        }
    } else if b.ref_ == r#ref {
        let cand_mv = RefmvsMvpair {
            mv: [
                if (b.mf & 1) != 0 && gmv[0].n() != INVALID_MV {
                    gmv[0]
                } else {
                    b.mv.mv[0]
                },
                if (b.mf & 1) != 0 && gmv[1].n() != INVALID_MV {
                    gmv[1]
                } else {
                    b.mv.mv[1]
                },
            ],
        };

        *have_refmv_match = 1;
        *have_newmv_match |= (b.mf >> 1) as i32;

        let last = *cnt;
        for n in 0..last {
            if mvstack[n].mv == cand_mv {
                mvstack[n].weight += weight;
                return;
            }
        }

        if last < 8 {
            mvstack[last].mv = cand_mv;
            mvstack[last].weight = weight;
            *cnt = last + 1;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_row(
    mvstack: &mut [RefmvsCandidate; 8],
    cnt: &mut usize,
    r#ref: RefmvsRefpair,
    gmv: &[Mv; 2],
    rr: &[RefmvsBlock],
    b: usize,
    bw4: i32,
    w4: i32,
    max_rows: i32,
    step: i32,
    have_newmv_match: &mut i32,
    have_refmv_match: &mut i32,
) -> i32 {
    let mut cand_b = &rr[b];
    let first_cand_bs = cand_b.bs;
    let first_cand_b_dim = &BLOCK_DIMENSIONS[first_cand_bs as usize];
    let mut cand_bw4 = first_cand_b_dim[0] as i32;
    let mut len = imax(step, imin(bw4, cand_bw4));

    if bw4 <= cand_bw4 {
        // FIXME weight can be higher for odd blocks (bx4 & 1), but then the
        // position of the first block has to be odd already, i.e. not just
        // for row_offset=-3/-5
        // FIXME why can this not be cand_bw4?
        let weight = if bw4 == 1 {
            2
        } else {
            imax(2, imin(2 * max_rows, first_cand_b_dim[1] as i32))
        };
        add_spatial_candidate(
            mvstack,
            cnt,
            len * weight,
            cand_b,
            r#ref,
            gmv,
            have_newmv_match,
            have_refmv_match,
        );
        return weight >> 1;
    }

    let mut x = 0;
    loop {
        // FIXME if we overhang above, we could fill a bitmask so we don't have
        // to repeat the add_spatial_candidate() for the next row, but just increase
        // the weight here
        add_spatial_candidate(
            mvstack,
            cnt,
            len * 2,
            cand_b,
            r#ref,
            gmv,
            have_newmv_match,
            have_refmv_match,
        );
        x += len;
        if x >= w4 {
            return 1;
        }
        cand_b = &rr[b + x as usize];
        cand_bw4 = BLOCK_DIMENSIONS[cand_b.bs as usize][0] as i32;
        debug_assert!(cand_bw4 < bw4);
        len = imax(step, cand_bw4);
    }
}

#[allow(clippy::too_many_arguments)]
fn scan_col(
    mvstack: &mut [RefmvsCandidate; 8],
    cnt: &mut usize,
    r#ref: RefmvsRefpair,
    gmv: &[Mv; 2],
    rr: &[RefmvsBlock],
    b: &[usize],
    bh4: i32,
    h4: i32,
    bx4: i32,
    max_cols: i32,
    step: i32,
    have_newmv_match: &mut i32,
    have_refmv_match: &mut i32,
) -> i32 {
    let mut cand_b = &rr[b[0] + bx4 as usize];
    let first_cand_bs = cand_b.bs;
    let first_cand_b_dim = &BLOCK_DIMENSIONS[first_cand_bs as usize];
    let mut cand_bh4 = first_cand_b_dim[1] as i32;
    let mut len = imax(step, imin(bh4, cand_bh4));

    if bh4 <= cand_bh4 {
        // FIXME weight can be higher for odd blocks (by4 & 1), but then the
        // position of the first block has to be odd already, i.e. not just
        // for col_offset=-3/-5
        // FIXME why can this not be cand_bh4?
        let weight = if bh4 == 1 {
            2
        } else {
            imax(2, imin(2 * max_cols, first_cand_b_dim[0] as i32))
        };
        add_spatial_candidate(
            mvstack,
            cnt,
            len * weight,
            cand_b,
            r#ref,
            gmv,
            have_newmv_match,
            have_refmv_match,
        );
        return weight >> 1;
    }

    let mut y = 0;
    loop {
        // FIXME if we overhang above, we could fill a bitmask so we don't have
        // to repeat the add_spatial_candidate() for the next row, but just increase
        // the weight here
        add_spatial_candidate(
            mvstack,
            cnt,
            len * 2,
            cand_b,
            r#ref,
            gmv,
            have_newmv_match,
            have_refmv_match,
        );
        y += len;
        if y >= h4 {
            return 1;
        }
        cand_b = &rr[b[y as usize] + bx4 as usize];
        cand_bh4 = BLOCK_DIMENSIONS[cand_b.bs as usize][1] as i32;
        debug_assert!(cand_bh4 < bh4);
        len = imax(step, cand_bh4);
    }
}

#[inline]
fn mv_projection(mv: Mv, num: i32, den: i32) -> Mv {
    static DIV_MULT: [u16; 32] = [
        0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170,
        1092, 1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
    ];
    debug_assert!(den > 0 && den < 32);
    debug_assert!(num > -32 && num < 32);
    let frac = num * DIV_MULT[den as usize] as i32;
    let y = mv.y as i32 * frac;
    let x = mv.x as i32 * frac;
    // Round and clip according to AV1 spec section 7.9.3
    Mv {
        // 0x3fff == (1 << 14) - 1
        y: iclip((y + 8192 + (y >> 31)) >> 14, -0x3fff, 0x3fff) as i16,
        x: iclip((x + 8192 + (x >> 31)) >> 14, -0x3fff, 0x3fff) as i16,
    }
}

fn add_temporal_candidate(
    rf: &RefmvsFrame,
    mvstack: &mut [RefmvsCandidate; 8],
    cnt: &mut usize,
    rb: &RefmvsTemporalBlock,
    r#ref: RefmvsRefpair,
    globalmv_ctx: Option<&mut i32>,
    gmv: Option<&[Mv; 2]>,
) {
    if rb.mv.n() == INVALID_MV {
        return;
    }
    let frm_hdr = rf.frm_hdr.as_deref().expect("refmvs frame header");

    let mut mv = mv_projection(
        rb.mv,
        rf.pocdiff[(r#ref.ref_[0] - 1) as usize] as i32,
        rb.ref_ as i32,
    );
    fix_mv_precision(frm_hdr, &mut mv);

    let last = *cnt;
    if r#ref.ref_[1] == -1 {
        if let (Some(globalmv_ctx), Some(gmv)) = (globalmv_ctx, gmv) {
            *globalmv_ctx = (((mv.x as i32 - gmv[0].x as i32).abs()
                | (mv.y as i32 - gmv[0].y as i32).abs())
                >= 16) as i32;
        }

        for n in 0..last {
            if mvstack[n].mv.mv[0] == mv {
                mvstack[n].weight += 2;
                return;
            }
        }
        if last < 8 {
            mvstack[last].mv.mv[0] = mv;
            mvstack[last].weight = 2;
            *cnt = last + 1;
        }
    } else {
        let mut mvp = RefmvsMvpair {
            mv: [
                mv,
                mv_projection(
                    rb.mv,
                    rf.pocdiff[(r#ref.ref_[1] - 1) as usize] as i32,
                    rb.ref_ as i32,
                ),
            ],
        };
        fix_mv_precision(frm_hdr, &mut mvp.mv[1]);

        for n in 0..last {
            if mvstack[n].mv == mvp {
                mvstack[n].weight += 2;
                return;
            }
        }
        if last < 8 {
            mvstack[last].mv = mvp;
            mvstack[last].weight = 2;
            *cnt = last + 1;
        }
    }
}

/// `add_compound_extended_candidate()`: `same` is `mvstack[cnt..]`; its
/// `diff` alias is `same[2..]`.
fn add_compound_extended_candidate(
    same: &mut [RefmvsCandidate],
    same_count: &mut [i32; 4],
    cand_b: &RefmvsBlock,
    sign0: u8,
    sign1: u8,
    r#ref: RefmvsRefpair,
    sign_bias: &[u8; 7],
) {
    // refmvs_candidate *const diff = &same[2];
    // int *const diff_count = &same_count[2];

    for n in 0..2 {
        let cand_ref = cand_b.ref_.ref_[n] as i32;

        if cand_ref <= 0 {
            break;
        }

        let mut cand_mv = cand_b.mv.mv[n];
        if cand_ref == r#ref.ref_[0] as i32 {
            if same_count[0] < 2 {
                same[same_count[0] as usize].mv.mv[0] = cand_mv;
                same_count[0] += 1;
            }
            if same_count[2 + 1] < 2 {
                if (sign1 ^ sign_bias[(cand_ref - 1) as usize]) != 0 {
                    cand_mv.y = cand_mv.y.wrapping_neg();
                    cand_mv.x = cand_mv.x.wrapping_neg();
                }
                same[2 + same_count[2 + 1] as usize].mv.mv[1] = cand_mv;
                same_count[2 + 1] += 1;
            }
        } else if cand_ref == r#ref.ref_[1] as i32 {
            if same_count[1] < 2 {
                same[same_count[1] as usize].mv.mv[1] = cand_mv;
                same_count[1] += 1;
            }
            if same_count[2] < 2 {
                if (sign0 ^ sign_bias[(cand_ref - 1) as usize]) != 0 {
                    cand_mv.y = cand_mv.y.wrapping_neg();
                    cand_mv.x = cand_mv.x.wrapping_neg();
                }
                same[2 + same_count[2] as usize].mv.mv[0] = cand_mv;
                same_count[2] += 1;
            }
        } else {
            let i_cand_mv = Mv {
                x: cand_mv.x.wrapping_neg(),
                y: cand_mv.y.wrapping_neg(),
            };

            if same_count[2] < 2 {
                same[2 + same_count[2] as usize].mv.mv[0] =
                    if (sign0 ^ sign_bias[(cand_ref - 1) as usize]) != 0 {
                        i_cand_mv
                    } else {
                        cand_mv
                    };
                same_count[2] += 1;
            }

            if same_count[2 + 1] < 2 {
                same[2 + same_count[2 + 1] as usize].mv.mv[1] =
                    if (sign1 ^ sign_bias[(cand_ref - 1) as usize]) != 0 {
                        i_cand_mv
                    } else {
                        cand_mv
                    };
                same_count[2 + 1] += 1;
            }
        }
    }
}

fn add_single_extended_candidate(
    mvstack: &mut [RefmvsCandidate; 8],
    cnt: &mut usize,
    cand_b: &RefmvsBlock,
    sign: u8,
    sign_bias: &[u8; 7],
) {
    for n in 0..2 {
        let cand_ref = cand_b.ref_.ref_[n] as i32;

        if cand_ref <= 0 {
            break;
        }
        // we need to continue even if cand_ref == ref.ref[0], since
        // the candidate could have been added as a globalmv variant,
        // which changes the value
        // FIXME if scan_{row,col}() returned a mask for the nearest
        // edge, we could skip the appropriate ones here

        let mut cand_mv = cand_b.mv.mv[n];
        if (sign ^ sign_bias[(cand_ref - 1) as usize]) != 0 {
            cand_mv.y = cand_mv.y.wrapping_neg();
            cand_mv.x = cand_mv.x.wrapping_neg();
        }

        let last = *cnt;
        let mut m = 0;
        while m < last {
            if cand_mv == mvstack[m].mv.mv[0] {
                break;
            }
            m += 1;
        }
        if m == last {
            mvstack[m].mv.mv[0] = cand_mv;
            mvstack[m].weight = 2; // "minimal"
            *cnt = last + 1;
        }
    }
}

/*
 * refmvs_frame allocates memory for one sbrow (32 blocks high, whole frame
 * wide) of 4x4-resolution refmvs_block entries for spatial MV referencing.
 * mvrefs_tile[] keeps a list of 35 (32 + 3 above) pointers into this memory,
 * and each sbrow, the bottom entries (y=27/29/31) are exchanged with the top
 * (-5/-3/-1) pointers by calling dav1d_refmvs_tile_sbrow_init() at the start
 * of each tile/sbrow.
 *
 * For temporal MV referencing, we call dav1d_refmvs_save_tmvs() at the end of
 * each tile/sbrow (when tile column threading is enabled), or at the start of
 * each interleaved sbrow (i.e. once for all tile columns together, when tile
 * column threading is disabled). This will copy the 4x4-resolution spatial MVs
 * into 8x8-resolution refmvs_temporal_block structures. Then, for subsequent
 * frames, at the start of each tile/sbrow (when tile column threading is
 * enabled) or at the start of each interleaved sbrow (when tile column
 * threading is disabled), we call load_tmvs(), which will project the MVs to
 * their respective position in the current frame.
 */

/// Translation of `dav1d_refmvs_find()`: returns (`cnt`, `ctx`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn refmvs_find(
    rt: &RefmvsTile,
    rf: &RefmvsFrame,
    mvstack: &mut [RefmvsCandidate; 8],
    r#ref: RefmvsRefpair,
    bs: u8,
    edge_flags: u8,
    by4: i32,
    bx4: i32,
) -> (usize, i32) {
    let frm_hdr = rf.frm_hdr.as_deref().expect("refmvs frame header");
    let rr = &rf.r[..];
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];
    let bw4 = b_dim[0] as i32;
    let w4 = imin(imin(bw4, 16), rt.tile_col_end - bx4);
    let bh4 = b_dim[1] as i32;
    let h4 = imin(imin(bh4, 16), rt.tile_row_end - by4);
    let mut gmv = [Mv::ZERO; 2];
    let mut tgmv = [Mv::ZERO; 2];

    let mut cnt: usize = 0;
    debug_assert!(
        r#ref.ref_[0] >= 0 && r#ref.ref_[0] <= 8 && r#ref.ref_[1] >= -1 && r#ref.ref_[1] <= 8
    );
    if r#ref.ref_[0] > 0 {
        let g = &frm_hdr.gmv[(r#ref.ref_[0] - 1) as usize];
        tgmv[0] = get_gmv_2d(g, bx4, by4, bw4, bh4, frm_hdr);
        gmv[0] = if g.type_ > DAV1D_WM_TYPE_TRANSLATION {
            tgmv[0]
        } else {
            INVALID_MV_MV
        };
    } else {
        tgmv[0] = Mv::from_n(0);
        gmv[0] = INVALID_MV_MV;
    }
    if r#ref.ref_[1] > 0 {
        let g = &frm_hdr.gmv[(r#ref.ref_[1] - 1) as usize];
        tgmv[1] = get_gmv_2d(g, bx4, by4, bw4, bh4, frm_hdr);
        gmv[1] = if g.type_ > DAV1D_WM_TYPE_TRANSLATION {
            tgmv[1]
        } else {
            INVALID_MV_MV
        };
    }

    // top
    let mut have_newmv = 0;
    let mut have_col_mvs = 0;
    let mut have_row_mvs = 0;
    let mut max_rows: u32 = 0;
    let mut n_rows: u32 = !0;
    let mut b_top = 0usize;
    if by4 > rt.tile_row_start {
        max_rows = imin((by4 - rt.tile_row_start + 1) >> 1, 2 + (bh4 > 1) as i32) as u32;
        b_top = rt.r[((by4 & 31) - 1 + 5) as usize] + bx4 as usize;
        n_rows = scan_row(
            mvstack,
            &mut cnt,
            r#ref,
            &gmv,
            rr,
            b_top,
            bw4,
            w4,
            max_rows as i32,
            if bw4 >= 16 { 4 } else { 1 },
            &mut have_newmv,
            &mut have_row_mvs,
        ) as u32;
    }

    // left
    let mut max_cols: u32 = 0;
    let mut n_cols: u32 = !0;
    let b_left = &rt.r[((by4 & 31) + 5) as usize..];
    if bx4 > rt.tile_col_start {
        max_cols = imin((bx4 - rt.tile_col_start + 1) >> 1, 2 + (bw4 > 1) as i32) as u32;
        n_cols = scan_col(
            mvstack,
            &mut cnt,
            r#ref,
            &gmv,
            rr,
            b_left,
            bh4,
            h4,
            bx4 - 1,
            max_cols as i32,
            if bh4 >= 16 { 4 } else { 1 },
            &mut have_newmv,
            &mut have_col_mvs,
        ) as u32;
    }

    // top/right
    if n_rows != !0u32
        && (edge_flags & EDGE_I444_TOP_HAS_RIGHT) != 0
        && imax(bw4, bh4) <= 16
        && bw4 + bx4 < rt.tile_col_end
    {
        add_spatial_candidate(
            mvstack,
            &mut cnt,
            4,
            &rr[b_top + bw4 as usize],
            r#ref,
            &gmv,
            &mut have_newmv,
            &mut have_row_mvs,
        );
    }

    let nearest_match = have_col_mvs + have_row_mvs;
    let nearest_cnt = cnt;
    for n in 0..nearest_cnt {
        mvstack[n].weight += 640;
    }

    // temporal
    let mut globalmv_ctx = frm_hdr.use_ref_frame_mvs;
    if rf.use_ref_frame_mvs != 0 {
        let stride = rf.rp_stride;
        let by8 = by4 >> 1;
        let bx8 = bx4 >> 1;
        let rbi = rt.rp_proj + (by8 & 15) as usize * stride + bx8 as usize;
        let mut rb = rbi;
        let step_h = if bw4 >= 16 { 2 } else { 1 };
        let step_v = if bh4 >= 16 { 2 } else { 1 };
        let w8 = imin((w4 + 1) >> 1, 8);
        let h8 = imin((h4 + 1) >> 1, 8);
        let mut y = 0;
        while y < h8 {
            let mut x = 0;
            while x < w8 {
                let blk = rf.rp_proj[rb + x as usize];
                if (x | y) == 0 {
                    add_temporal_candidate(
                        rf,
                        mvstack,
                        &mut cnt,
                        &blk,
                        r#ref,
                        Some(&mut globalmv_ctx),
                        Some(&tgmv),
                    );
                } else {
                    add_temporal_candidate(rf, mvstack, &mut cnt, &blk, r#ref, None, Some(&tgmv));
                }
                x += step_h;
            }
            rb += stride * step_v as usize;
            y += step_v;
        }
        if imin(bw4, bh4) >= 2 && imax(bw4, bh4) < 16 {
            let bh8 = bh4 >> 1;
            let bw8 = bw4 >> 1;
            let rb = rbi + bh8 as usize * stride;
            let has_bottom = by8 + bh8 < imin(rt.tile_row_end >> 1, (by8 & !7) + 8);
            if has_bottom && bx8 - 1 >= imax(rt.tile_col_start >> 1, bx8 & !7) {
                let blk = rf.rp_proj[rb - 1];
                add_temporal_candidate(rf, mvstack, &mut cnt, &blk, r#ref, None, None);
            }
            if bx8 + bw8 < imin(rt.tile_col_end >> 1, (bx8 & !7) + 8) {
                if has_bottom {
                    let blk = rf.rp_proj[rb + bw8 as usize];
                    add_temporal_candidate(rf, mvstack, &mut cnt, &blk, r#ref, None, None);
                }
                if by8 + bh8 - 1 < imin(rt.tile_row_end >> 1, (by8 & !7) + 8) {
                    let blk = rf.rp_proj[rb + bw8 as usize - stride];
                    add_temporal_candidate(rf, mvstack, &mut cnt, &blk, r#ref, None, None);
                }
            }
        }
    }
    debug_assert!(cnt <= 8);

    // top/left (which, confusingly, is part of "secondary" references)
    let mut have_dummy_newmv_match = 0;
    if (n_rows | n_cols) != !0u32 {
        add_spatial_candidate(
            mvstack,
            &mut cnt,
            4,
            &rr[b_top - 1],
            r#ref,
            &gmv,
            &mut have_dummy_newmv_match,
            &mut have_row_mvs,
        );
    }

    // "secondary" (non-direct neighbour) top & left edges
    // what is different about secondary is that everything is now in 8x8 resolution
    for n in 2..=3u32 {
        if n > n_rows && n <= max_rows {
            let row = rt.r[((((by4 & 31) - 2 * n as i32 + 1) | 1) + 5) as usize];
            n_rows = n_rows.wrapping_add(scan_row(
                mvstack,
                &mut cnt,
                r#ref,
                &gmv,
                rr,
                row + (bx4 | 1) as usize,
                bw4,
                w4,
                1 + max_rows as i32 - n as i32,
                if bw4 >= 16 { 4 } else { 2 },
                &mut have_dummy_newmv_match,
                &mut have_row_mvs,
            ) as u32);
        }

        if n > n_cols && n <= max_cols {
            n_cols = n_cols.wrapping_add(scan_col(
                mvstack,
                &mut cnt,
                r#ref,
                &gmv,
                rr,
                &rt.r[(((by4 & 31) | 1) + 5) as usize..],
                bh4,
                h4,
                (bx4 - n as i32 * 2 + 1) | 1,
                1 + max_cols as i32 - n as i32,
                if bh4 >= 16 { 4 } else { 2 },
                &mut have_dummy_newmv_match,
                &mut have_col_mvs,
            ) as u32);
        }
    }
    debug_assert!(cnt <= 8);

    let ref_match_count = have_col_mvs + have_row_mvs;

    // context build-up
    let (refmv_ctx, newmv_ctx) = match nearest_match {
        0 => (imin(2, ref_match_count), (ref_match_count > 0) as i32),
        1 => (imin(ref_match_count * 3, 4), 3 - have_newmv),
        _ => (5, 5 - have_newmv), // 2
    };

    // sorting (nearest, then "secondary")
    let mut len = nearest_cnt;
    while len != 0 {
        let mut last = 0;
        for n in 1..len {
            if mvstack[n - 1].weight < mvstack[n].weight {
                mvstack.swap(n - 1, n);
                last = n;
            }
        }
        len = last;
    }
    len = cnt;
    while len > nearest_cnt {
        let mut last = nearest_cnt;
        for n in nearest_cnt + 1..len {
            if mvstack[n - 1].weight < mvstack[n].weight {
                mvstack.swap(n - 1, n);
                last = n;
            }
        }
        len = last;
    }

    let ctx;
    if r#ref.ref_[1] > 0 {
        if cnt < 2 {
            let sign0 = rf.sign_bias[(r#ref.ref_[0] - 1) as usize];
            let sign1 = rf.sign_bias[(r#ref.ref_[1] - 1) as usize];
            let sz4 = imin(w4, h4);
            let base = cnt;
            let mut same_count = [0i32; 4];

            // non-self references in top
            if n_rows != !0u32 {
                let mut x = 0;
                while x < sz4 {
                    let cand_b = rr[b_top + x as usize];
                    add_compound_extended_candidate(
                        &mut mvstack[base..],
                        &mut same_count,
                        &cand_b,
                        sign0,
                        sign1,
                        r#ref,
                        &rf.sign_bias,
                    );
                    x += BLOCK_DIMENSIONS[cand_b.bs as usize][0] as i32;
                }
            }

            // non-self references in left
            if n_cols != !0u32 {
                let mut y = 0;
                while y < sz4 {
                    let cand_b = rr[b_left[y as usize] + (bx4 - 1) as usize];
                    add_compound_extended_candidate(
                        &mut mvstack[base..],
                        &mut same_count,
                        &cand_b,
                        sign0,
                        sign1,
                        r#ref,
                        &rf.sign_bias,
                    );
                    y += BLOCK_DIMENSIONS[cand_b.bs as usize][1] as i32;
                }
            }

            // refmvs_candidate *const diff = &same[2];
            // const int *const diff_count = &same_count[2];

            // merge together
            for n in 0..2 {
                let mut m = same_count[n] as usize;

                if m >= 2 {
                    continue;
                }

                let l = same_count[2 + n];
                if l != 0 {
                    mvstack[base + m].mv.mv[n] = mvstack[base + 2].mv.mv[n];
                    m += 1;
                    if m == 2 {
                        continue;
                    }
                    if l == 2 {
                        mvstack[base + 1].mv.mv[n] = mvstack[base + 2 + 1].mv.mv[n];
                        continue;
                    }
                }
                loop {
                    mvstack[base + m].mv.mv[n] = tgmv[n];
                    m += 1;
                    if m >= 2 {
                        break;
                    }
                }
            }

            // if the first extended was the same as the non-extended one,
            // then replace it with the second extended one
            let mut n = cnt;
            if n == 1 && mvstack[0].mv == mvstack[base].mv {
                mvstack[1].mv = mvstack[2].mv;
            }
            loop {
                mvstack[n].weight = 2;
                n += 1;
                if n >= 2 {
                    break;
                }
            }
            cnt = 2;
        }

        // clamping
        let left = -(bx4 + bw4 + 4) * 4 * 8;
        let right = (rf.iw4 - bx4 + 4) * 4 * 8;
        let top = -(by4 + bh4 + 4) * 4 * 8;
        let bottom = (rf.ih4 - by4 + 4) * 4 * 8;

        let n_refmvs = cnt;
        let mut n = 0;
        loop {
            let s = &mut mvstack[n];
            s.mv.mv[0].x = iclip(s.mv.mv[0].x as i32, left, right) as i16;
            s.mv.mv[0].y = iclip(s.mv.mv[0].y as i32, top, bottom) as i16;
            s.mv.mv[1].x = iclip(s.mv.mv[1].x as i32, left, right) as i16;
            s.mv.mv[1].y = iclip(s.mv.mv[1].y as i32, top, bottom) as i16;
            n += 1;
            if n >= n_refmvs {
                break;
            }
        }

        ctx = match refmv_ctx >> 1 {
            0 => imin(newmv_ctx, 1),
            1 => 1 + imin(newmv_ctx, 3),
            _ => iclip(3 + newmv_ctx, 4, 7), // 2
        };

        return (cnt, ctx);
    } else if cnt < 2 && r#ref.ref_[0] > 0 {
        let sign = rf.sign_bias[(r#ref.ref_[0] - 1) as usize];
        let sz4 = imin(w4, h4);

        // non-self references in top
        if n_rows != !0u32 {
            let mut x = 0;
            while x < sz4 && cnt < 2 {
                let cand_b = rr[b_top + x as usize];
                add_single_extended_candidate(mvstack, &mut cnt, &cand_b, sign, &rf.sign_bias);
                x += BLOCK_DIMENSIONS[cand_b.bs as usize][0] as i32;
            }
        }

        // non-self references in left
        if n_cols != !0u32 {
            let mut y = 0;
            while y < sz4 && cnt < 2 {
                let cand_b = rr[b_left[y as usize] + (bx4 - 1) as usize];
                add_single_extended_candidate(mvstack, &mut cnt, &cand_b, sign, &rf.sign_bias);
                y += BLOCK_DIMENSIONS[cand_b.bs as usize][1] as i32;
            }
        }
    }
    debug_assert!(cnt <= 8);

    // clamping
    let n_refmvs = cnt;
    if n_refmvs != 0 {
        let left = -(bx4 + bw4 + 4) * 4 * 8;
        let right = (rf.iw4 - bx4 + 4) * 4 * 8;
        let top = -(by4 + bh4 + 4) * 4 * 8;
        let bottom = (rf.ih4 - by4 + 4) * 4 * 8;

        let mut n = 0;
        loop {
            let s = &mut mvstack[n];
            s.mv.mv[0].x = iclip(s.mv.mv[0].x as i32, left, right) as i16;
            s.mv.mv[0].y = iclip(s.mv.mv[0].y as i32, top, bottom) as i16;
            n += 1;
            if n >= n_refmvs {
                break;
            }
        }
    }

    for n in cnt..2 {
        mvstack[n].mv.mv[0] = tgmv[0];
    }

    (cnt, (refmv_ctx << 4) | (globalmv_ctx << 3) | newmv_ctx)
}

/// Translation of `dav1d_refmvs_tile_sbrow_init()` (one tile thread, so
/// `tile_row_idx` is 0, and no 2-pass decoding).
#[allow(clippy::too_many_arguments)]
pub(crate) fn refmvs_tile_sbrow_init(
    rt: &mut RefmvsTile,
    rf: &RefmvsFrame,
    tile_col_start4: i32,
    tile_col_end4: i32,
    tile_row_start4: i32,
    tile_row_end4: i32,
    sby: i32,
) {
    let tile_row_idx = 0;
    rt.rp_proj = 16 * rf.rp_stride * tile_row_idx;
    let mut r = 35 * rf.r_stride * tile_row_idx;
    let sbsz = rf.sbsz as usize;
    let off = ((rf.sbsz * sby) & 16) as usize;
    for i in 0..sbsz {
        rt.r[off + 5 + i] = r;
        r += rf.r_stride;
    }
    rt.r[off] = r;
    r += rf.r_stride;
    rt.r[off + 1] = NO_ROW;
    rt.r[off + 2] = r;
    r += rf.r_stride;
    rt.r[off + 3] = NO_ROW;
    rt.r[off + 4] = r;
    if (sby & 1) != 0 {
        rt.r.swap(off, off + sbsz);
        rt.r.swap(off + 2, off + sbsz + 2);
        rt.r.swap(off + 4, off + sbsz + 4);
    }

    rt.tile_row_start = tile_row_start4;
    rt.tile_row_end = imin(tile_row_end4, rf.ih4);
    rt.tile_col_start = tile_col_start4;
    rt.tile_col_end = imin(tile_col_end4, rf.iw4);
}

/// Translation of `load_tmvs_c()` (one tile thread: `tile_row_idx` is 0).
pub(crate) fn load_tmvs(
    rf: &mut RefmvsFrame,
    col_start8: i32,
    col_end8: i32,
    row_start8: i32,
    mut row_end8: i32,
) {
    let tile_row_idx = 0usize;
    debug_assert!(row_start8 >= 0);
    debug_assert!((row_end8 - row_start8) as u32 <= 16);
    row_end8 = imin(row_end8, rf.ih8);
    let col_start8i = imax(col_start8 - 8, 0);
    let col_end8i = imin(col_end8 + 8, rf.iw8);

    let stride = rf.rp_stride;
    let mut rp_proj = 16 * stride * tile_row_idx + (row_start8 & 15) as usize * stride;
    for _y in row_start8..row_end8 {
        for x in col_start8..col_end8 {
            rf.rp_proj[rp_proj + x as usize].mv = INVALID_MV_MV;
        }
        rp_proj += stride;
    }

    let rp_proj = 16 * stride * tile_row_idx;
    for n in 0..rf.n_mfmvs as usize {
        let ref2cur = rf.mfmv_ref2cur[n];
        if ref2cur == i32::MIN {
            continue;
        }

        let r#ref = rf.mfmv_ref[n] as i32;
        let ref_sign = r#ref - 4;
        let Some(rp_ref) = rf.rp_ref[r#ref as usize].clone() else {
            continue;
        };
        let mut r = row_start8 as usize * stride;
        for y in row_start8..row_end8 {
            let y_sb_align = y & !7;
            let y_proj_start = imax(y_sb_align, row_start8);
            let y_proj_end = imin(y_sb_align + 8, row_end8);
            let mut x = col_start8i;
            while x < col_end8i {
                let mut rb = r + x as usize;
                let b_ref = rp_ref[rb].ref_ as i32;
                if b_ref == 0 {
                    x += 1;
                    continue;
                }
                let ref2ref = rf.mfmv_ref2ref[n][(b_ref - 1) as usize];
                if ref2ref == 0 {
                    x += 1;
                    continue;
                }
                let b_mv = rp_ref[rb].mv;
                let offset = mv_projection(b_mv, ref2cur, ref2ref);
                let mut pos_x =
                    x + apply_sign((offset.x as i32).abs() >> 6, offset.x as i32 ^ ref_sign);
                let pos_y =
                    y + apply_sign((offset.y as i32).abs() >> 6, offset.y as i32 ^ ref_sign);
                if pos_y >= y_proj_start && pos_y < y_proj_end {
                    let pos = (pos_y & 15) as usize * stride;
                    loop {
                        let x_sb_align = x & !7;
                        if pos_x >= imax(x_sb_align - 8, col_start8)
                            && pos_x < imin(x_sb_align + 16, col_end8)
                        {
                            let e = &mut rf.rp_proj[rp_proj + pos + pos_x as usize];
                            e.mv = rp_ref[rb].mv;
                            e.ref_ = ref2ref as i8;
                        }
                        x += 1;
                        if x >= col_end8i {
                            break;
                        }
                        rb += 1;
                        if rp_ref[rb].ref_ as i32 != b_ref || rp_ref[rb].mv != b_mv {
                            break;
                        }
                        pos_x += 1;
                    }
                } else {
                    loop {
                        x += 1;
                        if x >= col_end8i {
                            break;
                        }
                        rb += 1;
                        if rp_ref[rb].ref_ as i32 != b_ref || rp_ref[rb].mv != b_mv {
                            break;
                        }
                    }
                }
                // x-- and the loop's x++ cancel out
            }
            r += stride;
        }
    }
}

/// Translation of `save_tmvs_c()` and `dav1d_refmvs_save_tmvs()`: `rr`
/// is `rt->r + 6`.
pub(crate) fn save_tmvs(
    rt: &RefmvsTile,
    rf: &mut RefmvsFrame,
    col_start8: i32,
    mut col_end8: i32,
    row_start8: i32,
    mut row_end8: i32,
) {
    debug_assert!(row_start8 >= 0);
    debug_assert!((row_end8 - row_start8) as u32 <= 16);
    row_end8 = imin(row_end8, rf.ih8);
    col_end8 = imin(col_end8, rf.iw8);

    let stride = rf.rp_stride;
    let ref_sign = rf.mfmv_sign;
    let mut rp = row_start8 as usize * stride;
    let rr = &rt.r[6..];

    for y in row_start8..row_end8 {
        let b = rr[((y & 15) * 2) as usize];

        let mut x = col_start8;
        while x < col_end8 {
            let cand_b = rf.r[b + (x * 2 + 1) as usize];
            let bw8 = (BLOCK_DIMENSIONS[cand_b.bs as usize][0] + 1) >> 1;

            if cand_b.ref_.ref_[1] > 0
                && ref_sign[(cand_b.ref_.ref_[1] - 1) as usize] != 0
                && ((cand_b.mv.mv[1].y as i32).abs() | (cand_b.mv.mv[1].x as i32).abs()) < 4096
            {
                for _n in 0..bw8 {
                    rf.rp[rp + x as usize] = RefmvsTemporalBlock {
                        mv: cand_b.mv.mv[1],
                        ref_: cand_b.ref_.ref_[1],
                    };
                    x += 1;
                }
            } else if cand_b.ref_.ref_[0] > 0
                && ref_sign[(cand_b.ref_.ref_[0] - 1) as usize] != 0
                && ((cand_b.mv.mv[0].y as i32).abs() | (cand_b.mv.mv[0].x as i32).abs()) < 4096
            {
                for _n in 0..bw8 {
                    rf.rp[rp + x as usize] = RefmvsTemporalBlock {
                        mv: cand_b.mv.mv[0],
                        ref_: cand_b.ref_.ref_[0],
                    };
                    x += 1;
                }
            } else {
                for _n in 0..bw8 {
                    rf.rp[rp + x as usize].mv = Mv::from_n(0);
                    rf.rp[rp + x as usize].ref_ = 0; // "invalid"
                    x += 1;
                }
            }
        }
        rp += stride;
    }
}

/// Translation of `dav1d_refmvs_init_frame()` (one tile thread and one
/// frame thread). The frame's temporal vector buffer (`rp`) and the
/// references' (`rp_ref`) are set by the caller.
pub(crate) fn refmvs_init_frame(
    rf: &mut RefmvsFrame,
    seq_hdr: &Dav1dSequenceHeader,
    frm_hdr: &Arc<Dav1dFrameHeader>,
    ref_poc: &[u32; 7],
    ref_ref_poc: &[[u32; 7]; 7],
) -> Result<(), ()> {
    rf.sbsz = 16 << seq_hdr.sb128;
    rf.frm_hdr = Some(frm_hdr.clone());
    rf.iw8 = (frm_hdr.width[0] + 7) >> 3;
    rf.ih8 = (frm_hdr.height + 7) >> 3;
    rf.iw4 = rf.iw8 << 1;
    rf.ih4 = rf.ih8 << 1;

    let r_stride = (((frm_hdr.width[0] + 127) & !127) >> 2) as usize;
    let n_tile_rows = 1;
    if r_stride != rf.r_stride || n_tile_rows != rf.n_tile_rows {
        rf.r = Vec::new();
        rf.r = super::mem::try_vec(RefmvsBlock::default(), 35 * r_stride * n_tile_rows as usize)
            .map_err(|_| ())?;
        rf.r_stride = r_stride;
    }

    let rp_stride = r_stride >> 1;
    if rp_stride != rf.rp_stride || n_tile_rows != rf.n_tile_rows {
        rf.rp_proj = Vec::new();
        rf.rp_proj = super::mem::try_vec(
            RefmvsTemporalBlock::default(),
            16 * rp_stride * n_tile_rows as usize,
        )
        .map_err(|_| ())?;
        rf.rp_stride = rp_stride;
    }
    rf.n_tile_rows = n_tile_rows;
    let poc = frm_hdr.frame_offset as u32;
    for i in 0..7 {
        let poc_diff = get_poc_diff(seq_hdr.order_hint_n_bits, ref_poc[i] as i32, poc as i32);
        rf.sign_bias[i] = (poc_diff > 0) as u8;
        rf.mfmv_sign[i] = (poc_diff < 0) as u8;
        rf.pocdiff[i] = iclip(
            get_poc_diff(seq_hdr.order_hint_n_bits, poc as i32, ref_poc[i] as i32),
            -31,
            31,
        ) as i8;
    }

    // temporal MV setup
    rf.n_mfmvs = 0;
    if frm_hdr.use_ref_frame_mvs != 0 && seq_hdr.order_hint_n_bits != 0 {
        let mut total = 2;
        if rf.rp_ref[0].is_some() && ref_ref_poc[0][6] != ref_poc[3]
        /* alt-of-last != gold */
        {
            rf.mfmv_ref[rf.n_mfmvs as usize] = 0; // last
            rf.n_mfmvs += 1;
            total = 3;
        }
        if rf.rp_ref[4].is_some()
            && get_poc_diff(
                seq_hdr.order_hint_n_bits,
                ref_poc[4] as i32,
                frm_hdr.frame_offset,
            ) > 0
        {
            rf.mfmv_ref[rf.n_mfmvs as usize] = 4; // bwd
            rf.n_mfmvs += 1;
        }
        if rf.rp_ref[5].is_some()
            && get_poc_diff(
                seq_hdr.order_hint_n_bits,
                ref_poc[5] as i32,
                frm_hdr.frame_offset,
            ) > 0
        {
            rf.mfmv_ref[rf.n_mfmvs as usize] = 5; // altref2
            rf.n_mfmvs += 1;
        }
        if rf.n_mfmvs < total
            && rf.rp_ref[6].is_some()
            && get_poc_diff(
                seq_hdr.order_hint_n_bits,
                ref_poc[6] as i32,
                frm_hdr.frame_offset,
            ) > 0
        {
            rf.mfmv_ref[rf.n_mfmvs as usize] = 6; // altref
            rf.n_mfmvs += 1;
        }
        if rf.n_mfmvs < total && rf.rp_ref[1].is_some() {
            rf.mfmv_ref[rf.n_mfmvs as usize] = 1; // last2
            rf.n_mfmvs += 1;
        }

        for n in 0..rf.n_mfmvs as usize {
            let rpoc = ref_poc[rf.mfmv_ref[n] as usize];
            let diff1 = get_poc_diff(seq_hdr.order_hint_n_bits, rpoc as i32, frm_hdr.frame_offset);
            if diff1.abs() > 31 {
                rf.mfmv_ref2cur[n] = i32::MIN;
            } else {
                rf.mfmv_ref2cur[n] = if rf.mfmv_ref[n] < 4 { -diff1 } else { diff1 };
                for m in 0..7 {
                    let rrpoc = ref_ref_poc[rf.mfmv_ref[n] as usize][m];
                    let diff2 = get_poc_diff(seq_hdr.order_hint_n_bits, rpoc as i32, rrpoc as i32);
                    // unsigned comparison also catches the < 0 case
                    rf.mfmv_ref2ref[n][m] = if diff2 as u32 > 31 { 0 } else { diff2 };
                }
            }
        }
    }
    rf.use_ref_frame_mvs = (rf.n_mfmvs > 0) as i32;

    Ok(())
}

/// Translation of `splat_mv_c()`: `rr` is `&rt->r[(t->by & 31) + 5]`.
pub(crate) fn splat_mv(
    r: &mut [RefmvsBlock],
    rr: &[usize],
    rmv: &RefmvsBlock,
    bx4: usize,
    bw4: usize,
    bh4: usize,
) {
    for &row in &rr[..bh4] {
        let row = row + bx4;
        r[row..row + bw4].fill(*rmv);
    }
}
