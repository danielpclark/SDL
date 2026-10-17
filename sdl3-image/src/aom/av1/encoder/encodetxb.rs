// Rust translation of av1/encoder/encodetxb.c and av1/encoder/encodetxb.h
// (and av1_get_tx_eob() and struct tokenize_b_args from
// av1/encoder/tokenize.h) from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches), as built without CONFIG_ENTROPY_STATS.
// Copyright (c) 2017, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Coding of the transform coefficients: the packing of a transform
//! block's coefficients, and the recording (with the CDF updates of the
//! frame contexts) of a coded block's coefficients for packing. The saver's
//! blocks are all intra and their transforms all of the 2D class.

use super::bitstream::av1_write_tx_type;
use super::block::*;
use super::encodemb::av1_foreach_transformed_block_in_plane;
use super::encoder::{Av1Comp, ThreadData};
use crate::aom::aom_dsp::aom_dsp_common::{ceil_power_of_two, round_power_of_two, TranLowT};
use crate::aom::aom_dsp::bitwriter::{aom_write_bit, aom_write_symbol, AomWriter};
use crate::aom::aom_dsp::prob::update_cdf;
use crate::aom::av1::common::alloccommon::try_vec;
use crate::aom::av1::common::av1_common_int::{av1_num_planes, Av1Common};
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::entropy::*;
use crate::aom::av1::common::entropymode::FrameContext;
use crate::aom::av1::common::entropymode::AV1_EXT_TX_IND;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::scan::{get_scan, ScanOrder};
use crate::aom::av1::common::seg_common::{segfeature_active, Segmentation, SEG_LVL_SKIP};
use crate::aom::av1::common::txb_common::*;

// (encodetxb.h)

pub(crate) const TXB_SKIP_CTX_MASK: i32 = 15;
pub(crate) const DC_SIGN_CTX_SHIFT: i32 = 4;
pub(crate) const DC_SIGN_CTX_MASK: i32 = 3;

// These numbers are empirically obtained.
pub(crate) static PLANE_RD_MULT: [[i32; PLANE_TYPES as usize]; REF_TYPES as usize] =
    [[17, 13], [16, 10]];

// (tokenize.h)

/// Translation of `struct tokenize_b_args`.
pub(crate) struct TokenizeBArgs<'a> {
    pub(crate) cpi: &'a Av1Comp,
    pub(crate) td: &'a mut ThreadData,
    pub(crate) this_rate: i32,
    pub(crate) allow_update_cdf: u8,
    pub(crate) dry_run: i32,
}

#[inline]
pub(crate) fn av1_get_tx_eob(seg: &Segmentation, segment_id: i32, tx_size: TxSize) -> i32 {
    let eob_max = av1_get_max_eob(tx_size);
    if segfeature_active(seg, segment_id as u8, SEG_LVL_SKIP) {
        0
    } else {
        eob_max
    }
}

// (encodetxb.c)

/// Translation of `av1_alloc_txb_buf()`: `cpi->coeff_buffer_base` gets a
/// buffer per superblock (upstream carves them out of one pool).
pub(crate) fn av1_alloc_txb_buf(cpi: &mut Av1Comp) -> Result<(), ()> {
    let cm = &cpi.common;
    let num_sb_rows = ceil_power_of_two(cm.mi_params.mi_rows, cm.seq_params.mib_size_log2);
    let num_sb_cols = ceil_power_of_two(cm.mi_params.mi_cols, cm.seq_params.mib_size_log2);
    let size = (num_sb_rows * num_sb_cols) as usize;
    let num_planes = av1_num_planes(cm);
    let subsampling_x = cm.seq_params.subsampling_x;
    let subsampling_y = cm.seq_params.subsampling_y;
    let luma_max_sb_square = 1usize << NUM_PELS_LOG2_LOOKUP[cm.seq_params.sb_size as usize];
    let chroma_max_sb_square = luma_max_sb_square >> (subsampling_x + subsampling_y);
    let txb_unit_size = (TX_SIZE_W_MIN * TX_SIZE_H_MIN) as usize;

    av1_free_txb_buf(cpi);
    // TODO(jingning): This should be further reduced.
    let mut coeff_buffer_base = Vec::new();
    coeff_buffer_base.try_reserve_exact(size).map_err(|_| ())?;
    for _i in 0..size {
        let mut buf = CbCoeffBuffer::default();
        for plane in 0..num_planes {
            let max_sb_square = if plane == AOM_PLANE_Y {
                luma_max_sb_square
            } else {
                chroma_max_sb_square
            };
            buf.tcoeff[plane] = try_vec(max_sb_square, 0)?;
            buf.eobs[plane] = try_vec(max_sb_square / txb_unit_size, 0)?;
            buf.entropy_ctx[plane] = try_vec(max_sb_square / txb_unit_size, 0)?;
        }
        coeff_buffer_base.push(buf);
    }
    cpi.coeff_buffer_base = coeff_buffer_base;
    Ok(())
}

