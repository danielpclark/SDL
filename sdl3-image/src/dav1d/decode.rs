// Rust translation of src/decode.c and src/decode.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018-2021, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Block, partition and tile decoding, and the per-frame setup.
//!
//! Single-threaded (`n_fc == 1`, `n_tc == 1`): the frame threading passes
//! (`t->frame_thread.pass` 1 and 2, `read_coef_blocks`, the
//! `*_lowest_px()` reference progress tracking) and the task queue are
//! gone, and `dav1d_submit_frame()` decodes the frame at once. The
//! decoded frame's references are updated after the frame decoded (the
//! C code refreshes them before decoding and clears them again on an
//! error, which ends in the same state).

use std::sync::Arc;

use super::bitdepth::Pixel;
use super::cdf::{
    cdf_thread_copy, cdf_thread_init_static, cdf_thread_update, CdfMvComponent, CdfThreadContext,
};
use super::dequant_tables::DQ_TBL;
use super::env::*;
use super::headers::*;
use super::internal::*;
use super::intops::*;
use super::intra_edge::*;
use super::levels::*;
use super::lf_mask::{calc_eih, calc_lf_values, create_lf_mask_inter, create_lf_mask_intra};
use super::lf_mask::{Av1Filter, Av1Restoration, Av1RestorationUnit};
use super::mem::try_vec;
use super::msac::MsacContext;
use super::picture::*;
use super::qm::qm_tbl;
use super::recon::{backup_ipred_edge, filter_sbrow, recon_b_inter, recon_b_intra};
use super::refmvs::*;
use super::tables::*;
use super::warpmv::{find_affine_int, get_shear_params};

/// `DAV1D_ERR(e)`
pub(crate) const ENOMEM: i32 = 12;
pub(crate) const EINVAL: i32 = 22;
pub(crate) const ERANGE: i32 = 34;
pub(crate) const ENOPROTOOPT: i32 = 92;
pub(crate) const EAGAIN: i32 = 11;

#[inline]
pub(crate) const fn dav1d_err(e: i32) -> i32 {
    -e
}

fn init_quant_tables(
    seq_hdr: &Dav1dSequenceHeader,
    frame_hdr: &Dav1dFrameHeader,
    qidx: i32,
    dq: &mut [[[u16; 2]; 3]; 8],
) {
    let hbd = seq_hdr.hbd as usize;
    for i in 0..(if frame_hdr.segmentation.enabled != 0 {
        8
    } else {
        1
    }) {
        let yac = if frame_hdr.segmentation.enabled != 0 {
            iclip_u8(qidx + frame_hdr.segmentation.seg_data.d[i].delta_q)
        } else {
            qidx
        };
        let ydc = iclip_u8(yac + frame_hdr.quant.ydc_delta) as usize;
        let uac = iclip_u8(yac + frame_hdr.quant.uac_delta) as usize;
        let udc = iclip_u8(yac + frame_hdr.quant.udc_delta) as usize;
        let vac = iclip_u8(yac + frame_hdr.quant.vac_delta) as usize;
        let vdc = iclip_u8(yac + frame_hdr.quant.vdc_delta) as usize;
        let yac = yac as usize;

        dq[i][0][0] = DQ_TBL[hbd][ydc][0];
        dq[i][0][1] = DQ_TBL[hbd][yac][1];
        dq[i][1][0] = DQ_TBL[hbd][udc][0];
        dq[i][1][1] = DQ_TBL[hbd][uac][1];
        dq[i][2][0] = DQ_TBL[hbd][vdc][0];
        dq[i][2][1] = DQ_TBL[hbd][vac][1];
    }
}

fn read_mv_component_diff(
    msac: &mut MsacContext,
    mv_comp: &mut CdfMvComponent,
    have_fp: bool,
    have_hp: bool,
) -> i32 {
    let sign = msac.decode_bool_adapt(&mut mv_comp.sign);
    let cl = msac.decode_symbol_adapt16(&mut mv_comp.classes, 10) as i32;
    let up;
    let fp;
    let hp;

    if cl == 0 {
        up = msac.decode_bool_adapt(&mut mv_comp.class0) as i32;
        if have_fp {
            fp = msac.decode_symbol_adapt4(&mut mv_comp.class0_fp[up as usize], 3) as i32;
            hp = if have_hp {
                msac.decode_bool_adapt(&mut mv_comp.class0_hp) as i32
            } else {
                1
            };
        } else {
            fp = 3;
            hp = 1;
        }
    } else {
        let mut u = 1 << cl;
        for n in 0..cl as usize {
            u |= (msac.decode_bool_adapt(&mut mv_comp.class_n[n]) as i32) << n;
        }
        up = u;
        if have_fp {
            fp = msac.decode_symbol_adapt4(&mut mv_comp.class_n_fp, 3) as i32;
            hp = if have_hp {
                msac.decode_bool_adapt(&mut mv_comp.class_n_hp) as i32
            } else {
                1
            };
        } else {
            fp = 3;
            hp = 1;
        }
    }

    let diff = ((up << 3) | (fp << 1) | hp) + 1;

    if sign != 0 {
        -diff
    } else {
        diff
    }
}

/// `read_mv_residual()`: `mv_cdf` is the component CDFs of `ts->cdf.mv`
/// or `ts->cdf.dmv`; the joint CDF is always `ts->cdf.mv.joint`.
fn read_mv_residual(
    ts: &mut Dav1dTileState,
    ref_mv: &mut Mv,
    dmv: bool,
    have_fp: bool,
    have_hp: bool,
) {
    let joint = ts
        .msac
        .decode_symbol_adapt4(&mut ts.cdf.mv.joint, N_MV_JOINTS - 1);
    let comps = if dmv {
        &mut ts.cdf.dmv.comp
    } else {
        &mut ts.cdf.mv.comp
    };
    // (the C code adds to the int16_t members; the sum wraps like it does)
    match joint {
        MV_JOINT_HV => {
            ref_mv.y = (ref_mv.y as i32
                + read_mv_component_diff(&mut ts.msac, &mut comps[0], have_fp, have_hp))
                as i16;
            ref_mv.x = (ref_mv.x as i32
                + read_mv_component_diff(&mut ts.msac, &mut comps[1], have_fp, have_hp))
                as i16;
        }
        MV_JOINT_H => {
            ref_mv.x = (ref_mv.x as i32
                + read_mv_component_diff(&mut ts.msac, &mut comps[1], have_fp, have_hp))
                as i16;
        }
        MV_JOINT_V => {
            ref_mv.y = (ref_mv.y as i32
                + read_mv_component_diff(&mut ts.msac, &mut comps[0], have_fp, have_hp))
                as i16;
        }
        _ => {}
    }
}

fn read_tx_tree(
    t: &mut Dav1dTaskContext,
    a: &mut BlockContext,
    ts: &mut Dav1dTileState,
    f_bw: i32,
    f_bh: i32,
    from: u8,
    depth: usize,
    masks: &mut [u16; 2],
    x_off: i32,
    y_off: i32,
) {
    let bx4 = (t.bx & 31) as usize;
    let by4 = (t.by & 31) as usize;
    let t_dim = &TXFM_DIMENSIONS[from as usize];
    let txw = t_dim.lw;
    let txh = t_dim.lh;
    let is_split;

    if depth < 2 && from > TX_4X4 {
        let cat = 2 * (TX_64X64 as usize - t_dim.max as usize) - depth;
        let a_ = (a.tx[bx4] < txw as i8) as usize;
        let l_ = (t.l.tx[by4] < txh as i8) as usize;

        is_split = ts
            .msac
            .decode_bool_adapt(&mut ts.cdf.m.txpart[cat][a_ + l_])
            != 0;
        if is_split {
            masks[depth] |= 1 << (y_off * 4 + x_off);
        }
    } else {
        is_split = false;
    }

    if is_split && t_dim.max > TX_8X8 {
        let sub = t_dim.sub;
        let sub_t_dim = &TXFM_DIMENSIONS[sub as usize];
        let txsw = sub_t_dim.w as i32;
        let txsh = sub_t_dim.h as i32;

        read_tx_tree(
            t,
            a,
            ts,
            f_bw,
            f_bh,
            sub,
            depth + 1,
            masks,
            x_off * 2 + 0,
            y_off * 2 + 0,
        );
        t.bx += txsw;
        if txw >= txh && t.bx < f_bw {
            read_tx_tree(
                t,
                a,
                ts,
                f_bw,
                f_bh,
                sub,
                depth + 1,
                masks,
                x_off * 2 + 1,
                y_off * 2 + 0,
            );
        }
        t.bx -= txsw;
        t.by += txsh;
        if txh >= txw && t.by < f_bh {
            read_tx_tree(
                t,
                a,
                ts,
                f_bw,
                f_bh,
                sub,
                depth + 1,
                masks,
                x_off * 2 + 0,
                y_off * 2 + 1,
            );
            t.bx += txsw;
            if txw >= txh && t.bx < f_bw {
                read_tx_tree(
                    t,
                    a,
                    ts,
                    f_bw,
                    f_bh,
                    sub,
                    depth + 1,
                    masks,
                    x_off * 2 + 1,
                    y_off * 2 + 1,
                );
            }
            t.bx -= txsw;
        }
        t.by -= txsh;
    } else {
        set_ctx(
            &mut t.l.tx,
            by4,
            t_dim.h as usize,
            if is_split { TX_4X4 as i8 } else { txh as i8 },
        );
        set_ctx(
            &mut a.tx,
            bx4,
            t_dim.w as usize,
            if is_split { TX_4X4 as i8 } else { txw as i8 },
        );
    }
}

fn neg_deinterleave(diff: i32, r#ref: i32, max: i32) -> i32 {
    if r#ref == 0 {
        return diff;
    }
    if r#ref >= max - 1 {
        return max - diff - 1;
    }
    if 2 * r#ref < max {
        if diff <= 2 * r#ref {
            if diff & 1 != 0 {
                return r#ref + ((diff + 1) >> 1);
            } else {
                return r#ref - (diff >> 1);
            }
        }
        diff
    } else {
        if diff <= 2 * (max - r#ref - 1) {
            if diff & 1 != 0 {
                return r#ref + ((diff + 1) >> 1);
            } else {
                return r#ref - (diff >> 1);
            }
        }
        max - (diff + 1)
    }
}

/// `r[k][x]`: the refmvs block at row `k` (relative to the block's row,
/// `&t->rt.r[(t->by & 31) + 5]`) and column `x`.
#[inline]
fn rblk<'a>(rf: &'a RefmvsFrame, rt: &RefmvsTile, by: i32, k: i32, x: i32) -> &'a RefmvsBlock {
    let row = rt.r[((by & 31) + 5 + k) as usize];
    &rf.r[row + x as usize]
}

/// `bs(rp)`: the block dimensions of a refmvs block.
#[inline]
fn rbs(rp: &RefmvsBlock) -> &'static [u8; 4] {
    &BLOCK_DIMENSIONS[rp.bs as usize]
}

fn find_matching_ref(
    t: &Dav1dTaskContext,
    rf: &RefmvsFrame,
    ts_col_end: i32,
    intra_edge_flags: u8,
    bw4: i32,
    bh4: i32,
    w4: i32,
    h4: i32,
    have_left: bool,
    have_top: bool,
    r#ref: i8,
    masks: &mut [u64; 2],
) {
    let mut count = 0;
    let mut have_topleft = have_top && have_left;
    let mut have_topright = imax(bw4, bh4) < 32
        && have_top
        && t.bx + bw4 < ts_col_end
        && (intra_edge_flags & EDGE_I444_TOP_HAS_RIGHT) != 0;

    let matches = |rp: &RefmvsBlock| rp.ref_.ref_[0] == r#ref + 1 && rp.ref_.ref_[1] == -1;

    if have_top {
        let mut x2 = t.bx;
        let r2 = rblk(rf, &t.rt, t.by, -1, x2);
        if matches(r2) {
            masks[0] |= 1;
            count = 1;
        }
        let mut aw4 = rbs(r2)[0] as i32;
        if aw4 >= bw4 {
            let off = t.bx & (aw4 - 1);
            if off != 0 {
                have_topleft = false;
            }
            if aw4 - off > bw4 {
                have_topright = false;
            }
        } else {
            let mut mask: u32 = 1 << aw4;
            let mut x = aw4;
            while x < w4 {
                x2 += aw4;
                let r2 = rblk(rf, &t.rt, t.by, -1, x2);
                if matches(r2) {
                    masks[0] |= mask as u64;
                    count += 1;
                    if count >= 8 {
                        return;
                    }
                }
                aw4 = rbs(r2)[0] as i32;
                mask <<= aw4;
                x += aw4;
            }
        }
    }
    if have_left {
        let mut y2 = 0;
        let r2 = rblk(rf, &t.rt, t.by, y2, t.bx - 1);
        if matches(r2) {
            masks[1] |= 1;
            count += 1;
            if count >= 8 {
                return;
            }
        }
        let mut lh4 = rbs(r2)[1] as i32;
        if lh4 >= bh4 {
            if t.by & (lh4 - 1) != 0 {
                have_topleft = false;
            }
        } else {
            let mut mask: u32 = 1 << lh4;
            let mut y = lh4;
            while y < h4 {
                y2 += lh4;
                let r2 = rblk(rf, &t.rt, t.by, y2, t.bx - 1);
                if matches(r2) {
                    masks[1] |= mask as u64;
                    count += 1;
                    if count >= 8 {
                        return;
                    }
                }
                lh4 = rbs(r2)[1] as i32;
                mask <<= lh4;
                y += lh4;
            }
        }
    }
    if have_topleft && matches(rblk(rf, &t.rt, t.by, -1, t.bx - 1)) {
        masks[1] |= 1u64 << 32;
        count += 1;
        if count >= 8 {
            return;
        }
    }
    if have_topright && matches(rblk(rf, &t.rt, t.by, -1, t.bx + bw4)) {
        masks[0] |= 1u64 << 32;
    }
}

fn derive_warpmv(
    t: &Dav1dTaskContext,
    rf: &RefmvsFrame,
    bw4: i32,
    bh4: i32,
    masks: &[u64; 2],
    mv: Mv,
    wmp: &mut Dav1dWarpedMotionParams,
) {
    let mut pts = [[[0i32; 2 /* x, y */]; 2 /* in, out */]; 8];
    let mut np = 0;

    let mut add_sample = |np: &mut usize, dx: i32, dy: i32, sx: i32, sy: i32, rp: &RefmvsBlock| {
        pts[*np][0][0] = 16 * (2 * dx + sx * rbs(rp)[0] as i32) - 8;
        pts[*np][0][1] = 16 * (2 * dy + sy * rbs(rp)[1] as i32) - 8;
        pts[*np][1][0] = pts[*np][0][0] + rp.mv.mv[0].x as i32;
        pts[*np][1][1] = pts[*np][0][1] + rp.mv.mv[0].y as i32;
        *np += 1;
    };

    // use masks[] to find the projectable motion vectors in the edges
    if masks[0] as u32 == 1 && (masks[1] >> 32) == 0 {
        let off = t.bx & (rbs(rblk(rf, &t.rt, t.by, -1, t.bx))[0] as i32 - 1);
        add_sample(&mut np, -off, 0, 1, -1, rblk(rf, &t.rt, t.by, -1, t.bx));
    } else {
        // top
        let mut off: u32 = 0;
        let mut xmask = masks[0] as u32;
        while np < 8 && xmask != 0 {
            let tz = ctz(xmask);
            off += tz as u32;
            xmask >>= tz;
            add_sample(
                &mut np,
                off as i32,
                0,
                1,
                -1,
                rblk(rf, &t.rt, t.by, -1, t.bx + off as i32),
            );
            xmask &= !1;
        }
    }
    if np < 8 && masks[1] == 1 {
        let off = t.by & (rbs(rblk(rf, &t.rt, t.by, 0, t.bx - 1))[1] as i32 - 1);
        add_sample(
            &mut np,
            0,
            -off,
            -1,
            1,
            rblk(rf, &t.rt, t.by, -off, t.bx - 1),
        );
    } else {
        // left
        let mut off: u32 = 0;
        let mut ymask = masks[1] as u32;
        while np < 8 && ymask != 0 {
            let tz = ctz(ymask);
            off += tz as u32;
            ymask >>= tz;
            add_sample(
                &mut np,
                0,
                off as i32,
                -1,
                1,
                rblk(rf, &t.rt, t.by, off as i32, t.bx - 1),
            );
            ymask &= !1;
        }
    }
    if np < 8 && (masks[1] >> 32) != 0 {
        // top/left
        add_sample(&mut np, 0, 0, -1, -1, rblk(rf, &t.rt, t.by, -1, t.bx - 1));
    }
    if np < 8 && (masks[0] >> 32) != 0 {
        // top/right
        add_sample(
            &mut np,
            bw4,
            0,
            1,
            -1,
            rblk(rf, &t.rt, t.by, -1, t.bx + bw4),
        );
    }
    debug_assert!(np > 0 && np <= 8);

    // select according to motion vector difference against a threshold
    let mut mvd = [0i32; 8];
    let mut ret = 0;
    let thresh = 4 * iclip(imax(bw4, bh4), 4, 28);
    for i in 0..np {
        mvd[i] = (pts[i][1][0] - pts[i][0][0] - mv.x as i32).abs()
            + (pts[i][1][1] - pts[i][0][1] - mv.y as i32).abs();
        if mvd[i] > thresh {
            mvd[i] = -1;
        } else {
            ret += 1;
        }
    }
    if ret == 0 {
        ret = 1;
    } else {
        let mut i = 0usize;
        let mut j = np as isize - 1;
        let mut k = 0;
        while k < np - ret {
            while mvd[i] != -1 {
                i += 1;
            }
            while mvd[j as usize] == -1 {
                j -= 1;
            }
            debug_assert!(i as isize != j);
            if i as isize > j {
                break;
            }
            // replace the discarded samples;
            mvd[i] = mvd[j as usize];
            pts[i] = pts[j as usize];
            k += 1;
            i += 1;
            j -= 1;
        }
    }

    if !find_affine_int(&pts, ret, bw4, bh4, mv, wmp, t.bx, t.by) && !get_shear_params(wmp) {
        wmp.type_ = DAV1D_WM_TYPE_AFFINE;
    } else {
        wmp.type_ = DAV1D_WM_TYPE_IDENTITY;
    }
}

