// Rust translation of silk/typedef.h, silk/SigProc_FIX.h, silk/macros.h and
// silk/Inlines.h from libopus 1.4 (https://opus-codec.org, at the revision
// SDL_mixer's external/opus pins).
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SILK's fixed-point macros and inline functions, as functions (the
//! generic C versions, with `OPUS_FAST_INT64`, as a 64-bit build has it).
//! C's `int` arithmetic, which wraps in practice where it overflows (only
//! with corrupt streams), wraps here too; shifts by the width of the type
//! or more shift by the count modulo the width, as x86 does. A division by
//! zero (which would trap in C, and only corrupt streams could ask for)
//! gives 0. `silk_assert()` is off, as in upstream's release build.

#![allow(dead_code)]

pub(crate) const SILK_MAX_ORDER_LPC: usize = 24; /* max order of the LPC analysis in schur() and k2a() */

pub(crate) const SILK_INT32_MAX: i32 = 0x7FFFFFFF; /*  2^31 - 1 =  2147483647 */
pub(crate) const SILK_INT32_MIN: i32 = i32::MIN; /* -2^31     = -2147483648 */
pub(crate) const SILK_INT16_MAX: i32 = 0x7FFF; /*  2^15 - 1 =  32767 */
pub(crate) const SILK_INT16_MIN: i32 = -0x8000; /* -2^15     = -32768 */

/* (a32 * (opus_int32)((opus_int16)(b32))) >> 16 output have to be 32bit int */
#[inline(always)]
pub(crate) fn silk_smulwb(a32: i32, b32: i32) -> i32 {
    ((a32 as i64 * (b32 as i16) as i64) >> 16) as i32
}

/* a32 + (b32 * (opus_int32)((opus_int16)(c32))) >> 16 output have to be 32bit int */
#[inline(always)]
pub(crate) fn silk_smlawb(a32: i32, b32: i32, c32: i32) -> i32 {
    (a32 as i64 + ((b32 as i64 * (c32 as i16) as i64) >> 16)) as i32
}

/* (a32 * (b32 >> 16)) >> 16 */
#[inline(always)]
pub(crate) fn silk_smulwt(a32: i32, b32: i32) -> i32 {
    ((a32 as i64 * (b32 >> 16) as i64) >> 16) as i32
}

/* (opus_int32)((opus_int16)(a3))) * (opus_int32)((opus_int16)(b32)) output have to be 32bit int */
#[inline(always)]
pub(crate) fn silk_smulbb(a32: i32, b32: i32) -> i32 {
    (a32 as i16 as i32).wrapping_mul(b32 as i16 as i32)
}

/* a32 + (opus_int32)((opus_int16)(b32)) * (opus_int32)((opus_int16)(c32)) output have to be 32bit int */
#[inline(always)]
pub(crate) fn silk_smlabb(a32: i32, b32: i32, c32: i32) -> i32 {
    a32.wrapping_add((b32 as i16 as i32).wrapping_mul(c32 as i16 as i32))
}

/* (a32 * b32) >> 16 */
#[inline(always)]
pub(crate) fn silk_smulww(a32: i32, b32: i32) -> i32 {
    ((a32 as i64 * b32 as i64) >> 16) as i32
}

/* a32 + ((b32 * c32) >> 16) */
#[inline(always)]
pub(crate) fn silk_smlaww(a32: i32, b32: i32, c32: i32) -> i32 {
    (a32 as i64 + ((b32 as i64 * c32 as i64) >> 16)) as i32
}

/* add/subtract with output saturated */
/// Translation of `silk_ADD_SAT32()`.
#[inline(always)]
pub(crate) fn silk_add_sat32(a: i32, b: i32) -> i32 {
    if ((a as u32).wrapping_add(b as u32) & 0x80000000) == 0 {
        if ((a & b) as u32 & 0x80000000) != 0 {
            SILK_INT32_MIN
        } else {
            a.wrapping_add(b)
        }
    } else if ((a | b) as u32 & 0x80000000) == 0 {
        SILK_INT32_MAX
    } else {
        a.wrapping_add(b)
    }
}

/// Translation of `silk_SUB_SAT32()`.
#[inline(always)]
pub(crate) fn silk_sub_sat32(a: i32, b: i32) -> i32 {
    if ((a as u32).wrapping_sub(b as u32) & 0x80000000) == 0 {
        if (a as u32 & (b as u32 ^ 0x80000000) & 0x80000000) != 0 {
            SILK_INT32_MIN
        } else {
            a.wrapping_sub(b)
        }
    } else if ((a as u32 ^ 0x80000000) & b as u32 & 0x80000000) != 0 {
        SILK_INT32_MAX
    } else {
        a.wrapping_sub(b)
    }
}