pub(crate) fn av1_free_txb_buf(cpi: &mut Av1Comp) {
    cpi.coeff_buffer_base = Vec::new();
}

fn write_golomb(w: &mut AomWriter, level: i32) {
    let x = level + 1;
    let mut i = x;
    let mut length = 0;

    while i != 0 {
        i >>= 1;
        length += 1;
    }
    debug_assert!(length > 0);

    for _i in 0..length - 1 {
        aom_write_bit(w, 0);
    }

    let mut i = length - 1;
    while i >= 0 {
        aom_write_bit(w, (x >> i) & 0x01);
        i -= 1;
    }
}

static EOB_TO_POS_SMALL: [i8; 33] = [
    0, 1, 2, // 0-2
    3, 3, // 3-4
    4, 4, 4, 4, // 5-8
    5, 5, 5, 5, 5, 5, 5, 5, // 9-16
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, // 17-32
];

static EOB_TO_POS_LARGE: [i8; 17] = [
    6, // place holder
    7, // 33-64
    8, 8, // 65-128
    9, 9, 9, 9, // 129-256
    10, 10, 10, 10, 10, 10, 10, 10, // 257-512
    11, // 513-
];

pub(crate) fn av1_get_eob_pos_token(eob: i32, extra: &mut i32) -> i32 {
    let t: i32;

    if eob < 33 {
        t = EOB_TO_POS_SMALL[eob as usize] as i32;
    } else {
        let e = ((eob - 1) >> 5).min(16);
        t = EOB_TO_POS_LARGE[e as usize] as i32;
    }

    *extra = eob - AV1_EOB_GROUP_START[t as usize] as i32;

    t
}

pub(crate) fn av1_update_eob_context(
    eob: i32,
    tx_size: TxSize,
    tx_class: TxClass,
    plane: i32,
    ec_ctx: &mut FrameContext,
    allow_update_cdf: u8,
) {
    let mut eob_extra = 0;
    let eob_pt = av1_get_eob_pos_token(eob, &mut eob_extra);
    let txs_ctx = get_txsize_entropy_ctx(tx_size) as usize;

    let eob_multi_size = TXSIZE_LOG2_MINUS4[tx_size as usize];
    let eob_multi_ctx = if tx_class == TX_CLASS_2D { 0 } else { 1 };
    let plane = plane as usize;
    let sym = (eob_pt - 1) as i8;

    match eob_multi_size {
        0 => {
            if allow_update_cdf != 0 {
                update_cdf(&mut ec_ctx.eob_flag_cdf16[plane][eob_multi_ctx], sym, 5);
            }
        }
        1 => {
            if allow_update_cdf != 0 {
                update_cdf(&mut ec_ctx.eob_flag_cdf32[plane][eob_multi_ctx], sym, 6);
            }
        }
        2 => {
            if allow_update_cdf != 0 {
                update_cdf(&mut ec_ctx.eob_flag_cdf64[plane][eob_multi_ctx], sym, 7);
            }
        }
        3 => {
            if allow_update_cdf != 0 {
                update_cdf(&mut ec_ctx.eob_flag_cdf128[plane][eob_multi_ctx], sym, 8);
            }
        }
        4 => {
            if allow_update_cdf != 0 {
                update_cdf(&mut ec_ctx.eob_flag_cdf256[plane][eob_multi_ctx], sym, 9);
            }
        }
        5 => {
            if allow_update_cdf != 0 {
                update_cdf(&mut ec_ctx.eob_flag_cdf512[plane][eob_multi_ctx], sym, 10);
            }
        }
        _ => {
            // (case 6 and default)
            if allow_update_cdf != 0 {
                update_cdf(&mut ec_ctx.eob_flag_cdf1024[plane][eob_multi_ctx], sym, 11);
            }
        }
    }

    if AV1_EOB_OFFSET_BITS[eob_pt as usize] > 0 {
        let eob_ctx = (eob_pt - 3) as usize;
        let eob_shift = AV1_EOB_OFFSET_BITS[eob_pt as usize] as i32 - 1;
        let bit = if eob_extra & (1 << eob_shift) != 0 {
            1
        } else {
            0
        };
        if allow_update_cdf != 0 {
            update_cdf(&mut ec_ctx.eob_extra_cdf[txs_ctx][plane][eob_ctx], bit, 2);
        }
    }
}

#[inline]
fn get_nz_map_ctx(
    levels: &[u8],
    lv: usize,
    coeff_idx: i32,
    bwl: i32,
    height: i32,
    scan_idx: i32,
    is_eob: bool,
    tx_size: TxSize,
    tx_class: TxClass,
) -> i32 {
    if is_eob {
        if scan_idx == 0 {
            return 0;
        }
        if scan_idx <= (height << bwl) / 8 {
            return 1;
        }
        if scan_idx <= (height << bwl) / 4 {
            return 2;
        }
        return 3;
    }
    let stats = get_nz_mag(
        levels,
        lv + get_padded_idx(coeff_idx, bwl) as usize,
        bwl,
        tx_class,
    );
    get_nz_map_ctx_from_stats(stats, coeff_idx, bwl, tx_size, tx_class)
}

