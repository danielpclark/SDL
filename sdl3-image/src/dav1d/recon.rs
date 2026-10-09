// Rust translation of src/recon_tmpl.c and src/recon.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Coefficient decoding and block reconstruction (prediction and inverse
//! transforms), and the per-superblock-row in-loop filter driver.
//!
//! `dav1d_read_coef_blocks()` (the first pass of two-pass frame threading)
//! is not needed with one frame at a time.

use super::bitdepth::Pixel;
use super::cdef_apply::cdef_brow;
use super::env::{get_uv_inter_txtp, set_ctx};
use super::headers::*;
use super::internal::*;
use super::intops::*;
use super::intra_edge::*;
use super::ipred::{cfl_ac, cfl_pred, intra_pred, pal_pred};
use super::ipred_prepare::{prepare_intra_edges, sm_flag, sm_uv_flag};
use super::itx::itxfm_add;
use super::levels::*;
use super::lf_apply::{copy_lpf, loopfilter_sbrow_cols, loopfilter_sbrow_rows};
use super::lr_apply::lr_sbrow;
use super::mc;
use super::picture::Dav1dThreadPicture;
use super::refmvs::{RefmvsBlock, RefmvsFrame, RefmvsTile};
use super::scan::scans;
use super::tables::*;
use super::wedge::{ii_masks, wedge_masks};

#[inline]
fn read_golomb(msac: &mut super::msac::MsacContext) -> u32 {
    let mut len = 0;
    let mut val: u32 = 1;

    while msac.decode_bool_equi() == 0 && len < 32 {
        len += 1;
    }
    while len > 0 {
        len -= 1;
        val = (val << 1).wrapping_add(msac.decode_bool_equi());
    }

    val.wrapping_sub(1)
}

/// The OR of the first `n` context bytes (the `MERGE_CTX()` loads).
#[inline]
fn or_bytes(c: &[u8], n: usize) -> u32 {
    c[..n].iter().fold(0u32, |acc, &v| acc | v as u32)
}

#[inline]
fn get_skip_ctx(t_dim: &TxfmInfo, bs: u8, a: &[u8], l: &[u8], chroma: bool, layout: i32) -> usize {
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];

    if chroma {
        let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
        let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
        let not_one_blk = b_dim[2] as i32 - (b_dim[2] != 0 && ss_hor != 0) as i32 > t_dim.lw as i32
            || b_dim[3] as i32 - (b_dim[3] != 0 && ss_ver != 0) as i32 > t_dim.lh as i32;

        // MERGE_CTX(dir, type, no_val): any byte differs from 0x40
        debug_assert!(t_dim.lw <= TX_32X32 && t_dim.lh <= TX_32X32);
        let ca = a[..1 << t_dim.lw].iter().any(|&v| v != 0x40) as usize;
        let cl = l[..1 << t_dim.lh].iter().any(|&v| v != 0x40) as usize;

        7 + not_one_blk as usize * 3 + ca + cl
    } else if b_dim[2] == t_dim.lw && b_dim[3] == t_dim.lh {
        0
    } else {
        // MERGE_CTX(dir, type, tx): the OR of all bytes
        let la = or_bytes(a, 1 << t_dim.lw);
        let ll = or_bytes(l, 1 << t_dim.lh);

        SKIP_CTX[umin(la & 0x3F, 4) as usize][umin(ll & 0x3F, 4) as usize] as usize
    }
}

/// `get_dc_sign_ctx()`: the sign of the sum of the dc signs (bits 6-7 of
/// the context bytes, 1 being zero) over the transform's width and height.
#[inline]
fn get_dc_sign_ctx(tx: u8, a: &[u8], l: &[u8]) -> usize {
    let t_dim = &TXFM_DIMENSIONS[tx as usize];
    let (w, h) = (t_dim.w as usize, t_dim.h as usize);
    let mut s = 0i32;
    for &v in &a[..w] {
        s += ((v & 0xC0) >> 6) as i32;
    }
    for &v in &l[..h] {
        s += ((v & 0xC0) >> 6) as i32;
    }
    s -= (w + h) as i32;

    (s != 0) as usize + (s > 0) as usize
}

#[inline]
fn get_lo_ctx(
    levels: &[u8],
    p: usize,
    tx_class: u8,
    hi_mag: &mut u32,
    ctx_offsets: Option<&[[u8; 5]; 5]>,
    x: u32,
    y: u32,
    stride: usize,
) -> u32 {
    let mut mag = levels[p + 0 * stride + 1] as u32 + levels[p + 1 * stride + 0] as u32;
    let offset;
    if tx_class == TX_CLASS_2D {
        mag += levels[p + 1 * stride + 1] as u32;
        *hi_mag = mag;
        mag += levels[p + 0 * stride + 2] as u32 + levels[p + 2 * stride + 0] as u32;
        offset = ctx_offsets.expect("2D context offsets")[umin(y, 4) as usize][umin(x, 4) as usize]
            as u32;
    } else {
        mag += levels[p + 0 * stride + 2] as u32;
        *hi_mag = mag;
        mag += levels[p + 0 * stride + 3] as u32 + levels[p + 0 * stride + 4] as u32;
        offset = 26 + if y > 1 { 10 } else { y * 5 };
    }
    offset + if mag > 512 { 4 } else { (mag + 64) >> 7 }
}

/// The frame-level inputs of `decode_coefs()`.
struct CoefFrame<'a> {
    frame_hdr: &'a Dav1dFrameHeader,
    layout: i32,
    bpc: i32,
    /// `ts->dq[b->seg_id][plane]`
    dq_tbl: [u16; 2],
    /// `f->qm[tx][plane]`
    qm: Option<&'static [u8]>,
}

