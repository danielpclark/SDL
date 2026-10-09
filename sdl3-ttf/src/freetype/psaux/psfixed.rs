// Rust translation of src/psaux/psfixed.h and src/psaux/pstypes.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright 2007-2013 Adobe Systems Incorporated.
//
// This software, and all works of authorship, whether in source or
// object code form as indicated by the copyright notice(s) included
// herein (collectively, the "Work") is made available, and may only be
// used, modified, and distributed under the FreeType Project License,
// LICENSE.TXT.  Additionally, subject to the terms and conditions of the
// FreeType Project License, each contributor to the Work hereby grants
// to any individual or legal entity exercising permissions granted by
// the FreeType Project License and this section (hereafter, "You" or
// "Your") a perpetual, worldwide, non-exclusive, no-charge,
// royalty-free, irrevocable (except as stated in this section) patent
// license to make, have made, use, offer to sell, sell, import, and
// otherwise transfer the Work, where such license applies only to those
// patent claims licensable by such contributor that are necessarily
// infringed by their contribution(s) alone or by combination of their
// contribution(s) with the Work to which such contribution(s) was
// submitted.  If You institute patent litigation against any entity
// (including a cross-claim or counterclaim in a lawsuit) alleging that
// the Work or a contribution incorporated within the Work constitutes
// direct or contributory patent infringement, then any patent licenses
// granted to You under this License for that Work shall terminate as of
// the date such litigation is filed.
//
// By using, modifying, or distributing the Work you indicate that you
// have read and understood the terms and conditions of the
// FreeType Project License as well as those provided in this section,
// and you accept them fully.
//
// This is an altered (translated) version of the original software; the
// FreeType Project License is in FTL.TXT (see also LICENSE.txt).

//! Adobe's code for Fixed Point Mathematics (specification only), and the
//! data models the engine expects.

use super::super::fttypes::*;

/* integers at least 32 bits wide */
pub type Cf2UInt = u32;
pub type Cf2Int = i32;

/* fixed-float numbers */
pub type Cf2F16Dot16 = FtInt32;

/* rasterizer integer and fixed-point arithmetic must be 32-bit */

/// `CF2_Fixed`
pub type Cf2Fixed = Cf2F16Dot16;
/// `CF2_Frac`: 2.30 fixed-point
pub type Cf2Frac = FtInt32;

pub const CF2_FIXED_MAX: Cf2Fixed = 0x7FFFFFFF;
pub const CF2_FIXED_MIN: Cf2Fixed = 0x80000000u32 as Cf2Fixed;
pub const CF2_FIXED_ONE: Cf2Fixed = 0x10000;
pub const CF2_FIXED_EPSILON: Cf2Fixed = 0x0001;

/* in C 89, left and right shift of negative numbers is  */
/* implementation specific behaviour in the general case */

/// `cf2_intToFixed`
#[inline]
pub const fn cf2_int_to_fixed(i: i32) -> Cf2Fixed {
    ((i as FtUInt32) << 16) as Cf2Fixed
}
/// `cf2_fixedToInt`
#[inline]
pub const fn cf2_fixed_to_int(x: Cf2Fixed) -> FtShort {
    (((x as FtUInt32).wrapping_add(0x8000)) >> 16) as FtShort
}
/// `cf2_fixedRound`
#[inline]
pub const fn cf2_fixed_round(x: Cf2Fixed) -> Cf2Fixed {
    (((x as FtUInt32).wrapping_add(0x8000)) & 0xFFFF0000) as Cf2Fixed
}
/// `cf2_doubleToFixed`
#[inline]
pub fn cf2_double_to_fixed(f: f64) -> Cf2Fixed {
    (f * 65536.0 + 0.5) as Cf2Fixed
}
/// `cf2_fixedAbs`
#[inline]
pub const fn cf2_fixed_abs(x: Cf2Fixed) -> Cf2Fixed {
    if x < 0 {
        x.wrapping_neg()
    } else {
        x
    }
}
/// `cf2_fixedFloor`
#[inline]
pub const fn cf2_fixed_floor(x: Cf2Fixed) -> Cf2Fixed {
    ((x as FtUInt32) & 0xFFFF0000) as Cf2Fixed
}
/// `cf2_fixedFraction`
#[inline]
pub const fn cf2_fixed_fraction(x: Cf2Fixed) -> Cf2Fixed {
    x.wrapping_sub(cf2_fixed_floor(x))
}
/// `cf2_fracToFixed`
#[inline]
pub const fn cf2_frac_to_fixed(x: Cf2Frac) -> Cf2Fixed {
    (x.wrapping_add(0x2000).wrapping_sub((x < 0) as i32)) >> 14
}

/// `CF2_NumberType`: signed numeric types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cf2NumberType {
    #[default]
    Fixed, /* 16.16 */
    Frac, /*  2.30 */
    Int,  /* 32.0  */
}