/// Translation of `av1_txb_init_levels_c()`: `levels` is
/// `levels_buf[lv..]`.
pub(crate) fn av1_txb_init_levels(
    coeff: &[TranLowT],
    width: i32,
    height: i32,
    levels_buf: &mut [u8],
    lv: usize,
) {
    let stride = (width + TX_PAD_HOR) as usize;
    let (width, height) = (width as usize, height as usize);
    let mut ls = lv;

    let end = lv + stride * height;
    levels_buf[end..end + TX_PAD_BOTTOM as usize * stride + TX_PAD_END as usize].fill(0);

    for i in 0..height {
        for j in 0..width {
            levels_buf[ls] = coeff[i * width + j].unsigned_abs().min(i8::MAX as u32) as u8;
            ls += 1;
        }
        for _j in 0..TX_PAD_HOR {
            levels_buf[ls] = 0;
            ls += 1;
        }
    }
}

/// Translation of `av1_get_nz_map_contexts_c()`.
pub(crate) fn av1_get_nz_map_contexts(
    levels: &[u8],
    lv: usize,
    scan: &[i16],
    eob: u16,
    tx_size: TxSize,
    tx_class: TxClass,
    coeff_contexts: &mut [i8],
) {
    let bwl = get_txb_bwl(tx_size);
    let height = get_txb_high(tx_size);
    for i in 0..eob as i32 {
        let pos = scan[i as usize] as i32;
        coeff_contexts[pos as usize] = get_nz_map_ctx(
            levels,
            lv,
            pos,
            bwl,
            height,
            i,
            i == eob as i32 - 1,
            tx_size,
            tx_class,
        ) as i8;
    }
}

