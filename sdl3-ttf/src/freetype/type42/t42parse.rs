// Rust translation of src/type42/t42parse.c and src/type42/t42parse.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by Roberto Alameda.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 42 font parser (body).
//!
//! The base dictionary is an owned buffer (a memory stream's is a copy of
//! its bytes, which C points to); the root parser's positions are offsets
//! into it. The keyword table (`t42_keywords`) is [`T42_KEYWORDS`].

use super::super::base::ftcalc::{ft_div_fix, ft_matrix_check};
use super::super::base::ftmemory::{ft_new_array, ft_qalloc};
use super::super::base::ftstream::{ft_peek_ulong, FtStreamRec};
use super::super::fttypes::*;
use super::super::psaux::psconv::at;
use super::super::psaux::psobjs::*;
use super::super::t1tables::*;
use super::t42objs::T42FaceRec;
use crate::{t1_callback_field, t1_field};

/// `T42_ParserRec`
#[derive(Debug, Clone, Default)]
pub struct T42ParserRec {
    pub root: PsParserRec,
    pub base_dict: Vec<u8>,
    pub base_len: FtLong,
    pub in_memory: bool,
}

/// `T42_Loader`
#[derive(Debug, Default)]
pub struct T42LoaderRec {
    pub parser: T42ParserRec, /* parser used to read the stream */

    pub num_chars: FtInt,           /* number of characters in encoding */
    pub encoding_table: PsTableRec, /* PS_Table used to store the       */
    /* encoding character names         */
    pub num_glyphs: FtInt,
    pub glyph_names: PsTableRec,
    pub charstrings: PsTableRec,
    pub swap_table: PsTableRec, /* For moving .notdef glyph to index 0. */
}

/// The type of the keyword callbacks (`T1_Field_ParseFunc`).
pub type T42FieldReader = fn(face: &mut T42FaceRec, loader: &mut T42LoaderRec);

/// A keyword of the Type 42 loader.
pub type T42Field = T1FieldRec<T42FieldReader>;

