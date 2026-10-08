// Rust translation of src/base/ftcalc.c and include/freetype/internal/ftcalc.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Arithmetic computations (body).
//!
//! Support for 1-complement arithmetic has been totally dropped in this
//! release.  You can still write your own code if you need it.
//!
//! Implementing basic computation routines.
//!
//! FT_MulDiv(), FT_MulFix(), FT_DivFix(), FT_RoundFix(), FT_CeilFix(),
//! and FT_FloorFix() are declared in freetype.h.
//!
//! `FT_INT64` is defined (as it is with GCC and every compiler upstream
//! knows of today), so the 64-bit versions are translated. `FT_MulFix`
//! is the x86_64 GCC version upstream's build uses
//! (`FT_MulFix_x86_64`, through `FT_MULFIX_ASSEMBLER`, also behind the
//! exported function): it takes and returns `FT_Int32` values.

use super::super::fttypes::*;
use super::fttrigon::ft_vector_length;

/// `ADD_INT`
#[inline]
pub const fn add_int(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}
/// `SUB_INT`
#[inline]
pub const fn sub_int(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}
/// `MUL_INT`
#[inline]
pub const fn mul_int(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}
/// `NEG_INT`
#[inline]
pub const fn neg_int(a: i32) -> i32 {
    a.wrapping_neg()
}
/// `ADD_LONG`
#[inline]
pub const fn add_long(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}
/// `SUB_LONG`
#[inline]
pub const fn sub_long(a: i64, b: i64) -> i64 {
    a.wrapping_sub(b)
}
/// `MUL_LONG`
#[inline]
pub const fn mul_long(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}
/// `NEG_LONG`
#[inline]
pub const fn neg_long(a: i64) -> i64 {
    a.wrapping_neg()
}
/// `ADD_INT32`
#[inline]
pub const fn add_int32(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}
/// `SUB_INT32`
#[inline]
pub const fn sub_int32(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}
/// `MUL_INT32`
#[inline]
pub const fn mul_int32(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}
/// `NEG_INT32`
#[inline]
pub const fn neg_int32(a: i32) -> i32 {
    a.wrapping_neg()
}
/// `ADD_INT64`
#[inline]
pub const fn add_int64(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}
/// `SUB_INT64`
#[inline]
pub const fn sub_int64(a: i64, b: i64) -> i64 {
    a.wrapping_sub(b)
}
/// `MUL_INT64`
#[inline]
pub const fn mul_int64(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}
/// `NEG_INT64`
#[inline]
pub const fn neg_int64(a: i64) -> i64 {
    a.wrapping_neg()
}

/// `INT_TO_F26DOT6`
#[inline]
pub const fn int_to_f26dot6(x: i64) -> i64 {
    x.wrapping_mul(64)
}
/// `INT_TO_F2DOT14`
#[inline]
pub const fn int_to_f2dot14(x: i64) -> i64 {
    x.wrapping_mul(16384)
}
/// `INT_TO_FIXED`
#[inline]
pub const fn int_to_fixed(x: i64) -> i64 {
    x.wrapping_mul(65536)
}
/// `F2DOT14_TO_FIXED`
#[inline]
pub const fn f2dot14_to_fixed(x: i64) -> i64 {
    x.wrapping_mul(4)
}
/// `FIXED_TO_INT`
#[inline]
pub const fn fixed_to_int(x: i64) -> i64 {
    ft_round_fix(x) >> 16
}
/// `ROUND_F26DOT6`
#[inline]
pub const fn round_f26dot6(x: i64) -> i64 {
    (x.wrapping_add(32 - (x < 0) as i64)) & -64
}

/// `FT_MSB` (`31 - __builtin_clz( x )`; undefined for zero upstream, -1
/// here)
#[inline]
pub const fn ft_msb(z: u32) -> i32 {
    31 - z.leading_zeros() as i32
}

/// The x86_64 `FT_MulFix_x86_64` behind `FT_MulFix`.
#[inline]
pub const fn ft_mul_fix_x86_64(a: i32, b: i32) -> i32 {
    let mut ret: i64 = a as i64 * b as i64;
    let tmp = ret >> 63;
    ret += 0x8000 + tmp;
    (ret >> 16) as i32
}

/* The following three functions are available regardless of whether */
/* FT_INT64 is defined.                                              */

/// `FT_RoundFix`
#[inline]
pub const fn ft_round_fix(a: FtFixed) -> FtFixed {
    (add_long(a, 0x8000 - (a < 0) as i64)) & !0xFFFF
}

/// `FT_CeilFix`
#[inline]
pub const fn ft_ceil_fix(a: FtFixed) -> FtFixed {
    (add_long(a, 0xFFFF)) & !0xFFFF
}

