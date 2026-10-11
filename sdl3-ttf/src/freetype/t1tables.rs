// Rust translation of include/freetype/t1tables.h from FreeType (2.13.2,
// as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Basic Type 1/Type 2 tables definitions and interface (specification
//! only).
//!
//! The strings (`FT_String*`) are their bytes, without the terminating
//! null byte. The blend record's per-design blocks (`design_pos`, the
//! design maps' points, the weight vectors, and the font infos, privates
//! and bounding boxes of the designs) are owned vectors; the `[0]`
//! entries of C's `font_infos`, `privates` and `bboxes`, which point to
//! the face's own records, are those records.

#![allow(non_snake_case, non_upper_case_globals)]

use super::fttypes::*;

/* Note that we separate font data in PS_FontInfoRec and PS_PrivateRec */
/* structures in order to support Multiple Master fonts.               */

/// `PS_FontInfoRec`: a structure used to model a Type 1 or Type 2
/// FontInfo dictionary.  Note that for Multiple Master fonts, each
/// instance has its own FontInfo dictionary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PsFontInfoRec {
    pub version: Option<Vec<u8>>,
    pub notice: Option<Vec<u8>>,
    pub full_name: Option<Vec<u8>>,
    pub family_name: Option<Vec<u8>>,
    pub weight: Option<Vec<u8>>,
    pub italic_angle: FtLong,
    pub is_fixed_pitch: FtByte, /* (an `FT_Bool') */
    pub underline_position: FtShort,
    pub underline_thickness: FtUShort,
}

/// `PS_PrivateRec`: a structure used to model a Type 1 or Type 2 private
/// dictionary.  Note that for Multiple Master fonts, each instance has
/// its own Private dictionary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PsPrivateRec {
    pub unique_id: FtInt,
    pub lenIV: FtInt,

    pub num_blue_values: FtByte,
    pub num_other_blues: FtByte,
    pub num_family_blues: FtByte,
    pub num_family_other_blues: FtByte,

    pub blue_values: [FtShort; 14],
    pub other_blues: [FtShort; 10],

    pub family_blues: [FtShort; 14],
    pub family_other_blues: [FtShort; 10],

    pub blue_scale: FtFixed,
    pub blue_shift: FtInt,
    pub blue_fuzz: FtInt,

    pub standard_width: [FtUShort; 1],
    pub standard_height: [FtUShort; 1],

    pub num_snap_widths: FtByte,
    pub num_snap_heights: FtByte,
    pub force_bold: FtByte,    /* (an `FT_Bool') */
    pub round_stem_up: FtByte, /* (an `FT_Bool') */

    pub snap_widths: [FtShort; 13],  /* including std width  */
    pub snap_heights: [FtShort; 13], /* including std height */

    pub expansion_factor: FtFixed,

    pub language_group: FtLong,
    pub password: FtLong,

    pub min_feature: [FtShort; 2],
}

/// `T1_Blend_Flags`: a set of flags used to indicate which fields are
/// present in a given blend dictionary (font info or private).  Used to
/// support Multiple Masters fonts.
pub type T1BlendFlags = FtUInt;

/* required fields in a FontInfo blend dictionary */
pub const T1_BLEND_UNDERLINE_POSITION: T1BlendFlags = 0;
pub const T1_BLEND_UNDERLINE_THICKNESS: T1BlendFlags = 1;
pub const T1_BLEND_ITALIC_ANGLE: T1BlendFlags = 2;

/* required fields in a Private blend dictionary */
pub const T1_BLEND_BLUE_VALUES: T1BlendFlags = 3;
pub const T1_BLEND_OTHER_BLUES: T1BlendFlags = 4;
pub const T1_BLEND_STANDARD_WIDTH: T1BlendFlags = 5;
pub const T1_BLEND_STANDARD_HEIGHT: T1BlendFlags = 6;
pub const T1_BLEND_STEM_SNAP_WIDTHS: T1BlendFlags = 7;
pub const T1_BLEND_STEM_SNAP_HEIGHTS: T1BlendFlags = 8;
pub const T1_BLEND_BLUE_SCALE: T1BlendFlags = 9;
pub const T1_BLEND_BLUE_SHIFT: T1BlendFlags = 10;
pub const T1_BLEND_FAMILY_BLUES: T1BlendFlags = 11;
pub const T1_BLEND_FAMILY_OTHER_BLUES: T1BlendFlags = 12;
pub const T1_BLEND_FORCE_BOLD: T1BlendFlags = 13;

pub const T1_BLEND_MAX: T1BlendFlags = 14; /* do not remove */

/* maximum number of Multiple Masters designs, as defined in the spec */
pub const T1_MAX_MM_DESIGNS: usize = 16;

/* maximum number of Multiple Masters axes, as defined in the spec */
pub const T1_MAX_MM_AXIS: usize = 4;

