// Rust translation of src/autofit/aftypes.h (with afws-iter.h and afcover.h)
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter types.
//!
//! The auto-fitter is a complete rewrite of the old auto-hinter.
//! Its main feature is the ability to differentiate between different
//! writing systems and scripts in order to apply specific rules.
//!
//! The code has also been compartmentalized into several entities that
//! should make algorithmic experimentation easier than with the old
//! code.
//!
//! Translation notes:
//!
//! * The style metrics (`AF_StyleMetricsRec` and the records of the
//!   writing systems that derive from it, `AF_LatinMetricsRec` and
//!   `AF_CJKMetricsRec`) are one structure, [`AfStyleMetricsRec`], whose
//!   writing-system part is an enumeration.
//! * The scaler keeps what the auto-fitter reads of its face (the units
//!   per EM, the size's horizontal ppem and the style flags), taken when
//!   the scaler is set up, instead of the face pointer; the face doesn't
//!   change while a glyph is loaded.
//! * The face globals (`metrics->globals`), the face, and the module are
//!   passed to the functions that need them.

use super::super::base::ftobjs::{FtFace, FtFaceRec};
use super::super::fttypes::*;
use super::afblue::{AfBlueString, AfBlueStringset};
use super::afcjk::AfCjkMetricsRec;
use super::afglobal::AfFaceGlobalsRec;
use super::afhints::AfGlyphHintsRec;
use super::aflatin::AfLatinMetricsRec;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                 U T I L I T Y   S T U F F                     *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `AF_WidthRec`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfWidthRec {
    pub org: FtPos, /* original position/width in font units             */
    pub cur: FtPos, /* current/scaled position/width in device subpixels */
    pub fit: FtPos, /* current/fitted position/width in device subpixels */
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                       S C A L E R S                           *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/*
 * A scaler models the target pixel device that will receive the
 * auto-hinted glyph image.
 */

pub const AF_SCALER_FLAG_NO_HORIZONTAL: FtUInt32 = 1; /* disable horizontal hinting */
pub const AF_SCALER_FLAG_NO_VERTICAL: FtUInt32 = 2; /* disable vertical hinting   */
pub const AF_SCALER_FLAG_NO_ADVANCE: FtUInt32 = 4; /* disable advance hinting    */

/// What the auto-fitter reads of a scaler's face (`scaler->face`).
#[derive(Debug, Clone, Copy, Default)]
pub struct AfScalerFace {
    /// `face->units_per_EM`
    pub units_per_EM: FtUShort,
    /// `face->size->metrics.x_ppem`
    pub x_ppem: FtUShort,
    /// `face->style_flags`
    pub style_flags: FtLong,
}

impl AfScalerFace {
    /// The values of `face`.
    pub fn of(face: &FtFaceRec) -> AfScalerFace {
        AfScalerFace {
            units_per_EM: face.units_per_EM,
            x_ppem: face.size.metrics.x_ppem,
            style_flags: face.style_flags,
        }
    }
}

/// `AF_ScalerRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfScalerRec {
    pub face: AfScalerFace,        /* source font face                      */
    pub x_scale: FtFixed,          /* from font units to 1/64 device pixels */
    pub y_scale: FtFixed,          /* from font units to 1/64 device pixels */
    pub x_delta: FtPos,            /* in 1/64 device pixels                 */
    pub y_delta: FtPos,            /* in 1/64 device pixels                 */
    pub render_mode: FtRenderMode, /* monochrome, anti-aliased, LCD, etc.   */
    pub flags: FtUInt32,           /* additional control flags, see above   */
}

