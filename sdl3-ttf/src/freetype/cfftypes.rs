// Rust translation of include/freetype/internal/cfftypes.h and
// include/freetype/internal/cffotypes.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2017-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Basic OpenType/CFF type definitions and interface (specification
//! only).
//!
//! The C structures keep their fields. Pointers into the font data become
//! owned bytes (`bytes`, the string pool) and offsets into them (the
//! subroutine and string pointer tables); the bytes of a loaded index are
//! shared (`Arc`), so that the charstring interpreter can read subroutines
//! while it updates the font record. A `CFF_Blend`'s `font` pointer is
//! the flag whether it was set (its uses pass the font's variation store
//! instead), and a private dictionary's `subfont` back pointer likewise.
//!
//! `CFF_FaceRec` is `TT_FaceRec` (see `tttypes`), whose `cff` field holds
//! the face's `extra.data`; the CFF driver's size (`CFF_SizeRec`) and glyph
//! slot (`CFF_GlyphSlotRec`) additions are kept in the face too, as faces
//! have a single size and glyph slot here.

#![allow(non_snake_case)]

use std::sync::Arc;

use super::fttypes::*;
use super::pshinter::pshglob::PshGlobalsRec;

/// `CFF_IndexRec`: a structure used to model a CFF Index table.
///
/// * `has_stream`: C's non-NULL `stream` (the index was initialized).
/// * `start`: the position of the first index byte in the input stream.
/// * `count`: the number of elements in the index.
/// * `off_size`: the size in bytes of object offsets in index.
/// * `data_offset`: the position of first data byte in the index's bytes.
/// * `data_size`: the size of the data table in this index.
/// * `offsets`: a table of element offsets in the index.  Must be loaded
///   explicitly.
/// * `bytes`: if the index is loaded in memory, its bytes.
#[derive(Debug, Clone, Default)]
pub struct CffIndexRec {
    pub has_stream: bool,
    pub start: FtULong,
    pub hdr_size: FtUInt,
    pub count: FtUInt,
    pub off_size: FtByte,
    pub data_offset: FtULong,
    pub data_size: FtULong,

    pub offsets: Option<Vec<FtULong>>,
    pub bytes: Option<Arc<[u8]>>,
}

/// `CFF_EncodingRec`
#[derive(Debug, Clone)]
pub struct CffEncodingRec {
    pub format: FtUInt,
    pub offset: FtULong,

    pub count: FtUInt,
    pub sids: [FtUShort; 256], /* avoid dynamic allocations */
    pub codes: [FtUShort; 256],
}

impl Default for CffEncodingRec {
    fn default() -> Self {
        CffEncodingRec {
            format: 0,
            offset: 0,
            count: 0,
            sids: [0; 256],
            codes: [0; 256],
        }
    }
}

/// `CFF_CharsetRec`
#[derive(Debug, Clone, Default)]
pub struct CffCharsetRec {
    pub format: FtUInt,
    pub offset: FtULong,

    pub sids: Option<Vec<FtUShort>>,
    pub cids: Option<Vec<FtUShort>>, /* the inverse mapping of `sids'; only needed */
    /* for CID-keyed fonts                        */
    pub max_cid: FtUInt,
    pub num_glyphs: FtUInt,
}

/* cf. similar fields in file `ttgxvar.h' from the `truetype' module */

/// `CFF_VarData`
#[derive(Debug, Clone, Default)]
pub struct CffVarData {
    pub regionIdxCount: FtUInt, /* number of region indexes           */
    pub regionIndices: Vec<FtUInt>, /* array of `regionIdxCount' indices; */
                                /* these index `varRegionList'        */
}

/// `CFF_AxisCoords`: contribution of one axis to a region
#[derive(Debug, Clone, Copy, Default)]
pub struct CffAxisCoords {
    pub startCoord: FtFixed,
    pub peakCoord: FtFixed, /* zero peak means no effect (factor = 1) */
    pub endCoord: FtFixed,
}

/// `CFF_VarRegion`
#[derive(Debug, Clone, Default)]
pub struct CffVarRegion {
    pub axisList: Vec<CffAxisCoords>, /* array of axisCount records */
}

