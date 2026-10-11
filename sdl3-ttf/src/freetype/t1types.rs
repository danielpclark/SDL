// Rust translation of include/freetype/internal/t1types.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Basic Type1/Type2 type definitions and interface (specification
//! only).
//!
//! The font's subroutines, charstrings and glyph names are their loaded
//! `PS_Table`s (C's `subrs_block`, `subrs` and `subrs_len`, and so on);
//! the encoding's character names are glyph indices (`None` for C's
//! `".notdef"` literal). The modules the face records (`psnames`,
//! `psaux`, `pshinter`) are called directly; the face also keeps the
//! driver's glyph slot and size additions.

#![allow(non_snake_case)]

use std::collections::HashMap;

use super::base::ftobjs::FtFaceRec;
use super::cfftypes::CffGlyphSlotRec;
use super::fttypes::*;
use super::psaux::psobjs::PsTableData;
use super::pshinter::pshglob::PshGlobalsRec;
use super::t1tables::*;

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/***                                                                   ***/
/***                                                                   ***/
/***              REQUIRED TYPE1/TYPE2 TABLES DEFINITIONS              ***/
/***                                                                   ***/
/***                                                                   ***/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/// `T1_EncodingRec`: a structure modeling a custom encoding.
///
/// * `num_chars`: the number of character codes in the encoding.
///   Usually 256.
/// * `code_first`: the lowest valid character code in the encoding.
/// * `code_last`: the highest valid character code in the encoding + 1.
///   When equal to code_first there are no valid character codes.
/// * `char_index`: an array of corresponding glyph indices.
/// * `char_name`: an array of corresponding glyph names (the glyphs'
///   indices; `None` for `".notdef"`).
#[derive(Debug, Clone, Default)]
pub struct T1EncodingRec {
    pub num_chars: FtInt,
    pub code_first: FtInt,
    pub code_last: FtInt,

    pub char_index: Vec<FtUShort>,
    pub char_name: Vec<Option<usize>>,
}

/// `PS_FontExtraRec`: used to hold extra data of `PS_FontInfoRec` that
/// cannot be stored in the publicly defined structure.
///
/// Note these can't be blended with multiple-masters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PsFontExtraRec {
    pub fs_type: FtUShort,
}

/// `T1_FontRec`
#[derive(Debug, Clone, Default)]
pub struct T1FontRec {
    pub font_info: PsFontInfoRec,   /* font info dictionary   */
    pub font_extra: PsFontExtraRec, /* font info extra fields */
    pub private_dict: PsPrivateRec, /* private dictionary     */
    pub font_name: Option<Vec<u8>>, /* top-level dictionary   */

    pub encoding_type: T1EncodingType,
    pub encoding: T1EncodingRec,

    pub num_subrs: FtInt,
    /// `subrs_block`, `subrs` and `subrs_len` (`None` without subroutines)
    pub subrs: Option<PsTableData>,
    pub subrs_hash: Option<HashMap<FtInt, usize>>,

    pub num_glyphs: FtInt,
    /// `glyph_names_block` and `glyph_names`: array of glyph names (C
    /// strings)
    pub glyph_names: PsTableData,
    /// `charstrings_block`, `charstrings` and `charstrings_len`: array of
    /// glyph charstrings
    pub charstrings: PsTableData,

    pub paint_type: FtByte,
    pub font_type: FtByte,
    pub font_matrix: FtMatrix,
    pub font_offset: FtVector,
    pub font_bbox: FtBBox,
    pub font_id: FtLong,

    pub stroke_width: FtFixed,
}

