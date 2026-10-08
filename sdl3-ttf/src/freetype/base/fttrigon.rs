// Rust translation of src/base/fttrigon.c and include/freetype/fttrigon.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2001-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType trigonometric functions (body).
//!
//! This is a fixed-point CORDIC implementation of trigonometric
//! functions as well as transformations between Cartesian and polar
//! coordinates.  The angles are represented as 16.16 fixed-point values
//! in degrees, i.e., the angular resolution is 2^-16 degrees.  Note that
//! only vectors longer than 2^16*180/pi (or at least 22 bits) on a
//! discrete Cartesian grid can have the same or better angular
//! resolution.  Therefore, to maintain this precision, some functions
//! require an interim upscaling of the vectors, whereas others operate
//! with 24-bit long vectors directly.

use super::super::fttypes::*;
use super::ftcalc::{ft_div_fix, ft_msb};

/// `FT_ANGLE_PI`
pub const FT_ANGLE_PI: FtAngle = 180 << 16;
/// `FT_ANGLE_2PI`
pub const FT_ANGLE_2PI: FtAngle = FT_ANGLE_PI * 2;
/// `FT_ANGLE_PI2`
pub const FT_ANGLE_PI2: FtAngle = FT_ANGLE_PI / 2;
/// `FT_ANGLE_PI4`
pub const FT_ANGLE_PI4: FtAngle = FT_ANGLE_PI / 4;

/* the Cordic shrink factor 0.858785336480436 * 2^32 */
const FT_TRIG_SCALE: u64 = 0xDBD95B16;

/* the highest bit in overflow-safe vector components, */
/* MSB of 0.858785336480436 * sqrt(0.5) * 2^30         */
const FT_TRIG_SAFE_MSB: i32 = 29;

/* this table was generated for FT_PI = 180L << 16, i.e. degrees */
const FT_TRIG_MAX_ITERS: i32 = 23;

static FT_TRIG_ARCTAN_TABLE: [FtAngle; 22] = [
    1740967, 919879, 466945, 234379, 117304, 58666, 29335, 14668, 7334, 3667, 1833, 917, 458, 229,
    115, 57, 29, 14, 7, 4, 2, 1,
];

/* multiply a given value by the CORDIC shrink factor */
fn ft_trig_downscale(mut val: FtFixed) -> FtFixed {
    let mut s: i32 = 1;

    if val < 0 {
        val = val.wrapping_neg();
        s = -1;
    }

    /* 0x40000000 comes from regression analysis between true */
    /* and CORDIC hypotenuse, so it minimizes the error       */
    val = (((val as u64)
        .wrapping_mul(FT_TRIG_SCALE)
        .wrapping_add(0x40000000))
        >> 32) as FtFixed;

    if s < 0 {
        val.wrapping_neg()
    } else {
        val
    }
}

/* undefined and never called for zero vector */
fn ft_trig_prenorm(vec: &mut FtVector) -> i32 {
    let x = vec.x;
    let y = vec.y;

    let mut shift = ft_msb((ft_abs(x) | ft_abs(y)) as u32);

    if shift <= FT_TRIG_SAFE_MSB {
        shift = FT_TRIG_SAFE_MSB - shift;
        vec.x = ((x as u64) << shift) as FtPos;
        vec.y = ((y as u64) << shift) as FtPos;
    } else {
        shift -= FT_TRIG_SAFE_MSB;
        vec.x = x >> shift;
        vec.y = y >> shift;
        shift = -shift;
    }

    shift
}

