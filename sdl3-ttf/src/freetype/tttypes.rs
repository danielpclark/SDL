// Rust translation of include/freetype/internal/tttypes.h and the color
// types of include/freetype/ftcolor.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Basic SFNT/TrueType type definitions and interface (specification
//! only).
//!
//! `TT_FaceRec` keeps its fields; the pointers into frames extracted from
//! the stream become owned byte vectors, and the hook functions
//! (`goto_table`, `access_glyph_frame`, ...) and service pointers
//! (`sfnt`, `psnames`, ...) are direct calls, as there is one
//! implementation of each. The TrueType driver's size object
//! (`TT_SizeRec`, minus its `FT_SizeRec` root, which is the face's) is
//! kept in the face, as faces have a single size here.

#![allow(non_snake_case)]

use std::sync::Arc;

use super::base::ftobjs::FtFaceRec;
use super::fttypes::*;
use super::tttables::*;

/// `FT_Color`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtColor {
    pub blue: FtByte,
    pub green: FtByte,
    pub red: FtByte,
    pub alpha: FtByte,
}

pub const FT_PALETTE_FOR_LIGHT_BACKGROUND: u16 = 0x01;
pub const FT_PALETTE_FOR_DARK_BACKGROUND: u16 = 0x02;

/// `FT_Palette_Data`
#[derive(Debug, Clone, Default)]
pub struct FtPaletteData {
    pub num_palettes: FtUShort,
    pub palette_name_ids: Option<Vec<FtUShort>>,
    pub palette_flags: Option<Vec<FtUShort>>,

    pub num_palette_entries: FtUShort,
    pub palette_entry_name_ids: Option<Vec<FtUShort>>,
}

/// `TTC_HeaderRec`
#[derive(Debug, Clone, Default)]
pub struct TtcHeaderRec {
    pub tag: FtULong,
    pub version: FtFixed,
    pub count: FtLong,
    pub offsets: Vec<FtULong>,
}

/// `SFNT_HeaderRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct SfntHeaderRec {
    pub format_tag: FtULong,
    pub num_tables: FtUShort,
    pub search_range: FtUShort,
    pub entry_selector: FtUShort,
    pub range_shift: FtUShort,

    pub offset: FtULong, /* not in file */
}

/// `TT_TableRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct TtTableRec {
    pub Tag: FtULong,      /*        table type */
    pub CheckSum: FtULong, /*    table checksum */
    pub Offset: FtULong,   /* table file offset */
    pub Length: FtULong,   /*      table length */
}

/// `TT_LongMetricsRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct TtLongMetricsRec {
    pub advance: FtUShort,
    pub bearing: FtShort,
}

/// `TT_NameRec`
#[derive(Debug, Clone, Default)]
pub struct TtNameRec {
    pub platformID: FtUShort,
    pub encodingID: FtUShort,
    pub languageID: FtUShort,
    pub nameID: FtUShort,
    pub stringLength: FtUShort,
    pub stringOffset: FtULong,

    /* this last field is not defined in the spec */
    /* but used by the FreeType engine            */
    pub string: Option<Vec<u8>>,
}

/// `TT_LangTagRec`
#[derive(Debug, Clone, Default)]
pub struct TtLangTagRec {
    pub stringLength: FtUShort,
    pub stringOffset: FtULong,

    /* this last field is not defined in the spec */
    /* but used by the FreeType engine            */
    pub string: Option<Vec<u8>>,
}

/// `TT_NameTableRec` (its `stream` is the face's)
#[derive(Debug, Clone, Default)]
pub struct TtNameTableRec {
    pub format: FtUShort,
    pub numNameRecords: FtUInt,
    pub storageOffset: FtUInt,
    pub names: Vec<TtNameRec>,
    pub numLangTagRecords: FtUInt,
    pub langTags: Vec<TtLangTagRec>,
}

/// `TT_GaspRangeRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct TtGaspRangeRec {
    pub maxPPEM: FtUShort,
    pub gaspFlag: FtUShort,
}

pub const TT_GASP_GRIDFIT: u16 = 0x01;
pub const TT_GASP_DOGRAY: u16 = 0x02;

/// `TT_GaspRec`
#[derive(Debug, Clone, Default)]
pub struct TtGaspRec {
    pub version: FtUShort,
    pub numRanges: FtUShort,
    pub gaspRanges: Vec<TtGaspRangeRec>,
}

/// `TT_SBit_MetricsRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct TtSBitMetricsRec {
    pub height: FtUShort,
    pub width: FtUShort,

    pub horiBearingX: FtShort,
    pub horiBearingY: FtShort,
    pub horiAdvance: FtUShort,

    pub vertBearingX: FtShort,
    pub vertBearingY: FtShort,
    pub vertAdvance: FtUShort,
}

