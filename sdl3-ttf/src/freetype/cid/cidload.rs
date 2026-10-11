// Rust translation of src/cid/cidload.c, src/cid/cidload.h and
// src/cid/cidtoken.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! CID-keyed Type1 font loader (body).
//!
//! The keyword table (`cid_field_records`, with `cidtoken.h`) is
//! [`CID_FIELD_RECORDS`]. The subroutines of a font dictionary are one
//! block with their offsets (C's `code` pointers into it).

use std::sync::Arc;

use super::super::base::ftcalc::{ft_div_fix, ft_matrix_check};
use super::super::base::ftmemory::{ft_new_array, ft_qalloc};
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::psaux::psconv::at;
use super::super::psaux::psobjs::*;
use super::super::t1tables::*;
use super::super::t1types::*;
use super::cidparse::*;
use crate::{t1_callback_field, t1_field, t1_table_field, t1_table_field2};

/// `CID_Loader`
#[derive(Debug, Default)]
pub struct CidLoader {
    pub parser: CidParser, /* parser used to read the stream */
    pub num_chars: FtInt,  /* number of characters in encoding */
}

/// `cid_get_offset`: read a single offset (`offsize` bytes at `*start`
/// in `b`)
pub fn cid_get_offset(b: &[u8], start: &mut usize, offsize: FtUInt) -> FtULong {
    let mut result: FtULong = 0;
    let mut p = *start;

    for _ in 0..offsize {
        result <<= 8;
        result |= at(b, p) as FtULong;
        p += 1;
    }

    *start = p;
    result
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                    TYPE 1 SYMBOL PARSING                      *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// The type of the keyword callbacks (`T1_Field_ParseFunc`).
pub type CidFieldReader = fn(face: &mut CidFaceRec, parser: &mut CidParser);

/// A keyword of the CID loader.
pub type CidField = T1FieldRec<CidFieldReader>;

/// The parser's error as a result.
fn parser_result(error: FtError) -> FtResult<()> {
    if error == 0 {
        Ok(())
    } else {
        Err(error)
    }
}

/// `cid_load_keyword`
fn cid_load_keyword(
    face: &mut CidFaceRec,
    loader: &mut CidLoader,
    keyword: &CidField,
) -> FtResult<()> {
    let parser = &mut loader.parser;

    /* if the keyword has a dedicated callback, call it */
    if keyword.type_ == T1_FIELD_TYPE_CALLBACK {
        if let Some(reader) = keyword.reader {
            reader(face, parser);
        }
        return parser_result(parser.root.error);
    }

    let CidFaceRec {
        cid, font_extra, ..
    } = face;

    /* we must now compute the address of our target object */
    let object = match keyword.location {
        T1_FIELD_LOCATION_CID_INFO => T1Object::CidInfo(cid),

        T1_FIELD_LOCATION_FONT_INFO => T1Object::FontInfo(&mut cid.font_info),

        T1_FIELD_LOCATION_FONT_EXTRA => T1Object::FontExtra(font_extra),

        T1_FIELD_LOCATION_BBOX => T1Object::BBox(&mut cid.font_bbox),

        _ => {
            if parser.num_dict >= cid.num_dicts {
                return Err(FT_ERR_SYNTAX_ERROR);
            }

            let dict = &mut cid.font_dicts[parser.num_dict as usize];
            match keyword.location {
                T1_FIELD_LOCATION_PRIVATE => T1Object::Private(&mut dict.private_dict),

                _ => T1Object::CidDict(dict),
            }
        }
    };

    let mut dummy_object = [object];

    /* now, load the keyword data in the object's field(s) */
    if keyword.type_ == T1_FIELD_TYPE_INTEGER_ARRAY || keyword.type_ == T1_FIELD_TYPE_FIXED_ARRAY {
        cid_parser_load_field_table(parser, keyword, &mut dummy_object)
    } else {
        cid_parser_load_field(parser, keyword, &mut dummy_object)
    }

    /* Exit: */
}

/// `cid_parse_font_matrix`
fn cid_parse_font_matrix(face: &mut CidFaceRec, parser: &mut CidParser) {
    let mut temp: [FtFixed; 6] = [0; 6];

    if parser.num_dict < face.cid.num_dicts {
        let dict = &mut face.cid.font_dicts[parser.num_dict as usize];

        /* input is scaled by 1000 to accommodate default FontMatrix */
        let result = cid_parser_to_fixed_array(parser, 6, Some(&mut temp), 3);

        if result < 6 {
            return;
        }

        let temp_scale: FtFixed = temp[3].abs();

        if temp_scale == 0 {
            return;
        }

        /* atypical case */
        if temp_scale != 0x10000 {
            /* set units per EM based on FontMatrix values */
            face.root.units_per_EM = ft_div_fix(1000, temp_scale) as FtUShort;

            temp[0] = ft_div_fix(temp[0], temp_scale);
            temp[1] = ft_div_fix(temp[1], temp_scale);
            temp[2] = ft_div_fix(temp[2], temp_scale);
            temp[4] = ft_div_fix(temp[4], temp_scale);
            temp[5] = ft_div_fix(temp[5], temp_scale);
            temp[3] = if temp[3] < 0 { -0x10000 } else { 0x10000 };
        }

        let matrix = &mut dict.font_matrix;
        matrix.xx = temp[0];
        matrix.yx = temp[1];
        matrix.xy = temp[2];
        matrix.yy = temp[3];

        if !ft_matrix_check(matrix) {
            parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
            return;
        }

        /* note that the font offsets are expressed in integer font units */
        let offset = &mut dict.font_offset;
        offset.x = temp[4] >> 16;
        offset.y = temp[5] >> 16;
    }

    /* Exit: */
}

/// `parse_fd_array` (the stream's size is the face's)
fn parse_fd_array(face: &mut CidFaceRec, parser: &mut CidParser) {
    let cid = &mut face.cid;
    let stream_size = face.root.stream.as_ref().map_or(0, |s| s.size);

    let mut num_dicts: FtLong = cid_parser_to_int(parser);
    if num_dicts < 0 || num_dicts > FtInt::MAX as FtLong {
        return;
    }

    /*
     * A single entry in the FDArray must (at least) contain the following
     * structure elements.
     *
     *   %ADOBeginFontDict              18
     *   X dict begin                   13
     *     /FontMatrix [X X X X]        22
     *     /Private X dict begin        22
     *     end                           4
     *   end                             4
     *   %ADOEndFontDict                16
     *
     * This needs 18+13+22+22+4+4+16=99 bytes or more.  Normally, you also
     * need a `dup X' at the very beginning and a `put' at the end, so a
     * rough guess using 100 bytes as the minimum is justified.
     */
    let max_dicts: FtLong = (stream_size / 100) as FtLong;
    if num_dicts > max_dicts {
        num_dicts = max_dicts;
    }

    if cid.font_dicts.is_empty() {
        match ft_new_array::<CidFaceDictRec>(num_dicts) {
            Ok(dicts) => cid.font_dicts = dicts,
            Err(_) => return,
        }

        cid.num_dicts = num_dicts as FtUInt;

        /* set some default values (the same as for Type 1 fonts) */
        for dict in cid.font_dicts.iter_mut() {
            dict.private_dict.blue_shift = 7;
            dict.private_dict.blue_fuzz = 1;
            dict.private_dict.lenIV = 4;
            dict.private_dict.expansion_factor = (0.06 * 0x10000 as f64) as FtFixed;
            dict.private_dict.blue_scale = (0.039625 * 0x10000 as f64 * 1000.0) as FtFixed;
        }
    }

    /* Exit: */
}

/* By mistake, `expansion_factor' appears both in PS_PrivateRec */
/* and CID_FaceDictRec (both are public header files and can't  */
/* be thus changed).  We simply copy the value.                 */

/// `parse_expansion_factor`
fn parse_expansion_factor(face: &mut CidFaceRec, parser: &mut CidParser) {
    if parser.num_dict < face.cid.num_dicts {
        let dict = &mut face.cid.font_dicts[parser.num_dict as usize];

        dict.expansion_factor = cid_parser_to_fixed(parser, 0);
        dict.private_dict.expansion_factor = dict.expansion_factor;
    }
}

/* By mistake, `CID_FaceDictRec' doesn't contain a field for the */
/* `FontName' keyword.  FreeType doesn't need it, but it is nice */
/* to catch it for producing better trace output.                */

/// `parse_font_name` (without trace output, it does nothing)
fn parse_font_name(_face: &mut CidFaceRec, _parser: &mut CidParser) {}

/// The bounding box field (`T1_FIELD_BBOX`: the whole record, at the
/// offset of its `xMin`).
macro_rules! cid_bbox_field {
    ($ident:expr) => {
        T1FieldRec {
            ident: $ident,
            location: T1_FIELD_LOCATION_BBOX,
            type_: T1_FIELD_TYPE_BBOX,
            reader: None,
            access: Some({
                fn access<'a, 'b>(o: &'a mut T1Object<'b>) -> T1FieldRef<'a> {
                    match o {
                        T1Object::BBox(r) => T1FieldRef::BBox(r),
                        _ => T1FieldRef::None,
                    }
                }
                access
            }),
            array_max: 0,
            dict: 0,
        }
    };
}

