// Rust translation of av1/common/av1_txfm.c and av1/common/av1_txfm.h from
// aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2017, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The transforms' shared pieces: the cosine table, the butterfly
//! `half_btf()`, the 2D configuration and the rounding of arrays. (The
//! saver only reaches the DCTs, so the sine table of the ADSTs is left out;
//! `av1_range_check_buf()` and `range_check_value()` do nothing without
//! `CONFIG_COEFFICIENT_RANGE_CHECKING`, and are left out.)

use super::enums::*;
use crate::aom::aom_dsp::aom_dsp_common::{clamp64, clip_pixel_highbd, TranHighT};

// av1_cospi_arr[i][j] = (int)round(cos(PI*j/128) * (1<<(cos_bit_min+i)));
pub(crate) static AV1_COSPI_ARR_DATA: [[i32; 64]; 7] = [
    [
        1024, 1024, 1023, 1021, 1019, 1016, 1013, 1009, 1004, 999, 993, 987, 980, 972, 964, 955,
        946, 936, 926, 915, 903, 891, 878, 865, 851, 837, 822, 807, 792, 775, 759, 742, 724, 706,
        688, 669, 650, 630, 610, 590, 569, 548, 526, 505, 483, 460, 438, 415, 392, 369, 345, 321,
        297, 273, 249, 224, 200, 175, 150, 125, 100, 75, 50, 25,
    ],
    [
        2048, 2047, 2046, 2042, 2038, 2033, 2026, 2018, 2009, 1998, 1987, 1974, 1960, 1945, 1928,
        1911, 1892, 1872, 1851, 1829, 1806, 1782, 1757, 1730, 1703, 1674, 1645, 1615, 1583, 1551,
        1517, 1483, 1448, 1412, 1375, 1338, 1299, 1260, 1220, 1179, 1138, 1096, 1053, 1009, 965,
        921, 876, 830, 784, 737, 690, 642, 595, 546, 498, 449, 400, 350, 301, 251, 201, 151, 100,
        50,
    ],
    [
        4096, 4095, 4091, 4085, 4076, 4065, 4052, 4036, 4017, 3996, 3973, 3948, 3920, 3889, 3857,
        3822, 3784, 3745, 3703, 3659, 3612, 3564, 3513, 3461, 3406, 3349, 3290, 3229, 3166, 3102,
        3035, 2967, 2896, 2824, 2751, 2675, 2598, 2520, 2440, 2359, 2276, 2191, 2106, 2019, 1931,
        1842, 1751, 1660, 1567, 1474, 1380, 1285, 1189, 1092, 995, 897, 799, 700, 601, 501, 401,
        301, 201, 101,
    ],
    [
        8192, 8190, 8182, 8170, 8153, 8130, 8103, 8071, 8035, 7993, 7946, 7895, 7839, 7779, 7713,
        7643, 7568, 7489, 7405, 7317, 7225, 7128, 7027, 6921, 6811, 6698, 6580, 6458, 6333, 6203,
        6070, 5933, 5793, 5649, 5501, 5351, 5197, 5040, 4880, 4717, 4551, 4383, 4212, 4038, 3862,
        3683, 3503, 3320, 3135, 2948, 2760, 2570, 2378, 2185, 1990, 1795, 1598, 1401, 1202, 1003,
        803, 603, 402, 201,
    ],
    [
        16384, 16379, 16364, 16340, 16305, 16261, 16207, 16143, 16069, 15986, 15893, 15791, 15679,
        15557, 15426, 15286, 15137, 14978, 14811, 14635, 14449, 14256, 14053, 13842, 13623, 13395,
        13160, 12916, 12665, 12406, 12140, 11866, 11585, 11297, 11003, 10702, 10394, 10080, 9760,
        9434, 9102, 8765, 8423, 8076, 7723, 7366, 7005, 6639, 6270, 5897, 5520, 5139, 4756, 4370,
        3981, 3590, 3196, 2801, 2404, 2006, 1606, 1205, 804, 402,
    ],
    [
        32768, 32758, 32729, 32679, 32610, 32522, 32413, 32286, 32138, 31972, 31786, 31581, 31357,
        31114, 30853, 30572, 30274, 29957, 29622, 29269, 28899, 28511, 28106, 27684, 27246, 26791,
        26320, 25833, 25330, 24812, 24279, 23732, 23170, 22595, 22006, 21403, 20788, 20160, 19520,
        18868, 18205, 17531, 16846, 16151, 15447, 14733, 14010, 13279, 12540, 11793, 11039, 10279,
        9512, 8740, 7962, 7180, 6393, 5602, 4808, 4011, 3212, 2411, 1608, 804,
    ],
    [
        65536, 65516, 65457, 65358, 65220, 65043, 64827, 64571, 64277, 63944, 63572, 63162, 62714,
        62228, 61705, 61145, 60547, 59914, 59244, 58538, 57798, 57022, 56212, 55368, 54491, 53581,
        52639, 51665, 50660, 49624, 48559, 47464, 46341, 45190, 44011, 42806, 41576, 40320, 39040,
        37736, 36410, 35062, 33692, 32303, 30893, 29466, 28020, 26558, 25080, 23586, 22078, 20557,
        19024, 17479, 15924, 14359, 12785, 11204, 9616, 8022, 6424, 4821, 3216, 1608,
    ],
];