/* maximum number of elements in a design map */
pub const T1_MAX_MM_MAP_POINTS: usize = 20;

/// `PS_DesignMapRec`: this structure is used to store the BlendDesignMap
/// entry for an axis (its design and blend points; C's `blend_points`
/// follow `design_points` in one block)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PsDesignMapRec {
    pub num_points: FtByte,
    pub design_points: Vec<FtLong>,
    pub blend_points: Vec<FtFixed>,
}

/// `PS_BlendRec`
///
/// `design_pos[n]` is `design_pos[n * num_axis ..]`; `font_infos`,
/// `privates` and `bboxes` are the records of the designs (C's entries
/// 1 to `num_designs`; entry 0 is the face's own record).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PsBlendRec {
    pub num_designs: FtUInt,
    pub num_axis: FtUInt,

    pub axis_names: [Option<Vec<u8>>; T1_MAX_MM_AXIS],
    /// `design_pos` (one block of `num_designs * num_axis` positions,
    /// `None` until allocated)
    pub design_pos: Option<Vec<FtFixed>>,
    pub design_map: [PsDesignMapRec; T1_MAX_MM_AXIS],

    /// `weight_vector` (`None` until allocated, with
    /// `default_weight_vector`)
    pub weight_vector: Option<Vec<FtFixed>>,
    pub default_weight_vector: Vec<FtFixed>,

    pub font_infos: Vec<PsFontInfoRec>,
    pub privates: Vec<PsPrivateRec>,

    pub blend_bitflags: FtULong,

    pub bboxes: Vec<FtBBox>,

    /* since 2.3.0 */

    /* undocumented, optional: the default design instance;   */
    /* corresponds to default_weight_vector --                */
    /* num_default_design_vector == 0 means it is not present */
    /* in the font and associated metrics files               */
    pub default_design_vector: [FtUInt; T1_MAX_MM_DESIGNS],
    pub num_default_design_vector: FtUInt,
}

/// `CID_FaceDictRec`: a structure used to represent data in a CID
/// top-level dictionary.  In most cases, they are part of the font's
/// `/FDArray` array.  Within a CID font file, such (internal)
/// subfont dictionaries are enclosed by `%ADOBeginFontDict` and
/// `%ADOEndFontDict` comments.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CidFaceDictRec {
    pub private_dict: PsPrivateRec,

    pub len_buildchar: FtUInt,
    pub forcebold_threshold: FtFixed,
    pub stroke_width: FtPos,
    pub expansion_factor: FtFixed, /* this is a duplicate of           */
    /* `private_dict->expansion_factor' */
    pub paint_type: FtByte,
    pub font_type: FtByte,
    pub font_matrix: FtMatrix,
    pub font_offset: FtVector,

    pub num_subrs: FtUInt,
    pub subrmap_offset: FtULong,
    pub sd_bytes: FtUInt,
}

/// `CID_FaceInfoRec`: a structure used to represent CID Face information.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CidFaceInfoRec {
    pub cid_font_name: Option<Vec<u8>>,
    pub cid_version: FtFixed,
    pub cid_font_type: FtInt,

    pub registry: Option<Vec<u8>>,
    pub ordering: Option<Vec<u8>>,
    pub supplement: FtInt,

    pub font_info: PsFontInfoRec,
    pub font_bbox: FtBBox,
    pub uid_base: FtULong,

    pub num_xuid: FtInt,
    pub xuid: [FtULong; 16],

    pub cidmap_offset: FtULong,
    pub fd_bytes: FtUInt,
    pub gd_bytes: FtUInt,
    pub cid_count: FtULong,

    pub num_dicts: FtUInt,
    pub font_dicts: Vec<CidFaceDictRec>,

    pub data_offset: FtULong,
}

/// `T1_EncodingType`: an enumeration describing the 'Encoding' entry in a
/// Type 1 dictionary.
pub type T1EncodingType = FtUInt;

pub const T1_ENCODING_TYPE_NONE: T1EncodingType = 0;
pub const T1_ENCODING_TYPE_ARRAY: T1EncodingType = 1;
pub const T1_ENCODING_TYPE_STANDARD: T1EncodingType = 2;
pub const T1_ENCODING_TYPE_ISOLATIN1: T1EncodingType = 3;
pub const T1_ENCODING_TYPE_EXPERT: T1EncodingType = 4;

/// `PS_Dict_Keys`: an enumeration used in calls to `FT_Get_PS_Font_Value`
/// to identify the Type 1 dictionary entry to retrieve.
pub type PsDictKeys = FtUInt;

