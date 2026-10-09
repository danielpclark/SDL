// Rust translation of src/wedge.c and src/wedge.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The wedge and inter-intra blend masks, built once on first use
//! (`dav1d_init_wedge_masks()` and `dav1d_init_interintra_masks()`).
//!
//! Upstream's pointer tables into static buffers are (buffer, offset)
//! pairs here; [`wedge_masks`] and [`ii_masks`] return the slices.

use std::sync::OnceLock;

use super::intops::{imax, imin};
use super::levels::*;

// enum WedgeDirectionType
const WEDGE_HORIZONTAL: u8 = 0;
const WEDGE_VERTICAL: u8 = 1;
const WEDGE_OBLIQUE27: u8 = 2;
const WEDGE_OBLIQUE63: u8 = 3;
const WEDGE_OBLIQUE117: u8 = 4;
const WEDGE_OBLIQUE153: u8 = 5;

/// Translation of `wedge_code_type`: direction, x_offset, y_offset.
type WedgeCodeType = [u8; 3];

static WEDGE_CODEBOOK_16_HGTW: [WedgeCodeType; 16] = [
    [WEDGE_OBLIQUE27, 4, 4],
    [WEDGE_OBLIQUE63, 4, 4],
    [WEDGE_OBLIQUE117, 4, 4],
    [WEDGE_OBLIQUE153, 4, 4],
    [WEDGE_HORIZONTAL, 4, 2],
    [WEDGE_HORIZONTAL, 4, 4],
    [WEDGE_HORIZONTAL, 4, 6],
    [WEDGE_VERTICAL, 4, 4],
    [WEDGE_OBLIQUE27, 4, 2],
    [WEDGE_OBLIQUE27, 4, 6],
    [WEDGE_OBLIQUE153, 4, 2],
    [WEDGE_OBLIQUE153, 4, 6],
    [WEDGE_OBLIQUE63, 2, 4],
    [WEDGE_OBLIQUE63, 6, 4],
    [WEDGE_OBLIQUE117, 2, 4],
    [WEDGE_OBLIQUE117, 6, 4],
];

static WEDGE_CODEBOOK_16_HLTW: [WedgeCodeType; 16] = [
    [WEDGE_OBLIQUE27, 4, 4],
    [WEDGE_OBLIQUE63, 4, 4],
    [WEDGE_OBLIQUE117, 4, 4],
    [WEDGE_OBLIQUE153, 4, 4],
    [WEDGE_VERTICAL, 2, 4],
    [WEDGE_VERTICAL, 4, 4],
    [WEDGE_VERTICAL, 6, 4],
    [WEDGE_HORIZONTAL, 4, 4],
    [WEDGE_OBLIQUE27, 4, 2],
    [WEDGE_OBLIQUE27, 4, 6],
    [WEDGE_OBLIQUE153, 4, 2],
    [WEDGE_OBLIQUE153, 4, 6],
    [WEDGE_OBLIQUE63, 2, 4],
    [WEDGE_OBLIQUE63, 6, 4],
    [WEDGE_OBLIQUE117, 2, 4],
    [WEDGE_OBLIQUE117, 6, 4],
];

static WEDGE_CODEBOOK_16_HEQW: [WedgeCodeType; 16] = [
    [WEDGE_OBLIQUE27, 4, 4],
    [WEDGE_OBLIQUE63, 4, 4],
    [WEDGE_OBLIQUE117, 4, 4],
    [WEDGE_OBLIQUE153, 4, 4],
    [WEDGE_HORIZONTAL, 4, 2],
    [WEDGE_HORIZONTAL, 4, 6],
    [WEDGE_VERTICAL, 2, 4],
    [WEDGE_VERTICAL, 6, 4],
    [WEDGE_OBLIQUE27, 4, 2],
    [WEDGE_OBLIQUE27, 4, 6],
    [WEDGE_OBLIQUE153, 4, 2],
    [WEDGE_OBLIQUE153, 4, 6],
    [WEDGE_OBLIQUE63, 2, 4],
    [WEDGE_OBLIQUE63, 6, 4],
    [WEDGE_OBLIQUE117, 2, 4],
    [WEDGE_OBLIQUE117, 6, 4],
];