fn ft_trig_pseudo_rotate(vec: &mut FtVector, mut theta: FtAngle) {
    let mut x: FtFixed = vec.x;
    let mut y: FtFixed = vec.y;
    let mut xtemp: FtFixed;

    /* Rotate inside [-PI/4,PI/4] sector */
    while theta < -FT_ANGLE_PI4 {
        xtemp = y;
        y = x.wrapping_neg();
        x = xtemp;
        theta += FT_ANGLE_PI2;
    }

    while theta > FT_ANGLE_PI4 {
        xtemp = y.wrapping_neg();
        y = x;
        x = xtemp;
        theta -= FT_ANGLE_PI2;
    }

    let mut arctanptr = 0;

    /* Pseudorotations, with right shifts */
    let mut b: FtFixed = 1;
    for i in 1..FT_TRIG_MAX_ITERS {
        if theta < 0 {
            xtemp = x.wrapping_add(y.wrapping_add(b) >> i);
            y = y.wrapping_sub(x.wrapping_add(b) >> i);
            x = xtemp;
            theta += FT_TRIG_ARCTAN_TABLE[arctanptr];
        } else {
            xtemp = x.wrapping_sub(y.wrapping_add(b) >> i);
            y = y.wrapping_add(x.wrapping_add(b) >> i);
            x = xtemp;
            theta -= FT_TRIG_ARCTAN_TABLE[arctanptr];
        }
        arctanptr += 1;
        b <<= 1;
    }

    vec.x = x;
    vec.y = y;
}

fn ft_trig_pseudo_polarize(vec: &mut FtVector) {
    let mut theta: FtAngle;
    let mut x: FtFixed = vec.x;
    let mut y: FtFixed = vec.y;
    let mut xtemp: FtFixed;

    /* Get the vector into [-PI/4,PI/4] sector */
    if y > x {
        if y > x.wrapping_neg() {
            theta = FT_ANGLE_PI2;
            xtemp = y;
            y = x.wrapping_neg();
            x = xtemp;
        } else {
            theta = if y > 0 { FT_ANGLE_PI } else { -FT_ANGLE_PI };
            x = x.wrapping_neg();
            y = y.wrapping_neg();
        }
    } else if y < x.wrapping_neg() {
        theta = -FT_ANGLE_PI2;
        xtemp = y.wrapping_neg();
        y = x;
        x = xtemp;
    } else {
        theta = 0;
    }

    let mut arctanptr = 0;

    /* Pseudorotations, with right shifts */
    let mut b: FtFixed = 1;
    for i in 1..FT_TRIG_MAX_ITERS {
        if y > 0 {
            xtemp = x.wrapping_add(y.wrapping_add(b) >> i);
            y = y.wrapping_sub(x.wrapping_add(b) >> i);
            x = xtemp;
            theta += FT_TRIG_ARCTAN_TABLE[arctanptr];
        } else {
            xtemp = x.wrapping_sub(y.wrapping_add(b) >> i);
            y = y.wrapping_add(x.wrapping_add(b) >> i);
            x = xtemp;
            theta -= FT_TRIG_ARCTAN_TABLE[arctanptr];
        }
        arctanptr += 1;
        b <<= 1;
    }

    /* round theta to acknowledge its error that mostly comes */
    /* from accumulated rounding errors in the arctan table   */
    if theta >= 0 {
        theta = ft_pad_round(theta, 16);
    } else {
        theta = -ft_pad_round(-theta, 16);
    }

    vec.x = x;
    vec.y = theta;
}

/// `FT_Cos`
pub fn ft_cos(angle: FtAngle) -> FtFixed {
    let mut v = FtVector::default();
    ft_vector_unit(&mut v, angle);
    v.x
}

/// `FT_Sin`
pub fn ft_sin(angle: FtAngle) -> FtFixed {
    let mut v = FtVector::default();
    ft_vector_unit(&mut v, angle);
    v.y
}

/// `FT_Tan`
pub fn ft_tan(angle: FtAngle) -> FtFixed {
    let mut v = FtVector { x: 1 << 24, y: 0 };
    ft_trig_pseudo_rotate(&mut v, angle);
    ft_div_fix(v.y, v.x)
}

/// `FT_Atan2`
pub fn ft_atan2(dx: FtFixed, dy: FtFixed) -> FtAngle {
    if dx == 0 && dy == 0 {
        return 0;
    }

    let mut v = FtVector { x: dx, y: dy };
    ft_trig_prenorm(&mut v);
    ft_trig_pseudo_polarize(&mut v);

    v.y
}