/// `CID_SubrsRec` (`code` is one block of the subroutines, C's array of
/// pointers into it the subroutines' offsets, with one more for the end)
#[derive(Debug, Clone, Default)]
pub struct CidSubrsRec {
    pub num_subrs: FtInt,
    pub code: Option<std::sync::Arc<[usize]>>,
    pub code_bytes: Option<std::sync::Arc<[u8]>>,
}

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/***                                                                   ***/
/***                                                                   ***/
/***                AFM FONT INFORMATION STRUCTURES                    ***/
/***                                                                   ***/
/***                                                                   ***/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/// `AFM_TrackKernRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfmTrackKernRec {
    pub degree: FtInt,
    pub min_ptsize: FtFixed,
    pub min_kern: FtFixed,
    pub max_ptsize: FtFixed,
    pub max_kern: FtFixed,
}

/// `AFM_KernPairRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfmKernPairRec {
    pub index1: FtUInt,
    pub index2: FtUInt,
    pub x: FtInt,
    pub y: FtInt,
}

/// `AFM_FontInfoRec`
#[derive(Debug, Clone, Default)]
pub struct AfmFontInfoRec {
    pub IsCIDFont: FtBool,
    pub FontBBox: FtBBox,
    pub Ascender: FtFixed,                /* optional, mind the zero */
    pub Descender: FtFixed,               /* optional, mind the zero */
    pub TrackKerns: Vec<AfmTrackKernRec>, /* free if non-NULL */
    pub NumTrackKern: FtUInt,
    pub KernPairs: Vec<AfmKernPairRec>, /* free if non-NULL */
    pub NumKernPair: FtUInt,
}

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/***                                                                   ***/
/***                                                                   ***/
/***                ORIGINAL T1_FACE CLASS DEFINITION                  ***/
/***                                                                   ***/
/***                                                                   ***/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/// The fields of `T1_FaceRec` that its dictionary can set (as
/// `T1_FIELD_LOCATION_FACE` fields): undocumented, optional: indices of
/// subroutines that express the NormalizeDesignVector and the
/// ConvertDesignVector procedure, respectively, as Type 2 charstrings;
/// -1 if keywords not present.
#[derive(Debug, Clone, Copy, Default)]
pub struct T1FaceIndicesRec {
    pub ndv_idx: FtInt,
    pub cdv_idx: FtInt,
}

/// `T1_FaceRec`
///
/// `glyph` and `size` are the Type 1 driver's additions to its glyph slot
/// (`T1_GlyphSlotRec`: `hint`, `scaled`, `x_scale`, `y_scale`, which
/// `CFF_GlyphSlotRec` shares, and the unused `max_points` and
/// `max_contours`) and size (`T1_SizeRec`'s `internal->module_data`,
/// the PS hinter's globals).
#[derive(Debug, Default)]
pub struct T1FaceRec {
    pub root: FtFaceRec,
    pub type1: T1FontRec,
    /// (`psnames`: whether the `psnames' module is there)
    pub psnames: bool,
    /// (`psaux`: whether the `psaux' module is there)
    pub psaux: bool,
    pub afm_data: Option<Box<AfmFontInfoRec>>,

    /* support for Multiple Masters fonts */
    pub blend: Option<Box<PsBlendRec>>,

    /* undocumented, optional: indices of subroutines that express      */
    /* the NormalizeDesignVector and the ConvertDesignVector procedure, */
    /* respectively, as Type 2 charstrings; -1 if keywords not present  */
    pub indices: T1FaceIndicesRec,

    /* undocumented, optional: has the same meaning as len_buildchar */
    /* for Type 2 fonts; manipulated by othersubrs 19, 24, and 25    */
    pub len_buildchar: FtUInt,
    pub buildchar: Vec<FtLong>,

    /* since version 2.1 - interface to PostScript hinter */
    /// (`pshinter`: whether the `pshinter' module is there)
    pub pshinter: bool,

    /// `T1_GlyphSlotRec` (minus its root, the face's slot)
    pub glyph: CffGlyphSlotRec,
    pub glyph_max_points: FtInt,
    pub glyph_max_contours: FtInt,
    /// `T1_SizeRec`'s `internal->module_data`
    pub size_module_data: Option<Box<PshGlobalsRec>>,
}
