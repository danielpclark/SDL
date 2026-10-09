// Rust translation of src/sdf/ftsdfcommon.c (and ftsdfcommon.h) from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2020-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Written by Anuj Verma.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auxiliary data for Signed Distance Field support (body).
//!
//! This file contains common functions and properties for both the 'sdf'
//! and 'bsdf' renderers.

use super::super::base::ftcalc::ft_div_fix;
use super::super::fttypes::*;

/**************************************************************************
 *
 * default values (cannot be set individually for each renderer)
 *
 */

/// default spread value
pub const DEFAULT_SPREAD: FtUInt = 8;
/// minimum spread supported by the renderer
pub const MIN_SPREAD: FtUInt = 2;
/// maximum spread supported by the renderer
pub const MAX_SPREAD: FtUInt = 32;
/// pixel size in 26.6
pub const ONE_PIXEL: FtPos = 1 << 6;

/**************************************************************************
 *
 * common definitions (cannot be set individually for each renderer)
 *
 */

/* If this macro is set to 1 the rasterizer uses squared distances for */
/* computation.  It can greatly improve the performance but there is a */
/* chance of overflow and artifacts.  You can safely use it up to a    */
/* pixel size of 128.                                                  */
/// `USE_SQUARED_DISTANCES`
pub const USE_SQUARED_DISTANCES: bool = false;

/**************************************************************************
 *
 * common macros
 *
 */

/// `FT_INT_26D6`: convert int to 26.6 fixed-point
#[inline]
pub fn ft_int_26d6(x: FtPos) -> FtPos {
    x * 64
}

/// `FT_INT_16D16`: convert int to 16.16 fixed-point
#[inline]
pub fn ft_int_16d16(x: FtPos) -> FtPos {
    x * 65536
}

/// `FT_26D6_16D16`: convert 26.6 to 16.16 fixed-point
#[inline]
pub fn ft_26d6_16d16(x: FtPos) -> FtPos {
    x.wrapping_mul(1024)
}

/* FT_CALL: the `?' operator */

/*
 * The macro `VECTOR_LENGTH_16D16` computes either squared distances or
 * actual distances, depending on the value of `USE_SQUARED_DISTANCES`.
 *
 * By using squared distances the performance can be greatly improved but
 * there is a risk of overflow.
 */
/// `VECTOR_LENGTH_16D16` (`USE_SQUARED_DISTANCES` is 0)
#[inline]
pub fn vector_length_16d16(v: &FtVector) -> FtFixed {
    super::super::base::fttrigon::ft_vector_length(v)
}

/**************************************************************************
 *
 * common typedefs
 *
 */

/// with 26.6 fixed-point components
pub type Ft26D6Vec = FtVector;
/// with 16.16 fixed-point components
pub type Ft16D16Vec = FtVector;
/// 16.16 fixed-point representation
pub type Ft16D16 = FtInt32;
/// 26.6 fixed-point representation
pub type Ft26D6 = FtInt32;
/// format to represent SDF data
pub type FtSdfFormat = u8;
/// control box of a curve
pub type FtCBox = FtBBox;

/**************************************************************************
 *
 * common functions
 *
 */

/*
 * Original algorithm:
 *
 *   https://github.com/chmike/fpsqrt
 *
 * Use this to compute the square root of a 16.16 fixed-point number.
 */
/// `square_root`
pub fn square_root(val: Ft16D16) -> Ft16D16 {
    let mut t: FtULong;
    let mut q: FtULong;
    let mut b: FtULong;
    let mut r: FtULong;

    r = val as FtULong;
    b = 0x40000000;
    q = 0;

    while b > 0x40 {
        t = q.wrapping_add(b);
        if r >= t {
            r = r.wrapping_sub(t);
            q = t.wrapping_add(b);
        }
        r = r.wrapping_shl(1);
        b >>= 1;
    }
    q >>= 8;

    q as Ft16D16
}

/**************************************************************************
 *
 * format and sign manipulating functions
 *
 */

/*
 * Convert 16.16 fixed-point values to the desired output format.
 * In this case we reduce 16.16 fixed-point values to normalized
 * 8-bit values.
 *
 * The `max_value` in the parameter is the maximum value in the
 * distance field map and is equal to the spread.  We normalize
 * the distances using this value instead of computing the maximum
 * value for the entire bitmap.
 *
 * You can use this function to map the 16.16 signed values to any
 * format required.  Do note that the output buffer is 8-bit, so only
 * use an 8-bit format for `FT_SDFFormat`, or increase the buffer size in
 * `ftsdfrend.c`.
 */
/// `map_fixed_to_sdf`
pub fn map_fixed_to_sdf(dist: Ft16D16, max_value: Ft16D16) -> FtSdfFormat {
    let out: FtSdfFormat;
    let mut udist: Ft16D16;

    /* normalize the distance values */
    let dist = ft_div_fix(dist as FtLong, max_value as FtLong) as Ft16D16;

    udist = if dist < 0 { dist.wrapping_neg() } else { dist };

    /* Reduce the distance values to 8 bits.                   */
    /*                                                         */
    /* Since +1/-1 in 16.16 takes the 16th bit, we right-shift */
    /* the number by 9 to make it fit into the 7-bit range.    */
    /*                                                         */
    /* One bit is reserved for the sign.                       */
    udist >>= 9;

    /* Since `char` can only store a maximum positive value    */
    /* of 127 we need to make sure it does not wrap around and */
    /* give a negative value.                                  */
    if dist > 0 && udist > 127 {
        udist = 127;
    }
    if dist < 0 && udist > 128 {
        udist = 128;
    }

    /* Output the data; negative values are from [0, 127] and positive    */
    /* from [128, 255].  One important thing is that negative values      */
    /* are inverted here, that means [0, 128] maps to [-128, 0] linearly. */
    /* More on that in `freetype.h` near the documentation of             */
    /* `FT_RENDER_MODE_SDF`.                                              */
    out = if dist < 0 {
        128u8.wrapping_sub(udist as FtSdfFormat)
    } else {
        (udist as FtSdfFormat).wrapping_add(128)
    };

    out
}

/*
 * Invert the signed distance packed into the corresponding format.
 * So if the values are negative they will become positive in the
 * chosen format.
 *
 * [Note]: This function should only be used after converting the
 *         16.16 signed distance values to `FT_SDFFormat`.  If that
 *         conversion has not been done, then simply invert the sign
 *         and use the above function to pack the values.
 */
/// `invert_sign`
pub fn invert_sign(dist: FtSdfFormat) -> FtSdfFormat {
    255 - dist
}
