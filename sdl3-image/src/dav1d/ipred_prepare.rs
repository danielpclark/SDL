// Rust translation of src/ipred_prepare_tmpl.c and src/ipred_prepare.h
// from dav1d (https://code.videolan.org/videolan/dav1d, at the revision
// SDL_image's external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Luma intra edge preparation.
//!
//! x/y/start/w/h are in luma block (4px) units:
//! - x and y are the absolute block positions in the image;
//! - start/w/h are the *dependent tile* boundary positions. In practice, start
//!   is the horizontal tile start, w is the horizontal tile end, the vertical
//!   tile start is assumed to be 0 and h is the vertical image end.
//!
//! edge_flags signals which edges are available for this transform-block inside
//! the given partition, as well as for the partition inside the superblock
//! structure.
//!
//! dst and stride are pointers to the top/left position of the current block,
//! and can be used to locate the top, left, top/left, top/right and bottom/left
//! edge pointers also.
//!
//! angle is the angle_delta [-3..3] on input, and the absolute angle on output.
//!
//! mode is the intra prediction mode as coded in the bitstream. The return value
//! is this same mode, converted to an index in the DSP functions.
//!
//! tw/th are the size of the transform block in block (4px) units.
//!
//! topleft_out is a pointer to scratch memory that will be filled with the edge
//! pixels. The memory array should have space to be indexed in the [-2*w,2*w]
//! range, in the following order:
//! - [0] will be the top/left edge pixel;
//! - [1..w] will be the top edge pixels (1 being left-most, w being right-most);
//! - [w+1..2*w] will be the top/right edge pixels;
//! - [-1..-w] will be the left edge pixels (-1 being top-most, -w being bottom-
//!   most);
//! - [-w-1..-2*w] will be the bottom/left edge pixels.
//!
//! Each edge may remain uninitialized if it is not used by the returned mode
//! index. If edges are not available (because the edge position is outside the
//! tile dimensions or because edge_flags indicates lack of edge availability),
//! they will be extended from nearby edges as defined by the av1 spec.

#![allow(clippy::too_many_arguments)]

use super::bitdepth::{bitdepth_from_max, Pixel};
use super::env::BlockContext;
use super::intops::imin;
use super::intra_edge::{EDGE_I444_LEFT_HAS_BOTTOM, EDGE_I444_TOP_HAS_RIGHT};
use super::levels::*;

// These flags are OR'd with the angle argument into intra predictors.
// ANGLE_USE_EDGE_FILTER_FLAG signals that edges should be convolved
// with a filter before using them to predict values in a block.
// ANGLE_SMOOTH_EDGE_FLAG means that edges are smooth and should use
// reduced filter strength.
pub(crate) const ANGLE_USE_EDGE_FILTER_FLAG: i32 = 1024;
pub(crate) const ANGLE_SMOOTH_EDGE_FLAG: i32 = 512;

#[inline]
pub(crate) fn sm_flag(b: &BlockContext, idx: usize) -> i32 {
    if b.intra[idx] == 0 {
        return 0;
    }
    let m = b.mode[idx];
    if m == SMOOTH_PRED || m == SMOOTH_H_PRED || m == SMOOTH_V_PRED {
        ANGLE_SMOOTH_EDGE_FLAG
    } else {
        0
    }
}

#[inline]
pub(crate) fn sm_uv_flag(b: &BlockContext, idx: usize) -> i32 {
    let m = b.uvmode[idx];
    if m == SMOOTH_PRED || m == SMOOTH_H_PRED || m == SMOOTH_V_PRED {
        ANGLE_SMOOTH_EDGE_FLAG
    } else {
        0
    }
}

/// `av1_mode_conv[mode][have_left][have_top]` (the DC_PRED and PAETH_PRED
/// rows; the others are zero).
fn av1_mode_conv(mode: u8, have_left: bool, have_top: bool) -> u8 {
    match (mode, have_left, have_top) {
        (DC_PRED, false, false) => DC_128_PRED,
        (DC_PRED, false, true) => TOP_DC_PRED,
        (DC_PRED, true, false) => LEFT_DC_PRED,
        (DC_PRED, true, true) => DC_PRED,
        (PAETH_PRED, false, false) => DC_128_PRED,
        (PAETH_PRED, false, true) => VERT_PRED,
        (PAETH_PRED, true, false) => HOR_PRED,
        (PAETH_PRED, true, true) => PAETH_PRED,
        _ => 0,
    }
}

