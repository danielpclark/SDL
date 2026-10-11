// Rust translation of source/plutovg-ft-math.c, plutovg-ft-math.h and
// plutovg-ft-types.h from PlutoVG (0.0.12, as SDL_ttf's external/plutovg
// pins it): PlutoVG's copies of FreeType's fttrigon.c and fttypes.h.
// Copyright 2001-2005, 2012-2013 by David Turner, Robert Wilhelm, and
// Werner Lemberg (fttrigon.c; fttypes.h: Copyright (C) 1996-2020).
// This file is part of the FreeType project, and may only be used,
// modified, and distributed under the terms of the FreeType project
// license, FTL.TXT. This is an altered (translated) version of the
// original software; it is used under the FreeType License (see
// sdl3-ttf/FTL.TXT and LICENSE.txt).

//! FreeType trigonometric functions (body), and the simple types of
//! PlutoVG's rasterizer and stroker.
//!
//! The C types are those of x86-64 Linux (`long` is 64 bits). Arithmetic
//! wraps as C's does in practice where it can overflow.

/// `PVG_FT_Fixed`
pub type PvgFtFixed = i64;
/// `PVG_FT_Int`
pub type PvgFtInt = i32;
/// `PVG_FT_UInt`
pub type PvgFtUInt = u32;
/// `PVG_FT_Long`
pub type PvgFtLong = i64;
/// `PVG_FT_ULong`
pub type PvgFtULong = u64;
/// `PVG_FT_Pos`
pub type PvgFtPos = i64;
/// `PVG_FT_Error`
pub type PvgFtError = i32;
/// `PVG_FT_Angle`
pub type PvgFtAngle = PvgFtFixed;

/// `PVG_FT_Vector`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PvgFtVector {
    pub x: PvgFtPos,
    pub y: PvgFtPos,
}

/// `PVG_FT_ABS`
#[inline]
pub fn pvg_ft_abs(a: i64) -> i64 {
    if a < 0 {
        a.wrapping_neg()
    } else {
        a
    }
}

/// `PVG_FT_HYPOT`: approximate hypotenuse (its arguments made absolute,
/// as the C macro assigns them).
#[inline]
pub fn pvg_ft_hypot(x: &mut i64, y: &mut i64) -> i64 {
    *x = pvg_ft_abs(*x);
    *y = pvg_ft_abs(*y);
    if *x > *y {
        x.wrapping_add(y.wrapping_mul(3) >> 3)
    } else {
        y.wrapping_add(x.wrapping_mul(3) >> 3)
    }
}

pub const PVG_FT_ANGLE_PI: PvgFtAngle = 180 << 16;
pub const PVG_FT_ANGLE_2PI: PvgFtAngle = PVG_FT_ANGLE_PI * 2;
pub const PVG_FT_ANGLE_PI2: PvgFtAngle = PVG_FT_ANGLE_PI / 2;
pub const PVG_FT_ANGLE_PI4: PvgFtAngle = PVG_FT_ANGLE_PI / 4;

/// `PVG_FT_MSB` (`31 - __builtin_clz(x)`, of the low 32 bits).
#[inline]
fn pvg_ft_msb(x: u32) -> i32 {
    if x == 0 {
        /* FIXME (upstream): __builtin_clz(0) is undefined (x86's bsr leaves
        its destination unchanged); C gets here for vectors whose
        coordinates are multiples of 2^32 */
        return 0;
    }
    31 - x.leading_zeros() as i32
}

/// `PVG_FT_PAD_FLOOR`
#[inline]
fn pvg_ft_pad_floor(x: i64, n: i64) -> i64 {
    x & !(n - 1)
}

/// `PVG_FT_PAD_ROUND`
#[inline]
fn pvg_ft_pad_round(x: i64, n: i64) -> i64 {
    pvg_ft_pad_floor(x.wrapping_add(n / 2), n)
}

/// `PVG_FT_MulFix`
pub fn pvg_ft_mul_fix(mut a: PvgFtLong, mut b: PvgFtLong) -> PvgFtLong {
    let mut s: PvgFtInt = 1;

    if a < 0 {
        a = a.wrapping_neg();
        s = -s;
    }
    if b < 0 {
        b = b.wrapping_neg();
        s = -s;
    }

    let c = (a.wrapping_mul(b).wrapping_add(0x8000)) >> 16;

    if s > 0 {
        c
    } else {
        c.wrapping_neg()
    }
}