/// `cid_field_records`: `cidtoken.h` and the callbacks.
pub static CID_FIELD_RECORDS: [CidField; 48] = [
    /* cidtoken.h */

    /* FT_STRUCTURE CID_FaceInfoRec, T1CODE T1_FIELD_LOCATION_CID_INFO */
    t1_field!(
        "CIDFontName",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_KEY,
        CidInfo,
        cid_font_name,
        String,
        0,
        0
    ),
    t1_field!(
        "CIDFontVersion",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_FIXED,
        CidInfo,
        cid_version,
        Long,
        0,
        0
    ),
    t1_field!(
        "CIDFontType",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER,
        CidInfo,
        cid_font_type,
        Int,
        0,
        0
    ),
    t1_field!(
        "Registry",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_STRING,
        CidInfo,
        registry,
        String,
        0,
        0
    ),
    t1_field!(
        "Ordering",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_STRING,
        CidInfo,
        ordering,
        String,
        0,
        0
    ),
    t1_field!(
        "Supplement",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER,
        CidInfo,
        supplement,
        Int,
        0,
        0
    ),
    t1_field!(
        "UIDBase",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER,
        CidInfo,
        uid_base,
        ULong,
        0,
        0
    ),
    t1_table_field!(
        "XUID",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        CidInfo,
        xuid,
        ULongs,
        num_xuid,
        Int,
        16,
        0
    ),
    t1_field!(
        "CIDMapOffset",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER,
        CidInfo,
        cidmap_offset,
        ULong,
        0,
        0
    ),
    t1_field!(
        "FDBytes",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER,
        CidInfo,
        fd_bytes,
        UInt,
        0,
        0
    ),
    t1_field!(
        "GDBytes",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER,
        CidInfo,
        gd_bytes,
        UInt,
        0,
        0
    ),
    t1_field!(
        "CIDCount",
        T1_FIELD_LOCATION_CID_INFO,
        T1_FIELD_TYPE_INTEGER,
        CidInfo,
        cid_count,
        ULong,
        0,
        0
    ),
    /* FT_STRUCTURE PS_FontInfoRec, T1CODE T1_FIELD_LOCATION_FONT_INFO */
    t1_field!(
        "version",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        version,
        String,
        0,
        0
    ),
    t1_field!(
        "Notice",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        notice,
        String,
        0,
        0
    ),
    t1_field!(
        "FullName",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        full_name,
        String,
        0,
        0
    ),
    t1_field!(
        "FamilyName",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        family_name,
        String,
        0,
        0
    ),
    t1_field!(
        "Weight",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_STRING,
        FontInfo,
        weight,
        String,
        0,
        0
    ),
    t1_field!(
        "ItalicAngle",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_INTEGER,
        FontInfo,
        italic_angle,
        Long,
        0,
        0
    ),
    t1_field!(
        "isFixedPitch",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_BOOL,
        FontInfo,
        is_fixed_pitch,
        Byte,
        0,
        0
    ),
    t1_field!(
        "UnderlinePosition",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_INTEGER,
        FontInfo,
        underline_position,
        Short,
        0,
        0
    ),
    t1_field!(
        "UnderlineThickness",
        T1_FIELD_LOCATION_FONT_INFO,
        T1_FIELD_TYPE_INTEGER,
        FontInfo,
        underline_thickness,
        UShort,
        0,
        0
    ),
    /* FT_STRUCTURE PS_FontExtraRec, T1CODE T1_FIELD_LOCATION_FONT_EXTRA */
    t1_field!(
        "FSType",
        T1_FIELD_LOCATION_FONT_EXTRA,
        T1_FIELD_TYPE_INTEGER,
        FontExtra,
        fs_type,
        UShort,
        0,
        0
    ),
    /* FT_STRUCTURE CID_FaceDictRec, T1CODE T1_FIELD_LOCATION_FONT_DICT */
    t1_field!(
        "PaintType",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        CidDict,
        paint_type,
        Byte,
        0,
        0
    ),
    t1_field!(
        "FontType",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        CidDict,
        font_type,
        Byte,
        0,
        0
    ),
    t1_field!(
        "SubrMapOffset",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        CidDict,
        subrmap_offset,
        ULong,
        0,
        0
    ),
    t1_field!(
        "SDBytes",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        CidDict,
        sd_bytes,
        UInt,
        0,
        0
    ),
    t1_field!(
        "SubrCount",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        CidDict,
        num_subrs,
        UInt,
        0,
        0
    ),
    t1_field!(
        "lenBuildCharArray",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        CidDict,
        len_buildchar,
        UInt,
        0,
        0
    ),
    t1_field!(
        "ForceBoldThreshold",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_FIXED,
        CidDict,
        forcebold_threshold,
        Long,
        0,
        0
    ),
    t1_field!(
        "StrokeWidth",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_FIXED,
        CidDict,
        stroke_width,
        Long,
        0,
        0
    ),
    /* FT_STRUCTURE PS_PrivateRec, T1CODE T1_FIELD_LOCATION_PRIVATE */
    t1_field!(
        "UniqueID",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        unique_id,
        Int,
        0,
        0
    ),
    t1_field!(
        "lenIV",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        lenIV,
        Int,
        0,
        0
    ),
    t1_field!(
        "LanguageGroup",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        language_group,
        Long,
        0,
        0
    ),
    t1_field!(
        "password",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        password,
        Long,
        0,
        0
    ),
    t1_field!(
        "BlueScale",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_FIXED_1000,
        Private,
        blue_scale,
        Long,
        0,
        0
    ),
    t1_field!(
        "BlueShift",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        blue_shift,
        Int,
        0,
        0
    ),
    t1_field!(
        "BlueFuzz",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER,
        Private,
        blue_fuzz,
        Int,
        0,
        0
    ),
    t1_table_field!(
        "BlueValues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        blue_values,
        Shorts,
        num_blue_values,
        Byte,
        14,
        0
    ),
    t1_table_field!(
        "OtherBlues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        other_blues,
        Shorts,
        num_other_blues,
        Byte,
        10,
        0
    ),
    t1_table_field!(
        "FamilyBlues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        family_blues,
        Shorts,
        num_family_blues,
        Byte,
        14,
        0
    ),
    t1_table_field!(
        "FamilyOtherBlues",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        family_other_blues,
        Shorts,
        num_family_other_blues,
        Byte,
        10,
        0
    ),
    t1_table_field2!(
        "StdHW",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        standard_width,
        UShorts,
        1,
        0
    ),
    t1_table_field2!(
        "StdVW",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        standard_height,
        UShorts,
        1,
        0
    ),
    t1_table_field2!(
        "MinFeature",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        min_feature,
        Shorts,
        2,
        0
    ),
    t1_table_field!(
        "StemSnapH",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        snap_widths,
        Shorts,
        num_snap_widths,
        Byte,
        12,
        0
    ),
    t1_table_field!(
        "StemSnapV",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_INTEGER_ARRAY,
        Private,
        snap_heights,
        Shorts,
        num_snap_heights,
        Byte,
        12,
        0
    ),
    t1_field!(
        "ForceBold",
        T1_FIELD_LOCATION_PRIVATE,
        T1_FIELD_TYPE_BOOL,
        Private,
        force_bold,
        Byte,
        0,
        0
    ),
    /* FT_STRUCTURE FT_BBox, T1CODE T1_FIELD_LOCATION_BBOX */
    cid_bbox_field!("FontBBox"),
];