/// `decode_coefs()`: returns the eob (-1 if all coefficients are zero)
/// and the context value (`*res_ctx`).
fn decode_coefs<P: Pixel>(
    ts: &mut Dav1dTileState,
    levels: &mut [u8; 32 * 34],
    a: &[u8],
    l: &[u8],
    tx: u8,
    bs: u8,
    b: &Av1Block,
    intra: bool,
    plane: usize,
    cf: &mut [i32],
    txtp: &mut u8,
    fr: &CoefFrame<'_>,
) -> (i32, u8) {
    let chroma = plane != 0;
    let frame_hdr = fr.frame_hdr;
    let lossless = frame_hdr.segmentation.lossless[b.seg_id as usize] != 0;
    let t_dim = &TXFM_DIMENSIONS[tx as usize];
    let t_ctx = t_dim.ctx as usize;

    // does this block have any non-zero coefficients
    let sctx = get_skip_ctx(t_dim, bs, a, l, chroma, fr.layout);
    let all_skip = ts
        .msac
        .decode_bool_adapt(&mut ts.cdf.coef.skip[t_ctx][sctx]);
    if all_skip != 0 {
        *txtp = if lossless { WHT_WHT } else { DCT_DCT }; /* lossless ? WHT_WHT : DCT_DCT */
        return (-1, 0x40);
    }

    // transform type (chroma: derived, luma: explicitly coded)
    if lossless {
        debug_assert!(t_dim.max == TX_4X4);
        *txtp = WHT_WHT;
    } else if t_dim.max as i32 + intra as i32 >= TX_64X64 as i32 {
        *txtp = DCT_DCT;
    } else if chroma {
        // inferred from either the luma txtp (inter) or a LUT (intra)
        *txtp = if intra {
            TXTP_FROM_UVMODE[b.uv_mode as usize]
        } else {
            get_uv_inter_txtp(t_dim, *txtp)
        };
    } else if frame_hdr.segmentation.qidx[b.seg_id as usize] == 0 {
        // In libaom, lossless is checked by a literal qidx == 0, but not all
        // such blocks are actually lossless. The remainder gets an implicit
        // transform type (for luma)
        *txtp = DCT_DCT;
    } else {
        let idx;
        if intra {
            let y_mode_nofilt = if b.y_mode == FILTER_PRED {
                FILTER_MODE_TO_Y_MODE[b.y_angle as usize]
            } else {
                b.y_mode
            } as usize;
            if frame_hdr.reduced_txtp_set != 0 || t_dim.min == TX_16X16 {
                idx = ts.msac.decode_symbol_adapt4(
                    &mut ts.cdf.m.txtp_intra2[t_dim.min as usize][y_mode_nofilt],
                    4,
                );
                *txtp = TX_TYPES_PER_SET[idx as usize + 0];
            } else {
                idx = ts.msac.decode_symbol_adapt8(
                    &mut ts.cdf.m.txtp_intra1[t_dim.min as usize][y_mode_nofilt],
                    6,
                );
                *txtp = TX_TYPES_PER_SET[idx as usize + 5];
            }
        } else if frame_hdr.reduced_txtp_set != 0 || t_dim.max == TX_32X32 {
            idx = ts
                .msac
                .decode_bool_adapt(&mut ts.cdf.m.txtp_inter3[t_dim.min as usize]);
            *txtp = (idx.wrapping_sub(1) & IDTX as u32) as u8; /* idx ? DCT_DCT : IDTX */
        } else if t_dim.min == TX_16X16 {
            idx = ts.msac.decode_symbol_adapt16(&mut ts.cdf.m.txtp_inter2, 11);
            *txtp = TX_TYPES_PER_SET[idx as usize + 12];
        } else {
            idx = ts
                .msac
                .decode_symbol_adapt16(&mut ts.cdf.m.txtp_inter1[t_dim.min as usize], 15);
            *txtp = TX_TYPES_PER_SET[idx as usize + 24];
        }
    }

    // find end-of-block (eob)
    let tx2dszctx = imin(t_dim.lw as i32, TX_32X32 as i32) + imin(t_dim.lh as i32, TX_32X32 as i32);
    let tx_class = TX_TYPE_CLASS[*txtp as usize];
    let is_1d = (tx_class != TX_CLASS_2D) as usize;
    let c = chroma as usize;
    let n = 4 + tx2dszctx as usize;
    let coef = &mut ts.cdf.coef;
    let eob_bin = match tx2dszctx {
        0 => ts
            .msac
            .decode_symbol_adapt4(&mut coef.eob_bin_16[c][is_1d], n),
        1 => ts
            .msac
            .decode_symbol_adapt8(&mut coef.eob_bin_32[c][is_1d], n),
        2 => ts
            .msac
            .decode_symbol_adapt8(&mut coef.eob_bin_64[c][is_1d], n),
        3 => ts
            .msac
            .decode_symbol_adapt8(&mut coef.eob_bin_128[c][is_1d], n),
        4 => ts
            .msac
            .decode_symbol_adapt16(&mut coef.eob_bin_256[c][is_1d], n),
        5 => ts.msac.decode_symbol_adapt16(&mut coef.eob_bin_512[c], n),
        _ => ts.msac.decode_symbol_adapt16(&mut coef.eob_bin_1024[c], n),
    } as i32;
    let eob;
    if eob_bin > 1 {
        let eob_hi_bit_cdf = &mut ts.cdf.coef.eob_hi_bit[t_ctx][c][eob_bin as usize];
        let eob_hi_bit = ts.msac.decode_bool_adapt(eob_hi_bit_cdf) as i32;
        eob =
            ((eob_hi_bit | 2) << (eob_bin - 2)) | ts.msac.decode_bools((eob_bin - 2) as u32) as i32;
    } else {
        eob = eob_bin;
    }
    debug_assert!(eob >= 0);

    // base tokens
    let hi_ctx_i = imin(t_ctx as i32, 3) as usize;
    let mut rc: u32;
    let mut dc_tok: u32;

    if eob != 0 {
        let sw = imin(t_dim.w as i32, 8) as u32;
        let sh = imin(t_dim.h as i32, 8) as u32;

        /* eob */
        let mut ctx = 1 + (eob as u32 > sw * sh * 2) as usize + (eob as u32 > sw * sh * 4) as usize;
        let eob_tok = ts
            .msac
            .decode_symbol_adapt4(&mut ts.cdf.coef.eob_base_tok[t_ctx][c][ctx], 2);
        let mut tok = eob_tok + 1;
        let mut level_tok = tok * 0x41;
        let mut mag: u32 = 0;

        let scan;
        let lo_ctx_offsets;
        let stride: usize;
        let shift: u32;
        let shift2: u32;
        let mask: u32;
        match tx_class {
            TX_CLASS_2D => {
                let nonsquare_tx = (tx >= RTX_4X8) as usize;
                lo_ctx_offsets = Some(&LO_CTX_OFFSETS[nonsquare_tx + (tx as usize & nonsquare_tx)]);
                scan = scans(tx);
                stride = 4 * sh as usize;
                shift = if t_dim.lh < 4 { t_dim.lh as u32 + 2 } else { 5 };
                shift2 = 0;
                mask = 4 * sh - 1;
                levels[..stride * (4 * sw as usize + 2)].fill(0);
            }
            TX_CLASS_H => {
                lo_ctx_offsets = None;
                scan = &[][..];
                stride = 16;
                shift = t_dim.lh as u32 + 2;
                shift2 = 0;
                mask = 4 * sh - 1;
                levels[..stride * (4 * sh as usize + 2)].fill(0);
            }
            _ => {
                // TX_CLASS_V
                lo_ctx_offsets = None;
                scan = &[][..];
                stride = 16;
                shift = t_dim.lw as u32 + 2;
                shift2 = t_dim.lh as u32 + 2;
                mask = 4 * sw - 1;
                levels[..stride * (4 * sw as usize + 2)].fill(0);
            }
        }

        // DECODE_COEFS_CLASS(tx_class)
        let (mut x, mut y);
        if tx_class == TX_CLASS_2D {
            rc = scan[eob as usize] as u32;
            x = rc >> shift;
            y = rc & mask;
        } else if tx_class == TX_CLASS_H {
            /* Transposing reduces the stride and padding requirements */
            x = eob as u32 & mask;
            y = eob as u32 >> shift;
            rc = eob as u32;
        } else {
            /* tx_class == TX_CLASS_V */
            x = eob as u32 & mask;
            y = eob as u32 >> shift;
            rc = (x << shift2) | y;
        }
        if eob_tok == 2 {
            ctx = if if tx_class == TX_CLASS_2D {
                (x | y) > 1
            } else {
                y != 0
            } {
                14
            } else {
                7
            };
            tok = ts
                .msac
                .decode_hi_tok(&mut ts.cdf.coef.br_tok[hi_ctx_i][c][ctx]);
            level_tok = tok + (3 << 6);
        }
        cf[rc as usize] = (tok << 11) as i32;
        levels[x as usize * stride + y as usize] = level_tok as u8;
        let mut i = eob - 1;
        while i > 0 {
            /* ac */
            let rc_i;
            if tx_class == TX_CLASS_2D {
                rc_i = scan[i as usize] as u32;
                x = rc_i >> shift;
                y = rc_i & mask;
            } else if tx_class == TX_CLASS_H {
                x = i as u32 & mask;
                y = i as u32 >> shift;
                rc_i = i as u32;
            } else {
                /* tx_class == TX_CLASS_V */
                x = i as u32 & mask;
                y = i as u32 >> shift;
                rc_i = (x << shift2) | y;
            }
            debug_assert!(x < 32 && y < 32);
            let level = x as usize * stride + y as usize;
            let ctx = get_lo_ctx(
                levels,
                level,
                tx_class,
                &mut mag,
                lo_ctx_offsets,
                x,
                y,
                stride,
            );
            if tx_class == TX_CLASS_2D {
                y |= x;
            }
            let mut tok = ts
                .msac
                .decode_symbol_adapt4(&mut ts.cdf.coef.base_tok[t_ctx][c][ctx as usize], 3);
            if tok == 3 {
                mag &= 63;
                let ctx = (if y > (tx_class == TX_CLASS_2D) as u32 {
                    14
                } else {
                    7
                }) + if mag > 12 { 6 } else { (mag + 1) >> 1 };
                tok = ts
                    .msac
                    .decode_hi_tok(&mut ts.cdf.coef.br_tok[hi_ctx_i][c][ctx as usize]);
                levels[level] = (tok + (3 << 6)) as u8;
                cf[rc_i as usize] = ((tok << 11) | rc) as i32;
                rc = rc_i;
            } else {
                /* 0x1 for tok, 0x7ff as bitmask for rc, 0x41 for level_tok */
                levels[level] = (tok * 0x41) as u8;
                /* tok ? (tok << 11) | rc : 0 */
                let v = if tok != 0 { (tok << 11) | rc } else { 0 };
                if v != 0 {
                    rc = rc_i;
                }
                cf[rc_i as usize] = v as i32;
            }
            i -= 1;
        }
        /* dc */
        let ctx = if tx_class == TX_CLASS_2D {
            0
        } else {
            get_lo_ctx(levels, 0, tx_class, &mut mag, lo_ctx_offsets, 0, 0, stride)
        };
        dc_tok = ts
            .msac
            .decode_symbol_adapt4(&mut ts.cdf.coef.base_tok[t_ctx][c][ctx as usize], 3);
        if dc_tok == 3 {
            if tx_class == TX_CLASS_2D {
                mag = levels[0 * stride + 1] as u32
                    + levels[1 * stride + 0] as u32
                    + levels[1 * stride + 1] as u32;
            }
            mag &= 63;
            let ctx = if mag > 12 { 6 } else { (mag + 1) >> 1 };
            dc_tok = ts
                .msac
                .decode_hi_tok(&mut ts.cdf.coef.br_tok[hi_ctx_i][c][ctx as usize]);
        }
    } else {
        // dc-only
        let tok_br = ts
            .msac
            .decode_symbol_adapt4(&mut ts.cdf.coef.eob_base_tok[t_ctx][c][0], 2);
        dc_tok = 1 + tok_br;
        if tok_br == 2 {
            dc_tok = ts
                .msac
                .decode_hi_tok(&mut ts.cdf.coef.br_tok[hi_ctx_i][c][0]);
        }
        rc = 0;
    }

    // residual and sign
    let dq_tbl = &fr.dq_tbl;
    let qm_tbl = if *txtp < IDTX { fr.qm } else { None };
    let dq_shift = imax(0, t_ctx as i32 - 2) as u32;
    let cf_max: u32 = !(!127u32 << (if P::BPC8 { 8 } else { fr.bpc as u32 }));
    let mut cul_level: u32;
    let dc_sign_level: u32;

    // (the C code jumps into the ac loops with goto when dc_tok is 0)
    let mut do_ac = true;
    if dc_tok == 0 {
        cul_level = 0;
        dc_sign_level = 1 << 6;
    } else {
        let dc_sign_ctx = get_dc_sign_ctx(tx, a, l);
        let dc_sign_cdf = &mut ts.cdf.coef.dc_sign[c][dc_sign_ctx];
        let dc_sign = ts.msac.decode_bool_adapt(dc_sign_cdf);

        let mut dc_dq = dq_tbl[0] as u32;
        dc_sign_level = (dc_sign.wrapping_sub(1)) & (2 << 6);

        if let Some(qm_tbl) = qm_tbl {
            dc_dq = (dc_dq * qm_tbl[0] as u32 + 16) >> 5;

            if dc_tok == 15 {
                dc_tok = read_golomb(&mut ts.msac).wrapping_add(15);

                dc_tok &= 0xfffff;
                dc_dq = dc_dq.wrapping_mul(dc_tok) & 0xffffff;
            } else {
                dc_dq *= dc_tok;
                debug_assert!(dc_dq <= 0xffffff);
            }
            cul_level = dc_tok;
            dc_dq >>= dq_shift;
            dc_dq = umin(dc_dq, cf_max + dc_sign);
            cf[0] = if dc_sign != 0 {
                -(dc_dq as i32)
            } else {
                dc_dq as i32
            };
        } else {
            // non-qmatrix is the common case and allows for additional optimizations
            if dc_tok == 15 {
                dc_tok = read_golomb(&mut ts.msac).wrapping_add(15);

                dc_tok &= 0xfffff;
                dc_dq = (dc_dq.wrapping_mul(dc_tok) & 0xffffff) >> dq_shift;
                dc_dq = umin(dc_dq, cf_max + dc_sign);
            } else {
                dc_dq = (dc_dq * dc_tok) >> dq_shift;
                debug_assert!(dc_dq <= cf_max);
            }
            cul_level = dc_tok;
            cf[0] = if dc_sign != 0 {
                -(dc_dq as i32)
            } else {
                dc_dq as i32
            };
        }
        if rc == 0 {
            do_ac = false;
        }
    }

    if do_ac {
        let ac_dq = dq_tbl[1] as u32;
        if let Some(qm_tbl) = qm_tbl {
            // ac_qm:
            loop {
                let sign = ts.msac.decode_bool_equi();
                let rc_tok = cf[rc as usize] as u32;
                let mut tok;
                let mut dq = (ac_dq * qm_tbl[rc as usize] as u32 + 16) >> 5;

                if rc_tok >= (15 << 11) {
                    tok = read_golomb(&mut ts.msac).wrapping_add(15);

                    tok &= 0xfffff;
                    dq = dq.wrapping_mul(tok) & 0xffffff;
                } else {
                    tok = rc_tok >> 11;
                    dq *= tok;
                    debug_assert!(dq <= 0xffffff);
                }
                cul_level = cul_level.wrapping_add(tok);
                dq >>= dq_shift;
                let dq_sat = umin(dq, cf_max + sign);
                cf[rc as usize] = if sign != 0 {
                    -(dq_sat as i32)
                } else {
                    dq_sat as i32
                };

                rc = rc_tok & 0x3ff;
                if rc == 0 {
                    break;
                }
            }
        } else {
            // ac_noqm:
            loop {
                let sign = ts.msac.decode_bool_equi();
                let rc_tok = cf[rc as usize] as u32;
                let mut tok;
                let mut dq;

                // residual
                if rc_tok >= (15 << 11) {
                    tok = read_golomb(&mut ts.msac).wrapping_add(15);

                    // coefficient parsing, see 5.11.39
                    tok &= 0xfffff;

                    // dequant, see 7.12.3
                    dq = (ac_dq.wrapping_mul(tok) & 0xffffff) >> dq_shift;
                    dq = umin(dq, cf_max + sign);
                } else {
                    // cannot exceed cf_max, so we can avoid the clipping
                    tok = rc_tok >> 11;
                    dq = (ac_dq * tok) >> dq_shift;
                    debug_assert!(dq <= cf_max);
                }
                cul_level = cul_level.wrapping_add(tok);
                cf[rc as usize] = if sign != 0 { -(dq as i32) } else { dq as i32 };

                rc = rc_tok & 0x3ff; // next non-zero rc, zero if eob
                if rc == 0 {
                    break;
                }
            }
        }
    }

    // context
    (eob, (umin(cul_level, 63) | dc_sign_level) as u8)
}

/// The block's inputs for coefficient decoding from the frame context.
fn coef_frame<'a>(
    f: &'a Dav1dFrameContext,
    ts: &Dav1dTileState,
    frame_hdr: &'a Dav1dFrameHeader,
    b: &Av1Block,
    tx: u8,
    plane: usize,
) -> CoefFrame<'a> {
    let dq = match ts.dq {
        TileDq::Frame => &f.dq,
        TileDq::Tile => &ts.dqmem,
    };
    CoefFrame {
        frame_hdr,
        layout: f.cur.p.layout,
        bpc: f.cur.p.bpc,
        dq_tbl: dq[b.seg_id as usize][plane],
        qm: f.qm[tx as usize][plane],
    }
}