/// `TT_SBit_SmallMetricsRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct TtSBitSmallMetricsRec {
    pub height: FtByte,
    pub width: FtByte,

    pub bearingX: FtChar,
    pub bearingY: FtChar,
    pub advance: FtByte,
}

/// `TT_SBit_LineMetricsRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct TtSBitLineMetricsRec {
    pub ascender: FtChar,
    pub descender: FtChar,
    pub max_width: FtByte,
    pub caret_slope_numerator: FtChar,
    pub caret_slope_denominator: FtChar,
    pub caret_offset: FtChar,
    pub min_origin_SB: FtChar,
    pub min_advance_SB: FtChar,
    pub max_before_BL: FtChar,
    pub min_after_BL: FtChar,
    pub pads: [FtChar; 2],
}

/// `TT_Post_NamesRec`
#[derive(Debug, Clone, Default)]
pub struct TtPostNamesRec {
    pub loaded: bool,
    pub num_glyphs: FtUShort,

    pub num_names: FtUShort,
    pub glyph_indices: Vec<FtUShort>,
    pub glyph_names: Vec<Vec<u8>>,
}

/// `TT_BDFRec`: offsets into `table`
#[derive(Debug, Clone, Default)]
pub struct TtBdfRec {
    pub table: Vec<u8>,
    pub strings: usize,
    pub strings_size: FtULong,
    pub num_strikes: FtUInt,
    pub loaded: bool,
}

/// `TT_SbitTableType`
pub type TtSbitTableType = u32;
pub const TT_SBIT_TABLE_TYPE_NONE: TtSbitTableType = 0;
pub const TT_SBIT_TABLE_TYPE_EBLC: TtSbitTableType = 1; /* `EBLC' (Microsoft), */
/* `bloc' (Apple)      */
pub const TT_SBIT_TABLE_TYPE_CBLC: TtSbitTableType = 2; /* `CBLC' (Google)     */
pub const TT_SBIT_TABLE_TYPE_SBIX: TtSbitTableType = 3; /* `sbix' (Apple)      */
/* do not remove */
pub const TT_SBIT_TABLE_TYPE_MAX: TtSbitTableType = 4;

/* OpenType 1.8 brings new tables for variation font support; */
/* to make the old MM and GX fonts still work we need to check */
/* the presence (and validity) of the functionality provided   */
/* by those tables.  The following flag macros are for the     */
/* field `variation_support'.                                  */
/*                                                             */
/* Note that `fvar' gets checked immediately at font loading,  */
/* while the other features are only loaded if MM support is   */
/* actually requested.                                         */

/* FVAR */
pub const TT_FACE_FLAG_VAR_FVAR: u32 = 1 << 0;

/* HVAR */
pub const TT_FACE_FLAG_VAR_HADVANCE: u32 = 1 << 1;
pub const TT_FACE_FLAG_VAR_LSB: u32 = 1 << 2;
pub const TT_FACE_FLAG_VAR_RSB: u32 = 1 << 3;

/* VVAR */
pub const TT_FACE_FLAG_VAR_VADVANCE: u32 = 1 << 4;
pub const TT_FACE_FLAG_VAR_TSB: u32 = 1 << 5;
pub const TT_FACE_FLAG_VAR_BSB: u32 = 1 << 6;
pub const TT_FACE_FLAG_VAR_VORG: u32 = 1 << 7;

/* MVAR */
pub const TT_FACE_FLAG_VAR_MVAR: u32 = 1 << 8;

/// `TT_FaceRec`
#[derive(Debug, Default)]
pub struct TtFaceRec {
    pub root: FtFaceRec,

    pub ttc_header: TtcHeaderRec,

    pub format_tag: FtULong,
    pub num_tables: FtUShort,
    pub dir_tables: Vec<TtTableRec>,

    pub header: TtHeader,         /* TrueType header table          */
    pub horizontal: TtHoriHeader, /* TrueType horizontal header     */

    pub max_profile: TtMaxProfile,

    pub vertical_info: bool,
    pub vertical: TtVertHeader, /* TT Vertical header, if present */

    pub num_names: FtUShort,        /* number of name records  */
    pub name_table: TtNameTableRec, /* name table              */

    pub os2: TtOs2,               /* TrueType OS/2 table            */
    pub postscript: TtPostscript, /* TrueType Postscript table      */

    pub cmap_table: Option<Arc<[u8]>>, /* extracted `cmap' table */
    pub cmap_size: FtULong,