/// The callbacks of `cid_field_records`, after `cidtoken.h`, and the
/// table's terminating entry (C's NULL `ident` is empty).
pub static CID_FIELD_CALLBACKS: [CidField; 5] = [
    t1_callback_field!("FDArray", parse_fd_array as CidFieldReader, 0),
    t1_callback_field!("FontMatrix", cid_parse_font_matrix as CidFieldReader, 0),
    t1_callback_field!(
        "ExpansionFactor",
        parse_expansion_factor as CidFieldReader,
        0
    ),
    t1_callback_field!("FontName", parse_font_name as CidFieldReader, 0),
    T1FieldRec {
        ident: "",
        location: T1_FIELD_LOCATION_CID_INFO,
        type_: T1_FIELD_TYPE_NONE,
        reader: None,
        access: None,
        array_max: 0,
        dict: 0,
    },
];

/// `cid_parse_dict` (the PostScript section is the parser's, of `size`
/// bytes)
fn cid_parse_dict(face: &mut CidFaceRec, loader: &mut CidLoader, size: FtULong) -> FtResult<()> {
    loader.parser.root.cursor = 0;
    loader.parser.root.limit = size as usize;
    loader.parser.root.error = FT_ERR_OK;

    {
        let mut cur: usize = 0;
        let limit: usize = size as usize;

        loop {
            loader.parser.root.cursor = cur;
            cid_parser_skip_spaces(&mut loader.parser);

            let newlimit: isize = if loader.parser.root.cursor >= limit {
                limit as isize - 1 - 17
            } else {
                loader.parser.root.cursor as isize - 17
            };

            /* look for `%ADOBeginFontDict' */
            while (cur as isize) < newlimit {
                let b = &loader.parser.postscript;
                if at(b, cur) == b'%' && b.get(cur..cur + 17) == Some(&b"%ADOBeginFontDict"[..]) {
                    /* if /FDArray was found, then cid->num_dicts is > 0, and */
                    /* we can start increasing parser->num_dict               */
                    if face.cid.num_dicts > 0 {
                        loader.parser.num_dict = loader.parser.num_dict.wrapping_add(1);
                    }
                }
                cur += 1;
            }

            cur = loader.parser.root.cursor;
            /* no error can occur in cid_parser_skip_spaces */
            if cur >= limit {
                break;
            }

            cid_parser_skip_ps_token(&mut loader.parser);
            if loader.parser.root.cursor >= limit || loader.parser.root.error != 0 {
                break;
            }

            /* look for immediates */
            if at(&loader.parser.postscript, cur) == b'/' && cur + 2 < limit {
                cur += 1;
                let len = loader.parser.root.cursor - cur;

                if len > 0 && len < 22 {
                    /* now compare the immediate name to the keyword table */
                    for keyword in CID_FIELD_RECORDS.iter().chain(CID_FIELD_CALLBACKS.iter()) {
                        let name = keyword.ident.as_bytes();
                        if name.is_empty() {
                            break;
                        }

                        let b = &loader.parser.postscript;
                        if at(b, cur) == name[0] && len == name.len() {
                            let mut n = 1;
                            while n < len {
                                if at(b, cur + n) != name[n] {
                                    break;
                                }
                                n += 1;
                            }

                            if n >= len {
                                /* we found it - run the parsing callback */
                                loader.parser.root.error =
                                    match cid_load_keyword(face, loader, keyword) {
                                        Ok(()) => FT_ERR_OK,
                                        Err(e) => e,
                                    };
                                if loader.parser.root.error != 0 {
                                    return Err(loader.parser.root.error);
                                }
                                break;
                            }
                        }
                    }
                }
            }

            cur = loader.parser.root.cursor;
        }

        if face.cid.num_dicts == 0 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
    }

    parser_result(loader.parser.root.error)
}