/* conventionally in the font dictionary */
pub const PS_DICT_FONT_TYPE: PsDictKeys = 0; /* FT_Byte         */
pub const PS_DICT_FONT_MATRIX: PsDictKeys = 1; /* FT_Fixed        */
pub const PS_DICT_FONT_BBOX: PsDictKeys = 2; /* FT_Fixed        */
pub const PS_DICT_PAINT_TYPE: PsDictKeys = 3; /* FT_Byte         */
pub const PS_DICT_FONT_NAME: PsDictKeys = 4; /* FT_String*      */
pub const PS_DICT_UNIQUE_ID: PsDictKeys = 5; /* FT_Int          */
pub const PS_DICT_NUM_CHAR_STRINGS: PsDictKeys = 6; /* FT_Int          */
pub const PS_DICT_CHAR_STRING_KEY: PsDictKeys = 7; /* FT_String*      */
pub const PS_DICT_CHAR_STRING: PsDictKeys = 8; /* FT_String*      */
pub const PS_DICT_ENCODING_TYPE: PsDictKeys = 9; /* T1_EncodingType */
pub const PS_DICT_ENCODING_ENTRY: PsDictKeys = 10; /* FT_String*      */

/* conventionally in the font Private dictionary */
pub const PS_DICT_NUM_SUBRS: PsDictKeys = 11; /* FT_Int     */
pub const PS_DICT_SUBR: PsDictKeys = 12; /* FT_String* */
pub const PS_DICT_STD_HW: PsDictKeys = 13; /* FT_UShort  */
pub const PS_DICT_STD_VW: PsDictKeys = 14; /* FT_UShort  */
pub const PS_DICT_NUM_BLUE_VALUES: PsDictKeys = 15; /* FT_Byte    */
pub const PS_DICT_BLUE_VALUE: PsDictKeys = 16; /* FT_Short   */
pub const PS_DICT_BLUE_FUZZ: PsDictKeys = 17; /* FT_Int     */
pub const PS_DICT_NUM_OTHER_BLUES: PsDictKeys = 18; /* FT_Byte    */
pub const PS_DICT_OTHER_BLUE: PsDictKeys = 19; /* FT_Short   */
pub const PS_DICT_NUM_FAMILY_BLUES: PsDictKeys = 20; /* FT_Byte    */
pub const PS_DICT_FAMILY_BLUE: PsDictKeys = 21; /* FT_Short   */
pub const PS_DICT_NUM_FAMILY_OTHER_BLUES: PsDictKeys = 22; /* FT_Byte    */
pub const PS_DICT_FAMILY_OTHER_BLUE: PsDictKeys = 23; /* FT_Short   */
pub const PS_DICT_BLUE_SCALE: PsDictKeys = 24; /* FT_Fixed   */
pub const PS_DICT_BLUE_SHIFT: PsDictKeys = 25; /* FT_Int     */
pub const PS_DICT_NUM_STEM_SNAP_H: PsDictKeys = 26; /* FT_Byte    */
pub const PS_DICT_STEM_SNAP_H: PsDictKeys = 27; /* FT_Short   */
pub const PS_DICT_NUM_STEM_SNAP_V: PsDictKeys = 28; /* FT_Byte    */
pub const PS_DICT_STEM_SNAP_V: PsDictKeys = 29; /* FT_Short   */
pub const PS_DICT_FORCE_BOLD: PsDictKeys = 30; /* FT_Bool    */
pub const PS_DICT_RND_STEM_UP: PsDictKeys = 31; /* FT_Bool    */
pub const PS_DICT_MIN_FEATURE: PsDictKeys = 32; /* FT_Short   */
pub const PS_DICT_LEN_IV: PsDictKeys = 33; /* FT_Int     */
pub const PS_DICT_PASSWORD: PsDictKeys = 34; /* FT_Long    */
pub const PS_DICT_LANGUAGE_GROUP: PsDictKeys = 35; /* FT_Long    */

/* conventionally in the font FontInfo dictionary */
pub const PS_DICT_VERSION: PsDictKeys = 36; /* FT_String* */
pub const PS_DICT_NOTICE: PsDictKeys = 37; /* FT_String* */
pub const PS_DICT_FULL_NAME: PsDictKeys = 38; /* FT_String* */
pub const PS_DICT_FAMILY_NAME: PsDictKeys = 39; /* FT_String* */
pub const PS_DICT_WEIGHT: PsDictKeys = 40; /* FT_String* */
pub const PS_DICT_IS_FIXED_PITCH: PsDictKeys = 41; /* FT_Bool    */
pub const PS_DICT_UNDERLINE_POSITION: PsDictKeys = 42; /* FT_Short   */
pub const PS_DICT_UNDERLINE_THICKNESS: PsDictKeys = 43; /* FT_UShort  */
pub const PS_DICT_FS_TYPE: PsDictKeys = 44; /* FT_UShort  */
pub const PS_DICT_ITALIC_ANGLE: PsDictKeys = 45; /* FT_Long    */

pub const PS_DICT_MAX: PsDictKeys = PS_DICT_ITALIC_ANGLE;
