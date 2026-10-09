// Rust translation of src/levels.h from dav1d (https://code.videolan.org/videolan/dav1d,
// at the revision SDL_image's external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The enumerations of the block-level syntax (transform sizes and types,
//! block levels, sizes and partitions, prediction modes, filters), the
//! motion vector type and the per-block info (`Av1Block`).
//!
//! The C enums are used as integers throughout dav1d (indices, arithmetic,
//! bitmasks), so they are plain integer constants here, as in C.

#![allow(dead_code)]

// enum ObuMetaType
pub(crate) const OBU_META_HDR_CLL: u32 = 1;
pub(crate) const OBU_META_HDR_MDCV: u32 = 2;
pub(crate) const OBU_META_SCALABILITY: u32 = 3;
pub(crate) const OBU_META_ITUT_T35: u32 = 4;
pub(crate) const OBU_META_TIMECODE: u32 = 5;

// enum TxfmSize
pub(crate) const TX_4X4: u8 = 0;
pub(crate) const TX_8X8: u8 = 1;
pub(crate) const TX_16X16: u8 = 2;
pub(crate) const TX_32X32: u8 = 3;
pub(crate) const TX_64X64: u8 = 4;
pub(crate) const N_TX_SIZES: usize = 5;

// enum BlockLevel
pub(crate) const BL_128X128: u8 = 0;
pub(crate) const BL_64X64: u8 = 1;
pub(crate) const BL_32X32: u8 = 2;
pub(crate) const BL_16X16: u8 = 3;
pub(crate) const BL_8X8: u8 = 4;
pub(crate) const N_BL_LEVELS: usize = 5;

// enum RectTxfmSize
pub(crate) const RTX_4X8: u8 = N_TX_SIZES as u8;
pub(crate) const RTX_8X4: u8 = 6;
pub(crate) const RTX_8X16: u8 = 7;
pub(crate) const RTX_16X8: u8 = 8;
pub(crate) const RTX_16X32: u8 = 9;
pub(crate) const RTX_32X16: u8 = 10;
pub(crate) const RTX_32X64: u8 = 11;
pub(crate) const RTX_64X32: u8 = 12;
pub(crate) const RTX_4X16: u8 = 13;
pub(crate) const RTX_16X4: u8 = 14;
pub(crate) const RTX_8X32: u8 = 15;
pub(crate) const RTX_32X8: u8 = 16;
pub(crate) const RTX_16X64: u8 = 17;
pub(crate) const RTX_64X16: u8 = 18;
pub(crate) const N_RECT_TX_SIZES: usize = 19;

// enum TxfmType
pub(crate) const DCT_DCT: u8 = 0; // DCT  in both horizontal and vertical
pub(crate) const ADST_DCT: u8 = 1; // ADST in vertical, DCT in horizontal
pub(crate) const DCT_ADST: u8 = 2; // DCT  in vertical, ADST in horizontal
pub(crate) const ADST_ADST: u8 = 3; // ADST in both directions
pub(crate) const FLIPADST_DCT: u8 = 4;
pub(crate) const DCT_FLIPADST: u8 = 5;
pub(crate) const FLIPADST_FLIPADST: u8 = 6;
pub(crate) const ADST_FLIPADST: u8 = 7;
pub(crate) const FLIPADST_ADST: u8 = 8;
pub(crate) const IDTX: u8 = 9;
pub(crate) const V_DCT: u8 = 10;
pub(crate) const H_DCT: u8 = 11;
pub(crate) const V_ADST: u8 = 12;
pub(crate) const H_ADST: u8 = 13;
pub(crate) const V_FLIPADST: u8 = 14;
pub(crate) const H_FLIPADST: u8 = 15;
pub(crate) const N_TX_TYPES: usize = 16;
pub(crate) const WHT_WHT: u8 = N_TX_TYPES as u8;
pub(crate) const N_TX_TYPES_PLUS_LL: usize = 17;

// enum TxClass
pub(crate) const TX_CLASS_2D: u8 = 0;
pub(crate) const TX_CLASS_H: u8 = 1;
pub(crate) const TX_CLASS_V: u8 = 2;