/// Translation of `silk_CLZ16()`.
#[inline(always)]
pub(crate) fn silk_clz16(in16: i16) -> i32 {
    ((in16 as i32 as u32) << 16 | 0x8000).leading_zeros() as i32
}

/// Translation of `silk_CLZ32()`.
#[inline(always)]
pub(crate) fn silk_clz32(in32: i32) -> i32 {
    (in32 as u32).leading_zeros() as i32
}

/* Rotate a32 right by 'rot' bits. Negative rot values result in rotating
left. Output is 32bit int.
Note: contemporary compilers recognize the C expression below and
compile it into a 'ror' instruction if available. No need for OPUS_INLINE ASM! */
/// Translation of `silk_ROR32()`.
#[inline(always)]
pub(crate) fn silk_ror32(a32: i32, rot: i32) -> i32 {
    let x = a32 as u32;
    let r = rot as u32;
    let m = rot.wrapping_neg() as u32;
    if rot == 0 {
        a32
    } else if rot < 0 {
        (x.wrapping_shl(m) | x.wrapping_shr(32u32.wrapping_sub(m))) as i32
    } else {
        (x.wrapping_shl(32u32.wrapping_sub(r)) | x.wrapping_shr(r)) as i32
    }
}

/* (a32 * b32) output have to be 32bit int */
#[inline(always)]
pub(crate) fn silk_mul(a32: i32, b32: i32) -> i32 {
    a32.wrapping_mul(b32)
}

/* a32 + (b32 * c32) output have to be 32bit int */
#[inline(always)]
pub(crate) fn silk_mla(a32: i32, b32: i32, c32: i32) -> i32 {
    a32.wrapping_add(b32.wrapping_mul(c32))
}

/* ((a32 >> 16)  * (b32 >> 16)) output have to be 32bit int */
#[inline(always)]
pub(crate) fn silk_smultt(a32: i32, b32: i32) -> i32 {
    (a32 >> 16).wrapping_mul(b32 >> 16)
}

/* (a32 * b32) */
#[inline(always)]
pub(crate) fn silk_smull(a32: i32, b32: i32) -> i64 {
    a32 as i64 * b32 as i64
}

/* Adds two signed 32-bit values in a way that can overflow, while not relying on undefined behaviour
(just standard two's complement implementation-specific behaviour) */
#[inline(always)]
pub(crate) fn silk_add32_ovflw(a: i32, b: i32) -> i32 {
    (a as u32).wrapping_add(b as u32) as i32
}

/* Subtractss two signed 32-bit values in a way that can overflow, while not relying on undefined behaviour
(just standard two's complement implementation-specific behaviour) */
#[inline(always)]
pub(crate) fn silk_sub32_ovflw(a: i32, b: i32) -> i32 {
    (a as u32).wrapping_sub(b as u32) as i32
}

/* Multiply-accumulate macros that allow overflow in the addition (ie, no asserts in debug mode) */
#[inline(always)]
pub(crate) fn silk_mla_ovflw(a32: i32, b32: i32, c32: i32) -> i32 {
    silk_add32_ovflw(a32, (b32 as u32).wrapping_mul(c32 as u32) as i32)
}

#[inline(always)]
pub(crate) fn silk_smlabb_ovflw(a32: i32, b32: i32, c32: i32) -> i32 {
    silk_add32_ovflw(a32, (b32 as i16 as i32).wrapping_mul(c32 as i16 as i32))
}

/// Translation of `silk_DIV32_16()` (and `silk_DIV32()`).
#[inline(always)]
pub(crate) fn silk_div32_16(a32: i32, b16: i32) -> i32 {
    if b16 == 0 {
        0
    } else {
        a32.wrapping_div(b16)
    }
}

/// Translation of `silk_DIV32()`.
#[inline(always)]
pub(crate) fn silk_div32(a32: i32, b32: i32) -> i32 {
    silk_div32_16(a32, b32)
}

/// Translation of `silk_SAT16()`.
#[inline(always)]
pub(crate) fn silk_sat16(a: i32) -> i32 {
    if a > SILK_INT16_MAX {
        SILK_INT16_MAX
    } else if a < SILK_INT16_MIN {
        SILK_INT16_MIN
    } else {
        a
    }
}

/// Translation of `silk_ADD_SAT16()`.
#[inline(always)]
pub(crate) fn silk_add_sat16(a: i32, b: i32) -> i16 {
    silk_sat16(a.wrapping_add(b)) as i16
}

/// Translation of `silk_LSHIFT32()` (and `silk_LSHIFT()`).
#[inline(always)]
pub(crate) fn silk_lshift(a: i32, shift: i32) -> i32 {
    (a as u32).wrapping_shl(shift as u32) as i32
}