/// `cid_read_subrs`: read the subrmap and the subrs of each font dict
/// (`stream` is the face's `cid_stream`)
fn cid_read_subrs(face: &mut CidFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let cid = &face.cid;
    let mut max_offsets: FtUInt = 0;
    let mut offsets: Vec<FtULong> = Vec::new();

    let r = (|| -> FtResult<Vec<CidSubrsRec>> {
        let mut subrs: Vec<CidSubrsRec> = ft_new_array(cid.num_dicts as FtLong)?;

        for n in 0..cid.num_dicts as usize {
            let subr = &mut subrs[n];
            let dict = &cid.font_dicts[n];
            let len_iv: FtInt = dict.private_dict.lenIV;
            let num_subrs: FtUInt = dict.num_subrs;

            if num_subrs == 0 {
                continue;
            }

            /* reallocate offsets array if needed */
            if num_subrs.wrapping_add(1) > max_offsets {
                /* (FT_PAD_CEIL in unsigned arithmetic) */
                let new_max: FtUInt = num_subrs.wrapping_add(1).wrapping_add(3) & !3;

                if new_max <= max_offsets {
                    return Err(FT_ERR_SYNTAX_ERROR);
                }

                offsets
                    .try_reserve_exact((new_max - max_offsets) as usize)
                    .map_err(|_| FT_ERR_OUT_OF_MEMORY)?;
                offsets.resize(new_max as usize, 0);

                max_offsets = new_max;
            }

            /* read the subrmap's offsets */
            stream.seek(cid.data_offset.wrapping_add(dict.subrmap_offset))?;
            stream.enter_frame(num_subrs.wrapping_add(1).wrapping_mul(dict.sd_bytes) as FtULong)?;

            {
                let b = stream.frame_data();
                let mut p = 0usize;
                for count in 0..=num_subrs as usize {
                    offsets[count] = cid_get_offset(b, &mut p, dict.sd_bytes);
                }
            }

            stream.exit_frame();

            /* offsets must be ordered */
            for count in 1..=num_subrs as usize {
                if offsets[count - 1] > offsets[count] {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }
            }

            if offsets[num_subrs as usize] > stream.size.wrapping_sub(cid.data_offset) {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            /* now, compute the size of subrs charstrings, */
            /* allocate, and read them                     */
            let data_len: FtULong = offsets[num_subrs as usize] - offsets[0];

            let mut code: Vec<usize> = ft_new_array(num_subrs as FtLong + 1)?;
            let mut bytes = ft_qalloc(data_len as FtLong)?;

            stream.seek(cid.data_offset.wrapping_add(offsets[0]))?;
            stream.read(&mut bytes)?;

            /* set up pointers */
            for count in 1..=num_subrs as usize {
                let len = offsets[count] - offsets[count - 1];
                code[count] = code[count - 1] + len as usize;
            }

            /* decrypt subroutines, but only if lenIV >= 0 */
            if len_iv >= 0 {
                for count in 0..num_subrs as usize {
                    let len = (offsets[count + 1] - offsets[count]) as usize;
                    let start = code[count];
                    t1_decrypt(&mut bytes[start..], len, 4330);
                }
            }

            subr.code = Some(Arc::from(code));
            subr.code_bytes = Some(Arc::from(bytes));
            subr.num_subrs = num_subrs as FtInt;
        }

        Ok(subrs)
    })();

    match r {
        Ok(subrs) => {
            face.subrs = subrs;
            Ok(())
        }
        Err(e) => {
            /* Fail: */
            face.subrs = Vec::new();
            Err(e)
        }
    }

    /* Exit: */
}

/// `cid_init_loader`
fn cid_init_loader(loader: &mut CidLoader, _face: &CidFaceRec) {
    *loader = CidLoader::default();
}

/// `cid_done_loader`
fn cid_done_loader(loader: &mut CidLoader) {
    /* finalize parser */
    cid_parser_done(&mut loader.parser);
}

/// `cid_hex_to_binary` (the face's stream is `stream`)
fn cid_hex_to_binary(
    data: &mut [u8],
    data_len: FtULong,
    offset: FtULong,
    stream: &mut FtStreamRec,
    data_written: &mut FtULong,
) -> FtResult<()> {
    let mut buffer = [0u8; 256];
    let mut p: usize = 0;
    let mut plimit: usize = 0;
    let mut d: usize = 0;
    let dlimit: usize = data_len as usize;

    let mut upper_nibble = true;
    let mut done = false;

    let error = (|| -> FtResult<()> {
        stream.seek(offset)?;

        p = 0;
        plimit = p;

        while d < dlimit {
            if p >= plimit {
                let oldpos = stream.pos();
                let size = stream.size - oldpos;

                if size == 0 {
                    return Err(FT_ERR_SYNTAX_ERROR);
                }

                let n = if 256 > size { size as usize } else { 256 };
                stream.read(&mut buffer[..n])?;
                p = 0;
                plimit = p + (stream.pos() - oldpos) as usize;
            }

            let c = buffer[p];
            let val: FtByte;
            if c.is_ascii_digit() {
                val = c - b'0';
            } else if (b'a'..=b'f').contains(&c) {
                val = c - b'a' + 10;
            } else if (b'A'..=b'F').contains(&c) {
                val = c - b'A' + 10;
            } else if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' || c == 0x0C || c == 0 {
                p += 1;
                continue;
            } else if c == b'>' {
                val = 0;
                done = true;
            } else {
                return Err(FT_ERR_SYNTAX_ERROR);
            }

            if upper_nibble {
                data[d] = val << 4;
            } else {
                data[d] = data[d].wrapping_add(val);
                d += 1;
            }

            upper_nibble = !upper_nibble;

            if done {
                break;
            }

            p += 1;
        }

        Ok(())
    })();

    /* Exit: */
    *data_written = d as FtULong;
    error
}