static AV1_MODE_TO_ANGLE_MAP: [u8; 8] = [90, 180, 45, 135, 113, 157, 203, 67];

/// `av1_intra_prediction_edges[]`: needs_left, needs_top, needs_topleft,
/// needs_topright, needs_bottomleft.
#[derive(Clone, Copy)]
struct Edges {
    needs_left: bool,
    needs_top: bool,
    needs_topleft: bool,
    needs_topright: bool,
    needs_bottomleft: bool,
}

const fn e(l: bool, t: bool, tl: bool, tr: bool, bl: bool) -> Edges {
    Edges {
        needs_left: l,
        needs_top: t,
        needs_topleft: tl,
        needs_topright: tr,
        needs_bottomleft: bl,
    }
}

static AV1_INTRA_PREDICTION_EDGES: [Edges; N_IMPL_INTRA_PRED_MODES] = [
    /* DC_PRED */ e(true, true, false, false, false),
    /* VERT_PRED */ e(false, true, false, false, false),
    /* HOR_PRED */ e(true, false, false, false, false),
    /* LEFT_DC_PRED */ e(true, false, false, false, false),
    /* TOP_DC_PRED */ e(false, true, false, false, false),
    /* DC_128_PRED */ e(false, false, false, false, false),
    /* Z1_PRED */ e(false, true, true, true, false),
    /* Z2_PRED */ e(true, true, true, false, false),
    /* Z3_PRED */ e(true, false, true, false, true),
    /* SMOOTH_PRED */ e(true, true, false, false, false),
    /* SMOOTH_V_PRED */ e(true, true, false, false, false),
    /* SMOOTH_H_PRED */ e(true, true, false, false, false),
    /* PAETH_PRED */ e(true, true, true, false, false),
    /* FILTER_PRED */ e(true, true, true, false, false),
];

/// Where `prefilter_toplevel_sb_edge` points: the saved pre-filter row
/// above the superblock (`ipred_edge`, the offset of the sbrow's row in
/// it), or nothing (`NULL`) to read the row above `dst`.
pub(crate) type TopSbEdge<'a, P> = Option<(&'a [P], isize)>;