/// `CFF_VStoreRec`
#[derive(Debug, Clone, Default)]
pub struct CffVStoreRec {
    pub dataCount: FtUInt,
    pub varData: Vec<CffVarData>, /* array of dataCount records      */
    /* vsindex indexes this array      */
    pub axisCount: FtUShort,
    pub regionCount: FtUInt, /* total number of regions defined */
    pub varRegionList: Vec<CffVarRegion>,
}

/// `CFF_BlendRec`: this object manages one cached blend vector.
///
/// There is a BlendRec for Private DICT parsing in each subfont
/// and a BlendRec for charstrings in CF2_Font instance data.
/// A cached BV may be used across DICTs or Charstrings if inputs
/// have not changed.
///
/// `usedBV' is reset at the start of each parse or charstring.
/// vsindex cannot be changed after a BV is used.
///
/// Note: NDV is long (32/64 bit), while BV is 16.16 (FT_Int32).
#[derive(Debug, Clone, Default)]
pub struct CffBlendRec {
    pub builtBV: bool,         /* blendV has been built           */
    pub usedBV: bool,          /* blendV has been used            */
    pub font: bool,            /* top level font struct (set)     */
    pub lastVsindex: FtUInt,   /* last vsindex used               */
    pub lenNDV: FtUInt,        /* normDV length (aka numAxes)     */
    pub lastNDV: Vec<FtFixed>, /* last NDV used                   */
    pub lenBV: FtUInt,         /* BlendV length (aka numMasters)  */
    pub BV: Vec<FtInt32>,      /* current blendV (per DICT/glyph) */
}

/// `CFF_FontRecDictRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct CffFontRecDictRec {
    pub version: FtUInt,
    pub notice: FtUInt,
    pub copyright: FtUInt,
    pub full_name: FtUInt,
    pub family_name: FtUInt,
    pub weight: FtUInt,
    pub is_fixed_pitch: FtByte,
    pub italic_angle: FtFixed,
    pub underline_position: FtFixed,
    pub underline_thickness: FtFixed,
    pub paint_type: FtInt,
    pub charstring_type: FtInt,
    pub font_matrix: FtMatrix,
    pub has_font_matrix: bool,
    pub units_per_em: FtULong, /* temporarily used as scaling value also */
    pub font_offset: FtVector,
    pub unique_id: FtULong,
    pub font_bbox: FtBBox,
    pub stroke_width: FtPos,
    pub charset_offset: FtULong,
    pub encoding_offset: FtULong,
    pub charstrings_offset: FtULong,
    pub private_offset: FtULong,
    pub private_size: FtULong,
    pub synthetic_base: FtLong,
    pub embedded_postscript: FtUInt,

    /* these should only be used for the top-level font dictionary */
    pub cid_registry: FtUInt,
    pub cid_ordering: FtUInt,
    pub cid_supplement: FtLong,

    pub cid_font_version: FtLong,
    pub cid_font_revision: FtLong,
    pub cid_font_type: FtLong,
    pub cid_count: FtULong,
    pub cid_uid_base: FtULong,
    pub cid_fd_array_offset: FtULong,
    pub cid_fd_select_offset: FtULong,
    pub cid_font_name: FtUInt,

    /* the next fields come from the data of the deprecated          */
    /* `MultipleMaster' operator; they are needed to parse the (also */
    /* deprecated) `blend' operator in Type 2 charstrings            */
    pub num_designs: FtUShort,
    pub num_axes: FtUShort,

    /* fields for CFF2 */
    pub vstore_offset: FtULong,
    pub maxstack: FtUInt,
}

/// `CFF_PrivateRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct CffPrivateRec {
    pub num_blue_values: FtByte,
    pub num_other_blues: FtByte,
    pub num_family_blues: FtByte,
    pub num_family_other_blues: FtByte,

    pub blue_values: [FtPos; 14],
    pub other_blues: [FtPos; 10],
    pub family_blues: [FtPos; 14],
    pub family_other_blues: [FtPos; 10],

    pub blue_scale: FtFixed,
    pub blue_shift: FtPos,
    pub blue_fuzz: FtPos,
    pub standard_width: FtPos,
    pub standard_height: FtPos,

    pub num_snap_widths: FtByte,
    pub num_snap_heights: FtByte,
    pub snap_widths: [FtPos; 13],
    pub snap_heights: [FtPos; 13],
    pub force_bold: FtByte,
    pub force_bold_threshold: FtFixed,
    pub lenIV: FtInt,
    pub language_group: FtInt,
    pub expansion_factor: FtFixed,
    pub initial_random_seed: FtLong,
    pub local_subrs_offset: FtULong,
    pub default_width: FtPos,
    pub nominal_width: FtPos,

    /* fields for CFF2 */
    pub vsindex: FtUInt,
    /// `subfont` (set: the back pointer to the subfont holding this
    /// dictionary)
    pub subfont: bool,
}

