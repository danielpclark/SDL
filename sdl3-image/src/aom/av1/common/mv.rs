// Rust translation of the parts of av1/common/mv.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the encoder reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion vectors, and the global motion parameters (which stay the
//! identity: the real-time speed features never search global motion).

use super::enums::{BlockSize, BLOCK_SIZE_HIGH, BLOCK_SIZE_WIDE, MI_SIZE};

pub(crate) const INVALID_MV: u32 = 0x80008000;
pub(crate) const INVALID_MV_ROW_COL: i32 = -32768;

#[inline]
pub(crate) const fn get_mv_rawpel(x: i32) -> i32 {
    (x + 3 + (x >= 0) as i32) >> 3
}

#[inline]
pub(crate) const fn get_mv_subpel(x: i32) -> i32 {
    x * 8
}

// The motion vector in units of full pixel
/// `FULLPEL_MV`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FullpelMv {
    pub(crate) row: i16,
    pub(crate) col: i16,
}

// The motion vector in units of 1/8-pel
/// `MV`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Mv {
    pub(crate) row: i16,
    pub(crate) col: i16,
}

/// `kZeroMv`
pub(crate) const K_ZERO_MV: Mv = Mv { row: 0, col: 0 };
/// `kZeroFullMv`
pub(crate) const K_ZERO_FULL_MV: FullpelMv = FullpelMv { row: 0, col: 0 };

/// Translation of `int_mv`: the union's `as_int` view (`as_mv` and
/// `as_fullmv` are `{ row, col }` in the low and high halves, as on the
/// little-endian targets).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct IntMv {
    pub(crate) as_int: u32,
}

impl IntMv {
    /// `.as_mv`
    #[inline]
    pub(crate) const fn as_mv(self) -> Mv {
        Mv {
            row: self.as_int as u16 as i16,
            col: (self.as_int >> 16) as u16 as i16,
        }
    }

    /// `.as_fullmv`
    #[inline]
    pub(crate) const fn as_fullmv(self) -> FullpelMv {
        FullpelMv {
            row: self.as_int as u16 as i16,
            col: (self.as_int >> 16) as u16 as i16,
        }
    }

    /// An `int_mv` holding `mv`.
    #[inline]
    pub(crate) const fn from_mv(mv: Mv) -> IntMv {
        IntMv {
            as_int: (mv.row as u16 as u32) | ((mv.col as u16 as u32) << 16),
        }
    }

    /// An `int_mv` holding `mv`.
    #[inline]
    pub(crate) const fn from_fullmv(mv: FullpelMv) -> IntMv {
        IntMv {
            as_int: (mv.row as u16 as u32) | ((mv.col as u16 as u32) << 16),
        }
    }

    /// `.as_mv = mv`
    #[inline]
    pub(crate) fn set_mv(&mut self, mv: Mv) {
        *self = IntMv::from_mv(mv);
    }

    /// `.as_fullmv = mv`
    #[inline]
    pub(crate) fn set_fullmv(&mut self, mv: FullpelMv) {
        *self = IntMv::from_fullmv(mv);
    }
}

/// `MV32`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Mv32 {
    pub(crate) row: i32,
    pub(crate) col: i32,
}

// The mv limit for fullpel mvs
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FullMvLimits {
    pub(crate) col_min: i32,
    pub(crate) col_max: i32,
    pub(crate) row_min: i32,
    pub(crate) row_max: i32,
}

// The mv limit for subpel mvs
/// `SubpelMvLimits`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SubpelMvLimits {
    pub(crate) col_min: i32,
    pub(crate) col_max: i32,
    pub(crate) row_min: i32,
    pub(crate) row_max: i32,
}

/// Translation of `get_fullmv_from_mv()`.
#[inline]
pub(crate) const fn get_fullmv_from_mv(subpel_mv: &Mv) -> FullpelMv {
    FullpelMv {
        row: get_mv_rawpel(subpel_mv.row as i32) as i16,
        col: get_mv_rawpel(subpel_mv.col as i32) as i16,
    }
}

/// Translation of `get_mv_from_fullmv()`.
#[inline]
pub(crate) const fn get_mv_from_fullmv(full_mv: &FullpelMv) -> Mv {
    Mv {
        row: get_mv_subpel(full_mv.row as i32) as i16,
        col: get_mv_subpel(full_mv.col as i32) as i16,
    }
}

/// Translation of `convert_fullmv_to_mv()`.
#[inline]
pub(crate) fn convert_fullmv_to_mv(mv: &mut IntMv) {
    let m = get_mv_from_fullmv(&mv.as_fullmv());
    mv.set_mv(m);
}

// Bits of precision used for the model
pub(crate) const WARPEDMODEL_PREC_BITS: i32 = 16;
pub(crate) const GM_TRANS_PREC_BITS: i32 = 6;
pub(crate) const GM_ABS_TRANS_BITS: i32 = 12;
pub(crate) const GM_ABS_TRANS_ONLY_BITS: i32 = GM_ABS_TRANS_BITS - GM_TRANS_PREC_BITS + 3;
pub(crate) const GM_TRANS_PREC_DIFF: i32 = WARPEDMODEL_PREC_BITS - GM_TRANS_PREC_BITS;
pub(crate) const GM_TRANS_ONLY_PREC_DIFF: i32 = WARPEDMODEL_PREC_BITS - 3;

