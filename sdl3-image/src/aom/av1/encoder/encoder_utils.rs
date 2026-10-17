// Rust translation of the parts of av1/encoder/encoder_utils.c and
// av1/encoder/encoder_utils.h from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches.
// Copyright (c) 2020, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Encoder helpers: mode info sizing, frame setup and the per-frame
//! encoder state resets.

use crate::aom::aom_dsp::aom_dsp_common::{align_power_of_two, round_power_of_two};
use crate::aom::av1::common::alloccommon::calc_mi_size;
use crate::aom::av1::common::av1_common_int::CommonModeInfoParams;
use crate::aom::av1::common::enums::*;

#[inline]
pub(crate) fn size_in_mi(n: i32) -> i32 {
    align_power_of_two(n, 3) >> MI_SIZE_LOG2
}

#[inline]
pub(crate) fn set_mb_mi(mi_params: &mut CommonModeInfoParams, width: i32, height: i32) {
    mi_params.mi_cols = size_in_mi(width);
    mi_params.mi_rows = size_in_mi(height);
    mi_params.mi_stride = calc_mi_size(mi_params.mi_cols);

    mi_params.mb_cols = round_power_of_two(mi_params.mi_cols, 2);
    mi_params.mb_rows = round_power_of_two(mi_params.mi_rows, 2);
    mi_params.mbs = mi_params.mb_rows * mi_params.mb_cols;

    let mi_alloc_size_1d = MI_SIZE_WIDE[mi_params.mi_alloc_bsize as usize] as i32;
    mi_params.mi_alloc_stride = (mi_params.mi_stride + mi_alloc_size_1d - 1) / mi_alloc_size_1d;

    debug_assert!(
        MI_SIZE_WIDE[mi_params.mi_alloc_bsize as usize]
            == MI_SIZE_HIGH[mi_params.mi_alloc_bsize as usize]
    );
}

#[inline]
pub(crate) fn enc_free_mi(mi_params: &mut CommonModeInfoParams) {
    mi_params.mi_alloc = Vec::new();
    mi_params.mi_grid_base = Vec::new();
    mi_params.mi_alloc_size = 0;
    mi_params.tx_type_map = Vec::new();
}

#[inline]
pub(crate) fn enc_set_mb_mi(
    mi_params: &mut CommonModeInfoParams,
    width: i32,
    height: i32,
    min_partition_size: BlockSize,
) {
    mi_params.mi_alloc_bsize = min_partition_size;

    set_mb_mi(mi_params, width, height);
}