/// `CFF_FDSelectRec`
#[derive(Debug, Clone, Default)]
pub struct CffFdSelectRec {
    pub format: FtByte,
    pub range_count: FtUInt,

    /* that's the table, taken from the file `as is' */
    pub data: Option<Vec<u8>>,
    pub data_size: FtUInt,

    /* small cache for format 3 only */
    pub cache_first: FtUInt,
    pub cache_count: FtUInt,
    pub cache_fd: FtByte,
}

/// `CFF_SubFontRec`: a SubFont packs a font dict and a private dict
/// together.  They are needed to support CID-keyed CFF fonts.
#[derive(Debug, Clone, Default)]
pub struct CffSubFontRec {
    pub font_dict: CffFontRecDictRec,
    pub private_dict: CffPrivateRec,

    /* fields for CFF2 */
    pub blend: CffBlendRec,        /* current blend vector       */
    pub lenNDV: FtUInt,            /* current length NDV or zero */
    pub NDV: Option<Vec<FtFixed>>, /* ptr to current NDV or NULL */

    /* `blend_stack' is a writable buffer to hold blend results.          */
    /* This buffer is to the side of the normal cff parser stack;         */
    /* `cff_parse_blend' and `cff_blend_doBlend' push blend results here. */
    /* The normal stack then points to these values instead of the DICT   */
    /* because all other operators in Private DICT clear the stack.       */
    /* `blend_stack' could be cleared at each operator other than blend.  */
    /* Blended values are stored as 5-byte fixed-point values.            */
    /// `blend_stack` (base of stack allocation; its length is
    /// `blend_alloc`)
    pub blend_stack: Vec<u8>,
    /// `blend_top` (first empty slot), as an offset into `blend_stack`
    pub blend_top: usize,
    pub blend_used: FtUInt,  /* number of bytes in use       */
    pub blend_alloc: FtUInt, /* number of bytes allocated    */

    pub local_subrs_index: CffIndexRec,
    /// `local_subrs` (array of pointers into Local Subrs INDEX data), as
    /// offsets into `local_subrs_index.bytes`
    pub local_subrs: Option<Arc<[usize]>>,

    pub random: FtUInt32,
}

pub const CFF_MAX_CID_FONTS: usize = 256;

/// Which subfont of a `CFF_FontRec` (C's `CFF_SubFont` pointer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CffSubFontId {
    /// `&font->top_font`
    Top,
    /// `font->subfonts[i]`
    Sub(usize),
}

/// `PS_FontInfoRec` (the `font_info` cache of `FT_Get_PS_Font_Info`)
#[derive(Debug, Clone, Default)]
pub struct PsFontInfoRec {
    pub version: Option<Vec<u8>>,
    pub notice: Option<Vec<u8>>,
    pub full_name: Option<Vec<u8>>,
    pub family_name: Option<Vec<u8>>,
    pub weight: Option<Vec<u8>>,
    pub italic_angle: FtLong,
    pub is_fixed_pitch: FtByte,
    pub underline_position: FtShort,
    pub underline_thickness: FtUShort,
}

/// `PS_FontExtraRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct PsFontExtraRec {
    pub fs_type: FtUShort,
}

/// `CFF_FontRec`
#[derive(Debug, Default)]
pub struct CffFontRec {
    /* (`library', `stream' and `memory' are the face's) */
    pub base_offset: FtULong, /* offset to start of CFF */
    pub num_faces: FtUInt,
    pub num_glyphs: FtUInt,

    pub version_major: FtByte,
    pub version_minor: FtByte,
    pub header_size: FtByte,

    pub top_dict_length: FtUInt, /* cff2 only */

    pub cff2: bool,

    pub name_index: CffIndexRec,
    pub top_dict_index: CffIndexRec,
    pub global_subrs_index: CffIndexRec,