/// Translation of `dav1d_prepare_intra_edges()`: `dst[d..]` is the block
/// (in a plane whose stride is `stride`), `topleft_out[tl..]` the edge
/// buffer.
pub(crate) fn prepare_intra_edges<P: Pixel>(
    x: i32,
    have_left: bool,
    y: i32,
    have_top: bool,
    w: i32,
    h: i32,
    edge_flags: u8,
    dst: &[P],
    d: usize,
    stride: usize,
    prefilter_toplevel_sb_edge: TopSbEdge<'_, P>,
    mut mode: u8,
    angle: &mut i32,
    tw: i32,
    th: i32,
    filter_edge: bool,
    topleft_out: &mut [P],
    tl: usize,
    bitdepth_max: i32,
) -> u8 {
    let bitdepth = bitdepth_from_max(bitdepth_max);
    debug_assert!(y < h && x < w);

    match mode {
        VERT_PRED | HOR_PRED | DIAG_DOWN_LEFT_PRED | DIAG_DOWN_RIGHT_PRED | VERT_RIGHT_PRED
        | HOR_DOWN_PRED | HOR_UP_PRED | VERT_LEFT_PRED => {
            *angle = AV1_MODE_TO_ANGLE_MAP[(mode - VERT_PRED) as usize] as i32 + 3 * *angle;

            mode = if *angle <= 90 {
                if *angle < 90 && have_top {
                    Z1_PRED
                } else {
                    VERT_PRED
                }
            } else if *angle < 180 {
                Z2_PRED
            } else if *angle > 180 && have_left {
                Z3_PRED
            } else {
                HOR_PRED
            };
        }
        DC_PRED | PAETH_PRED => {
            mode = av1_mode_conv(mode, have_left, have_top);
        }
        _ => {}
    }

    let ed = AV1_INTRA_PREDICTION_EDGES[mode as usize];
    // dst_top: (buffer, offset)
    let mut dst_top: Option<(&[P], usize)> = None;
    if have_top && (ed.needs_top || ed.needs_topleft || (ed.needs_left && !have_left)) {
        dst_top = Some(match prefilter_toplevel_sb_edge {
            Some((edge, off)) => (edge, (off + x as isize * 4) as usize),
            None => (dst, d - stride),
        });
    }

    if ed.needs_left {
        let sz = (th << 2) as usize;
        let left = tl - sz;

        if have_left {
            let px_have = imin(sz as i32, (h - y) << 2) as usize;

            for i in 0..px_have {
                topleft_out[left + sz - 1 - i] = dst[d + stride * i - 1];
            }
            if px_have < sz {
                let v = topleft_out[left + sz - px_have];
                topleft_out[left..left + sz - px_have].fill(v);
            }
        } else {
            let v = match dst_top {
                Some((buf, o)) => buf[o],
                None => P::from_i32(((1 << bitdepth) >> 1) + 1),
            };
            topleft_out[left..left + sz].fill(v);
        }

        if ed.needs_bottomleft {
            let have_bottomleft = if !have_left || y + th >= h {
                false
            } else {
                (edge_flags & EDGE_I444_LEFT_HAS_BOTTOM) != 0
            };

            if have_bottomleft {
                let px_have = imin(sz as i32, (h - y - th) << 2) as usize;

                for i in 0..px_have {
                    topleft_out[left - (i + 1)] = dst[d + (sz + i) * stride - 1];
                }
                if px_have < sz {
                    let v = topleft_out[left - px_have];
                    topleft_out[left - sz..left - px_have].fill(v);
                }
            } else {
                let v = topleft_out[left];
                topleft_out[left - sz..left].fill(v);
            }
        }
    }

    if ed.needs_top {
        let sz = (tw << 2) as usize;
        let top = tl + 1;

        if have_top {
            let (buf, o) = dst_top.expect("dst_top");
            let px_have = imin(sz as i32, (w - x) << 2) as usize;
            topleft_out[top..top + px_have].copy_from_slice(&buf[o..o + px_have]);
            if px_have < sz {
                let v = topleft_out[top + px_have - 1];
                topleft_out[top + px_have..top + sz].fill(v);
            }
        } else {
            let v = if have_left {
                dst[d - 1]
            } else {
                P::from_i32(((1 << bitdepth) >> 1) - 1)
            };
            topleft_out[top..top + sz].fill(v);
        }

        if ed.needs_topright {
            let have_topright = if !have_top || x + tw >= w {
                false
            } else {
                (edge_flags & EDGE_I444_TOP_HAS_RIGHT) != 0
            };

            if have_topright {
                let (buf, o) = dst_top.expect("dst_top");
                let px_have = imin(sz as i32, (w - x - tw) << 2) as usize;

                topleft_out[top + sz..top + sz + px_have]
                    .copy_from_slice(&buf[o + sz..o + sz + px_have]);
                if px_have < sz {
                    let v = topleft_out[top + sz + px_have - 1];
                    topleft_out[top + sz + px_have..top + 2 * sz].fill(v);
                }
            } else {
                let v = topleft_out[top + sz - 1];
                topleft_out[top + sz..top + 2 * sz].fill(v);
            }
        }
    }

    if ed.needs_topleft {
        topleft_out[tl] = if have_left {
            match dst_top {
                Some((buf, o)) if have_top => buf[o - 1],
                _ => dst[d - 1],
            }
        } else {
            match dst_top {
                Some((buf, o)) if have_top => buf[o],
                _ => P::from_i32((1 << bitdepth) >> 1),
            }
        };

        if mode == Z2_PRED && tw + th >= 6 && filter_edge {
            topleft_out[tl] = P::from_i32(
                ((topleft_out[tl - 1].to_i32() + topleft_out[tl + 1].to_i32()) * 5
                    + topleft_out[tl].to_i32() * 6
                    + 8)
                    >> 4,
            );
        }
    }

    mode
}