/// `FT_FloorFix`
#[inline]
pub const fn ft_floor_fix(a: FtFixed) -> FtFixed {
    a & !0xFFFF
}

/// `FT_Hypot`
pub fn ft_hypot_fixed(x: FtFixed, y: FtFixed) -> FtFixed {
    let v = FtVector { x, y };
    ft_vector_length(&v)
}

/* transfer sign, leaving a positive number;                        */
/* we need an unsigned value to safely negate INT_MIN (or LONG_MIN) */
macro_rules! ft_move_sign {
    ($x:expr, $x_unsigned:ident, $s:ident) => {
        if $x < 0 {
            $x_unsigned = 0u64.wrapping_sub($x_unsigned);
            $s = -$s;
        }
    };
}

/// `FT_MulDiv`
pub fn ft_mul_div(a_: FtLong, b_: FtLong, c_: FtLong) -> FtLong {
    let mut s: i32 = 1;
    let mut a = a_ as u64;
    let mut b = b_ as u64;
    let mut c = c_ as u64;

    ft_move_sign!(a_, a, s);
    ft_move_sign!(b_, b, s);
    ft_move_sign!(c_, c, s);

    let d = if c > 0 {
        (a.wrapping_mul(b).wrapping_add(c >> 1)) / c
    } else {
        0x7FFFFFFF
    };

    let d_ = d as FtLong;

    if s < 0 {
        neg_long(d_)
    } else {
        d_
    }
}

/// `FT_MulDiv_No_Round`
pub fn ft_mul_div_no_round(a_: FtLong, b_: FtLong, c_: FtLong) -> FtLong {
    let mut s: i32 = 1;
    let mut a = a_ as u64;
    let mut b = b_ as u64;
    let mut c = c_ as u64;

    ft_move_sign!(a_, a, s);
    ft_move_sign!(b_, b, s);
    ft_move_sign!(c_, c, s);

    let d = if c > 0 {
        a.wrapping_mul(b) / c
    } else {
        0x7FFFFFFF
    };

    let d_ = d as FtLong;

    if s < 0 {
        neg_long(d_)
    } else {
        d_
    }
}

/// `FT_MulFix`
#[inline]
pub fn ft_mul_fix(a_: FtLong, b_: FtLong) -> FtLong {
    ft_mul_fix_x86_64(a_ as i32, b_ as i32) as FtLong
}

/// `FT_DivFix`
pub fn ft_div_fix(a_: FtLong, b_: FtLong) -> FtLong {
    let mut s: i32 = 1;
    let mut a = a_ as u64;
    let mut b = b_ as u64;

    ft_move_sign!(a_, a, s);
    ft_move_sign!(b_, b, s);

    let q = if b > 0 {
        ((a << 16).wrapping_add(b >> 1)) / b
    } else {
        0x7FFFFFFF
    };

    let q_ = q as FtLong;

    if s < 0 {
        neg_long(q_)
    } else {
        q_
    }
}

/// `FT_Matrix_Multiply` (documentation is in ftglyph.h)
pub fn ft_matrix_multiply(a: &FtMatrix, b: &mut FtMatrix) {
    let xx = add_long(ft_mul_fix(a.xx, b.xx), ft_mul_fix(a.xy, b.yx));
    let xy = add_long(ft_mul_fix(a.xx, b.xy), ft_mul_fix(a.xy, b.yy));
    let yx = add_long(ft_mul_fix(a.yx, b.xx), ft_mul_fix(a.yy, b.yx));
    let yy = add_long(ft_mul_fix(a.yx, b.xy), ft_mul_fix(a.yy, b.yy));

    b.xx = xx;
    b.xy = xy;
    b.yx = yx;
    b.yy = yy;
}

/// `FT_Matrix_Invert` (documentation is in ftglyph.h)
pub fn ft_matrix_invert(matrix: &mut FtMatrix) -> FtResult<()> {
    /* compute discriminant */
    let delta = ft_mul_fix(matrix.xx, matrix.yy).wrapping_sub(ft_mul_fix(matrix.xy, matrix.yx));

    if delta == 0 {
        return Err(FT_ERR_INVALID_ARGUMENT); /* matrix can't be inverted */
    }

    matrix.xy = ft_div_fix(matrix.xy, delta).wrapping_neg();
    matrix.yx = ft_div_fix(matrix.yx, delta).wrapping_neg();

    let xx = matrix.xx;
    let yy = matrix.yy;

    matrix.xx = ft_div_fix(yy, delta);
    matrix.yy = ft_div_fix(xx, delta);

    Ok(())
}