/// `PVG_FT_MulDiv`
pub fn pvg_ft_mul_div(mut a: PvgFtLong, mut b: PvgFtLong, mut c: PvgFtLong) -> PvgFtLong {
    let mut s: PvgFtInt = 1;

    if a < 0 {
        a = a.wrapping_neg();
        s = -s;
    }
    if b < 0 {
        b = b.wrapping_neg();
        s = -s;
    }
    if c < 0 {
        c = c.wrapping_neg();
        s = -s;
    }

    let d = if c > 0 {
        a.wrapping_mul(b).wrapping_add(c >> 1).wrapping_div(c)
    } else {
        0x7FFFFFFF
    };

    if s > 0 {
        d
    } else {
        d.wrapping_neg()
    }
}

/// `PVG_FT_DivFix`
pub fn pvg_ft_div_fix(mut a: PvgFtLong, mut b: PvgFtLong) -> PvgFtLong {
    let mut s: PvgFtInt = 1;

    if a < 0 {
        a = a.wrapping_neg();
        s = -s;
    }
    if b < 0 {
        b = b.wrapping_neg();
        s = -s;
    }

    let q = if b > 0 {
        ((a as u64).wrapping_shl(16).wrapping_add((b >> 1) as u64) / b as u64) as i64
    } else {
        0x7FFFFFFF
    };

    if s < 0 {
        q.wrapping_neg()
    } else {
        q
    }
}

/* This is a fixed-point CORDIC implementation of trigonometric          */
/* functions as well as transformations between Cartesian and polar      */
/* coordinates.  The angles are represented as 16.16 fixed-point values  */
/* in degrees, i.e., the angular resolution is 2^-16 degrees.  Note that */
/* only vectors longer than 2^16*180/pi (or at least 22 bits) on a       */
/* discrete Cartesian grid can have the same or better angular           */
/* resolution.  Therefore, to maintain this precision, some functions    */
/* require an interim upscaling of the vectors, whereas others operate   */
/* with 24-bit long vectors directly.                                    */

/* the Cordic shrink factor 0.858785336480436 * 2^32 */
const PVG_FT_TRIG_SCALE: u64 = 0xDBD95B16;

/* the highest bit in overflow-safe vector components, */
/* MSB of 0.858785336480436 * sqrt(0.5) * 2^30         */
const PVG_FT_TRIG_SAFE_MSB: i32 = 29;

/* this table was generated for PVG_FT_PI = 180L << 16, i.e. degrees */
const PVG_FT_TRIG_MAX_ITERS: i32 = 23;

static FT_TRIG_ARCTAN_TABLE: [PvgFtFixed; 22] = [
    1740967, 919879, 466945, 234379, 117304, 58666, 29335, 14668, 7334, 3667, 1833, 917, 458, 229,
    115, 57, 29, 14, 7, 4, 2, 1,
];

/// `ft_trig_downscale`: multiply a given value by the CORDIC shrink
/// factor
fn ft_trig_downscale(mut val: PvgFtFixed) -> PvgFtFixed {
    let s = val;
    val = pvg_ft_abs(val);

    let v = val
        .wrapping_mul(PVG_FT_TRIG_SCALE as i64)
        .wrapping_add(0x100000000);
    val = v >> 32;

    if s >= 0 {
        val
    } else {
        val.wrapping_neg()
    }
}

/// `ft_trig_prenorm`: undefined and never called for zero vector
fn ft_trig_prenorm(vec: &mut PvgFtVector) -> PvgFtInt {
    let x = vec.x;
    let y = vec.y;

    let mut shift = pvg_ft_msb((pvg_ft_abs(x) | pvg_ft_abs(y)) as u32);

    if shift <= PVG_FT_TRIG_SAFE_MSB {
        shift = PVG_FT_TRIG_SAFE_MSB - shift;
        vec.x = (x as u64).wrapping_shl(shift as u32) as i64;
        vec.y = (y as u64).wrapping_shl(shift as u32) as i64;
    } else {
        shift -= PVG_FT_TRIG_SAFE_MSB;
        vec.x = x >> shift;
        vec.y = y >> shift;
        shift = -shift;
    }

    shift
}