/// `read_coef_tree()`: `dst` is the offset of the transform block in the
/// luma plane.
fn read_coef_tree<P: Pixel>(
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    ts: &mut Dav1dTileState,
    frame_hdr: &Dav1dFrameHeader,
    bs: u8,
    b: &Av1Block,
    ytx: u8,
    depth: usize,
    tx_split: &[u16; 2],
    x_off: i32,
    y_off: i32,
    dst: usize,
) {
    let t_dim = &TXFM_DIMENSIONS[ytx as usize];
    let txw = t_dim.w as i32;
    let txh = t_dim.h as i32;
    let stride = f.cur_px.as_ref().expect("picture").stride[0];

    /* y_off can be larger than 3 since lossless blocks use TX_4X4 but can't
     * be splitted. Aviods an undefined left shift. */
    if depth < 2 && tx_split[depth] != 0 && tx_split[depth] & (1 << (y_off * 4 + x_off)) != 0 {
        let sub = t_dim.sub;
        let sub_t_dim = &TXFM_DIMENSIONS[sub as usize];
        let txsw = sub_t_dim.w as i32;
        let txsh = sub_t_dim.h as i32;

        read_coef_tree::<P>(
            f,
            t,
            ts,
            frame_hdr,
            bs,
            b,
            sub,
            depth + 1,
            tx_split,
            x_off * 2 + 0,
            y_off * 2 + 0,
            dst,
        );
        t.bx += txsw;
        if txw >= txh && t.bx < f.bw {
            read_coef_tree::<P>(
                f,
                t,
                ts,
                frame_hdr,
                bs,
                b,
                sub,
                depth + 1,
                tx_split,
                x_off * 2 + 1,
                y_off * 2 + 0,
                dst + 4 * txsw as usize,
            );
        }
        t.bx -= txsw;
        t.by += txsh;
        if txh >= txw && t.by < f.bh {
            let dst = dst + 4 * txsh as usize * stride;
            read_coef_tree::<P>(
                f,
                t,
                ts,
                frame_hdr,
                bs,
                b,
                sub,
                depth + 1,
                tx_split,
                x_off * 2 + 0,
                y_off * 2 + 1,
                dst,
            );
            t.bx += txsw;
            if txw >= txh && t.bx < f.bw {
                read_coef_tree::<P>(
                    f,
                    t,
                    ts,
                    frame_hdr,
                    bs,
                    b,
                    sub,
                    depth + 1,
                    tx_split,
                    x_off * 2 + 1,
                    y_off * 2 + 1,
                    dst + 4 * txsw as usize,
                );
            }
            t.bx -= txsw;
        }
        t.by -= txsh;
    } else {
        let bx4 = (t.bx & 31) as usize;
        let by4 = (t.by & 31) as usize;
        let mut txtp = 0u8;

        let fr = coef_frame(f, ts, frame_hdr, b, ytx, 0);
        let (eob, cf_ctx) = decode_coefs::<P>(
            ts,
            &mut t.scratch.levels,
            &f.a[t.a].lcoef[bx4..],
            &t.l.lcoef[by4..],
            ytx,
            bs,
            b,
            false,
            0,
            &mut t.cf,
            &mut txtp,
            &fr,
        );
        set_ctx(&mut t.l.lcoef, by4, imin(txh, f.bh - t.by) as usize, cf_ctx);
        set_ctx(
            &mut f.a[t.a].lcoef,
            bx4,
            imin(txw, f.bw - t.bx) as usize,
            cf_ctx,
        );
        let mut m = by4 * 32 + bx4;
        for _ in 0..txh {
            t.txtp_map[m..m + txw as usize].fill(txtp);
            m += 32;
        }
        if eob >= 0 {
            let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
            itxfm_add::<P>(
                ytx,
                txtp,
                &mut planes[0],
                dst,
                stride,
                &mut t.cf,
                eob,
                f.bitdepth_max,
            );
        }
    }
}

/// The frame dimensions motion compensation needs.
#[derive(Clone, Copy)]
struct McFrame {
    cur_w: i32,
    cur_h: i32,
    bw: i32,
    bh: i32,
    layout: i32,
    bitdepth_max: i32,
}