pub(crate) const MAX_TXFM_STAGE_NUM: usize = 12;

const COS_BIT_MIN: i32 = 10;

pub(crate) const NEW_SQRT2_BITS: i32 = 12;
// 2^12 * sqrt(2)
pub(crate) const NEW_SQRT2: i32 = 5793;
// 2^12 / sqrt(2)
pub(crate) const NEW_INV_SQRT2: i32 = 2896;

#[inline]
pub(crate) fn cospi_arr(n: i32) -> &'static [i32; 64] {
    &AV1_COSPI_ARR_DATA[(n - COS_BIT_MIN) as usize]
}

#[inline]
pub(crate) fn round_shift(value: i64, bit: i32) -> i32 {
    debug_assert!(bit >= 1);
    ((value + (1i64 << (bit - 1))) >> bit) as i32
}

/// Translation of `half_btf()`. The products are C `int` products; the
/// translation wraps them as the C compilers do.
#[inline]
pub(crate) fn half_btf(w0: i32, in0: i32, w1: i32, in1: i32, bit: i8) -> i32 {
    let result_64 = w0.wrapping_mul(in0) as i64 + w1.wrapping_mul(in1) as i64;
    let intermediate = result_64 + (1i64 << (bit - 1));
    // NOTE(rachelbarker): The value 'result_64' may not necessarily fit
    // into 32 bits. However, the result of this function is nominally
    // ROUND_POWER_OF_TWO_64(result_64, bit)
    // and that is required to fit into stage_range[stage] many bits
    // (checked by range_check_buf()).
    //
    // Here we've unpacked that rounding operation, and it can be shown
    // that the value of 'intermediate' here *does* fit into 32 bits
    // for any conformant bitstream.
    // The upshot is that, if you do all this calculation using
    // wrapping 32-bit arithmetic instead of (non-wrapping) 64-bit arithmetic,
    // then you'll still get the correct result.
    // To provide a check on this logic, we assert that 'intermediate'
    // would fit into an int32 if range checking is enabled.
    (intermediate >> bit) as i32
}

#[inline]
pub(crate) fn highbd_clip_pixel_add(dest: u16, trans: TranHighT, bd: i32) -> u16 {
    clip_pixel_highbd((dest as i32).wrapping_add(trans as i32), bd)
}

pub(crate) type TxfmFunc = fn(&[i32], &mut [i32], i8, &[i8]);

// (TXFM_TYPE)
pub(crate) type TxfmType = i32;
pub(crate) const TXFM_TYPE_DCT4: TxfmType = 0;
pub(crate) const TXFM_TYPE_DCT8: TxfmType = 1;
pub(crate) const TXFM_TYPE_DCT16: TxfmType = 2;
pub(crate) const TXFM_TYPE_DCT32: TxfmType = 3;
pub(crate) const TXFM_TYPE_DCT64: TxfmType = 4;
pub(crate) const TXFM_TYPE_ADST4: TxfmType = 5;
pub(crate) const TXFM_TYPE_ADST8: TxfmType = 6;
pub(crate) const TXFM_TYPE_ADST16: TxfmType = 7;
pub(crate) const TXFM_TYPE_IDENTITY4: TxfmType = 8;
pub(crate) const TXFM_TYPE_IDENTITY8: TxfmType = 9;
pub(crate) const TXFM_TYPE_IDENTITY16: TxfmType = 10;
pub(crate) const TXFM_TYPE_IDENTITY32: TxfmType = 11;
pub(crate) const TXFM_TYPES: TxfmType = 12;
pub(crate) const TXFM_TYPE_INVALID: TxfmType = 13;

#[derive(Clone, Copy, Default)]
pub(crate) struct Txfm2dFlipCfg {
    pub(crate) tx_size: TxSize,
    pub(crate) ud_flip: i32, // flip upside down
    pub(crate) lr_flip: i32, // flip left to right
    pub(crate) shift: &'static [i8],
    pub(crate) cos_bit_col: i8,
    pub(crate) cos_bit_row: i8,
    pub(crate) stage_range_col: [i8; MAX_TXFM_STAGE_NUM],
    pub(crate) stage_range_row: [i8; MAX_TXFM_STAGE_NUM],
    pub(crate) txfm_type_col: TxfmType,
    pub(crate) txfm_type_row: TxfmType,
    pub(crate) stage_num_col: i32,
    pub(crate) stage_num_row: i32,
}