pub(crate) fn av1_write_coeffs_txb(
    cm: &Av1Common,
    x: &mut Macroblock,
    w: &mut AomWriter,
    blk_row: i32,
    blk_col: i32,
    plane: usize,
    block: i32,
    tx_size: TxSize,
) {
    let plane_type = get_plane_type(plane);
    let cb_offset = x.mbmi_ext_frame_base[x.mbmi_ext_frame].cb_offset[plane_type as usize] as usize;
    let txb_offset = cb_offset / (TX_SIZE_W_MIN * TX_SIZE_H_MIN) as usize;
    let Macroblock {
        e_mbd: xd,
        cb_coeff_buffers,
        cb_coef_buff,
        ..
    } = x;
    let cb_coef_buff = &cb_coeff_buffers[*cb_coef_buff];
    let eob_txb = &cb_coef_buff.eobs[plane][txb_offset..];
    let eob = eob_txb[block as usize];
    let entropy_ctx = &cb_coef_buff.entropy_ctx[plane][txb_offset..];
    let txb_skip_ctx = (entropy_ctx[block as usize] as i32 & TXB_SKIP_CTX_MASK) as usize;
    let txs_ctx = get_txsize_entropy_ctx(tx_size) as usize;
    aom_write_symbol(
        w,
        (eob == 0) as i32,
        &mut xd.tile_ctx.txb_skip_cdf[txs_ctx][txb_skip_ctx],
        2,
    );
    if eob == 0 {
        return;
    }

    let tx_type = av1_get_tx_type(
        xd,
        plane_type,
        blk_row,
        blk_col,
        tx_size,
        cm.features.reduced_tx_set_used as i32,
    );
    // Only y plane's tx_type is transmitted
    if plane == 0 {
        av1_write_tx_type(cm, xd, tx_type, tx_size, w);
    }

    let ec_ctx: &mut FrameContext = &mut xd.tile_ctx;
    let pt = plane_type as usize;
    let mut eob_extra = 0;
    let eob_pt = av1_get_eob_pos_token(eob as i32, &mut eob_extra);
    let eob_multi_size = TXSIZE_LOG2_MINUS4[tx_size as usize];
    let tx_class = TX_TYPE_TO_CLASS[tx_type as usize];
    let eob_multi_ctx = if tx_class == TX_CLASS_2D { 0 } else { 1 };
    match eob_multi_size {
        0 => aom_write_symbol(
            w,
            eob_pt - 1,
            &mut ec_ctx.eob_flag_cdf16[pt][eob_multi_ctx],
            5,
        ),
        1 => aom_write_symbol(
            w,
            eob_pt - 1,
            &mut ec_ctx.eob_flag_cdf32[pt][eob_multi_ctx],
            6,
        ),
        2 => aom_write_symbol(
            w,
            eob_pt - 1,
            &mut ec_ctx.eob_flag_cdf64[pt][eob_multi_ctx],
            7,
        ),
        3 => aom_write_symbol(
            w,
            eob_pt - 1,
            &mut ec_ctx.eob_flag_cdf128[pt][eob_multi_ctx],
            8,
        ),
        4 => aom_write_symbol(
            w,
            eob_pt - 1,
            &mut ec_ctx.eob_flag_cdf256[pt][eob_multi_ctx],
            9,
        ),
        5 => aom_write_symbol(
            w,
            eob_pt - 1,
            &mut ec_ctx.eob_flag_cdf512[pt][eob_multi_ctx],
            10,
        ),
        _ => aom_write_symbol(
            w,
            eob_pt - 1,
            &mut ec_ctx.eob_flag_cdf1024[pt][eob_multi_ctx],
            11,
        ),
    }

    let eob_offset_bits = AV1_EOB_OFFSET_BITS[eob_pt as usize] as i32;
    if eob_offset_bits > 0 {
        let eob_ctx = (eob_pt - 3) as usize;
        let mut eob_shift = eob_offset_bits - 1;
        let mut bit = if eob_extra & (1 << eob_shift) != 0 {
            1
        } else {
            0
        };
        aom_write_symbol(w, bit, &mut ec_ctx.eob_extra_cdf[txs_ctx][pt][eob_ctx], 2);
        for i in 1..eob_offset_bits {
            eob_shift = eob_offset_bits - 1 - i;
            bit = if eob_extra & (1 << eob_shift) != 0 {
                1
            } else {
                0
            };
            aom_write_bit(w, bit);
        }
    }

    let width = get_txb_wide(tx_size);
    let height = get_txb_high(tx_size);
    let mut levels_buf = [0u8; TX_PAD_2D as usize];
    let lv = set_levels(width);
    let tcoeff_txb = &cb_coef_buff.tcoeff[plane][cb_offset..];
    let tcoeff = &tcoeff_txb[block_offset(block)..];
    av1_txb_init_levels(tcoeff, width, height, &mut levels_buf, lv);
    let scan_order = get_scan(tx_size, tx_type);
    let scan = scan_order.scan;
    let mut coeff_contexts = [0i8; MAX_TX_SQUARE as usize];
    av1_get_nz_map_contexts(
        &levels_buf,
        lv,
        scan,
        eob,
        tx_size,
        tx_class,
        &mut coeff_contexts,
    );

    let bwl = get_txb_bwl(tx_size);
    let mut c = eob as i32 - 1;
    while c >= 0 {
        let pos = scan[c as usize] as usize;
        let coeff_ctx = coeff_contexts[pos] as usize;
        let v = tcoeff[pos];
        let level = v.abs();

        if c == eob as i32 - 1 {
            aom_write_symbol(
                w,
                level.min(3) - 1,
                &mut ec_ctx.coeff_base_eob_cdf[txs_ctx][pt][coeff_ctx],
                3,
            );
        } else {
            aom_write_symbol(
                w,
                level.min(3),
                &mut ec_ctx.coeff_base_cdf[txs_ctx][pt][coeff_ctx],
                4,
            );
        }
        if level > NUM_BASE_LEVELS {
            // level is above 1.
            let base_range = level - 1 - NUM_BASE_LEVELS;
            let br_ctx = get_br_ctx(&levels_buf, lv, pos as i32, bwl, tx_class) as usize;
            let cdf = &mut ec_ctx.coeff_br_cdf[txs_ctx.min(TX_32X32 as usize)][pt][br_ctx];
            let mut idx = 0;
            while idx < COEFF_BASE_RANGE {
                let k = (base_range - idx).min(BR_CDF_SIZE - 1);
                aom_write_symbol(w, k, cdf, BR_CDF_SIZE);
                if k < BR_CDF_SIZE - 1 {
                    break;
                }
                idx += BR_CDF_SIZE - 1;
            }
        }
        c -= 1;
    }

    // Loop to code all signs in the transform block,
    // starting with the sign of DC (if applicable)
    for c in 0..eob as usize {
        let v = tcoeff[scan[c] as usize];
        let level = v.abs();
        let sign = if v < 0 { 1 } else { 0 };
        if level != 0 {
            if c == 0 {
                let dc_sign_ctx = ((entropy_ctx[block as usize] as i32 >> DC_SIGN_CTX_SHIFT)
                    & DC_SIGN_CTX_MASK) as usize;
                aom_write_symbol(w, sign, &mut ec_ctx.dc_sign_cdf[pt][dc_sign_ctx], 2);
            } else {
                aom_write_bit(w, sign);
            }
            if level > COEFF_BASE_RANGE + NUM_BASE_LEVELS {
                write_golomb(w, level - COEFF_BASE_RANGE - 1 - NUM_BASE_LEVELS);
            }
        }
    }
}