// enum IntraPredMode
pub(crate) const DC_PRED: u8 = 0;
pub(crate) const VERT_PRED: u8 = 1;
pub(crate) const HOR_PRED: u8 = 2;
pub(crate) const DIAG_DOWN_LEFT_PRED: u8 = 3;
pub(crate) const DIAG_DOWN_RIGHT_PRED: u8 = 4;
pub(crate) const VERT_RIGHT_PRED: u8 = 5;
pub(crate) const HOR_DOWN_PRED: u8 = 6;
pub(crate) const HOR_UP_PRED: u8 = 7;
pub(crate) const VERT_LEFT_PRED: u8 = 8;
pub(crate) const SMOOTH_PRED: u8 = 9;
pub(crate) const SMOOTH_V_PRED: u8 = 10;
pub(crate) const SMOOTH_H_PRED: u8 = 11;
pub(crate) const PAETH_PRED: u8 = 12;
pub(crate) const N_INTRA_PRED_MODES: usize = 13;
pub(crate) const CFL_PRED: u8 = N_INTRA_PRED_MODES as u8;
pub(crate) const N_UV_INTRA_PRED_MODES: usize = 14;
pub(crate) const N_IMPL_INTRA_PRED_MODES: usize = N_UV_INTRA_PRED_MODES;
pub(crate) const LEFT_DC_PRED: u8 = DIAG_DOWN_LEFT_PRED;
pub(crate) const TOP_DC_PRED: u8 = 4;
pub(crate) const DC_128_PRED: u8 = 5;
pub(crate) const Z1_PRED: u8 = 6;
pub(crate) const Z2_PRED: u8 = 7;
pub(crate) const Z3_PRED: u8 = 8;
pub(crate) const FILTER_PRED: u8 = N_INTRA_PRED_MODES as u8;

// enum InterIntraPredMode
pub(crate) const II_DC_PRED: u8 = 0;
pub(crate) const II_VERT_PRED: u8 = 1;
pub(crate) const II_HOR_PRED: u8 = 2;
pub(crate) const II_SMOOTH_PRED: u8 = 3;
pub(crate) const N_INTER_INTRA_PRED_MODES: usize = 4;

// enum BlockPartition
pub(crate) const PARTITION_NONE: u8 = 0; // [ ] <-.
pub(crate) const PARTITION_H: u8 = 1; // [-]   |
pub(crate) const PARTITION_V: u8 = 2; // [|]   |
pub(crate) const PARTITION_SPLIT: u8 = 3; // [+] --'
pub(crate) const PARTITION_T_TOP_SPLIT: u8 = 4; // [⊥] i.e. split top, H bottom
pub(crate) const PARTITION_T_BOTTOM_SPLIT: u8 = 5; // [т] i.e. H top, split bottom
pub(crate) const PARTITION_T_LEFT_SPLIT: u8 = 6; // [-|] i.e. split left, V right
pub(crate) const PARTITION_T_RIGHT_SPLIT: u8 = 7; // [|-] i.e. V left, split right
pub(crate) const PARTITION_H4: u8 = 8; // [Ⲷ]
pub(crate) const PARTITION_V4: u8 = 9; // [Ⲽ]
pub(crate) const N_PARTITIONS: usize = 10;
pub(crate) const N_SUB8X8_PARTITIONS: usize = PARTITION_T_TOP_SPLIT as usize;

// enum BlockSize
pub(crate) const BS_128X128: u8 = 0;
pub(crate) const BS_128X64: u8 = 1;
pub(crate) const BS_64X128: u8 = 2;
pub(crate) const BS_64X64: u8 = 3;
pub(crate) const BS_64X32: u8 = 4;
pub(crate) const BS_64X16: u8 = 5;
pub(crate) const BS_32X64: u8 = 6;
pub(crate) const BS_32X32: u8 = 7;
pub(crate) const BS_32X16: u8 = 8;
pub(crate) const BS_32X8: u8 = 9;
pub(crate) const BS_16X64: u8 = 10;
pub(crate) const BS_16X32: u8 = 11;
pub(crate) const BS_16X16: u8 = 12;
pub(crate) const BS_16X8: u8 = 13;
pub(crate) const BS_16X4: u8 = 14;
pub(crate) const BS_8X32: u8 = 15;
pub(crate) const BS_8X16: u8 = 16;
pub(crate) const BS_8X8: u8 = 17;
pub(crate) const BS_8X4: u8 = 18;
pub(crate) const BS_4X16: u8 = 19;
pub(crate) const BS_4X8: u8 = 20;
pub(crate) const BS_4X4: u8 = 21;
pub(crate) const N_BS_SIZES: usize = 22;

// enum Filter2d (order is horizontal, vertical)
pub(crate) const FILTER_2D_8TAP_REGULAR: u8 = 0;
pub(crate) const FILTER_2D_8TAP_REGULAR_SMOOTH: u8 = 1;
pub(crate) const FILTER_2D_8TAP_REGULAR_SHARP: u8 = 2;
pub(crate) const FILTER_2D_8TAP_SHARP_REGULAR: u8 = 3;
pub(crate) const FILTER_2D_8TAP_SHARP_SMOOTH: u8 = 4;
pub(crate) const FILTER_2D_8TAP_SHARP: u8 = 5;
pub(crate) const FILTER_2D_8TAP_SMOOTH_REGULAR: u8 = 6;
pub(crate) const FILTER_2D_8TAP_SMOOTH: u8 = 7;
pub(crate) const FILTER_2D_8TAP_SMOOTH_SHARP: u8 = 8;
pub(crate) const FILTER_2D_BILINEAR: u8 = 9;
pub(crate) const N_2D_FILTERS: usize = 10;