#[inline]
fn findoddzero(buf: &[u8], len: i32) -> bool {
    for n in 0..len as usize {
        if buf[n * 2] == 0 {
            return true;
        }
    }
    false
}

fn read_pal_plane(
    t: &mut Dav1dTaskContext,
    a: &BlockContext,
    ts: &mut Dav1dTileState,
    bpc: i32,
    b: &mut Av1Block,
    pl: usize,
    sz_ctx: usize,
    bx4: usize,
    by4: usize,
) {
    let pal_sz = ts
        .msac
        .decode_symbol_adapt8(&mut ts.cdf.m.pal_sz[pl][sz_ctx], 6) as usize
        + 2;
    b.pal_sz[pl] = pal_sz as u8;
    let mut cache = [0u16; 16];
    let mut used_cache = [0u16; 8];
    let mut l_cache = if pl != 0 {
        t.pal_sz_uv[1][by4]
    } else {
        t.l.pal_sz[by4]
    } as i32;
    let mut n_cache = 0;
    // don't reuse above palette outside SB64 boundaries
    let mut a_cache = if by4 & 15 != 0 {
        if pl != 0 {
            t.pal_sz_uv[0][bx4]
        } else {
            a.pal_sz[bx4]
        }
    } else {
        0
    } as i32;
    let l = t.al_pal[1][by4][pl];
    let a_ = t.al_pal[0][bx4][pl];
    let (mut li, mut ai) = (0, 0);

    // fill/sort cache
    while l_cache != 0 && a_cache != 0 {
        if l[li] < a_[ai] {
            if n_cache == 0 || cache[n_cache - 1] != l[li] {
                cache[n_cache] = l[li];
                n_cache += 1;
            }
            li += 1;
            l_cache -= 1;
        } else {
            if a_[ai] == l[li] {
                li += 1;
                l_cache -= 1;
            }
            if n_cache == 0 || cache[n_cache - 1] != a_[ai] {
                cache[n_cache] = a_[ai];
                n_cache += 1;
            }
            ai += 1;
            a_cache -= 1;
        }
    }
    if l_cache != 0 {
        loop {
            if n_cache == 0 || cache[n_cache - 1] != l[li] {
                cache[n_cache] = l[li];
                n_cache += 1;
            }
            li += 1;
            l_cache -= 1;
            if l_cache <= 0 {
                break;
            }
        }
    } else if a_cache != 0 {
        loop {
            if n_cache == 0 || cache[n_cache - 1] != a_[ai] {
                cache[n_cache] = a_[ai];
                n_cache += 1;
            }
            ai += 1;
            a_cache -= 1;
            if a_cache <= 0 {
                break;
            }
        }
    }

    // find reused cache entries
    let mut i = 0;
    let mut n = 0;
    while n < n_cache && i < pal_sz {
        if ts.msac.decode_bool_equi() != 0 {
            used_cache[i] = cache[n];
            i += 1;
        }
        n += 1;
    }
    let n_used_cache = i;

    // parse new entries
    let pal = &mut t.scratch.pal[pl];
    if i < pal_sz {
        let mut prev = ts.msac.decode_bools(bpc as u32) as i32;
        pal[i] = prev as u16;
        i += 1;

        if i < pal_sz {
            let mut bits = bpc - 3 + ts.msac.decode_bools(2) as i32;
            let max = (1 << bpc) - 1;
            let not_pl = (pl == 0) as i32;

            loop {
                let delta = ts.msac.decode_bools(bits as u32) as i32;
                prev = imin(prev + delta + not_pl, max);
                pal[i] = prev as u16;
                i += 1;
                if prev + not_pl >= max {
                    while i < pal_sz {
                        pal[i] = max as u16;
                        i += 1;
                    }
                    break;
                }
                bits = imin(bits, 1 + ulog2((max - prev - not_pl) as u32));
                if i >= pal_sz {
                    break;
                }
            }
        }

        // merge cache+new entries
        let mut n = 0;
        let mut m = n_used_cache;
        for i in 0..pal_sz {
            if n < n_used_cache && (m >= pal_sz || used_cache[n] <= pal[m]) {
                pal[i] = used_cache[n];
                n += 1;
            } else {
                debug_assert!(m < pal_sz);
                pal[i] = pal[m];
                m += 1;
            }
        }
    } else {
        pal[..n_used_cache].copy_from_slice(&used_cache[..n_used_cache]);
    }
}

fn read_pal_uv(
    t: &mut Dav1dTaskContext,
    a: &BlockContext,
    ts: &mut Dav1dTileState,
    bpc: i32,
    b: &mut Av1Block,
    sz_ctx: usize,
    bx4: usize,
    by4: usize,
) {
    read_pal_plane(t, a, ts, bpc, b, 1, sz_ctx, bx4, by4);

    // V pal coding
    let pal = &mut t.scratch.pal[2];
    if ts.msac.decode_bool_equi() != 0 {
        let bits = bpc - 4 + ts.msac.decode_bools(2) as i32;
        let mut prev = ts.msac.decode_bools(bpc as u32) as i32;
        pal[0] = prev as u16;
        let max = (1 << bpc) - 1;
        for i in 1..b.pal_sz[1] as usize {
            let mut delta = ts.msac.decode_bools(bits as u32) as i32;
            if delta != 0 && ts.msac.decode_bool_equi() != 0 {
                delta = -delta;
            }
            prev = (prev + delta) & max;
            pal[i] = prev as u16;
        }
    } else {
        for i in 0..b.pal_sz[1] as usize {
            pal[i] = ts.msac.decode_bools(bpc as u32) as u16;
        }
    }
}

// meant to be SIMD'able, so that theoretical complexity of this function
// times block size goes from w4*h4 to w4+h4-1
// a and b are previous two lines containing (a) top/left entries or (b)
// top/left entries, with a[0] being either the first top or first left entry,
// depending on top_offset being 1 or 0, and b being the first top/left entry
// for whichever has one. left_offset indicates whether the (len-1)th entry
// has a left neighbour.
// output is order[] and ctx for each member of this diagonal.
fn order_palette(
    pal_idx: &[u8],
    p0: usize,
    stride: usize,
    i: usize,
    first: usize,
    last: usize,
    order: &mut [[u8; 8]; 64],
    ctx: &mut [u8; 64],
) {
    let mut have_top = i > first;

    let mut p = p0 + first + (i - first) * stride;
    let mut n = 0;
    let mut j = first as isize;
    while j >= last as isize {
        let have_left = j > 0;

        debug_assert!(have_left || have_top);

        let mut mask: u32 = 0;
        let mut o_idx = 0;
        let mut add = |order: &mut [[u8; 8]; 64], v: u8| {
            debug_assert!(v < 8);
            order[n][o_idx] = v;
            o_idx += 1;
            mask |= 1 << v;
        };
        if !have_left {
            ctx[n] = 0;
            add(order, pal_idx[p - stride]);
        } else if !have_top {
            ctx[n] = 0;
            add(order, pal_idx[p - 1]);
        } else {
            let l = pal_idx[p - 1];
            let t = pal_idx[p - stride];
            let tl = pal_idx[p - (stride + 1)];
            let same_t_l = t == l;
            let same_t_tl = t == tl;
            let same_l_tl = l == tl;
            let same_all = same_t_l & same_t_tl & same_l_tl;

            if same_all {
                ctx[n] = 4;
                add(order, t);
            } else if same_t_l {
                ctx[n] = 3;
                add(order, t);
                add(order, tl);
            } else if same_t_tl | same_l_tl {
                ctx[n] = 2;
                add(order, tl);
                add(order, if same_t_tl { l } else { t });
            } else {
                ctx[n] = 1;
                add(order, t.min(l));
                add(order, t.max(l));
                add(order, tl);
            }
        }
        let mut m: u32 = 1;
        let mut bit = 0u8;
        while m < 0x100 {
            if mask & m == 0 {
                order[n][o_idx] = bit;
                o_idx += 1;
            }
            m <<= 1;
            bit += 1;
        }
        debug_assert!(o_idx == 8);
        have_top = true;
        j -= 1;
        n += 1;
        p += stride - 1;
    }
}

fn read_pal_indices(
    ts: &mut Dav1dTileState,
    scratch: &mut TaskScratch,
    p0: usize,
    b: &Av1Block,
    pl: usize,
    w4: i32,
    h4: i32,
    bw4: i32,
    bh4: i32,
) {
    let stride = bw4 as usize * 4;
    let TaskScratch {
        pal_idx,
        pal_order: order,
        pal_ctx: ctx,
        ..
    } = scratch;
    pal_idx[p0] = ts.msac.decode_uniform(b.pal_sz[pl] as u32) as u8;
    let color_map_cdf = &mut ts.cdf.m.color_map[pl][b.pal_sz[pl] as usize - 2];
    for i in 1..(4 * (w4 + h4) - 1) as usize {
        // top/left-to-bottom/right diagonals ("wave-front")
        let first = imin(i as i32, w4 * 4 - 1) as usize;
        let last = imax(0, i as i32 - h4 * 4 + 1) as usize;
        order_palette(pal_idx, p0, stride, i, first, last, order, ctx);
        let mut m = 0;
        let mut j = first as isize;
        while j >= last as isize {
            let color_idx = ts.msac.decode_symbol_adapt8(
                &mut color_map_cdf[ctx[m] as usize],
                b.pal_sz[pl] as usize - 1,
            );
            pal_idx[p0 + (i - j as usize) * stride + j as usize] = order[m][color_idx as usize];
            j -= 1;
            m += 1;
        }
    }
    // fill invisible edges
    if bw4 > w4 {
        for y in 0..(4 * h4) as usize {
            let row = p0 + y * stride;
            let v = pal_idx[row + 4 * w4 as usize - 1];
            pal_idx[row + 4 * w4 as usize..row + 4 * bw4 as usize].fill(v);
        }
    }
    if h4 < bh4 {
        let src = p0 + stride * (4 * h4 as usize - 1);
        for y in (h4 * 4) as usize..(bh4 * 4) as usize {
            pal_idx.copy_within(src..src + bw4 as usize * 4, p0 + y * stride);
        }
    }
}

fn read_vartx_tree(
    t: &mut Dav1dTaskContext,
    a: &mut BlockContext,
    ts: &mut Dav1dTileState,
    f_bw: i32,
    f_bh: i32,
    frame_hdr: &Dav1dFrameHeader,
    layout: i32,
    b: &mut Av1Block,
    bs: u8,
    bx4: usize,
    by4: usize,
) {
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];
    let bw4 = b_dim[0] as i32;
    let bh4 = b_dim[1] as i32;

    // var-tx tree coding
    let mut tx_split = [0u16; 2];
    b.max_ytx = MAX_TXFM_SIZE_FOR_BS[bs as usize][0];
    if b.skip == 0
        && (frame_hdr.segmentation.lossless[b.seg_id as usize] != 0 || b.max_ytx == TX_4X4)
    {
        b.max_ytx = TX_4X4;
        b.uvtx = TX_4X4;
        if frame_hdr.txfm_mode == DAV1D_TX_SWITCHABLE {
            set_ctx(&mut t.l.tx, by4, bh4 as usize, TX_4X4 as i8);
            set_ctx(&mut a.tx, bx4, bw4 as usize, TX_4X4 as i8);
        }
    } else if frame_hdr.txfm_mode != DAV1D_TX_SWITCHABLE || b.skip != 0 {
        if frame_hdr.txfm_mode == DAV1D_TX_SWITCHABLE {
            set_ctx(&mut t.l.tx, by4, bh4 as usize, b_dim[3] as i8);
            set_ctx(&mut a.tx, bx4, bw4 as usize, b_dim[2] as i8);
        }
        b.uvtx = MAX_TXFM_SIZE_FOR_BS[bs as usize][layout as usize];
    } else {
        debug_assert!(bw4 <= 16 || bh4 <= 16 || b.max_ytx == TX_64X64);
        let ytx = &TXFM_DIMENSIONS[b.max_ytx as usize];
        let mut y = 0;
        let mut y_off = 0;
        while y < bh4 {
            let mut x = 0;
            let mut x_off = 0;
            while x < bw4 {
                read_tx_tree(
                    t,
                    a,
                    ts,
                    f_bw,
                    f_bh,
                    b.max_ytx,
                    0,
                    &mut tx_split,
                    x_off,
                    y_off,
                );
                // contexts are updated inside read_tx_tree()
                t.bx += ytx.w as i32;
                x += ytx.w as i32;
                x_off += 1;
            }
            t.bx -= x;
            t.by += ytx.h as i32;
            y += ytx.h as i32;
            y_off += 1;
        }
        t.by -= y;
        b.uvtx = MAX_TXFM_SIZE_FOR_BS[bs as usize][layout as usize];
    }
    debug_assert!(tx_split[0] & !0x33 == 0);
    b.tx_split0 = tx_split[0] as u8;
    b.tx_split1 = tx_split[1];
}

#[inline]
fn get_prev_frame_segid(
    by: i32,
    bx: i32,
    w4: i32,
    mut h4: i32,
    ref_seg_map: &[u8],
    stride: usize,
) -> u32 {
    let mut seg_id = 8u32;
    let mut p = by as usize * stride + bx as usize;
    loop {
        for x in 0..w4 as usize {
            seg_id = umin(seg_id, ref_seg_map[p + x] as u32);
        }
        p += stride;
        h4 -= 1;
        if !(h4 > 0 && seg_id != 0) {
            break;
        }
    }
    debug_assert!(seg_id < 8);

    seg_id
}

#[inline]
fn splat_oneref_mv(
    rf: &mut RefmvsFrame,
    t: &Dav1dTaskContext,
    bs: u8,
    b: &Av1Block,
    bw4: i32,
    bh4: i32,
) {
    let mode = b.inter_mode;
    let tmpl = RefmvsBlock {
        ref_: RefmvsRefpair {
            ref_: [b.ref_[0] + 1, if b.interintra_type != 0 { 0 } else { -1 }],
        },
        mv: RefmvsMvpair {
            mv: [b.mv[0], Mv::ZERO],
        },
        bs,
        mf: ((mode == GLOBALMV && imin(bw4, bh4) >= 2) as u8) | ((mode == NEWMV) as u8 * 2),
    };
    let rr = &t.rt.r[((t.by & 31) + 5) as usize..];
    splat_mv(
        &mut rf.r,
        rr,
        &tmpl,
        t.bx as usize,
        bw4 as usize,
        bh4 as usize,
    );
}

#[inline]
fn splat_intrabc_mv(
    rf: &mut RefmvsFrame,
    t: &Dav1dTaskContext,
    bs: u8,
    b: &Av1Block,
    bw4: i32,
    bh4: i32,
) {
    let tmpl = RefmvsBlock {
        ref_: RefmvsRefpair { ref_: [0, -1] },
        mv: RefmvsMvpair {
            mv: [b.mv[0], Mv::ZERO],
        },
        bs,
        mf: 0,
    };
    let rr = &t.rt.r[((t.by & 31) + 5) as usize..];
    splat_mv(
        &mut rf.r,
        rr,
        &tmpl,
        t.bx as usize,
        bw4 as usize,
        bh4 as usize,
    );
}

#[inline]
fn splat_tworef_mv(
    rf: &mut RefmvsFrame,
    t: &Dav1dTaskContext,
    bs: u8,
    b: &Av1Block,
    bw4: i32,
    bh4: i32,
) {
    debug_assert!(bw4 >= 2 && bh4 >= 2);
    let mode = b.inter_mode;
    let tmpl = RefmvsBlock {
        ref_: RefmvsRefpair {
            ref_: [b.ref_[0] + 1, b.ref_[1] + 1],
        },
        mv: RefmvsMvpair { mv: b.mv },
        bs,
        mf: ((mode == GLOBALMV_GLOBALMV) as u8) | (((1u32 << mode) & 0xbc != 0) as u8 * 2),
    };
    let rr = &t.rt.r[((t.by & 31) + 5) as usize..];
    splat_mv(
        &mut rf.r,
        rr,
        &tmpl,
        t.bx as usize,
        bw4 as usize,
        bh4 as usize,
    );
}

#[inline]
fn splat_intraref(rf: &mut RefmvsFrame, t: &Dav1dTaskContext, bs: u8, bw4: i32, bh4: i32) {
    let tmpl = RefmvsBlock {
        ref_: RefmvsRefpair { ref_: [0, -1] },
        mv: RefmvsMvpair {
            mv: [INVALID_MV_MV, Mv::ZERO],
        },
        bs,
        mf: 0,
    };
    let rr = &t.rt.r[((t.by & 31) + 5) as usize..];
    splat_mv(
        &mut rf.r,
        rr,
        &tmpl,
        t.bx as usize,
        bw4 as usize,
        bh4 as usize,
    );
}

// (mc_lowest_px(), affine_lowest_px(), affine_lowest_px_luma(),
// affine_lowest_px_chroma() and obmc_lowest_px() track the reference rows
// a frame thread waits for; they are not needed with one frame at a time)

/// The block's segmentation data (`seg`).
type SegRef<'a> = Option<&'a Dav1dSegmentationData>;