/// `ft_trig_pseudo_rotate`
fn ft_trig_pseudo_rotate(vec: &mut PvgFtVector, mut theta: PvgFtAngle) {
    let mut x = vec.x;
    let mut y = vec.y;
    let mut xtemp;

    /* Rotate inside [-PI/4,PI/4] sector */
    while theta < -PVG_FT_ANGLE_PI4 {
        xtemp = y;
        y = x.wrapping_neg();
        x = xtemp;
        theta += PVG_FT_ANGLE_PI2;
    }

    while theta > PVG_FT_ANGLE_PI4 {
        xtemp = y.wrapping_neg();
        y = x;
        x = xtemp;
        theta -= PVG_FT_ANGLE_PI2;
    }

    let mut arctanptr = 0;

    /* Pseudorotations, with right shifts */
    let mut b: PvgFtFixed = 1;
    let mut i = 1;
    while i < PVG_FT_TRIG_MAX_ITERS {
        let v1 = y.wrapping_add(b) >> i;
        let v2 = x.wrapping_add(b) >> i;
        if theta < 0 {
            xtemp = x.wrapping_add(v1);
            y = y.wrapping_sub(v2);
            x = xtemp;
            theta += FT_TRIG_ARCTAN_TABLE[arctanptr];
        } else {
            xtemp = x.wrapping_sub(v1);
            y = y.wrapping_add(v2);
            x = xtemp;
            theta -= FT_TRIG_ARCTAN_TABLE[arctanptr];
        }
        arctanptr += 1;
        b <<= 1;
        i += 1;
    }

    vec.x = x;
    vec.y = y;
}

/// `ft_trig_pseudo_polarize`
fn ft_trig_pseudo_polarize(vec: &mut PvgFtVector) {
    let mut theta: PvgFtAngle;
    let mut x = vec.x;
    let mut y = vec.y;
    let mut xtemp;

    /* Get the vector into [-PI/4,PI/4] sector */
    if y > x {
        if y > x.wrapping_neg() {
            theta = PVG_FT_ANGLE_PI2;
            xtemp = y;
            y = x.wrapping_neg();
            x = xtemp;
        } else {
            theta = if y > 0 {
                PVG_FT_ANGLE_PI
            } else {
                -PVG_FT_ANGLE_PI
            };
            x = x.wrapping_neg();
            y = y.wrapping_neg();
        }
    } else {
        if y < x.wrapping_neg() {
            theta = -PVG_FT_ANGLE_PI2;
            xtemp = y.wrapping_neg();
            y = x;
            x = xtemp;
        } else {
            theta = 0;
        }
    }

    let mut arctanptr = 0;

    /* Pseudorotations, with right shifts */
    let mut b: PvgFtFixed = 1;
    let mut i = 1;
    while i < PVG_FT_TRIG_MAX_ITERS {
        let v1 = y.wrapping_add(b) >> i;
        let v2 = x.wrapping_add(b) >> i;
        if y > 0 {
            xtemp = x.wrapping_add(v1);
            y = y.wrapping_sub(v2);
            x = xtemp;
            theta += FT_TRIG_ARCTAN_TABLE[arctanptr];
        } else {
            xtemp = x.wrapping_sub(v1);
            y = y.wrapping_add(v2);
            x = xtemp;
            theta -= FT_TRIG_ARCTAN_TABLE[arctanptr];
        }
        arctanptr += 1;
        b <<= 1;
        i += 1;
    }

    /* round theta */
    if theta >= 0 {
        theta = pvg_ft_pad_round(theta, 32);
    } else {
        theta = -pvg_ft_pad_round(-theta, 32);
    }

    vec.x = x;
    vec.y = theta;
}

/// `PVG_FT_Cos`
pub fn pvg_ft_cos(angle: PvgFtAngle) -> PvgFtFixed {
    let mut v = PvgFtVector {
        x: (PVG_FT_TRIG_SCALE >> 8) as i64,
        y: 0,
    };
    ft_trig_pseudo_rotate(&mut v, angle);

    v.x.wrapping_add(0x80) >> 8
}

/// `PVG_FT_Sin`
pub fn pvg_ft_sin(angle: PvgFtAngle) -> PvgFtFixed {
    pvg_ft_cos(PVG_FT_ANGLE_PI2.wrapping_sub(angle))
}

/// `PVG_FT_Tan`
pub fn pvg_ft_tan(angle: PvgFtAngle) -> PvgFtFixed {
    let mut v = PvgFtVector {
        x: (PVG_FT_TRIG_SCALE >> 8) as i64,
        y: 0,
    };
    ft_trig_pseudo_rotate(&mut v, angle);

    pvg_ft_div_fix(v.y, v.x)
}