#[inline]
pub(crate) fn get_flip_cfg(tx_type: TxType, ud_flip: &mut i32, lr_flip: &mut i32) {
    match tx_type {
        DCT_DCT | ADST_DCT | DCT_ADST | ADST_ADST => {
            *ud_flip = 0;
            *lr_flip = 0;
        }
        IDTX | V_DCT | H_DCT | V_ADST | H_ADST => {
            *ud_flip = 0;
            *lr_flip = 0;
        }
        FLIPADST_DCT | FLIPADST_ADST | V_FLIPADST => {
            *ud_flip = 1;
            *lr_flip = 0;
        }
        DCT_FLIPADST | ADST_FLIPADST | H_FLIPADST => {
            *ud_flip = 0;
            *lr_flip = 1;
        }
        FLIPADST_FLIPADST => {
            *ud_flip = 1;
            *lr_flip = 1;
        }
        _ => {
            *ud_flip = 0;
            *lr_flip = 0;
            debug_assert!(false);
        }
    }
}

#[inline]
pub(crate) fn set_flip_cfg(tx_type: TxType, cfg: &mut Txfm2dFlipCfg) {
    get_flip_cfg(tx_type, &mut cfg.ud_flip, &mut cfg.lr_flip);
}

// Utility function that returns the log of the ratio of the col and row
// sizes.
#[inline]
pub(crate) fn get_rect_tx_log_ratio(col: i32, row: i32) -> i32 {
    if col == row {
        return 0;
    }
    if col > row {
        if col == row * 2 {
            return 1;
        }
        if col == row * 4 {
            return 2;
        }
        debug_assert!(false, "Unsupported transform size");
    } else {
        if row == col * 2 {
            return -1;
        }
        if row == col * 4 {
            return -2;
        }
        debug_assert!(false, "Unsupported transform size");
    }
    0 // Invalid
}

#[inline]
pub(crate) fn get_txw_idx(tx_size: TxSize) -> usize {
    (TX_SIZE_WIDE_LOG2[tx_size as usize] - TX_SIZE_WIDE_LOG2[0]) as usize
}
#[inline]
pub(crate) fn get_txh_idx(tx_size: TxSize) -> usize {
    (TX_SIZE_HIGH_LOG2[tx_size as usize] - TX_SIZE_HIGH_LOG2[0]) as usize
}

pub(crate) const MAX_TXWH_IDX: usize = 5;

// (av1_txfm.c)

pub(crate) fn av1_round_shift_array(arr: &mut [i32], size: i32, bit: i32) {
    if bit == 0 {
        return;
    } else {
        if bit > 0 {
            for i in 0..size as usize {
                arr[i] = round_shift(arr[i] as i64, bit);
            }
        } else {
            for i in 0..size as usize {
                arr[i] = clamp64(
                    (1i64 << (-bit)) * arr[i] as i64,
                    i32::MIN as i64,
                    i32::MAX as i64,
                ) as i32;
            }
        }
    }
}

pub(crate) static AV1_TXFM_TYPE_LS: [[TxfmType; TX_TYPES_1D as usize]; 5] = [
    [
        TXFM_TYPE_DCT4,
        TXFM_TYPE_ADST4,
        TXFM_TYPE_ADST4,
        TXFM_TYPE_IDENTITY4,
    ],
    [
        TXFM_TYPE_DCT8,
        TXFM_TYPE_ADST8,
        TXFM_TYPE_ADST8,
        TXFM_TYPE_IDENTITY8,
    ],
    [
        TXFM_TYPE_DCT16,
        TXFM_TYPE_ADST16,
        TXFM_TYPE_ADST16,
        TXFM_TYPE_IDENTITY16,
    ],
    [
        TXFM_TYPE_DCT32,
        TXFM_TYPE_INVALID,
        TXFM_TYPE_INVALID,
        TXFM_TYPE_IDENTITY32,
    ],
    [
        TXFM_TYPE_DCT64,
        TXFM_TYPE_INVALID,
        TXFM_TYPE_INVALID,
        TXFM_TYPE_INVALID,
    ],
];

pub(crate) static AV1_TXFM_STAGE_NUM_LIST: [i8; TXFM_TYPES as usize] = [
    4,  // TXFM_TYPE_DCT4
    6,  // TXFM_TYPE_DCT8
    8,  // TXFM_TYPE_DCT16
    10, // TXFM_TYPE_DCT32
    12, // TXFM_TYPE_DCT64
    7,  // TXFM_TYPE_ADST4
    8,  // TXFM_TYPE_ADST8
    10, // TXFM_TYPE_ADST16
    1,  // TXFM_TYPE_IDENTITY4
    1,  // TXFM_TYPE_IDENTITY8
    1,  // TXFM_TYPE_IDENTITY16
    1,  // TXFM_TYPE_IDENTITY32
];