pub(crate) fn av1_write_intra_coeffs_mb(
    cm: &Av1Common,
    x: &mut Macroblock,
    w: &mut AomWriter,
    bsize: BlockSize,
) {
    let num_planes = av1_num_planes(cm);
    let mut block = [0i32; MAX_MB_PLANE];
    let xd = &x.e_mbd;
    debug_assert!(
        bsize == get_plane_block_size(bsize, xd.plane[0].subsampling_x, xd.plane[0].subsampling_y)
    );
    let max_blocks_wide = max_block_wide(xd, bsize, 0);
    let max_blocks_high = max_block_high(xd, bsize, 0);
    let max_unit_bsize = BLOCK_64X64;
    let mut mu_blocks_wide = MI_SIZE_WIDE[max_unit_bsize as usize] as i32;
    let mut mu_blocks_high = MI_SIZE_HIGH[max_unit_bsize as usize] as i32;
    mu_blocks_wide = max_blocks_wide.min(mu_blocks_wide);
    mu_blocks_high = max_blocks_high.min(mu_blocks_high);

    let mut row = 0;
    while row < max_blocks_high {
        let mut col = 0;
        while col < max_blocks_wide {
            for plane in 0..num_planes {
                if plane != 0 && !x.e_mbd.is_chroma_ref {
                    break;
                }
                let tx_size = av1_get_tx_size(plane, &x.e_mbd);
                let stepr = TX_SIZE_HIGH_UNIT[tx_size as usize] as i32;
                let stepc = TX_SIZE_WIDE_UNIT[tx_size as usize] as i32;
                let step = stepr * stepc;
                let pd = &x.e_mbd.plane[plane];
                let (ss_x, ss_y) = (pd.subsampling_x, pd.subsampling_y);
                let unit_height =
                    round_power_of_two((mu_blocks_high + row).min(max_blocks_high), ss_y);
                let unit_width =
                    round_power_of_two((mu_blocks_wide + col).min(max_blocks_wide), ss_x);
                let mut blk_row = row >> ss_y;
                while blk_row < unit_height {
                    let mut blk_col = col >> ss_x;
                    while blk_col < unit_width {
                        av1_write_coeffs_txb(
                            cm,
                            x,
                            w,
                            blk_row,
                            blk_col,
                            plane,
                            block[plane],
                            tx_size,
                        );
                        block[plane] += step;
                        blk_col += stepc;
                    }
                    blk_row += stepr;
                }
            }
            col += mu_blocks_wide;
        }
        row += mu_blocks_high;
    }
}

pub(crate) fn av1_get_txb_entropy_context(
    qcoeff: &[TranLowT],
    scan_order: &ScanOrder,
    eob: i32,
) -> u8 {
    let scan = scan_order.scan;
    let mut cul_level: i32 = 0;

    if eob == 0 {
        return 0;
    }
    for c in 0..eob as usize {
        cul_level += qcoeff[scan[c] as usize].abs();
        if cul_level > COEFF_CONTEXT_MASK {
            break;
        }
    }

    cul_level = cul_level.min(COEFF_CONTEXT_MASK);
    set_dc_sign(&mut cul_level, qcoeff[0]);

    cul_level as u8
}

fn update_tx_type_count(
    cpi: &Av1Comp,
    cm: &Av1Common,
    xd: &mut MacroBlockD,
    blk_row: i32,
    blk_col: i32,
    plane: usize,
    tx_size: TxSize,
    allow_update_cdf: u8,
) {
    let mbmi = *xd.mbmi();
    let is_inter = is_inter_block(&mbmi) as i32;
    let reduced_tx_set_used = cm.features.reduced_tx_set_used as i32;

    // Only y plane's tx_type is updated
    if plane > 0 {
        return;
    }
    let tx_type = av1_get_tx_type(
        xd,
        PLANE_TYPE_Y,
        blk_row,
        blk_col,
        tx_size,
        reduced_tx_set_used,
    );
    // (the saver's blocks are all intra)
    debug_assert!(is_inter == 0);
    if cpi.oxcf.txfm_cfg.use_intra_dct_only {
        debug_assert!(tx_type == DCT_DCT);
    } else if cpi.oxcf.txfm_cfg.use_intra_default_tx_only {
        // (upstream only asserts here; with the saver's non-RD pick mode the
        // assertion always holds)
        debug_assert!(cpi.sf.rt_sf.use_nonrd_pick_mode != 0);
    }

    if get_ext_tx_types(tx_size, is_inter, reduced_tx_set_used) > 1
        && cm.quant_params.base_qindex > 0
        && mbmi.skip_txfm == 0
        && !segfeature_active(&cm.seg, mbmi.segment_id, SEG_LVL_SKIP)
    {
        let eset = get_ext_tx_set(tx_size, is_inter, reduced_tx_set_used);
        if eset > 0 {
            let tx_set_type = av1_get_ext_tx_set_type(tx_size, is_inter, reduced_tx_set_used);
            // (no filter intra)
            debug_assert!(mbmi.filter_intra_mode_info.use_filter_intra == 0);
            let intra_dir = mbmi.mode;
            if allow_update_cdf != 0 {
                update_cdf(
                    &mut xd.tile_ctx.intra_ext_tx_cdf[eset as usize]
                        [TXSIZE_SQR_MAP[tx_size as usize] as usize][intra_dir as usize],
                    AV1_EXT_TX_IND[tx_set_type as usize][tx_type as usize] as i8,
                    AV1_NUM_EXT_TX_SET[tx_set_type as usize],
                );
            }
        }
    }
}