    pub encoding: CffEncodingRec,
    pub charset: CffCharsetRec,

    pub charstrings_index: CffIndexRec,
    pub font_dict_index: CffIndexRec,
    pub private_index: CffIndexRec,
    pub local_subrs_index: CffIndexRec,

    pub font_name: Option<Vec<u8>>,

    /// `global_subrs` (array of pointers into Global Subrs INDEX data), as
    /// offsets into `global_subrs_index.bytes`
    pub global_subrs: Option<Arc<[usize]>>,

    /* array of pointers into String INDEX data stored at string_pool */
    pub num_strings: FtUInt,
    /// `strings`, as offsets into `string_pool`
    pub strings: Option<Vec<usize>>,
    pub string_pool: Vec<u8>,
    pub string_pool_size: FtULong,

    pub top_font: CffSubFontRec,
    pub num_subfonts: FtUInt,
    /// `subfonts` (allocated as a single block in C)
    pub subfonts: Vec<CffSubFontRec>,

    pub fd_select: CffFdSelectRec,

    /// `pshinter` (interface to PostScript hinter; set when found)
    pub pshinter: bool,

    /// `psnames` (interface to Postscript Names service; set when found)
    pub psnames: bool,

    /// `cffload` (interface to CFFLoad service; set when found)
    pub cffload: bool,

    /* since version 2.3.0 */
    pub font_info: Option<Box<PsFontInfoRec>>, /* font info dictionary */

    /* since version 2.3.6 */
    pub registry: Option<Vec<u8>>,
    pub ordering: Option<Vec<u8>>,

    /* since version 2.4.12 */
    pub cf2_instance: Option<Box<super::psaux::psfont::Cf2FontRec>>,

    /* since version 2.7.1 */
    pub vstore: CffVStoreRec, /* parsed vstore structure */

    /* since version 2.9 */
    pub font_extra: Option<Box<PsFontExtraRec>>,
}

impl CffFontRec {
    /// The subfont `id` names.
    pub fn subfont(&self, id: CffSubFontId) -> &CffSubFontRec {
        match id {
            CffSubFontId::Top => &self.top_font,
            CffSubFontId::Sub(i) => &self.subfonts[i],
        }
    }

    /// The subfont `id` names, mutably.
    pub fn subfont_mut(&mut self, id: CffSubFontId) -> &mut CffSubFontRec {
        match id {
            CffSubFontId::Top => &mut self.top_font,
            CffSubFontId::Sub(i) => &mut self.subfonts[i],
        }
    }
}

/* cffotypes.h */

/// The CFF driver's additions to its size object (`CFF_SizeRec`, minus
/// its `FT_SizeRec` root, which is the face's), with the size's
/// `internal->module_data` (`CFF_Internal`).
#[derive(Debug, Default)]
pub struct CffSizeRec {
    pub strike_index: FtULong, /* 0xFFFFFFFF to indicate invalid */

    /// `internal->module_data`
    pub module_data: Option<Box<CffInternalRec>>,
}

/// The CFF driver's additions to its glyph slot (`CFF_GlyphSlotRec`,
/// minus its `FT_GlyphSlotRec` root, which is the face's).
#[derive(Debug, Clone, Copy, Default)]
pub struct CffGlyphSlotRec {
    pub hint: bool,
    pub scaled: bool,

    pub x_scale: FtFixed,
    pub y_scale: FtFixed,
}

/// `CFF_InternalRec`: the interface to the 'internal' field of `FT_Size`.
#[derive(Debug, Default)]
pub struct CffInternalRec {
    pub topfont: Option<Box<PshGlobalsRec>>,
    /// `subfonts[CFF_MAX_CID_FONTS]` (the font's `num_subfonts` first)
    pub subfonts: Vec<Option<Box<PshGlobalsRec>>>,
}

/// `CFF_Transform`: subglyph transformation record.
#[derive(Debug, Clone, Copy, Default)]
pub struct CffTransform {
    pub xx: FtFixed,
    pub xy: FtFixed, /* transformation matrix coefficients */
    pub yx: FtFixed,
    pub yy: FtFixed,
    pub ox: FtF26Dot6,
    pub oy: FtF26Dot6, /* offsets                            */
}