/// The destination of `mc()`: a pixel block (`dst8`; the slice, offset
/// and stride) or an intermediate buffer (`dst16`).
enum McDst<'a, P> {
    Px(&'a mut [P], usize, usize),
    Tmp(&'a mut [i16]),
}

/// `mc()`: `refp` is `None` for intra block copy, the reference then being
/// the current picture (`&f->sr_cur`), whose plane is the destination.
fn mc<P: Pixel>(
    emu_edge_buf: &mut [P],
    dst: McDst<'_, P>,
    fr: &McFrame,
    svc: &[[ScalableMotionParams; 2]; 7],
    bw4: i32,
    bh4: i32,
    bx: i32,
    by: i32,
    pl: usize,
    mv: Mv,
    refp: Option<&Dav1dThreadPicture>,
    refidx: usize,
    filter_2d: u8,
) -> Result<(), ()> {
    let ss_ver = (pl != 0 && fr.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (pl != 0 && fr.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let h_mul = 4 >> ss_hor;
    let v_mul = 4 >> ss_ver;
    let mvx = mv.x as i32;
    let mvy = mv.y as i32;
    let mx = mvx & (15 >> (ss_hor == 0) as i32);
    let my = mvy & (15 >> (ss_ver == 0) as i32);
    let ref_data = refp.map(|r| r.p.data.as_deref().expect("reference picture"));
    let (ref_w, ref_h) = match refp {
        Some(r) => (r.p.p.w, r.p.p.h),
        None => (fr.cur_w, fr.cur_h),
    };

    if ref_w == fr.cur_w && ref_h == fr.cur_h {
        let dx = bx * h_mul + (mvx >> (3 + ss_hor));
        let dy = by * v_mul + (mvy >> (3 + ss_ver));
        let (w, h);

        if refp.is_some() {
            // i.e. not for intrabc
            w = (fr.cur_w + ss_hor) >> ss_hor;
            h = (fr.cur_h + ss_ver) >> ss_ver;
        } else {
            w = fr.bw * 4 >> ss_hor;
            h = fr.bh * 4 >> ss_ver;
        }
        let mxb = (mx != 0) as i32;
        let myb = (my != 0) as i32;
        let (src, s, src_stride): (&[P], usize, usize);
        // (intra block copy reads the current picture, which is also the
        // destination: its pixels are always copied to the edge emulation
        // buffer first, which gives the same pixels as reading them in
        // place)
        if refp.is_none()
            || dx < mxb * 3
            || dy < myb * 3
            || dx + bw4 * h_mul + mxb * 4 > w
            || dy + bh4 * v_mul + myb * 4 > h
        {
            {
                let (ref_plane, ref_stride): (&[P], usize) = match (ref_data, &dst) {
                    (Some(d), _) => (&P::planes(d)[pl], d.stride[(pl != 0) as usize]),
                    (None, McDst::Px(p, _, st)) => (&p[..], *st),
                    (None, McDst::Tmp(_)) => unreachable!("intra block copy is not compound"),
                };
                mc::emu_edge::<P>(
                    (bw4 * h_mul + mxb * 7) as isize,
                    (bh4 * v_mul + myb * 7) as isize,
                    w as isize,
                    h as isize,
                    (dx - mxb * 3) as isize,
                    (dy - myb * 3) as isize,
                    emu_edge_buf,
                    192,
                    ref_plane,
                    0,
                    ref_stride,
                );
            }
            src = emu_edge_buf;
            s = (192 * myb * 3 + mxb * 3) as usize;
            src_stride = 192;
        } else {
            let d = ref_data.expect("reference picture");
            src = &P::planes(d)[pl];
            src_stride = d.stride[(pl != 0) as usize];
            s = src_stride * dy as usize + dx as usize;
        }

        match dst {
            McDst::Px(dst, d, dst_stride) => {
                mc::mc::<P>(
                    filter_2d,
                    dst,
                    d,
                    dst_stride,
                    src,
                    s,
                    src_stride,
                    (bw4 * h_mul) as usize,
                    (bh4 * v_mul) as usize,
                    mx << (ss_hor == 0) as i32,
                    my << (ss_ver == 0) as i32,
                    fr.bitdepth_max,
                );
            }
            McDst::Tmp(tmp) => {
                mc::mct::<P>(
                    filter_2d,
                    tmp,
                    src,
                    s,
                    src_stride,
                    (bw4 * h_mul) as usize,
                    (bh4 * v_mul) as usize,
                    mx << (ss_hor == 0) as i32,
                    my << (ss_ver == 0) as i32,
                    fr.bitdepth_max,
                );
            }
        }
    } else {
        let d = ref_data.expect("reference picture");

        let orig_pos_y = (by * v_mul << 4) + mvy * (1 << (ss_ver == 0) as i32);
        let orig_pos_x = (bx * h_mul << 4) + mvx * (1 << (ss_hor == 0) as i32);
        let scale_mv = |val: i32, scale: i32| -> i32 {
            let tmp = val as i64 * scale as i64 + (scale as i64 - 0x4000) * 8;
            apply_sign64(((tmp.abs() + 128) >> 8) as i32, tmp) + 32
        };
        let pos_x = scale_mv(orig_pos_x, svc[refidx][0].scale);
        let pos_y = scale_mv(orig_pos_y, svc[refidx][1].scale);
        let left = pos_x >> 10;
        let top = pos_y >> 10;
        let right = ((pos_x + (bw4 * h_mul - 1) * svc[refidx][0].step) >> 10) + 1;
        let bottom = ((pos_y + (bh4 * v_mul - 1) * svc[refidx][1].step) >> 10) + 1;

        let w = (ref_w + ss_hor) >> ss_hor;
        let h = (ref_h + ss_ver) >> ss_ver;
        let (src, s, src_stride): (&[P], usize, usize);
        if left < 3 || top < 3 || right + 4 > w || bottom + 4 > h {
            mc::emu_edge::<P>(
                (right - left + 7) as isize,
                (bottom - top + 7) as isize,
                w as isize,
                h as isize,
                (left - 3) as isize,
                (top - 3) as isize,
                emu_edge_buf,
                320,
                &P::planes(d)[pl],
                0,
                d.stride[(pl != 0) as usize],
            );
            src = emu_edge_buf;
            s = 320 * 3 + 3;
            src_stride = 320;
        } else {
            src = &P::planes(d)[pl];
            src_stride = d.stride[(pl != 0) as usize];
            s = src_stride * top as usize + left as usize;
        }

        match dst {
            McDst::Px(dst, dd, dst_stride) => {
                mc::mc_scaled::<P>(
                    filter_2d,
                    dst,
                    dd,
                    dst_stride,
                    src,
                    s,
                    src_stride,
                    (bw4 * h_mul) as usize,
                    (bh4 * v_mul) as usize,
                    pos_x & 0x3ff,
                    pos_y & 0x3ff,
                    svc[refidx][0].step,
                    svc[refidx][1].step,
                    fr.bitdepth_max,
                );
            }
            McDst::Tmp(tmp) => {
                mc::mct_scaled::<P>(
                    filter_2d,
                    tmp,
                    src,
                    s,
                    src_stride,
                    (bw4 * h_mul) as usize,
                    (bh4 * v_mul) as usize,
                    pos_x & 0x3ff,
                    pos_y & 0x3ff,
                    svc[refidx][0].step,
                    svc[refidx][1].step,
                    fr.bitdepth_max,
                );
            }
        }
    }

    Ok(())
}

/// `r[k][x]` of the task's refmvs rows.
#[inline]
fn rblk<'a>(rf: &'a RefmvsFrame, rt: &RefmvsTile, by: i32, k: i32, x: i32) -> &'a RefmvsBlock {
    let row = rt.r[((by & 31) + 5 + k) as usize];
    &rf.r[row + x as usize]
}

/// `obmc()`: `dst[d..]` is the block in plane `pl`.
fn obmc<P: Pixel>(
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    ts: &Dav1dTileState,
    fr: &McFrame,
    d: usize,
    dst_stride: usize,
    b_dim: &[u8; 4],
    pl: usize,
    bx4: usize,
    by4: usize,
    w4: i32,
    h4: i32,
) -> Result<(), ()> {
    debug_assert!(t.bx & 1 == 0 && t.by & 1 == 0);
    let ss_ver = (pl != 0 && fr.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (pl != 0 && fr.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let h_mul = 4 >> ss_hor;
    let v_mul = 4 >> ss_ver;
    let tpx = P::task_px(&mut t.px);
    let dst = &mut P::planes_mut(f.cur_px.as_mut().expect("picture"))[pl];

    if t.by > ts.tiling.row_start
        && (pl == 0 || b_dim[0] as i32 * h_mul + b_dim[1] as i32 * v_mul >= 16)
    {
        let mut i = 0;
        let mut x = 0;
        while x < w4 && i < imin(b_dim[2] as i32, 4) {
            // only odd blocks are considered for overlap handling, hence +1
            let a_r = *rblk(&f.rf, &t.rt, t.by, -1, t.bx + x + 1);
            let a_b_dim = &BLOCK_DIMENSIONS[a_r.bs as usize];
            let step4 = iclip(a_b_dim[0] as i32, 2, 16);

            if a_r.ref_.ref_[0] > 0 {
                let ow4 = imin(step4, b_dim[0] as i32);
                let oh4 = imin(b_dim[1] as i32, 16) >> 1;
                let a = &f.a[t.a];
                let filt = FILTER_2D[a.filter[1][bx4 + x as usize + 1] as usize]
                    [a.filter[0][bx4 + x as usize + 1] as usize];
                mc::<P>(
                    &mut tpx.emu_edge,
                    McDst::Px(&mut tpx.lap, 0, (ow4 * h_mul) as usize),
                    fr,
                    &f.svc,
                    ow4,
                    (oh4 * 3 + 3) >> 2,
                    t.bx + x,
                    t.by,
                    pl,
                    a_r.mv.mv[0],
                    Some(&f.refp[a_r.ref_.ref_[0] as usize - 1]),
                    a_r.ref_.ref_[0] as usize - 1,
                    filt,
                )?;
                mc::blend_h::<P>(
                    dst,
                    d + (x * h_mul) as usize,
                    dst_stride,
                    &tpx.lap,
                    (h_mul * ow4) as usize,
                    (v_mul * oh4) as usize,
                );
                i += 1;
            }
            x += step4;
        }
    }

    if t.bx > ts.tiling.col_start {
        let mut i = 0;
        let mut y = 0;
        while y < h4 && i < imin(b_dim[3] as i32, 4) {
            // only odd blocks are considered for overlap handling, hence +1
            let l_r = *rblk(&f.rf, &t.rt, t.by, y + 1, t.bx - 1);
            let l_b_dim = &BLOCK_DIMENSIONS[l_r.bs as usize];
            let step4 = iclip(l_b_dim[1] as i32, 2, 16);

            if l_r.ref_.ref_[0] > 0 {
                let ow4 = imin(b_dim[0] as i32, 16) >> 1;
                let oh4 = imin(step4, b_dim[1] as i32);
                let filt = FILTER_2D[t.l.filter[1][by4 + y as usize + 1] as usize]
                    [t.l.filter[0][by4 + y as usize + 1] as usize];
                mc::<P>(
                    &mut tpx.emu_edge,
                    McDst::Px(&mut tpx.lap, 0, (h_mul * ow4) as usize),
                    fr,
                    &f.svc,
                    ow4,
                    oh4,
                    t.bx,
                    t.by + y,
                    pl,
                    l_r.mv.mv[0],
                    Some(&f.refp[l_r.ref_.ref_[0] as usize - 1]),
                    l_r.ref_.ref_[0] as usize - 1,
                    filt,
                )?;
                mc::blend_v::<P>(
                    dst,
                    d + (y * v_mul) as usize * dst_stride,
                    dst_stride,
                    &tpx.lap,
                    (h_mul * ow4) as usize,
                    (v_mul * oh4) as usize,
                );
                i += 1;
            }
            y += step4;
        }
    }
    Ok(())
}

/// The destination of `warp_affine()`: `dst8` (slice, offset, stride) or
/// `dst16` (slice, stride).
enum WarpDst<'a, P> {
    Px(&'a mut [P], usize, usize),
    Tmp(&'a mut [i16], usize),
}

fn warp_affine<P: Pixel>(
    emu_edge_buf: &mut [P],
    dst: WarpDst<'_, P>,
    fr: &McFrame,
    t_bx: i32,
    t_by: i32,
    b_dim: &[u8; 4],
    pl: usize,
    refp: &Dav1dThreadPicture,
    wmp: &Dav1dWarpedMotionParams,
) -> Result<(), ()> {
    let ss_ver = (pl != 0 && fr.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (pl != 0 && fr.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let h_mul = 4 >> ss_hor;
    let v_mul = 4 >> ss_ver;
    debug_assert!((b_dim[0] as i32 * h_mul) & 7 == 0 && (b_dim[1] as i32 * v_mul) & 7 == 0);
    let mat = &wmp.matrix;
    let width = (refp.p.p.w + ss_hor) >> ss_hor;
    let height = (refp.p.p.h + ss_ver) >> ss_ver;
    let rd = refp.p.data.as_deref().expect("reference picture");
    let ref_plane = &P::planes(rd)[pl];
    let ref_stride0 = rd.stride[(pl != 0) as usize];
    let (mut dst8, mut dst16) = (None, None);
    match dst {
        WarpDst::Px(p, o, s) => dst8 = Some((p, o, s)),
        WarpDst::Tmp(p, s) => dst16 = Some((p, 0usize, s)),
    }

    let mut y = 0;
    while y < b_dim[1] as i32 * v_mul {
        let src_y = t_by * 4 + ((y + 4) << ss_ver);
        let mat3_y = mat[3] as i64 * src_y as i64 + mat[0] as i64;
        let mat5_y = mat[5] as i64 * src_y as i64 + mat[1] as i64;
        let mut x = 0;
        while x < b_dim[0] as i32 * h_mul {
            // calculate transformation relative to center of 8x8 block in
            // luma pixel units
            let src_x = t_bx * 4 + ((x + 4) << ss_hor);
            let mvx = (mat[2] as i64 * src_x as i64 + mat3_y) >> ss_hor;
            let mvy = (mat[4] as i64 * src_x as i64 + mat5_y) >> ss_ver;

            let dx = (mvx >> 16) as i32 - 4;
            let mx = (((mvx as i32) & 0xffff) - wmp.alpha() * 4 - wmp.beta() * 7) & !0x3f;
            let dy = (mvy >> 16) as i32 - 4;
            let my = (((mvy as i32) & 0xffff) - wmp.gamma() * 4 - wmp.delta() * 4) & !0x3f;

            let (src, s, src_stride): (&[P], usize, usize);
            if dx < 3 || dx + 8 + 4 > width || dy < 3 || dy + 8 + 4 > height {
                mc::emu_edge::<P>(
                    15,
                    15,
                    width as isize,
                    height as isize,
                    (dx - 3) as isize,
                    (dy - 3) as isize,
                    emu_edge_buf,
                    32,
                    ref_plane,
                    0,
                    ref_stride0,
                );
                src = emu_edge_buf;
                s = 32 * 3 + 3;
                src_stride = 32;
            } else {
                src = ref_plane;
                src_stride = ref_stride0;
                s = src_stride * dy as usize + dx as usize;
            }
            if let Some((tmp, o, ts)) = dst16.as_mut() {
                mc::warp8x8t::<P>(
                    tmp,
                    *o + x as usize,
                    *ts,
                    src,
                    s,
                    src_stride,
                    &wmp.abcd,
                    mx,
                    my,
                    fr.bitdepth_max,
                );
            } else if let Some((p, o, st)) = dst8.as_mut() {
                mc::warp8x8::<P>(
                    p,
                    *o + x as usize,
                    *st,
                    src,
                    s,
                    src_stride,
                    &wmp.abcd,
                    mx,
                    my,
                    fr.bitdepth_max,
                );
            }
            x += 8;
        }
        if let Some((_, o, st)) = dst8.as_mut() {
            *o += 8 * *st;
        } else if let Some((_, o, st)) = dst16.as_mut() {
            *o += 8 * *st;
        }
        y += 8;
    }
    Ok(())
}

/// `top_sb_edge`: the pre-filter bottom row of the superblock row above in
/// `f->ipred_edge[plane]`, for blocks on a superblock row's top edge.
#[inline]
fn top_sb_edge<'a, P: Pixel>(
    fpx: &'a FramePixBufs<P>,
    plane: usize,
    sb128w: i32,
    sb_shift: i32,
    by: i32,
    on_top: bool,
) -> super::ipred_prepare::TopSbEdge<'a, P> {
    if on_top {
        let sby = by >> sb_shift;
        // (sby is 0 only for the frame's top row, where it is not read)
        Some((
            &fpx.ipred_edge[plane][..],
            (sb128w * 128 * (sby - 1)) as isize,
        ))
    } else {
        None
    }
}

/// `dav1d_recon_b_intra()`
pub(crate) fn recon_b_intra<P: Pixel>(
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    ts: &mut Dav1dTileState,
    bs: u8,
    intra_edge_flags: u8,
    b: &Av1Block,
) {
    let seq_hdr = f.seq_hdr.clone().expect("sequence header");
    let frame_hdr_arc = f.frame_hdr.clone().expect("frame header");
    let frame_hdr: &Dav1dFrameHeader = &frame_hdr_arc;
    let bx4 = (t.bx & 31) as usize;
    let by4 = (t.by & 31) as usize;
    let layout = f.cur.p.layout;
    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let cbx4 = bx4 >> ss_hor;
    let cby4 = by4 >> ss_ver;
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];
    let bw4 = b_dim[0] as i32;
    let bh4 = b_dim[1] as i32;
    let w4 = imin(bw4, f.bw - t.bx);
    let h4 = imin(bh4, f.bh - t.by);
    let cw4 = (w4 + ss_hor) >> ss_hor;
    let ch4 = (h4 + ss_ver) >> ss_ver;
    let has_chroma = layout != DAV1D_PIXEL_LAYOUT_I400
        && (bw4 > ss_hor || t.bx & 1 != 0)
        && (bh4 > ss_ver || t.by & 1 != 0);
    let t_dim = &TXFM_DIMENSIONS[b.tx as usize];
    let uv_t_dim = &TXFM_DIMENSIONS[b.uvtx as usize];
    let bdmax = f.bitdepth_max;
    let stride0 = f.cur_px.as_ref().expect("picture").stride[0];
    let stride1 = f.cur_px.as_ref().expect("picture").stride[1];
    let sb_step = f.sb_step;
    let sb_shift = f.sb_shift;
    let sb128w = f.sb128w;

    // coefficient coding
    let edge = 128; // bitfn(t->scratch.edge) + 128
    let cbw4 = (bw4 + ss_hor) >> ss_hor;
    let cbh4 = (bh4 + ss_ver) >> ss_ver;

    let intra_edge_filter_flag = seq_hdr.intra_edge_filter << 10;

    let mut init_y = 0;
    while init_y < h4 {
        let sub_h4 = imin(h4, 16 + init_y);
        let sub_ch4 = imin(ch4, (init_y + 16) >> ss_ver);
        let mut init_x = 0;
        while init_x < w4 {
            if b.pal_sz[0] != 0 {
                let dst = 4 * (t.by as usize * stride0 + t.bx as usize);
                let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                pal_pred::<P>(
                    &mut planes[0],
                    dst,
                    stride0,
                    &t.scratch.pal[0],
                    &t.scratch.pal_idx,
                    bw4 * 4,
                    bh4 * 4,
                );
            }

            let intra_flags = sm_flag(&f.a[t.a], bx4) | sm_flag(&t.l, by4) | intra_edge_filter_flag;
            let sb_has_tr = if init_x + 16 < w4 {
                true
            } else if init_y != 0 {
                false
            } else {
                intra_edge_flags & EDGE_I444_TOP_HAS_RIGHT != 0
            };
            let sb_has_bl = if init_x != 0 {
                false
            } else if init_y + 16 < h4 {
                true
            } else {
                intra_edge_flags & EDGE_I444_LEFT_HAS_BOTTOM != 0
            };
            let sub_w4 = imin(w4, init_x + 16);
            let mut y = init_y;
            t.by += init_y;
            while y < sub_h4 {
                let mut dst = 4 * (t.by as usize * stride0 + (t.bx + init_x) as usize);
                let mut x = init_x;
                t.bx += init_x;
                while x < sub_w4 {
                    if b.pal_sz[0] == 0 {
                        let mut angle = b.y_angle as i32;
                        let edge_flags =
                            (if (y > init_y || !sb_has_tr) && (x + t_dim.w as i32 >= sub_w4) {
                                0
                            } else {
                                EDGE_I444_TOP_HAS_RIGHT
                            }) | (if x > init_x || (!sb_has_bl && y + t_dim.h as i32 >= sub_h4) {
                                0
                            } else {
                                EDGE_I444_LEFT_HAS_BOTTOM
                            });
                        let fpx = P::frame_px(&f.px);
                        let tse =
                            top_sb_edge(fpx, 0, sb128w, sb_shift, t.by, t.by & (sb_step - 1) == 0);
                        let tpx = P::task_px(&mut t.px);
                        let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                        let m = prepare_intra_edges::<P>(
                            t.bx,
                            t.bx > ts.tiling.col_start,
                            t.by,
                            t.by > ts.tiling.row_start,
                            ts.tiling.col_end,
                            ts.tiling.row_end,
                            edge_flags,
                            &planes[0],
                            dst,
                            stride0,
                            tse,
                            b.y_mode,
                            &mut angle,
                            t_dim.w as i32,
                            t_dim.h as i32,
                            seq_hdr.intra_edge_filter != 0,
                            &mut tpx.edge,
                            edge,
                            bdmax,
                        );
                        intra_pred::<P>(
                            m,
                            &mut planes[0],
                            dst,
                            stride0,
                            &tpx.edge,
                            edge,
                            t_dim.w as i32 * 4,
                            t_dim.h as i32 * 4,
                            angle | intra_flags,
                            4 * f.bw - 4 * t.bx,
                            4 * f.bh - 4 * t.by,
                            bdmax,
                        );
                    }

                    // skip_y_pred:
                    if b.skip == 0 {
                        let mut txtp = 0u8;
                        let fr = coef_frame(f, ts, frame_hdr, b, b.tx, 0);
                        let (eob, cf_ctx) = decode_coefs::<P>(
                            ts,
                            &mut t.scratch.levels,
                            &f.a[t.a].lcoef[bx4 + x as usize..],
                            &t.l.lcoef[by4 + y as usize..],
                            b.tx,
                            bs,
                            b,
                            true,
                            0,
                            &mut t.cf,
                            &mut txtp,
                            &fr,
                        );
                        set_ctx(
                            &mut t.l.lcoef,
                            by4 + y as usize,
                            imin(t_dim.h as i32, f.bh - t.by) as usize,
                            cf_ctx,
                        );
                        set_ctx(
                            &mut f.a[t.a].lcoef,
                            bx4 + x as usize,
                            imin(t_dim.w as i32, f.bw - t.bx) as usize,
                            cf_ctx,
                        );
                        if eob >= 0 {
                            let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                            itxfm_add::<P>(
                                b.tx,
                                txtp,
                                &mut planes[0],
                                dst,
                                stride0,
                                &mut t.cf,
                                eob,
                                bdmax,
                            );
                        }
                    } else {
                        set_ctx(&mut t.l.lcoef, by4 + y as usize, t_dim.h as usize, 0x40);
                        set_ctx(
                            &mut f.a[t.a].lcoef,
                            bx4 + x as usize,
                            t_dim.w as usize,
                            0x40,
                        );
                    }
                    dst += 4 * t_dim.w as usize;
                    x += t_dim.w as i32;
                    t.bx += t_dim.w as i32;
                }
                t.bx -= x;
                y += t_dim.h as i32;
                t.by += t_dim.h as i32;
            }
            t.by -= y;

            if !has_chroma {
                init_x += 16;
                continue;
            }

            let stride = stride1;

            if b.uv_mode == CFL_PRED {
                debug_assert!(init_x == 0 && init_y == 0);

                let y_src = 4 * (t.bx & !ss_hor) as usize + 4 * (t.by & !ss_ver) as usize * stride0;
                let uv_off = 4 * ((t.bx >> ss_hor) as usize + (t.by >> ss_ver) as usize * stride);

                let furthest_r = ((cw4 << ss_hor) + t_dim.w as i32 - 1) & !(t_dim.w as i32 - 1);
                let furthest_b = ((ch4 << ss_ver) + t_dim.h as i32 - 1) & !(t_dim.h as i32 - 1);
                {
                    let planes = P::planes(f.cur_px.as_ref().expect("picture"));
                    cfl_ac::<P>(
                        layout,
                        &mut t.scratch.ac,
                        &planes[0],
                        y_src,
                        stride0,
                        cbw4 - (furthest_r >> ss_hor),
                        cbh4 - (furthest_b >> ss_ver),
                        cbw4 * 4,
                        cbh4 * 4,
                    );
                }
                for pl in 0..2 {
                    if b.cfl_alpha[pl] == 0 {
                        continue;
                    }
                    let mut angle = 0;
                    let fpx = P::frame_px(&f.px);
                    let tse = top_sb_edge(
                        fpx,
                        pl + 1,
                        sb128w,
                        sb_shift,
                        t.by,
                        (t.by & !ss_ver) & (sb_step - 1) == 0,
                    );
                    let xpos = t.bx >> ss_hor;
                    let ypos = t.by >> ss_ver;
                    let xstart = ts.tiling.col_start >> ss_hor;
                    let ystart = ts.tiling.row_start >> ss_ver;
                    let tpx = P::task_px(&mut t.px);
                    let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                    let m = prepare_intra_edges::<P>(
                        xpos,
                        xpos > xstart,
                        ypos,
                        ypos > ystart,
                        ts.tiling.col_end >> ss_hor,
                        ts.tiling.row_end >> ss_ver,
                        0,
                        &planes[1 + pl],
                        uv_off,
                        stride,
                        tse,
                        DC_PRED,
                        &mut angle,
                        uv_t_dim.w as i32,
                        uv_t_dim.h as i32,
                        false,
                        &mut tpx.edge,
                        edge,
                        bdmax,
                    );
                    cfl_pred::<P>(
                        m,
                        &mut planes[1 + pl],
                        uv_off,
                        stride,
                        &tpx.edge,
                        edge,
                        uv_t_dim.w as i32 * 4,
                        uv_t_dim.h as i32 * 4,
                        &t.scratch.ac,
                        b.cfl_alpha[pl] as i32,
                        bdmax,
                    );
                }
            } else if b.pal_sz[1] != 0 {
                let uv_dstoff =
                    4 * ((t.bx >> ss_hor) as usize + (t.by >> ss_ver) as usize * stride);
                let pal_idx = &t.scratch.pal_idx[(bw4 * bh4 * 16) as usize..];
                let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                pal_pred::<P>(
                    &mut planes[1],
                    uv_dstoff,
                    stride,
                    &t.scratch.pal[1],
                    pal_idx,
                    cbw4 * 4,
                    cbh4 * 4,
                );
                pal_pred::<P>(
                    &mut planes[2],
                    uv_dstoff,
                    stride,
                    &t.scratch.pal[2],
                    pal_idx,
                    cbw4 * 4,
                    cbh4 * 4,
                );
            }

            let sm_uv_fl = sm_uv_flag(&f.a[t.a], cbx4) | sm_uv_flag(&t.l, cby4);
            let uv_sb_has_tr = if ((init_x + 16) >> ss_hor) < cw4 {
                true
            } else if init_y != 0 {
                false
            } else {
                intra_edge_flags & (EDGE_I420_TOP_HAS_RIGHT >> (layout - 1)) != 0
            };
            let uv_sb_has_bl = if init_x != 0 {
                false
            } else if ((init_y + 16) >> ss_ver) < ch4 {
                true
            } else {
                intra_edge_flags & (EDGE_I420_LEFT_HAS_BOTTOM >> (layout - 1)) != 0
            };
            let sub_cw4 = imin(cw4, (init_x + 16) >> ss_hor);
            for pl in 0..2 {
                let mut y = init_y >> ss_ver;
                t.by += init_y;
                while y < sub_ch4 {
                    let mut dst = 4
                        * ((t.by >> ss_ver) as usize * stride
                            + ((t.bx + init_x) >> ss_hor) as usize);
                    let mut x = init_x >> ss_hor;
                    t.bx += init_x;
                    while x < sub_cw4 {
                        if !((b.uv_mode == CFL_PRED && b.cfl_alpha[pl] != 0) || b.pal_sz[1] != 0) {
                            let mut angle = b.uv_angle as i32;
                            // this probably looks weird because we're using
                            // luma flags in a chroma loop, but that's because
                            // prepare_intra_edges() expects luma flags as input
                            let edge_flags = (if (y > (init_y >> ss_ver) || !uv_sb_has_tr)
                                && (x + uv_t_dim.w as i32 >= sub_cw4)
                            {
                                0
                            } else {
                                EDGE_I444_TOP_HAS_RIGHT
                            }) | (if x > (init_x >> ss_hor)
                                || (!uv_sb_has_bl && y + uv_t_dim.h as i32 >= sub_ch4)
                            {
                                0
                            } else {
                                EDGE_I444_LEFT_HAS_BOTTOM
                            });
                            let fpx = P::frame_px(&f.px);
                            let tse = top_sb_edge(
                                fpx,
                                1 + pl,
                                sb128w,
                                sb_shift,
                                t.by,
                                (t.by & !ss_ver) & (sb_step - 1) == 0,
                            );
                            let uv_mode = if b.uv_mode == CFL_PRED {
                                DC_PRED
                            } else {
                                b.uv_mode
                            };
                            let xpos = t.bx >> ss_hor;
                            let ypos = t.by >> ss_ver;
                            let xstart = ts.tiling.col_start >> ss_hor;
                            let ystart = ts.tiling.row_start >> ss_ver;
                            let tpx = P::task_px(&mut t.px);
                            let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                            let m = prepare_intra_edges::<P>(
                                xpos,
                                xpos > xstart,
                                ypos,
                                ypos > ystart,
                                ts.tiling.col_end >> ss_hor,
                                ts.tiling.row_end >> ss_ver,
                                edge_flags,
                                &planes[1 + pl],
                                dst,
                                stride,
                                tse,
                                uv_mode,
                                &mut angle,
                                uv_t_dim.w as i32,
                                uv_t_dim.h as i32,
                                seq_hdr.intra_edge_filter != 0,
                                &mut tpx.edge,
                                edge,
                                bdmax,
                            );
                            angle |= intra_edge_filter_flag;
                            intra_pred::<P>(
                                m,
                                &mut planes[1 + pl],
                                dst,
                                stride,
                                &tpx.edge,
                                edge,
                                uv_t_dim.w as i32 * 4,
                                uv_t_dim.h as i32 * 4,
                                angle | sm_uv_fl,
                                (4 * f.bw + ss_hor - 4 * (t.bx & !ss_hor)) >> ss_hor,
                                (4 * f.bh + ss_ver - 4 * (t.by & !ss_ver)) >> ss_ver,
                                bdmax,
                            );
                        }

                        // skip_uv_pred:
                        if b.skip == 0 {
                            let mut txtp = 0u8;
                            let fr = coef_frame(f, ts, frame_hdr, b, b.uvtx, 1 + pl);
                            let (eob, cf_ctx) = decode_coefs::<P>(
                                ts,
                                &mut t.scratch.levels,
                                &f.a[t.a].ccoef[pl][cbx4 + x as usize..],
                                &t.l.ccoef[pl][cby4 + y as usize..],
                                b.uvtx,
                                bs,
                                b,
                                true,
                                1 + pl,
                                &mut t.cf,
                                &mut txtp,
                                &fr,
                            );
                            set_ctx(
                                &mut t.l.ccoef[pl],
                                cby4 + y as usize,
                                imin(uv_t_dim.h as i32, (f.bh - t.by + ss_ver) >> ss_ver) as usize,
                                cf_ctx,
                            );
                            set_ctx(
                                &mut f.a[t.a].ccoef[pl],
                                cbx4 + x as usize,
                                imin(uv_t_dim.w as i32, (f.bw - t.bx + ss_hor) >> ss_hor) as usize,
                                cf_ctx,
                            );
                            if eob >= 0 {
                                let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                                itxfm_add::<P>(
                                    b.uvtx,
                                    txtp,
                                    &mut planes[1 + pl],
                                    dst,
                                    stride,
                                    &mut t.cf,
                                    eob,
                                    bdmax,
                                );
                            }
                        } else {
                            set_ctx(
                                &mut t.l.ccoef[pl],
                                cby4 + y as usize,
                                uv_t_dim.h as usize,
                                0x40,
                            );
                            set_ctx(
                                &mut f.a[t.a].ccoef[pl],
                                cbx4 + x as usize,
                                uv_t_dim.w as usize,
                                0x40,
                            );
                        }
                        dst += uv_t_dim.w as usize * 4;
                        x += uv_t_dim.w as i32;
                        t.bx += (uv_t_dim.w as i32) << ss_hor;
                    }
                    t.bx -= x << ss_hor;
                    y += uv_t_dim.h as i32;
                    t.by += (uv_t_dim.h as i32) << ss_ver;
                }
                t.by -= y << ss_ver;
            }
            init_x += 16;
        }
        init_y += 16;
    }
}

/// `dav1d_recon_b_inter()`
pub(crate) fn recon_b_inter<P: Pixel>(
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    ts: &mut Dav1dTileState,
    bs: u8,
    b: &Av1Block,
) -> Result<(), ()> {
    let frame_hdr_arc = f.frame_hdr.clone().expect("frame header");
    let frame_hdr: &Dav1dFrameHeader = &frame_hdr_arc;
    let bx4 = (t.bx & 31) as usize;
    let by4 = (t.by & 31) as usize;
    let layout = f.cur.p.layout;
    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let cbx4 = bx4 >> ss_hor;
    let cby4 = by4 >> ss_ver;
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];
    let bw4 = b_dim[0] as i32;
    let bh4 = b_dim[1] as i32;
    let w4 = imin(bw4, f.bw - t.bx);
    let h4 = imin(bh4, f.bh - t.by);
    let has_chroma = layout != DAV1D_PIXEL_LAYOUT_I400
        && (bw4 > ss_hor || t.bx & 1 != 0)
        && (bh4 > ss_ver || t.by & 1 != 0);
    let chr_layout_idx = if layout == DAV1D_PIXEL_LAYOUT_I400 {
        0
    } else {
        (DAV1D_PIXEL_LAYOUT_I444 - layout) as usize
    };
    let bdmax = f.bitdepth_max;
    let stride0 = f.cur_px.as_ref().expect("picture").stride[0];
    let stride1 = f.cur_px.as_ref().expect("picture").stride[1];
    let fr = McFrame {
        cur_w: f.cur.p.w,
        cur_h: f.cur.p.h,
        bw: f.bw,
        bh: f.bh,
        layout,
        bitdepth_max: bdmax,
    };

    // prediction
    let cbh4 = (bh4 + ss_ver) >> ss_ver;
    let cbw4 = (bw4 + ss_hor) >> ss_hor;
    let mut dst = 4 * (t.by as usize * stride0 + t.bx as usize);
    let uvdstoff = 4 * ((t.bx >> ss_hor) as usize + (t.by >> ss_ver) as usize * stride1);
    if is_key_or_intra(frame_hdr) {
        // intrabc
        debug_assert!(frame_hdr.super_res.enabled == 0);
        let tpx = P::task_px(&mut t.px);
        let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
        mc::<P>(
            &mut tpx.emu_edge,
            McDst::Px(&mut planes[0], dst, stride0),
            &fr,
            &f.svc,
            bw4,
            bh4,
            t.bx,
            t.by,
            0,
            b.mv[0],
            None,
            0, /* unused */
            FILTER_2D_BILINEAR,
        )?;
        if has_chroma {
            for pl in 1..3 {
                mc::<P>(
                    &mut tpx.emu_edge,
                    McDst::Px(&mut planes[pl], uvdstoff, stride1),
                    &fr,
                    &f.svc,
                    bw4 << (bw4 == ss_hor) as i32,
                    bh4 << (bh4 == ss_ver) as i32,
                    t.bx & !ss_hor,
                    t.by & !ss_ver,
                    pl,
                    b.mv[0],
                    None,
                    0, /* unused */
                    FILTER_2D_BILINEAR,
                )?;
            }
        }
    } else if b.comp_type == COMP_INTER_NONE {
        let r0 = b.ref_[0] as usize;
        let filter_2d = b.filter2d;

        if imin(bw4, bh4) > 1
            && ((b.inter_mode == GLOBALMV && f.gmv_warp_allowed[r0] != 0)
                || (b.motion_mode == MM_WARP && t.warpmv.type_ > DAV1D_WM_TYPE_TRANSLATION))
        {
            let wmp = if b.motion_mode == MM_WARP {
                t.warpmv
            } else {
                frame_hdr.gmv[r0]
            };
            let tpx = P::task_px(&mut t.px);
            let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
            warp_affine::<P>(
                &mut tpx.emu_edge,
                WarpDst::Px(&mut planes[0], dst, stride0),
                &fr,
                t.bx,
                t.by,
                b_dim,
                0,
                &f.refp[r0],
                &wmp,
            )?;
        } else {
            {
                let tpx = P::task_px(&mut t.px);
                let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                mc::<P>(
                    &mut tpx.emu_edge,
                    McDst::Px(&mut planes[0], dst, stride0),
                    &fr,
                    &f.svc,
                    bw4,
                    bh4,
                    t.bx,
                    t.by,
                    0,
                    b.mv[0],
                    Some(&f.refp[r0]),
                    r0,
                    filter_2d,
                )?;
            }
            if b.motion_mode == MM_OBMC {
                obmc::<P>(f, t, ts, &fr, dst, stride0, b_dim, 0, bx4, by4, w4, h4)?;
            }
        }
        if b.interintra_type != 0 {
            let tl_edge = 32; // bitfn(t->scratch.edge) + 32
            let mut m = if b.interintra_mode == II_SMOOTH_PRED {
                SMOOTH_PRED
            } else {
                b.interintra_mode
            };
            let mut angle = 0;
            let fpx = P::frame_px(&f.px);
            let tse = top_sb_edge(
                fpx,
                0,
                f.sb128w,
                f.sb_shift,
                t.by,
                t.by & (f.sb_step - 1) == 0,
            );
            let tpx = P::task_px(&mut t.px);
            let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
            m = prepare_intra_edges::<P>(
                t.bx,
                t.bx > ts.tiling.col_start,
                t.by,
                t.by > ts.tiling.row_start,
                ts.tiling.col_end,
                ts.tiling.row_end,
                0,
                &planes[0],
                dst,
                stride0,
                tse,
                m,
                &mut angle,
                bw4,
                bh4,
                false,
                &mut tpx.edge,
                tl_edge,
                bdmax,
            );
            intra_pred::<P>(
                m,
                &mut tpx.interintra,
                0,
                4 * bw4 as usize,
                &tpx.edge,
                tl_edge,
                bw4 * 4,
                bh4 * 4,
                0,
                0,
                0,
                bdmax,
            );
            let ii_mask = if b.interintra_type == INTER_INTRA_BLEND {
                ii_masks(bs, 0, b.interintra_mode as usize)
            } else {
                wedge_masks(bs, 0, 0, b.wedge_idx as usize)
            };
            mc::blend::<P>(
                &mut planes[0],
                dst,
                stride0,
                &tpx.interintra,
                (bw4 * 4) as usize,
                (bh4 * 4) as usize,
                ii_mask,
            );
        }

        if has_chroma {
            // sub8x8 derivation
            let mut is_sub8x8 = bw4 == ss_hor || bh4 == ss_ver;
            if is_sub8x8 {
                debug_assert!(ss_hor == 1);
                if bw4 == 1 {
                    is_sub8x8 &= rblk(&f.rf, &t.rt, t.by, 0, t.bx - 1).ref_.ref_[0] > 0;
                }
                if bh4 == ss_ver {
                    is_sub8x8 &= rblk(&f.rf, &t.rt, t.by, -1, t.bx).ref_.ref_[0] > 0;
                }
                if bw4 == 1 && bh4 == ss_ver {
                    is_sub8x8 &= rblk(&f.rf, &t.rt, t.by, -1, t.bx - 1).ref_.ref_[0] > 0;
                }
            }

            // chroma prediction
            if is_sub8x8 {
                debug_assert!(ss_hor == 1);
                let mut h_off = 0;
                let mut v_off = 0;
                let tpx = P::task_px(&mut t.px);
                let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                if bw4 == 1 && bh4 == ss_ver {
                    let rr = *rblk(&f.rf, &t.rt, t.by, -1, t.bx - 1);
                    for pl in 0..2 {
                        mc::<P>(
                            &mut tpx.emu_edge,
                            McDst::Px(&mut planes[1 + pl], uvdstoff, stride1),
                            &fr,
                            &f.svc,
                            bw4,
                            bh4,
                            t.bx - 1,
                            t.by - 1,
                            1 + pl,
                            rr.mv.mv[0],
                            Some(&f.refp[rr.ref_.ref_[0] as usize - 1]),
                            rr.ref_.ref_[0] as usize - 1,
                            t.tl_4x4_filter,
                        )?;
                    }
                    v_off = 2 * stride1;
                    h_off = 2;
                }
                if bw4 == 1 {
                    let left_filter_2d =
                        FILTER_2D[t.l.filter[1][by4] as usize][t.l.filter[0][by4] as usize];
                    let rr = *rblk(&f.rf, &t.rt, t.by, 0, t.bx - 1);
                    for pl in 0..2 {
                        mc::<P>(
                            &mut tpx.emu_edge,
                            McDst::Px(&mut planes[1 + pl], uvdstoff + v_off, stride1),
                            &fr,
                            &f.svc,
                            bw4,
                            bh4,
                            t.bx - 1,
                            t.by,
                            1 + pl,
                            rr.mv.mv[0],
                            Some(&f.refp[rr.ref_.ref_[0] as usize - 1]),
                            rr.ref_.ref_[0] as usize - 1,
                            left_filter_2d,
                        )?;
                    }
                    h_off = 2;
                }
                if bh4 == ss_ver {
                    let a = &f.a[t.a];
                    let top_filter_2d =
                        FILTER_2D[a.filter[1][bx4] as usize][a.filter[0][bx4] as usize];
                    let rr = *rblk(&f.rf, &t.rt, t.by, -1, t.bx);
                    for pl in 0..2 {
                        mc::<P>(
                            &mut tpx.emu_edge,
                            McDst::Px(&mut planes[1 + pl], uvdstoff + h_off, stride1),
                            &fr,
                            &f.svc,
                            bw4,
                            bh4,
                            t.bx,
                            t.by - 1,
                            1 + pl,
                            rr.mv.mv[0],
                            Some(&f.refp[rr.ref_.ref_[0] as usize - 1]),
                            rr.ref_.ref_[0] as usize - 1,
                            top_filter_2d,
                        )?;
                    }
                    v_off = 2 * stride1;
                }
                for pl in 0..2 {
                    mc::<P>(
                        &mut tpx.emu_edge,
                        McDst::Px(&mut planes[1 + pl], uvdstoff + h_off + v_off, stride1),
                        &fr,
                        &f.svc,
                        bw4,
                        bh4,
                        t.bx,
                        t.by,
                        1 + pl,
                        b.mv[0],
                        Some(&f.refp[r0]),
                        r0,
                        filter_2d,
                    )?;
                }
            } else {
                if imin(cbw4, cbh4) > 1
                    && ((b.inter_mode == GLOBALMV && f.gmv_warp_allowed[r0] != 0)
                        || (b.motion_mode == MM_WARP && t.warpmv.type_ > DAV1D_WM_TYPE_TRANSLATION))
                {
                    let wmp = if b.motion_mode == MM_WARP {
                        t.warpmv
                    } else {
                        frame_hdr.gmv[r0]
                    };
                    let tpx = P::task_px(&mut t.px);
                    let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                    for pl in 0..2 {
                        warp_affine::<P>(
                            &mut tpx.emu_edge,
                            WarpDst::Px(&mut planes[1 + pl], uvdstoff, stride1),
                            &fr,
                            t.bx,
                            t.by,
                            b_dim,
                            1 + pl,
                            &f.refp[r0],
                            &wmp,
                        )?;
                    }
                } else {
                    for pl in 0..2 {
                        {
                            let tpx = P::task_px(&mut t.px);
                            let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                            mc::<P>(
                                &mut tpx.emu_edge,
                                McDst::Px(&mut planes[1 + pl], uvdstoff, stride1),
                                &fr,
                                &f.svc,
                                bw4 << (bw4 == ss_hor) as i32,
                                bh4 << (bh4 == ss_ver) as i32,
                                t.bx & !ss_hor,
                                t.by & !ss_ver,
                                1 + pl,
                                b.mv[0],
                                Some(&f.refp[r0]),
                                r0,
                                filter_2d,
                            )?;
                        }
                        if b.motion_mode == MM_OBMC {
                            obmc::<P>(
                                f,
                                t,
                                ts,
                                &fr,
                                uvdstoff,
                                stride1,
                                b_dim,
                                1 + pl,
                                bx4,
                                by4,
                                w4,
                                h4,
                            )?;
                        }
                    }
                }
                if b.interintra_type != 0 {
                    // FIXME for 8x32 with 4:2:2 subsampling, this probably does
                    // the wrong thing since it will select 4x16, not 4x32, as a
                    // transform size...
                    let ii_mask = if b.interintra_type == INTER_INTRA_BLEND {
                        ii_masks(bs, chr_layout_idx, b.interintra_mode as usize)
                    } else {
                        wedge_masks(bs, chr_layout_idx, 0, b.wedge_idx as usize)
                    };

                    for pl in 0..2 {
                        let tl_edge = 32; // bitfn(t->scratch.edge) + 32
                        let mut m = if b.interintra_mode == II_SMOOTH_PRED {
                            SMOOTH_PRED
                        } else {
                            b.interintra_mode
                        };
                        let mut angle = 0;
                        let uvdst = uvdstoff;
                        let fpx = P::frame_px(&f.px);
                        let tse = top_sb_edge(
                            fpx,
                            pl + 1,
                            f.sb128w,
                            f.sb_shift,
                            t.by,
                            t.by & (f.sb_step - 1) == 0,
                        );
                        let tpx = P::task_px(&mut t.px);
                        let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                        m = prepare_intra_edges::<P>(
                            t.bx >> ss_hor,
                            (t.bx >> ss_hor) > (ts.tiling.col_start >> ss_hor),
                            t.by >> ss_ver,
                            (t.by >> ss_ver) > (ts.tiling.row_start >> ss_ver),
                            ts.tiling.col_end >> ss_hor,
                            ts.tiling.row_end >> ss_ver,
                            0,
                            &planes[1 + pl],
                            uvdst,
                            stride1,
                            tse,
                            m,
                            &mut angle,
                            cbw4,
                            cbh4,
                            false,
                            &mut tpx.edge,
                            tl_edge,
                            bdmax,
                        );
                        intra_pred::<P>(
                            m,
                            &mut tpx.interintra,
                            0,
                            cbw4 as usize * 4,
                            &tpx.edge,
                            tl_edge,
                            cbw4 * 4,
                            cbh4 * 4,
                            0,
                            0,
                            0,
                            bdmax,
                        );
                        mc::blend::<P>(
                            &mut planes[1 + pl],
                            uvdst,
                            stride1,
                            &tpx.interintra,
                            (cbw4 * 4) as usize,
                            (cbh4 * 4) as usize,
                            ii_mask,
                        );
                    }
                }
            }
        }

        // skip_inter_chroma_pred:
        t.tl_4x4_filter = filter_2d;
    } else {
        let filter_2d = b.filter2d;
        // Maximum super block size is 128x128
        let mut jnt_weight = 0;
        let mut mask: &[u8] = &[];

        for i in 0..2 {
            let ri = b.ref_[i] as usize;
            let tpx = P::task_px(&mut t.px);
            if b.inter_mode == GLOBALMV_GLOBALMV && f.gmv_warp_allowed[ri] != 0 {
                warp_affine::<P>(
                    &mut tpx.emu_edge,
                    WarpDst::Tmp(&mut t.scratch.compinter[i], bw4 as usize * 4),
                    &fr,
                    t.bx,
                    t.by,
                    b_dim,
                    0,
                    &f.refp[ri],
                    &frame_hdr.gmv[ri],
                )?;
            } else {
                mc::<P>(
                    &mut tpx.emu_edge,
                    McDst::Tmp(&mut t.scratch.compinter[i]),
                    &fr,
                    &f.svc,
                    bw4,
                    bh4,
                    t.bx,
                    t.by,
                    0,
                    b.mv[i],
                    Some(&f.refp[ri]),
                    ri,
                    filter_2d,
                )?;
            }
        }
        let tmp = &t.scratch.compinter;
        let ms = b.mask_sign as usize;
        {
            let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
            let (w, h) = ((bw4 * 4) as usize, (bh4 * 4) as usize);
            match b.comp_type {
                COMP_INTER_AVG => {
                    mc::avg::<P>(&mut planes[0], dst, stride0, &tmp[0], &tmp[1], w, h, bdmax);
                }
                COMP_INTER_WEIGHTED_AVG => {
                    jnt_weight = f.jnt_weights[b.ref_[0] as usize][b.ref_[1] as usize] as i32;
                    mc::w_avg::<P>(
                        &mut planes[0],
                        dst,
                        stride0,
                        &tmp[0],
                        &tmp[1],
                        w,
                        h,
                        jnt_weight,
                        bdmax,
                    );
                }
                COMP_INTER_SEG => {
                    mc::w_mask::<P>(
                        &mut planes[0],
                        dst,
                        stride0,
                        &tmp[ms],
                        &tmp[1 - ms],
                        w,
                        h,
                        &mut t.scratch.seg_mask,
                        b.mask_sign as i32,
                        chr_layout_idx >= 1,
                        chr_layout_idx >= 2,
                        bdmax,
                    );
                    mask = &t.scratch.seg_mask;
                }
                COMP_INTER_WEDGE => {
                    mask = wedge_masks(bs, 0, 0, b.wedge_idx as usize);
                    mc::mask::<P>(
                        &mut planes[0],
                        dst,
                        stride0,
                        &tmp[ms],
                        &tmp[1 - ms],
                        w,
                        h,
                        mask,
                        bdmax,
                    );
                    if has_chroma {
                        mask = wedge_masks(bs, chr_layout_idx, ms, b.wedge_idx as usize);
                    }
                }
                _ => {}
            }
        }
        // chroma
        if has_chroma {
            for pl in 0..2 {
                for i in 0..2 {
                    let ri = b.ref_[i] as usize;
                    let tpx = P::task_px(&mut t.px);
                    if b.inter_mode == GLOBALMV_GLOBALMV
                        && imin(cbw4, cbh4) > 1
                        && f.gmv_warp_allowed[ri] != 0
                    {
                        warp_affine::<P>(
                            &mut tpx.emu_edge,
                            WarpDst::Tmp(&mut t.scratch.compinter[i], (bw4 * 4 >> ss_hor) as usize),
                            &fr,
                            t.bx,
                            t.by,
                            b_dim,
                            1 + pl,
                            &f.refp[ri],
                            &frame_hdr.gmv[ri],
                        )?;
                    } else {
                        mc::<P>(
                            &mut tpx.emu_edge,
                            McDst::Tmp(&mut t.scratch.compinter[i]),
                            &fr,
                            &f.svc,
                            bw4,
                            bh4,
                            t.bx,
                            t.by,
                            1 + pl,
                            b.mv[i],
                            Some(&f.refp[ri]),
                            ri,
                            filter_2d,
                        )?;
                    }
                }
                let tmp = &t.scratch.compinter;
                let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                let uvdst = uvdstoff;
                let (w, h) = ((bw4 * 4 >> ss_hor) as usize, (bh4 * 4 >> ss_ver) as usize);
                match b.comp_type {
                    COMP_INTER_AVG => {
                        mc::avg::<P>(
                            &mut planes[1 + pl],
                            uvdst,
                            stride1,
                            &tmp[0],
                            &tmp[1],
                            w,
                            h,
                            bdmax,
                        );
                    }
                    COMP_INTER_WEIGHTED_AVG => {
                        mc::w_avg::<P>(
                            &mut planes[1 + pl],
                            uvdst,
                            stride1,
                            &tmp[0],
                            &tmp[1],
                            w,
                            h,
                            jnt_weight,
                            bdmax,
                        );
                    }
                    COMP_INTER_WEDGE | COMP_INTER_SEG => {
                        mc::mask::<P>(
                            &mut planes[1 + pl],
                            uvdst,
                            stride1,
                            &tmp[ms],
                            &tmp[1 - ms],
                            w,
                            h,
                            mask,
                            bdmax,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    let cw4 = (w4 + ss_hor) >> ss_hor;
    let ch4 = (h4 + ss_ver) >> ss_ver;

    if b.skip != 0 {
        // reset coef contexts
        set_ctx(&mut t.l.lcoef, by4, bh4 as usize, 0x40);
        set_ctx(&mut f.a[t.a].lcoef, bx4, bw4 as usize, 0x40);
        if has_chroma {
            set_ctx(&mut t.l.ccoef[0], cby4, cbh4 as usize, 0x40);
            set_ctx(&mut t.l.ccoef[1], cby4, cbh4 as usize, 0x40);
            set_ctx(&mut f.a[t.a].ccoef[0], cbx4, cbw4 as usize, 0x40);
            set_ctx(&mut f.a[t.a].ccoef[1], cbx4, cbw4 as usize, 0x40);
        }
        return Ok(());
    }

    let uvtx = &TXFM_DIMENSIONS[b.uvtx as usize];
    let ytx = &TXFM_DIMENSIONS[b.max_ytx as usize];
    let tx_split = [b.tx_split0 as u16, b.tx_split1];

    let mut init_y = 0;
    while init_y < bh4 {
        let mut init_x = 0;
        while init_x < bw4 {
            // coefficient coding & inverse transforms
            let mut y_off = (init_y != 0) as i32;
            let mut y;
            dst += stride0 * 4 * init_y as usize;
            y = init_y;
            t.by += init_y;
            while y < imin(h4, init_y + 16) {
                let mut x;
                let mut x_off = (init_x != 0) as i32;
                x = init_x;
                t.bx += init_x;
                while x < imin(w4, init_x + 16) {
                    read_coef_tree::<P>(
                        f,
                        t,
                        ts,
                        frame_hdr,
                        bs,
                        b,
                        b.max_ytx,
                        0,
                        &tx_split,
                        x_off,
                        y_off,
                        dst + x as usize * 4,
                    );
                    t.bx += ytx.w as i32;
                    x += ytx.w as i32;
                    x_off += 1;
                }
                dst += stride0 * 4 * ytx.h as usize;
                t.bx -= x;
                t.by += ytx.h as i32;
                y += ytx.h as i32;
                y_off += 1;
            }
            dst -= stride0 * 4 * y as usize;
            t.by -= y;

            // chroma coefs and inverse transform
            if has_chroma {
                for pl in 0..2 {
                    let mut uvdst = uvdstoff + (stride1 * init_y as usize * 4 >> ss_ver);
                    y = init_y >> ss_ver;
                    t.by += init_y;
                    while y < imin(ch4, (init_y + 16) >> ss_ver) {
                        let mut x = init_x >> ss_hor;
                        t.bx += init_x;
                        while x < imin(cw4, (init_x + 16) >> ss_hor) {
                            let mut txtp = t.txtp_map[(by4 + (y << ss_ver) as usize) * 32
                                + bx4
                                + (x << ss_hor) as usize];
                            let fr = coef_frame(f, ts, frame_hdr, b, b.uvtx, 1 + pl);
                            let (eob, cf_ctx) = decode_coefs::<P>(
                                ts,
                                &mut t.scratch.levels,
                                &f.a[t.a].ccoef[pl][cbx4 + x as usize..],
                                &t.l.ccoef[pl][cby4 + y as usize..],
                                b.uvtx,
                                bs,
                                b,
                                false,
                                1 + pl,
                                &mut t.cf,
                                &mut txtp,
                                &fr,
                            );
                            set_ctx(
                                &mut t.l.ccoef[pl],
                                cby4 + y as usize,
                                imin(uvtx.h as i32, (f.bh - t.by + ss_ver) >> ss_ver) as usize,
                                cf_ctx,
                            );
                            set_ctx(
                                &mut f.a[t.a].ccoef[pl],
                                cbx4 + x as usize,
                                imin(uvtx.w as i32, (f.bw - t.bx + ss_hor) >> ss_hor) as usize,
                                cf_ctx,
                            );
                            if eob >= 0 {
                                let planes = P::planes_mut(f.cur_px.as_mut().expect("picture"));
                                itxfm_add::<P>(
                                    b.uvtx,
                                    txtp,
                                    &mut planes[1 + pl],
                                    uvdst + 4 * x as usize,
                                    stride1,
                                    &mut t.cf,
                                    eob,
                                    bdmax,
                                );
                            }
                            t.bx += (uvtx.w as i32) << ss_hor;
                            x += uvtx.w as i32;
                        }
                        uvdst += stride1 * 4 * uvtx.h as usize;
                        t.bx -= x << ss_hor;
                        t.by += (uvtx.h as i32) << ss_ver;
                        y += uvtx.h as i32;
                    }
                    t.by -= y << ss_ver;
                }
            }
            init_x += 16;
        }
        init_y += 16;
    }
    Ok(())
}

/// The picture rows of superblock row `sby` (`f->lf.p[]` + y * stride):
/// the offsets in the luma and chroma planes.
#[inline]
fn sbrow_offsets(sby: i32, sb_step: i32, stride: [usize; 2], ss_ver: i32) -> [usize; 3] {
    let y = (sby * sb_step * 4) as usize;
    [
        y * stride[0],
        (y * stride[1]) >> ss_ver,
        (y * stride[1]) >> ss_ver,
    ]
}

/// `dav1d_filter_sbrow_deblock_cols()`
fn filter_sbrow_deblock_cols<P: Pixel>(inloop_filters: u32, f: &mut Dav1dFrameContext, sby: i32) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    if inloop_filters & DAV1D_INLOOPFILTER_DEBLOCK == 0
        || (frame_hdr.loopfilter.level_y[0] == 0 && frame_hdr.loopfilter.level_y[1] == 0)
    {
        return;
    }
    let ss_ver = (f.cur.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let stride = f.cur_px.as_ref().expect("picture").stride;
    let p = sbrow_offsets(sby, f.sb_step, stride, ss_ver);
    let mask = ((sby >> (f.seq_hdr().sb128 == 0) as i32) * f.sb128w) as usize;
    let start_of_tile_row = f.lf.start_of_tile_row[sby as usize] as i32;
    loopfilter_sbrow_cols::<P>(f, p, mask, sby, start_of_tile_row);
}

/// `dav1d_filter_sbrow_deblock_rows()`
fn filter_sbrow_deblock_rows<P: Pixel>(inloop_filters: u32, f: &mut Dav1dFrameContext, sby: i32) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    let ss_ver = (f.cur.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let stride = f.cur_px.as_ref().expect("picture").stride;
    let p = sbrow_offsets(sby, f.sb_step, stride, ss_ver);
    let mask = ((sby >> (f.seq_hdr().sb128 == 0) as i32) * f.sb128w) as usize;
    if inloop_filters & DAV1D_INLOOPFILTER_DEBLOCK != 0
        && (frame_hdr.loopfilter.level_y[0] != 0 || frame_hdr.loopfilter.level_y[1] != 0)
    {
        loopfilter_sbrow_rows::<P>(f, p, mask, sby);
    }
    if f.seq_hdr().cdef != 0 || f.lf.restore_planes != 0 {
        // Store loop filtered pixels required by CDEF / LR
        copy_lpf::<P>(f, p, sby);
    }
}

/// `dav1d_filter_sbrow_cdef()`
fn filter_sbrow_cdef<P: Pixel>(
    inloop_filters: u32,
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    sby: i32,
) {
    if inloop_filters & DAV1D_INLOOPFILTER_CDEF == 0 {
        return;
    }
    let sbsz = f.sb_step;
    let ss_ver = (f.cur.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let stride = f.cur_px.as_ref().expect("picture").stride;
    let p = sbrow_offsets(sby, sbsz, stride, ss_ver);
    let not_sb128 = (f.seq_hdr().sb128 == 0) as i32;
    let prev_mask = (((sby - 1) >> not_sb128) * f.sb128w) as usize;
    let mask = ((sby >> not_sb128) * f.sb128w) as usize;
    let start = sby * sbsz;
    if sby != 0 {
        let p_up = [
            p[0] - 8 * stride[0],
            p[1] - (8 * stride[1] >> ss_ver),
            p[2] - (8 * stride[1] >> ss_ver),
        ];
        cdef_brow::<P>(f, t, p_up, prev_mask, start - 2, start, true, sby);
    }
    let n_blks = sbsz - 2 * (sby + 1 < f.sbh) as i32;
    let end = imin(start + n_blks, f.bh);
    cdef_brow::<P>(f, t, p, mask, start, end, false, sby);
}

/// `dav1d_filter_sbrow_resize()`
fn filter_sbrow_resize<P: Pixel>(f: &mut Dav1dFrameContext, sby: i32) {
    let sbsz = f.sb_step;
    let layout = f.cur.p.layout;
    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let src_px = f.cur_px.as_ref().expect("picture");
    let dst_px = f.sr_px.as_mut().expect("upscaled picture");
    let p = sbrow_offsets(sby, sbsz, src_px.stride, ss_ver);
    let sr_p = sbrow_offsets(sby, sbsz, dst_px.stride, ss_ver);
    let has_chroma = layout != DAV1D_PIXEL_LAYOUT_I400;
    let src_planes = P::planes(src_px);
    let src_stride_all = src_px.stride;
    let dst_stride_all = dst_px.stride;
    let dst_planes = P::planes_mut(dst_px);
    for pl in 0..1 + 2 * has_chroma as usize {
        let ss_ver = (pl != 0 && layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
        let h_start = (8 * (sby != 0) as i32 >> ss_ver) as usize;
        let dst_stride = dst_stride_all[(pl != 0) as usize];
        let dst = sr_p[pl] - h_start * dst_stride;
        let src_stride = src_stride_all[(pl != 0) as usize];
        let src = p[pl] - h_start * src_stride;
        let h_end = 4 * (sbsz - 2 * (sby + 1 < f.sbh) as i32) >> ss_ver;
        let ss_hor = (pl != 0 && layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
        let dst_w = (f.sr_cur.p.p.w + ss_hor) >> ss_hor;
        let src_w = (4 * f.bw + ss_hor) >> ss_hor;
        let img_h = (f.cur.p.h - sbsz * 4 * sby + ss_ver) >> ss_ver;

        mc::resize::<P>(
            &mut dst_planes[pl],
            dst,
            dst_stride,
            &src_planes[pl],
            src,
            src_stride,
            dst_w as usize,
            (imin(img_h, h_end) + h_start as i32) as usize,
            src_w,
            f.resize_step[(pl != 0) as usize],
            f.resize_start[(pl != 0) as usize],
            f.bitdepth_max,
        );
    }
}

/// `dav1d_filter_sbrow_lr()`
fn filter_sbrow_lr<P: Pixel>(inloop_filters: u32, f: &mut Dav1dFrameContext, sby: i32) {
    if inloop_filters & DAV1D_INLOOPFILTER_RESTORATION == 0 {
        return;
    }
    let ss_ver = (f.cur.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let stride = f
        .sr_px
        .as_ref()
        .or(f.cur_px.as_ref())
        .expect("picture")
        .stride;
    let sr_p = sbrow_offsets(sby, f.sb_step, stride, ss_ver);
    lr_sbrow::<P>(f, sr_p, sby);
}

/// `dav1d_filter_sbrow()`
pub(crate) fn filter_sbrow<P: Pixel>(
    inloop_filters: u32,
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    sby: i32,
) {
    filter_sbrow_deblock_cols::<P>(inloop_filters, f, sby);
    filter_sbrow_deblock_rows::<P>(inloop_filters, f, sby);
    if f.seq_hdr().cdef != 0 {
        filter_sbrow_cdef::<P>(inloop_filters, f, t, sby);
    }
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    if frame_hdr.width[0] != frame_hdr.width[1] {
        filter_sbrow_resize::<P>(f, sby);
    }
    if f.lf.restore_planes != 0 {
        filter_sbrow_lr::<P>(inloop_filters, f, sby);
    }
}

/// `dav1d_backup_ipred_edge()`
pub(crate) fn backup_ipred_edge<P: Pixel>(
    f: &mut Dav1dFrameContext,
    t: &Dav1dTaskContext,
    ts: &Dav1dTileState,
) {
    let sby = t.by >> f.sb_shift;
    let sby_off = (f.sb128w * 128 * sby) as usize;
    let x_off = ts.tiling.col_start as usize;
    let cur = f.cur_px.as_ref().expect("picture");
    let stride = cur.stride;
    let planes = P::planes(cur);
    let fpx = P::frame_px_mut(&mut f.px);

    let y = x_off * 4 + ((t.by + f.sb_step) * 4 - 1) as usize * stride[0];
    let n = 4 * (ts.tiling.col_end as usize - x_off);
    fpx.ipred_edge[0][sby_off + x_off * 4..sby_off + x_off * 4 + n]
        .copy_from_slice(&planes[0][y..y + n]);

    if f.cur.p.layout != DAV1D_PIXEL_LAYOUT_I400 {
        let ss_ver = (f.cur.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
        let ss_hor = (f.cur.p.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;

        let uv_off =
            (x_off * 4 >> ss_hor) + ((((t.by + f.sb_step) * 4) >> ss_ver) - 1) as usize * stride[1];
        let n = 4 * (ts.tiling.col_end as usize - x_off) >> ss_hor;
        for pl in 1..=2 {
            let o = sby_off + (x_off * 4 >> ss_hor);
            fpx.ipred_edge[pl][o..o + n].copy_from_slice(&planes[pl][uv_off..uv_off + n]);
        }
    }
}