fn decode_b<P: Pixel>(
    inloop_filters: u32,
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    ts: &mut Dav1dTileState,
    bl: u8,
    bs: u8,
    bp: u8,
    intra_edge_flags: u8,
) -> Result<(), ()> {
    let _ = inloop_filters;
    let seq_hdr_arc = f.seq_hdr.clone().expect("sequence header");
    let frame_hdr_arc = f.frame_hdr.clone().expect("frame header");
    let seq_hdr: &Dav1dSequenceHeader = &seq_hdr_arc;
    let frame_hdr: &Dav1dFrameHeader = &frame_hdr_arc;
    let mut b = Av1Block::default();
    let b = &mut b;
    let b_dim = &BLOCK_DIMENSIONS[bs as usize];
    let bx4 = (t.bx & 31) as usize;
    let by4 = (t.by & 31) as usize;
    let layout = f.cur.p.layout;
    let ss_ver = (layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
    let cbx4 = bx4 >> ss_hor;
    let cby4 = by4 >> ss_ver;
    let bw4 = b_dim[0] as i32;
    let bh4 = b_dim[1] as i32;
    let w4 = imin(bw4, f.bw - t.bx);
    let h4 = imin(bh4, f.bh - t.by);
    let cbw4 = (bw4 + ss_hor) >> ss_hor;
    let cbh4 = (bh4 + ss_ver) >> ss_ver;
    let have_left = t.bx > ts.tiling.col_start;
    let have_top = t.by > ts.tiling.row_start;
    let has_chroma = layout != DAV1D_PIXEL_LAYOUT_I400
        && (bw4 > ss_hor || t.bx & 1 != 0)
        && (bh4 > ss_ver || t.by & 1 != 0);
    let b4_stride = f.b4_stride;
    let ta = t.a;

    let cw4 = (w4 + ss_hor) >> ss_hor;
    let ch4 = (h4 + ss_ver) >> ss_ver;

    b.bl = bl;
    b.bp = bp;
    b.bs = bs;

    let mut seg: SegRef<'_> = None;

    // segment_id (if seg_feature for skip/ref/gmv is enabled)
    let mut seg_pred = 0u8;
    if frame_hdr.segmentation.enabled != 0 {
        if frame_hdr.segmentation.update_map == 0 {
            if let Some(prev) = &f.prev_segmap {
                let seg_id = get_prev_frame_segid(t.by, t.bx, w4, h4, prev, b4_stride);
                if seg_id >= 8 {
                    return Err(());
                }
                b.seg_id = seg_id as u8;
            } else {
                b.seg_id = 0;
            }
            seg = Some(&frame_hdr.segmentation.seg_data.d[b.seg_id as usize]);
        } else if frame_hdr.segmentation.seg_data.preskip != 0 {
            if frame_hdr.segmentation.temporal != 0 && {
                seg_pred = ts.msac.decode_bool_adapt(
                    &mut ts.cdf.m.seg_pred[(f.a[ta].seg_pred[bx4] + t.l.seg_pred[by4]) as usize],
                ) as u8;
                seg_pred != 0
            } {
                // temporal predicted seg_id
                if let Some(prev) = &f.prev_segmap {
                    let seg_id = get_prev_frame_segid(t.by, t.bx, w4, h4, prev, b4_stride);
                    if seg_id >= 8 {
                        return Err(());
                    }
                    b.seg_id = seg_id as u8;
                } else {
                    b.seg_id = 0;
                }
            } else {
                let (pred_seg_id, seg_ctx) = get_cur_frame_segid(
                    t.by as usize,
                    t.bx as usize,
                    have_top,
                    have_left,
                    f.cur_segmap.as_deref().expect("segmentation map"),
                    b4_stride,
                );
                let diff = ts
                    .msac
                    .decode_symbol_adapt8(&mut ts.cdf.m.seg_id[seg_ctx], DAV1D_MAX_SEGMENTS - 1);
                let last_active_seg_id = frame_hdr.segmentation.seg_data.last_active_segid;
                let mut seg_id =
                    neg_deinterleave(diff as i32, pred_seg_id as i32, last_active_seg_id + 1)
                        as u32;
                if seg_id > last_active_seg_id as u32 {
                    seg_id = 0; // error?
                }
                if seg_id >= DAV1D_MAX_SEGMENTS as u32 {
                    seg_id = 0; // error?
                }
                b.seg_id = seg_id as u8;
            }

            seg = Some(&frame_hdr.segmentation.seg_data.d[b.seg_id as usize]);
        }
    } else {
        b.seg_id = 0;
    }

    // skip_mode
    if seg.is_none_or(|s| s.globalmv == 0 && s.ref_ == -1 && s.skip == 0)
        && frame_hdr.skip_mode_enabled != 0
        && imin(bw4, bh4) > 1
    {
        let smctx = (f.a[ta].skip_mode[bx4] + t.l.skip_mode[by4]) as usize;
        b.skip_mode = ts.msac.decode_bool_adapt(&mut ts.cdf.m.skip_mode[smctx]) as u8;
    } else {
        b.skip_mode = 0;
    }

    // skip
    if b.skip_mode != 0 || seg.is_some_and(|s| s.skip != 0) {
        b.skip = 1;
    } else {
        let sctx = (f.a[ta].skip[bx4] + t.l.skip[by4]) as usize;
        b.skip = ts.msac.decode_bool_adapt(&mut ts.cdf.m.skip[sctx]) as u8;
    }

    // segment_id
    if frame_hdr.segmentation.enabled != 0
        && frame_hdr.segmentation.update_map != 0
        && frame_hdr.segmentation.seg_data.preskip == 0
    {
        if b.skip == 0 && frame_hdr.segmentation.temporal != 0 && {
            seg_pred = ts.msac.decode_bool_adapt(
                &mut ts.cdf.m.seg_pred[(f.a[ta].seg_pred[bx4] + t.l.seg_pred[by4]) as usize],
            ) as u8;
            seg_pred != 0
        } {
            // temporal predicted seg_id
            if let Some(prev) = &f.prev_segmap {
                let seg_id = get_prev_frame_segid(t.by, t.bx, w4, h4, prev, b4_stride);
                if seg_id >= 8 {
                    return Err(());
                }
                b.seg_id = seg_id as u8;
            } else {
                b.seg_id = 0;
            }
        } else {
            let (pred_seg_id, seg_ctx) = get_cur_frame_segid(
                t.by as usize,
                t.bx as usize,
                have_top,
                have_left,
                f.cur_segmap.as_deref().expect("segmentation map"),
                b4_stride,
            );
            let mut seg_id;
            if b.skip != 0 {
                seg_id = pred_seg_id;
            } else {
                let diff = ts
                    .msac
                    .decode_symbol_adapt8(&mut ts.cdf.m.seg_id[seg_ctx], DAV1D_MAX_SEGMENTS - 1);
                let last_active_seg_id = frame_hdr.segmentation.seg_data.last_active_segid;
                seg_id = neg_deinterleave(diff as i32, pred_seg_id as i32, last_active_seg_id + 1)
                    as u32;
                if seg_id > last_active_seg_id as u32 {
                    seg_id = 0; // error?
                }
            }
            if seg_id >= DAV1D_MAX_SEGMENTS as u32 {
                seg_id = 0; // error?
            }
            b.seg_id = seg_id as u8;
        }

        seg = Some(&frame_hdr.segmentation.seg_data.d[b.seg_id as usize]);
    }

    // cdef index
    if b.skip == 0 {
        let idx = if seq_hdr.sb128 != 0 {
            (((t.bx & 16) >> 4) + ((t.by & 16) >> 3)) as usize
        } else {
            0
        };
        let (mi, off) = t.cur_sb_cdef_idx;
        let cdef_idx = &mut f.lf.mask[mi].cdef_idx;
        if cdef_idx[off + idx] == -1 {
            let v = ts.msac.decode_bools(frame_hdr.cdef.n_bits as u32) as i8;
            cdef_idx[off + idx] = v;
            if bw4 > 16 {
                cdef_idx[off + idx + 1] = v;
            }
            if bh4 > 16 {
                cdef_idx[off + idx + 2] = v;
            }
            if bw4 == 32 && bh4 == 32 {
                cdef_idx[off + idx + 3] = v;
            }
        }
    }

    // delta-q/lf
    let sb_mask = 31 >> (seq_hdr.sb128 == 0) as i32;
    if t.bx & sb_mask == 0 && t.by & sb_mask == 0 {
        let prev_qidx = ts.last_qidx;
        let have_delta_q = frame_hdr.delta.q.present != 0
            && (bs
                != (if seq_hdr.sb128 != 0 {
                    BS_128X128
                } else {
                    BS_64X64
                })
                || b.skip == 0);

        let prev_delta_lf = ts.last_delta_lf;

        if have_delta_q {
            let mut delta_q = ts.msac.decode_symbol_adapt4(&mut ts.cdf.m.delta_q, 3) as i32;
            if delta_q == 3 {
                let n_bits = 1 + ts.msac.decode_bools(3);
                delta_q = ts.msac.decode_bools(n_bits) as i32 + 1 + (1 << n_bits);
            }
            if delta_q != 0 {
                if ts.msac.decode_bool_equi() != 0 {
                    delta_q = -delta_q;
                }
                delta_q *= 1 << frame_hdr.delta.q.res_log2;
            }
            ts.last_qidx = iclip(ts.last_qidx + delta_q, 1, 255);

            if frame_hdr.delta.lf.present != 0 {
                let n_lfs = if frame_hdr.delta.lf.multi != 0 {
                    if layout != DAV1D_PIXEL_LAYOUT_I400 {
                        4
                    } else {
                        2
                    }
                } else {
                    1
                };

                for i in 0..n_lfs {
                    let mut delta_lf = ts.msac.decode_symbol_adapt4(
                        &mut ts.cdf.m.delta_lf[i + frame_hdr.delta.lf.multi as usize],
                        3,
                    ) as i32;
                    if delta_lf == 3 {
                        let n_bits = 1 + ts.msac.decode_bools(3);
                        delta_lf = ts.msac.decode_bools(n_bits) as i32 + 1 + (1 << n_bits);
                    }
                    if delta_lf != 0 {
                        if ts.msac.decode_bool_equi() != 0 {
                            delta_lf = -delta_lf;
                        }
                        delta_lf *= 1 << frame_hdr.delta.lf.res_log2;
                    }
                    ts.last_delta_lf[i] =
                        iclip(ts.last_delta_lf[i] as i32 + delta_lf, -63, 63) as i8;
                }
            }
        }
        if ts.last_qidx == frame_hdr.quant.yac {
            // assign frame-wide q values to this sb
            ts.dq = TileDq::Frame;
        } else if ts.last_qidx != prev_qidx {
            // find sb-specific quant parameters
            init_quant_tables(seq_hdr, frame_hdr, ts.last_qidx, &mut ts.dqmem);
            ts.dq = TileDq::Tile;
        }
        if ts.last_delta_lf == [0, 0, 0, 0] {
            // assign frame-wide lf values to this sb
            ts.lflvl = TileLflvl::Frame;
        } else if ts.last_delta_lf != prev_delta_lf {
            // find sb-specific lf lvl parameters
            calc_lf_values(&mut ts.lflvlmem, frame_hdr, &ts.last_delta_lf);
            ts.lflvl = TileLflvl::Tile;
        }
    }

    if b.skip_mode != 0 {
        b.intra = 0;
    } else if is_inter_or_switch(frame_hdr) {
        if let Some(s) = seg.filter(|s| s.ref_ >= 0 || s.globalmv != 0) {
            b.intra = (s.ref_ == 0) as u8;
        } else {
            let ictx = get_intra_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
            b.intra = (ts.msac.decode_bool_adapt(&mut ts.cdf.m.intra[ictx]) == 0) as u8;
        }
    } else if frame_hdr.allow_intrabc != 0 {
        b.intra = (ts.msac.decode_bool_adapt(&mut ts.cdf.m.intrabc) == 0) as u8;
    } else {
        b.intra = 1;
    }

    // intra/inter-specific stuff
    if b.intra != 0 {
        let ymode_cdf: &mut [u16] = if is_inter_or_switch(frame_hdr) {
            &mut ts.cdf.m.y_mode[YMODE_SIZE_CONTEXT[bs as usize] as usize]
        } else {
            &mut ts.cdf.kfym[INTRA_MODE_CONTEXT[f.a[ta].mode[bx4] as usize] as usize]
                [INTRA_MODE_CONTEXT[t.l.mode[by4] as usize] as usize]
        };
        b.y_mode = ts
            .msac
            .decode_symbol_adapt16(ymode_cdf, N_INTRA_PRED_MODES - 1) as u8;

        // angle delta
        if b_dim[2] + b_dim[3] >= 2 && b.y_mode >= VERT_PRED && b.y_mode <= VERT_LEFT_PRED {
            let acdf = &mut ts.cdf.m.angle_delta[(b.y_mode - VERT_PRED) as usize];
            let angle = ts.msac.decode_symbol_adapt8(acdf, 6) as i32;
            b.y_angle = (angle - 3) as i8;
        } else {
            b.y_angle = 0;
        }

        if has_chroma {
            let cfl_allowed = if frame_hdr.segmentation.lossless[b.seg_id as usize] != 0 {
                cbw4 == 1 && cbh4 == 1
            } else {
                CFL_ALLOWED_MASK & (1 << bs) != 0
            };
            let uvmode_cdf = &mut ts.cdf.m.uv_mode[cfl_allowed as usize][b.y_mode as usize];
            b.uv_mode = ts.msac.decode_symbol_adapt16(
                uvmode_cdf,
                N_UV_INTRA_PRED_MODES - 1 - (!cfl_allowed) as usize,
            ) as u8;

            b.uv_angle = 0;
            if b.uv_mode == CFL_PRED {
                let sign = ts.msac.decode_symbol_adapt8(&mut ts.cdf.m.cfl_sign, 7) as i32 + 1;
                let sign_u = sign * 0x56 >> 8;
                let sign_v = sign - sign_u * 3;
                debug_assert!(sign_u == sign / 3);
                if sign_u != 0 {
                    let ctx = ((sign_u == 2) as i32 * 3 + sign_v) as usize;
                    b.cfl_alpha[0] = (ts
                        .msac
                        .decode_symbol_adapt16(&mut ts.cdf.m.cfl_alpha[ctx], 15)
                        + 1) as i8;
                    if sign_u == 1 {
                        b.cfl_alpha[0] = -b.cfl_alpha[0];
                    }
                } else {
                    b.cfl_alpha[0] = 0;
                }
                if sign_v != 0 {
                    let ctx = ((sign_v == 2) as i32 * 3 + sign_u) as usize;
                    b.cfl_alpha[1] = (ts
                        .msac
                        .decode_symbol_adapt16(&mut ts.cdf.m.cfl_alpha[ctx], 15)
                        + 1) as i8;
                    if sign_v == 1 {
                        b.cfl_alpha[1] = -b.cfl_alpha[1];
                    }
                } else {
                    b.cfl_alpha[1] = 0;
                }
            } else if b_dim[2] + b_dim[3] >= 2
                && b.uv_mode >= VERT_PRED
                && b.uv_mode <= VERT_LEFT_PRED
            {
                let acdf = &mut ts.cdf.m.angle_delta[(b.uv_mode - VERT_PRED) as usize];
                let angle = ts.msac.decode_symbol_adapt8(acdf, 6) as i32;
                b.uv_angle = (angle - 3) as i8;
            }
        }

        b.pal_sz = [0, 0];
        if frame_hdr.allow_screen_content_tools != 0 && imax(bw4, bh4) <= 16 && bw4 + bh4 >= 4 {
            let sz_ctx = (b_dim[2] + b_dim[3] - 2) as usize;
            if b.y_mode == DC_PRED {
                let pal_ctx = (f.a[ta].pal_sz[bx4] > 0) as usize + (t.l.pal_sz[by4] > 0) as usize;
                let use_y_pal = ts
                    .msac
                    .decode_bool_adapt(&mut ts.cdf.m.pal_y[sz_ctx][pal_ctx]);
                if use_y_pal != 0 {
                    read_pal_plane(t, &f.a[ta], ts, f.cur.p.bpc, b, 0, sz_ctx, bx4, by4);
                }
            }

            if has_chroma && b.uv_mode == DC_PRED {
                let pal_ctx = (b.pal_sz[0] > 0) as usize;
                let use_uv_pal = ts.msac.decode_bool_adapt(&mut ts.cdf.m.pal_uv[pal_ctx]);
                if use_uv_pal != 0 {
                    // see aomedia bug 2183 for why we use luma coordinates
                    read_pal_uv(t, &f.a[ta], ts, f.cur.p.bpc, b, sz_ctx, bx4, by4);
                }
            }
        }

        if b.y_mode == DC_PRED
            && b.pal_sz[0] == 0
            && imax(b_dim[2] as i32, b_dim[3] as i32) <= 3
            && seq_hdr.filter_intra != 0
        {
            let is_filter = ts
                .msac
                .decode_bool_adapt(&mut ts.cdf.m.use_filter_intra[bs as usize]);
            if is_filter != 0 {
                b.y_mode = FILTER_PRED;
                b.y_angle = ts.msac.decode_symbol_adapt4(&mut ts.cdf.m.filter_intra, 4) as i8;
            }
        }

        if b.pal_sz[0] != 0 {
            read_pal_indices(ts, &mut t.scratch, 0, b, 0, w4, h4, bw4, bh4);
        }

        if has_chroma && b.pal_sz[1] != 0 {
            read_pal_indices(
                ts,
                &mut t.scratch,
                (bw4 * bh4 * 16) as usize,
                b,
                1,
                cw4,
                ch4,
                cbw4,
                cbh4,
            );
        }

        let t_dim;
        if frame_hdr.segmentation.lossless[b.seg_id as usize] != 0 {
            b.tx = TX_4X4;
            b.uvtx = TX_4X4;
            t_dim = &TXFM_DIMENSIONS[TX_4X4 as usize];
        } else {
            b.tx = MAX_TXFM_SIZE_FOR_BS[bs as usize][0];
            b.uvtx = MAX_TXFM_SIZE_FOR_BS[bs as usize][layout as usize];
            let mut td = &TXFM_DIMENSIONS[b.tx as usize];
            if frame_hdr.txfm_mode == DAV1D_TX_SWITCHABLE && td.max > TX_4X4 {
                let tctx = get_tx_ctx(&f.a[ta], &t.l, td, by4, bx4);
                let tx_cdf = &mut ts.cdf.m.txsz[td.max as usize - 1][tctx];
                let mut depth = ts
                    .msac
                    .decode_symbol_adapt4(tx_cdf, imin(td.max as i32, 2) as usize);

                while depth > 0 {
                    depth -= 1;
                    b.tx = td.sub;
                    td = &TXFM_DIMENSIONS[b.tx as usize];
                }
            }
            t_dim = td;
        }

        // reconstruction
        recon_b_intra::<P>(f, t, ts, bs, intra_edge_flags, b);

        if frame_hdr.loopfilter.level_y[0] != 0 || frame_hdr.loopfilter.level_y[1] != 0 {
            let lflvl = match ts.lflvl {
                TileLflvl::Frame => &f.lf.lvl,
                TileLflvl::Tile => &ts.lflvlmem,
            };
            let a = &mut f.a[ta];
            let auv_luv = if has_chroma {
                Some((&mut a.tx_lpf_uv[cbx4..], &mut t.l.tx_lpf_uv[cby4..]))
            } else {
                None
            };
            create_lf_mask_intra(
                &mut f.lf.mask[t.lf_mask],
                &mut f.lf.level,
                b4_stride,
                &lflvl[b.seg_id as usize],
                t.bx,
                t.by,
                f.w4,
                f.h4,
                bs,
                b.tx,
                b.uvtx,
                layout,
                &mut a.tx_lpf_y[bx4..],
                &mut t.l.tx_lpf_y[by4..],
                auv_luv,
            );
        }

        // update contexts
        let y_mode_nofilt = if b.y_mode == FILTER_PRED {
            DC_PRED
        } else {
            b.y_mode
        };
        let inter_or_switch = is_inter_or_switch(frame_hdr);
        let pal_uv = if has_chroma { b.pal_sz[1] } else { 0 };
        {
            let (dir, off, n, txl, idx) = (&mut t.l, by4, bh4 as usize, t_dim.lh, 1);
            set_ctx(&mut dir.tx_intra, off, n, txl as i8);
            set_ctx(&mut dir.tx, off, n, txl as i8);
            set_ctx(&mut dir.mode, off, n, y_mode_nofilt);
            set_ctx(&mut dir.pal_sz, off, n, b.pal_sz[0]);
            set_ctx(&mut dir.seg_pred, off, n, seg_pred);
            set_ctx(&mut dir.skip_mode, off, n, 0);
            set_ctx(&mut dir.intra, off, n, 1);
            set_ctx(&mut dir.skip, off, n, b.skip);
            /* see aomedia bug 2183 for why we use luma coordinates here */
            set_ctx(&mut t.pal_sz_uv[idx], off, n, pal_uv);
            if inter_or_switch {
                set_ctx(&mut dir.comp_type, off, n, COMP_INTER_NONE);
                set_ctx(&mut dir.ref_[0], off, n, -1);
                set_ctx(&mut dir.ref_[1], off, n, -1);
                set_ctx(&mut dir.filter[0], off, n, DAV1D_N_SWITCHABLE_FILTERS as u8);
                set_ctx(&mut dir.filter[1], off, n, DAV1D_N_SWITCHABLE_FILTERS as u8);
            }
        }
        {
            let (dir, off, n, txl, idx) = (&mut f.a[ta], bx4, bw4 as usize, t_dim.lw, 0);
            set_ctx(&mut dir.tx_intra, off, n, txl as i8);
            set_ctx(&mut dir.tx, off, n, txl as i8);
            set_ctx(&mut dir.mode, off, n, y_mode_nofilt);
            set_ctx(&mut dir.pal_sz, off, n, b.pal_sz[0]);
            set_ctx(&mut dir.seg_pred, off, n, seg_pred);
            set_ctx(&mut dir.skip_mode, off, n, 0);
            set_ctx(&mut dir.intra, off, n, 1);
            set_ctx(&mut dir.skip, off, n, b.skip);
            /* see aomedia bug 2183 for why we use luma coordinates here */
            set_ctx(&mut t.pal_sz_uv[idx], off, n, pal_uv);
            if inter_or_switch {
                set_ctx(&mut dir.comp_type, off, n, COMP_INTER_NONE);
                set_ctx(&mut dir.ref_[0], off, n, -1);
                set_ctx(&mut dir.ref_[1], off, n, -1);
                set_ctx(&mut dir.filter[0], off, n, DAV1D_N_SWITCHABLE_FILTERS as u8);
                set_ctx(&mut dir.filter[1], off, n, DAV1D_N_SWITCHABLE_FILTERS as u8);
            }
        }
        if b.pal_sz[0] != 0 {
            let pal = t.scratch.pal[0];
            for x in 0..bw4 as usize {
                t.al_pal[0][bx4 + x][0] = pal;
            }
            for y in 0..bh4 as usize {
                t.al_pal[1][by4 + y][0] = pal;
            }
        }
        if has_chroma {
            set_ctx(&mut t.l.uvmode, cby4, cbh4 as usize, b.uv_mode);
            set_ctx(&mut f.a[ta].uvmode, cbx4, cbw4 as usize, b.uv_mode);
            if b.pal_sz[1] != 0 {
                let pal = t.scratch.pal;
                // see aomedia bug 2183 for why we use luma coordinates here
                for pl in 1..=2 {
                    for x in 0..bw4 as usize {
                        t.al_pal[0][bx4 + x][pl] = pal[pl];
                    }
                    for y in 0..bh4 as usize {
                        t.al_pal[1][by4 + y][pl] = pal[pl];
                    }
                }
            }
        }
        if is_inter_or_switch(frame_hdr) || frame_hdr.allow_intrabc != 0 {
            splat_intraref(&mut f.rf, t, bs, bw4, bh4);
        }
    } else if is_key_or_intra(frame_hdr) {
        // intra block copy
        let mut mvstack = [RefmvsCandidate::default(); 8];
        let (_n_mvs, _ctx) = refmvs_find(
            &t.rt,
            &f.rf,
            &mut mvstack,
            RefmvsRefpair { ref_: [0, -1] },
            bs,
            intra_edge_flags,
            t.by,
            t.bx,
        );

        if mvstack[0].mv.mv[0].n() != 0 {
            b.mv[0] = mvstack[0].mv.mv[0];
        } else if mvstack[1].mv.mv[0].n() != 0 {
            b.mv[0] = mvstack[1].mv.mv[0];
        } else if t.by - (16 << seq_hdr.sb128) < ts.tiling.row_start {
            b.mv[0].y = 0;
            b.mv[0].x = (-(512 << seq_hdr.sb128) - 2048) as i16;
        } else {
            b.mv[0].y = -(512 << seq_hdr.sb128) as i16;
            b.mv[0].x = 0;
        }

        read_mv_residual(ts, &mut b.mv[0], true, false, frame_hdr.hp != 0);

        // clip intrabc motion vector to decoded parts of current tile
        let mut border_left = ts.tiling.col_start * 4;
        let mut border_top = ts.tiling.row_start * 4;
        if has_chroma {
            if bw4 < 2 && ss_hor != 0 {
                border_left += 4;
            }
            if bh4 < 2 && ss_ver != 0 {
                border_top += 4;
            }
        }
        let mut src_left = t.bx * 4 + (b.mv[0].x as i32 >> 3);
        let mut src_top = t.by * 4 + (b.mv[0].y as i32 >> 3);
        let mut src_right = src_left + bw4 * 4;
        let mut src_bottom = src_top + bh4 * 4;
        let border_right = ((ts.tiling.col_end + (bw4 - 1)) & !(bw4 - 1)) * 4;

        // check against left or right tile boundary and adjust if necessary
        if src_left < border_left {
            src_right += border_left - src_left;
            src_left += border_left - src_left;
        } else if src_right > border_right {
            src_left -= src_right - border_right;
            src_right -= src_right - border_right;
        }
        // check against top tile boundary and adjust if necessary
        if src_top < border_top {
            src_bottom += border_top - src_top;
            src_top += border_top - src_top;
        }

        let sbx = (t.bx >> (4 + seq_hdr.sb128)) << (6 + seq_hdr.sb128);
        let sby = (t.by >> (4 + seq_hdr.sb128)) << (6 + seq_hdr.sb128);
        let sb_size = 1 << (6 + seq_hdr.sb128);
        // check for overlap with current superblock
        if src_bottom > sby && src_right > sbx {
            if src_top - border_top >= src_bottom - sby {
                // if possible move src up into the previous suberblock row
                src_top -= src_bottom - sby;
                src_bottom -= src_bottom - sby;
            } else if src_left - border_left >= src_right - sbx {
                // if possible move src left into the previous suberblock
                src_left -= src_right - sbx;
                src_right -= src_right - sbx;
            }
        }
        // move src up if it is below current superblock row
        if src_bottom > sby + sb_size {
            src_top -= src_bottom - (sby + sb_size);
            src_bottom -= src_bottom - (sby + sb_size);
        }
        // error out if mv still overlaps with the current superblock
        if src_bottom > sby && src_right > sbx {
            return Err(());
        }

        b.mv[0].x = ((src_left - t.bx * 4) * 8) as i16;
        b.mv[0].y = ((src_top - t.by * 4) * 8) as i16;

        read_vartx_tree(
            t,
            &mut f.a[ta],
            ts,
            f.bw,
            f.bh,
            frame_hdr,
            layout,
            b,
            bs,
            bx4,
            by4,
        );

        // reconstruction
        recon_b_inter::<P>(f, t, ts, bs, b)?;

        splat_intrabc_mv(&mut f.rf, t, bs, b, bw4, bh4);

        {
            let (dir, off, n, idx) = (&mut t.l, by4, bh4 as usize, 1);
            set_ctx(&mut dir.tx_intra, off, n, b_dim[2 + idx] as i8);
            set_ctx(&mut dir.mode, off, n, DC_PRED);
            set_ctx(&mut dir.pal_sz, off, n, 0);
            /* see aomedia bug 2183 for why this is outside if (has_chroma) */
            set_ctx(&mut t.pal_sz_uv[idx], off, n, 0);
            set_ctx(&mut dir.seg_pred, off, n, seg_pred);
            set_ctx(&mut dir.skip_mode, off, n, 0);
            set_ctx(&mut dir.intra, off, n, 0);
            set_ctx(&mut dir.skip, off, n, b.skip);
        }
        {
            let (dir, off, n, idx) = (&mut f.a[ta], bx4, bw4 as usize, 0);
            set_ctx(&mut dir.tx_intra, off, n, b_dim[2 + idx] as i8);
            set_ctx(&mut dir.mode, off, n, DC_PRED);
            set_ctx(&mut dir.pal_sz, off, n, 0);
            /* see aomedia bug 2183 for why this is outside if (has_chroma) */
            set_ctx(&mut t.pal_sz_uv[idx], off, n, 0);
            set_ctx(&mut dir.seg_pred, off, n, seg_pred);
            set_ctx(&mut dir.skip_mode, off, n, 0);
            set_ctx(&mut dir.intra, off, n, 0);
            set_ctx(&mut dir.skip, off, n, b.skip);
        }
        if has_chroma {
            set_ctx(&mut t.l.uvmode, cby4, cbh4 as usize, DC_PRED);
            set_ctx(&mut f.a[ta].uvmode, cbx4, cbw4 as usize, DC_PRED);
        }
    } else {
        // inter-specific mode/mv coding
        let is_comp;
        let mut has_subpel_filter;

        if b.skip_mode != 0 {
            is_comp = true;
        } else if seg.is_none_or(|s| s.ref_ == -1 && s.globalmv == 0 && s.skip == 0)
            && frame_hdr.switchable_comp_refs != 0
            && imin(bw4, bh4) > 1
        {
            let ctx = get_comp_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
            is_comp = ts.msac.decode_bool_adapt(&mut ts.cdf.m.comp[ctx]) != 0;
        } else {
            is_comp = false;
        }

        let hp = frame_hdr.hp != 0;
        if b.skip_mode != 0 {
            b.ref_[0] = frame_hdr.skip_mode_refs[0] as i8;
            b.ref_[1] = frame_hdr.skip_mode_refs[1] as i8;
            b.comp_type = COMP_INTER_AVG;
            b.inter_mode = NEARESTMV_NEARESTMV;
            b.drl_idx = NEAREST_DRL;
            has_subpel_filter = false;

            let mut mvstack = [RefmvsCandidate::default(); 8];
            let _ = refmvs_find(
                &t.rt,
                &f.rf,
                &mut mvstack,
                RefmvsRefpair {
                    ref_: [b.ref_[0] + 1, b.ref_[1] + 1],
                },
                bs,
                intra_edge_flags,
                t.by,
                t.bx,
            );

            b.mv[0] = mvstack[0].mv.mv[0];
            b.mv[1] = mvstack[0].mv.mv[1];
            fix_mv_precision(frame_hdr, &mut b.mv[0]);
            fix_mv_precision(frame_hdr, &mut b.mv[1]);
        } else if is_comp {
            let dir_ctx = get_comp_dir_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
            if ts.msac.decode_bool_adapt(&mut ts.cdf.m.comp_dir[dir_ctx]) != 0 {
                // bidir - first reference (fw)
                let ctx1 = av1_get_fwd_ref_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                if ts
                    .msac
                    .decode_bool_adapt(&mut ts.cdf.m.comp_fwd_ref[0][ctx1])
                    != 0
                {
                    let ctx2 = av1_get_fwd_ref_2_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                    b.ref_[0] = 2 + ts
                        .msac
                        .decode_bool_adapt(&mut ts.cdf.m.comp_fwd_ref[2][ctx2])
                        as i8;
                } else {
                    let ctx2 = av1_get_fwd_ref_1_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                    b.ref_[0] = ts
                        .msac
                        .decode_bool_adapt(&mut ts.cdf.m.comp_fwd_ref[1][ctx2])
                        as i8;
                }

                // second reference (bw)
                let ctx3 = av1_get_bwd_ref_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                if ts
                    .msac
                    .decode_bool_adapt(&mut ts.cdf.m.comp_bwd_ref[0][ctx3])
                    != 0
                {
                    b.ref_[1] = 6;
                } else {
                    let ctx4 = av1_get_bwd_ref_1_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                    b.ref_[1] = 4 + ts
                        .msac
                        .decode_bool_adapt(&mut ts.cdf.m.comp_bwd_ref[1][ctx4])
                        as i8;
                }
            } else {
                // unidir
                // (av1_get_uni_p_ctx is av1_get_ref_ctx)
                let uctx_p = av1_get_ref_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                if ts
                    .msac
                    .decode_bool_adapt(&mut ts.cdf.m.comp_uni_ref[0][uctx_p])
                    != 0
                {
                    b.ref_[0] = 4;
                    b.ref_[1] = 6;
                } else {
                    let uctx_p1 = av1_get_uni_p1_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                    b.ref_[0] = 0;
                    b.ref_[1] = 1 + ts
                        .msac
                        .decode_bool_adapt(&mut ts.cdf.m.comp_uni_ref[1][uctx_p1])
                        as i8;
                    if b.ref_[1] == 2 {
                        // (av1_get_uni_p2_ctx is av1_get_fwd_ref_2_ctx)
                        let uctx_p2 =
                            av1_get_fwd_ref_2_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                        b.ref_[1] += ts
                            .msac
                            .decode_bool_adapt(&mut ts.cdf.m.comp_uni_ref[2][uctx_p2])
                            as i8;
                    }
                }
            }

            let mut mvstack = [RefmvsCandidate::default(); 8];
            let (n_mvs, ctx) = refmvs_find(
                &t.rt,
                &f.rf,
                &mut mvstack,
                RefmvsRefpair {
                    ref_: [b.ref_[0] + 1, b.ref_[1] + 1],
                },
                bs,
                intra_edge_flags,
                t.by,
                t.bx,
            );

            b.inter_mode = ts.msac.decode_symbol_adapt8(
                &mut ts.cdf.m.comp_inter_mode[ctx as usize],
                N_COMP_INTER_PRED_MODES - 1,
            ) as u8;

            let im = &COMP_INTER_PRED_MODES[b.inter_mode as usize];
            b.drl_idx = NEAREST_DRL;
            if b.inter_mode == NEWMV_NEWMV {
                if n_mvs > 1 {
                    // NEARER, NEAR or NEARISH
                    let drl_ctx_v1 = get_drl_context(&mvstack, 0);
                    b.drl_idx += ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v1]) as u8;
                    if b.drl_idx == NEARER_DRL && n_mvs > 2 {
                        let drl_ctx_v2 = get_drl_context(&mvstack, 1);
                        b.drl_idx +=
                            ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v2]) as u8;
                    }
                }
            } else if im[0] == NEARMV || im[1] == NEARMV {
                b.drl_idx = NEARER_DRL;
                if n_mvs > 2 {
                    // NEAR or NEARISH
                    let drl_ctx_v2 = get_drl_context(&mvstack, 1);
                    b.drl_idx += ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v2]) as u8;
                    if b.drl_idx == NEAR_DRL && n_mvs > 3 {
                        let drl_ctx_v3 = get_drl_context(&mvstack, 2);
                        b.drl_idx +=
                            ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v3]) as u8;
                    }
                }
            }
            debug_assert!(b.drl_idx >= NEAREST_DRL && b.drl_idx <= NEARISH_DRL);

            has_subpel_filter = imin(bw4, bh4) == 1 || b.inter_mode != GLOBALMV_GLOBALMV;
            for idx in 0..2 {
                // assign_comp_mv(idx)
                match im[idx] {
                    NEARMV | NEARESTMV => {
                        b.mv[idx] = mvstack[b.drl_idx as usize].mv.mv[idx];
                        fix_mv_precision(frame_hdr, &mut b.mv[idx]);
                    }
                    GLOBALMV => {
                        has_subpel_filter |=
                            frame_hdr.gmv[b.ref_[idx] as usize].type_ == DAV1D_WM_TYPE_TRANSLATION;
                        b.mv[idx] = get_gmv_2d(
                            &frame_hdr.gmv[b.ref_[idx] as usize],
                            t.bx,
                            t.by,
                            bw4,
                            bh4,
                            frame_hdr,
                        );
                    }
                    NEWMV => {
                        b.mv[idx] = mvstack[b.drl_idx as usize].mv.mv[idx];
                        read_mv_residual(
                            ts,
                            &mut b.mv[idx],
                            false,
                            frame_hdr.force_integer_mv == 0,
                            hp,
                        );
                    }
                    _ => {}
                }
            }

            // jnt_comp vs. seg vs. wedge
            let mut is_segwedge = false;
            if seq_hdr.masked_compound != 0 {
                let mask_ctx = get_mask_comp_ctx(&f.a[ta], &t.l, by4, bx4);

                is_segwedge = ts.msac.decode_bool_adapt(&mut ts.cdf.m.mask_comp[mask_ctx]) != 0;
            }

            if !is_segwedge {
                if seq_hdr.jnt_comp != 0 {
                    let jnt_ctx = get_jnt_comp_ctx(
                        seq_hdr.order_hint_n_bits,
                        f.cur.frame_hdr.as_ref().expect("frame header").frame_offset as u32,
                        f.refp[b.ref_[0] as usize]
                            .p
                            .frame_hdr
                            .as_ref()
                            .expect("reference frame header")
                            .frame_offset as u32,
                        f.refp[b.ref_[1] as usize]
                            .p
                            .frame_hdr
                            .as_ref()
                            .expect("reference frame header")
                            .frame_offset as u32,
                        &f.a[ta],
                        &t.l,
                        by4,
                        bx4,
                    );
                    b.comp_type = COMP_INTER_WEIGHTED_AVG
                        + ts.msac.decode_bool_adapt(&mut ts.cdf.m.jnt_comp[jnt_ctx]) as u8;
                } else {
                    b.comp_type = COMP_INTER_AVG;
                }
            } else {
                if WEDGE_ALLOWED_MASK & (1 << bs) != 0 {
                    let ctx = WEDGE_CTX_LUT[bs as usize] as usize;
                    b.comp_type = COMP_INTER_WEDGE
                        - ts.msac.decode_bool_adapt(&mut ts.cdf.m.wedge_comp[ctx]) as u8;
                    if b.comp_type == COMP_INTER_WEDGE {
                        b.wedge_idx = ts
                            .msac
                            .decode_symbol_adapt16(&mut ts.cdf.m.wedge_idx[ctx], 15)
                            as u8;
                    }
                } else {
                    b.comp_type = COMP_INTER_SEG;
                }
                b.mask_sign = ts.msac.decode_bool_equi() as u8;
            }
        } else {
            b.comp_type = COMP_INTER_NONE;

            // ref
            if let Some(s) = seg.filter(|s| s.ref_ > 0) {
                b.ref_[0] = (s.ref_ - 1) as i8;
            } else if seg.is_some_and(|s| s.globalmv != 0 || s.skip != 0) {
                b.ref_[0] = 0;
            } else {
                let ctx1 = av1_get_ref_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                if ts.msac.decode_bool_adapt(&mut ts.cdf.m.ref_[0][ctx1]) != 0 {
                    // (av1_get_ref_2_ctx is av1_get_bwd_ref_ctx)
                    let ctx2 = av1_get_bwd_ref_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                    if ts.msac.decode_bool_adapt(&mut ts.cdf.m.ref_[1][ctx2]) != 0 {
                        b.ref_[0] = 6;
                    } else {
                        // (av1_get_ref_6_ctx is av1_get_bwd_ref_1_ctx)
                        let ctx3 =
                            av1_get_bwd_ref_1_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                        b.ref_[0] =
                            4 + ts.msac.decode_bool_adapt(&mut ts.cdf.m.ref_[5][ctx3]) as i8;
                    }
                } else {
                    // (av1_get_ref_3_ctx is av1_get_fwd_ref_ctx)
                    let ctx2 = av1_get_fwd_ref_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                    if ts.msac.decode_bool_adapt(&mut ts.cdf.m.ref_[2][ctx2]) != 0 {
                        // (av1_get_ref_5_ctx is av1_get_fwd_ref_2_ctx)
                        let ctx3 =
                            av1_get_fwd_ref_2_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                        b.ref_[0] =
                            2 + ts.msac.decode_bool_adapt(&mut ts.cdf.m.ref_[4][ctx3]) as i8;
                    } else {
                        // (av1_get_ref_4_ctx is av1_get_fwd_ref_1_ctx)
                        let ctx3 =
                            av1_get_fwd_ref_1_ctx(&f.a[ta], &t.l, by4, bx4, have_top, have_left);
                        b.ref_[0] = ts.msac.decode_bool_adapt(&mut ts.cdf.m.ref_[3][ctx3]) as i8;
                    }
                }
            }
            b.ref_[1] = -1;

            let mut mvstack = [RefmvsCandidate::default(); 8];
            let (n_mvs, ctx) = refmvs_find(
                &t.rt,
                &f.rf,
                &mut mvstack,
                RefmvsRefpair {
                    ref_: [b.ref_[0] + 1, -1],
                },
                bs,
                intra_edge_flags,
                t.by,
                t.bx,
            );

            // mode parsing and mv derivation from ref_mvs
            if seg.is_some_and(|s| s.skip != 0 || s.globalmv != 0)
                || ts
                    .msac
                    .decode_bool_adapt(&mut ts.cdf.m.newmv_mode[(ctx & 7) as usize])
                    != 0
            {
                if seg.is_some_and(|s| s.skip != 0 || s.globalmv != 0)
                    || ts
                        .msac
                        .decode_bool_adapt(&mut ts.cdf.m.globalmv_mode[((ctx >> 3) & 1) as usize])
                        == 0
                {
                    b.inter_mode = GLOBALMV;
                    b.mv[0] = get_gmv_2d(
                        &frame_hdr.gmv[b.ref_[0] as usize],
                        t.bx,
                        t.by,
                        bw4,
                        bh4,
                        frame_hdr,
                    );
                    has_subpel_filter = imin(bw4, bh4) == 1
                        || frame_hdr.gmv[b.ref_[0] as usize].type_ == DAV1D_WM_TYPE_TRANSLATION;
                } else {
                    has_subpel_filter = true;
                    if ts
                        .msac
                        .decode_bool_adapt(&mut ts.cdf.m.refmv_mode[((ctx >> 4) & 15) as usize])
                        != 0
                    {
                        // NEAREST, NEARER, NEAR or NEARISH
                        b.inter_mode = NEARMV;
                        b.drl_idx = NEARER_DRL;
                        if n_mvs > 2 {
                            // NEARER, NEAR or NEARISH
                            let drl_ctx_v2 = get_drl_context(&mvstack, 1);
                            b.drl_idx +=
                                ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v2]) as u8;
                            if b.drl_idx == NEAR_DRL && n_mvs > 3 {
                                // NEAR or NEARISH
                                let drl_ctx_v3 = get_drl_context(&mvstack, 2);
                                b.drl_idx +=
                                    ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v3])
                                        as u8;
                            }
                        }
                    } else {
                        b.inter_mode = NEARESTMV;
                        b.drl_idx = NEAREST_DRL;
                    }
                    debug_assert!(b.drl_idx >= NEAREST_DRL && b.drl_idx <= NEARISH_DRL);
                    b.mv[0] = mvstack[b.drl_idx as usize].mv.mv[0];
                    if b.drl_idx < NEAR_DRL {
                        fix_mv_precision(frame_hdr, &mut b.mv[0]);
                    }
                }
            } else {
                has_subpel_filter = true;
                b.inter_mode = NEWMV;
                b.drl_idx = NEAREST_DRL;
                if n_mvs > 1 {
                    // NEARER, NEAR or NEARISH
                    let drl_ctx_v1 = get_drl_context(&mvstack, 0);
                    b.drl_idx += ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v1]) as u8;
                    if b.drl_idx == NEARER_DRL && n_mvs > 2 {
                        // NEAR or NEARISH
                        let drl_ctx_v2 = get_drl_context(&mvstack, 1);
                        b.drl_idx +=
                            ts.msac.decode_bool_adapt(&mut ts.cdf.m.drl_bit[drl_ctx_v2]) as u8;
                    }
                }
                debug_assert!(b.drl_idx >= NEAREST_DRL && b.drl_idx <= NEARISH_DRL);
                if n_mvs > 1 {
                    b.mv[0] = mvstack[b.drl_idx as usize].mv.mv[0];
                } else {
                    debug_assert!(b.drl_idx == 0);
                    b.mv[0] = mvstack[0].mv.mv[0];
                    fix_mv_precision(frame_hdr, &mut b.mv[0]);
                }
                read_mv_residual(ts, &mut b.mv[0], false, frame_hdr.force_integer_mv == 0, hp);
            }

            // interintra flags
            let ii_sz_grp = YMODE_SIZE_CONTEXT[bs as usize] as usize;
            if seq_hdr.inter_intra != 0
                && INTERINTRA_ALLOWED_MASK & (1 << bs) != 0
                && ts
                    .msac
                    .decode_bool_adapt(&mut ts.cdf.m.interintra[ii_sz_grp])
                    != 0
            {
                b.interintra_mode = ts.msac.decode_symbol_adapt4(
                    &mut ts.cdf.m.interintra_mode[ii_sz_grp],
                    N_INTER_INTRA_PRED_MODES - 1,
                ) as u8;
                let wedge_ctx = WEDGE_CTX_LUT[bs as usize] as usize;
                b.interintra_type = INTER_INTRA_BLEND
                    + ts.msac
                        .decode_bool_adapt(&mut ts.cdf.m.interintra_wedge[wedge_ctx])
                        as u8;
                if b.interintra_type == INTER_INTRA_WEDGE {
                    b.wedge_idx = ts
                        .msac
                        .decode_symbol_adapt16(&mut ts.cdf.m.wedge_idx[wedge_ctx], 15)
                        as u8;
                }
            } else {
                b.interintra_type = INTER_INTRA_NONE;
            }

            // motion variation
            if frame_hdr.switchable_motion_mode != 0
                && b.interintra_type == INTER_INTRA_NONE
                && imin(bw4, bh4) >= 2
                // is not warped global motion
                && !(frame_hdr.force_integer_mv == 0
                    && b.inter_mode == GLOBALMV
                    && frame_hdr.gmv[b.ref_[0] as usize].type_ > DAV1D_WM_TYPE_TRANSLATION)
                // has overlappable neighbours
                && ((have_left && findoddzero(&t.l.intra[by4 + 1..], h4 >> 1))
                    || (have_top && findoddzero(&f.a[ta].intra[bx4 + 1..], w4 >> 1)))
            {
                // reaching here means the block allows obmc - check warp by
                // finding matching-ref blocks in top/left edges
                let mut mask = [0u64; 2];
                find_matching_ref(
                    t,
                    &f.rf,
                    ts.tiling.col_end,
                    intra_edge_flags,
                    bw4,
                    bh4,
                    w4,
                    h4,
                    have_left,
                    have_top,
                    b.ref_[0],
                    &mut mask,
                );
                let allow_warp = f.svc[b.ref_[0] as usize][0].scale == 0
                    && frame_hdr.force_integer_mv == 0
                    && frame_hdr.warp_motion != 0
                    && (mask[0] | mask[1]) != 0;

                b.motion_mode = if allow_warp {
                    ts.msac
                        .decode_symbol_adapt4(&mut ts.cdf.m.motion_mode[bs as usize], 2)
                        as u8
                } else {
                    ts.msac.decode_bool_adapt(&mut ts.cdf.m.obmc[bs as usize]) as u8
                };
                if b.motion_mode == MM_WARP {
                    has_subpel_filter = false;
                    let mut wm = t.warpmv;
                    derive_warpmv(t, &f.rf, bw4, bh4, &mask, b.mv[0], &mut wm);
                    t.warpmv = wm;
                }
            } else {
                b.motion_mode = MM_TRANSLATION;
            }
        }

        // subpel filter
        let mut filter = [0i32; 2];
        if frame_hdr.subpel_filter_mode == DAV1D_FILTER_SWITCHABLE {
            if has_subpel_filter {
                let comp = b.comp_type != COMP_INTER_NONE;
                let ctx1 = get_filter_ctx(&f.a[ta], &t.l, comp, 0, b.ref_[0], by4, bx4);
                filter[0] = ts.msac.decode_symbol_adapt4(
                    &mut ts.cdf.m.filter[0][ctx1],
                    DAV1D_N_SWITCHABLE_FILTERS as usize - 1,
                ) as i32;
                if seq_hdr.dual_filter != 0 {
                    let ctx2 = get_filter_ctx(&f.a[ta], &t.l, comp, 1, b.ref_[0], by4, bx4);
                    filter[1] = ts.msac.decode_symbol_adapt4(
                        &mut ts.cdf.m.filter[1][ctx2],
                        DAV1D_N_SWITCHABLE_FILTERS as usize - 1,
                    ) as i32;
                } else {
                    filter[1] = filter[0];
                }
            } else {
                filter = [DAV1D_FILTER_8TAP_REGULAR; 2];
            }
        } else {
            filter = [frame_hdr.subpel_filter_mode; 2];
        }
        b.filter2d = FILTER_2D[filter[1] as usize][filter[0] as usize];

        read_vartx_tree(
            t,
            &mut f.a[ta],
            ts,
            f.bw,
            f.bh,
            frame_hdr,
            layout,
            b,
            bs,
            bx4,
            by4,
        );

        // reconstruction
        recon_b_inter::<P>(f, t, ts, bs, b)?;

        if frame_hdr.loopfilter.level_y[0] != 0 || frame_hdr.loopfilter.level_y[1] != 0 {
            let is_globalmv = b.inter_mode == if is_comp { GLOBALMV_GLOBALMV } else { GLOBALMV };
            let lflvl = match ts.lflvl {
                TileLflvl::Frame => &f.lf.lvl,
                TileLflvl::Tile => &ts.lflvlmem,
            };
            let lf_lvls = &lflvl[b.seg_id as usize];
            let tx_split = [b.tx_split0 as u16, b.tx_split1];
            let mut ytx = b.max_ytx;
            let mut uvtx = b.uvtx;
            if frame_hdr.segmentation.lossless[b.seg_id as usize] != 0 {
                ytx = TX_4X4;
                uvtx = TX_4X4;
            }
            let a = &mut f.a[ta];
            let auv_luv = if has_chroma {
                Some((&mut a.tx_lpf_uv[cbx4..], &mut t.l.tx_lpf_uv[cby4..]))
            } else {
                None
            };
            create_lf_mask_inter(
                &mut f.lf.mask[t.lf_mask],
                &mut f.lf.level,
                b4_stride,
                lf_lvls,
                (b.ref_[0] + 1) as usize,
                (!is_globalmv) as usize,
                t.bx,
                t.by,
                f.w4,
                f.h4,
                b.skip != 0,
                bs,
                ytx,
                &tx_split,
                uvtx,
                layout,
                &mut a.tx_lpf_y[bx4..],
                &mut t.l.tx_lpf_y[by4..],
                auv_luv,
            );
        }

        // context updates
        if is_comp {
            splat_tworef_mv(&mut f.rf, t, bs, b, bw4, bh4);
        } else {
            splat_oneref_mv(&mut f.rf, t, bs, b, bw4, bh4);
        }

        {
            let (dir, off, n, idx) = (&mut t.l, by4, bh4 as usize, 1);
            set_ctx(&mut dir.seg_pred, off, n, seg_pred);
            set_ctx(&mut dir.skip_mode, off, n, b.skip_mode);
            set_ctx(&mut dir.intra, off, n, 0);
            set_ctx(&mut dir.skip, off, n, b.skip);
            set_ctx(&mut dir.pal_sz, off, n, 0);
            /* see aomedia bug 2183 for why this is outside if (has_chroma) */
            set_ctx(&mut t.pal_sz_uv[idx], off, n, 0);
            set_ctx(&mut dir.tx_intra, off, n, b_dim[2 + idx] as i8);
            set_ctx(&mut dir.comp_type, off, n, b.comp_type);
            set_ctx(&mut dir.filter[0], off, n, filter[0] as u8);
            set_ctx(&mut dir.filter[1], off, n, filter[1] as u8);
            set_ctx(&mut dir.mode, off, n, b.inter_mode);
            set_ctx(&mut dir.ref_[0], off, n, b.ref_[0]);
            set_ctx(&mut dir.ref_[1], off, n, b.ref_[1]);
        }
        {
            let (dir, off, n, idx) = (&mut f.a[ta], bx4, bw4 as usize, 0);
            set_ctx(&mut dir.seg_pred, off, n, seg_pred);
            set_ctx(&mut dir.skip_mode, off, n, b.skip_mode);
            set_ctx(&mut dir.intra, off, n, 0);
            set_ctx(&mut dir.skip, off, n, b.skip);
            set_ctx(&mut dir.pal_sz, off, n, 0);
            /* see aomedia bug 2183 for why this is outside if (has_chroma) */
            set_ctx(&mut t.pal_sz_uv[idx], off, n, 0);
            set_ctx(&mut dir.tx_intra, off, n, b_dim[2 + idx] as i8);
            set_ctx(&mut dir.comp_type, off, n, b.comp_type);
            set_ctx(&mut dir.filter[0], off, n, filter[0] as u8);
            set_ctx(&mut dir.filter[1], off, n, filter[1] as u8);
            set_ctx(&mut dir.mode, off, n, b.inter_mode);
            set_ctx(&mut dir.ref_[0], off, n, b.ref_[0]);
            set_ctx(&mut dir.ref_[1], off, n, b.ref_[1]);
        }

        if has_chroma {
            set_ctx(&mut t.l.uvmode, cby4, cbh4 as usize, DC_PRED);
            set_ctx(&mut f.a[ta].uvmode, cbx4, cbw4 as usize, DC_PRED);
        }
    }

    // update contexts
    if frame_hdr.segmentation.enabled != 0 && frame_hdr.segmentation.update_map != 0 {
        let segmap = Arc::make_mut(f.cur_segmap.as_mut().expect("segmentation map"));
        let mut p = t.by as usize * b4_stride + t.bx as usize;
        for _ in 0..bh4 {
            segmap[p..p + bw4 as usize].fill(b.seg_id);
            p += b4_stride;
        }
    }
    if b.skip == 0 {
        let lf_mask = &mut f.lf.mask[t.lf_mask];
        let mask: u32 = (!0u32 >> (32 - bw4)) << (bx4 & 15);
        let bx_idx = (bx4 & 16) >> 4;
        let mut y = 0;
        let mut ni = by4 >> 1;
        while y < bh4 {
            lf_mask.noskip_mask[ni][bx_idx] |= mask as u16;
            if bw4 == 32 {
                // this should be mask >> 16, but it's 0xffffffff anyway
                lf_mask.noskip_mask[ni][1] |= mask as u16;
            }
            y += 2;
            ni += 1;
        }
    }

    Ok(())
}