/// Translation of `silk_RSHIFT32()` (and `silk_RSHIFT()`).
#[inline(always)]
pub(crate) fn silk_rshift(a: i32, shift: i32) -> i32 {
    a.wrapping_shr(shift as u32)
}

/// Translation of `silk_LSHIFT_ovflw()`.
#[inline(always)]
pub(crate) fn silk_lshift_ovflw(a: i32, shift: i32) -> i32 {
    silk_lshift(a, shift)
}

/// Translation of `silk_ADD_LSHIFT32()` (and `silk_ADD_LSHIFT()`).
#[inline(always)]
pub(crate) fn silk_add_lshift32(a: i32, b: i32, shift: i32) -> i32 {
    a.wrapping_add(silk_lshift(b, shift))
}

/// Translation of `silk_ADD_RSHIFT32()`.
#[inline(always)]
pub(crate) fn silk_add_rshift32(a: i32, b: i32, shift: i32) -> i32 {
    a.wrapping_add(silk_rshift(b, shift))
}

/// Translation of `silk_SUB_LSHIFT32()`.
#[inline(always)]
pub(crate) fn silk_sub_lshift32(a: i32, b: i32, shift: i32) -> i32 {
    a.wrapping_sub(silk_lshift(b, shift))
}

/// Translation of `silk_ADD_RSHIFT_uint()`.
#[inline(always)]
pub(crate) fn silk_add_rshift_uint(a: u32, b: u32, shift: i32) -> u32 {
    a.wrapping_add(b.wrapping_shr(shift as u32))
}

/* Requires that shift > 0 */
/// Translation of `silk_RSHIFT_ROUND()`.
#[inline(always)]
pub(crate) fn silk_rshift_round(a: i32, shift: i32) -> i32 {
    if shift == 1 {
        (a >> 1).wrapping_add(a & 1)
    } else {
        (a.wrapping_shr((shift - 1) as u32).wrapping_add(1)) >> 1
    }
}

/// Translation of `silk_RSHIFT_ROUND64()`.
#[inline(always)]
pub(crate) fn silk_rshift_round64(a: i64, shift: i32) -> i64 {
    if shift == 1 {
        (a >> 1).wrapping_add(a & 1)
    } else {
        (a.wrapping_shr((shift - 1) as u32).wrapping_add(1)) >> 1
    }
}

/// Translation of `silk_LIMIT()` (and `silk_LIMIT_int()`, `silk_LIMIT_32()`).
#[inline(always)]
pub(crate) fn silk_limit(a: i32, limit1: i32, limit2: i32) -> i32 {
    if limit1 > limit2 {
        if a > limit1 {
            limit1
        } else if a < limit2 {
            limit2
        } else {
            a
        }
    } else if a > limit2 {
        limit2
    } else if a < limit1 {
        limit1
    } else {
        a
    }
}

/// Translation of `silk_LSHIFT_SAT32()`.
#[inline(always)]
pub(crate) fn silk_lshift_sat32(a: i32, shift: i32) -> i32 {
    silk_lshift(
        silk_limit(
            a,
            silk_rshift(SILK_INT32_MIN, shift),
            silk_rshift(SILK_INT32_MAX, shift),
        ),
        shift,
    )
}

/// Translation of `silk_abs()` (`silk_abs(silk_intXX_MIN)` stays negative).
#[inline(always)]
pub(crate) fn silk_abs(a: i32) -> i32 {
    if a > 0 {
        a
    } else {
        a.wrapping_neg()
    }
}

/* PSEUDO-RANDOM GENERATOR                                                          */
/* Make sure to store the result as the seed for the next call (also in between     */
/* frames), otherwise result won't be random at all. When only using some of the    */
/* bits, take the most significant bits by right-shifting.                          */
pub(crate) const RAND_MULTIPLIER: i32 = 196314165;
pub(crate) const RAND_INCREMENT: i32 = 907633515;
/// Translation of `silk_RAND()`.
#[inline(always)]
pub(crate) fn silk_rand(seed: i32) -> i32 {
    silk_mla_ovflw(RAND_INCREMENT, seed, RAND_MULTIPLIER)
}

/* the following seems faster on x86 */
/// Translation of `silk_SMMUL()`.
#[inline(always)]
pub(crate) fn silk_smmul(a32: i32, b32: i32) -> i32 {
    (silk_smull(a32, b32) >> 32) as i32
}

/* Macro to convert floating-point constants to fixed-point */
/// Translation of `SILK_FIX_CONST()`: `((opus_int32)((C) * ((opus_int64)1 << (Q)) + 0.5))`.
pub(crate) const fn silk_fix_const(c: f64, q: u32) -> i32 {
    (c * (1i64 << q) as f64 + 0.5) as i32
}