/// A mask: index of its buffer and offset in it (`NONE` for `NULL`).
#[derive(Clone, Copy)]
struct MaskRef {
    buf: u16,
    off: u32,
}

const NONE: MaskRef = MaskRef {
    buf: u16::MAX,
    off: 0,
};

struct Masks {
    bufs: Vec<Vec<u8>>,
    /// `dav1d_wedge_masks[N_BS_SIZES][3 /* 444/luma, 422, 420 */][2 /* sign */][16 /* wedge_idx */]`
    wedge: [[[[MaskRef; 16]; 2]; 3]; N_BS_SIZES],
    /// `dav1d_ii_masks[N_BS_SIZES][3 /* 444/luma, 422, 420 */][N_INTER_INTRA_PRED_MODES]`
    ii: [[[MaskRef; N_INTER_INTRA_PRED_MODES]; 3]; N_BS_SIZES],
}

impl Masks {
    fn get(&self, m: MaskRef) -> &[u8] {
        &self.bufs[m.buf as usize][m.off as usize..]
    }
}

fn insert_border(dst: &mut [u8], src: &[u8], ctr: i32) {
    if ctr > 4 {
        dst[..(ctr - 4) as usize].fill(0);
    }
    let d = (imax(ctr, 4) - 4) as usize;
    let s = imax(4 - ctr, 0) as usize;
    let n = imin(64 - ctr, 8) as usize;
    dst[d..d + n].copy_from_slice(&src[s..s + n]);
    if ctr < 64 - 4 {
        dst[(ctr + 4) as usize..64].fill(64);
    }
}

fn transpose(dst: &mut [u8], src: &[u8]) {
    let mut y_off = 0;
    for y in 0..64 {
        let mut x_off = 0;
        for x in 0..64 {
            dst[x_off + y] = src[y_off + x];
            x_off += 64;
        }
        y_off += 64;
    }
}

fn hflip(dst: &mut [u8], src: &[u8]) {
    let mut y_off = 0;
    for _y in 0..64 {
        for x in 0..64 {
            dst[y_off + 64 - 1 - x] = src[y_off + x];
        }
        y_off += 64;
    }
}

fn invert(dst: &mut [u8], src: &[u8], w: usize, h: usize) {
    let mut y_off = 0;
    for _y in 0..h {
        for x in 0..w {
            dst[y_off + x] = 64 - src[y_off + x];
        }
        y_off += w;
    }
}

fn copy2d(dst: &mut [u8], src: &[u8], w: usize, h: usize, x_off: usize, y_off: usize) {
    let mut s = y_off * 64 + x_off;
    let mut d = 0;
    for _y in 0..h {
        dst[d..d + w].copy_from_slice(&src[s..s + w]);
        s += 64;
        d += w;
    }
}

fn init_chroma(chroma: &mut [u8], luma: &[u8], sign: i32, w: usize, h: usize, ss_ver: usize) {
    let mut l = 0;
    let mut c = 0;
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let mut sum = luma[l + x] as i32 + luma[l + x + 1] as i32 + 1;
            if ss_ver != 0 {
                sum += luma[l + w + x] as i32 + luma[l + w + x + 1] as i32 + 1;
            }
            chroma[c + (x >> 1)] = ((sum - sign) >> (1 + ss_ver)) as u8;
            x += 2;
        }
        l += w << ss_ver;
        c += w >> 1;
        y += 1 + ss_ver;
    }
}