fn decode_sb<P: Pixel>(
    inloop_filters: u32,
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    ts: &mut Dav1dTileState,
    tree: &EdgeTree,
    bl: u8,
    node_idx: usize,
) -> Result<(), ()> {
    let hsz = 16 >> bl;
    let have_h_split = f.bw > t.bx + hsz;
    let have_v_split = f.bh > t.by + hsz;
    let node = tree.levels[bl as usize][node_idx];
    let layout = f.cur.p.layout;

    if !have_h_split && !have_v_split {
        debug_assert!(bl < BL_8X8);
        return decode_sb::<P>(
            inloop_filters,
            f,
            t,
            ts,
            tree,
            bl + 1,
            intra_edge_split(tree, bl, node_idx, 0),
        );
    }

    let bp;
    let bx8 = ((t.bx & 31) >> 1) as usize;
    let by8 = ((t.by & 31) >> 1) as usize;
    let ctx = get_partition_ctx(&f.a[t.a], &t.l, bl, by8, bx8);

    if have_h_split && have_v_split {
        let pc = &mut ts.cdf.m.partition[bl as usize][ctx];
        bp = ts
            .msac
            .decode_symbol_adapt16(pc, PARTITION_TYPE_COUNT[bl as usize] as usize)
            as u8;
        if layout == DAV1D_PIXEL_LAYOUT_I422
            && (bp == PARTITION_V
                || bp == PARTITION_V4
                || bp == PARTITION_T_LEFT_SPLIT
                || bp == PARTITION_T_RIGHT_SPLIT)
        {
            return Err(());
        }
        let b = &BLOCK_SIZES[bl as usize][bp as usize];

        macro_rules! dec_b {
            ($bs:expr, $edge:expr) => {
                decode_b::<P>(inloop_filters, f, t, ts, bl, $bs, bp, $edge)?
            };
        }
        macro_rules! dec_sb {
            ($n:expr) => {
                decode_sb::<P>(
                    inloop_filters,
                    f,
                    t,
                    ts,
                    tree,
                    bl + 1,
                    intra_edge_split(tree, bl, node_idx, $n),
                )?
            };
        }

        match bp {
            PARTITION_NONE => {
                dec_b!(b[0], node.o);
            }
            PARTITION_H => {
                dec_b!(b[0], node.h[0]);
                t.by += hsz;
                dec_b!(b[0], node.h[1]);
                t.by -= hsz;
            }
            PARTITION_V => {
                dec_b!(b[0], node.v[0]);
                t.bx += hsz;
                dec_b!(b[0], node.v[1]);
                t.bx -= hsz;
            }
            PARTITION_SPLIT => {
                if bl == BL_8X8 {
                    let tip = &node;
                    debug_assert!(hsz == 1);
                    dec_b!(BS_4X4, EDGE_ALL_TR_AND_BL);
                    let tl_filter = t.tl_4x4_filter;
                    t.bx += 1;
                    dec_b!(BS_4X4, tip.split[0]);
                    t.bx -= 1;
                    t.by += 1;
                    dec_b!(BS_4X4, tip.split[1]);
                    t.bx += 1;
                    t.tl_4x4_filter = tl_filter;
                    dec_b!(BS_4X4, tip.split[2]);
                    t.bx -= 1;
                    t.by -= 1;
                } else {
                    dec_sb!(0);
                    t.bx += hsz;
                    dec_sb!(1);
                    t.bx -= hsz;
                    t.by += hsz;
                    dec_sb!(2);
                    t.bx += hsz;
                    dec_sb!(3);
                    t.bx -= hsz;
                    t.by -= hsz;
                }
            }
            PARTITION_T_TOP_SPLIT => {
                dec_b!(b[0], EDGE_ALL_TR_AND_BL);
                t.bx += hsz;
                dec_b!(b[0], node.v[1]);
                t.bx -= hsz;
                t.by += hsz;
                dec_b!(b[1], node.h[1]);
                t.by -= hsz;
            }
            PARTITION_T_BOTTOM_SPLIT => {
                dec_b!(b[0], node.h[0]);
                t.by += hsz;
                dec_b!(b[1], node.v[0]);
                t.bx += hsz;
                dec_b!(b[1], 0);
                t.bx -= hsz;
                t.by -= hsz;
            }
            PARTITION_T_LEFT_SPLIT => {
                dec_b!(b[0], EDGE_ALL_TR_AND_BL);
                t.by += hsz;
                dec_b!(b[0], node.h[1]);
                t.by -= hsz;
                t.bx += hsz;
                dec_b!(b[1], node.v[1]);
                t.bx -= hsz;
            }
            PARTITION_T_RIGHT_SPLIT => {
                dec_b!(b[0], node.v[0]);
                t.bx += hsz;
                dec_b!(b[1], node.h[0]);
                t.by += hsz;
                dec_b!(b[1], 0);
                t.by -= hsz;
                t.bx -= hsz;
            }
            PARTITION_H4 => {
                let branch = &node;
                dec_b!(b[0], node.h[0]);
                t.by += hsz >> 1;
                dec_b!(b[0], branch.h4);
                t.by += hsz >> 1;
                dec_b!(b[0], EDGE_ALL_LEFT_HAS_BOTTOM);
                t.by += hsz >> 1;
                if t.by < f.bh {
                    dec_b!(b[0], node.h[1]);
                }
                t.by -= hsz * 3 >> 1;
            }
            PARTITION_V4 => {
                let branch = &node;
                dec_b!(b[0], node.v[0]);
                t.bx += hsz >> 1;
                dec_b!(b[0], branch.v4);
                t.bx += hsz >> 1;
                dec_b!(b[0], EDGE_ALL_TOP_HAS_RIGHT);
                t.bx += hsz >> 1;
                if t.bx < f.bw {
                    dec_b!(b[0], node.v[1]);
                }
                t.bx -= hsz * 3 >> 1;
            }
            _ => unreachable!("partition type"),
        }
    } else if have_h_split {
        let pc = &ts.cdf.m.partition[bl as usize][ctx];
        let is_split = ts.msac.decode_bool(gather_top_partition_prob(pc, bl)) != 0;

        debug_assert!(bl < BL_8X8);
        if is_split {
            bp = PARTITION_SPLIT;
            decode_sb::<P>(
                inloop_filters,
                f,
                t,
                ts,
                tree,
                bl + 1,
                intra_edge_split(tree, bl, node_idx, 0),
            )?;
            t.bx += hsz;
            decode_sb::<P>(
                inloop_filters,
                f,
                t,
                ts,
                tree,
                bl + 1,
                intra_edge_split(tree, bl, node_idx, 1),
            )?;
            t.bx -= hsz;
        } else {
            bp = PARTITION_H;
            decode_b::<P>(
                inloop_filters,
                f,
                t,
                ts,
                bl,
                BLOCK_SIZES[bl as usize][PARTITION_H as usize][0],
                PARTITION_H,
                node.h[0],
            )?;
        }
    } else {
        debug_assert!(have_v_split);
        let pc = &ts.cdf.m.partition[bl as usize][ctx];
        let is_split = ts.msac.decode_bool(gather_left_partition_prob(pc, bl)) != 0;
        if layout == DAV1D_PIXEL_LAYOUT_I422 && !is_split {
            return Err(());
        }

        debug_assert!(bl < BL_8X8);
        if is_split {
            bp = PARTITION_SPLIT;
            decode_sb::<P>(
                inloop_filters,
                f,
                t,
                ts,
                tree,
                bl + 1,
                intra_edge_split(tree, bl, node_idx, 0),
            )?;
            t.by += hsz;
            decode_sb::<P>(
                inloop_filters,
                f,
                t,
                ts,
                tree,
                bl + 1,
                intra_edge_split(tree, bl, node_idx, 2),
            )?;
            t.by -= hsz;
        } else {
            bp = PARTITION_V;
            decode_b::<P>(
                inloop_filters,
                f,
                t,
                ts,
                bl,
                BLOCK_SIZES[bl as usize][PARTITION_V as usize][0],
                PARTITION_V,
                node.v[0],
            )?;
        }
    }

    if bp != PARTITION_SPLIT || bl == BL_8X8 {
        let n = hsz as usize;
        set_ctx(
            &mut f.a[t.a].partition,
            bx8,
            n,
            AL_PART_CTX[0][bl as usize][bp as usize],
        );
        set_ctx(
            &mut t.l.partition,
            by8,
            n,
            AL_PART_CTX[1][bl as usize][bp as usize],
        );
    }

    Ok(())
}

