// Rust translation of src/psaux/psglue.h from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
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

//! Adobe's code for shared stuff (specification only).
//!
//! The outline callbacks are the FreeType client outline's (`psft`), which
//! the glyph path calls directly; their record keeps the winding
//! momentum, and their error is the font instance's.

use super::super::fttypes::*;
use super::psfixed::*;

/* rendering parameters */

/* apply hints to rendered glyphs */
pub const CF2_FLAGS_HINTED: Cf2Int = 1;
/* for testing */
pub const CF2_FLAGS_DARKENED: Cf2Int = 2;

/// `CF2_RenderingFlags`: type for holding the flags
pub type Cf2RenderingFlags = Cf2Int;

/* elements of a glyph outline */
/// `CF2_PathOp`
pub type Cf2PathOp = Cf2Int;
pub const CF2_PATH_OP_MOVE_TO: Cf2PathOp = 1; /* change the current point */
pub const CF2_PATH_OP_LINE_TO: Cf2PathOp = 2; /* line                     */
pub const CF2_PATH_OP_QUAD_TO: Cf2PathOp = 3; /* quadratic curve          */
pub const CF2_PATH_OP_CUBE_TO: Cf2PathOp = 4; /* cubic curve              */

/// `CF2_Matrix`: a matrix of fixed-point values
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cf2Matrix {
    pub a: Cf2F16Dot16,
    pub b: Cf2F16Dot16,
    pub c: Cf2F16Dot16,
    pub d: Cf2F16Dot16,
    pub tx: Cf2F16Dot16,
    pub ty: Cf2F16Dot16,
}

/// `CF2_CallbackParamsRec`: a common structure for all callback
/// parameters.
///
/// Some members may be unused.  For example, `pt0' is not used for
/// `moveTo' and `pt3' is not used for `quadTo'.  The initial point `pt0'
/// is included for each path element for generality; curve conversions
/// need it.  The `op' parameter allows one function to handle multiple
/// element types.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2CallbackParamsRec {
    pub pt0: FtVector,
    pub pt1: FtVector,
    pub pt2: FtVector,
    pub pt3: FtVector,

    pub op: Cf2Int,
}

/// `CF2_OutlineCallbacksRec` (its `moveTo`, `lineTo`, `quadTo` and
/// `cubeTo` are the `cf2_builder_*` functions of `psft`, and its `error`
/// the font instance's)
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2OutlineCallbacksRec {
    pub windingMomentum: Cf2Int, /* for winding order detection */
}