/// `TransformationType`
pub(crate) type TransformationType = u8;
pub(crate) const IDENTITY: TransformationType = 0; // identity transformation, 0-parameter
pub(crate) const TRANSLATION: TransformationType = 1; // translational motion 2-parameter
pub(crate) const ROTZOOM: TransformationType = 2; // simplified affine with rotation + zoom only, 4-parameter
pub(crate) const AFFINE: TransformationType = 3; // affine, 6-parameter

// The order of values in the wmmat matrix below is best described
// by the homography:
//      [x'     (m2 m3 m0   [x
//  z .  y'  =   m4 m5 m1 *  y
//       1]      m6 m7 1)    1]
/// `WarpedMotionParams`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WarpedMotionParams {
    pub(crate) wmmat: [i32; 6],
    pub(crate) alpha: i16,
    pub(crate) beta: i16,
    pub(crate) gamma: i16,
    pub(crate) delta: i16,
    pub(crate) wmtype: TransformationType,
    pub(crate) invalid: i8,
}

/// `default_warp_params`
pub(crate) const DEFAULT_WARP_PARAMS: WarpedMotionParams = WarpedMotionParams {
    wmmat: [
        0,
        0,
        1 << WARPEDMODEL_PREC_BITS,
        0,
        0,
        1 << WARPEDMODEL_PREC_BITS,
    ],
    alpha: 0,
    beta: 0,
    gamma: 0,
    delta: 0,
    wmtype: IDENTITY,
    invalid: 0,
};

impl Default for WarpedMotionParams {
    fn default() -> Self {
        DEFAULT_WARP_PARAMS
    }
}

#[inline]
fn block_center_x(mi_col: i32, bs: BlockSize) -> i32 {
    let bw = BLOCK_SIZE_WIDE[bs as usize] as i32;
    mi_col * MI_SIZE + bw / 2 - 1
}

#[inline]
fn block_center_y(mi_row: i32, bs: BlockSize) -> i32 {
    let bh = BLOCK_SIZE_HIGH[bs as usize] as i32;
    mi_row * MI_SIZE + bh / 2 - 1
}

#[inline]
fn round_power_of_two_signed(value: i32, n: i32) -> i32 {
    if value < 0 {
        -((-value + ((1 << n) >> 1)) >> n)
    } else {
        (value + ((1 << n) >> 1)) >> n
    }
}

#[inline]
fn convert_to_trans_prec(allow_hp: bool, coor: i32) -> i32 {
    if allow_hp {
        round_power_of_two_signed(coor, WARPEDMODEL_PREC_BITS - 3)
    } else {
        round_power_of_two_signed(coor, WARPEDMODEL_PREC_BITS - 2) * 2
    }
}

/// Translation of `integer_mv_precision()`.
#[inline]
pub(crate) fn integer_mv_precision(mv: &mut Mv) {
    let mut m = mv.row % 8;
    if m != 0 {
        mv.row -= m;
        if m.abs() > 4 {
            if m > 0 {
                mv.row += 8;
            } else {
                mv.row -= 8;
            }
        }
    }

    m = mv.col % 8;
    if m != 0 {
        mv.col -= m;
        if m.abs() > 4 {
            if m > 0 {
                mv.col += 8;
            } else {
                mv.col -= 8;
            }
        }
    }
}

// Convert a global motion vector into a motion vector at the centre of the
// given block.
//
// The resulting motion vector will have three fractional bits of precision. If
// allow_hp is zero, the bottom bit will always be zero. If CONFIG_AMVR and
// is_integer is true, the bottom three bits will be zero (so the motion vector
// represents an integer)
/// Translation of `gm_get_motion_vector()`.
#[inline]
pub(crate) fn gm_get_motion_vector(
    gm: &WarpedMotionParams,
    allow_hp: bool,
    bsize: BlockSize,
    mi_col: i32,
    mi_row: i32,
    is_integer: bool,
) -> IntMv {
    if gm.wmtype == IDENTITY {
        return IntMv { as_int: 0 };
    }
    let mat = &gm.wmmat;
    if gm.wmtype == TRANSLATION {
        // (see upstream's note on the AV1 specification's swapped
        // translation, https://crbug.com/aomedia/3328)
        let mut mv = Mv {
            row: (gm.wmmat[0] >> GM_TRANS_ONLY_PREC_DIFF) as i16,
            col: (gm.wmmat[1] >> GM_TRANS_ONLY_PREC_DIFF) as i16,
        };
        if is_integer {
            integer_mv_precision(&mut mv);
        }
        return IntMv::from_mv(mv);
    }

    let x = block_center_x(mi_col, bsize);
    let y = block_center_y(mi_row, bsize);

    let xc = (mat[2] - (1 << WARPEDMODEL_PREC_BITS))
        .wrapping_mul(x)
        .wrapping_add(mat[3].wrapping_mul(y))
        .wrapping_add(mat[0]);
    let yc = mat[4]
        .wrapping_mul(x)
        .wrapping_add((mat[5] - (1 << WARPEDMODEL_PREC_BITS)).wrapping_mul(y))
        .wrapping_add(mat[1]);
    let tx = convert_to_trans_prec(allow_hp, xc);
    let ty = convert_to_trans_prec(allow_hp, yc);

    let mut mv = Mv {
        row: ty as i16,
        col: tx as i16,
    };

    if is_integer {
        integer_mv_precision(&mut mv);
    }
    IntMv::from_mv(mv)
}