fn reset_context(ctx: &mut BlockContext, keyframe: bool) {
    ctx.intra.fill(keyframe as u8);
    ctx.uvmode.fill(DC_PRED);
    if keyframe {
        ctx.mode.fill(DC_PRED);
    }

    ctx.partition.fill(0);
    ctx.skip.fill(0);
    ctx.skip_mode.fill(0);
    ctx.tx_lpf_y.fill(2);
    ctx.tx_lpf_uv.fill(1);
    ctx.tx_intra.fill(-1);
    ctx.tx.fill(TX_64X64 as i8);
    if !keyframe {
        for r in ctx.ref_.iter_mut() {
            r.fill(-1);
        }
        ctx.comp_type.fill(0);
        ctx.mode.fill(NEARESTMV);
    }
    ctx.lcoef.fill(0x40);
    for c in ctx.ccoef.iter_mut() {
        c.fill(0x40);
    }
    for f in ctx.filter.iter_mut() {
        f.fill(DAV1D_N_SWITCHABLE_FILTERS as u8);
    }
    ctx.seg_pred.fill(0);
    ctx.pal_sz.fill(0);
}

// (ss_size_mul[] sizes the frame threading coefficient and palette buffers)

fn setup_tile(
    ts: &mut Dav1dTileState,
    f: &mut Dav1dFrameContext,
    data: &Arc<[u8]>,
    start: usize,
    sz: usize,
    tile_row: i32,
    tile_col: i32,
) {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    let sb128 = f.seq_hdr().sb128;
    let col_sb_start = frame_hdr.tiling.col_start_sb[tile_col as usize] as i32;
    let col_sb128_start = col_sb_start >> (sb128 == 0) as i32;
    let col_sb_end = frame_hdr.tiling.col_start_sb[tile_col as usize + 1] as i32;
    let row_sb_start = frame_hdr.tiling.row_start_sb[tile_row as usize] as i32;
    let row_sb_end = frame_hdr.tiling.row_start_sb[tile_row as usize + 1] as i32;
    let sb_shift = f.sb_shift;

    cdf_thread_copy(&mut ts.cdf, &f.in_cdf);
    ts.last_qidx = frame_hdr.quant.yac;
    ts.last_delta_lf = [0; 4];

    ts.msac = MsacContext::init(data.clone(), start, sz, frame_hdr.disable_cdf_update != 0);

    ts.tiling.row = tile_row;
    ts.tiling.col = tile_col;
    ts.tiling.col_start = col_sb_start << sb_shift;
    ts.tiling.col_end = imin(col_sb_end << sb_shift, f.bw);
    ts.tiling.row_start = row_sb_start << sb_shift;
    ts.tiling.row_end = imin(row_sb_end << sb_shift, f.bh);

    // Reference Restoration Unit (used for exp coding)
    let sb_idx;
    let unit_idx;
    if frame_hdr.width[0] != frame_hdr.width[1] {
        // vertical components only
        sb_idx = (ts.tiling.row_start >> 5) * f.sr_sb128w;
        unit_idx = (ts.tiling.row_start & 16) >> 3;
    } else {
        sb_idx = (ts.tiling.row_start >> 5) * f.sb128w + col_sb128_start;
        unit_idx = ((ts.tiling.row_start & 16) >> 3) + ((ts.tiling.col_start & 16) >> 4);
    }
    for p in 0..3 {
        if (f.lf.restore_planes >> p) & 1 == 0 {
            continue;
        }

        let lr_ref;
        if frame_hdr.width[0] != frame_hdr.width[1] {
            let ss_hor = (p != 0 && f.cur.p.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
            let d = frame_hdr.super_res.width_scale_denominator;
            let unit_size_log2 = frame_hdr.restoration.unit_size[(p != 0) as usize];
            let rnd = (8 << unit_size_log2) - 1;
            let shift = unit_size_log2 + 3;
            let x = ((4 * ts.tiling.col_start * d >> ss_hor) + rnd) >> shift;
            let px_x = x << (unit_size_log2 + ss_hor);
            let u_idx = unit_idx + ((px_x & 64) >> 6);
            let sb128x = px_x >> 7;
            if sb128x >= f.sr_sb128w {
                continue;
            }
            lr_ref = ((sb_idx + sb128x) as usize, u_idx as usize);
        } else {
            lr_ref = (sb_idx as usize, unit_idx as usize);
        }
        ts.lr_ref[p] = Some(lr_ref);

        let lr = &mut f.lf.lr_mask[lr_ref.0].lr[p][lr_ref.1];
        lr.filter_v[0] = 3;
        lr.filter_v[1] = -7;
        lr.filter_v[2] = 15;
        lr.filter_h[0] = 3;
        lr.filter_h[1] = -7;
        lr.filter_h[2] = 15;
        lr.sgr_weights[0] = -32;
        lr.sgr_weights[1] = 31;
    }
}

/// `read_restoration_info()`: `lr` is the unit `f->lf.lr_mask[.0].lr[p][.1]`.
fn read_restoration_info(
    ts: &mut Dav1dTileState,
    lr_mask: &mut [Av1Restoration],
    lr: (usize, usize),
    p: usize,
    frame_type: u8,
) {
    let mut unit: Av1RestorationUnit = lr_mask[lr.0].lr[p][lr.1];
    // FIXME (upstream): ts->lr_ref[p] is left as it was (possibly from an
    // earlier tile or frame) when setup_tile() skips this plane's
    // reference unit; this reads the same stale unit then.
    let lr_ref = ts.lr_ref[p]
        .map(|r| lr_mask[r.0].lr[p][r.1])
        .unwrap_or_default();

    if frame_type == DAV1D_RESTORATION_SWITCHABLE {
        let filter = ts
            .msac
            .decode_symbol_adapt4(&mut ts.cdf.m.restore_switchable, 2);
        unit.type_ = if filter != 0 {
            if filter == 2 {
                DAV1D_RESTORATION_SGRPROJ
            } else {
                DAV1D_RESTORATION_WIENER
            }
        } else {
            DAV1D_RESTORATION_NONE
        };
    } else {
        let type_ = ts
            .msac
            .decode_bool_adapt(if frame_type == DAV1D_RESTORATION_WIENER {
                &mut ts.cdf.m.restore_wiener
            } else {
                &mut ts.cdf.m.restore_sgrproj
            });
        unit.type_ = if type_ != 0 {
            frame_type
        } else {
            DAV1D_RESTORATION_NONE
        };
    }

    if unit.type_ == DAV1D_RESTORATION_WIENER {
        unit.filter_v[0] = if p != 0 {
            0
        } else {
            (ts.msac.decode_subexp(lr_ref.filter_v[0] as i32 + 5, 16, 1) - 5) as i8
        };
        unit.filter_v[1] =
            (ts.msac.decode_subexp(lr_ref.filter_v[1] as i32 + 23, 32, 2) - 23) as i8;
        unit.filter_v[2] =
            (ts.msac.decode_subexp(lr_ref.filter_v[2] as i32 + 17, 64, 3) - 17) as i8;

        unit.filter_h[0] = if p != 0 {
            0
        } else {
            (ts.msac.decode_subexp(lr_ref.filter_h[0] as i32 + 5, 16, 1) - 5) as i8
        };
        unit.filter_h[1] =
            (ts.msac.decode_subexp(lr_ref.filter_h[1] as i32 + 23, 32, 2) - 23) as i8;
        unit.filter_h[2] =
            (ts.msac.decode_subexp(lr_ref.filter_h[2] as i32 + 17, 64, 3) - 17) as i8;
        unit.sgr_weights = lr_ref.sgr_weights;
        ts.lr_ref[p] = Some(lr);
    } else if unit.type_ == DAV1D_RESTORATION_SGRPROJ {
        let idx = ts.msac.decode_bools(4);
        let sgr_params = &SGR_PARAMS[idx as usize];
        unit.sgr_idx = idx as u8;
        unit.sgr_weights[0] = if sgr_params[0] != 0 {
            (ts.msac
                .decode_subexp(lr_ref.sgr_weights[0] as i32 + 96, 128, 4)
                - 96) as i8
        } else {
            0
        };
        unit.sgr_weights[1] = if sgr_params[1] != 0 {
            (ts.msac
                .decode_subexp(lr_ref.sgr_weights[1] as i32 + 32, 128, 4)
                - 32) as i8
        } else {
            95
        };
        unit.filter_v = lr_ref.filter_v;
        unit.filter_h = lr_ref.filter_h;
        ts.lr_ref[p] = Some(lr);
    }
    lr_mask[lr.0].lr[p][lr.1] = unit;
}

/// `dav1d_decode_tile_sbrow()`
fn decode_tile_sbrow<P: Pixel>(
    inloop_filters: u32,
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
    ts: &mut Dav1dTileState,
) -> Result<(), ()> {
    let seq_hdr = f.seq_hdr.clone().expect("sequence header");
    let frame_hdr = f.frame_hdr.clone().expect("frame header");
    let root_bl = if seq_hdr.sb128 != 0 {
        BL_128X128
    } else {
        BL_64X64
    };
    let sb_step = f.sb_step;
    let tile_row = ts.tiling.row;
    let tile_col = ts.tiling.col;
    let col_sb_start = frame_hdr.tiling.col_start_sb[tile_col as usize] as i32;
    let col_sb128_start = col_sb_start >> (seq_hdr.sb128 == 0) as i32;

    if is_inter_or_switch(&frame_hdr) || frame_hdr.allow_intrabc != 0 {
        refmvs_tile_sbrow_init(
            &mut t.rt,
            &f.rf,
            ts.tiling.col_start,
            ts.tiling.col_end,
            ts.tiling.row_start,
            ts.tiling.row_end,
            t.by >> f.sb_shift,
        );
    }

    reset_context(&mut t.l, is_key_or_intra(&frame_hdr));

    // error out on symbol decoder overread
    if ts.msac.cnt < -15 {
        return Err(());
    }

    t.pal_sz_uv[1] = [0; 32];
    let sb128y = t.by >> 5;
    let tree = intra_edge_tree(root_bl);
    t.bx = ts.tiling.col_start;
    t.a = (col_sb128_start + tile_row * f.sb128w) as usize;
    t.lf_mask = (sb128y * f.sb128w + col_sb128_start) as usize;
    while t.bx < ts.tiling.col_end {
        if root_bl == BL_128X128 {
            t.cur_sb_cdef_idx = (t.lf_mask, 0);
            f.lf.mask[t.lf_mask].cdef_idx = [-1; 4];
        } else {
            let off = (((t.bx & 16) >> 4) + ((t.by & 16) >> 3)) as usize;
            t.cur_sb_cdef_idx = (t.lf_mask, off);
            f.lf.mask[t.lf_mask].cdef_idx[off] = -1;
        }
        // Restoration filter
        for p in 0..3 {
            if (f.lf.restore_planes >> p) & 1 == 0 {
                continue;
            }

            let ss_ver = (p != 0 && f.cur.p.layout == DAV1D_PIXEL_LAYOUT_I420) as i32;
            let ss_hor = (p != 0 && f.cur.p.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
            let unit_size_log2 = frame_hdr.restoration.unit_size[(p != 0) as usize];
            let y = t.by * 4 >> ss_ver;
            let h = (f.cur.p.h + ss_ver) >> ss_ver;

            let unit_size = 1 << unit_size_log2;
            let mask = (unit_size - 1) as u32;
            if y as u32 & mask != 0 {
                continue;
            }
            let half_unit = unit_size >> 1;
            // Round half up at frame boundaries, if there's more than one
            // restoration unit
            if y != 0 && y + half_unit > h {
                continue;
            }

            let frame_type = frame_hdr.restoration.type_[p];

            if frame_hdr.width[0] != frame_hdr.width[1] {
                let w = (f.sr_cur.p.p.w + ss_hor) >> ss_hor;
                let n_units = imax(1, (w + half_unit) >> unit_size_log2);

                let d = frame_hdr.super_res.width_scale_denominator;
                let rnd = unit_size * 8 - 1;
                let shift = unit_size_log2 + 3;
                let x0 = ((4 * t.bx * d >> ss_hor) + rnd) >> shift;
                let x1 = ((4 * (t.bx + sb_step) * d >> ss_hor) + rnd) >> shift;

                for x in x0..imin(x1, n_units) {
                    let px_x = x << (unit_size_log2 + ss_hor);
                    let sb_idx = (t.by >> 5) * f.sr_sb128w + (px_x >> 7);
                    let unit_idx = ((t.by & 16) >> 3) + ((px_x & 64) >> 6);

                    read_restoration_info(
                        ts,
                        &mut f.lf.lr_mask,
                        (sb_idx as usize, unit_idx as usize),
                        p,
                        frame_type,
                    );
                }
            } else {
                let x = 4 * t.bx >> ss_hor;
                if x as u32 & mask != 0 {
                    continue;
                }
                let w = (f.cur.p.w + ss_hor) >> ss_hor;
                // Round half up at frame boundaries, if there's more than one
                // restoration unit
                if x != 0 && x + half_unit > w {
                    continue;
                }
                let sb_idx = (t.by >> 5) * f.sr_sb128w + (t.bx >> 5);
                let unit_idx = ((t.by & 16) >> 3) + ((t.bx & 16) >> 4);

                read_restoration_info(
                    ts,
                    &mut f.lf.lr_mask,
                    (sb_idx as usize, unit_idx as usize),
                    p,
                    frame_type,
                );
            }
        }
        decode_sb::<P>(inloop_filters, f, t, ts, tree, root_bl, 0)?;
        if t.bx & 16 != 0 || seq_hdr.sb128 != 0 {
            t.a += 1;
            t.lf_mask += 1;
        }
        t.bx += sb_step;
    }

    // backup pre-loopfilter pixels for intra prediction of the next sbrow
    backup_ipred_edge::<P>(f, t, ts);

    // backup t->a/l.tx_lpf_y/uv at tile boundaries to use them to "fix"
    // up the initial value in neighbour tiles when running the loopfilter
    let mut align_h = ((f.bh + 31) & !31) as usize;
    let sb_step = sb_step as usize;
    let by = t.by as usize;
    let o = align_h * tile_col as usize + by;
    f.lf.tx_lpf_right_edge[0][o..o + sb_step]
        .copy_from_slice(&t.l.tx_lpf_y[by & 16..(by & 16) + sb_step]);
    let ss_ver = (f.cur.p.layout == DAV1D_PIXEL_LAYOUT_I420) as usize;
    align_h >>= ss_ver;
    let o = align_h * tile_col as usize + (by >> ss_ver);
    let n = sb_step >> ss_ver;
    f.lf.tx_lpf_right_edge[1][o..o + n]
        .copy_from_slice(&t.l.tx_lpf_uv[(by & 16) >> ss_ver..((by & 16) >> ss_ver) + n]);

    Ok(())
}

/// Reallocates `v` with `n` elements of `val` (failing like `malloc()`
/// when the size is too large).
fn realloc_vec<T: Clone>(v: &mut Vec<T>, val: T, n: usize) -> Result<(), i32> {
    *v = Vec::new();
    *v = try_vec(val, n).map_err(|_| dav1d_err(ENOMEM))?;
    Ok(())
}

/// `dav1d_decode_frame_init()`
fn decode_frame_init<P: Pixel>(f: &mut Dav1dFrameContext) -> Result<(), i32> {
    let seq_hdr = f.seq_hdr.clone().expect("sequence header");
    let frame_hdr = f.frame_hdr.clone().expect("frame header");

    if f.sbh as usize > f.lf.start_of_tile_row.len() {
        realloc_vec(&mut f.lf.start_of_tile_row, 0, f.sbh as usize)?;
    }
    let mut sby = 0;
    for tile_row in 0..frame_hdr.tiling.rows as usize {
        f.lf.start_of_tile_row[sby] = tile_row as u8;
        sby += 1;
        while sby < frame_hdr.tiling.row_start_sb[tile_row + 1] as usize {
            f.lf.start_of_tile_row[sby] = 0;
            sby += 1;
        }
    }

    let n_ts = frame_hdr.tiling.cols * frame_hdr.tiling.rows;
    if n_ts != f.n_ts {
        f.ts = Vec::new();
        let mut ts = Vec::new();
        ts.try_reserve_exact(n_ts as usize)
            .map_err(|_| dav1d_err(ENOMEM))?;
        ts.resize_with(n_ts as usize, Dav1dTileState::default);
        f.ts = ts;
        f.n_ts = n_ts;
    }

    let a_sz = f.sb128w * frame_hdr.tiling.rows;
    if a_sz != f.a_sz {
        if let Err(e) = realloc_vec(&mut f.a, BlockContext::default(), a_sz as usize) {
            f.a_sz = 0;
            return Err(e);
        }
        f.a_sz = a_sz;
    }

    let num_sb128 = (f.sb128w * f.sb128h) as usize;
    let hbd = seq_hdr.hbd != 0;

    // update allocation of block contexts for above
    let y_stride = f.cur_px.as_ref().expect("picture").stride[0];
    let uv_stride = f.cur_px.as_ref().expect("picture").stride[1];
    let has_resize = frame_hdr.width[0] != frame_hdr.width[1];
    let px_bytes = if hbd { 2 } else { 1 };
    if (y_stride * px_bytes) as isize * f.sbh as isize * 4 != f.lf.cdef_buf_plane_sz[0]
        || (uv_stride * px_bytes) as isize * f.sbh as isize * 8 != f.lf.cdef_buf_plane_sz[1]
        || f.sbh != f.lf.cdef_buf_sbh
    {
        let fpx = P::frame_px_mut(&mut f.px);
        for tf in 0..2 {
            let r = realloc_vec(&mut fpx.cdef_line[tf][0], P::default(), 2 * y_stride)
                .and_then(|_| realloc_vec(&mut fpx.cdef_line[tf][1], P::default(), 2 * uv_stride))
                .and_then(|_| realloc_vec(&mut fpx.cdef_line[tf][2], P::default(), 2 * uv_stride));
            if let Err(e) = r {
                f.lf.cdef_buf_plane_sz = [0, 0];
                return Err(e);
            }
        }

        f.lf.cdef_buf_plane_sz[0] = (y_stride * px_bytes) as isize * f.sbh as isize * 4;
        f.lf.cdef_buf_plane_sz[1] = (uv_stride * px_bytes) as isize * f.sbh as isize * 8;
        f.lf.cdef_buf_sbh = f.sbh;
    }
    let _ = has_resize;

    let num_lines = 12;
    let sr_px = f.sr_px.as_ref().or(f.cur_px.as_ref()).expect("picture");
    let y_stride = sr_px.stride[0];
    let uv_stride = sr_px.stride[1];
    if (y_stride * px_bytes * num_lines) as isize != f.lf.lr_buf_plane_sz[0]
        || (uv_stride * px_bytes * num_lines * 2) as isize != f.lf.lr_buf_plane_sz[1]
    {
        let fpx = P::frame_px_mut(&mut f.px);
        let r = realloc_vec(&mut fpx.lr_lpf_line[0], P::default(), y_stride * num_lines)
            .and_then(|_| realloc_vec(&mut fpx.lr_lpf_line[1], P::default(), uv_stride * num_lines))
            .and_then(|_| {
                realloc_vec(&mut fpx.lr_lpf_line[2], P::default(), uv_stride * num_lines)
            });
        if let Err(e) = r {
            f.lf.lr_buf_plane_sz = [0, 0];
            return Err(e);
        }

        f.lf.lr_buf_plane_sz[0] = (y_stride * px_bytes * num_lines) as isize;
        f.lf.lr_buf_plane_sz[1] = (uv_stride * px_bytes * num_lines * 2) as isize;
    }

    // update allocation for loopfilter masks
    if num_sb128 as i32 != f.lf.mask_sz {
        let r = realloc_vec(&mut f.lf.mask, Av1Filter::default(), num_sb128)
            .and_then(|_| realloc_vec(&mut f.lf.level, [0u8; 4], num_sb128 * 32 * 32));
        if let Err(e) = r {
            f.lf.mask_sz = 0;
            return Err(e);
        }
        f.lf.mask_sz = num_sb128 as i32;
    }

    f.sr_sb128w = (f.sr_cur.p.p.w + 127) >> 7;
    let lr_mask_sz = f.sr_sb128w * f.sb128h;
    if lr_mask_sz != f.lf.lr_mask_sz {
        if let Err(e) = realloc_vec(
            &mut f.lf.lr_mask,
            Av1Restoration::default(),
            lr_mask_sz as usize,
        ) {
            f.lf.lr_mask_sz = 0;
            return Err(e);
        }
        f.lf.lr_mask_sz = lr_mask_sz;
    }
    f.lf.restore_planes = ((frame_hdr.restoration.type_[0] != DAV1D_RESTORATION_NONE) as i32)
        + (((frame_hdr.restoration.type_[1] != DAV1D_RESTORATION_NONE) as i32) << 1)
        + (((frame_hdr.restoration.type_[2] != DAV1D_RESTORATION_NONE) as i32) << 2);
    if frame_hdr.loopfilter.sharpness != f.lf.last_sharpness {
        calc_eih(&mut f.lf.lim_lut, frame_hdr.loopfilter.sharpness);
        f.lf.last_sharpness = frame_hdr.loopfilter.sharpness;
    }
    calc_lf_values(&mut f.lf.lvl, &frame_hdr, &[0, 0, 0, 0]);
    f.lf.mask.fill(Av1Filter::default());

    let ipred_edge_sz = f.sbh * f.sb128w << hbd as i32;
    if ipred_edge_sz != f.ipred_edge_sz {
        let n = (f.sbh * f.sb128w * 128) as usize;
        let fpx = P::frame_px_mut(&mut f.px);
        let r = realloc_vec(&mut fpx.ipred_edge[0], P::default(), n)
            .and_then(|_| realloc_vec(&mut fpx.ipred_edge[1], P::default(), n))
            .and_then(|_| realloc_vec(&mut fpx.ipred_edge[2], P::default(), n));
        if let Err(e) = r {
            f.ipred_edge_sz = 0;
            return Err(e);
        }
        f.ipred_edge_sz = ipred_edge_sz;
    }

    let re_sz = f.sb128h * frame_hdr.tiling.cols;
    if re_sz != f.lf.re_sz {
        let r = realloc_vec(&mut f.lf.tx_lpf_right_edge[0], 0, re_sz as usize * 32)
            .and_then(|_| realloc_vec(&mut f.lf.tx_lpf_right_edge[1], 0, re_sz as usize * 32));
        if let Err(e) = r {
            f.lf.re_sz = 0;
            return Err(e);
        }
        f.lf.re_sz = re_sz;
    }

    // init ref mvs
    if is_inter_or_switch(&frame_hdr) || frame_hdr.allow_intrabc != 0 {
        refmvs_init_frame(&mut f.rf, &seq_hdr, &frame_hdr, &f.refpoc, &f.refrefpoc)
            .map_err(|_| dav1d_err(ENOMEM))?;
    }

    // setup dequant tables
    init_quant_tables(&seq_hdr, &frame_hdr, frame_hdr.quant.yac, &mut f.dq);
    if frame_hdr.quant.qm != 0 {
        for i in 0..N_RECT_TX_SIZES {
            f.qm[i][0] = qm_tbl(frame_hdr.quant.qm_y as usize, 0, i as u8);
            f.qm[i][1] = qm_tbl(frame_hdr.quant.qm_u as usize, 1, i as u8);
            f.qm[i][2] = qm_tbl(frame_hdr.quant.qm_v as usize, 1, i as u8);
        }
    } else {
        f.qm = [[None; 3]; N_RECT_TX_SIZES];
    }

    // setup jnt_comp weights
    if frame_hdr.switchable_comp_refs != 0 {
        let cur_poc = f.cur.frame_hdr.as_ref().expect("frame header").frame_offset;
        for i in 0..7 {
            let ref0poc = f.refp[i]
                .p
                .frame_hdr
                .as_ref()
                .expect("reference")
                .frame_offset;

            for j in i + 1..7 {
                let ref1poc = f.refp[j]
                    .p
                    .frame_hdr
                    .as_ref()
                    .expect("reference")
                    .frame_offset;

                let d1 = imin(
                    get_poc_diff(seq_hdr.order_hint_n_bits, ref0poc, cur_poc).abs(),
                    31,
                );
                let d0 = imin(
                    get_poc_diff(seq_hdr.order_hint_n_bits, ref1poc, cur_poc).abs(),
                    31,
                );
                let order = (d0 <= d1) as usize;

                const QUANT_DIST_WEIGHT: [[u8; 2]; 3] = [[2, 3], [2, 5], [2, 7]];
                const QUANT_DIST_LOOKUP_TABLE: [[u8; 2]; 4] = [[9, 7], [11, 5], [12, 4], [13, 3]];

                let mut k = 0;
                while k < 3 {
                    let c0 = QUANT_DIST_WEIGHT[k][order] as i32;
                    let c1 = QUANT_DIST_WEIGHT[k][1 - order] as i32;
                    let d0_c0 = d0 * c0;
                    let d1_c1 = d1 * c1;
                    if (d0 > d1 && d0_c0 < d1_c1) || (d0 <= d1 && d0_c0 > d1_c1) {
                        break;
                    }
                    k += 1;
                }

                f.jnt_weights[i][j] = QUANT_DIST_LOOKUP_TABLE[k][order];
            }
        }
    }

    // (the loopfilter pointers f->lf.p[] and f->lf.sr_p[] are the
    // pictures' planes)

    Ok(())
}

/// `dav1d_decode_frame_init_cdf()`
fn decode_frame_init_cdf(f: &mut Dav1dFrameContext) -> Result<(), i32> {
    let frame_hdr = f.frame_hdr.clone().expect("frame header");

    if frame_hdr.refresh_context != 0 {
        let in_cdf = f.in_cdf.clone();
        cdf_thread_copy(f.out_cdf.as_mut().expect("output CDFs"), &in_cdf);
    }

    // parse individual tiles per tile group
    let mut tile_row = 0;
    let mut tile_col = 0;
    f.update_set = false;
    let tiles = std::mem::take(&mut f.tile);
    let mut tss = std::mem::take(&mut f.ts);
    let mut res = Ok(());
    'groups: for tg in tiles.iter().take(f.n_tile_data as usize) {
        let buf = tg.data.buf.clone().expect("tile data");
        let mut data = tg.data.offset;
        let mut size = tg.data.sz;

        for j in tg.start..=tg.end {
            let tile_sz;
            if j == tg.end {
                tile_sz = size;
            } else {
                if frame_hdr.tiling.n_bytes as usize > size {
                    res = Err(dav1d_err(EINVAL));
                    break 'groups;
                }
                let mut sz = 0usize;
                for k in 0..frame_hdr.tiling.n_bytes {
                    sz |= (buf[data] as usize) << (k * 8);
                    data += 1;
                }
                tile_sz = sz + 1;
                size -= frame_hdr.tiling.n_bytes as usize;
                if tile_sz > size {
                    res = Err(dav1d_err(EINVAL));
                    break 'groups;
                }
            }

            setup_tile(
                &mut tss[j as usize],
                f,
                &buf,
                data,
                tile_sz,
                tile_row,
                tile_col,
            );
            tile_col += 1;

            if tile_col == frame_hdr.tiling.cols {
                tile_col = 0;
                tile_row += 1;
            }
            if j == frame_hdr.tiling.update && frame_hdr.refresh_context != 0 {
                f.update_set = true;
            }
            data += tile_sz;
            size -= tile_sz;
        }
    }
    f.tile = tiles;
    f.ts = tss;

    res
}

/// `dav1d_decode_frame_main()`
fn decode_frame_main<P: Pixel>(
    inloop_filters: u32,
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
) -> Result<(), i32> {
    let seq_hdr = f.seq_hdr.clone().expect("sequence header");
    let frame_hdr = f.frame_hdr.clone().expect("frame header");

    P::task_px(&mut t.px).ensure();

    for n in 0..(f.sb128w * frame_hdr.tiling.rows) as usize {
        reset_context(&mut f.a[n], is_key_or_intra(&frame_hdr));
    }

    // no threading - we explicitly interleave tile/sbrow decoding
    // and post-filtering, so that the full process runs in-line
    let mut tss = std::mem::take(&mut f.ts);
    let mut res = Ok(());
    'rows: for tile_row in 0..frame_hdr.tiling.rows {
        let sbh_end = imin(
            frame_hdr.tiling.row_start_sb[tile_row as usize + 1] as i32,
            f.sbh,
        );
        for sby in frame_hdr.tiling.row_start_sb[tile_row as usize] as i32..sbh_end {
            t.by = sby << (4 + seq_hdr.sb128);
            let by_end = (t.by + f.sb_step) >> 1;
            if frame_hdr.use_ref_frame_mvs != 0 {
                load_tmvs(&mut f.rf, 0, f.bw >> 1, t.by >> 1, by_end);
            }
            for tile_col in 0..frame_hdr.tiling.cols {
                let ts = &mut tss[(tile_row * frame_hdr.tiling.cols + tile_col) as usize];
                if decode_tile_sbrow::<P>(inloop_filters, f, t, ts).is_err() {
                    res = Err(dav1d_err(EINVAL));
                    break 'rows;
                }
            }
            if is_inter_or_switch(&frame_hdr) {
                save_tmvs(&t.rt, &mut f.rf, 0, f.bw >> 1, t.by >> 1, by_end);
            }

            // loopfilter + cdef + restoration
            filter_sbrow::<P>(inloop_filters, f, t, sby);
        }
    }
    f.ts = tss;

    res
}

