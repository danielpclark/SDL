// Rust translation of src/psaux/psobjs.c and of the table, parser,
// builder and decoder records of include/freetype/internal/psaux.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auxiliary functions for PostScript fonts (body): the PostScript table
//! and parser, the Type 1, CFF and PS glyph builders, the PS decoder
//! wrapper, `t1_make_subfont`, `t1_decrypt` and `cff_random`.
//!
//! A builder holds the glyph slot it builds into (with the CFF driver's
//! slot additions, which the Type 1 and CID drivers' share); the slot's
//! glyph loader is C's `loader`, whose `base` and `current` outlines
//! `base` and `current` point to. The PS builder wraps a CFF builder
//! (the Type 1 builder's own), whose `pos_x`, `pos_y`, `left_bearing`,
//! `advance` and `bbox` C's PS builder points to. The face data the
//! builders and the Adobe engine read through `builder->face` (and the
//! face's driver and size) are copied into [`CffBuilderFace`].

use std::collections::HashMap;
use std::sync::Arc;

use super::super::base::ftcalc::{fixed_to_int, ft_round_fix};
use super::super::base::ftgloadr::FtGlyphLoaderRec;
use super::super::base::ftmemory::{ft_new_array, ft_qalloc, ft_renew_array};
use super::super::base::ftobjs::FtGlyphSlotRec;
use super::super::base::ftstream::FtStreamRec;
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::super::t1tables::*;
use super::super::t1types::*;
use super::cffdecode::CffDecoder;
use super::psconv::*;
use super::psfont::Cf2FontRec;
use super::t1decode::T1DecoderRec;
use super::PsDriverRec;

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                             PS_TABLE                          *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `PS_TableRec`: a `PS_Table` is a simple object used to store an array
/// of objects in a single memory block.
///
/// * `block`: the address in memory of the growheap's block (its length
///   is the block's `capacity`).
/// * `cursor`: the current top of the growheap within its block.
/// * `capacity`: the current size of the heap block.  Increments by
///   1kByte chunks.
/// * `init`: set to 0xDEADBEEF if `elements` and `lengths` have been
///   allocated.
/// * `max_elems`: the maximum number of elements in table.
/// * `elements`: an array of element addresses (offsets into `block`).
/// * `lengths`: an array of element sizes.
#[derive(Debug, Clone, Default)]
pub struct PsTableRec {
    pub block: Vec<u8>,
    pub cursor: usize,
    pub capacity: usize,
    pub init: FtULong,

    pub max_elems: FtInt,
    pub elements: Vec<Option<usize>>,
    pub lengths: Vec<FtUInt>,
}

impl PsTableRec {
    /// The element `idx` (`elements[idx]`, for `lengths[idx]` bytes).
    pub fn element(&self, idx: usize) -> Option<&[u8]> {
        let start = (*self.elements.get(idx)?)?;
        let len = *self.lengths.get(idx)? as usize;
        self.block.get(start..start + len)
    }

    /// The table's data, as the face keeps it once loaded.
    pub fn freeze(self) -> PsTableData {
        PsTableData {
            block: Arc::from(&self.block[..self.cursor.min(self.block.len())]),
            elements: Arc::from(self.elements),
            lengths: Arc::from(self.lengths),
        }
    }
}

/// The block, elements and lengths of a loaded `PS_Table` (the Type 1
/// font's subroutines, charstrings and glyph names), shared with the
/// decoders and charmaps that read them.
#[derive(Debug, Clone)]
pub struct PsTableData {
    pub block: Arc<[u8]>,
    pub elements: Arc<[Option<usize>]>,
    pub lengths: Arc<[FtUInt]>,
}

impl Default for PsTableData {
    fn default() -> Self {
        PsTableData {
            block: Arc::from(&[][..]),
            elements: Arc::from(&[][..]),
            lengths: Arc::from(&[][..]),
        }
    }
}

impl PsTableData {
    /// The element `idx` (`elements[idx]`, for `lengths[idx]` bytes).
    pub fn element(&self, idx: usize) -> Option<&[u8]> {
        let start = (*self.elements.get(idx)?)?;
        let len = *self.lengths.get(idx)? as usize;
        self.block.get(start..start + len)
    }

    /// The element `idx` as a C string (up to its first null byte).
    pub fn name(&self, idx: usize) -> Option<&[u8]> {
        let start = (*self.elements.get(idx)?)?;
        let s = self.block.get(start..)?;
        let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Some(&s[..len])
    }

    /// `lengths[idx]`
    pub fn length(&self, idx: usize) -> FtUInt {
        self.lengths.get(idx).copied().unwrap_or(0)
    }
}

/// `ps_table_new`: initializes a `PS_Table`.
///
/// * `count`: the table size = the maximum number of elements.
pub fn ps_table_new(table: &mut PsTableRec, count: FtInt) -> FtResult<()> {
    let r = (|| {
        table.elements = ft_new_array(count as FtLong)?;
        table.lengths = ft_new_array(count as FtLong)?;
        Ok(())
    })();

    if r.is_ok() {
        table.max_elems = count;
        table.init = 0xDEADBEEF;
        table.block = Vec::new();
        table.capacity = 0;
        table.cursor = 0;
    }

    /* Exit: */
    if r.is_err() {
        table.elements = Vec::new();
    }

    r
}

/// `ps_table_realloc`
fn ps_table_realloc(table: &mut PsTableRec, new_size: usize) -> FtResult<()> {
    /* (re)allocate the base block */
    ft_renew_array(&mut table.block, new_size as FtLong)?;

    /* (the elements are offsets into the block, which need no rebasing) */

    table.capacity = new_size;
    Ok(())
}