/// `BLOCK_OFFSET()`
#[inline]
pub(crate) fn block_offset(block: i32) -> usize {
    (block << 4) as usize
}

pub(crate) fn av1_update_and_record_txb_context(
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    args: &mut TokenizeBArgs,
) {
    let cpi = args.cpi;
    let cm = &cpi.common;
    let td = &mut *args.td;
    let x = &mut td.mb;
    let eob = x.plane[plane].eobs[block as usize] as i32;
    let block_offset = block_offset(block);
    let plane_type = x.e_mbd.plane[plane].plane_type;
    let tx_type = av1_get_tx_type(
        &x.e_mbd,
        plane_type,
        blk_row,
        blk_col,
        tx_size,
        cm.features.reduced_tx_set_used as i32,
    );
    let scan_order = get_scan(tx_size, tx_type);
    debug_assert!(args.dry_run != DRY_RUN_COSTCOEFFS);
    let cul_level: u8;
    if args.dry_run == OUTPUT_ENABLED {
        let mut txb_ctx = TxbCtx::default();
        {
            let xd = &x.e_mbd;
            let pd = &xd.plane[plane];
            get_txb_ctx(
                plane_bsize,
                tx_size,
                plane,
                &xd.above_entropy(plane)[pd.above_entropy_context + blk_col as usize..],
                &xd.left_entropy_context[plane][pd.left_entropy_context + blk_row as usize..],
                &mut txb_ctx,
            );
        }
        let bwl = get_txb_bwl(tx_size);
        let width = get_txb_wide(tx_size);
        let height = get_txb_high(tx_size);
        let allow_update_cdf = args.allow_update_cdf;
        let txsize_ctx = get_txsize_entropy_ctx(tx_size) as usize;
        let pt = plane_type as usize;
        if allow_update_cdf != 0 {
            update_cdf(
                &mut x.e_mbd.tile_ctx.txb_skip_cdf[txsize_ctx][txb_ctx.txb_skip_ctx as usize],
                (eob == 0) as i8,
                2,
            );
        }

        let cb_offset =
            x.mbmi_ext_frame_base[x.mbmi_ext_frame].cb_offset[plane_type as usize] as usize;
        let txb_offset = cb_offset / (TX_SIZE_W_MIN * TX_SIZE_H_MIN) as usize;
        let cb_idx = x.cb_coef_buff;
        {
            let cb_coef_buff = &mut x.cb_coeff_buffers[cb_idx];
            cb_coef_buff.entropy_ctx[plane][txb_offset + block as usize] =
                txb_ctx.txb_skip_ctx as u8;
            cb_coef_buff.eobs[plane][txb_offset + block as usize] = eob as u16;
        }

        if eob == 0 {
            av1_set_entropy_contexts(
                &mut x.e_mbd,
                plane,
                plane_bsize,
                tx_size,
                0,
                blk_col,
                blk_row,
            );
            return;
        }
        let segment_id = x.e_mbd.mbmi().segment_id as i32;
        let seg_eob = av1_get_tx_eob(&cpi.common.seg, segment_id, tx_size) as usize;
        let tc = cb_offset + block_offset;
        {
            let Macroblock {
                plane: planes,
                cb_coeff_buffers,
                ..
            } = &mut *x;
            cb_coeff_buffers[cb_idx].tcoeff[plane][tc..tc + seg_eob]
                .copy_from_slice(&planes[plane].qcoeff[block_offset..block_offset + seg_eob]);
        }

        let mut levels_buf = [0u8; TX_PAD_2D as usize];
        let lv = set_levels(width);
        av1_txb_init_levels(
            &x.cb_coeff_buffers[cb_idx].tcoeff[plane][tc..],
            width,
            height,
            &mut levels_buf,
            lv,
        );
        update_tx_type_count(
            cpi,
            cm,
            &mut x.e_mbd,
            blk_row,
            blk_col,
            plane,
            tx_size,
            allow_update_cdf,
        );

        let tx_class = TX_TYPE_TO_CLASS[tx_type as usize];
        let scan = scan_order.scan;

        // record tx type usage
        td.rd_counts.tx_type_used[tx_size as usize][tx_type as usize] += 1;
        let x = &mut td.mb;

        let ec_ctx: &mut FrameContext = &mut x.e_mbd.tile_ctx;
        av1_update_eob_context(eob, tx_size, tx_class, plane_type, ec_ctx, allow_update_cdf);

        let mut coeff_contexts = [0i8; MAX_TX_SQUARE as usize];
        av1_get_nz_map_contexts(
            &levels_buf,
            lv,
            scan,
            eob as u16,
            tx_size,
            tx_class,
            &mut coeff_contexts,
        );

        let qcoeff = &x.plane[plane].qcoeff[block_offset..];
        let mut c = eob - 1;
        while c >= 0 {
            let pos = scan[c as usize] as usize;
            let coeff_ctx = coeff_contexts[pos] as usize;
            let v = qcoeff[pos];
            let level = v.abs();
            // abs_sum_level is needed to decide the job scheduling order of
            // pack bitstream multi-threading. This data is not needed if
            // multi-threading is disabled.
            // (pack_bs_mt_enabled: the saver encodes with one thread)
            debug_assert!(!cpi.mt_info.pack_bs_mt_enabled);

            if allow_update_cdf != 0 {
                if c == eob - 1 {
                    debug_assert!(coeff_ctx < 4);
                    update_cdf(
                        &mut ec_ctx.coeff_base_eob_cdf[txsize_ctx][pt][coeff_ctx],
                        (level.min(3) - 1) as i8,
                        3,
                    );
                } else {
                    update_cdf(
                        &mut ec_ctx.coeff_base_cdf[txsize_ctx][pt][coeff_ctx],
                        level.min(3) as i8,
                        4,
                    );
                }
            }
            if c == eob - 1 {
                debug_assert!(coeff_ctx < 4);
            }
            if level > NUM_BASE_LEVELS {
                let base_range = level - 1 - NUM_BASE_LEVELS;
                let br_ctx = get_br_ctx(&levels_buf, lv, pos as i32, bwl, tx_class) as usize;
                let mut idx = 0;
                while idx < COEFF_BASE_RANGE {
                    let k = (base_range - idx).min(BR_CDF_SIZE - 1);
                    if allow_update_cdf != 0 {
                        update_cdf(
                            &mut ec_ctx.coeff_br_cdf[txsize_ctx.min(TX_32X32 as usize)][pt][br_ctx],
                            k as i8,
                            BR_CDF_SIZE,
                        );
                    }
                    if k < BR_CDF_SIZE - 1 {
                        break;
                    }
                    idx += BR_CDF_SIZE - 1;
                }
            }
            c -= 1;
        }
        // Update the context needed to code the DC sign (if applicable)
        let tcoeff = &x.cb_coeff_buffers[cb_idx].tcoeff[plane][tc..];
        if tcoeff[0] != 0 {
            let dc_sign = if tcoeff[0] < 0 { 1 } else { 0 };
            let dc_sign_ctx = txb_ctx.dc_sign_ctx;
            if allow_update_cdf != 0 {
                update_cdf(
                    &mut ec_ctx.dc_sign_cdf[pt][dc_sign_ctx as usize],
                    dc_sign,
                    2,
                );
            }
            x.cb_coeff_buffers[cb_idx].entropy_ctx[plane][txb_offset + block as usize] |=
                (dc_sign_ctx << DC_SIGN_CTX_SHIFT) as u8;
        }
        cul_level = av1_get_txb_entropy_context(
            &x.cb_coeff_buffers[cb_idx].tcoeff[plane][tc..],
            &scan_order,
            eob,
        );
    } else {
        cul_level =
            av1_get_txb_entropy_context(&x.plane[plane].qcoeff[block_offset..], &scan_order, eob);
    }
    let x = &mut td.mb;
    av1_set_entropy_contexts(
        &mut x.e_mbd,
        plane,
        plane_bsize,
        tx_size,
        cul_level as i32,
        blk_col,
        blk_row,
    );
}