/// `dav1d_decode_frame_exit()`: drops the frame's references; the
/// decoded picture, segmentation map, motion vectors and CDFs stay with
/// the frame context until `submit_frame()` hands them out.
fn decode_frame_exit(f: &mut Dav1dFrameContext) {
    for i in 0..7 {
        f.refp[i].unref();
        f.rf.rp_ref[i] = None;
    }

    f.in_cdf = CdfThreadContext::default();
    f.prev_segmap = None;

    for i in 0..f.n_tile_data as usize {
        f.tile[i].data.unref();
    }
}

/// `dav1d_decode_frame()`
fn decode_frame(
    inloop_filters: u32,
    f: &mut Dav1dFrameContext,
    t: &mut Dav1dTaskContext,
) -> Result<(), i32> {
    let hbd = f.seq_hdr().hbd != 0;
    let mut res = if hbd {
        decode_frame_init::<u16>(f)
    } else {
        decode_frame_init::<u8>(f)
    };
    if res.is_ok() {
        res = decode_frame_init_cdf(f);
    }
    if res.is_ok() {
        res = if hbd {
            decode_frame_main::<u16>(inloop_filters, f, t)
        } else {
            decode_frame_main::<u8>(inloop_filters, f, t)
        };
        let frame_hdr = f.frame_hdr.clone().expect("frame header");
        if res.is_ok() && frame_hdr.refresh_context != 0 && f.update_set {
            let update = frame_hdr.tiling.update as usize;
            let (out, ts) = (f.out_cdf.as_mut().expect("output CDFs"), &f.ts[update]);
            cdf_thread_update(&frame_hdr, out, &ts.cdf);
        }
    }
    decode_frame_exit(f);
    f.n_tile_data = 0;
    res
}