/// `fill2d_16x2()`: `b444`, `b422` and `b420` are the indices of the
/// masks_444/422/420 buffers (the 444 one is also `dst`).
#[allow(clippy::too_many_arguments)]
fn fill2d_16x2(
    m: &mut Masks,
    w: usize,
    h: usize,
    bs: u8,
    master: &[Vec<u8>; 6],
    cb: &[WedgeCodeType; 16],
    b444: usize,
    b422: usize,
    b420: usize,
    signs: u32,
) {
    {
        let dst = &mut m.bufs[b444];
        let mut ptr = 0;
        for n in 0..16 {
            copy2d(
                &mut dst[ptr..],
                &master[cb[n][0] as usize],
                w,
                h,
                32 - (w * cb[n][1] as usize >> 3),
                32 - (h * cb[n][2] as usize >> 3),
            );
            ptr += w * h;
        }
        let (first, inv) = dst.split_at_mut(ptr);
        let mut off = 0;
        for _n in 0..16 {
            invert(&mut inv[off..], &first[off..], w, h);
            off += w * h;
        }
    }

    let n_stride_444 = w * h;
    let n_stride_422 = n_stride_444 >> 1;
    let n_stride_420 = n_stride_444 >> 2;
    let sign_stride_444 = 16 * n_stride_444;
    let sign_stride_422 = 16 * n_stride_422;
    let sign_stride_420 = 16 * n_stride_420;
    let (mut masks_444, mut masks_422, mut masks_420) = (0, 0, 0);
    // assign pointers in externally visible array
    for n in 0..16 {
        let sign = ((signs >> n) & 1) as usize;
        let r = |buf: usize, off: usize| MaskRef {
            buf: buf as u16,
            off: off as u32,
        };
        let wm = &mut m.wedge[bs as usize];
        wm[0][0][n] = r(b444, masks_444 + sign * sign_stride_444);
        // not using !sign is intentional here, since 444 does not require
        // any rounding since no chroma subsampling is applied.
        wm[0][1][n] = r(b444, masks_444 + sign * sign_stride_444);
        wm[1][0][n] = r(b422, masks_422 + sign * sign_stride_422);
        wm[1][1][n] = r(b422, masks_422 + (1 - sign) * sign_stride_422);
        wm[2][0][n] = r(b420, masks_420 + sign * sign_stride_420);
        wm[2][1][n] = r(b420, masks_420 + (1 - sign) * sign_stride_420);
        masks_444 += n_stride_444;
        masks_422 += n_stride_422;
        masks_420 += n_stride_420;

        // since the pointers come from inside, we know that
        // violation of the const is OK here. Any other approach
        // means we would have to duplicate the sign correction
        // logic in two places, which isn't very nice, or mark
        // the table faced externally as non-const, which also sucks
        let luma_ref = wm[0][0][n];
        let luma = m.bufs[b444][luma_ref.off as usize..luma_ref.off as usize + w * h].to_vec();
        let (c10, c11, c20, c21) = (wm[1][0][n], wm[1][1][n], wm[2][0][n], wm[2][1][n]);
        init_chroma(&mut m.bufs[b422][c10.off as usize..], &luma, 0, w, h, 0);
        init_chroma(&mut m.bufs[b422][c11.off as usize..], &luma, 1, w, h, 0);
        init_chroma(&mut m.bufs[b420][c20.off as usize..], &luma, 0, w, h, 1);
        init_chroma(&mut m.bufs[b420][c21.off as usize..], &luma, 1, w, h, 1);
    }
}

// enum WedgeMasterLineType
const WEDGE_MASTER_LINE_ODD: usize = 0;
const WEDGE_MASTER_LINE_EVEN: usize = 1;
const WEDGE_MASTER_LINE_VERT: usize = 2;