/// `ps_table_add`: adds an object to a `PS_Table`, possibly growing its
/// memory block.
///
/// * `idx`: the index of the object in the table.
/// * `object`: the object to copy in memory (its first `length` bytes;
///   C's object can be in the table's own block, which here is copied
///   first by the caller).
/// * `length`: the length in bytes of the source object.
pub fn ps_table_add(
    table: &mut PsTableRec,
    idx: FtInt,
    object: &[u8],
    length: FtUInt,
) -> FtResult<()> {
    if idx < 0 || idx >= table.max_elems {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let length = length as usize;

    /* grow the base block if needed */
    if table.cursor + length > table.capacity {
        let mut new_size: usize = table.capacity;

        while new_size < table.cursor + length {
            /* increase size by 25% and round up to the nearest multiple
            of 1024 */
            new_size += (new_size >> 2) + 1;
            new_size = ft_pad_ceil(new_size as i64, 1024) as usize;
        }

        ps_table_realloc(table, new_size)?;
    }

    /* add the object to the base block and adjust offset */
    table.elements[idx as usize] = Some(table.cursor);
    table.lengths[idx as usize] = length as FtUInt;

    let dest = &mut table.block[table.cursor..table.cursor + length];
    let n = length.min(object.len());
    dest[..n].copy_from_slice(&object[..n]);
    dest[n..].fill(0);

    table.cursor += length;

    Ok(())
}

/// `ps_table_done`: finalizes a `PS_TableRec` (i.e., reallocate it to its
/// current cursor).
pub fn ps_table_done(table: &mut PsTableRec) {
    /* no problem if shrinking fails */
    let cursor = table.cursor;
    let _ = ps_table_realloc(table, cursor);
}

/// `ps_table_release`
pub fn ps_table_release(table: &mut PsTableRec) {
    if table.init == 0xDEADBEEF {
        table.block = Vec::new();
        table.elements = Vec::new();
        table.lengths = Vec::new();
        table.init = 0;
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                            T1 PARSER                          *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `PS_ParserRec`: a `PS_Parser` is an object used to parse a Type 1
/// font very quickly.
///
/// * `cursor`: the current position in the text.
/// * `base`: start of the processed text.
/// * `limit`: end of the processed text.
/// * `error`: the last error returned.
///
/// The positions are offsets into the parsed bytes, which the parsing
/// functions take (`b`).
#[derive(Debug, Clone, Copy, Default)]
pub struct PsParserRec {
    pub cursor: usize,
    pub base: usize,
    pub limit: usize,
    pub error: FtError,
}

/// `T1_TokenType`: simple enumeration type used to identify token types
pub type T1TokenType = FtUInt;

pub const T1_TOKEN_TYPE_NONE: T1TokenType = 0;
pub const T1_TOKEN_TYPE_ANY: T1TokenType = 1;
pub const T1_TOKEN_TYPE_STRING: T1TokenType = 2;
pub const T1_TOKEN_TYPE_ARRAY: T1TokenType = 3;
pub const T1_TOKEN_TYPE_KEY: T1TokenType = 4; /* aka `name' */

/* do not remove */
pub const T1_TOKEN_TYPE_MAX: T1TokenType = 5;

/// `T1_TokenRec`: a simple structure used to identify tokens
///
/// * `start`: first character of token in input stream (`None` for
///   NULL).
/// * `limit`: first character after the token.
/// * `type_`: type of token.
#[derive(Debug, Clone, Copy, Default)]
pub struct T1TokenRec {
    pub start: Option<usize>,
    pub limit: Option<usize>,
    pub type_: T1TokenType,
}

impl T1TokenRec {
    /// `start` (0 for NULL, never dereferenced then).
    pub fn start(&self) -> usize {
        self.start.unwrap_or(0)
    }

    /// `limit` (0 for NULL, never dereferenced then).
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(0)
    }
}

/// `T1_FieldType`: enumeration type used to identify object fields
pub type T1FieldType = FtUInt;

pub const T1_FIELD_TYPE_NONE: T1FieldType = 0;
pub const T1_FIELD_TYPE_BOOL: T1FieldType = 1;
pub const T1_FIELD_TYPE_INTEGER: T1FieldType = 2;
pub const T1_FIELD_TYPE_FIXED: T1FieldType = 3;
pub const T1_FIELD_TYPE_FIXED_1000: T1FieldType = 4;
pub const T1_FIELD_TYPE_STRING: T1FieldType = 5;
pub const T1_FIELD_TYPE_KEY: T1FieldType = 6;
pub const T1_FIELD_TYPE_BBOX: T1FieldType = 7;
pub const T1_FIELD_TYPE_MM_BBOX: T1FieldType = 8;
pub const T1_FIELD_TYPE_INTEGER_ARRAY: T1FieldType = 9;
pub const T1_FIELD_TYPE_FIXED_ARRAY: T1FieldType = 10;
pub const T1_FIELD_TYPE_CALLBACK: T1FieldType = 11;

/* do not remove */
pub const T1_FIELD_TYPE_MAX: T1FieldType = 12;

/// `T1_FieldLocation`
pub type T1FieldLocation = FtUInt;

pub const T1_FIELD_LOCATION_CID_INFO: T1FieldLocation = 0;
pub const T1_FIELD_LOCATION_FONT_DICT: T1FieldLocation = 1;
pub const T1_FIELD_LOCATION_FONT_EXTRA: T1FieldLocation = 2;
pub const T1_FIELD_LOCATION_FONT_INFO: T1FieldLocation = 3;
pub const T1_FIELD_LOCATION_PRIVATE: T1FieldLocation = 4;
pub const T1_FIELD_LOCATION_BBOX: T1FieldLocation = 5;
pub const T1_FIELD_LOCATION_LOADER: T1FieldLocation = 6;
pub const T1_FIELD_LOCATION_FACE: T1FieldLocation = 7;
pub const T1_FIELD_LOCATION_BLEND: T1FieldLocation = 8;

/* do not remove */
pub const T1_FIELD_LOCATION_MAX: T1FieldLocation = 9;

/// The records a field can be in (C's `objects`, which point to them):
/// the field's location's records.
#[derive(Debug)]
pub enum T1Object<'a> {
    FontInfo(&'a mut PsFontInfoRec),
    FontExtra(&'a mut PsFontExtraRec),
    Private(&'a mut PsPrivateRec),
    BBox(&'a mut FtBBox),
    /// a Type 1 font's (or Type 42 font's) font dictionary
    Font(&'a mut T1FontRec),
    /// the face fields of `T1_FIELD_LOCATION_FACE` (`ndv_idx`, `cdv_idx`)
    T1Face(&'a mut T1FaceIndicesRec),
    Blend(&'a mut PsBlendRec),
    CidInfo(&'a mut CidFaceInfoRec),
    CidDict(&'a mut CidFaceDictRec),
}

/// A field's element count (C's `count_offset`, written as one byte).
#[derive(Debug)]
pub enum T1FieldCount<'a> {
    None,
    Byte(&'a mut FtByte),
    Int(&'a mut FtInt),
    UInt(&'a mut FtUInt),
}

impl T1FieldCount<'_> {
    /// `*(FT_Byte*)( objects[0] + count_offset ) = (FT_Byte)n` (the low
    /// byte of a wider count).
    fn set(&mut self, n: FtInt) {
        let b = n as FtByte;
        match self {
            T1FieldCount::None => {}
            T1FieldCount::Byte(c) => **c = b,
            T1FieldCount::Int(c) => **c = (**c & !0xFF) | b as FtInt,
            T1FieldCount::UInt(c) => **c = (**c & !0xFF) | b as FtUInt,
        }
    }
}

/// The storage of a field in a record (C's `objects[idx] + offset`, of
/// `size` bytes, or the array of a table field and its count).
#[derive(Debug)]
pub enum T1FieldRef<'a> {
    None,
    Byte(&'a mut FtByte),
    Bool(&'a mut FtBool),
    Short(&'a mut FtShort),
    UShort(&'a mut FtUShort),
    Int(&'a mut FtInt),
    UInt(&'a mut FtUInt),
    /// an `FT_Long`, `FT_Fixed` or `FT_Pos`
    Long(&'a mut FtLong),
    ULong(&'a mut FtULong),
    String(&'a mut Option<Vec<u8>>),
    BBox(&'a mut FtBBox),
    Shorts(&'a mut [FtShort], T1FieldCount<'a>),
    UShorts(&'a mut [FtUShort], T1FieldCount<'a>),
    UInts(&'a mut [FtUInt], T1FieldCount<'a>),
    ULongs(&'a mut [FtULong], T1FieldCount<'a>),
}

/// A field's accessor: its storage in a record of the field's location
/// (`T1FieldRef::None` for another record).
pub type T1FieldAccess = for<'a, 'b> fn(&'a mut T1Object<'b>) -> T1FieldRef<'a>;

/// `T1_FieldRec`: structure type used to model object fields (`R` is the
/// type of the driver's parsing callbacks).
///
/// * `ident`: field identifier.
/// * `location`: where the field is.
/// * `type_`: type of field.
/// * `reader`: the callback of a `T1_FIELD_TYPE_CALLBACK` field.
/// * `access`: the field's storage (C's `offset` and `size`, and
///   `count_offset`).
/// * `array_max`: maximum number of elements for array.
/// * `dict`: where we expect it.
#[derive(Debug)]
pub struct T1FieldRec<R: 'static> {
    pub ident: &'static str,
    pub location: T1FieldLocation,
    pub type_: T1FieldType,
    pub reader: Option<R>,
    pub access: Option<T1FieldAccess>,
    pub array_max: FtUInt,
    pub dict: FtUInt,
}

pub const T1_FIELD_DICT_FONTDICT: FtUInt = 1 << 0; /* also FontInfo and FDArray */
pub const T1_FIELD_DICT_PRIVATE: FtUInt = 1 << 1;

/// A `T1_FieldRec` of a value (`T1_FIELD_BOOL`, `T1_FIELD_NUM`, ...;
/// `T1_NEW_SIMPLE_FIELD` and `T1_NEW_TABLE_FIELD`): the identifier,
/// location, type, record, the record's field, its storage variant, the
/// array maximum and the dictionaries.
#[macro_export]
macro_rules! t1_field {
    ($ident:expr, $loc:expr, $ty:expr, $obj:ident, $field:ident, $var:ident, $max:expr, $dict:expr) => {
        $crate::freetype::psaux::psobjs::T1FieldRec {
            ident: $ident,
            location: $loc,
            type_: $ty,
            reader: None,
            access: Some({
                fn access<'a, 'b>(
                    o: &'a mut $crate::freetype::psaux::psobjs::T1Object<'b>,
                ) -> $crate::freetype::psaux::psobjs::T1FieldRef<'a> {
                    match o {
                        $crate::freetype::psaux::psobjs::T1Object::$obj(r) => {
                            $crate::freetype::psaux::psobjs::T1FieldRef::$var(&mut r.$field)
                        }
                        _ => $crate::freetype::psaux::psobjs::T1FieldRef::None,
                    }
                }
                access
            }),
            array_max: $max,
            dict: $dict,
        }
    };
}

/// A `T1_FieldRec` of a table with its count (`T1_NEW_TABLE_FIELD`): the
/// identifier, location, type, record, the record's array, its storage
/// variant, the count field and its variant, the array maximum and the
/// dictionaries.
#[macro_export]
macro_rules! t1_table_field {
    ($ident:expr, $loc:expr, $ty:expr, $obj:ident, $field:ident, $var:ident,
     $count:ident, $cvar:ident, $max:expr, $dict:expr) => {
        $crate::freetype::psaux::psobjs::T1FieldRec {
            ident: $ident,
            location: $loc,
            type_: $ty,
            reader: None,
            access: Some({
                fn access<'a, 'b>(
                    o: &'a mut $crate::freetype::psaux::psobjs::T1Object<'b>,
                ) -> $crate::freetype::psaux::psobjs::T1FieldRef<'a> {
                    match o {
                        $crate::freetype::psaux::psobjs::T1Object::$obj(r) => {
                            let r = &mut **r;
                            $crate::freetype::psaux::psobjs::T1FieldRef::$var(
                                &mut r.$field[..],
                                $crate::freetype::psaux::psobjs::T1FieldCount::$cvar(&mut r.$count),
                            )
                        }
                        _ => $crate::freetype::psaux::psobjs::T1FieldRef::None,
                    }
                }
                access
            }),
            array_max: $max,
            dict: $dict,
        }
    };
}

/// A `T1_FieldRec` of a table without a count (`T1_NEW_TABLE_FIELD2`):
/// the identifier, location, type, record, the record's array, its
/// storage variant, the array maximum and the dictionaries.
#[macro_export]
macro_rules! t1_table_field2 {
    ($ident:expr, $loc:expr, $ty:expr, $obj:ident, $field:ident, $var:ident, $max:expr, $dict:expr) => {
        $crate::freetype::psaux::psobjs::T1FieldRec {
            ident: $ident,
            location: $loc,
            type_: $ty,
            reader: None,
            access: Some({
                fn access<'a, 'b>(
                    o: &'a mut $crate::freetype::psaux::psobjs::T1Object<'b>,
                ) -> $crate::freetype::psaux::psobjs::T1FieldRef<'a> {
                    match o {
                        $crate::freetype::psaux::psobjs::T1Object::$obj(r) => {
                            $crate::freetype::psaux::psobjs::T1FieldRef::$var(
                                &mut r.$field[..],
                                $crate::freetype::psaux::psobjs::T1FieldCount::None,
                            )
                        }
                        _ => $crate::freetype::psaux::psobjs::T1FieldRef::None,
                    }
                }
                access
            }),
            array_max: $max,
            dict: $dict,
        }
    };
}

/// A `T1_FieldRec` of a callback (`T1_NEW_CALLBACK_FIELD`).
#[macro_export]
macro_rules! t1_callback_field {
    ($ident:expr, $reader:expr, $dict:expr) => {
        $crate::freetype::psaux::psobjs::T1FieldRec {
            ident: $ident,
            location: $crate::freetype::psaux::psobjs::T1_FIELD_LOCATION_CID_INFO,
            type_: $crate::freetype::psaux::psobjs::T1_FIELD_TYPE_CALLBACK,
            reader: Some($reader),
            access: None,
            array_max: 0,
            dict: $dict,
        }
    };
}

/// The bytes of `b` from `cur` to `limit` (empty if out of range).
#[inline]
fn bytes(b: &[u8], cur: usize, limit: usize) -> &[u8] {
    b.get(cur..limit.min(b.len())).unwrap_or(&[])
}

/// `skip_comment`: first character must be already part of the comment
fn skip_comment(b: &[u8], acur: &mut usize, limit: usize) {
    let mut cur = *acur;

    while cur < limit {
        if is_ps_newline(at(b, cur)) {
            break;
        }
        cur += 1;
    }

    *acur = cur;
}

/// `skip_spaces`
fn skip_spaces(b: &[u8], acur: &mut usize, limit: usize) {
    let mut cur = *acur;

    while cur < limit {
        if !is_ps_space(at(b, cur)) {
            if at(b, cur) == b'%' {
                /* According to the PLRM, a comment is equal to a space. */
                skip_comment(b, &mut cur, limit);
            } else {
                break;
            }
        }
        cur += 1;
    }

    *acur = cur;
}

#[inline]
fn is_octal_digit(c: u8) -> bool {
    (b'0'..=b'7').contains(&c)
}

/// `skip_literal_string`: first character must be `(';  `*acur' is
/// positioned at the character after the closing `)'
fn skip_literal_string(b: &[u8], acur: &mut usize, limit: usize) -> FtResult<()> {
    let mut cur = *acur;
    let mut embed: FtInt = 0;
    let mut error = Err(FT_ERR_INVALID_FILE_FORMAT);

    while cur < limit {
        let c = at(b, cur);

        cur += 1;

        if c == b'\\' {
            /* Red Book 3rd ed., section `Literal Text Strings', p. 29:     */
            /* A backslash can introduce three different types              */
            /* of escape sequences:                                         */
            /*   - a special escaped char like \r, \n, etc.                 */
            /*   - a one-, two-, or three-digit octal number                */
            /*   - none of the above in which case the backslash is ignored */

            if cur == limit {
                /* error (or to be ignored?) */
                break;
            }

            match at(b, cur) {
                /* skip `special' escape */
                b'n' | b'r' | b't' | b'b' | b'f' | b'\\' | b'(' | b')' => {
                    cur += 1;
                }

                _ => {
                    /* skip octal escape or ignore backslash */
                    let mut i = 0;
                    while i < 3 && cur < limit {
                        if !is_octal_digit(at(b, cur)) {
                            break;
                        }

                        cur += 1;
                        i += 1;
                    }
                }
            }
        } else if c == b'(' {
            embed += 1;
        } else if c == b')' {
            embed -= 1;
            if embed == 0 {
                error = Ok(());
                break;
            }
        }
    }

    *acur = cur;

    error
}

/// `skip_string`: first character must be `<'
fn skip_string(b: &[u8], acur: &mut usize, limit: usize) -> FtResult<()> {
    let mut cur = *acur;
    let mut err = Ok(());

    loop {
        cur += 1;
        if cur >= limit {
            break;
        }

        /* All whitespace characters are ignored. */
        skip_spaces(b, &mut cur, limit);
        if cur >= limit {
            break;
        }

        if !is_ps_xdigit(at(b, cur)) {
            break;
        }
    }

    if cur < limit && at(b, cur) != b'>' {
        err = Err(FT_ERR_INVALID_FILE_FORMAT);
    } else {
        cur += 1;
    }

    *acur = cur;
    err
}

/// `skip_procedure`: first character must be the opening brace that
/// starts the procedure
///
/// NB: [ and ] need not match: `/foo {[} def' is a valid PostScript
/// fragment, even within a Type1 font
fn skip_procedure(b: &[u8], acur: &mut usize, limit: usize) -> FtResult<()> {
    let mut cur: usize;
    let mut embed: FtInt = 0;
    let mut error: FtResult<()> = Ok(());

    cur = *acur;
    'end: {
        while cur < limit && error.is_ok() {
            match at(b, cur) {
                b'{' => {
                    embed += 1;
                }

                b'}' => {
                    embed -= 1;
                    if embed == 0 {
                        cur += 1;
                        break 'end;
                    }
                }

                b'(' => {
                    error = skip_literal_string(b, &mut cur, limit);
                }

                b'<' => {
                    error = skip_string(b, &mut cur, limit);
                }

                b'%' => {
                    skip_comment(b, &mut cur, limit);
                }

                _ => {}
            }
            cur += 1;
        }
    }

    /* end: */
    if embed != 0 {
        error = Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    *acur = cur;

    error
}

/***********************************************************************
 *
 * All exported parsing routines handle leading whitespace and stop at
 * the first character which isn't part of the just handled token.
 *
 */

/// `ps_parser_skip_PS_token`
pub fn ps_parser_skip_ps_token(parser: &mut PsParserRec, b: &[u8]) {
    /* Note: PostScript allows any non-delimiting, non-whitespace        */
    /*       character in a name (PS Ref Manual, 3rd ed, p31).           */
    /*       PostScript delimiters are (, ), <, >, [, ], {, }, /, and %. */

    let mut cur = parser.cursor;
    let limit = parser.limit;
    let mut error: FtResult<()> = Ok(());

    skip_spaces(b, &mut cur, limit); /* this also skips comments */

    'exit: {
        if cur >= limit {
            break 'exit;
        }

        /* self-delimiting, single-character tokens */
        if at(b, cur) == b'[' || at(b, cur) == b']' {
            cur += 1;
            break 'exit;
        }

        /* skip balanced expressions (procedures and strings) */

        if at(b, cur) == b'{' {
            /* {...} */
            error = skip_procedure(b, &mut cur, limit);
            break 'exit;
        }

        if at(b, cur) == b'(' {
            /* (...) */
            error = skip_literal_string(b, &mut cur, limit);
            break 'exit;
        }

        if at(b, cur) == b'<' {
            /* <...> */
            if cur + 1 < limit && at(b, cur + 1) == b'<' {
                /* << */
                cur += 1;
                cur += 1;
            } else {
                error = skip_string(b, &mut cur, limit);
            }

            break 'exit;
        }

        if at(b, cur) == b'>' {
            cur += 1;
            if cur >= limit || at(b, cur) != b'>' {
                /* >> */
                error = Err(FT_ERR_INVALID_FILE_FORMAT);
                break 'exit;
            }
            cur += 1;
            break 'exit;
        }

        if at(b, cur) == b'/' {
            cur += 1;
        }

        /* anything else */
        while cur < limit {
            /* *cur might be invalid (e.g., ')' or '}'), but this   */
            /* is handled by the test `cur == parser->cursor' below */
            if is_ps_delim(at(b, cur)) {
                break;
            }

            cur += 1;
        }
    }

    /* Exit: */
    if cur < limit && cur == parser.cursor {
        error = Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if cur > limit {
        cur = limit;
    }

    parser.error = match error {
        Ok(()) => FT_ERR_OK,
        Err(e) => e,
    };
    parser.cursor = cur;
}

/// `ps_parser_skip_spaces`
pub fn ps_parser_skip_spaces(parser: &mut PsParserRec, b: &[u8]) {
    skip_spaces(b, &mut parser.cursor, parser.limit);
}

/// `ps_parser_to_token`: `token' here means either something between
/// balanced delimiters or the next token; the delimiters are not
/// removed.
pub fn ps_parser_to_token(parser: &mut PsParserRec, b: &[u8], token: &mut T1TokenRec) {
    let mut cur: usize;
    let limit: usize;
    let mut embed: FtInt;

    token.type_ = T1_TOKEN_TYPE_NONE;
    token.start = None;
    token.limit = None;

    /* first of all, skip leading whitespace */
    ps_parser_skip_spaces(parser, b);

    cur = parser.cursor;
    limit = parser.limit;

    if cur >= limit {
        return;
    }

    match at(b, cur) {
        /************* check for literal string *****************/
        b'(' => {
            token.type_ = T1_TOKEN_TYPE_STRING;
            token.start = Some(cur);

            if skip_literal_string(b, &mut cur, limit).is_ok() {
                token.limit = Some(cur);
            }
        }

        /************* check for programs/array *****************/
        b'{' => {
            token.type_ = T1_TOKEN_TYPE_ARRAY;
            token.start = Some(cur);

            if skip_procedure(b, &mut cur, limit).is_ok() {
                token.limit = Some(cur);
            }
        }

        /************* check for table/array ********************/
        /* XXX: in theory we should also look for "<<"          */
        /*      since this is semantically equivalent to "[";   */
        /*      in practice it doesn't matter (?)               */
        b'[' => {
            token.type_ = T1_TOKEN_TYPE_ARRAY;
            embed = 1;
            token.start = Some(cur);
            cur += 1;

            /* we need this to catch `[ ]' */
            parser.cursor = cur;
            ps_parser_skip_spaces(parser, b);
            cur = parser.cursor;

            while cur < limit && parser.error == 0 {
                /* XXX: this is wrong because it does not      */
                /*      skip comments, procedures, and strings */
                if at(b, cur) == b'[' {
                    embed += 1;
                } else if at(b, cur) == b']' {
                    embed -= 1;
                    if embed <= 0 {
                        cur += 1;
                        token.limit = Some(cur);
                        break;
                    }
                }

                parser.cursor = cur;
                ps_parser_skip_ps_token(parser, b);
                /* we need this to catch `[XXX ]' */
                ps_parser_skip_spaces(parser, b);
                cur = parser.cursor;
            }
        }

        /* ************ otherwise, it is any token **************/
        c => {
            token.start = Some(cur);
            token.type_ = if c == b'/' {
                T1_TOKEN_TYPE_KEY
            } else {
                T1_TOKEN_TYPE_ANY
            };
            ps_parser_skip_ps_token(parser, b);
            cur = parser.cursor;
            if parser.error == 0 {
                token.limit = Some(cur);
            }
        }
    }

    if token.limit.is_none() {
        token.start = None;
        token.type_ = T1_TOKEN_TYPE_NONE;
    }

    parser.cursor = cur;
}

/// `ps_parser_to_token_array`
///
/// NB: `tokens' can be NULL if we only want to count the number of array
/// elements
pub fn ps_parser_to_token_array(
    parser: &mut PsParserRec,
    b: &[u8],
    mut tokens: Option<&mut [T1TokenRec]>,
    max_tokens: FtUInt,
    pnum_tokens: &mut FtInt,
) {
    let mut master = T1TokenRec::default();

    *pnum_tokens = -1;

    /* this also handles leading whitespace */
    ps_parser_to_token(parser, b, &mut master);

    if master.type_ == T1_TOKEN_TYPE_ARRAY {
        let old_cursor = parser.cursor;
        let old_limit = parser.limit;
        let mut cur: usize = 0;
        let limit: usize = max_tokens as usize;

        /* don't include outermost delimiters */
        parser.cursor = master.start() + 1;
        parser.limit = master.limit() - 1;

        while parser.cursor < parser.limit {
            let mut token = T1TokenRec::default();

            ps_parser_to_token(parser, b, &mut token);
            if token.type_ == T1_TOKEN_TYPE_NONE {
                break;
            }

            if let Some(tokens) = tokens.as_deref_mut() {
                if cur < limit {
                    if let Some(t) = tokens.get_mut(cur) {
                        *t = token;
                    }
                }
            }

            cur += 1;
        }

        *pnum_tokens = cur as FtInt;

        parser.cursor = old_cursor;
        parser.limit = old_limit;
    }
}

/// `ps_tocoordarray`: first character must be a delimiter or a part of a
/// number
///
/// NB: `coords' can be NULL if we just want to skip the array; in this
/// case we ignore `max_coords'
fn ps_tocoordarray(
    b: &[u8],
    acur: &mut usize,
    limit: usize,
    max_coords: FtInt,
    mut coords: Option<&mut [FtShort]>,
) -> FtInt {
    let mut cur = *acur;
    let mut count: FtInt = 0;

    'exit: {
        if cur >= limit {
            break 'exit;
        }

        /* check for the beginning of an array; otherwise, only one number */
        /* will be read                                                    */
        let c = at(b, cur);
        let mut ender: u8 = 0;

        if c == b'[' {
            ender = b']';
        } else if c == b'{' {
            ender = b'}';
        }

        if ender != 0 {
            cur += 1;
        }

        /* now, read the coordinates */
        while cur < limit {
            /* skip whitespace in front of data */
            skip_spaces(b, &mut cur, limit);
            if cur >= limit {
                break 'exit;
            }

            if at(b, cur) == ender {
                cur += 1;
                break;
            }

            let old_cur = cur;

            if coords.is_some() && count >= max_coords {
                break;
            }

            /* call PS_Conv_ToFixed() even if coords == NULL */
            /* to properly parse number at `cur'             */
            let v = (ps_conv_to_fixed(b, &mut cur, limit, 0) >> 16) as FtShort;
            if let Some(coords) = coords.as_deref_mut() {
                if let Some(c) = coords.get_mut(count as usize) {
                    *c = v;
                }
            }

            if old_cur == cur {
                count = -1;
                break 'exit;
            } else {
                count += 1;
            }

            if ender == 0 {
                break;
            }
        }
    }

    /* Exit: */
    *acur = cur;
    count
}

/// `ps_tofixedarray`: first character must be a delimiter or a part of a
/// number
///
/// NB: `values' can be NULL if we just want to skip the array; in this
/// case we ignore `max_values'
///
/// return number of successfully parsed values
fn ps_tofixedarray(
    b: &[u8],
    acur: &mut usize,
    limit: usize,
    max_values: FtInt,
    mut values: Option<&mut [FtFixed]>,
    power_ten: FtInt,
) -> FtInt {
    let mut cur = *acur;
    let mut count: FtInt = 0;

    'exit: {
        if cur >= limit {
            break 'exit;
        }

        /* Check for the beginning of an array.  Otherwise, only one number */
        /* will be read.                                                    */
        let c = at(b, cur);
        let mut ender: u8 = 0;

        if c == b'[' {
            ender = b']';
        } else if c == b'{' {
            ender = b'}';
        }

        if ender != 0 {
            cur += 1;
        }

        /* now, read the values */
        while cur < limit {
            /* skip whitespace in front of data */
            skip_spaces(b, &mut cur, limit);
            if cur >= limit {
                break 'exit;
            }

            if at(b, cur) == ender {
                cur += 1;
                break;
            }

            let old_cur = cur;

            if values.is_some() && count >= max_values {
                break;
            }

            /* call PS_Conv_ToFixed() even if coords == NULL */
            /* to properly parse number at `cur'             */
            let v = ps_conv_to_fixed(b, &mut cur, limit, power_ten as FtLong);
            if let Some(values) = values.as_deref_mut() {
                if let Some(c) = values.get_mut(count as usize) {
                    *c = v;
                }
            }

            if old_cur == cur {
                count = -1;
                break 'exit;
            } else {
                count += 1;
            }

            if ender == 0 {
                break;
            }
        }
    }

    /* Exit: */
    *acur = cur;
    count
}

/* (the `#if 0'ed ps_tostring) */

/// `ps_tobool`
fn ps_tobool(b: &[u8], acur: &mut usize, limit: usize) -> FtInt {
    let mut cur = *acur;
    let mut result: FtBool = false;

    /* return 1 if we find `true', 0 otherwise */
    if cur + 3 < limit
        && at(b, cur) == b't'
        && at(b, cur + 1) == b'r'
        && at(b, cur + 2) == b'u'
        && at(b, cur + 3) == b'e'
    {
        result = true;
        cur += 5;
    } else if cur + 4 < limit
        && at(b, cur) == b'f'
        && at(b, cur + 1) == b'a'
        && at(b, cur + 2) == b'l'
        && at(b, cur + 3) == b's'
        && at(b, cur + 4) == b'e'
    {
        result = false;
        cur += 6;
    }

    *acur = cur;
    result as FtInt
}

/// `Store_Integer` of `ps_parser_load_field`: stores `val` in the field
/// as C does by the field's size (`element` is the element of an array
/// field).
fn store_integer(field: &mut T1FieldRef<'_>, element: usize, val: FtLong) {
    match field {
        T1FieldRef::None | T1FieldRef::String(_) => {}
        T1FieldRef::Byte(q) => **q = val as FtByte,
        T1FieldRef::Bool(q) => **q = val as FtByte != 0,
        T1FieldRef::Short(q) => **q = val as FtUShort as FtShort,
        T1FieldRef::UShort(q) => **q = val as FtUShort,
        T1FieldRef::Int(q) => **q = val as FtUInt32 as FtInt,
        T1FieldRef::UInt(q) => **q = val as FtUInt32,
        T1FieldRef::Long(q) => **q = val,
        T1FieldRef::ULong(q) => **q = val as FtULong,
        T1FieldRef::BBox(q) => {
            /* (a bounding box as a table of four values) */
            match element {
                0 => q.xMin = val,
                1 => q.yMin = val,
                2 => q.xMax = val,
                3 => q.yMax = val,
                _ => {}
            }
        }
        T1FieldRef::Shorts(a, _) => {
            if let Some(q) = a.get_mut(element) {
                *q = val as FtUShort as FtShort;
            }
        }
        T1FieldRef::UShorts(a, _) => {
            if let Some(q) = a.get_mut(element) {
                *q = val as FtUShort;
            }
        }
        T1FieldRef::UInts(a, _) => {
            if let Some(q) = a.get_mut(element) {
                *q = val as FtUInt32;
            }
        }
        T1FieldRef::ULongs(a, _) => {
            if let Some(q) = a.get_mut(element) {
                *q = val as FtULong;
            }
        }
    }
}

/// `ps_parser_load_field`: load a simple field (i.e. non-table) into the
/// current list of objects (`element` is C's offset of the table element
/// `ps_parser_load_field_table` loads, 0 otherwise)
pub fn ps_parser_load_field<R>(
    parser: &mut PsParserRec,
    b: &[u8],
    field: &T1FieldRec<R>,
    field_type: T1FieldType,
    element: usize,
    objects: &mut [T1Object<'_>],
    max_objects: FtUInt,
) -> FtResult<()> {
    let mut token = T1TokenRec::default();
    let mut cur: usize;
    let mut limit: usize;
    let mut count: FtUInt;
    let mut idx: FtUInt;
    let mut type_: T1FieldType;

    'fail: {
        /* this also skips leading whitespace */
        ps_parser_to_token(parser, b, &mut token);
        if token.type_ == T1_TOKEN_TYPE_NONE {
            break 'fail;
        }

        count = 1;
        idx = 0;
        cur = token.start();
        limit = token.limit();

        type_ = field_type;

        /* we must detect arrays in /FontBBox */
        let mut field_array = false;
        if type_ == T1_FIELD_TYPE_BBOX {
            let mut token2 = T1TokenRec::default();
            let old_cur = parser.cursor;
            let old_limit = parser.limit;

            /* don't include delimiters */
            parser.cursor = token.start() + 1;
            parser.limit = token.limit() - 1;

            ps_parser_to_token(parser, b, &mut token2);
            parser.cursor = old_cur;
            parser.limit = old_limit;

            if token2.type_ == T1_TOKEN_TYPE_ARRAY {
                type_ = T1_FIELD_TYPE_MM_BBOX;
                field_array = true;
            }
        } else if token.type_ == T1_TOKEN_TYPE_ARRAY {
            count = max_objects;
            field_array = true;
        }

        if field_array {
            /* FieldArray: */
            /* if this is an array and we have no blend, an error occurs */
            if max_objects == 0 {
                break 'fail;
            }

            idx = 1;

            /* don't include delimiters */
            cur += 1;
            limit -= 1;
        }

        while count > 0 {
            let val: FtLong;

            skip_spaces(b, &mut cur, limit);

            let Some(object) = objects.get_mut(idx as usize) else {
                count -= 1;
                idx += 1;
                continue;
            };

            match type_ {
                T1_FIELD_TYPE_BOOL
                | T1_FIELD_TYPE_FIXED
                | T1_FIELD_TYPE_FIXED_1000
                | T1_FIELD_TYPE_INTEGER => {
                    val = match type_ {
                        T1_FIELD_TYPE_BOOL => ps_tobool(b, &mut cur, limit) as FtLong,
                        T1_FIELD_TYPE_FIXED => ps_conv_to_fixed(b, &mut cur, limit, 0),
                        T1_FIELD_TYPE_FIXED_1000 => ps_conv_to_fixed(b, &mut cur, limit, 3),
                        _ => ps_conv_to_int(b, &mut cur, limit),
                    };

                    /* Store_Integer: */
                    if let Some(access) = field.access {
                        let mut q = access(object);
                        store_integer(&mut q, element, val);
                    }
                }

                T1_FIELD_TYPE_STRING | T1_FIELD_TYPE_KEY => {
                    let mut len: usize = limit.wrapping_sub(cur);

                    if cur >= limit {
                        count -= 1;
                        idx += 1;
                        continue;
                    }

                    /* we allow both a string or a name   */
                    /* for cases like /FontName (foo) def */
                    if token.type_ == T1_TOKEN_TYPE_KEY {
                        /* don't include leading `/' */
                        len -= 1;
                        cur += 1;
                    } else if token.type_ == T1_TOKEN_TYPE_STRING {
                        /* don't include delimiting parentheses    */
                        /* XXX we don't handle <<...>> here        */
                        /* XXX should we convert octal escapes?    */
                        /*     if so, what encoding should we use? */
                        cur += 1;
                        len = len.wrapping_sub(2);
                    } else {
                        return Err(FT_ERR_INVALID_FILE_FORMAT);
                    }

                    /* for this to work (FT_String**)q must have been */
                    /* initialized to NULL                            */
                    let Some(access) = field.access else {
                        count -= 1;
                        idx += 1;
                        continue;
                    };
                    let mut q = access(object);
                    if let T1FieldRef::String(q) = &mut q {
                        if q.is_some() {
                            **q = None;
                        }

                        let mut string = ft_qalloc(len as FtLong + 1)?;

                        let src = bytes(b, cur, cur + len);
                        string[..src.len()].copy_from_slice(src);
                        string.truncate(len);

                        **q = Some(string);
                    }
                }

                T1_FIELD_TYPE_BBOX => {
                    let mut temp: [FtFixed; 4] = [0; 4];

                    let result = ps_tofixedarray(b, &mut cur, limit, 4, Some(&mut temp), 0);

                    if result < 4 {
                        return Err(FT_ERR_INVALID_FILE_FORMAT);
                    }

                    if let Some(access) = field.access {
                        if let T1FieldRef::BBox(bbox) = access(object) {
                            bbox.xMin = ft_round_fix(temp[0]);
                            bbox.yMin = ft_round_fix(temp[1]);
                            bbox.xMax = ft_round_fix(temp[2]);
                            bbox.yMax = ft_round_fix(temp[3]);
                        }
                    }
                }

                T1_FIELD_TYPE_MM_BBOX => {
                    let mut temp: Vec<FtFixed> = ft_new_array(max_objects as FtLong * 4)?;

                    for i in 0..4usize {
                        let m = max_objects as usize;
                        let result = ps_tofixedarray(
                            b,
                            &mut cur,
                            limit,
                            max_objects as FtInt,
                            Some(&mut temp[i * m..(i + 1) * m]),
                            0,
                        );
                        if result < 0 || (result as FtUInt) < max_objects {
                            return Err(FT_ERR_INVALID_FILE_FORMAT);
                        }

                        skip_spaces(b, &mut cur, limit);
                    }

                    let m = max_objects as usize;
                    for i in 0..m {
                        let Some(object) = objects.get_mut(i) else {
                            continue;
                        };
                        if let Some(access) = field.access {
                            if let T1FieldRef::BBox(bbox) = access(object) {
                                bbox.xMin = ft_round_fix(temp[i]);
                                bbox.yMin = ft_round_fix(temp[i + m]);
                                bbox.xMax = ft_round_fix(temp[i + 2 * m]);
                                bbox.yMax = ft_round_fix(temp[i + 3 * m]);
                            }
                        }
                    }
                }

                _ => {
                    /* an error occurred */
                    break 'fail;
                }
            }

            count -= 1;
            idx += 1;
        }

        /* (the obsolete `pflags') */

        /* Exit: */
        return Ok(());
    }

    /* Fail: */
    Err(FT_ERR_INVALID_FILE_FORMAT)
}

pub const T1_MAX_TABLE_ELEMENTS: usize = 32;

/// `ps_parser_load_field_table`
pub fn ps_parser_load_field_table<R>(
    parser: &mut PsParserRec,
    b: &[u8],
    field: &T1FieldRec<R>,
    objects: &mut [T1Object<'_>],
    max_objects: FtUInt,
) -> FtResult<()> {
    let mut elements = [T1TokenRec::default(); T1_MAX_TABLE_ELEMENTS];
    let mut num_elements: FtInt = 0;
    let mut error: FtResult<()> = Ok(());

    let mut fieldtype = T1_FIELD_TYPE_INTEGER;
    if field.type_ == T1_FIELD_TYPE_FIXED_ARRAY || field.type_ == T1_FIELD_TYPE_BBOX {
        fieldtype = T1_FIELD_TYPE_FIXED;
    }

    ps_parser_to_token_array(
        parser,
        b,
        Some(&mut elements),
        T1_MAX_TABLE_ELEMENTS as FtUInt,
        &mut num_elements,
    );
    if num_elements < 0 {
        return Err(FT_ERR_IGNORE);
    }
    if num_elements as FtUInt > field.array_max {
        num_elements = field.array_max as FtInt;
    }

    let old_cursor = parser.cursor;
    let old_limit = parser.limit;

    /* we store the elements count if necessary;           */
    /* we further assume that `count_offset' can't be zero */
    if field.type_ != T1_FIELD_TYPE_BBOX {
        if let (Some(access), Some(object)) = (field.access, objects.get_mut(0)) {
            match access(object) {
                T1FieldRef::Shorts(_, mut c)
                | T1FieldRef::UShorts(_, mut c)
                | T1FieldRef::UInts(_, mut c)
                | T1FieldRef::ULongs(_, mut c) => c.set(num_elements),
                _ => {}
            }
        }
    }

    /* we now load each element, adjusting the field.offset on each one */
    let mut element: usize = 0;
    while num_elements > 0 {
        /* (more tokens than the table holds are not recorded) */
        let token = elements.get(element).copied().unwrap_or_default();
        parser.cursor = token.start();
        parser.limit = token.limit();

        error = ps_parser_load_field(parser, b, field, fieldtype, element, objects, max_objects);
        if error.is_err() {
            break;
        }

        element += 1;
        num_elements -= 1;
    }

    /* (the obsolete `pflags') */

    parser.cursor = old_cursor;
    parser.limit = old_limit;

    /* Exit: */
    error
}

/// `ps_parser_to_int`
pub fn ps_parser_to_int(parser: &mut PsParserRec, b: &[u8]) -> FtLong {
    ps_parser_skip_spaces(parser, b);
    ps_conv_to_int(b, &mut parser.cursor, parser.limit)
}

/// `ps_parser_to_bytes`: first character must be `<' if `delimiters' is
/// non-zero (the bytes are decoded into `bytes`, at most `max_bytes`)
pub fn ps_parser_to_bytes(
    parser: &mut PsParserRec,
    b: &[u8],
    bytes: &mut [u8],
    max_bytes: usize,
    pnum_bytes: &mut FtULong,
    delimiters: bool,
) -> FtResult<()> {
    ps_parser_skip_spaces(parser, b);
    let mut cur = parser.cursor;

    if cur >= parser.limit {
        return Ok(());
    }

    if delimiters {
        if at(b, cur) != b'<' {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        cur += 1;
    }

    *pnum_bytes = ps_conv_ascii_hex_decode(b, &mut cur, parser.limit, bytes, max_bytes) as FtULong;

    parser.cursor = cur;

    if delimiters {
        if cur < parser.limit && at(b, cur) != b'>' {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        parser.cursor += 1;
    }

    /* Exit: */
    Ok(())
}

/// `ps_parser_to_fixed`
pub fn ps_parser_to_fixed(parser: &mut PsParserRec, b: &[u8], power_ten: FtInt) -> FtFixed {
    ps_parser_skip_spaces(parser, b);
    ps_conv_to_fixed(b, &mut parser.cursor, parser.limit, power_ten as FtLong)
}

/// `ps_parser_to_coord_array`
pub fn ps_parser_to_coord_array(
    parser: &mut PsParserRec,
    b: &[u8],
    max_coords: FtInt,
    coords: Option<&mut [FtShort]>,
) -> FtInt {
    ps_parser_skip_spaces(parser, b);
    ps_tocoordarray(b, &mut parser.cursor, parser.limit, max_coords, coords)
}

/// `ps_parser_to_fixed_array`
pub fn ps_parser_to_fixed_array(
    parser: &mut PsParserRec,
    b: &[u8],
    max_values: FtInt,
    values: Option<&mut [FtFixed]>,
    power_ten: FtInt,
) -> FtInt {
    ps_parser_skip_spaces(parser, b);
    ps_tofixedarray(
        b,
        &mut parser.cursor,
        parser.limit,
        max_values,
        values,
        power_ten,
    )
}

/* (the `#if 0'ed T1_ToString and T1_ToBool) */

/// `ps_parser_init`
pub fn ps_parser_init(parser: &mut PsParserRec, base: usize, limit: usize) {
    parser.error = FT_ERR_OK;
    parser.base = base;
    parser.limit = limit;
    parser.cursor = base;
}

/// `ps_parser_done`
pub fn ps_parser_done(_parser: &mut PsParserRec) {}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                            T1 BUILDER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `T1_ParseState`: an enumeration type to handle charstring parsing
/// states
pub type T1ParseState = FtUInt;

pub const T1_PARSE_START: T1ParseState = 0;
pub const T1_PARSE_HAVE_WIDTH: T1ParseState = 1;
pub const T1_PARSE_HAVE_MOVETO: T1ParseState = 2;
pub const T1_PARSE_HAVE_PATH: T1ParseState = 3;

/// `T1_BuilderRec`: a structure used during glyph loading to store its
/// outline: the fields it shares with the CFF builder (`builder`: the
/// face data, glyph, positions, bearing, advance, bounding box and
/// flags, which the PS builder of the Adobe engine reads), and its parse
/// state.
#[derive(Debug)]
pub struct T1BuilderRec<'a> {
    pub builder: CffBuilder<'a>,
    pub parse_state: T1ParseState,
}

/// `t1_builder_init`: initializes a given glyph builder.
///
/// * `face`: the current face object (the data read from it).
/// * `size`: whether there is a current size object (with its
///   `internal->module_data`, the PS hinter globals).
/// * `glyph`: the current glyph object.
/// * `hinting`: whether hinting should be applied.
pub fn t1_builder_init<'a>(
    face: CffBuilderFace,
    size: Option<bool>,
    glyph: Option<CffGlyph<'a>>,
    hinting: bool,
) -> T1BuilderRec<'a> {
    let mut builder = CffBuilder {
        face,
        glyph,
        pos_x: 0,
        pos_y: 0,
        left_bearing: FtVector::default(),
        advance: FtVector::default(),
        bbox: FtBBox::default(),
        path_begun: false,
        load_points: true,
        no_recurse: false,
        metrics_only: false,
        hints_funcs: false,
        hints_globals: None,
    };

    if let Some(glyph) = builder.glyph.as_mut() {
        let glyph_hints = glyph.root.internal.glyph_hints.is_some();
        let loader = glyph
            .root
            .internal
            .loader
            .get_or_insert_with(FtGlyphLoaderRec::new);

        loader.rewind();

        builder.hints_globals = if size == Some(true) {
            Some(CffSubFontId::Top)
        } else {
            None
        };
        builder.hints_funcs = false;

        if hinting {
            builder.hints_funcs = glyph_hints;
        }
    }

    builder.pos_x = 0;
    builder.pos_y = 0;

    builder.left_bearing.x = 0;
    builder.left_bearing.y = 0;
    builder.advance.x = 0;
    builder.advance.y = 0;

    T1BuilderRec {
        builder,
        parse_state: T1_PARSE_START,
    }
}

/// `t1_builder_done`: finalizes a given glyph builder.  Its contents can
/// still be used after the call, but the function saves important
/// information within the corresponding glyph slot.
pub fn t1_builder_done(builder: &mut T1BuilderRec<'_>) {
    if let Some(glyph) = builder.builder.glyph.as_mut() {
        copy_base_outline(glyph);
    }
}

/// `t1_builder_check_points`: check that there is enough space for
/// `count' more points
pub fn t1_builder_check_points(builder: &mut T1BuilderRec<'_>, count: FtInt) -> FtResult<()> {
    match builder.builder.loader() {
        Some(loader) => loader.check_points_macro(count as FtUInt, 0),
        None => Ok(()),
    }
}

/// `t1_builder_add_point`: add a new point, do not check space
pub fn t1_builder_add_point(builder: &mut T1BuilderRec<'_>, x: FtPos, y: FtPos, flag: FtByte) {
    let load_points = builder.builder.load_points;
    let Some(loader) = builder.builder.loader() else {
        return;
    };

    if load_points {
        let n = loader.current.n_points as u16 as usize;

        if let Some(point) = loader.current_points().get_mut(n) {
            point.x = fixed_to_int(x);
            point.y = fixed_to_int(y);
        }
        if let Some(control) = loader.current_tags().get_mut(n) {
            *control = if flag != 0 {
                FT_CURVE_TAG_ON
            } else {
                FT_CURVE_TAG_CUBIC
            };
        }
    }
    loader.current.n_points = loader.current.n_points.wrapping_add(1);
}

/// `t1_builder_add_point1`: check space for a new on-curve point, then
/// add it
pub fn t1_builder_add_point1(builder: &mut T1BuilderRec<'_>, x: FtPos, y: FtPos) -> FtResult<()> {
    t1_builder_check_points(builder, 1)?;
    t1_builder_add_point(builder, x, y, 1);
    Ok(())
}

/// `t1_builder_add_contour`: check space for a new contour, then add it
pub fn t1_builder_add_contour(builder: &mut T1BuilderRec<'_>) -> FtResult<()> {
    let load_points = builder.builder.load_points;

    /* this might happen in invalid fonts */
    let Some(loader) = builder.builder.loader() else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    };

    if !load_points {
        loader.current.n_contours = loader.current.n_contours.wrapping_add(1);
        return Ok(());
    }

    loader.check_points_macro(0, 1)?;

    if loader.current.n_contours > 0 {
        let c = loader.current.n_contours as usize - 1;
        let v = (loader.current.n_points as i32 - 1) as i16;
        loader.current_contours()[c] = v;
    }
    loader.current.n_contours = loader.current.n_contours.wrapping_add(1);

    Ok(())
}

/// `t1_builder_start_point`: if a path was begun, add its first on-curve
/// point
pub fn t1_builder_start_point(builder: &mut T1BuilderRec<'_>, x: FtPos, y: FtPos) -> FtResult<()> {
    /* test whether we are building a new contour */

    if builder.parse_state == T1_PARSE_HAVE_PATH {
        Ok(())
    } else {
        builder.parse_state = T1_PARSE_HAVE_PATH;
        t1_builder_add_contour(builder)?;
        t1_builder_add_point1(builder, x, y)
    }
}

/// `t1_builder_close_contour`: close the current contour
pub fn t1_builder_close_contour(builder: &mut T1BuilderRec<'_>) {
    if let Some(loader) = builder.builder.loader() {
        close_contour(loader);
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                           CFF BUILDER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// The face data a builder reads through C's `builder->face`.
#[derive(Debug, Clone, Default)]
#[allow(non_snake_case)]
pub struct CffBuilderFace {
    /// `face->units_per_EM`
    pub units_per_EM: FtUShort,
    /// `face->size->metrics.y_ppem`
    pub y_ppem: FtUShort,
    /// `face->internal->no_stem_darkening`
    pub no_stem_darkening: FtChar,
    /// `((TT_Face)face)->is_cff2`
    pub is_cff2: bool,
    /// the face's driver (`FT_FACE_DRIVER( face )`, a `PS_Driver`)
    pub driver: PsDriverRec,
    /// the face's normalized design coordinates (`mm->get_var_blend`):
    /// their number and the coordinates (`None` without a blend)
    pub num_coords: FtUInt,
    pub normalizedcoords: Option<Vec<FtFixed>>,
}

/// A CFF glyph slot (`CFF_GlyphSlot`): the root slot and the CFF driver's
/// additions.
#[derive(Debug)]
pub struct CffGlyph<'a> {
    pub root: &'a mut FtGlyphSlotRec,
    pub cff: &'a mut CffGlyphSlotRec,
}

/// `CFF_Builder`: a structure used during glyph loading to store its
/// outline.
///
/// * `face`: the current face object (the data read from it).
/// * `glyph`: the current glyph slot.
/// * `pos_x`: the horizontal translation (if composite glyph).
/// * `pos_y`: the vertical translation (if composite glyph).
/// * `left_bearing`: the left side bearing point.
/// * `advance`: the horizontal advance vector.
/// * `bbox`: unused.
/// * `path_begun`: a flag which indicates that a new path has begun.
/// * `load_points`: if this flag is not set, no points are loaded.
/// * `no_recurse`: set but not used.
/// * `metrics_only`: a boolean indicating that we only want to compute the
///   metrics of a given glyph, not load all of its points.
/// * `hints_funcs`: auxiliary pointer for hinting (set: the PS hinter's
///   Type 2 hints functions).
/// * `hints_globals`: auxiliary pointer for hinting (the PS hinter globals
///   of the top font or a subfont).
#[derive(Debug)]
pub struct CffBuilder<'a> {
    pub face: CffBuilderFace,
    pub glyph: Option<CffGlyph<'a>>,

    pub pos_x: FtPos,
    pub pos_y: FtPos,

    pub left_bearing: FtVector,
    pub advance: FtVector,

    pub bbox: FtBBox, /* bounding box */
    pub path_begun: bool,
    pub load_points: bool,
    pub no_recurse: bool,

    pub metrics_only: bool,

    pub hints_funcs: bool,                   /* hinter-specific */
    pub hints_globals: Option<CffSubFontId>, /* hinter-specific */
}

impl<'a> CffBuilder<'a> {
    /// The glyph's loader (`builder->loader`).
    pub fn loader(&mut self) -> Option<&mut FtGlyphLoaderRec> {
        self.glyph
            .as_mut()
            .and_then(|g| g.root.internal.loader.as_mut())
    }
}

/// `cff_builder_init`: initializes a given glyph builder.
///
/// * `face`: the current face object.
/// * `size`: whether there is a size, and whether its
///   `internal->module_data` (`CFF_Internal`) is set.
/// * `glyph`: the current glyph object.
/// * `hinting`: whether hinting is active.
pub fn cff_builder_init<'a>(
    face: CffBuilderFace,
    size: Option<bool>,
    glyph: Option<CffGlyph<'a>>,
    hinting: bool,
) -> CffBuilder<'a> {
    let mut builder = CffBuilder {
        face,
        glyph,
        pos_x: 0,
        pos_y: 0,
        left_bearing: FtVector::default(),
        advance: FtVector::default(),
        bbox: FtBBox::default(),
        path_begun: false,
        load_points: true,
        no_recurse: false,
        metrics_only: false,
        hints_funcs: false,
        hints_globals: None,
    };

    if let Some(glyph) = builder.glyph.as_mut() {
        let glyph_hints = glyph.root.internal.glyph_hints.is_some();
        let loader = glyph
            .root
            .internal
            .loader
            .get_or_insert_with(FtGlyphLoaderRec::new);

        loader.rewind();

        builder.hints_globals = None;
        builder.hints_funcs = false;

        if hinting {
            if let Some(internal) = size {
                if internal {
                    builder.hints_globals = Some(CffSubFontId::Top);
                    builder.hints_funcs = glyph_hints;
                }
            }
        }
    }

    builder.pos_x = 0;
    builder.pos_y = 0;

    builder.left_bearing.x = 0;
    builder.left_bearing.y = 0;
    builder.advance.x = 0;
    builder.advance.y = 0;

    builder
}

/// Copies the loader's base outline into the glyph slot (C's structure
/// copy `glyph->root.outline = *builder->base`).
fn copy_base_outline(glyph: &mut CffGlyph<'_>) {
    let Some(loader) = glyph.root.internal.loader.as_ref() else {
        return;
    };
    let base = &loader.base.outline;
    let n = (base.n_points.max(0) as usize).min(base.points.len());
    let nc = (base.n_contours.max(0) as usize).min(base.contours.len());

    glyph.root.outline = FtOutline {
        n_contours: base.n_contours,
        n_points: base.n_points,
        points: base.points[..n].to_vec(),
        tags: base.tags[..n].to_vec(),
        contours: base.contours[..nc].to_vec(),
        flags: base.flags,
    };
}

/// `cff_builder_done`: finalizes a given glyph builder.  Its contents can
/// still be used after the call, but the function saves important
/// information within the corresponding glyph slot.
pub fn cff_builder_done(builder: &mut CffBuilder<'_>) {
    if let Some(glyph) = builder.glyph.as_mut() {
        copy_base_outline(glyph);
    }
}

/// `cff_check_points`: check that there is enough space for `count' more
/// points
pub fn cff_check_points(builder: &mut CffBuilder<'_>, count: FtInt) -> FtResult<()> {
    match builder.loader() {
        Some(loader) => loader.check_points_macro(count as FtUInt, 0),
        None => Ok(()),
    }
}

/// `cff_builder_add_point`: add a new point, do not check space
pub fn cff_builder_add_point(builder: &mut CffBuilder<'_>, x: FtPos, y: FtPos, flag: FtByte) {
    let load_points = builder.load_points;
    let Some(loader) = builder.loader() else {
        return;
    };

    if load_points {
        let n = loader.current.n_points as u16 as usize;

        /* cf2_decoder_parse_charstrings uses 16.16 coordinates */
        if let Some(point) = loader.current_points().get_mut(n) {
            point.x = x >> 10;
            point.y = y >> 10;
        }
        if let Some(control) = loader.current_tags().get_mut(n) {
            *control = if flag != 0 {
                FT_CURVE_TAG_ON
            } else {
                FT_CURVE_TAG_CUBIC
            };
        }
    }
    loader.current.n_points = loader.current.n_points.wrapping_add(1);
}

/// `cff_builder_add_point1`: check space for a new on-curve point, then
/// add it
pub fn cff_builder_add_point1(builder: &mut CffBuilder<'_>, x: FtPos, y: FtPos) -> FtResult<()> {
    cff_check_points(builder, 1)?;
    cff_builder_add_point(builder, x, y, 1);
    Ok(())
}

/// `cff_builder_add_contour`: check space for a new contour, then add it
pub fn cff_builder_add_contour(builder: &mut CffBuilder<'_>) -> FtResult<()> {
    let load_points = builder.load_points;
    let Some(loader) = builder.loader() else {
        return Ok(());
    };

    if !load_points {
        loader.current.n_contours = loader.current.n_contours.wrapping_add(1);
        return Ok(());
    }

    loader.check_points_macro(0, 1)?;

    if loader.current.n_contours > 0 {
        let c = loader.current.n_contours as usize - 1;
        let v = (loader.current.n_points as i32 - 1) as i16;
        loader.current_contours()[c] = v;
    }
    loader.current.n_contours = loader.current.n_contours.wrapping_add(1);

    Ok(())
}

/// `cff_builder_start_point`: if a path was begun, add its first on-curve
/// point
pub fn cff_builder_start_point(builder: &mut CffBuilder<'_>, x: FtPos, y: FtPos) -> FtResult<()> {
    /* test whether we are building a new contour */
    if !builder.path_begun {
        builder.path_begun = true;
        cff_builder_add_contour(builder)?;
        cff_builder_add_point1(builder, x, y)?;
    }

    Ok(())
}

/// The body of `cff_builder_close_contour` and `ps_builder_close_contour`
/// (they are the same) on a loader's current outline.
fn close_contour(loader: &mut FtGlyphLoaderRec) {
    let n_contours = loader.current.n_contours as i32;
    let n_points = loader.current.n_points as i32;

    let first: i32 = if n_contours <= 1 {
        0
    } else {
        loader.current_contours()[n_contours as usize - 2] as i32 + 1
    };

    /* in malformed fonts it can happen that a contour was started */
    /* but no points were added                                    */
    if n_contours != 0 && first == n_points {
        loader.current.n_contours -= 1;
        return;
    }

    /* We must not include the last point in the path if it */
    /* is located on the first point.                       */
    if n_points > 1 {
        let points = loader.current_points();
        let p1 = points
            .get(first.max(0) as usize)
            .copied()
            .unwrap_or_default();
        let p2 = points[n_points as usize - 1];
        let control = loader.current_tags()[n_points as usize - 1];

        /* `delete' last point only if it coincides with the first */
        /* point and it is not a control point (which can happen). */
        if p1.x == p2.x && p1.y == p2.y && control == FT_CURVE_TAG_ON {
            loader.current.n_points -= 1;
        }
    }

    if loader.current.n_contours > 0 {
        let n_points = loader.current.n_points as i32;

        /* Don't add contours only consisting of one point, i.e.,  */
        /* check whether the first and the last point is the same. */
        if first == n_points - 1 {
            loader.current.n_contours -= 1;
            loader.current.n_points -= 1;
        } else {
            let c = loader.current.n_contours as usize - 1;
            loader.current_contours()[c] = (n_points - 1) as i16;
        }
    }
}

/// `cff_builder_close_contour`: close the current contour
pub fn cff_builder_close_contour(builder: &mut CffBuilder<'_>) {
    if let Some(loader) = builder.loader() {
        close_contour(loader);
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                            PS BUILDER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `PS_Builder`: a structure used during glyph loading to store its
/// outline (a wrapper around the CFF builder `builder`, whose glyph,
/// loader, positions, bearing, advance and bounding box it uses).
///
/// * `path_begun`: a flag which indicates that a new path has begun.
/// * `load_points`: if this flag is not set, no points are loaded.
/// * `no_recurse`: set but not used.
/// * `metrics_only`: a boolean indicating that we only want to compute the
///   metrics of a given glyph, not load all of its points.
/// * `is_t1`: set if current font type is Type 1.
#[derive(Debug)]
pub struct PsBuilder<'a, 'b> {
    pub builder: &'a mut CffBuilder<'b>,

    pub path_begun: bool,
    pub load_points: bool,
    pub no_recurse: bool,

    pub metrics_only: bool,
    pub is_t1: bool,
}

/// `ps_builder_init`: initializes a given glyph builder (the wrapper of
/// a CFF builder, or of the CFF builder part of a Type 1 builder if
/// `is_t1`).
pub fn ps_builder_init<'a, 'b>(
    cffbuilder: &'a mut CffBuilder<'b>,
    is_t1: bool,
) -> PsBuilder<'a, 'b> {
    /* (a Type 1 builder's path is not begun) */
    let path_begun = if is_t1 { false } else { cffbuilder.path_begun };
    let load_points = cffbuilder.load_points;
    let no_recurse = cffbuilder.no_recurse;
    let metrics_only = cffbuilder.metrics_only;

    PsBuilder {
        builder: cffbuilder,
        path_begun,
        load_points,
        no_recurse,
        metrics_only,
        is_t1,
    }
}

/// `ps_builder_done`: finalizes a given glyph builder.  Its contents can
/// still be used after the call, but the function saves important
/// information within the corresponding glyph slot.
pub fn ps_builder_done(builder: &mut PsBuilder<'_, '_>) {
    if let Some(glyph) = builder.builder.glyph.as_mut() {
        copy_base_outline(glyph);
    }
}

/// `ps_builder_check_points`: check that there is enough space for
/// `count' more points
pub fn ps_builder_check_points(builder: &mut PsBuilder<'_, '_>, count: FtInt) -> FtResult<()> {
    match builder.builder.loader() {
        Some(loader) => loader.check_points_macro(count as FtUInt, 0),
        None => Ok(()),
    }
}

/// `ps_builder_add_point`: add a new point, do not check space
pub fn ps_builder_add_point(builder: &mut PsBuilder<'_, '_>, x: FtPos, y: FtPos, flag: FtByte) {
    let load_points = builder.load_points;
    let Some(loader) = builder.builder.loader() else {
        return;
    };

    if load_points {
        let n = loader.current.n_points as u16 as usize;

        /* (the old engines are not compiled) */
        /* cf2_decoder_parse_charstrings uses 16.16 coordinates */
        if let Some(point) = loader.current_points().get_mut(n) {
            point.x = x >> 10;
            point.y = y >> 10;
        }
        if let Some(control) = loader.current_tags().get_mut(n) {
            *control = if flag != 0 {
                FT_CURVE_TAG_ON
            } else {
                FT_CURVE_TAG_CUBIC
            };
        }
    }
    loader.current.n_points = loader.current.n_points.wrapping_add(1);
}

/// `ps_builder_add_point1`: check space for a new on-curve point, then add
/// it
pub fn ps_builder_add_point1(builder: &mut PsBuilder<'_, '_>, x: FtPos, y: FtPos) -> FtResult<()> {
    ps_builder_check_points(builder, 1)?;
    ps_builder_add_point(builder, x, y, 1);
    Ok(())
}

/// `ps_builder_add_contour`: check space for a new contour, then add it
pub fn ps_builder_add_contour(builder: &mut PsBuilder<'_, '_>) -> FtResult<()> {
    let load_points = builder.load_points;

    /* this might happen in invalid fonts */
    let Some(loader) = builder.builder.loader() else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    };

    if !load_points {
        loader.current.n_contours = loader.current.n_contours.wrapping_add(1);
        return Ok(());
    }

    loader.check_points_macro(0, 1)?;

    if loader.current.n_contours > 0 {
        let c = loader.current.n_contours as usize - 1;
        let v = (loader.current.n_points as i32 - 1) as i16;
        loader.current_contours()[c] = v;
    }
    loader.current.n_contours = loader.current.n_contours.wrapping_add(1);

    Ok(())
}

/// `ps_builder_start_point`: if a path was begun, add its first on-curve
/// point
pub fn ps_builder_start_point(builder: &mut PsBuilder<'_, '_>, x: FtPos, y: FtPos) -> FtResult<()> {
    /* test whether we are building a new contour */
    if !builder.path_begun {
        builder.path_begun = true;
        ps_builder_add_contour(builder)?;
        ps_builder_add_point1(builder, x, y)?;
    }

    Ok(())
}

/// `ps_builder_close_contour`: close the current contour
pub fn ps_builder_close_contour(builder: &mut PsBuilder<'_, '_>) {
    if let Some(loader) = builder.builder.loader() {
        close_contour(loader);
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                            PS DECODER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

pub const PS_MAX_OPERANDS: usize = 48;
pub const PS_MAX_SUBRS_CALLS: usize = 16; /* maximum subroutine nesting;         */
/* only 10 are allowed but there exist */
/* fonts like `HiraKakuProN-W3.ttf'    */
/* (Hiragino Kaku Gothic ProN W3;      */
/* 8.2d6e1; 2014-12-19) that exceed    */
/* this limit                          */

/// `PS_Decoder`
///
/// The decoder wraps a CFF decoder (CFF mode: [`PsDecoderFont::Cff`],
/// whose font, stream and glyph width it uses) or a Type 1 decoder (Type
/// 1 mode: [`PsDecoderFont::T1`]). The subroutine tables are offsets into
/// their index's bytes (`locals_bytes`, `globals_bytes`; a CID font's
/// subroutines, with C's `subrs[idx + 1]` ends); a Type 1 font's are its
/// subroutines table.
#[derive(Debug)]
pub struct PsDecoder<'a, 'b> {
    pub builder: PsBuilder<'a, 'b>,

    pub flex_state: FtInt,
    pub num_flex_vectors: FtInt,
    pub flex_vectors: [FtVector; 7],

    pub font: PsDecoderFont<'a>,

    pub width_only: bool,
    pub num_hints: FtInt,

    pub num_locals: FtUInt,
    pub num_globals: FtUInt,

    pub locals_bias: FtInt,
    pub globals_bias: FtInt,

    pub locals: Option<Arc<[usize]>>,
    pub locals_bytes: Option<Arc<[u8]>>,
    pub globals: Option<Arc<[usize]>>,
    pub globals_bytes: Option<Arc<[u8]>>,

    pub num_glyphs: FtUInt, /* number of glyphs in font */

    pub hint_mode: FtRenderMode,

    pub seac: bool,
}

/// The font a PS decoder renders.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] /* (a decoder is a stack variable) */
pub enum PsDecoderFont<'a> {
    /// CFF mode: the CFF font (`cff`, whose `cf2_instance` is the
    /// decoder's), its subfont for the current glyph, the face's stream
    /// (for the charstrings of seac components) and the CFF decoder's
    /// glyph width
    Cff {
        cff: &'a mut CffFontRec,
        current_subfont: CffSubFontId, /* for current glyph_index */
        stream: &'a mut FtStreamRec,
        glyph_width: &'a mut FtPos,
    },
    /// Type 1 mode
    T1(PsDecoderT1<'a>),
}

/// The Type 1 decoder's part of a PS decoder (`Type 1 stuff', and the
/// Type 1 decoder's synthetic subfont and font instance).
///
/// * `current_subfont`: the subfont `t1_make_subfont` made.
/// * `cf2_instance`: the Type 1 decoder's font instance.
/// * `glyph_names`: the font's glyph names, for seac (`None` for CID
///   fonts).
/// * `charstrings`: the font's charstrings, for seac (the Type 1 face's).
/// * `lenIV`: internal for sub routine calls.
/// * `locals_t1`: the subroutines of a Type 1 font, with their lengths
///   (`locals_len`; `None` for CID fonts, whose subroutines are
///   `locals`, with their seed bytes).
/// * `locals_hash`: used if `num_subrs' was massaged.
/// * `blend`: for multiple master support.
/// * `buildchar`: the face's BuildCharArray (its length is
///   `len_buildchar`).
#[derive(Debug)]
#[allow(non_snake_case)]
pub struct PsDecoderT1<'a> {
    pub current_subfont: &'a mut CffSubFontRec,
    pub cf2_instance: &'a mut Option<Box<Cf2FontRec>>,

    pub glyph_names: Option<PsTableData>,
    pub charstrings: Option<PsTableData>,

    pub lenIV: FtInt,
    pub locals_t1: Option<PsTableData>,
    pub locals_hash: Option<&'a HashMap<FtInt, usize>>,

    pub font_matrix: FtMatrix,
    pub font_offset: FtVector,

    pub blend: Option<&'a PsBlendRec>,

    pub len_buildchar: FtUInt,
    pub buildchar: &'a mut [FtLong],
}

impl<'a> PsDecoder<'a, '_> {
    /// `decoder->current_subfont`
    pub fn subfont(&self) -> &CffSubFontRec {
        match &self.font {
            PsDecoderFont::Cff {
                cff,
                current_subfont,
                ..
            } => cff.subfont(*current_subfont),
            PsDecoderFont::T1(t1) => t1.current_subfont,
        }
    }

    /// `decoder->current_subfont`, mutably
    pub fn subfont_mut(&mut self) -> &mut CffSubFontRec {
        match &mut self.font {
            PsDecoderFont::Cff {
                cff,
                current_subfont,
                ..
            } => cff.subfont_mut(*current_subfont),
            PsDecoderFont::T1(t1) => t1.current_subfont,
        }
    }

    /// The current subfont's identity (C's `current_subfont` pointer; the
    /// synthetic subfont of a Type 1 decoder is always the same one).
    pub fn subfont_id(&self) -> CffSubFontId {
        match &self.font {
            PsDecoderFont::Cff {
                current_subfont, ..
            } => *current_subfont,
            PsDecoderFont::T1(_) => CffSubFontId::Top,
        }
    }

    /// `decoder->cf2_instance` (the font instance's data)
    pub fn cf2_instance(&mut self) -> &mut Option<Box<Cf2FontRec>> {
        match &mut self.font {
            PsDecoderFont::Cff { cff, .. } => &mut cff.cf2_instance,
            PsDecoderFont::T1(t1) => t1.cf2_instance,
        }
    }

    /// `decoder->cff` (CFF mode)
    pub fn cff(&self) -> Option<&CffFontRec> {
        match &self.font {
            PsDecoderFont::Cff { cff, .. } => Some(cff),
            PsDecoderFont::T1(_) => None,
        }
    }

    /// The Type 1 decoder's part (Type 1 mode).
    pub fn t1(&self) -> Option<&PsDecoderT1<'a>> {
        match &self.font {
            PsDecoderFont::Cff { .. } => None,
            PsDecoderFont::T1(t1) => Some(t1),
        }
    }

    /// The Type 1 decoder's part (Type 1 mode), mutably.
    pub fn t1_mut(&mut self) -> Option<&mut PsDecoderT1<'a>> {
        match &mut self.font {
            PsDecoderFont::Cff { .. } => None,
            PsDecoderFont::T1(t1) => Some(t1),
        }
    }
}

/// `ps_decoder_init`: creates a wrapper decoder for use in the combined
/// Type 1 / CFF interpreter (of a CFF decoder; [`ps_decoder_init_t1`] is
/// the Type 1 decoder's).
pub fn ps_decoder_init<'a, 'b>(decoder: &'a mut CffDecoder<'b>, is_t1: bool) -> PsDecoder<'a, 'b> {
    let CffDecoder {
        builder,
        cff,
        stream,
        glyph_width,
        width_only,
        num_locals,
        num_globals,
        locals_bias,
        globals_bias,
        locals,
        locals_bytes,
        globals,
        globals_bytes,
        hint_mode,
        current_subfont,
        ..
    } = decoder;

    PsDecoder {
        builder: ps_builder_init(builder, is_t1),

        flex_state: 0,
        num_flex_vectors: 0,
        flex_vectors: [FtVector::default(); 7],

        font: PsDecoderFont::Cff {
            cff,
            current_subfont: *current_subfont,
            stream,
            glyph_width,
        },

        num_globals: *num_globals,
        globals: globals.clone(),
        globals_bytes: globals_bytes.clone(),
        globals_bias: *globals_bias,
        num_locals: *num_locals,
        locals: locals.clone(),
        locals_bytes: locals_bytes.clone(),
        locals_bias: *locals_bias,

        width_only: *width_only,
        num_hints: 0,

        num_glyphs: 0,

        hint_mode: *hint_mode,

        seac: false,
    }
}

/// `ps_decoder_init` of a Type 1 decoder (`is_t1` set), with the subfont
/// `t1_make_subfont` made for it (C's `psdecoder.current_subfont`, set by
/// the caller).
pub fn ps_decoder_init_t1<'a, 'b>(
    t1_decoder: &'a mut T1DecoderRec<'b>,
    subfont: &'a mut CffSubFontRec,
) -> PsDecoder<'a, 'b> {
    let T1DecoderRec {
        builder,
        num_glyphs,
        glyph_names,
        charstrings,
        hint_mode,
        blend,
        num_subrs,
        subrs,
        cid_subrs,
        cid_subrs_bytes,
        subrs_hash,
        buildchar,
        len_buildchar,
        lenIV,
        font_matrix,
        font_offset,
        cf2_instance,
        ..
    } = t1_decoder;

    PsDecoder {
        builder: ps_builder_init(&mut builder.builder, true),

        flex_state: 0,
        num_flex_vectors: 0,
        flex_vectors: [FtVector::default(); 7],

        font: PsDecoderFont::T1(PsDecoderT1 {
            current_subfont: subfont,
            cf2_instance,

            glyph_names: glyph_names.clone(),
            charstrings: charstrings.clone(),

            lenIV: *lenIV,
            locals_t1: subrs.clone(),
            locals_hash: *subrs_hash,

            font_matrix: *font_matrix,
            font_offset: *font_offset,

            blend: *blend,

            len_buildchar: *len_buildchar,
            buildchar: &mut buildchar[..],
        }),

        num_globals: 0,
        globals: None,
        globals_bytes: None,
        globals_bias: 0,
        num_locals: *num_subrs as FtUInt,
        locals: cid_subrs.clone(),
        locals_bytes: cid_subrs_bytes.clone(),
        locals_bias: 0,

        width_only: false,
        num_hints: 0,

        num_glyphs: *num_glyphs,

        hint_mode: *hint_mode,

        seac: false,
    }
}

/// `t1_make_subfont`: synthesize a SubFont object for Type 1 fonts, for
/// use in the new interpreter to access Private dict data
/// (`random_seed` is the face's `internal->random_seed`).
pub fn t1_make_subfont(
    random_seed: &mut FtInt32,
    priv_: &PsPrivateRec,
    subfont: &mut CffSubFontRec,
) {
    *subfont = CffSubFontRec::default();
    let cpriv = &mut subfont.private_dict;

    let count = priv_.num_blue_values as usize;
    cpriv.num_blue_values = priv_.num_blue_values;
    for n in 0..count.min(14) {
        cpriv.blue_values[n] = priv_.blue_values[n] as FtPos;
    }

    let count = priv_.num_other_blues as usize;
    cpriv.num_other_blues = priv_.num_other_blues;
    for n in 0..count.min(10) {
        cpriv.other_blues[n] = priv_.other_blues[n] as FtPos;
    }

    let count = priv_.num_family_blues as usize;
    cpriv.num_family_blues = priv_.num_family_blues;
    for n in 0..count.min(14) {
        cpriv.family_blues[n] = priv_.family_blues[n] as FtPos;
    }

    let count = priv_.num_family_other_blues as usize;
    cpriv.num_family_other_blues = priv_.num_family_other_blues;
    for n in 0..count.min(10) {
        cpriv.family_other_blues[n] = priv_.family_other_blues[n] as FtPos;
    }

    cpriv.blue_scale = priv_.blue_scale;
    cpriv.blue_shift = priv_.blue_shift as FtPos;
    cpriv.blue_fuzz = priv_.blue_fuzz as FtPos;

    cpriv.standard_width = priv_.standard_width[0] as FtPos;
    cpriv.standard_height = priv_.standard_height[0] as FtPos;

    let count = priv_.num_snap_widths as usize;
    cpriv.num_snap_widths = priv_.num_snap_widths;
    for n in 0..count.min(13) {
        cpriv.snap_widths[n] = priv_.snap_widths[n] as FtPos;
    }

    let count = priv_.num_snap_heights as usize;
    cpriv.num_snap_heights = priv_.num_snap_heights;
    for n in 0..count.min(13) {
        cpriv.snap_heights[n] = priv_.snap_heights[n] as FtPos;
    }

    cpriv.force_bold = priv_.force_bold;
    cpriv.lenIV = priv_.lenIV;
    cpriv.language_group = priv_.language_group as FtInt;
    cpriv.expansion_factor = priv_.expansion_factor;

    cpriv.subfont = true;

    /* Initialize the random number generator. */
    if *random_seed != -1 {
        /* If we have a face-specific seed, use it.    */
        /* If non-zero, update it to a positive value. */
        subfont.random = *random_seed as FtUInt32;
        if *random_seed != 0 {
            loop {
                *random_seed = cff_random(*random_seed as FtUInt32) as FtInt32;
                if *random_seed >= 0 {
                    break;
                }
            }
        }
    }
    if subfont.random == 0 {
        /* compute random seed from some memory addresses */
        let seed_local: FtUInt32 = 0;
        let mut seed: FtUInt32 = ((&seed_local as *const FtUInt32 as usize)
            ^ (random_seed as *const FtInt32 as usize)
            ^ (subfont as *const CffSubFontRec as usize))
            as FtUInt32;
        seed = seed ^ (seed >> 10) ^ (seed >> 20);
        if seed == 0 {
            seed = 0x7384;
        }

        subfont.random = seed;
    }
}

/// `t1_decrypt`
pub fn t1_decrypt(buffer: &mut [u8], length: usize, seed: FtUShort) {
    let mut seed = seed;
    ps_conv_eexec_decode_in_place(buffer, 0, length, &mut seed);
}

/// `cff_random`
pub fn cff_random(mut r: FtUInt32) -> FtUInt32 {
    /* a 32bit version of the `xorshift' algorithm */
    r ^= r << 13;
    r ^= r >> 17;
    r ^= r << 5;

    r
}