/// Computes the shear parameters of the global motion vectors that
/// `dav1d_get_shear_params()` stores into `f->frame_hdr->gmv[]` while
/// checking `gmv_warp_allowed[]` in `dav1d_submit_frame()`.
fn compute_gmv_shear(hdr: &Dav1dFrameHeader) -> Dav1dFrameHeader {
    let mut h = hdr.clone();
    if is_inter_or_switch(hdr) {
        for i in 0..7 {
            if h.gmv[i].type_ > DAV1D_WM_TYPE_TRANSLATION && h.force_integer_mv == 0 {
                get_shear_params(&mut h.gmv[i]);
            }
        }
    }
    h
}

fn get_upscale_x0(in_w: i32, out_w: i32, step: i32) -> i32 {
    let err = out_w * step - (in_w << 14);
    let x0 = (-((out_w - in_w) << 13) + (out_w >> 1)) / out_w + 128 - (err / 2);
    x0 & 0x3fff
}

/// `scale_fac(ref_sz, this_sz)`
#[inline]
fn scale_fac(ref_sz: i32, this_sz: i32) -> i32 {
    ((ref_sz << 14) + (this_sz >> 1)) / this_sz
}

/// `dav1d_submit_frame()`
pub(crate) fn submit_frame(c: &mut Dav1dContext) -> Result<(), i32> {
    let res = submit_frame_inner(c);
    if res.is_err() {
        // error:
        let f = &mut *c.fc;
        f.in_cdf = CdfThreadContext::default();
        f.out_cdf = None;
        for i in 0..7 {
            f.refp[i].unref();
            f.rf.rp_ref[i] = None;
        }
        c.out.unref();
        f.cur = Dav1dPicture::default();
        f.sr_cur.unref();
        f.cur_px = None;
        f.sr_px = None;
        f.cur_segmap = None;
        f.prev_segmap = None;
        f.rf.rp = Vec::new();
        f.seq_hdr = None;
        f.frame_hdr = None;
        c.cached_error_props = c.in_.m.clone();

        for i in 0..f.n_tile_data as usize {
            f.tile[i].data.unref();
        }
        f.n_tile_data = 0;
    }
    res
}