/// `cid_face_open`
pub fn cid_face_open(face: &mut CidFaceRec, face_index: FtInt) -> FtResult<()> {
    let mut loader = CidLoader::default();

    cid_init_loader(&mut loader, face);

    let error = cid_face_open_loader(face, face_index, &mut loader);

    /* Exit: */
    cid_done_loader(&mut loader);
    error
}

fn cid_face_open_loader(
    face: &mut CidFaceRec,
    face_index: FtInt,
    loader: &mut CidLoader,
) -> FtResult<()> {
    {
        let Some(stream) = face.root.stream.as_mut() else {
            return Err(FT_ERR_INVALID_STREAM_HANDLE);
        };
        cid_parser_new(&mut loader.parser, stream)?;
    }

    let postscript_len = loader.parser.postscript_len;
    cid_parse_dict(face, loader, postscript_len)?;

    if face_index < 0 {
        return Ok(());
    }

    let parser = &mut loader.parser;
    let mut binary_length: FtULong = 0;
    let stream_size = face.root.stream.as_ref().map_or(0, |s| s.size);

    if parser.binary_length != 0 {
        if parser.binary_length > stream_size.wrapping_sub(parser.data_offset) {
            parser.binary_length = stream_size.wrapping_sub(parser.data_offset);
        }

        /* we must convert the data section from hexadecimal to binary */
        let mut binary_data = ft_qalloc(parser.binary_length as FtLong)?;
        {
            let Some(stream) = face.root.stream.as_mut() else {
                return Err(FT_ERR_INVALID_STREAM_HANDLE);
            };
            let len = parser.binary_length;
            cid_hex_to_binary(
                &mut binary_data,
                len,
                parser.data_offset,
                stream,
                &mut binary_length,
            )?;
        }

        binary_data.truncate(binary_length as usize);
        face.cid_stream = Some(FtStreamRec::open_memory(Arc::from(binary_data)));
        face.cid.data_offset = 0;
    } else {
        /* (C's copy of the face's stream: the face's stream itself) */
        face.cid_stream = None;
        face.cid.data_offset = parser.data_offset;
    }

    let cid = &mut face.cid;

    /* sanity tests */

    if cid.gd_bytes == 0 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* allow at most 32bit offsets */
    if cid.fd_bytes > 4 || cid.gd_bytes > 4 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let cid_stream_size = match face.cid_stream.as_ref() {
        Some(s) => s.size,
        None => stream_size,
    };
    binary_length = cid_stream_size.wrapping_sub(cid.data_offset);

    if cid.cidmap_offset > binary_length {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* the initial pre-check prevents the multiplication overflow */
    if cid.cid_count > FtULong::MAX / 8
        || cid.cid_count * (cid.fd_bytes + cid.gd_bytes) as FtULong
            > binary_length - cid.cidmap_offset
    {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    for dict in cid.font_dicts.iter_mut() {
        /* the upper limits are ad-hoc values */
        if dict.private_dict.blue_shift > 1000 || dict.private_dict.blue_shift < 0 {
            dict.private_dict.blue_shift = 7;
        }

        if dict.private_dict.blue_fuzz > 1000 || dict.private_dict.blue_fuzz < 0 {
            dict.private_dict.blue_fuzz = 1;
        }

        if dict.num_subrs != 0 && dict.sd_bytes == 0 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        if dict.sd_bytes > 4 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        if dict.subrmap_offset > binary_length {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        /* the initial pre-check prevents the multiplication overflow */
        if dict.num_subrs > FtUInt::MAX / 4
            || (dict.num_subrs * dict.sd_bytes) as FtULong > binary_length - dict.subrmap_offset
        {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
    }

    /* we can now safely proceed */
    with_cid_stream(face, cid_read_subrs)
}

/// Runs `f` with the face's `cid_stream` (the face's own stream when it
/// is C's copy of it).
pub fn with_cid_stream<R>(
    face: &mut CidFaceRec,
    f: impl FnOnce(&mut CidFaceRec, &mut FtStreamRec) -> R,
) -> R {
    if let Some(mut stream) = face.cid_stream.take() {
        let r = f(face, &mut stream);
        if face.cid_stream.is_none() {
            face.cid_stream = Some(stream);
        }
        r
    } else {
        let mut stream = face.root.stream.take().expect("face without a stream");
        let r = f(face, &mut stream);
        if face.root.stream.is_none() {
            face.root.stream = Some(stream);
        }
        r
    }
}