/// Translation of `dav1d_init_wedge_masks()` and
/// `dav1d_init_interintra_masks()`.
fn init_masks() -> Box<Masks> {
    static WEDGE_MASTER_BORDER: [[u8; 8]; 3] = [
        /* WEDGE_MASTER_LINE_ODD */ [1, 2, 6, 18, 37, 53, 60, 63],
        /* WEDGE_MASTER_LINE_EVEN */ [1, 4, 11, 27, 46, 58, 62, 63],
        /* WEDGE_MASTER_LINE_VERT */ [0, 2, 7, 21, 43, 57, 62, 64],
    ];
    let mut master: [Vec<u8>; 6] = std::array::from_fn(|_| vec![0u8; 64 * 64]);

    // create master templates
    let mut off = 0;
    for _y in 0..64 {
        insert_border(
            &mut master[WEDGE_VERTICAL as usize][off..],
            &WEDGE_MASTER_BORDER[WEDGE_MASTER_LINE_VERT],
            32,
        );
        off += 64;
    }
    let mut off = 0;
    let mut ctr = 48;
    let mut y = 0;
    while y < 64 {
        insert_border(
            &mut master[WEDGE_OBLIQUE63 as usize][off..],
            &WEDGE_MASTER_BORDER[WEDGE_MASTER_LINE_EVEN],
            ctr,
        );
        insert_border(
            &mut master[WEDGE_OBLIQUE63 as usize][off + 64..],
            &WEDGE_MASTER_BORDER[WEDGE_MASTER_LINE_ODD],
            ctr - 1,
        );
        y += 2;
        off += 128;
        ctr -= 1;
    }

    let m63 = master[WEDGE_OBLIQUE63 as usize].clone();
    transpose(&mut master[WEDGE_OBLIQUE27 as usize], &m63);
    let mv = master[WEDGE_VERTICAL as usize].clone();
    transpose(&mut master[WEDGE_HORIZONTAL as usize], &mv);
    hflip(&mut master[WEDGE_OBLIQUE117 as usize], &m63);
    let m27 = master[WEDGE_OBLIQUE27 as usize].clone();
    hflip(&mut master[WEDGE_OBLIQUE153 as usize], &m27);

    let mut m = Box::new(Masks {
        bufs: Vec::new(),
        wedge: [[[[NONE; 16]; 2]; 3]; N_BS_SIZES],
        ii: [[[NONE; N_INTER_INTRA_PRED_MODES]; 3]; N_BS_SIZES],
    });
    // #define fill(w, h, sz_422, sz_420, hvsw, signs)
    let fill = |m: &mut Masks, w: usize, h: usize, bs: u8, cb: &[WedgeCodeType; 16], signs: u32| {
        let b444 = new_buf(m, 2 * 16 * w * h);
        let b422 = new_buf(m, 2 * 16 * (w >> 1) * h);
        let b420 = new_buf(m, 2 * 16 * (w >> 1) * (h >> 1));
        fill2d_16x2(m, w, h, bs, &master, cb, b444, b422, b420, signs);
    };

    fill(&mut m, 32, 32, BS_32X32, &WEDGE_CODEBOOK_16_HEQW, 0x7bfb);
    fill(&mut m, 32, 16, BS_32X16, &WEDGE_CODEBOOK_16_HLTW, 0x7beb);
    fill(&mut m, 32, 8, BS_32X8, &WEDGE_CODEBOOK_16_HLTW, 0x6beb);
    fill(&mut m, 16, 32, BS_16X32, &WEDGE_CODEBOOK_16_HGTW, 0x7beb);
    fill(&mut m, 16, 16, BS_16X16, &WEDGE_CODEBOOK_16_HEQW, 0x7bfb);
    fill(&mut m, 16, 8, BS_16X8, &WEDGE_CODEBOOK_16_HLTW, 0x7beb);
    fill(&mut m, 8, 32, BS_8X32, &WEDGE_CODEBOOK_16_HGTW, 0x7aeb);
    fill(&mut m, 8, 16, BS_8X16, &WEDGE_CODEBOOK_16_HGTW, 0x7beb);
    fill(&mut m, 8, 8, BS_8X8, &WEDGE_CODEBOOK_16_HEQW, 0x7bfb);

    // dav1d_init_interintra_masks()
    let ii_dc_mask = new_buf(&mut m, 32 * 32);
    m.bufs[ii_dc_mask].fill(32);
    // ii_nondc_mask_WxH[N_II_PRED_MODES][W * H], N_II_PRED_MODES = 3
    let nondc = |m: &mut Masks, w: usize, h: usize, step: usize| -> usize {
        let b = new_buf(m, 3 * w * h);
        build_nondc_ii_masks(&mut m.bufs[b], w, h, step);
        b
    };
    let m32x32 = nondc(&mut m, 32, 32, 1);
    let m16x32 = nondc(&mut m, 16, 32, 1);
    let m16x16 = nondc(&mut m, 16, 16, 2);
    let m8x32 = nondc(&mut m, 8, 32, 1);
    let m8x16 = nondc(&mut m, 8, 16, 2);
    let m8x8 = nondc(&mut m, 8, 8, 4);
    let m4x16 = nondc(&mut m, 4, 16, 2);
    let m4x8 = nondc(&mut m, 4, 8, 4);
    let m4x4 = nondc(&mut m, 4, 4, 8);

    // #define set1(sz) / #define set(sz_444, sz_422, sz_420)
    let set1 = |b: usize, w: usize, h: usize| -> [MaskRef; N_INTER_INTRA_PRED_MODES] {
        let r = |k: usize| MaskRef {
            buf: b as u16,
            off: (k * w * h) as u32,
        };
        [
            /* II_DC_PRED */
            MaskRef {
                buf: ii_dc_mask as u16,
                off: 0,
            },
            /* II_VERT_PRED */ r(II_VERT_PRED as usize - 1),
            /* II_HOR_PRED */ r(II_HOR_PRED as usize - 1),
            /* II_SMOOTH_PRED */ r(II_SMOOTH_PRED as usize - 1),
        ]
    };
    m.ii[BS_8X8 as usize] = [set1(m8x8, 8, 8), set1(m4x8, 4, 8), set1(m4x4, 4, 4)];
    m.ii[BS_8X16 as usize] = [set1(m8x16, 8, 16), set1(m4x16, 4, 16), set1(m4x8, 4, 8)];
    m.ii[BS_16X8 as usize] = [set1(m16x16, 16, 16), set1(m8x8, 8, 8), set1(m8x8, 8, 8)];
    m.ii[BS_16X16 as usize] = [set1(m16x16, 16, 16), set1(m8x16, 8, 16), set1(m8x8, 8, 8)];
    m.ii[BS_16X32 as usize] = [set1(m16x32, 16, 32), set1(m8x32, 8, 32), set1(m8x16, 8, 16)];
    m.ii[BS_32X16 as usize] = [
        set1(m32x32, 32, 32),
        set1(m16x16, 16, 16),
        set1(m16x16, 16, 16),
    ];
    m.ii[BS_32X32 as usize] = [
        set1(m32x32, 32, 32),
        set1(m16x32, 16, 32),
        set1(m16x16, 16, 16),
    ];

    m
}