fn submit_frame_inner(c: &mut Dav1dContext) -> Result<(), i32> {
    let f = &mut *c.fc;

    f.seq_hdr = c.seq_hdr.clone();
    f.frame_hdr = c.frame_hdr.take();
    let seq_hdr = f.seq_hdr.clone().expect("sequence header");
    let frame_hdr = f.frame_hdr.clone().expect("frame header");

    let bpc = 8 + 2 * seq_hdr.hbd;

    let mut ref_coded_width = [0i32; 7];
    if is_inter_or_switch(&frame_hdr) {
        if frame_hdr.primary_ref_frame != DAV1D_PRIMARY_REF_NONE {
            let pri_ref = frame_hdr.refidx[frame_hdr.primary_ref_frame as usize] as usize;
            if c.refs[pri_ref].p.p.data.is_none() {
                return Err(dav1d_err(EINVAL));
            }
        }
        for i in 0..7 {
            let refidx = frame_hdr.refidx[i] as usize;
            let rp = &c.refs[refidx].p.p;
            if rp.data.is_none()
                || frame_hdr.width[0] * 2 < rp.p.w
                || frame_hdr.height * 2 < rp.p.h
                || frame_hdr.width[0] > rp.p.w * 16
                || frame_hdr.height > rp.p.h * 16
                || seq_hdr.layout != rp.p.layout
                || bpc != rp.p.bpc
            {
                for j in 0..i {
                    f.refp[j].unref();
                }
                return Err(dav1d_err(EINVAL));
            }
            f.refp[i] = c.refs[refidx].p.clone();
            ref_coded_width[i] = rp.frame_hdr.as_ref().expect("reference").width[0];
            if frame_hdr.width[0] != rp.p.w || frame_hdr.height != rp.p.h {
                f.svc[i][0].scale = scale_fac(rp.p.w, frame_hdr.width[0]);
                f.svc[i][1].scale = scale_fac(rp.p.h, frame_hdr.height);
                f.svc[i][0].step = (f.svc[i][0].scale + 8) >> 4;
                f.svc[i][1].step = (f.svc[i][1].scale + 8) >> 4;
            } else {
                f.svc[i][0].scale = 0;
                f.svc[i][1].scale = 0;
            }
            let mut gmv = frame_hdr.gmv[i];
            f.gmv_warp_allowed[i] = (frame_hdr.gmv[i].type_ > DAV1D_WM_TYPE_TRANSLATION
                && frame_hdr.force_integer_mv == 0
                && !get_shear_params(&mut gmv)
                && f.svc[i][0].scale == 0) as u8;
        }
    }
    // Note (upstream): dav1d_get_shear_params() above also stores the
    // shear parameters into f->frame_hdr->gmv[i]; the frame header here is
    // shared and immutable, and recon recomputes them on a copy (see
    // recon.rs), which yields the same values.
    let frame_hdr = Arc::new(compute_gmv_shear(&frame_hdr));
    f.frame_hdr = Some(frame_hdr.clone());

    // setup entropy
    if frame_hdr.primary_ref_frame == DAV1D_PRIMARY_REF_NONE {
        f.in_cdf = cdf_thread_init_static(frame_hdr.quant.yac);
    } else {
        let pri_ref = frame_hdr.refidx[frame_hdr.primary_ref_frame as usize] as usize;
        f.in_cdf = c.cdf[pri_ref].clone();
    }
    if frame_hdr.refresh_context != 0 {
        f.out_cdf = Some(super::cdf::cdf_context_new());
    }

    // FIXME qsort so tiles are in order (for frame threading)
    f.tile = std::mem::take(&mut c.tile);
    f.n_tile_data = c.n_tile_data;
    c.n_tile_data = 0;

    // allocate frame
    let (mut sr_cur, sr_data) = picture_alloc_with_edges(
        frame_hdr.width[1],
        frame_hdr.height,
        &f.seq_hdr,
        &f.frame_hdr,
        bpc,
    )
    .map_err(|_| dav1d_err(ENOMEM))?;
    // (dav1d_thread_picture_alloc())
    let props = f.tile[0].data.m.clone();
    sr_cur.copy_props(&c.content_light, &c.mastering_display, &c.itut_t35, &props);

    // Must be removed from the context after being attached to the frame
    c.itut_t35 = None;

    // Don't clear these flags from c->frame_flags if the frame is not visible.
    // This way they will be added to the next visible frame too.
    let flags_mask = if frame_hdr.show_frame != 0 || c.output_invisible_frames {
        0
    } else {
        PICTURE_FLAG_NEW_SEQUENCE | PICTURE_FLAG_NEW_OP_PARAMS_INFO
    };
    f.sr_cur = Dav1dThreadPicture {
        p: sr_cur,
        visible: frame_hdr.show_frame,
        showable: frame_hdr.showable_frame,
        flags: c.frame_flags,
    };
    c.frame_flags &= flags_mask;

    if frame_hdr.width[0] != frame_hdr.width[1] {
        let (cur, cur_data) =
            picture_alloc_copy(frame_hdr.width[0], &f.sr_cur.p).map_err(|_| dav1d_err(ENOMEM))?;
        f.cur = cur;
        f.cur_px = Some(cur_data);
        f.sr_px = Some(sr_data);
    } else {
        f.cur = f.sr_cur.p.clone();
        f.cur_px = Some(sr_data);
        f.sr_px = None;
    }

    if frame_hdr.width[0] != frame_hdr.width[1] {
        f.resize_step[0] = scale_fac(f.cur.p.w, f.sr_cur.p.p.w);
        let ss_hor = (f.cur.p.layout != DAV1D_PIXEL_LAYOUT_I444) as i32;
        let in_cw = (f.cur.p.w + ss_hor) >> ss_hor;
        let out_cw = (f.sr_cur.p.p.w + ss_hor) >> ss_hor;
        f.resize_step[1] = scale_fac(in_cw, out_cw);
        f.resize_start[0] = get_upscale_x0(f.cur.p.w, f.sr_cur.p.p.w, f.resize_step[0]);
        f.resize_start[1] = get_upscale_x0(in_cw, out_cw, f.resize_step[1]);
    }

    // (the frame is moved into the output queue after it decoded)

    f.w4 = (frame_hdr.width[0] + 3) >> 2;
    f.h4 = (frame_hdr.height + 3) >> 2;
    f.bw = ((frame_hdr.width[0] + 7) >> 3) << 1;
    f.bh = ((frame_hdr.height + 7) >> 3) << 1;
    f.sb128w = (f.bw + 31) >> 5;
    f.sb128h = (f.bh + 31) >> 5;
    f.sb_shift = 4 + seq_hdr.sb128;
    f.sb_step = 16 << seq_hdr.sb128;
    f.sbh = (f.bh + f.sb_step - 1) >> f.sb_shift;
    f.b4_stride = ((f.bw + 31) & !31) as usize;
    f.bitdepth_max = (1 << f.cur.p.bpc) - 1;

    // ref_mvs
    let mut has_mvs = false;
    if is_inter_or_switch(&frame_hdr) || frame_hdr.allow_intrabc != 0 {
        f.rf.rp = Vec::new();
        f.rf.rp = try_vec(
            RefmvsTemporalBlock::default(),
            f.sb128h as usize * 16 * (f.b4_stride >> 1),
        )
        .map_err(|_| dav1d_err(ENOMEM))?;
        has_mvs = true;
        if frame_hdr.allow_intrabc == 0 {
            for i in 0..7 {
                f.refpoc[i] = f.refp[i]
                    .p
                    .frame_hdr
                    .as_ref()
                    .expect("reference")
                    .frame_offset as u32;
            }
        } else {
            f.refpoc = [0; 7];
        }
        if frame_hdr.use_ref_frame_mvs != 0 {
            for i in 0..7 {
                let refidx = frame_hdr.refidx[i] as usize;
                let ref_w = ((ref_coded_width[i] + 7) >> 3) << 1;
                let ref_h = ((f.refp[i].p.p.h + 7) >> 3) << 1;
                if c.refs[refidx].refmvs.is_some() && ref_w == f.bw && ref_h == f.bh {
                    f.rf.rp_ref[i] = c.refs[refidx].refmvs.clone();
                } else {
                    f.rf.rp_ref[i] = None;
                }
                f.refrefpoc[i] = c.refs[refidx].refpoc;
            }
        } else {
            f.rf.rp_ref = Default::default();
        }
    } else {
        f.rf.rp = Vec::new();
        f.rf.rp_ref = Default::default();
    }

    // segmap
    if frame_hdr.segmentation.enabled != 0 {
        // By default, the previous segmentation map is not initialised.
        f.prev_segmap = None;

        // We might need a previous frame's segmentation map. This
        // happens if there is either no update or a temporal update.
        if frame_hdr.segmentation.temporal != 0 || frame_hdr.segmentation.update_map == 0 {
            let pri_ref = frame_hdr.primary_ref_frame as usize;
            debug_assert!(pri_ref != DAV1D_PRIMARY_REF_NONE as usize);
            let ref_w = ((ref_coded_width[pri_ref] + 7) >> 3) << 1;
            let ref_h = ((f.refp[pri_ref].p.p.h + 7) >> 3) << 1;
            if ref_w == f.bw && ref_h == f.bh {
                f.prev_segmap = c.refs[frame_hdr.refidx[pri_ref] as usize].segmap.clone();
            }
        }

        let segmap_size = f.b4_stride * 32 * f.sb128h as usize;
        if frame_hdr.segmentation.update_map != 0 {
            // We're updating an existing map, but need somewhere to
            // put the new values. Allocate them here (the data
            // actually gets set elsewhere)
            f.cur_segmap = Some(Arc::new(
                try_vec(0u8, segmap_size).map_err(|_| dav1d_err(ENOMEM))?,
            ));
        } else if let Some(prev) = &f.prev_segmap {
            // We're not updating an existing map, and we have a valid
            // reference. Use that.
            f.cur_segmap = Some(prev.clone());
        } else {
            // We need to make a new map. Allocate one here and zero it out.
            f.cur_segmap = Some(Arc::new(
                try_vec(0u8, segmap_size).map_err(|_| dav1d_err(ENOMEM))?,
            ));
        }
    } else {
        f.cur_segmap = None;
        f.prev_segmap = None;
    }

    // (the references are updated after the frame decoded)
    let in_cdf = f.in_cdf.clone();
    let refresh_frame_flags = frame_hdr.refresh_frame_flags as u32;

    let res = decode_frame(c.inloop_filters, &mut c.fc, &mut c.tc);
    let f = &mut *c.fc;
    if let Err(e) = res {
        for i in 0..8 {
            if refresh_frame_flags & (1 << i) != 0 {
                c.refs[i].p.unref();
                c.cdf[i] = CdfThreadContext::default();
                c.refs[i].segmap = None;
                c.refs[i].refmvs = None;
            }
        }
        return Err(e);
    }

    // the decoded picture
    let cur_px = f.cur_px.take().expect("picture");
    if let Some(sr_px) = f.sr_px.take() {
        f.cur.data = Some(Arc::new(cur_px));
        f.sr_cur.p.data = Some(Arc::new(sr_px));
    } else {
        let data = Arc::new(cur_px);
        f.cur.data = Some(data.clone());
        f.sr_cur.p.data = Some(data);
    }
    let out_cdf = match f.out_cdf.take() {
        Some(cdf) if frame_hdr.refresh_context != 0 => CdfThreadContext::Ref(Arc::from(cdf)),
        _ => CdfThreadContext::default(),
    };
    let mvs = if has_mvs {
        Some(Arc::new(std::mem::take(&mut f.rf.rp)))
    } else {
        None
    };

    // move f->cur into output queue
    if frame_hdr.show_frame != 0 || c.output_invisible_frames {
        c.out = f.sr_cur.clone();
        c.event_flags |= picture_get_event_flags(&f.sr_cur);
    }

    // update references etc.
    for i in 0..8 {
        if refresh_frame_flags & (1 << i) != 0 {
            c.refs[i].p = f.sr_cur.clone();

            c.cdf[i] = if frame_hdr.refresh_context != 0 {
                out_cdf.clone()
            } else {
                in_cdf.clone()
            };

            c.refs[i].segmap = f.cur_segmap.clone();
            c.refs[i].refmvs = None;
            if frame_hdr.allow_intrabc == 0 {
                c.refs[i].refmvs = mvs.clone();
            }
            c.refs[i].refpoc = f.refpoc;
        }
    }

    f.cur = Dav1dPicture::default();
    f.sr_cur.unref();
    f.cur_segmap = None;
    f.seq_hdr = None;
    f.frame_hdr = None;

    Ok(())
}
