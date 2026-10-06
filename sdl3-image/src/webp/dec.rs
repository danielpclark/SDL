// Rust translation of src/dec/common_dec.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2015 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The decoder: definitions and macros common to encoding and decoding,
//! and the decoder's files.

#![allow(dead_code)] // (the header's whole constant set)

pub(crate) mod alpha_dec;
pub(crate) mod buffer_dec;
pub(crate) mod frame_dec;
pub(crate) mod io_dec;
pub(crate) mod quant_dec;
pub(crate) mod tree_dec;
pub(crate) mod vp8_dec;
pub(crate) mod vp8l_dec;
pub(crate) mod webp_dec;

// intra prediction modes
// 4x4 modes
pub(crate) const B_DC_PRED: i32 = 0;
pub(crate) const B_TM_PRED: i32 = 1;
pub(crate) const B_VE_PRED: i32 = 2;
pub(crate) const B_HE_PRED: i32 = 3;
pub(crate) const B_RD_PRED: i32 = 4;
pub(crate) const B_VR_PRED: i32 = 5;
pub(crate) const B_LD_PRED: i32 = 6;
pub(crate) const B_VL_PRED: i32 = 7;
pub(crate) const B_HD_PRED: i32 = 8;
pub(crate) const B_HU_PRED: i32 = 9;
/// = 10
pub(crate) const NUM_BMODES: i32 = B_HU_PRED + 1 - B_DC_PRED;

// Luma16 or UV modes
pub(crate) const DC_PRED: i32 = B_DC_PRED;
pub(crate) const V_PRED: i32 = B_VE_PRED;
pub(crate) const H_PRED: i32 = B_HE_PRED;
pub(crate) const TM_PRED: i32 = B_TM_PRED;
/// refined I4x4 mode
pub(crate) const B_PRED: i32 = NUM_BMODES;
pub(crate) const NUM_PRED_MODES: i32 = 4;

// special modes
pub(crate) const B_DC_PRED_NOTOP: i32 = 4;
pub(crate) const B_DC_PRED_NOLEFT: i32 = 5;
pub(crate) const B_DC_PRED_NOTOPLEFT: i32 = 6;
pub(crate) const NUM_B_DC_MODES: i32 = 7;

pub(crate) const MB_FEATURE_TREE_PROBS: usize = 3;
pub(crate) const NUM_MB_SEGMENTS: usize = 4;
pub(crate) const NUM_REF_LF_DELTAS: usize = 4;
/// I4x4, ZERO, *, SPLIT
pub(crate) const NUM_MODE_LF_DELTAS: usize = 4;
pub(crate) const MAX_NUM_PARTITIONS: usize = 8;
// Probabilities
/// 0: i16-AC,  1: i16-DC,  2:chroma-AC,  3:i4-AC
pub(crate) const NUM_TYPES: usize = 4;
pub(crate) const NUM_BANDS: usize = 8;
pub(crate) const NUM_CTX: usize = 3;
pub(crate) const NUM_PROBAS: usize = 11;