/// `FT_Vector_Unit`
pub fn ft_vector_unit(vec: &mut FtVector, angle: FtAngle) {
    vec.x = (FT_TRIG_SCALE >> 8) as FtPos;
    vec.y = 0;
    ft_trig_pseudo_rotate(vec, angle);
    vec.x = (vec.x + 0x80) >> 8;
    vec.y = (vec.y + 0x80) >> 8;
}

/// `FT_Vector_Rotate`
pub fn ft_vector_rotate(vec: &mut FtVector, angle: FtAngle) {
    if angle == 0 {
        return;
    }

    let mut v = *vec;

    if v.x == 0 && v.y == 0 {
        return;
    }

    let mut shift = ft_trig_prenorm(&mut v);
    ft_trig_pseudo_rotate(&mut v, angle);
    v.x = ft_trig_downscale(v.x);
    v.y = ft_trig_downscale(v.y);

    if shift > 0 {
        let half: i64 = (1i32 << (shift - 1)) as i64;

        vec.x = (v.x.wrapping_add(half) - (v.x < 0) as i64) >> shift;
        vec.y = (v.y.wrapping_add(half) - (v.y < 0) as i64) >> shift;
    } else {
        shift = -shift;
        vec.x = ((v.x as u64) << shift) as FtPos;
        vec.y = ((v.y as u64) << shift) as FtPos;
    }
}

/// `FT_Vector_Length`
pub fn ft_vector_length(vec: &FtVector) -> FtFixed {
    let mut v = *vec;

    /* handle trivial cases */
    if v.x == 0 {
        return ft_abs(v.y);
    } else if v.y == 0 {
        return ft_abs(v.x);
    }

    /* general case */
    let shift = ft_trig_prenorm(&mut v);
    ft_trig_pseudo_polarize(&mut v);

    v.x = ft_trig_downscale(v.x);

    if shift > 0 {
        return (v.x + (1i64 << (shift - 1))) >> shift;
    }

    ((v.x as u32) << -shift) as FtFixed
}

/// `FT_Vector_Polarize`
pub fn ft_vector_polarize(vec: &FtVector, length: &mut FtFixed, angle: &mut FtAngle) {
    let mut v = *vec;

    if v.x == 0 && v.y == 0 {
        return;
    }

    let shift = ft_trig_prenorm(&mut v);
    ft_trig_pseudo_polarize(&mut v);

    v.x = ft_trig_downscale(v.x);

    *length = if shift >= 0 {
        v.x >> shift
    } else {
        ((v.x as u32) << -shift) as FtFixed
    };
    *angle = v.y;
}

/// `FT_Vector_From_Polar`
pub fn ft_vector_from_polar(vec: &mut FtVector, length: FtFixed, angle: FtAngle) {
    vec.x = length;
    vec.y = 0;

    ft_vector_rotate(vec, angle);
}

/// `FT_Angle_Diff`
pub fn ft_angle_diff(angle1: FtAngle, angle2: FtAngle) -> FtAngle {
    let mut delta = angle2.wrapping_sub(angle1);

    while delta <= -FT_ANGLE_PI {
        delta += FT_ANGLE_2PI;
    }

    while delta > FT_ANGLE_PI {
        delta -= FT_ANGLE_2PI;
    }

    delta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cordic_values() {
        assert_eq!(ft_cos(0), 0x10000);
        assert_eq!(ft_sin(FT_ANGLE_PI2), 0x10000);
        assert_eq!(ft_atan2(0x10000, 0x10000), FT_ANGLE_PI4);
        assert_eq!(
            ft_vector_length(&FtVector {
                x: 3 << 16,
                y: 4 << 16
            }),
            5 << 16
        );
        assert_eq!(ft_angle_diff(0, FT_ANGLE_2PI - 1), -1);
    }
}