fn new_buf(m: &mut Masks, sz: usize) -> usize {
    m.bufs.push(vec![0u8; sz]);
    m.bufs.len() - 1
}

/// `build_nondc_ii_masks()`: `masks` holds `mask_v`, `mask_h` and
/// `mask_sm` (`ii_nondc_mask_WxH[II_VERT_PRED - 1]` and the next two).
fn build_nondc_ii_masks(masks: &mut [u8], w: usize, h: usize, step: usize) {
    static II_WEIGHTS_1D: [u8; 32] = [
        60, 52, 45, 39, 34, 30, 26, 22, 19, 17, 15, 13, 11, 10, 8, 7, 6, 6, 5, 4, 4, 3, 3, 2, 2, 2,
        2, 1, 1, 1, 1, 1,
    ];

    let (mask_v, rest) = masks.split_at_mut(w * h);
    let (mask_h, mask_sm) = rest.split_at_mut(w * h);
    let mut off = 0;
    for y in 0..h {
        mask_v[off..off + w].fill(II_WEIGHTS_1D[y * step]);
        for x in 0..w {
            mask_sm[off + x] = II_WEIGHTS_1D[imin(x as i32, y as i32) as usize * step];
            mask_h[off + x] = II_WEIGHTS_1D[x * step];
        }
        off += w;
    }
}

static MASKS: OnceLock<Box<Masks>> = OnceLock::new();

/// `dav1d_wedge_masks[bs][layout_idx][sign][wedge_idx]`.
pub(crate) fn wedge_masks(
    bs: u8,
    layout_idx: usize,
    sign: usize,
    wedge_idx: usize,
) -> &'static [u8] {
    let m = MASKS.get_or_init(init_masks);
    m.get(m.wedge[bs as usize][layout_idx][sign][wedge_idx])
}

/// `dav1d_ii_masks[bs][layout_idx][mode]`.
pub(crate) fn ii_masks(bs: u8, layout_idx: usize, mode: usize) -> &'static [u8] {
    let m = MASKS.get_or_init(init_masks);
    m.get(m.ii[bs as usize][layout_idx][mode])
}