/* get number of leading zeros and fractional part (the bits right after the leading one */
/// Translation of `silk_CLZ_FRAC()`: `(lz, frac_Q7)`.
#[inline(always)]
pub(crate) fn silk_clz_frac(in_: i32) -> (i32, i32) {
    let lzeros = silk_clz32(in_);
    (lzeros, silk_ror32(in_, 24 - lzeros) & 0x7f)
}

/* Approximation of square root                                          */
/* Accuracy: < +/- 10%  for output values > 15                           */
/*           < +/- 2.5% for output values > 120                          */
/// Translation of `silk_SQRT_APPROX()`.
#[inline(always)]
pub(crate) fn silk_sqrt_approx(x: i32) -> i32 {
    if x <= 0 {
        return 0;
    }

    let (lz, frac_q7) = silk_clz_frac(x);

    let mut y: i32 = if lz & 1 != 0 {
        32768
    } else {
        46214 /* 46214 = sqrt(2) * 32768 */
    };

    /* get scaling right */
    y >>= silk_rshift(lz, 1);

    /* increment using fractional part of input */
    y = silk_smlawb(y, y, silk_smulbb(213, frac_q7));

    y
}

/* Divide two int32 values and return result as int32 in a given Q-domain */
/// Translation of `silk_DIV32_varQ()`: a good approximation of
/// `(a32 << Qres) / b32`.
#[inline(always)]
pub(crate) fn silk_div32_varq(a32: i32, b32: i32, qres: i32) -> i32 {
    /* Compute number of bits head room and normalize inputs */
    let a_headrm = silk_clz32(silk_abs(a32)) - 1;
    let mut a32_nrm = silk_lshift(a32, a_headrm); /* Q: a_headrm                  */
    let b_headrm = silk_clz32(silk_abs(b32)) - 1;
    let b32_nrm = silk_lshift(b32, b_headrm); /* Q: b_headrm                  */

    /* Inverse of b32, with 14 bits of precision */
    let b32_inv = silk_div32_16(SILK_INT32_MAX >> 2, silk_rshift(b32_nrm, 16)); /* Q: 29 + 16 - b_headrm        */

    /* First approximation */
    let mut result = silk_smulwb(a32_nrm, b32_inv); /* Q: 29 + a_headrm - b_headrm  */

    /* Compute residual by subtracting product of denominator and first approximation */
    /* It's OK to overflow because the final value of a32_nrm should always be small */
    a32_nrm = silk_sub32_ovflw(a32_nrm, silk_lshift_ovflw(silk_smmul(b32_nrm, result), 3)); /* Q: a_headrm   */

    /* Refinement */
    result = silk_smlawb(result, a32_nrm, b32_inv); /* Q: 29 + a_headrm - b_headrm  */

    /* Convert to Qres domain */
    let lshift = 29 + a_headrm - b_headrm - qres;
    if lshift < 0 {
        silk_lshift_sat32(result, -lshift)
    } else if lshift < 32 {
        silk_rshift(result, lshift)
    } else {
        /* Avoid undefined result */
        0
    }
}

/* Invert int32 value and return result as int32 in a given Q-domain */
/// Translation of `silk_INVERSE32_varQ()`: a good approximation of
/// `(1 << Qres) / b32`.
#[inline(always)]
pub(crate) fn silk_inverse32_varq(b32: i32, qres: i32) -> i32 {
    /* Compute number of bits head room and normalize input */
    let b_headrm = silk_clz32(silk_abs(b32)) - 1;
    let b32_nrm = silk_lshift(b32, b_headrm); /* Q: b_headrm                */

    /* Inverse of b32, with 14 bits of precision */
    let b32_inv = silk_div32_16(SILK_INT32_MAX >> 2, silk_rshift(b32_nrm, 16)); /* Q: 29 + 16 - b_headrm    */

    /* First approximation */
    let mut result = silk_lshift(b32_inv, 16); /* Q: 61 - b_headrm            */

    /* Compute residual by subtracting product of denominator and first approximation from one */
    let err_q32 = silk_lshift((1i32 << 29).wrapping_sub(silk_smulwb(b32_nrm, b32_inv)), 3); /* Q32                        */

    /* Refinement */
    result = silk_smlaww(result, err_q32, b32_inv); /* Q: 61 - b_headrm            */

    /* Convert to Qres domain */
    let lshift = 61 - b_headrm - qres;
    if lshift <= 0 {
        silk_lshift_sat32(result, -lshift)
    } else if lshift < 32 {
        silk_rshift(result, lshift)
    } else {
        /* Avoid undefined result */
        0
    }
}