/// `FT_Matrix_Multiply_Scaled`
pub fn ft_matrix_multiply_scaled(a: &FtMatrix, b: &mut FtMatrix, scaling: FtLong) {
    let val = 0x10000i64.wrapping_mul(scaling);

    let xx = add_long(ft_mul_div(a.xx, b.xx, val), ft_mul_div(a.xy, b.yx, val));
    let xy = add_long(ft_mul_div(a.xx, b.xy, val), ft_mul_div(a.xy, b.yy, val));
    let yx = add_long(ft_mul_div(a.yx, b.xx, val), ft_mul_div(a.yy, b.yx, val));
    let yy = add_long(ft_mul_div(a.yx, b.xy, val), ft_mul_div(a.yy, b.yy, val));

    b.xx = xx;
    b.xy = xy;
    b.yx = yx;
    b.yy = yy;
}

/// `FT_Matrix_Check`
pub fn ft_matrix_check(matrix: &FtMatrix) -> bool {
    let mut xx = matrix.xx;
    let mut xy = matrix.xy;
    let mut yx = matrix.yx;
    let mut yy = matrix.yy;
    let val = ft_abs(xx) | ft_abs(xy) | ft_abs(yx) | ft_abs(yy);

    /* we only handle non-zero 32-bit values */
    if val == 0 || val > 0x7FFFFFFF {
        return false;
    }

    /* Scale matrix to avoid the temp1 overflow, which is */
    /* more stringent than avoiding the temp2 overflow.   */

    let shift = ft_msb(val as u32) - 12;

    if shift > 0 {
        xx >>= shift;
        xy >>= shift;
        yx >>= shift;
        yy >>= shift;
    }

    let temp1: u64 = 32u64.wrapping_mul(ft_abs(xx * yy - xy * yx) as u64);
    let temp2: u64 = ((xx * xx) as u64)
        .wrapping_add((xy * xy) as u64)
        .wrapping_add((yx * yx) as u64)
        .wrapping_add((yy * yy) as u64);

    temp1 > temp2
}

/// `FT_Vector_Transform_Scaled`
pub fn ft_vector_transform_scaled(vector: &mut FtVector, matrix: &FtMatrix, scaling: FtLong) {
    let val = 0x10000i64.wrapping_mul(scaling);

    let xz = add_long(
        ft_mul_div(vector.x, matrix.xx, val),
        ft_mul_div(vector.y, matrix.xy, val),
    );
    let yz = add_long(
        ft_mul_div(vector.x, matrix.yx, val),
        ft_mul_div(vector.y, matrix.yy, val),
    );

    vector.x = xz;
    vector.y = yz;
}

/// `FT_Vector_NormLen`
pub fn ft_vector_norm_len(vector: &mut FtVector) -> u32 {
    let mut x_ = vector.x as i32;
    let mut y_ = vector.y as i32;
    let mut b: i32;
    let mut z: i32;
    let mut u: u32;
    let mut v: u32;
    let mut l: u32;
    let mut sx: i32 = 1;
    let mut sy: i32 = 1;

    let mut x = x_ as u32;
    let mut y = y_ as u32;

    if x_ < 0 {
        x = 0u32.wrapping_sub(x);
        sx = -sx;
    }
    if y_ < 0 {
        y = 0u32.wrapping_sub(y);
        sy = -sy;
    }

    /* trivial cases */
    if x == 0 {
        if y > 0 {
            vector.y = (sy * 0x10000) as FtPos;
        }
        return y;
    } else if y == 0 {
        if x > 0 {
            vector.x = (sx * 0x10000) as FtPos;
        }
        return x;
    }

    /* Estimate length and prenormalize by shifting so that */
    /* the new approximate length is between 2/3 and 4/3.   */
    /* The magic constant 0xAAAAAAAAUL (2/3 of 2^32) helps  */
    /* achieve this in 16.16 fixed-point representation.    */
    l = if x > y {
        x.wrapping_add(y >> 1)
    } else {
        y.wrapping_add(x >> 1)
    };

    let mut shift = 31 - ft_msb(l);
    shift -= 15 + (l >= (0xAAAAAAAAu32 >> shift)) as i32;

    if shift > 0 {
        x <<= shift;
        y <<= shift;

        /* re-estimate length for tiny vectors */
        l = if x > y {
            x.wrapping_add(y >> 1)
        } else {
            y.wrapping_add(x >> 1)
        };
    } else {
        x >>= -shift;
        y >>= -shift;
        l >>= -shift;
    }

    /* lower linear approximation for reciprocal length minus one */
    b = 0x10000i32.wrapping_sub(l as i32);

    x_ = x as i32;
    y_ = y as i32;

    /* Newton's iterations */
    loop {
        u = (x_.wrapping_add(x_.wrapping_mul(b) >> 16)) as u32;
        v = (y_.wrapping_add(y_.wrapping_mul(b) >> 16)) as u32;

        /* Normalized squared length in the parentheses approaches 2^32. */
        /* On two's complement systems, converting to signed gives the   */
        /* difference with 2^32 even if the expression wraps around.     */
        z = (u.wrapping_mul(u).wrapping_add(v.wrapping_mul(v)) as i32).wrapping_neg() / 0x200;
        z = z.wrapping_mul((0x10000i32.wrapping_add(b)) >> 8) / 0x10000;

        b = b.wrapping_add(z);

        if z <= 0 {
            break;
        }
    }

    vector.x = if sx < 0 { -(u as FtPos) } else { u as FtPos };
    vector.y = if sy < 0 { -(v as FtPos) } else { v as FtPos };

    /* Conversion to signed helps to recover from likely wrap around */
    /* in calculating the prenormalized length, because it gives the */
    /* correct difference with 2^32 on two's complement systems.     */
    l = (0x10000i32
        .wrapping_add((u.wrapping_mul(x).wrapping_add(v.wrapping_mul(y)) as i32) / 0x10000))
        as u32;
    if shift > 0 {
        l = (l.wrapping_add(1 << (shift - 1))) >> shift;
    } else {
        l <<= -shift;
    }

    l
}

