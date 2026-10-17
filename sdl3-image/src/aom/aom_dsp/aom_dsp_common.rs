// Rust translation of aom_dsp/aom_dsp_common.h, aom_ports/mem.h and
// aom_ports/bitops.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The helpers every part of aom uses: clamps, the rounding shifts of
//! mem.h (as functions over `i32` and `i64`) and `get_msb()`.

pub(crate) type QmValT = u8;
pub(crate) const AOM_QM_BITS: i32 = 5;

// Note:
// tran_low_t  is the datatype used for final transform coefficients.
// tran_high_t is the datatype used for intermediate transform stages.
pub(crate) type TranHighT = i64;
pub(crate) type TranLowT = i32;

#[inline]
pub(crate) fn clamp(value: i32, low: i32, high: i32) -> i32 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

#[inline]
pub(crate) fn clamp64(value: i64, low: i64, high: i64) -> i64 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

#[inline]
pub(crate) fn fclamp(value: f64, low: f64, high: f64) -> f64 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

#[inline]
pub(crate) fn clip_pixel_highbd(val: i32, bd: i32) -> u16 {
    match bd {
        10 => clamp(val, 0, 1023) as u16,
        12 => clamp(val, 0, 4095) as u16,
        _ => clamp(val, 0, 255) as u16,
    }
}

// The result of this branchless code is equivalent to (value < 0 ? 0 : value)
// or max(0, value) and might be faster in some cases.
// Care should be taken since the behavior of right shifting signed type
// negative value is undefined by C standards and implementation defined,
#[inline]
pub(crate) fn negative_to_zero(value: i32) -> u32 {
    (value & !(value >> 31)) as u32
}

/* Shift down with rounding for use when n >= 0. Usually value >= 0, but the
 * macro can be used with a negative value if the direction of rounding is
 * acceptable.
 */
#[inline]
pub(crate) fn round_power_of_two(value: i32, n: i32) -> i32 {
    (value + ((1 << n) >> 1)) >> n
}

/// `ROUND_POWER_OF_TWO()` over unsigned values.
#[inline]
pub(crate) fn round_power_of_two_u32(value: u32, n: u32) -> u32 {
    (value + ((1u32 << n) >> 1)) >> n
}

/// `ROUND_POWER_OF_TWO()` over 64-bit values (as the macro is used on
/// them, with a 32-bit rounding constant).
#[inline]
pub(crate) fn round_power_of_two_i64(value: i64, n: i32) -> i64 {
    (value + ((1i32 << n) >> 1) as i64) >> n
}

/* Shift down with rounding for signed integers, for use when n >= 0 */
#[inline]
pub(crate) fn round_power_of_two_signed(value: i32, n: i32) -> i32 {
    if value < 0 {
        -round_power_of_two(-value, n)
    } else {
        round_power_of_two(value, n)
    }
}

/* Shift down with rounding for use when n >= 0 (64-bit value). Usually
 * value >= 0, but the macro can be used with a negative value if the direction
 * of rounding is acceptable.
 */
#[inline]
pub(crate) fn round_power_of_two_64(value: i64, n: i32) -> i64 {
    (value + ((1i64 << n) >> 1)) >> n
}

/// `ROUND_POWER_OF_TWO_64()` over unsigned values.
#[inline]
pub(crate) fn round_power_of_two_u64(value: u64, n: i32) -> u64 {
    (value + ((1u64 << n) >> 1)) >> n
}

/* Shift down with rounding for signed integers, for use when n >= 0 (64-bit
 * value)
 */
#[inline]
pub(crate) fn round_power_of_two_signed_64(value: i64, n: i32) -> i64 {
    if value < 0 {
        -round_power_of_two_64(-value, n)
    } else {
        round_power_of_two_64(value, n)
    }
}

/* Shift down with ceil() for use when n >= 0 and value >= 0.*/
#[inline]
pub(crate) fn ceil_power_of_two(value: i32, n: i32) -> i32 {
    (value + (1 << n) - 1) >> n
}

#[inline]
pub(crate) fn align_power_of_two(value: i32, n: i32) -> i32 {
    (value + ((1 << n) - 1)) & !((1 << n) - 1)
}

#[inline]
pub(crate) fn align_power_of_two_unsigned(value: u32, n: u32) -> u32 {
    (value + ((1u32 << n) - 1)) & !((1u32 << n) - 1)
}

// get_msb:
// Returns (int)floor(log2(n)). n must be > 0.
// These versions of get_msb() are only valid when n != 0 because all
// of the optimized versions are undefined when n == 0:
// https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html
#[inline]
pub(crate) fn get_msb(n: u32) -> i32 {
    debug_assert!(n != 0);
    31 ^ n.leading_zeros() as i32
}