// enum MVJoint
pub(crate) const MV_JOINT_ZERO: u32 = 0;
pub(crate) const MV_JOINT_H: u32 = 1;
pub(crate) const MV_JOINT_V: u32 = 2;
pub(crate) const MV_JOINT_HV: u32 = 3;
pub(crate) const N_MV_JOINTS: usize = 4;

// enum InterPredMode
pub(crate) const NEARESTMV: u8 = 0;
pub(crate) const NEARMV: u8 = 1;
pub(crate) const GLOBALMV: u8 = 2;
pub(crate) const NEWMV: u8 = 3;
pub(crate) const N_INTER_PRED_MODES: usize = 4;

// enum DRL_PROXIMITY
pub(crate) const NEAREST_DRL: u8 = 0;
pub(crate) const NEARER_DRL: u8 = 1;
pub(crate) const NEAR_DRL: u8 = 2;
pub(crate) const NEARISH_DRL: u8 = 3;

// enum CompInterPredMode
pub(crate) const NEARESTMV_NEARESTMV: u8 = 0;
pub(crate) const NEARMV_NEARMV: u8 = 1;
pub(crate) const NEARESTMV_NEWMV: u8 = 2;
pub(crate) const NEWMV_NEARESTMV: u8 = 3;
pub(crate) const NEARMV_NEWMV: u8 = 4;
pub(crate) const NEWMV_NEARMV: u8 = 5;
pub(crate) const GLOBALMV_GLOBALMV: u8 = 6;
pub(crate) const NEWMV_NEWMV: u8 = 7;
pub(crate) const N_COMP_INTER_PRED_MODES: usize = 8;

// enum CompInterType
pub(crate) const COMP_INTER_NONE: u8 = 0;
pub(crate) const COMP_INTER_WEIGHTED_AVG: u8 = 1;
pub(crate) const COMP_INTER_AVG: u8 = 2;
pub(crate) const COMP_INTER_SEG: u8 = 3;
pub(crate) const COMP_INTER_WEDGE: u8 = 4;

// enum InterIntraType
pub(crate) const INTER_INTRA_NONE: u8 = 0;
pub(crate) const INTER_INTRA_BLEND: u8 = 1;
pub(crate) const INTER_INTRA_WEDGE: u8 = 2;

/// Translation of `union mv`: a motion vector in 1/8 pel units. The C
/// union's `n` view (the two components as one 32-bit word) is used for
/// whole-vector comparisons and the `INVALID_MV` marker; [`Mv::n`]
/// computes it as on the little-endian targets dav1d's layout assumes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mv {
    pub(crate) y: i16,
    pub(crate) x: i16,
}

impl Mv {
    pub(crate) const ZERO: Mv = Mv { y: 0, x: 0 };

    /// The `n` member of `union mv`.
    pub(crate) fn n(self) -> u32 {
        (self.y as u16 as u32) | ((self.x as u16 as u32) << 16)
    }

    /// Build a vector from the `n` member of `union mv`.
    pub(crate) fn from_n(n: u32) -> Mv {
        Mv {
            y: n as u16 as i16,
            x: (n >> 16) as u16 as i16,
        }
    }
}

// enum MotionMode
pub(crate) const MM_TRANSLATION: u8 = 0;
pub(crate) const MM_OBMC: u8 = 1;
pub(crate) const MM_WARP: u8 = 2;

pub(crate) const QINDEX_RANGE: usize = 256;

/// Translation of `Av1Block`, the decoded mode info of one block. The C
/// struct keeps the intra and inter fields in a union; they are separate
/// fields here (no field is read through the other member of the union).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Av1Block {
    pub(crate) bl: u8,
    pub(crate) bs: u8,
    pub(crate) bp: u8,
    pub(crate) intra: u8,
    pub(crate) seg_id: u8,
    pub(crate) skip_mode: u8,
    pub(crate) skip: u8,
    pub(crate) uvtx: u8,
    // intra
    pub(crate) y_mode: u8,
    pub(crate) uv_mode: u8,
    pub(crate) tx: u8,
    pub(crate) pal_sz: [u8; 2],
    pub(crate) y_angle: i8,
    pub(crate) uv_angle: i8,
    pub(crate) cfl_alpha: [i8; 2],
    // inter
    pub(crate) mv: [Mv; 2],
    pub(crate) wedge_idx: u8,
    pub(crate) mask_sign: u8,
    pub(crate) interintra_mode: u8,
    pub(crate) comp_type: u8,
    pub(crate) inter_mode: u8,
    pub(crate) motion_mode: u8,
    pub(crate) drl_idx: u8,
    pub(crate) ref_: [i8; 2],
    pub(crate) max_ytx: u8,
    pub(crate) filter2d: u8,
    pub(crate) interintra_type: u8,
    pub(crate) tx_split0: u8,
    pub(crate) tx_split1: u16,
}