/// `ft_corner_orientation`
pub fn ft_corner_orientation(in_x: FtPos, in_y: FtPos, out_x: FtPos, out_y: FtPos) -> i32 {
    /* we silently ignore overflow errors since such large values */
    /* lead to even more (harmless) rendering errors later on     */
    let delta = sub_int64(mul_int64(in_x, out_y), mul_int64(in_y, out_x));

    (delta > 0) as i32 - (delta < 0) as i32
}

/// `ft_corner_is_flat`
pub fn ft_corner_is_flat(in_x: FtPos, in_y: FtPos, out_x: FtPos, out_y: FtPos) -> bool {
    let ax = in_x.wrapping_add(out_x);
    let ay = in_y.wrapping_add(out_y);

    /* The idea of this function is to compare the length of the */
    /* hypotenuse with the `in' and `out' length.  The `corner'  */
    /* represented by `in' and `out' is flat if the hypotenuse's */
    /* length isn't too large.                                   */
    /*                                                           */
    /* This approach has the advantage that the angle between    */
    /* `in' and `out' is not checked.  In case one of the two    */
    /* vectors is `dominant', that is, much larger than the      */
    /* other vector, we thus always have a flat corner.          */
    /*                                                           */
    /*                hypotenuse                                 */
    /*       x---------------------------x                       */
    /*        \                      /                           */
    /*         \                /                                */
    /*      in  \          /  out                                */
    /*           \    /                                          */
    /*            o                                              */
    /*              Point                                        */

    let d_in = ft_hypot(in_x, in_y);
    let d_out = ft_hypot(out_x, out_y);
    let d_hypot = ft_hypot(ax, ay);

    /* now do a simple length comparison: */
    /*                                    */
    /*   d_in + d_out < 17/16 d_hypot     */

    (d_in.wrapping_add(d_out).wrapping_sub(d_hypot)) < (d_hypot >> 4)
}

/// `FT_MulAddFix`
pub fn ft_mul_add_fix(s: &[FtFixed], f: &[i32], count: usize) -> i32 {
    let mut temp: i64 = 0;

    for i in 0..count {
        temp = temp.wrapping_add(s[i].wrapping_mul(f[i] as i64));
    }

    ((temp.wrapping_add(0x8000)) >> 16) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_fix_matches_x86_64_gcc() {
        assert_eq!(ft_mul_fix(0x10000, 0x10000), 0x10000);
        assert_eq!(ft_mul_fix(-0x18000, 0x8000), -0xC000);
        assert_eq!(ft_mul_fix(3, 0x8000), 2);
        assert_eq!(ft_mul_fix(-3, 0x8000), -2);
        // arguments are truncated to 32 bits, as with the inline version
        assert_eq!(ft_mul_fix(0x1_0001_0000, 0x10000), 0x10000);
    }

    #[test]
    fn div_and_muldiv() {
        assert_eq!(ft_div_fix(1, 2), 0x8000);
        assert_eq!(ft_div_fix(1, 0), 0x7FFFFFFF);
        assert_eq!(ft_div_fix(-1, 0), -0x7FFFFFFF);
        assert_eq!(ft_mul_div(3, 5, 2), 8);
        assert_eq!(ft_mul_div(-3, 5, 2), -8);
        assert_eq!(ft_mul_div_no_round(3, 5, 2), 7);
        assert_eq!(ft_round_fix(-0x8000), -0x10000);
        assert_eq!(ft_round_fix(0x8000), 0x10000);
    }
}