/// `PVG_FT_Atan2`
pub fn pvg_ft_atan2(dx: PvgFtFixed, dy: PvgFtFixed) -> PvgFtAngle {
    if dx == 0 && dy == 0 {
        return 0;
    }

    let mut v = PvgFtVector { x: dx, y: dy };
    ft_trig_prenorm(&mut v);
    ft_trig_pseudo_polarize(&mut v);

    v.y
}

/// `PVG_FT_Vector_Unit`
pub fn pvg_ft_vector_unit(vec: &mut PvgFtVector, angle: PvgFtAngle) {
    vec.x = (PVG_FT_TRIG_SCALE >> 8) as i64;
    vec.y = 0;
    ft_trig_pseudo_rotate(vec, angle);
    vec.x = vec.x.wrapping_add(0x80) >> 8;
    vec.y = vec.y.wrapping_add(0x80) >> 8;
}

/// `PVG_FT_Vector_Rotate`
pub fn pvg_ft_vector_rotate(vec: &mut PvgFtVector, angle: PvgFtAngle) {
    let mut v = *vec;

    if v.x == 0 && v.y == 0 {
        return;
    }

    let mut shift = ft_trig_prenorm(&mut v);
    ft_trig_pseudo_rotate(&mut v, angle);
    v.x = ft_trig_downscale(v.x);
    v.y = ft_trig_downscale(v.y);

    if shift > 0 {
        let half: i32 = 1i32.wrapping_shl((shift - 1) as u32);

        vec.x = v.x.wrapping_add(half as i64).wrapping_sub((v.x < 0) as i64) >> shift;
        vec.y = v.y.wrapping_add(half as i64).wrapping_sub((v.y < 0) as i64) >> shift;
    } else {
        shift = -shift;
        vec.x = (v.x as u64).wrapping_shl(shift as u32) as i64;
        vec.y = (v.y as u64).wrapping_shl(shift as u32) as i64;
    }
}

/// `PVG_FT_Vector_Length`
pub fn pvg_ft_vector_length(vec: &PvgFtVector) -> PvgFtFixed {
    let mut v = *vec;

    /* handle trivial cases */
    if v.x == 0 {
        return pvg_ft_abs(v.y);
    } else if v.y == 0 {
        return pvg_ft_abs(v.x);
    }

    /* general case */
    let shift = ft_trig_prenorm(&mut v);
    ft_trig_pseudo_polarize(&mut v);

    v.x = ft_trig_downscale(v.x);

    if shift > 0 {
        return v.x.wrapping_add(1i64 << (shift - 1)) >> shift;
    }

    ((v.x as u32).wrapping_shl((-shift) as u32)) as PvgFtFixed
}

/// `PVG_FT_Vector_Polarize`
pub fn pvg_ft_vector_polarize(vec: &PvgFtVector, length: &mut PvgFtFixed, angle: &mut PvgFtAngle) {
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
        ((v.x as u32).wrapping_shl((-shift) as u32)) as PvgFtFixed
    };
    *angle = v.y;
}

/// `PVG_FT_Vector_From_Polar`
pub fn pvg_ft_vector_from_polar(vec: &mut PvgFtVector, length: PvgFtFixed, angle: PvgFtAngle) {
    vec.x = length;
    vec.y = 0;

    pvg_ft_vector_rotate(vec, angle);
}

/// `PVG_FT_Angle_Diff`
pub fn pvg_ft_angle_diff(angle1: PvgFtAngle, angle2: PvgFtAngle) -> PvgFtAngle {
    let mut delta = angle2.wrapping_sub(angle1);

    if delta <= -PVG_FT_ANGLE_PI {
        /* (C loops subtracting 2pi; this is its result) */
        delta = delta.wrapping_add(PVG_FT_ANGLE_2PI);
        if delta <= -PVG_FT_ANGLE_PI {
            let k = (-PVG_FT_ANGLE_PI - delta) / PVG_FT_ANGLE_2PI + 1;
            delta += k * PVG_FT_ANGLE_2PI;
        }
    }

    if delta > PVG_FT_ANGLE_PI {
        delta -= PVG_FT_ANGLE_2PI;
        if delta > PVG_FT_ANGLE_PI {
            let k = (delta - PVG_FT_ANGLE_PI - 1) / PVG_FT_ANGLE_2PI + 1;
            delta -= k * PVG_FT_ANGLE_2PI;
        }
    }

    delta
}