/// The bounding box field (`T1_FIELD_BBOX`: the whole record, at the
/// offset of its `xMin`).
macro_rules! t42_bbox_field {
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

/* as Type42 fonts have no Private dict,         */
/* we set the last argument of T1_FIELD_XXX to 0 */

/// `t42_keywords`
pub static T42_KEYWORDS: [T42Field; 20] = [
    /* FT_STRUCTURE T1_FontInfo, T1CODE T1_FIELD_LOCATION_FONT_INFO */
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
    /* FT_STRUCTURE T1_FontRec, T1CODE T1_FIELD_LOCATION_FONT_DICT */
    t1_field!(
        "FontName",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_KEY,
        Font,
        font_name,
        String,
        0,
        0
    ),
    t1_field!(
        "PaintType",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        Font,
        paint_type,
        Byte,
        0,
        0
    ),
    t1_field!(
        "FontType",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_INTEGER,
        Font,
        font_type,
        Byte,
        0,
        0
    ),
    t1_field!(
        "StrokeWidth",
        T1_FIELD_LOCATION_FONT_DICT,
        T1_FIELD_TYPE_FIXED,
        Font,
        stroke_width,
        Long,
        0,
        0
    ),
    /* FT_STRUCTURE FT_BBox, T1CODE T1_FIELD_LOCATION_BBOX */
    t42_bbox_field!("FontBBox"),
    t1_callback_field!("FontMatrix", t42_parse_font_matrix as T42FieldReader, 0),
    t1_callback_field!("Encoding", t42_parse_encoding as T42FieldReader, 0),
    t1_callback_field!("CharStrings", t42_parse_charstrings as T42FieldReader, 0),
    t1_callback_field!("sfnts", t42_parse_sfnts as T42FieldReader, 0),
    /* (the terminating NULL entry: an empty `ident') */
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

/* (the parsing macros: the root parser on the base dictionary) */

fn t1_skip_spaces(p: &mut T42ParserRec) {
    ps_parser_skip_spaces(&mut p.root, &p.base_dict);
}

fn t1_skip_ps_token(p: &mut T42ParserRec) {
    ps_parser_skip_ps_token(&mut p.root, &p.base_dict);
}

fn t1_to_int(p: &mut T42ParserRec) -> FtLong {
    ps_parser_to_int(&mut p.root, &p.base_dict)
}

fn t1_to_fixed_array(p: &mut T42ParserRec, m: FtInt, f: Option<&mut [FtFixed]>, t: FtInt) -> FtInt {
    ps_parser_to_fixed_array(&mut p.root, &p.base_dict, m, f, t)
}

fn t1_to_token(p: &mut T42ParserRec, t: &mut T1TokenRec) {
    ps_parser_to_token(&mut p.root, &p.base_dict, t);
}

/// An error as the parser's error code.
fn error_code(r: FtResult<()>) -> FtError {
    match r {
        Ok(()) => FT_ERR_OK,
        Err(e) => e,
    }
}

/********************* Parsing Functions ******************/

/// `t42_parser_init`
pub fn t42_parser_init(parser: &mut T42ParserRec, stream: &mut FtStreamRec) -> FtResult<()> {
    ps_parser_init(&mut parser.root, 0, 0);

    parser.base_len = 0;
    parser.base_dict = Vec::new();
    parser.in_memory = false;

    /********************************************************************
     *
     * Here a short summary of what is going on:
     *
     *   When creating a new Type 42 parser, we try to locate and load
     *   the base dictionary, loading the whole font into memory.
     *
     *   When `loading' the base dictionary, we only set up pointers
     *   in the case of a memory-based stream.  Otherwise, we allocate
     *   and load the base dictionary in it.
     *
     *   parser->in_memory is set if we have a memory stream.
     */

    let error = (|| -> FtResult<()> {
        stream.seek(0)?;
        stream.enter_frame(17)?;

        let mut error = Ok(());
        if stream.frame_data().get(..17) != Some(&b"%!PS-TrueTypeFont"[..]) {
            error = Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }

        stream.exit_frame();

        error?;
        stream.seek(0)?;

        let size: FtLong = stream.size as FtLong;

        /* now, try to load `size' bytes of the `base' dictionary we */
        /* found previously                                          */

        /* if it is a memory-based resource, set up pointers */
        if stream.read.is_none() {
            parser.in_memory = true;
            let pos = stream.pos() as usize;

            /* check that the `size' field is valid */
            stream.skip(size)?;

            /* (a copy of the stream's bytes) */
            let base = stream
                .memory_base()
                .cloned()
                .unwrap_or_else(|| Vec::new().into());
            let mut dict = ft_qalloc(size)?;
            dict.copy_from_slice(&base[pos..pos + size as usize]);
            parser.base_dict = dict;
            parser.base_len = size;
        } else {
            /* read segment in memory */
            let mut dict = ft_qalloc(size)?;
            stream.read(&mut dict)?;
            parser.base_dict = dict;

            parser.base_len = size;
        }

        parser.root.base = 0;
        parser.root.cursor = 0;
        parser.root.limit = parser.root.cursor + parser.base_len as usize;

        Ok(())
    })();

    /* Exit: */
    if error.is_err() && !parser.in_memory {
        parser.base_dict = Vec::new();
    }

    error
}

/// `t42_parser_done`
pub fn t42_parser_done(parser: &mut T42ParserRec) {
    /* free the base dictionary only when we have a disk stream */
    /* (the copy of a memory stream's dictionary goes, too)     */
    parser.base_dict = Vec::new();

    ps_parser_done(&mut parser.root);
}

/// `t42_is_space`
fn t42_is_space(c: FtByte) -> bool {
    c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' || c == 0x0C || c == b'\0'
}

/// `t42_parse_font_matrix`
fn t42_parse_font_matrix(face: &mut T42FaceRec, loader: &mut T42LoaderRec) {
    let parser = &mut loader.parser;
    let mut temp: [FtFixed; 6] = [0; 6];

    let result = t1_to_fixed_array(parser, 6, Some(&mut temp), 0);

    if result < 6 {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    let temp_scale: FtFixed = temp[3].abs();

    if temp_scale == 0 {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    /* atypical case */
    if temp_scale != 0x10000 {
        temp[0] = ft_div_fix(temp[0], temp_scale);
        temp[1] = ft_div_fix(temp[1], temp_scale);
        temp[2] = ft_div_fix(temp[2], temp_scale);
        temp[4] = ft_div_fix(temp[4], temp_scale);
        temp[5] = ft_div_fix(temp[5], temp_scale);
        temp[3] = if temp[3] < 0 { -0x10000 } else { 0x10000 };
    }

    let matrix = &mut face.type1.font_matrix;
    matrix.xx = temp[0];
    matrix.yx = temp[1];
    matrix.xy = temp[2];
    matrix.yy = temp[3];

    if !ft_matrix_check(matrix) {
        parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    /* note that the offsets must be expressed in integer font units */
    let offset = &mut face.type1.font_offset;
    offset.x = temp[4] >> 16;
    offset.y = temp[5] >> 16;
}

/// `t42_parse_encoding`
fn t42_parse_encoding(face: &mut T42FaceRec, loader: &mut T42LoaderRec) {
    let limit = loader.parser.root.limit;

    t1_skip_spaces(&mut loader.parser);
    let mut cur = loader.parser.root.cursor;
    if cur >= limit {
        loader.parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
        return;
    }

    let c = at(&loader.parser.base_dict, cur);

    /* if we have a number or `[', the encoding is an array, */
    /* and we must load it now                               */
    if c.is_ascii_digit() || c == b'[' {
        let encode = &mut face.type1.encoding;
        let char_table = &mut loader.encoding_table;
        let parser = &mut loader.parser;
        let count: FtInt;
        let mut only_immediates = false;

        /* read the number of entries in the encoding; should be 256 */
        if c == b'[' {
            count = 256;
            only_immediates = true;
            parser.root.cursor += 1;
        } else {
            count = t1_to_int(parser) as FtInt;
        }

        /* only composite fonts (which we don't support) */
        /* can have larger values                        */
        if count > 256 {
            parser.root.error = FT_ERR_INVALID_FILE_FORMAT;
            return;
        }

        t1_skip_spaces(parser);
        if parser.root.cursor >= limit {
            return;
        }

        /* PostScript happily allows overwriting of encoding arrays */
        if !encode.char_index.is_empty() {
            encode.char_index = Vec::new();
            encode.char_name = Vec::new();
            ps_table_release(char_table);
        }

        /* we use a T1_Table to store our charnames */
        loader.num_chars = count;
        encode.num_chars = count;
        let r = (|| -> FtResult<()> {
            encode.char_index = ft_new_array(count as FtLong)?;
            encode.char_name = ft_new_array(count as FtLong)?;
            ps_table_new(char_table, count)
        })();
        if let Err(error) = r {
            parser.root.error = error;
            return;
        }

        /* We need to `zero' out encoding_table.elements */
        for n in 0..count {
            let _ = ps_table_add(char_table, n, b".notdef\0", 8);
        }

        /* Now we need to read records of the form                */
        /*                                                        */
        /*   ... charcode /charname ...                           */
        /*                                                        */
        /* for each entry in our table.                           */
        /*                                                        */
        /* We simply look for a number followed by an immediate   */
        /* name.  Note that this ignores correctly the sequence   */
        /* that is often seen in type42 fonts:                    */
        /*                                                        */
        /*   0 1 255 { 1 index exch /.notdef put } for dup        */
        /*                                                        */
        /* used to clean the encoding array before anything else. */
        /*                                                        */
        /* Alternatively, if the array is directly given as       */
        /*                                                        */
        /*   /Encoding [ ... ]                                    */
        /*                                                        */
        /* we only read immediates.                               */

        let mut n: FtInt = 0;
        t1_skip_spaces(parser);

        while parser.root.cursor < limit {
            cur = parser.root.cursor;
            let b = &parser.base_dict;

            /* we stop when we encounter `def' or `]' */
            if at(b, cur) == b'd'
                && cur + 3 < limit
                && at(b, cur + 1) == b'e'
                && at(b, cur + 2) == b'f'
                && t42_is_space(at(b, cur + 3))
            {
                cur += 3;
                break;
            }
            if at(b, cur) == b']' {
                cur += 1;
                break;
            }

            /* check whether we have found an entry */
            if at(b, cur).is_ascii_digit() || only_immediates {
                let charcode: FtInt;

                if only_immediates {
                    charcode = n;
                } else {
                    charcode = t1_to_int(parser) as FtInt;
                    t1_skip_spaces(parser);

                    /* protect against invalid charcode */
                    if cur == parser.root.cursor {
                        parser.root.error = FT_ERR_UNKNOWN_FILE_FORMAT;
                        return;
                    }
                }

                cur = parser.root.cursor;

                if cur + 2 < limit && at(&parser.base_dict, cur) == b'/' && n < count {
                    cur += 1;

                    parser.root.cursor = cur;
                    t1_skip_ps_token(parser);
                    if parser.root.cursor >= limit {
                        return;
                    }
                    if parser.root.error != 0 {
                        return;
                    }

                    let len = parser.root.cursor - cur;

                    let b = &parser.base_dict;
                    let src = b.get(cur..(cur + len + 1).min(b.len())).unwrap_or(&[]);
                    if let Err(error) = ps_table_add(char_table, charcode, src, len as FtUInt + 1) {
                        parser.root.error = error;
                        return;
                    }
                    if let Some(start) = char_table.elements[charcode as usize] {
                        char_table.block[start + len] = b'\0';
                    }

                    n += 1;
                } else if only_immediates {
                    /* Since the current position is not updated for           */
                    /* immediates-only mode we would get an infinite loop if   */
                    /* we don't do anything here.                              */
                    /*                                                         */
                    /* This encoding array is not valid according to the       */
                    /* type42 specification (it might be an encoding for a CID */
                    /* type42 font, however), so we conclude that this font is */
                    /* NOT a type42 font.                                      */
                    parser.root.error = FT_ERR_UNKNOWN_FILE_FORMAT;
                    return;
                }
            } else {
                t1_skip_ps_token(parser);
                if parser.root.error != 0 {
                    return;
                }
            }

            t1_skip_spaces(parser);
        }

        face.type1.encoding_type = T1_ENCODING_TYPE_ARRAY;
        parser.root.cursor = cur;
    }
    /* Otherwise, we should have either `StandardEncoding', */
    /* `ExpertEncoding', or `ISOLatin1Encoding'             */
    else {
        let b = &loader.parser.base_dict;
        let starts = |s: &[u8]| b.get(cur..cur + s.len()) == Some(s);

        if cur + 17 < limit && starts(b"StandardEncoding") {
            face.type1.encoding_type = T1_ENCODING_TYPE_STANDARD;
        } else if cur + 15 < limit && starts(b"ExpertEncoding") {
            face.type1.encoding_type = T1_ENCODING_TYPE_EXPERT;
        } else if cur + 18 < limit && starts(b"ISOLatin1Encoding") {
            face.type1.encoding_type = T1_ENCODING_TYPE_ISOLATIN1;
        } else {
            loader.parser.root.error = FT_ERR_IGNORE;
        }
    }
}

/// `T42_Load_Status`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum T42LoadStatus {
    BeforeStart,
    BeforeTableDir,
    OtherTables,
}

/// The current string of the sfnts array (C's `string_buf`): none yet,
/// binary data in the base dictionary (at an offset), or hex data decoded
/// into an allocated buffer (C's `allocated`).
#[derive(Debug)]
enum StringBuf {
    None,
    Binary(usize),
    Hex(Vec<u8>),
}

/// `t42_parse_sfnts`
fn t42_parse_sfnts(face: &mut T42FaceRec, loader: &mut T42LoaderRec) {
    let parser = &mut loader.parser;
    let limit = parser.root.limit;
    let mut num_tables: FtInt = 0;
    let mut ttf_count: FtLong;
    let mut ttf_reserved: FtLong;

    let mut string_size: FtULong;
    let mut real_size: FtULong = 0;
    let mut string_buf = StringBuf::None;

    let mut status: T42LoadStatus;

    /* There should only be one sfnts array, but free any previous. */
    face.ttf_data = Vec::new();
    face.ttf_size = 0;

    /* The format is                                */
    /*                                              */
    /*   /sfnts [ <hexstring> <hexstring> ... ] def */
    /*                                              */
    /* or                                           */
    /*                                              */
    /*   /sfnts [                                   */
    /*      <num_bin_bytes> RD <binary data>        */
    /*      <num_bin_bytes> RD <binary data>        */
    /*      ...                                     */
    /*   ] def                                      */
    /*                                              */
    /* with exactly one space after the `RD' token. */

    let fail = 'exit: {
        t1_skip_spaces(parser);

        if parser.root.cursor >= limit || {
            let c = at(&parser.base_dict, parser.root.cursor);
            parser.root.cursor += 1;
            c != b'['
        } {
            break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
        }

        t1_skip_spaces(parser);
        status = T42LoadStatus::BeforeStart;
        string_size = 0;
        ttf_count = 0;
        ttf_reserved = 12;
        match ft_qalloc(ttf_reserved) {
            Ok(data) => face.ttf_data = data,
            Err(e) => break 'exit Some(e),
        }

        while parser.root.cursor < limit {
            let cur = parser.root.cursor;
            let c = at(&parser.base_dict, cur);

            if c == b']' {
                parser.root.cursor += 1;
                face.ttf_size = ttf_count;
                break 'exit None;
            } else if c == b'<' {
                if matches!(string_buf, StringBuf::Binary(_)) {
                    break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                }

                t1_skip_ps_token(parser);
                if parser.root.error != 0 {
                    break 'exit None;
                }

                /* don't include delimiters */
                string_size =
                    ((parser.root.cursor as FtLong - cur as FtLong - 2 + 1) / 2) as FtULong;
                if string_size == 0 {
                    break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                }
                let mut buf = match std::mem::replace(&mut string_buf, StringBuf::None) {
                    StringBuf::Hex(buf) => buf,
                    _ => Vec::new(),
                };
                if buf
                    .try_reserve_exact((string_size as usize).saturating_sub(buf.len()))
                    .is_err()
                {
                    break 'exit Some(FT_ERR_OUT_OF_MEMORY);
                }
                buf.resize(string_size as usize, 0);

                parser.root.cursor = cur;
                let max = string_size as usize;
                let _ = ps_parser_to_bytes(
                    &mut parser.root,
                    &parser.base_dict,
                    &mut buf,
                    max,
                    &mut real_size,
                    true,
                );
                string_buf = StringBuf::Hex(buf);
                string_size = real_size;
            } else if c.is_ascii_digit() {
                if matches!(string_buf, StringBuf::Hex(_)) {
                    break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                }

                let tmp: FtLong = t1_to_int(parser);
                if tmp < 0 {
                    break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                } else {
                    string_size = tmp as FtULong;
                }

                t1_skip_ps_token(parser); /* `RD' */
                if parser.root.error != 0 {
                    return;
                }

                string_buf = StringBuf::Binary(parser.root.cursor + 1); /* one space after `RD' */

                if (limit as FtLong - parser.root.cursor as FtLong) as FtULong <= string_size {
                    break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                } else {
                    parser.root.cursor += string_size as usize + 1;
                }
            }

            let bytes: &[u8] = match &string_buf {
                StringBuf::None => {
                    break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                }
                StringBuf::Binary(start) => parser.base_dict.get(*start..).unwrap_or(&[]),
                StringBuf::Hex(buf) => buf,
            };

            /* A string can have a trailing zero (odd) byte for padding. */
            /* Ignore it.                                                */
            if (string_size & 1) != 0 && at(bytes, (string_size - 1) as usize) == 0 {
                string_size -= 1;
            }

            if string_size == 0 {
                break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
            }

            /* The whole TTF is now loaded into `string_buf'.  We are */
            /* checking its contents while copying it to `ttf_data'.  */

            let size: FtULong = (limit as FtLong - parser.root.cursor as FtLong) as FtULong;

            for n in 0..string_size as usize {
                let byte = at(bytes, n);
                'state: loop {
                    match status {
                        T42LoadStatus::BeforeStart => {
                            /* load offset table, 12 bytes */
                            if ttf_count < 12 {
                                face.ttf_data[ttf_count as usize] = byte;
                                ttf_count += 1;
                                break 'state;
                            } else {
                                num_tables =
                                    16 * face.ttf_data[4] as FtInt + face.ttf_data[5] as FtInt;
                                status = T42LoadStatus::BeforeTableDir;
                                ttf_reserved = 12 + 16 * num_tables as FtLong;

                                if (size as FtLong) < ttf_reserved {
                                    break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                                }

                                if face
                                    .ttf_data
                                    .try_reserve_exact(
                                        (ttf_reserved as usize).saturating_sub(face.ttf_data.len()),
                                    )
                                    .is_err()
                                {
                                    break 'exit Some(FT_ERR_OUT_OF_MEMORY);
                                }
                                face.ttf_data.resize(ttf_reserved as usize, 0);
                            }
                            /* FALL_THROUGH */
                        }

                        T42LoadStatus::BeforeTableDir => {
                            /* the offset table is read; read the table directory */
                            if ttf_count < ttf_reserved {
                                face.ttf_data[ttf_count as usize] = byte;
                                ttf_count += 1;
                                break 'state;
                            } else {
                                for i in 0..num_tables as usize {
                                    let p = 12 + 16 * i + 12;

                                    let len: FtULong = ft_peek_ulong(&face.ttf_data, p) as FtULong;

                                    if len > size || ttf_reserved > (size - len) as FtLong {
                                        break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                                    }

                                    /* Pad to a 4-byte boundary length */
                                    ttf_reserved += ((len + 3) & !3u32 as FtULong) as FtLong;
                                }
                                ttf_reserved += 1;

                                status = T42LoadStatus::OtherTables;

                                if face
                                    .ttf_data
                                    .try_reserve_exact(
                                        (ttf_reserved as usize).saturating_sub(face.ttf_data.len()),
                                    )
                                    .is_err()
                                {
                                    break 'exit Some(FT_ERR_OUT_OF_MEMORY);
                                }
                                face.ttf_data.resize(ttf_reserved as usize, 0);
                            }
                            /* FALL_THROUGH */
                        }

                        T42LoadStatus::OtherTables => {
                            /* all other tables are just copied */
                            if ttf_count >= ttf_reserved {
                                break 'exit Some(FT_ERR_INVALID_FILE_FORMAT);
                            }
                            face.ttf_data[ttf_count as usize] = byte;
                            ttf_count += 1;
                            break 'state;
                        }
                    }
                }
            }

            t1_skip_spaces(parser);
        }

        /* if control reaches this point, the format was not valid */
        Some(FT_ERR_INVALID_FILE_FORMAT)
    };

    if let Some(error) = fail {
        /* Fail: */
        parser.root.error = error;
    }

    /* Exit: */
    if parser.root.error != 0 {
        face.ttf_data = Vec::new();
        face.ttf_size = 0;
    }
    /* (an allocated `string_buf' goes) */
}

/// The element `i` of a table and its length.
fn elem(table: &PsTableRec, i: usize) -> (&[u8], FtUInt) {
    (
        table.element(i).unwrap_or(&[]),
        table.lengths.get(i).copied().unwrap_or(0),
    )
}

/// The C string at the start of `s` (up to its first null byte).
fn c_str(s: &[u8]) -> &[u8] {
    let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..len]
}

/// `t42_parse_charstrings`
fn t42_parse_charstrings(_face: &mut T42FaceRec, loader: &mut T42LoaderRec) {
    if let Err(error) = t42_parse_charstrings_body(loader) {
        /* Fail: */
        loader.parser.root.error = error;
    }
}

fn t42_parse_charstrings_body(loader: &mut T42LoaderRec) -> FtResult<()> {
    let T42LoaderRec {
        parser,
        charstrings: code_table,
        glyph_names: name_table,
        swap_table,
        num_glyphs: loader_num_glyphs,
        ..
    } = loader;

    let limit = parser.root.limit;
    let mut notdef_index: FtInt = 0;
    let mut notdef_found = false;

    t1_skip_spaces(parser);

    if parser.root.cursor >= limit {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    let c = at(&parser.base_dict, parser.root.cursor);
    if c.is_ascii_digit() {
        *loader_num_glyphs = t1_to_int(parser) as FtInt;
        if parser.root.error != 0 {
            return Ok(());
        }
        if *loader_num_glyphs < 0 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        /* we certainly need more than 4 bytes per glyph */
        let room = (limit as FtLong - parser.root.cursor as FtLong) >> 2;
        if *loader_num_glyphs as FtLong > room {
            *loader_num_glyphs = room as FtInt;
        }
    } else if c == b'<' {
        /* We have `<< ... >>'.  Count the number of `/' in the dictionary */
        /* to get its size.                                                */
        let mut count: FtInt = 0;

        t1_skip_ps_token(parser);
        if parser.root.error != 0 {
            return Ok(());
        }
        t1_skip_spaces(parser);
        let cur = parser.root.cursor;

        while parser.root.cursor < limit {
            let c = at(&parser.base_dict, parser.root.cursor);
            if c == b'/' {
                count += 1;
            } else if c == b'>' {
                *loader_num_glyphs = count;
                parser.root.cursor = cur; /* rewind */
                break;
            }
            t1_skip_ps_token(parser);
            if parser.root.error != 0 {
                return Ok(());
            }
            t1_skip_spaces(parser);
        }
    } else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if parser.root.cursor >= limit {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* initialize tables */

    /* contrary to Type1, we disallow multiple CharStrings arrays */
    if swap_table.init != 0 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    ps_table_new(code_table, *loader_num_glyphs)?;

    ps_table_new(name_table, *loader_num_glyphs)?;

    /* Initialize table for swapping index notdef_index and */
    /* index 0 names and codes (if necessary).              */

    ps_table_new(swap_table, 4)?;

    let mut n: FtInt = 0;

    loop {
        /* We support two formats.                     */
        /*                                             */
        /*   `/glyphname' + index [+ `def']            */
        /*   `(glyphname)' [+ `cvn'] + index [+ `def'] */
        /*                                             */
        /* The latter format gets created by the       */
        /* LilyPond typesetting program.               */

        t1_skip_spaces(parser);

        let mut cur = parser.root.cursor;
        if cur >= limit {
            break;
        }

        let b = &parser.base_dict;

        /* We stop when we find an `end' keyword or '>' */
        if at(b, cur) == b'e'
            && cur + 3 < limit
            && at(b, cur + 1) == b'n'
            && at(b, cur + 2) == b'd'
            && t42_is_space(at(b, cur + 3))
        {
            break;
        }
        if at(b, cur) == b'>' {
            break;
        }

        t1_skip_ps_token(parser);
        if parser.root.cursor >= limit {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
        if parser.root.error != 0 {
            return Ok(());
        }

        let c = at(&parser.base_dict, cur);
        if c == b'/' || c == b'(' {
            let have_literal = c == b'(';

            if cur + if have_literal { 3 } else { 2 } >= limit {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            cur += 1; /* skip `/' */
            let mut len = parser.root.cursor - cur;
            if have_literal {
                len -= 1;
            }

            let b = &parser.base_dict;
            let name = b.get(cur..(cur + len + 1).min(b.len())).unwrap_or(&[]);
            ps_table_add(name_table, n, name, len as FtUInt + 1)?;

            /* add a trailing zero to the name table */
            if let Some(start) = name_table.elements[n as usize] {
                name_table.block[start + len] = b'\0';
            }

            /* record index of /.notdef */
            if at(b, cur) == b'.' && name_table.element(n as usize).map(c_str) == Some(b".notdef") {
                notdef_index = n;
                notdef_found = true;
            }

            t1_skip_spaces(parser);

            if have_literal {
                t1_skip_ps_token(parser);
            }

            cur = parser.root.cursor;

            let _ = t1_to_int(parser);
            if parser.root.cursor >= limit {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            let len = parser.root.cursor - cur;

            let b = &parser.base_dict;
            let code = b.get(cur..(cur + len + 1).min(b.len())).unwrap_or(&[]);
            ps_table_add(code_table, n, code, len as FtUInt + 1)?;

            if let Some(start) = code_table.elements[n as usize] {
                code_table.block[start + len] = b'\0';
            }

            n += 1;
            if n >= *loader_num_glyphs {
                break;
            }
        }
    }

    *loader_num_glyphs = n;

    if !notdef_found {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* if /.notdef does not occupy index 0, do our magic. */
    if name_table.element(0).map(c_str) != Some(b".notdef") {
        /* Swap glyph in index 0 with /.notdef glyph.  First, add index 0  */
        /* name and code entries to swap_table.  Then place notdef_index   */
        /* name and code entries into swap_table.  Then swap name and code */
        /* entries at indices notdef_index and 0 using values stored in    */
        /* swap_table.                                                     */

        /* Index 0 name */
        let (o, l) = elem(name_table, 0);
        ps_table_add(swap_table, 0, o, l)?;

        /* Index 0 code */
        let (o, l) = elem(code_table, 0);
        ps_table_add(swap_table, 1, o, l)?;

        /* Index notdef_index name */
        let (o, l) = elem(name_table, notdef_index as usize);
        ps_table_add(swap_table, 2, o, l)?;

        /* Index notdef_index code */
        let (o, l) = elem(code_table, notdef_index as usize);
        ps_table_add(swap_table, 3, o, l)?;

        let (o, l) = elem(swap_table, 0);
        ps_table_add(name_table, notdef_index, o, l)?;

        let (o, l) = elem(swap_table, 1);
        ps_table_add(code_table, notdef_index, o, l)?;

        let (o, l) = elem(swap_table, 2);
        ps_table_add(name_table, 0, o, l)?;

        let (o, l) = elem(swap_table, 3);
        ps_table_add(code_table, 0, o, l)?;
    }

    Ok(())
}

/// `t42_load_keyword`
fn t42_load_keyword(
    face: &mut T42FaceRec,
    loader: &mut T42LoaderRec,
    field: &T42Field,
) -> FtResult<()> {
    let max_objects: FtUInt = 0;

    /* if the keyword has a dedicated callback, call it */
    if field.type_ == T1_FIELD_TYPE_CALLBACK {
        if let Some(reader) = field.reader {
            reader(face, loader);
        }
        let error = loader.parser.root.error;
        return if error == 0 { Ok(()) } else { Err(error) };
    }

    /* now the keyword is either a simple field or a table of fields; */
    /* we are now going to take care of it                            */

    let type1 = &mut face.type1;
    let dummy_object = match field.location {
        T1_FIELD_LOCATION_FONT_INFO => T1Object::FontInfo(&mut type1.font_info),

        T1_FIELD_LOCATION_FONT_EXTRA => T1Object::FontExtra(&mut type1.font_extra),

        T1_FIELD_LOCATION_BBOX => T1Object::BBox(&mut type1.font_bbox),

        _ => T1Object::Font(type1),
    };

    let mut objects = [dummy_object];
    let parser = &mut loader.parser;

    if field.type_ == T1_FIELD_TYPE_INTEGER_ARRAY || field.type_ == T1_FIELD_TYPE_FIXED_ARRAY {
        ps_parser_load_field_table(
            &mut parser.root,
            &parser.base_dict,
            field,
            &mut objects,
            max_objects,
        )
    } else {
        ps_parser_load_field(
            &mut parser.root,
            &parser.base_dict,
            field,
            field.type_,
            0,
            &mut objects,
            max_objects,
        )
    }

    /* Exit: */
}

/// `t42_parse_dict` (the base dictionary, of `size` bytes)
pub fn t42_parse_dict(
    face: &mut T42FaceRec,
    loader: &mut T42LoaderRec,
    size: FtLong,
) -> FtResult<()> {
    loader.parser.root.cursor = 0;
    loader.parser.root.limit = size as usize;
    loader.parser.root.error = FT_ERR_OK;

    let limit = loader.parser.root.limit;

    t1_skip_spaces(&mut loader.parser);

    'exit: {
        while loader.parser.root.cursor < limit {
            let mut cur = loader.parser.root.cursor;
            let b = &loader.parser.base_dict;

            /* look for `FontDirectory' which causes problems for some fonts */
            if at(b, cur) == b'F'
                && cur + 25 < limit
                && b.get(cur..cur + 13) == Some(&b"FontDirectory"[..])
            {
                /* skip the `FontDirectory' keyword */
                t1_skip_ps_token(&mut loader.parser);
                t1_skip_spaces(&mut loader.parser);
                cur = loader.parser.root.cursor;
                let mut cur2 = cur;

                /* look up the `known' keyword */
                while cur < limit {
                    let b = &loader.parser.base_dict;
                    if at(b, cur) == b'k'
                        && cur + 5 < limit
                        && b.get(cur..cur + 5) == Some(&b"known"[..])
                    {
                        break;
                    }

                    t1_skip_ps_token(&mut loader.parser);
                    if loader.parser.root.error != 0 {
                        break 'exit;
                    }
                    t1_skip_spaces(&mut loader.parser);
                    cur = loader.parser.root.cursor;
                }

                if cur < limit {
                    let mut token = T1TokenRec::default();

                    /* skip the `known' keyword and the token following it */
                    t1_skip_ps_token(&mut loader.parser);
                    t1_to_token(&mut loader.parser, &mut token);

                    /* if the last token was an array, skip it! */
                    if token.type_ == T1_TOKEN_TYPE_ARRAY {
                        cur2 = loader.parser.root.cursor;
                    }
                }
                loader.parser.root.cursor = cur2;
            }
            /* look for immediates */
            else if at(b, cur) == b'/' && cur + 2 < limit {
                cur += 1;

                loader.parser.root.cursor = cur;
                t1_skip_ps_token(&mut loader.parser);
                if loader.parser.root.error != 0 {
                    break 'exit;
                }

                let len = loader.parser.root.cursor - cur;

                if len > 0 && len < 22 && loader.parser.root.cursor < limit {
                    /* now compare the immediate name to the keyword table */

                    /* loop through all known keywords */
                    for keyword in T42_KEYWORDS.iter() {
                        let name = keyword.ident.as_bytes();

                        if name.is_empty() {
                            continue;
                        }

                        let b = &loader.parser.base_dict;
                        if at(b, cur) == name[0]
                            && len == name.len()
                            && b.get(cur..cur + len) == Some(name)
                        {
                            /* we found it -- run the parsing callback! */
                            loader.parser.root.error =
                                error_code(t42_load_keyword(face, loader, keyword));
                            if loader.parser.root.error != 0 {
                                return Err(loader.parser.root.error);
                            }
                            break;
                        }
                    }
                }
            } else {
                t1_skip_ps_token(&mut loader.parser);
                if loader.parser.root.error != 0 {
                    break 'exit;
                }
            }

            t1_skip_spaces(&mut loader.parser);
        }
    }

    /* Exit: */
    let error = loader.parser.root.error;
    if error == 0 {
        Ok(())
    } else {
        Err(error)
    }
}

/// `t42_loader_init`
pub fn t42_loader_init(loader: &mut T42LoaderRec, _face: &T42FaceRec) {
    *loader = T42LoaderRec::default();
    loader.num_glyphs = 0;
    loader.num_chars = 0;

    /* initialize the tables -- simply set their `init' field to 0 */
    loader.encoding_table.init = 0;
    loader.charstrings.init = 0;
    loader.glyph_names.init = 0;
}

/// `t42_loader_done`
pub fn t42_loader_done(loader: &mut T42LoaderRec) {
    /* finalize tables */
    ps_table_release(&mut loader.encoding_table);
    ps_table_release(&mut loader.charstrings);
    ps_table_release(&mut loader.glyph_names);
    ps_table_release(&mut loader.swap_table);

    /* finalize parser */
    t42_parser_done(&mut loader.parser);
}