pub(crate) fn av1_record_txb_context(
    plane: usize,
    block: i32,
    blk_row: i32,
    blk_col: i32,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    args: &mut TokenizeBArgs,
) {
    let cpi = args.cpi;
    let cm = &cpi.common;
    let td = &mut *args.td;
    let x = &mut td.mb;
    let eob = x.plane[plane].eobs[block as usize] as i32;
    let block_offset = block_offset(block);
    let plane_type = x.e_mbd.plane[plane].plane_type;
    let tx_type = av1_get_tx_type(
        &x.e_mbd,
        plane_type,
        blk_row,
        blk_col,
        tx_size,
        cm.features.reduced_tx_set_used as i32,
    );
    let scan_order = get_scan(tx_size, tx_type);
    debug_assert!(args.dry_run != DRY_RUN_COSTCOEFFS);
    let cul_level: u8;
    if args.dry_run == OUTPUT_ENABLED {
        let mut txb_ctx = TxbCtx::default();
        {
            let xd = &x.e_mbd;
            let pd = &xd.plane[plane];
            get_txb_ctx(
                plane_bsize,
                tx_size,
                plane,
                &xd.above_entropy(plane)[pd.above_entropy_context + blk_col as usize..],
                &xd.left_entropy_context[plane][pd.left_entropy_context + blk_row as usize..],
                &mut txb_ctx,
            );
        }
        let cb_offset =
            x.mbmi_ext_frame_base[x.mbmi_ext_frame].cb_offset[plane_type as usize] as usize;
        let txb_offset = cb_offset / (TX_SIZE_W_MIN * TX_SIZE_H_MIN) as usize;
        let cb_idx = x.cb_coef_buff;
        {
            let cb_coef_buff = &mut x.cb_coeff_buffers[cb_idx];
            cb_coef_buff.entropy_ctx[plane][txb_offset + block as usize] =
                txb_ctx.txb_skip_ctx as u8;
            cb_coef_buff.eobs[plane][txb_offset + block as usize] = eob as u16;
        }

        if eob == 0 {
            av1_set_entropy_contexts(
                &mut x.e_mbd,
                plane,
                plane_bsize,
                tx_size,
                0,
                blk_col,
                blk_row,
            );
            return;
        }
        let segment_id = x.e_mbd.mbmi().segment_id as i32;
        let seg_eob = av1_get_tx_eob(&cpi.common.seg, segment_id, tx_size) as usize;
        let tc = cb_offset + block_offset;
        {
            let Macroblock {
                plane: planes,
                cb_coeff_buffers,
                ..
            } = &mut *x;
            cb_coeff_buffers[cb_idx].tcoeff[plane][tc..tc + seg_eob]
                .copy_from_slice(&planes[plane].qcoeff[block_offset..block_offset + seg_eob]);
        }

        // (do_coeff_scan is cpi->mt_info.pack_bs_mt_enabled, false: the saver
        // encodes with one thread, so the scan accumulating td->abs_sum_level
        // never runs)
        debug_assert!(!cpi.mt_info.pack_bs_mt_enabled);

        // record tx type usage
        td.rd_counts.tx_type_used[tx_size as usize][tx_type as usize] += 1;
        let x = &mut td.mb;

        let tcoeff0 = x.cb_coeff_buffers[cb_idx].tcoeff[plane][tc];
        if tcoeff0 != 0 {
            let dc_sign_ctx = txb_ctx.dc_sign_ctx;
            x.cb_coeff_buffers[cb_idx].entropy_ctx[plane][txb_offset + block as usize] |=
                (dc_sign_ctx << DC_SIGN_CTX_SHIFT) as u8;
        }
        cul_level = av1_get_txb_entropy_context(
            &x.cb_coeff_buffers[cb_idx].tcoeff[plane][tc..],
            &scan_order,
            eob,
        );
    } else {
        cul_level =
            av1_get_txb_entropy_context(&x.plane[plane].qcoeff[block_offset..], &scan_order, eob);
    }
    let x = &mut td.mb;
    av1_set_entropy_contexts(
        &mut x.e_mbd,
        plane,
        plane_bsize,
        tx_size,
        cul_level as i32,
        blk_col,
        blk_row,
    );
}