/// `AF_SCALER_EQUAL_SCALES`
pub fn af_scaler_equal_scales(a: &AfScalerRec, b: &AfScalerRec) -> bool {
    a.x_scale == b.x_scale
        && a.y_scale == b.y_scale
        && a.x_delta == b.x_delta
        && a.y_delta == b.y_delta
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                W R I T I N G   S Y S T E M S                  *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/*
 * For the auto-hinter, a writing system consists of multiple scripts that
 * can be handled similarly *in a typographical way*; the relationship is
 * not based on history.  For example, both the Greek and the unrelated
 * Armenian scripts share the same features like ascender, descender,
 * x-height, etc.  Essentially, a writing system is covered by a
 * submodule of the auto-fitter; it contains
 *
 * - a specific global analyzer that computes global metrics specific to
 *   the script (based on script-specific characters to identify ascender
 *   height, x-height, etc.),
 *
 * - a specific glyph analyzer that computes segments and edges for each
 *   glyph covered by the script,
 *
 * - a specific grid-fitting algorithm that distorts the scaled glyph
 *   outline according to the results of the glyph analyzer.
 */

/// `AF_WritingSystem`: the list of known writing systems (`afws-iter.h`)
pub type AfWritingSystem = u32;
pub const AF_WRITING_SYSTEM_DUMMY: AfWritingSystem = 0;
pub const AF_WRITING_SYSTEM_LATIN: AfWritingSystem = 1;
pub const AF_WRITING_SYSTEM_CJK: AfWritingSystem = 2;
pub const AF_WRITING_SYSTEM_INDIC: AfWritingSystem = 3;
pub const AF_WRITING_SYSTEM_MAX: AfWritingSystem = 4; /* do not remove */

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                        S C R I P T S                          *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/*
 * Each script is associated with two sets of Unicode ranges to test
 * whether the font face supports the script, and which non-base
 * characters the script contains.
 *
 * We use four-letter script tags from the OpenType specification,
 * extended by `NONE', which indicates `no script'.
 */

/// `AF_Script` (the values are in `afscript.rs`)
pub type AfScript = u32;

/// `AF_Script_UniRangeRec`
#[derive(Debug, Clone, Copy)]
pub struct AfScriptUniRangeRec {
    pub first: FtUInt32,
    pub last: FtUInt32,
}

/// `AF_ScriptClassRec`
#[derive(Debug)]
pub struct AfScriptClassRec {
    pub script: AfScript,

    /* last element in the ranges must be { 0, 0 } */
    pub script_uni_ranges: &'static [AfScriptUniRangeRec],
    pub script_uni_nonbase_ranges: &'static [AfScriptUniRangeRec],

    pub top_to_bottom_hinting: bool,

    /// for default width and height (NUL-terminated)
    pub standard_charstring: &'static [u8],
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                      C O V E R A G E S                        *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/*
 * Usually, a font contains more glyphs than can be addressed by its
 * character map.
 *
 * In the PostScript font world, encoding vectors specific to a given
 * task are used to select such glyphs, and these glyphs can be often
 * recognized by having a suffix in its glyph names.  For example, a
 * superscript glyph `A' might be called `A.sup'.  Unfortunately, this
 * naming scheme is not standardized and thus unusable for us.
 *
 * In the OpenType world, a better solution was invented, namely
 * `features', which cleanly separate a character's input encoding from
 * the corresponding glyph's appearance, and which don't use glyph names
 * at all.  For our purposes, and slightly generalized, an OpenType
 * feature is a name of a mapping that maps character codes to
 * non-standard glyph indices (features get used for other things also).
 * For example, the `sups' feature provides superscript glyphs, thus
 * mapping character codes like `A' or `B' to superscript glyph
 * representation forms.  How this mapping happens is completely
 * uninteresting to us.
 *
 * For the auto-hinter, a `coverage' represents all glyphs of an OpenType
 * feature collected in a set (as listed below) that can be hinted
 * together.  To continue the above example, superscript glyphs must not
 * be hinted together with normal glyphs because the blue zones
 * completely differ.
 *
 * Note that FreeType itself doesn't compute coverages; it only provides
 * the glyphs addressable by the default Unicode character map.  Instead,
 * we use the HarfBuzz library (if available), which has many functions
 * exactly for this purpose.
 *
 * AF_COVERAGE_DEFAULT is special: It should cover everything that isn't
 * listed separately (including the glyphs addressable by the character
 * map).  In case HarfBuzz isn't available, it exactly covers the glyphs
 * addressable by the character map.
 *
 */

/// `AF_Coverage` (the coverages of `afcover.h`)
pub type AfCoverage = u32;
pub const AF_COVERAGE_PETITE_CAPITALS_FROM_CAPITALS: AfCoverage = 0;
pub const AF_COVERAGE_SMALL_CAPITALS_FROM_CAPITALS: AfCoverage = 1;
pub const AF_COVERAGE_ORDINALS: AfCoverage = 2;
pub const AF_COVERAGE_PETITE_CAPITALS: AfCoverage = 3;
pub const AF_COVERAGE_RUBY: AfCoverage = 4;
pub const AF_COVERAGE_SCIENTIFIC_INFERIORS: AfCoverage = 5;
pub const AF_COVERAGE_SMALL_CAPITALS: AfCoverage = 6;
pub const AF_COVERAGE_SUBSCRIPT: AfCoverage = 7;
pub const AF_COVERAGE_SUPERSCRIPT: AfCoverage = 8;
pub const AF_COVERAGE_TITLING: AfCoverage = 9;
pub const AF_COVERAGE_DEFAULT: AfCoverage = 10;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                         S T Y L E S                           *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/*
 * The topmost structure for modelling the auto-hinter glyph input data
 * is a `style class', grouping everything together.
 */

/// `AF_Style` (the values are in `afstyles.rs`)
pub type AfStyle = u32;

/// `AF_StyleClassRec`
#[derive(Debug)]
pub struct AfStyleClassRec {
    pub style: AfStyle,

    pub writing_system: AfWritingSystem,
    pub script: AfScript,
    pub blue_stringset: AfBlueStringset,
    pub coverage: AfCoverage,
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                   S T Y L E   M E T R I C S                   *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

pub const AF_HINTING_BOTTOM_TO_TOP: bool = false;
pub const AF_HINTING_TOP_TO_BOTTOM: bool = true;

/// `AF_Blue_StringRec` (declared in `afblue.h`)
#[derive(Debug, Clone, Copy)]
pub struct AfBlueStringRec {
    pub string: AfBlueString,
    pub properties: FtUShort,
}

/* This is the main structure that combines everything.  Autofit modules */
/* specific to writing systems derive their structures from it, for      */
/* example `AF_LatinMetrics'.                                            */

/// `AF_StyleMetricsRec`, with the records the writing systems derive
/// from it (`AF_LatinMetricsRec`, `AF_CJKMetricsRec`) in `ws`
#[derive(Debug, Clone)]
pub struct AfStyleMetricsRec {
    pub style_class: &'static AfStyleClassRec,
    pub scaler: AfScalerRec,
    pub digits_have_same_width: bool,

    /* `globals' (to access properties) is passed to the functions */
    /// the writing system's part of the metrics
    pub ws: AfWritingSystemMetrics,
}

/// The part of the style metrics specific to a writing system.
#[derive(Debug, Clone)]
pub enum AfWritingSystemMetrics {
    /// `sizeof ( AF_StyleMetricsRec )`: the dummy writing system's
    Dummy,
    /// `AF_LatinMetricsRec`
    Latin(Box<AfLatinMetricsRec>),
    /// `AF_CJKMetricsRec` (also the Indic writing system's)
    Cjk(Box<AfCjkMetricsRec>),
}

impl AfStyleMetricsRec {
    /// `(AF_LatinMetrics)metrics`
    pub fn latin(&self) -> &AfLatinMetricsRec {
        match &self.ws {
            AfWritingSystemMetrics::Latin(m) => m,
            _ => panic!("not latin metrics"),
        }
    }
    /// `(AF_LatinMetrics)metrics`
    pub fn latin_mut(&mut self) -> &mut AfLatinMetricsRec {
        match &mut self.ws {
            AfWritingSystemMetrics::Latin(m) => m,
            _ => panic!("not latin metrics"),
        }
    }
    /// `(AF_CJKMetrics)metrics`
    pub fn cjk(&self) -> &AfCjkMetricsRec {
        match &self.ws {
            AfWritingSystemMetrics::Cjk(m) => m,
            _ => panic!("not CJK metrics"),
        }
    }
    /// `(AF_CJKMetrics)metrics`
    pub fn cjk_mut(&mut self) -> &mut AfCjkMetricsRec {
        match &mut self.ws {
            AfWritingSystemMetrics::Cjk(m) => m,
            _ => panic!("not CJK metrics"),
        }
    }
    /// `( (AF_LatinMetrics)(metrics) )->units_per_em` (which C also reads
    /// from CJK metrics, whose layout is the same up to there)
    pub fn units_per_em(&self) -> FtUInt {
        match &self.ws {
            AfWritingSystemMetrics::Latin(m) => m.units_per_em,
            AfWritingSystemMetrics::Cjk(m) => m.units_per_em,
            AfWritingSystemMetrics::Dummy => 0,
        }
    }
}

/// `AF_LATIN_CONSTANT`: constants are given with units_per_em == 2048 in
/// mind
pub fn af_latin_constant(metrics: &AfStyleMetricsRec, c: FtLong) -> FtLong {
    (c * metrics.units_per_em() as FtLong) / 2048
}

/* Declare and define vtables for classes */

/// `AF_WritingSystem_InitMetricsFunc`: This function parses an FT_Face to
/// compute global metrics for a specific style (the internal error code
/// -1 says that no blue zones have been found).
pub type AfWritingSystemInitMetricsFunc = fn(
    metrics: &mut AfStyleMetricsRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
) -> FtResult<()>;

/// `AF_WritingSystem_ScaleMetricsFunc`
pub type AfWritingSystemScaleMetricsFunc =
    fn(metrics: &mut AfStyleMetricsRec, scaler: &AfScalerRec, globals: &AfFaceGlobalsRec);

/// `AF_WritingSystem_DoneMetricsFunc`
pub type AfWritingSystemDoneMetricsFunc = fn(metrics: &mut AfStyleMetricsRec);

/// `AF_WritingSystem_GetStdWidthsFunc`: returns `(stdHW, stdVW)`
pub type AfWritingSystemGetStdWidthsFunc = fn(metrics: &AfStyleMetricsRec) -> (FtPos, FtPos);

/// `AF_WritingSystem_InitHintsFunc`
pub type AfWritingSystemInitHintsFunc =
    fn(hints: &mut AfGlyphHintsRec, metrics: &AfStyleMetricsRec) -> FtResult<()>;

/// `AF_WritingSystem_ApplyHintsFunc`
pub type AfWritingSystemApplyHintsFunc = fn(
    glyph_index: FtUInt,
    hints: &mut AfGlyphHintsRec,
    outline: &mut FtOutline,
    metrics: &AfStyleMetricsRec,
    globals: &AfFaceGlobalsRec,
) -> FtResult<()>;

/// `AF_WritingSystemClassRec`
#[derive(Debug)]
pub struct AfWritingSystemClassRec {
    pub writing_system: AfWritingSystem,

    /* `style_metrics_size': the variant of `AfWritingSystemMetrics' */
    pub style_metrics_init: Option<AfWritingSystemInitMetricsFunc>,
    pub style_metrics_scale: Option<AfWritingSystemScaleMetricsFunc>,
    pub style_metrics_done: Option<AfWritingSystemDoneMetricsFunc>,
    pub style_metrics_getstdw: Option<AfWritingSystemGetStdWidthsFunc>,

    pub style_hints_init: Option<AfWritingSystemInitHintsFunc>,
    pub style_hints_apply: Option<AfWritingSystemApplyHintsFunc>,
}