    pub gasp: TtGaspRec, /* the `gasp' table */

    /* the `PCLT' table */
    pub pclt: TtPclt,

    /* postscript names table */
    pub postscript_names: TtPostNamesRec,

    /* since version 2.10 */
    pub palette_data: FtPaletteData,
    pub palette_index: FtUShort,
    pub palette: Vec<FtColor>,
    pub have_foreground_color: bool,
    pub foreground_color: FtColor,

    /***********************************************************************
     *
     * TrueType-specific fields (ignored by the CFF driver)
     *
     */

    /* the font program, if any */
    pub font_program_size: FtULong,
    pub font_program: Arc<[u8]>,

    /* the cvt program, if any */
    pub cvt_program_size: FtULong,
    pub cvt_program: Arc<[u8]>,

    /* the original, unscaled, control value table */
    pub cvt_size: FtULong,
    pub cvt: Vec<FtInt32>,

    /* A pointer to the bytecode interpreter to use.  This is also */
    /* used to hook the debugger for the `ttdebug' utility.        */
    /* (always TT_RunIns here) */

    /* a typeless pointer to the FT_Service_PsCMapsRec table used to */
    /* handle glyph names <-> unicode & Mac values                   */
    /* (the psnames module, called directly) */
    pub postscript_name: Option<String>,

    /* since version 2.2 */
    pub glyf_len: FtULong,
    pub glyf_offset: FtULong, /* since 2.7.1 */

    pub is_cff2: bool, /* since 2.7.1 */

    pub doblend: bool,
    pub blend: Option<Box<super::truetype::ttgxvar::GxBlendRec>>,

    pub variation_support: FtUInt32, /* since 2.7.1 */

    pub var_postscript_prefix: Option<String>, /* since 2.7.2 */
    pub var_postscript_prefix_len: FtUInt,

    pub var_default_named_instance: FtUInt, /* since 2.13.1 */

    pub non_var_style_name: Option<String>, /* since 2.13.1 */

    /* since version 2.2 */
    pub horz_metrics_size: FtULong,
    pub vert_metrics_size: FtULong,

    pub num_locations: FtULong, /* up to 0xFFFF + 1 */
    pub glyph_locations: Vec<u8>,

    pub hdmx_table: Vec<u8>,
    pub hdmx_table_size: FtULong,
    pub hdmx_record_count: FtUInt,
    pub hdmx_record_size: FtULong,
    /// offsets of the records in `hdmx_table`
    pub hdmx_records: Vec<usize>,

    pub sbit_table: Vec<u8>,
    pub sbit_table_size: FtULong,
    pub sbit_table_type: TtSbitTableType,
    pub sbit_num_strikes: FtUInt,
    pub sbit_strike_map: Vec<FtUInt>,

    pub kern_table: Vec<u8>,
    pub kern_table_size: FtULong,
    pub num_kern_tables: FtUInt,
    pub kern_avail_bits: FtUInt32,
    pub kern_order_bits: FtUInt32,

    pub bdf: TtBdfRec,

    /* since 2.3.0 */
    pub horz_metrics_offset: FtULong,
    pub vert_metrics_offset: FtULong,

    /* since 2.7 */
    pub ebdt_start: FtULong, /* either `CBDT', `EBDT', or `bdat' */
    pub ebdt_size: FtULong,

    /* since 2.10 */
    pub cpal: Option<Box<super::sfnt::ttcpal::Cpal>>,
    pub colr: Option<Box<super::sfnt::ttcolr::Colr>>,

    /* since 2.12 */
    pub svg: Option<Box<super::sfnt::ttsvg::Svg>>,

    /* the TrueType driver's size data (see the module documentation) */
    pub size: super::truetype::ttobjs::TtSizeRec,
}

/// `TT_GlyphZoneRec`: the arrays are owned here (C points them into the
/// glyph loader or the size's twilight allocation; see ttgload).
#[derive(Debug, Clone, Default)]
pub struct TtGlyphZoneRec {
    pub max_points: FtUShort,
    pub max_contours: FtShort,
    pub n_points: FtUShort,  /* number of points in zone    */
    pub n_contours: FtShort, /* number of contours          */

    pub org: Vec<FtVector>,  /* original point coordinates  */
    pub cur: Vec<FtVector>,  /* current point coordinates   */
    pub orus: Vec<FtVector>, /* original (unscaled) point coordinates */

    pub tags: Vec<FtByte>,       /* current touch flags         */
    pub contours: Vec<FtUShort>, /* contour end points          */

    pub first_point: FtUShort, /* offset of first (#0) point  */
}