pub(crate) fn av1_update_intra_mb_txb_context(
    cpi: &Av1Comp,
    td: &mut ThreadData,
    dry_run: i32,
    bsize: BlockSize,
    allow_update_cdf: u8,
) {
    let cm = &cpi.common;
    let num_planes = av1_num_planes(cm);
    let mut arg = TokenizeBArgs {
        cpi,
        td,
        this_rate: 0,
        allow_update_cdf,
        dry_run,
    };
    if arg.td.mb.e_mbd.mbmi().skip_txfm != 0 {
        av1_reset_entropy_context(&mut arg.td.mb.e_mbd, bsize, num_planes);
        return;
    }
    let visit: fn(usize, i32, i32, i32, BlockSize, TxSize, &mut TokenizeBArgs) =
        if allow_update_cdf != 0 {
            av1_update_and_record_txb_context
        } else {
            av1_record_txb_context
        };
    for plane in 0..num_planes {
        let xd = &arg.td.mb.e_mbd;
        if plane != 0 && !xd.is_chroma_ref {
            break;
        }
        let pd = &xd.plane[plane];
        let ss_x = pd.subsampling_x;
        let ss_y = pd.subsampling_y;
        let plane_bsize = get_plane_block_size(bsize, ss_x, ss_y);
        av1_foreach_transformed_block_in_plane(
            &mut arg,
            |a| &a.td.mb.e_mbd,
            plane_bsize,
            plane,
            |a, plane, block, blk_row, blk_col, plane_bsize, tx_size| {
                visit(plane, block, blk_row, blk_col, plane_bsize, tx_size, a)
            },
        );
    }
}

/// Translation of `av1_get_cb_coeff_buffer()`: the index of the
/// superblock's buffer in `cpi->coeff_buffer_base`.
pub(crate) fn av1_get_cb_coeff_buffer(cpi: &Av1Comp, mi_row: i32, mi_col: i32) -> usize {
    let cm = &cpi.common;
    let mib_size_log2 = cm.seq_params.mib_size_log2;
    let stride = ceil_power_of_two(cm.mi_params.mi_cols, cm.seq_params.mib_size_log2);
    let offset = (mi_row >> mib_size_log2) * stride + (mi_col >> mib_size_log2);
    offset as usize
}
