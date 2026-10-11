// Rust translation of src/bdf/bdflib.c and src/bdf/bdf.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
//
// Copyright 2000 Computing Research Labs, New Mexico State University
// Copyright 2001-2014
//   Francesco Zappa Nardelli
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL
// THE COMPUTING RESEARCH LAB OR NEW MEXICO STATE UNIVERSITY BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT
// OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR
// THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// This is an altered (translated) version of the original software.

//! The BDF font loader (`bdflib`), based on bdf.c,v 1.22 2000/03/16
//! 20:08:50, taken from Mark Leisher's xmbdfed package.
//!
//! The lines are split in place as in C: a line is a NUL-terminated byte
//! slice of the input buffer, and the fields of [`BdfListRec`] are offsets
//! into it ([`BdfField`]; C's pointers to the static `empty` string and
//! its NULL terminator are variants of their own). The property tables
//! (`FT_HashRec`) are hash maps from the C strings to their indices.

use std::collections::HashMap;

use super::super::base::ftcalc::ft_mul_div;
use super::super::base::ftmemory::{ft_new_array, ft_renew_array};
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;

/* Imported from bdfP.h */

/// `_bdf_glyph_modified`
pub fn bdf_glyph_modified(map: &[u64], e: usize) -> bool {
    map[e >> 5] & (1u64 << (e & 31)) != 0
}

/// `_bdf_set_glyph_modified`
pub fn bdf_set_glyph_modified(map: &mut [u64], e: usize) {
    map[e >> 5] |= 1u64 << (e & 31);
}

/// `_bdf_clear_glyph_modified`
pub fn bdf_clear_glyph_modified(map: &mut [u64], e: usize) {
    map[e >> 5] &= !(1u64 << (e & 31));
}

/* end of bdfP.h */

/*************************************************************************
 *
 * BDF font options macros and types.
 *
 */

pub const BDF_CORRECT_METRICS: i32 = 0x01; /* Correct invalid metrics when loading. */
pub const BDF_KEEP_COMMENTS: i32 = 0x02; /* Preserve the font comments.           */
pub const BDF_KEEP_UNENCODED: i32 = 0x04; /* Keep the unencoded glyphs.            */
pub const BDF_PROPORTIONAL: i32 = 0x08; /* Font has proportional spacing.        */
pub const BDF_MONOWIDTH: i32 = 0x10; /* Font has mono width.                  */
pub const BDF_CHARCELL: i32 = 0x20; /* Font has charcell spacing.            */

pub const BDF_ALL_SPACING: i32 = BDF_PROPORTIONAL | BDF_MONOWIDTH | BDF_CHARCELL;

pub const BDF_DEFAULT_LOAD_OPTIONS: i32 =
    BDF_CORRECT_METRICS | BDF_KEEP_COMMENTS | BDF_KEEP_UNENCODED | BDF_PROPORTIONAL;

/// `bdf_options_t`
#[derive(Debug, Clone, Copy, Default)]
pub struct BdfOptions {
    pub correct_metrics: i32,
    pub keep_unencoded: i32,
    pub keep_comments: i32,
    pub font_spacing: i32,
}

/*************************************************************************
 *
 * BDF font property macros and types.
 *
 */

pub const BDF_ATOM: i32 = 1;
pub const BDF_INTEGER: i32 = 2;
pub const BDF_CARDINAL: i32 = 3;

/// `bdf_property_t`: a particular property of a font. There are a set of
/// defaults and each font has their own.
///
/// The `value` union is `atom` for atoms and the bits of `l`/`ul` (`num`)
/// for the integers and cardinals.
#[derive(Debug, Clone, Default)]
pub struct BdfProperty {
    pub name: Vec<u8>, /* Name of the property.   */
    pub format: i32,   /* Format of the property. */
    pub builtin: i32,  /* A builtin property.     */
    /// `value.atom`
    pub atom: Option<Vec<u8>>,
    /// `value.l` and `value.ul`
    pub num: u64,
}

impl BdfProperty {
    /// `value.l`
    pub fn value_l(&self) -> i64 {
        self.num as i64
    }
    /// `value.ul`
    pub fn value_ul(&self) -> u64 {
        self.num
    }
}

/*************************************************************************
 *
 * BDF font metric and glyph types.
 *
 */

/// `bdf_bbx_t`
#[derive(Debug, Clone, Copy, Default)]
pub struct BdfBbx {
    pub width: u16,
    pub height: u16,

    pub x_offset: i16,
    pub y_offset: i16,

    pub ascent: i16,
    pub descent: i16,
}

/// `bdf_glyph_t`
#[derive(Debug, Clone, Default)]
pub struct BdfGlyph {
    pub name: Option<Vec<u8>>, /* Glyph name.                          */
    pub encoding: u64,         /* Glyph encoding.                      */
    pub swidth: u16,           /* Scalable width.                      */
    pub dwidth: u16,           /* Device width.                        */
    pub bbx: BdfBbx,           /* Glyph bounding box.                  */
    pub bitmap: Vec<u8>,       /* Glyph bitmap.                        */
    pub bpr: u64,              /* Number of bytes used per row.        */
    pub bytes: u16,            /* Number of bytes used for the bitmap. */
}

/// `bdf_font_t`
#[derive(Debug, Default)]
pub struct BdfFont {
    pub name: Option<Vec<u8>>, /* Name of the font.                   */
    pub bbx: BdfBbx,           /* Font bounding box.                  */

    pub point_size: u64,   /* Point size of the font.             */
    pub resolution_x: u64, /* Font horizontal resolution.         */
    pub resolution_y: u64, /* Font vertical resolution.           */

    pub spacing: i32, /* Font spacing value.                 */

    pub monowidth: u16, /* Logical width for monowidth font.   */

    pub default_char: u64, /* Encoding of the default glyph.      */

    pub font_ascent: i64,  /* Font ascent.                        */
    pub font_descent: i64, /* Font descent.                       */

    pub glyphs_size: u64,      /* Glyph structures allocated.         */
    pub glyphs_used: u64,      /* Glyph structures used.              */
    pub glyphs: Vec<BdfGlyph>, /* Glyphs themselves.                  */

    pub unencoded_size: u64,      /* Unencoded glyph struct. allocated.  */
    pub unencoded_used: u64,      /* Unencoded glyph struct. used.       */
    pub unencoded: Vec<BdfGlyph>, /* Unencoded glyphs themselves.        */

    pub props_size: u64,         /* Font properties allocated.          */
    pub props_used: u64,         /* Font properties used.               */
    pub props: Vec<BdfProperty>, /* Font properties themselves.         */

    pub comments: Vec<u8>, /* Font comments.                      */
    pub comments_len: u64, /* Length of comment string.           */

    /// `internal`: the font's property names and their indices in `props`
    pub internal: Option<HashMap<Vec<u8>, usize>>,

    pub bpp: u16, /* Bits per pixel.                     */

    pub user_props: Vec<BdfProperty>,
    pub nuser_props: u64,
    /// `proptbl`: the property names and their indices in
    /// `bdf_properties_` (and then `user_props`)
    pub proptbl: HashMap<Vec<u8>, usize>,
}

/*************************************************************************
 *
 * Types for load/save callbacks.
 *
 */

/* Error codes. */
pub const BDF_MISSING_START: i32 = -1;
pub const BDF_MISSING_FONTNAME: i32 = -2;
pub const BDF_MISSING_SIZE: i32 = -3;
pub const BDF_MISSING_CHARS: i32 = -4;
pub const BDF_MISSING_STARTCHAR: i32 = -5;
pub const BDF_MISSING_ENCODING: i32 = -6;
pub const BDF_MISSING_BBX: i32 = -7;

pub const BDF_OUT_OF_MEMORY: i32 = -20;

pub const BDF_INVALID_LINE: i32 = -100;

/* bdflib.c */

const BUFSIZE: usize = 128;

/*************************************************************************
 *
 * Default BDF font options.
 *
 */

static BDF_OPTS_: BdfOptions = BdfOptions {
    correct_metrics: 1,             /* Correct metrics.               */
    keep_unencoded: 1,              /* Preserve unencoded glyphs.     */
    keep_comments: 0,               /* Preserve comments.             */
    font_spacing: BDF_PROPORTIONAL, /* Default spacing.               */
};

/*************************************************************************
 *
 * Builtin BDF font properties.
 *
 */

/* List of most properties that might appear in a font.  Doesn't include */
/* the RAW_* and AXIS_* properties in X11R6 polymorphic fonts.           */

static BDF_PROPERTIES_: [(&str, i32); 83] = [
    ("ADD_STYLE_NAME", BDF_ATOM),
    ("AVERAGE_WIDTH", BDF_INTEGER),
    ("AVG_CAPITAL_WIDTH", BDF_INTEGER),
    ("AVG_LOWERCASE_WIDTH", BDF_INTEGER),
    ("CAP_HEIGHT", BDF_INTEGER),
    ("CHARSET_COLLECTIONS", BDF_ATOM),
    ("CHARSET_ENCODING", BDF_ATOM),
    ("CHARSET_REGISTRY", BDF_ATOM),
    ("COMMENT", BDF_ATOM),
    ("COPYRIGHT", BDF_ATOM),
    ("DEFAULT_CHAR", BDF_CARDINAL),
    ("DESTINATION", BDF_CARDINAL),
    ("DEVICE_FONT_NAME", BDF_ATOM),
    ("END_SPACE", BDF_INTEGER),
    ("FACE_NAME", BDF_ATOM),
    ("FAMILY_NAME", BDF_ATOM),
    ("FIGURE_WIDTH", BDF_INTEGER),
    ("FONT", BDF_ATOM),
    ("FONTNAME_REGISTRY", BDF_ATOM),
    ("FONT_ASCENT", BDF_INTEGER),
    ("FONT_DESCENT", BDF_INTEGER),
    ("FOUNDRY", BDF_ATOM),
    ("FULL_NAME", BDF_ATOM),
    ("ITALIC_ANGLE", BDF_INTEGER),
    ("MAX_SPACE", BDF_INTEGER),
    ("MIN_SPACE", BDF_INTEGER),
    ("NORM_SPACE", BDF_INTEGER),
    ("NOTICE", BDF_ATOM),
    ("PIXEL_SIZE", BDF_INTEGER),
    ("POINT_SIZE", BDF_INTEGER),
    ("QUAD_WIDTH", BDF_INTEGER),
    ("RAW_ASCENT", BDF_INTEGER),
    ("RAW_AVERAGE_WIDTH", BDF_INTEGER),
    ("RAW_AVG_CAPITAL_WIDTH", BDF_INTEGER),
    ("RAW_AVG_LOWERCASE_WIDTH", BDF_INTEGER),
    ("RAW_CAP_HEIGHT", BDF_INTEGER),
    ("RAW_DESCENT", BDF_INTEGER),
    ("RAW_END_SPACE", BDF_INTEGER),
    ("RAW_FIGURE_WIDTH", BDF_INTEGER),
    ("RAW_MAX_SPACE", BDF_INTEGER),
    ("RAW_MIN_SPACE", BDF_INTEGER),
    ("RAW_NORM_SPACE", BDF_INTEGER),
    ("RAW_PIXEL_SIZE", BDF_INTEGER),
    ("RAW_POINT_SIZE", BDF_INTEGER),
    ("RAW_PIXELSIZE", BDF_INTEGER),
    ("RAW_POINTSIZE", BDF_INTEGER),
    ("RAW_QUAD_WIDTH", BDF_INTEGER),
    ("RAW_SMALL_CAP_SIZE", BDF_INTEGER),
    ("RAW_STRIKEOUT_ASCENT", BDF_INTEGER),
    ("RAW_STRIKEOUT_DESCENT", BDF_INTEGER),
    ("RAW_SUBSCRIPT_SIZE", BDF_INTEGER),
    ("RAW_SUBSCRIPT_X", BDF_INTEGER),
    ("RAW_SUBSCRIPT_Y", BDF_INTEGER),
    ("RAW_SUPERSCRIPT_SIZE", BDF_INTEGER),
    ("RAW_SUPERSCRIPT_X", BDF_INTEGER),
    ("RAW_SUPERSCRIPT_Y", BDF_INTEGER),
    ("RAW_UNDERLINE_POSITION", BDF_INTEGER),
    ("RAW_UNDERLINE_THICKNESS", BDF_INTEGER),
    ("RAW_X_HEIGHT", BDF_INTEGER),
    ("RELATIVE_SETWIDTH", BDF_CARDINAL),
    ("RELATIVE_WEIGHT", BDF_CARDINAL),
    ("RESOLUTION", BDF_INTEGER),
    ("RESOLUTION_X", BDF_CARDINAL),
    ("RESOLUTION_Y", BDF_CARDINAL),
    ("SETWIDTH_NAME", BDF_ATOM),
    ("SLANT", BDF_ATOM),
    ("SMALL_CAP_SIZE", BDF_INTEGER),
    ("SPACING", BDF_ATOM),
    ("STRIKEOUT_ASCENT", BDF_INTEGER),
    ("STRIKEOUT_DESCENT", BDF_INTEGER),
    ("SUBSCRIPT_SIZE", BDF_INTEGER),
    ("SUBSCRIPT_X", BDF_INTEGER),
    ("SUBSCRIPT_Y", BDF_INTEGER),
    ("SUPERSCRIPT_SIZE", BDF_INTEGER),
    ("SUPERSCRIPT_X", BDF_INTEGER),
    ("SUPERSCRIPT_Y", BDF_INTEGER),
    ("UNDERLINE_POSITION", BDF_INTEGER),
    ("UNDERLINE_THICKNESS", BDF_INTEGER),
    ("WEIGHT", BDF_CARDINAL),
    ("WEIGHT_NAME", BDF_ATOM),
    ("X_HEIGHT", BDF_INTEGER),
    ("_MULE_BASELINE_OFFSET", BDF_INTEGER),
    ("_MULE_RELATIVE_COMPOSE", BDF_INTEGER),
];

/// `num_bdf_properties_`
const NUM_BDF_PROPERTIES_: usize = BDF_PROPERTIES_.len();

/// The builtin property `i` (`bdf_properties_ + i`).
fn bdf_builtin_property(i: usize) -> BdfProperty {
    let (name, format) = BDF_PROPERTIES_[i];
    BdfProperty {
        name: name.as_bytes().to_vec(),
        format,
        builtin: 1,
        atom: None,
        num: 0,
    }
}

/// The C string at the start of `s` (up to its NUL, if any).
fn cstr(s: &[u8]) -> &[u8] {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..n]
}

/// The byte at `i` of the C string `s` (0 past its end).
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `ft_strncmp( name, property, n ) == 0`
fn strncmp_eq(name: &[u8], property: &[u8], n: usize) -> bool {
    for i in 0..n {
        let a = at(name, i);
        let b = at(property, i);
        if a != b {
            return false;
        }
        if a == 0 {
            return true;
        }
    }
    true
}

/// `_bdf_strncmp`: an auxiliary macro to parse properties, to be used in
/// conditionals.  It behaves like `strncmp' but also tests the following
/// character whether it is a whitespace or null. `property' is a constant
/// string of length `n' to compare with. Returns whether they differ.
fn bdf_strncmp(name: &[u8], property: &[u8], n: usize) -> bool {
    !strncmp_eq(name, property, n)
        || !(at(name, n) == b' '
            || at(name, n) == b'\0'
            || at(name, n) == b'\n'
            || at(name, n) == b'\r'
            || at(name, n) == b'\t')
}

/*************************************************************************
 *
 * Utility types and functions.
 *
 */

/// A field of a split line (`char*`): an offset into the line, the static
/// `empty` string, or NULL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BdfField {
    /// `empty`
    #[default]
    Empty,
    /// NULL
    Null,
    /// a string at this offset of the line
    At(usize),
}

/// `bdf_list_t_`: List structure for splitting lines into fields.
#[derive(Debug, Default)]
pub struct BdfListRec {
    pub field: Vec<BdfField>,
    pub size: u64,
    pub used: u64,
}

impl BdfListRec {
    /// `list->field[i]`
    fn get(&self, i: usize) -> BdfField {
        self.field.get(i).copied().unwrap_or(BdfField::Empty)
    }
}

/// The C string a field points to (NULL is `None`).
fn field_str(line: &[u8], f: BdfField) -> Option<&[u8]> {
    match f {
        BdfField::Empty => Some(b""),
        BdfField::Null => None,
        BdfField::At(i) => Some(cstr(line.get(i..).unwrap_or(&[]))),
    }
}

/// The next line parser (C's `bdf_line_func_t_` pointer `next`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BdfLineFunc {
    Start,
    Properties,
    Glyphs,
    End,
}

/// `bdf_parse_t_`: Structure used while loading BDF fonts.
#[derive(Debug)]
struct BdfParse {
    flags: u64,
    cnt: u64,
    row: u64,

    minlb: i16,
    maxlb: i16,
    maxrb: i16,
    maxas: i16,
    maxds: i16,

    rbearing: i16,

    glyph_name: Option<Vec<u8>>,
    glyph_enc: i64,

    font: Option<Box<BdfFont>>,
    opts: BdfOptions,

    list: BdfListRec,

    size: u64, /* the stream size */
}

/// `setsbit`
fn setsbit(m: &mut [u8; 32], cc: u8) {
    m[(cc >> 3) as usize] |= 1 << (cc & 7);
}

/// `sbitset`
fn sbitset(m: &[u8; 32], cc: u8) -> bool {
    m[(cc >> 3) as usize] & (1 << (cc & 7)) != 0
}

/// `bdf_list_init_`
fn bdf_list_init_(list: &mut BdfListRec) {
    *list = BdfListRec::default();
}

/// `bdf_list_done_`
fn bdf_list_done_(list: &mut BdfListRec) {
    *list = BdfListRec::default();
}

/// `bdf_list_ensure_`
fn bdf_list_ensure_(list: &mut BdfListRec, num_items: u64) -> FtResult<()> {
    /* same as bdf_list_t_.used */
    if num_items > list.size {
        let oldsize = list.size; /* same as bdf_list_t_.size */
        let mut newsize = oldsize.wrapping_add(oldsize >> 1).wrapping_add(5);
        /* (FT_INT_MAX / sizeof ( char* )) */
        let bigsize: u64 = (i32::MAX as u64) / 8;

        if oldsize == bigsize {
            return Err(FT_ERR_OUT_OF_MEMORY);
        } else if newsize < oldsize || newsize > bigsize {
            newsize = bigsize;
        }

        ft_renew_array(&mut list.field, newsize as FtLong)?;

        list.size = newsize;
    }

    Ok(())
}

/// `bdf_list_shift_`
fn bdf_list_shift_(list: &mut BdfListRec, n: u64) {
    if list.used == 0 || n == 0 {
        return;
    }

    if n >= list.used {
        list.used = 0;
        return;
    }

    let mut u = n as usize;
    let mut i = 0usize;
    while (u as u64) < list.used {
        list.field[i] = list.field[u];
        i += 1;
        u += 1;
    }
    list.used -= n;
}

/// `bdf_list_join_`: joins the fields with `c` in place (into the first
/// field's string, in `line`); returns the first field (NULL for an
/// empty list) and the length.
fn bdf_list_join_(list: &BdfListRec, line: &mut [u8], c: u8, alen: &mut u64) -> BdfField {
    *alen = 0;

    if list.used == 0 {
        return BdfField::Null;
    }

    let dp = list.field[0];
    let BdfField::At(d) = dp else {
        /* (the static `empty' string; only its own empty content is
        `copied') */
        return dp;
    };
    let mut j = 0usize;
    for i in 0..list.used as usize {
        if let BdfField::At(mut fp) = list.field[i] {
            while at(line, fp) != 0 {
                line[d + j] = line[fp];
                j += 1;
                fp += 1;
            }
        }

        if (i as u64) + 1 < list.used {
            line[d + j] = c;
            j += 1;
        }
    }
    line[d + j] = 0;

    *alen = j as u64;
    dp
}

/// `bdf_list_split_`: The code below ensures that we have at least 4 + 1
/// `field' elements in `list' (which are possibly NULL) so that we don't
/// have to check the number of fields in most cases.
fn bdf_list_split_(
    list: &mut BdfListRec,
    separators: &[u8],
    line: &mut [u8],
    linelen: u64,
) -> FtResult<()> {
    /* Initialize the list. */
    list.used = 0;
    if list.size != 0 {
        for f in list.field.iter_mut().take(5) {
            *f = BdfField::Empty;
        }
    }

    /* If the line is empty, then simply return. */
    if linelen == 0 || at(line, 0) == 0 {
        return Ok(());
    }

    /* In the original code, if the `separators' parameter is NULL or */
    /* empty, the list is split into individual bytes.  We don't need */
    /* this, so an error is signaled.                                 */
    if separators.is_empty() {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* Prepare the separator bitmap. */
    let mut seps = [0u8; 32];

    /* If the very last character of the separator string is a plus, then */
    /* set the `mult' flag to indicate that multiple separators should be */
    /* collapsed into one.                                                */
    let mut mult = false;
    for (k, &sp) in separators.iter().enumerate() {
        if sp == b'+' && k + 1 == separators.len() {
            mult = true;
        } else {
            setsbit(&mut seps, sp);
        }
    }

    /* Break the line up into fields. */
    let mut final_empty: u64 = 0;
    let mut sp = 0usize;
    let mut ep = 0usize;
    let end = linelen as usize;
    while sp < end && at(line, sp) != 0 {
        /* Collect everything that is not a separator. */
        while at(line, ep) != 0 && !sbitset(&seps, at(line, ep)) {
            ep += 1;
        }

        /* Resize the list if necessary. */
        if list.used == list.size {
            bdf_list_ensure_(list, list.used + 1)?;
        }

        /* Assign the field appropriately. */
        list.field[list.used as usize] = if ep > sp {
            BdfField::At(sp)
        } else {
            BdfField::Empty
        };
        list.used += 1;

        sp = ep;

        if mult {
            /* If multiple separators should be collapsed, do it now by */
            /* setting all the separator characters to 0.               */
            while at(line, ep) != 0 && sbitset(&seps, at(line, ep)) {
                line[ep] = 0;
                ep += 1;
            }
        } else if at(line, ep) != 0 {
            /* Don't collapse multiple separators by making them 0, so just */
            /* make the one encountered 0.                                  */
            line[ep] = 0;
            ep += 1;
        }

        final_empty = (ep > sp && at(line, ep) == 0) as u64;
        sp = ep;
    }

    /* Finally, NULL-terminate the list. */
    if list.used + final_empty >= list.size {
        bdf_list_ensure_(list, list.used + final_empty + 1)?;
    }

    if final_empty != 0 {
        list.field[list.used as usize] = BdfField::Empty;
        list.used += 1;
    }

    list.field[list.used as usize] = BdfField::Null;

    Ok(())
}

/// `NO_SKIP`: this value cannot be stored in a 'char'
const NO_SKIP: i32 = 256;

/// What a line parser returns: an error, or (C's -1) that the line is to
/// be parsed again by the next parser.
enum BdfLineError {
    Error(FtError),
    Redo,
}

impl From<FtError> for BdfLineError {
    fn from(e: FtError) -> Self {
        BdfLineError::Error(e)
    }
}

type BdfLineResult = Result<(), BdfLineError>;

/// Calls the line parser `cb`.
fn call_line_func(
    cb: &mut BdfLineFunc,
    line: &mut [u8],
    linelen: u64,
    lineno: u64,
    p: &mut BdfParse,
) -> BdfLineResult {
    match *cb {
        BdfLineFunc::Start => bdf_parse_start_(line, linelen, lineno, cb, p),
        BdfLineFunc::Properties => bdf_parse_properties_(line, linelen, lineno, cb, p),
        BdfLineFunc::Glyphs => bdf_parse_glyphs_(line, linelen, lineno, cb, p),
        BdfLineFunc::End => bdf_parse_end_(line, linelen, lineno, cb, p),
    }
}

/// `bdf_readstream_`
fn bdf_readstream_(
    stream: &mut FtStreamRec,
    callback: BdfLineFunc,
    client_data: &mut BdfParse,
    lno: &mut u64,
) -> FtResult<()> {
    let mut error: FtResult<()> = Ok(());

    /* initial size and allocation of the input buffer */
    let mut buf_size: u64 = 1024;

    let mut buf = super::super::base::ftmemory::ft_qalloc(buf_size as FtLong)?;

    let mut cb = callback;
    let mut lineno: u64 = 1;
    buf[0] = 0;
    let mut start: isize = 0;
    let mut avail: isize = 0;
    let mut cursor: isize = 0;
    let mut refill = true;
    let mut to_skip: i32 = NO_SKIP;
    let mut bytes: isize = 0; /* make compiler happy */

    loop {
        if refill {
            bytes = stream.try_read(&mut buf[cursor as usize..buf_size as usize]) as isize;
            avail = cursor + bytes;
            cursor = 0;
            refill = false;
        }

        let mut end = start;

        /* should we skip an optional character like \n or \r? */
        if start < avail && buf[start as usize] as i32 == to_skip {
            start += 1;
            to_skip = NO_SKIP;
            continue;
        }

        /* try to find the end of the line */
        while end < avail && buf[end as usize] != b'\n' && buf[end as usize] != b'\r' {
            end += 1;
        }

        /* if we hit the end of the buffer, try shifting its content */
        /* or even resizing it                                       */
        if end >= avail {
            if bytes == 0 {
                /* last line in file doesn't end in \r or \n; */
                /* ignore it then exit                        */
                if lineno == 1 {
                    error = Err(FT_ERR_MISSING_STARTFONT_FIELD);
                }
                break;
            }

            if start == 0 {
                /* this line is definitely too long; try resizing the input */
                /* buffer a bit to handle it.                               */
                if buf_size >= 65536 {
                    /* limit ourselves to 64KByte */
                    if lineno == 1 {
                        return Err(FT_ERR_MISSING_STARTFONT_FIELD);
                    } else {
                        return Err(FT_ERR_INVALID_ARGUMENT);
                    }
                }

                let new_size = buf_size * 2;
                ft_renew_array(&mut buf, new_size as FtLong)?;

                cursor = avail;
                buf_size = new_size;
            } else {
                bytes = avail - start;

                buf.copy_within(start as usize..(start + bytes) as usize, 0);

                cursor = bytes;
                start = 0;
            }
            refill = true;
            continue;
        }

        /* Temporarily NUL-terminate the line. */
        let hold = buf[end as usize];
        buf[end as usize] = 0;

        /* XXX: Use encoding independent value for 0x1A */
        if buf[start as usize] != b'#' && buf[start as usize] != 0x1A && end > start {
            let linelen = (end - start) as u64;
            let line = &mut buf[start as usize..=end as usize];
            let mut r = call_line_func(&mut cb, line, linelen, lineno, client_data);
            /* Redo if we have encountered CHARS without properties. */
            if matches!(r, Err(BdfLineError::Redo)) {
                r = call_line_func(&mut cb, line, linelen, lineno, client_data);
            }
            match r {
                Ok(()) => {}
                Err(BdfLineError::Error(e)) => {
                    error = Err(e);
                    break;
                }
                /* (C's -1 is an error a second time) */
                Err(BdfLineError::Redo) => {
                    error = Err(-1);
                    break;
                }
            }
        }

        lineno += 1;
        buf[end as usize] = hold;
        start = end + 1;

        if hold == b'\n' {
            to_skip = b'\r' as i32;
        } else if hold == b'\r' {
            to_skip = b'\n' as i32;
        } else {
            to_skip = NO_SKIP;
        }
    }

    *lno = lineno;

    error
}

/* XXX: make this work with EBCDIC also */

static A2I: [u8; 128] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

static DDIGITS: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

static HDIGITS: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x03, 0x7E, 0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// `a2i[(int)*s]` (only read for digits)
fn a2i(c: u8) -> u8 {
    A2I.get(c as usize).copied().unwrap_or(0)
}

/// `bdf_atoul_`: Routine to convert a decimal ASCII string to an unsigned
/// long integer.
fn bdf_atoul_(s: Option<&[u8]>) -> u64 {
    let Some(s) = s else {
        return 0;
    };
    if at(s, 0) == 0 {
        return 0;
    }

    let mut v: u64 = 0;
    let mut i = 0;
    while sbitset(&DDIGITS, at(s, i)) {
        if v < (u64::MAX - 9) / 10 {
            v = v * 10 + a2i(s[i]) as u64;
        } else {
            v = u64::MAX;
            break;
        }
        i += 1;
    }

    v
}

/// `bdf_atol_`: Routine to convert a decimal ASCII string to a signed long
/// integer.
fn bdf_atol_(s: Option<&[u8]>) -> i64 {
    let Some(s) = s else {
        return 0;
    };
    if at(s, 0) == 0 {
        return 0;
    }

    /* Check for a minus sign. */
    let mut neg = false;
    let mut i = 0;
    if s[0] == b'-' {
        i += 1;
        neg = true;
    }

    let mut v: i64 = 0;
    while sbitset(&DDIGITS, at(s, i)) {
        if v < (i64::MAX - 9) / 10 {
            v = v * 10 + a2i(s[i]) as i64;
        } else {
            v = i64::MAX;
            break;
        }
        i += 1;
    }

    if !neg {
        v
    } else {
        -v
    }
}

/// `bdf_atous_`: Routine to convert a decimal ASCII string to an unsigned
/// short integer.
fn bdf_atous_(s: Option<&[u8]>) -> u16 {
    let Some(s) = s else {
        return 0;
    };
    if at(s, 0) == 0 {
        return 0;
    }

    let mut v: u16 = 0;
    let mut i = 0;
    while sbitset(&DDIGITS, at(s, i)) {
        if v < (u16::MAX - 9) / 10 {
            v = v * 10 + a2i(s[i]) as u16;
        } else {
            v = u16::MAX;
            break;
        }
        i += 1;
    }

    v
}

/// `bdf_atos_`: Routine to convert a decimal ASCII string to a signed
/// short integer.
fn bdf_atos_(s: Option<&[u8]>) -> i16 {
    let Some(s) = s else {
        return 0;
    };
    if at(s, 0) == 0 {
        return 0;
    }

    /* Check for a minus. */
    let mut neg = false;
    let mut i = 0;
    if s[0] == b'-' {
        i += 1;
        neg = true;
    }

    let mut v: i16 = 0;
    while sbitset(&DDIGITS, at(s, i)) {
        if v < (i16::MAX - 9) / 10 {
            v = v * 10 + a2i(s[i]) as i16;
        } else {
            v = i16::MAX;
            break;
        }
        i += 1;
    }

    if !neg {
        v
    } else {
        -v
    }
}

/// `by_encoding`: Routine to compare two glyphs by encoding so they can be
/// sorted.
fn by_encoding(c1: &BdfGlyph, c2: &BdfGlyph) -> std::cmp::Ordering {
    c1.encoding.cmp(&c2.encoding)
}

/// `bdf_create_property`
fn bdf_create_property(name: &[u8], format: i32, font: &mut BdfFont) -> FtResult<()> {
    /* First check whether the property has        */
    /* already been added or not.  If it has, then */
    /* simply ignore it.                           */
    if font.proptbl.contains_key(name) {
        return Ok(());
    }

    ft_renew_array(&mut font.user_props, (font.nuser_props + 1) as FtLong)?;

    let n = name.len() + 1;
    if n as u64 > FtLong::MAX as u64 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let p = &mut font.user_props[font.nuser_props as usize];
    p.name = name.to_vec();

    p.format = format;
    p.builtin = 0;
    p.atom = None; /* nothing is ever stored here */

    let n = NUM_BDF_PROPERTIES_ + font.nuser_props as usize;

    font.proptbl.insert(name.to_vec(), n);

    font.nuser_props += 1;

    Ok(())
}

/// `bdf_get_property`
fn bdf_get_property(name: &[u8], font: &BdfFont) -> Option<BdfProperty> {
    if name.is_empty() {
        return None;
    }

    let propid = *font.proptbl.get(name)?;

    if propid >= NUM_BDF_PROPERTIES_ {
        return font.user_props.get(propid - NUM_BDF_PROPERTIES_).cloned();
    }

    Some(bdf_builtin_property(propid))
}

/*************************************************************************
 *
 * BDF font file parsing flags and functions.
 *
 */

/* Parse flags. */

const BDF_START_: u64 = 0x0001;
const BDF_FONT_NAME_: u64 = 0x0002;
const BDF_SIZE_: u64 = 0x0004;
const BDF_FONT_BBX_: u64 = 0x0008;
const BDF_PROPS_: u64 = 0x0010;
const BDF_GLYPHS_: u64 = 0x0020;
const BDF_GLYPH_: u64 = 0x0040;
const BDF_ENCODING_: u64 = 0x0080;
const BDF_SWIDTH_: u64 = 0x0100;
const BDF_DWIDTH_: u64 = 0x0200;
const BDF_BBX_: u64 = 0x0400;
const BDF_BITMAP_: u64 = 0x0800;

const BDF_SWIDTH_ADJ_: u64 = 0x1000;

const BDF_GLYPH_BITS_: u64 =
    BDF_GLYPH_ | BDF_ENCODING_ | BDF_SWIDTH_ | BDF_DWIDTH_ | BDF_BBX_ | BDF_BITMAP_;

const BDF_GLYPH_WIDTH_CHECK_: u64 = 0x40000000;
const BDF_GLYPH_HEIGHT_CHECK_: u64 = 0x80000000;

/// `bdf_add_comment_`
fn bdf_add_comment_(font: &mut BdfFont, comment: &[u8], len: u64) -> FtResult<()> {
    ft_renew_array(&mut font.comments, (font.comments_len + len + 1) as FtLong)?;

    let cp = font.comments_len as usize;

    for i in 0..len as usize {
        font.comments[cp + i] = at(comment, i);
    }
    font.comments[cp + len as usize] = b'\0';

    font.comments_len += len + 1;

    Ok(())
}

/// `bdf_set_default_spacing_`: Set the spacing from the font name if it
/// exists, or set it to the default specified in the options.
fn bdf_set_default_spacing_(font: &mut BdfFont, opts: &BdfOptions, _lineno: u64) -> FtResult<()> {
    let fname = match font.name.as_ref() {
        Some(n) if at(n, 0) != 0 => cstr(n).to_vec(),
        _ => return Err(FT_ERR_INVALID_ARGUMENT),
    };

    let mut list = BdfListRec::default();
    bdf_list_init_(&mut list);

    font.spacing = opts.font_spacing;

    let len = fname.len() + 1;
    /* Limit ourselves to 256 characters in the font name. */
    if len >= 256 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let mut name = [0u8; 256];
    name[..len - 1].copy_from_slice(&fname);

    let error = bdf_list_split_(&mut list, b"-", &mut name, len as u64);
    if error.is_ok() && list.used == 15 {
        match field_str(&name, list.get(11))
            .map(|s| at(s, 0))
            .unwrap_or(0)
        {
            b'C' | b'c' => font.spacing = BDF_CHARCELL,
            b'M' | b'm' => font.spacing = BDF_MONOWIDTH,
            b'P' | b'p' => font.spacing = BDF_PROPORTIONAL,
            _ => {}
        }
    }

    /* Fail: */
    bdf_list_done_(&mut list);

    error
}

/// `bdf_is_atom_`: Determine whether the property is an atom or not.  If
/// it is, then clean it up so the double quotes are removed if they exist.
/// Returns the offsets of the name and the value in the line.
fn bdf_is_atom_(line: &mut [u8], linelen: u64, font: &BdfFont) -> Option<(usize, usize)> {
    let mut sp = 0usize;
    let mut ep = 0usize;

    while at(line, ep) != 0 && at(line, ep) != b' ' && at(line, ep) != b'\t' {
        ep += 1;
    }

    let hold = at(line, ep);
    line[ep] = b'\0';

    let p = bdf_get_property(cstr(&line[sp..]), font);

    /* If the property exists and is not an atom, just return here. */
    if let Some(p) = p {
        if p.format != BDF_ATOM {
            line[ep] = hold; /* Undo NUL-termination. */
            return None;
        }
    }

    let name = sp;

    /* The property is an atom.  Trim all leading and trailing whitespace */
    /* and double quotes for the atom value.                              */
    sp = ep;
    let mut ep = linelen as usize;

    /* Trim the leading whitespace if it exists. */
    if sp < ep {
        loop {
            sp += 1;
            if !(at(line, sp) == b' ' || at(line, sp) == b'\t') {
                break;
            }
        }
    }

    /* Trim the leading double quote if it exists. */
    if at(line, sp) == b'"' {
        sp += 1;
    }

    let value = sp;

    /* Trim the trailing whitespace if it exists. */
    if sp < ep {
        loop {
            line[ep] = b'\0';
            ep -= 1;
            if !(at(line, ep) == b' ' || at(line, ep) == b'\t') {
                break;
            }
        }
    }

    /* Trim the trailing double quote if it exists. */
    if at(line, ep) == b'"' {
        line[ep] = b'\0';
    }

    Some((name, value))
}

/// `bdf_add_property_`
fn bdf_add_property_(
    font: &mut BdfFont,
    name: &[u8],
    value: Option<&[u8]>,
    _lineno: u64,
) -> FtResult<()> {
    /* First, check whether the property already exists in the font. */
    if let Some(&propid) = font.internal.as_ref().and_then(|h| h.get(name)) {
        /* The property already exists in the font, so simply replace */
        /* the value of the property with the current value.          */
        let fp = &mut font.props[propid];

        match fp.format {
            BDF_ATOM => {
                /* Delete the current atom if it exists. */
                fp.atom = None;

                if let Some(v) = value {
                    if at(v, 0) != 0 {
                        fp.atom = Some(v.to_vec());
                    }
                }
            }

            BDF_INTEGER => {
                fp.num = bdf_atol_(value) as u64;
            }

            BDF_CARDINAL => {
                fp.num = bdf_atoul_(value);
            }

            _ => {}
        }

        return Ok(());
    }

    /* See whether this property type exists yet or not. */
    /* If not, create it.                                */
    let mut propid = font.proptbl.get(name).copied();
    if propid.is_none() {
        bdf_create_property(name, BDF_ATOM, font)?;
        propid = font.proptbl.get(name).copied();
    }
    let propid = propid.unwrap_or(0);

    /* Allocate another property if this is overflowing. */
    if font.props_used == font.props_size {
        ft_renew_array(&mut font.props, (font.props_size + 1) as FtLong)?;

        font.props_size += 1;
    }

    let prop = if propid >= NUM_BDF_PROPERTIES_ {
        font.user_props
            .get(propid - NUM_BDF_PROPERTIES_)
            .cloned()
            .unwrap_or_default()
    } else {
        bdf_builtin_property(propid)
    };

    let props_used = font.props_used as usize;
    let fp = &mut font.props[props_used];

    fp.name = prop.name.clone();
    fp.format = prop.format;
    fp.builtin = prop.builtin;

    match prop.format {
        BDF_ATOM => {
            fp.atom = None;
            if let Some(v) = value {
                if at(v, 0) != 0 {
                    fp.atom = Some(v.to_vec());
                }
            }
        }

        BDF_INTEGER => {
            fp.num = bdf_atol_(value) as u64;
        }

        BDF_CARDINAL => {
            fp.num = bdf_atoul_(value);
        }

        _ => {}
    }

    /* If the property happens to be a comment, then it doesn't need */
    /* to be added to the internal hash table.                       */
    if bdf_strncmp(name, b"COMMENT", 7) {
        /* Add the property to the font property table. */
        let fname = fp.name.clone();
        if let Some(h) = font.internal.as_mut() {
            if h.try_reserve(1).is_err() {
                return Err(FT_ERR_OUT_OF_MEMORY);
            }
            h.insert(fname, props_used);
        }
    }

    font.props_used += 1;

    /* Some special cases need to be handled here.  The DEFAULT_CHAR       */
    /* property needs to be located if it exists in the property list, the */
    /* FONT_ASCENT and FONT_DESCENT need to be assigned if they are        */
    /* present, and the SPACING property should override the default       */
    /* spacing.                                                            */
    let fp = &font.props[props_used];
    if !bdf_strncmp(name, b"DEFAULT_CHAR", 12) {
        font.default_char = fp.value_ul();
    } else if !bdf_strncmp(name, b"FONT_ASCENT", 11) {
        font.font_ascent = fp.value_l();
    } else if !bdf_strncmp(name, b"FONT_DESCENT", 12) {
        font.font_descent = fp.value_l();
    } else if !bdf_strncmp(name, b"SPACING", 7) {
        let Some(atom) = fp.atom.as_ref() else {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        };

        if atom[0] == b'p' || atom[0] == b'P' {
            font.spacing = BDF_PROPORTIONAL;
        } else if atom[0] == b'm' || atom[0] == b'M' {
            font.spacing = BDF_MONOWIDTH;
        } else if atom[0] == b'c' || atom[0] == b'C' {
            font.spacing = BDF_CHARCELL;
        }
    }

    Ok(())
}

static NIBBLE_MASK: [u8; 8] = [0xFF, 0x80, 0xC0, 0xE0, 0xF0, 0xF8, 0xFC, 0xFE];

/// `bdf_parse_end_`: a no-op; we ignore everything after `ENDFONT'
fn bdf_parse_end_(
    _line: &mut [u8],
    _linelen: u64,
    _lineno: u64,
    _call_data: &mut BdfLineFunc,
    _client_data: &mut BdfParse,
) -> BdfLineResult {
    Ok(())
}

/// The glyph being constructed (`glyph`).
fn current_glyph(p: &mut BdfParse) -> Option<&mut BdfGlyph> {
    let font = p.font.as_mut()?;
    /* Point at the glyph being constructed. */
    if p.glyph_enc == -1 {
        let i = font.unencoded_used.wrapping_sub(1) as usize;
        font.unencoded.get_mut(i)
    } else {
        let i = font.glyphs_used.wrapping_sub(1) as usize;
        font.glyphs.get_mut(i)
    }
}

/// `bdf_parse_glyphs_`: Actually parse the glyph info and bitmaps.
fn bdf_parse_glyphs_(
    line: &mut [u8],
    linelen: u64,
    lineno: u64,
    call_data: &mut BdfLineFunc,
    client_data: &mut BdfParse,
) -> BdfLineResult {
    let p = client_data;
    let r = bdf_parse_glyphs_body(line, linelen, lineno, call_data, p);

    /* Exit: */
    if r.is_err() && (p.flags & BDF_GLYPH_) != 0 {
        p.glyph_name = None;
    }

    r
}

fn bdf_parse_glyphs_body(
    line: &mut [u8],
    mut linelen: u64,
    _lineno: u64,
    next: &mut BdfLineFunc,
    p: &mut BdfParse,
) -> BdfLineResult {
    let Some(font) = p.font.as_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT.into());
    };

    /* Check for a comment. */
    if !bdf_strncmp(line, b"COMMENT", 7) {
        if p.opts.keep_comments != 0 {
            linelen -= 7;

            let mut s = 7;
            if at(line, s) != 0 {
                s += 1;
                linelen -= 1;
            }
            bdf_add_comment_(font, &line[s..], linelen)?;
        }
        return Ok(());
    }

    /* The very first thing expected is the number of glyphs. */
    if p.flags & BDF_GLYPHS_ == 0 {
        if bdf_strncmp(line, b"CHARS", 5) {
            return Err(FT_ERR_MISSING_CHARS_FIELD.into());
        }

        bdf_list_split_(&mut p.list, b" +", line, linelen)?;
        p.cnt = bdf_atoul_(field_str(line, p.list.get(1)));
        font.glyphs_size = p.cnt;

        /* We need at least 20 bytes per glyph. */
        if p.cnt > p.size / 20 {
            p.cnt = p.size / 20;
            font.glyphs_size = p.cnt;
        }

        /* Make sure the number of glyphs is non-zero. */
        if p.cnt == 0 {
            font.glyphs_size = 64;
        }

        /* Limit ourselves to 1,114,112 glyphs in the font (this is the */
        /* number of code points available in Unicode).                 */
        if p.cnt >= 0x110000 {
            return Err(FT_ERR_INVALID_ARGUMENT.into());
        }

        font.glyphs = ft_new_array(font.glyphs_size as FtLong)?;

        p.flags |= BDF_GLYPHS_;

        return Ok(());
    }

    /* Check for the ENDFONT field. */
    if !bdf_strncmp(line, b"ENDFONT", 7) {
        if p.flags & BDF_GLYPH_BITS_ != 0 {
            /* Missing ENDCHAR field. */
            return Err(FT_ERR_CORRUPTED_FONT_GLYPHS.into());
        }

        /* Sort the glyphs by encoding. */
        /* (`ft_qsort'; glibc's is a merge sort, which is stable) */
        let used = (font.glyphs_used as usize).min(font.glyphs.len());
        font.glyphs[..used].sort_by(by_encoding);

        p.flags &= !BDF_START_;
        *next = BdfLineFunc::End;

        return Ok(());
    }

    /* Check for the ENDCHAR field. */
    if !bdf_strncmp(line, b"ENDCHAR", 7) {
        p.glyph_enc = 0;
        p.flags &= !BDF_GLYPH_BITS_;

        return Ok(());
    }

    /* Check whether a glyph is being scanned but should be */
    /* ignored because it is an unencoded glyph.            */
    if (p.flags & BDF_GLYPH_) != 0 && p.glyph_enc == -1 && p.opts.keep_unencoded == 0 {
        return Ok(());
    }

    /* Check for the STARTCHAR field. */
    if !bdf_strncmp(line, b"STARTCHAR", 9) {
        if p.flags & BDF_GLYPH_BITS_ != 0 {
            /* Missing ENDCHAR field. */
            return Err(FT_ERR_MISSING_STARTCHAR_FIELD.into());
        }

        /* Set the character name in the parse info first until the */
        /* encoding can be checked for an unencoded character.      */
        p.glyph_name = None;

        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        bdf_list_shift_(&mut p.list, 1);

        let mut slen = 0;
        let s = bdf_list_join_(&p.list, line, b' ', &mut slen);

        let Some(s) = field_str(line, s) else {
            return Err(FT_ERR_INVALID_FILE_FORMAT.into());
        };

        let mut name = super::super::base::ftmemory::ft_qalloc(slen as FtLong + 1)?;
        let n = (slen as usize).min(s.len());
        name[..n].copy_from_slice(&s[..n]);
        p.glyph_name = Some(name);

        p.flags |= BDF_GLYPH_;

        return Ok(());
    }

    /* Check for the ENCODING field. */
    if !bdf_strncmp(line, b"ENCODING", 8) {
        if p.flags & BDF_GLYPH_ == 0 {
            /* Missing STARTCHAR field. */
            return Err(FT_ERR_MISSING_STARTCHAR_FIELD.into());
        }

        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        p.glyph_enc = bdf_atol_(field_str(line, p.list.get(1)));

        /* Normalize negative encoding values.  The specification only */
        /* allows -1, but we can be more generous here.                */
        if p.glyph_enc < -1 {
            p.glyph_enc = -1;
        }

        /* Check for alternative encoding format. */
        if p.glyph_enc == -1 && p.list.used > 2 {
            p.glyph_enc = bdf_atol_(field_str(line, p.list.get(2)));
        }

        if p.glyph_enc < -1 || p.glyph_enc >= 0x110000 {
            p.glyph_enc = -1;
        }

        if p.glyph_enc >= 0 {
            /* Make sure there are enough glyphs allocated in case the */
            /* number of characters happen to be wrong.                */
            if font.glyphs_used == font.glyphs_size {
                ft_renew_array(&mut font.glyphs, (font.glyphs_size + 64) as FtLong)?;

                font.glyphs_size += 64;
            }

            let glyph = &mut font.glyphs[font.glyphs_used as usize];
            font.glyphs_used += 1;
            glyph.name = p.glyph_name.take();
            glyph.encoding = p.glyph_enc as u64;

            /* Reset the initial glyph info. */
        } else {
            /* Unencoded glyph.  Check whether it should */
            /* be added or not.                          */
            if p.opts.keep_unencoded != 0 {
                /* Allocate the next unencoded glyph. */
                if font.unencoded_used == font.unencoded_size {
                    ft_renew_array(&mut font.unencoded, (font.unencoded_size + 4) as FtLong)?;

                    font.unencoded_size += 4;
                }

                let glyph = &mut font.unencoded[font.unencoded_used as usize];
                glyph.name = p.glyph_name.take();
                glyph.encoding = font.unencoded_used;
                font.unencoded_used += 1;

                /* Reset the initial glyph info. */
            } else {
                /* Free up the glyph name if the unencoded shouldn't be */
                /* kept.                                                */
                p.glyph_name = None;
            }
        }

        /* Clear the flags that might be added when width and height are */
        /* checked for consistency.                                      */
        p.flags &= !(BDF_GLYPH_WIDTH_CHECK_ | BDF_GLYPH_HEIGHT_CHECK_);

        p.flags |= BDF_ENCODING_;

        return Ok(());
    }

    if p.flags & BDF_ENCODING_ == 0 {
        /* Missing_Encoding: */
        /* Missing ENCODING field. */
        return Err(FT_ERR_MISSING_ENCODING_FIELD.into());
    }

    let bpp = font.bpp;
    let point_size = font.point_size;
    let resolution_x = font.resolution_x;
    let correct_metrics = p.opts.correct_metrics;

    /* Check whether a bitmap is being constructed. */
    if p.flags & BDF_BITMAP_ != 0 {
        let row = p.row;
        let mut flags = p.flags;
        let Some(glyph) = current_glyph(p) else {
            return Ok(());
        };

        /* If there are more rows than are specified in the glyph metrics, */
        /* ignore the remaining lines.                                     */
        if row >= glyph.bbx.height as u64 {
            if flags & BDF_GLYPH_HEIGHT_CHECK_ == 0 {
                flags |= BDF_GLYPH_HEIGHT_CHECK_;
            }
            p.flags = flags;

            return Ok(());
        }

        /* Only collect the number of nibbles indicated by the glyph     */
        /* metrics.  If there are more columns, they are simply ignored. */
        let nibbles = glyph.bpr << 1;
        let mut bp = (row * glyph.bpr) as usize;

        let mut i: u64 = 0;
        while i < nibbles {
            let c = at(line, i as usize);
            if !sbitset(&HDIGITS, c) {
                break;
            }
            if let Some(b) = glyph.bitmap.get_mut(bp) {
                *b = (*b << 4).wrapping_add(a2i(c));
            }
            if i + 1 < nibbles && (i & 1) != 0 {
                bp += 1;
                if let Some(b) = glyph.bitmap.get_mut(bp) {
                    *b = 0;
                }
            }
            i += 1;
        }

        /* If any line has not enough columns,            */
        /* indicate they have been padded with zero bits. */
        if i < nibbles && flags & BDF_GLYPH_WIDTH_CHECK_ == 0 {
            flags |= BDF_GLYPH_WIDTH_CHECK_;
        }

        /* Remove possible garbage at the right. */
        let mask_index = ((glyph.bbx.width as u32 * bpp as u32) & 7) as usize;
        if glyph.bbx.width != 0 {
            if let Some(b) = glyph.bitmap.get_mut(bp) {
                *b &= NIBBLE_MASK[mask_index];
            }
        }

        /* If any line has extra columns, indicate they have been removed. */
        if i == nibbles
            && sbitset(&HDIGITS, at(line, nibbles as usize))
            && flags & BDF_GLYPH_WIDTH_CHECK_ == 0
        {
            flags |= BDF_GLYPH_WIDTH_CHECK_;
        }

        p.flags = flags;
        p.row += 1;
        return Ok(());
    }

    /* Expect the SWIDTH (scalable width) field next. */
    if !bdf_strncmp(line, b"SWIDTH", 6) {
        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        let v = bdf_atous_(field_str(line, p.list.get(1)));
        if let Some(glyph) = current_glyph(p) {
            glyph.swidth = v;
        }
        p.flags |= BDF_SWIDTH_;

        return Ok(());
    }

    /* Expect the DWIDTH (device width) field next. */
    if !bdf_strncmp(line, b"DWIDTH", 6) {
        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        let v = bdf_atous_(field_str(line, p.list.get(1)));
        let flags = p.flags;
        if let Some(glyph) = current_glyph(p) {
            glyph.dwidth = v;

            if flags & BDF_SWIDTH_ == 0 {
                /* Missing SWIDTH field.  Emit an auto correction message and set */
                /* the scalable width from the device width.                      */
                glyph.swidth = ft_mul_div(
                    glyph.dwidth as FtLong,
                    72000,
                    point_size.wrapping_mul(resolution_x) as FtLong,
                ) as u16;
            }
        }

        p.flags |= BDF_DWIDTH_;
        return Ok(());
    }

    /* Expect the BBX field next. */
    if !bdf_strncmp(line, b"BBX", 3) {
        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        let width = bdf_atous_(field_str(line, p.list.get(1)));
        let height = bdf_atous_(field_str(line, p.list.get(2)));
        let x_offset = bdf_atos_(field_str(line, p.list.get(3)));
        let y_offset = bdf_atos_(field_str(line, p.list.get(4)));

        let flags = p.flags;
        let (mut maxas, mut maxds, mut maxrb, mut minlb, mut maxlb) =
            (p.maxas, p.maxds, p.maxrb, p.minlb, p.maxlb);
        let mut rbearing = p.rbearing;
        let mut swidth_adj = false;
        if let Some(glyph) = current_glyph(p) {
            glyph.bbx.width = width;
            glyph.bbx.height = height;
            glyph.bbx.x_offset = x_offset;
            glyph.bbx.y_offset = y_offset;

            /* Generate the ascent and descent of the character. */
            glyph.bbx.ascent = (glyph.bbx.height as i32 + glyph.bbx.y_offset as i32) as i16;
            glyph.bbx.descent = (-(glyph.bbx.y_offset as i32)) as i16;

            /* Determine the overall font bounding box as the characters are */
            /* loaded so corrections can be done later if indicated.         */
            maxas = glyph.bbx.ascent.max(maxas);
            maxds = glyph.bbx.descent.max(maxds);

            rbearing = (glyph.bbx.width as i32 + glyph.bbx.x_offset as i32) as i16;

            maxrb = rbearing.max(maxrb);
            minlb = glyph.bbx.x_offset.min(minlb);
            maxlb = glyph.bbx.x_offset.max(maxlb);

            if flags & BDF_DWIDTH_ == 0 {
                /* Missing DWIDTH field.  Emit an auto correction message and set */
                /* the device width to the glyph width.                           */
                glyph.dwidth = glyph.bbx.width;
            }

            /* If the BDF_CORRECT_METRICS flag is set, then adjust the SWIDTH */
            /* value if necessary.                                            */
            if correct_metrics != 0 {
                /* Determine the point size of the glyph. */
                let sw = ft_mul_div(
                    glyph.dwidth as FtLong,
                    72000,
                    point_size.wrapping_mul(resolution_x) as FtLong,
                ) as u16;

                if sw != glyph.swidth {
                    glyph.swidth = sw;

                    swidth_adj = true;
                }
            }
        }
        p.maxas = maxas;
        p.maxds = maxds;
        p.rbearing = rbearing;
        p.maxrb = maxrb;
        p.minlb = minlb;
        p.maxlb = maxlb;
        if swidth_adj {
            p.flags |= BDF_SWIDTH_ADJ_;
        }

        p.flags |= BDF_BBX_;
        return Ok(());
    }

    /* And finally, gather up the bitmap. */
    if !bdf_strncmp(line, b"BITMAP", 6) {
        if p.flags & BDF_BBX_ == 0 {
            /* Missing BBX field. */
            return Err(FT_ERR_MISSING_BBX_FIELD.into());
        }

        if let Some(glyph) = current_glyph(p) {
            /* Allocate enough space for the bitmap. */
            glyph.bpr = ((glyph.bbx.width as i32 * bpp as i32 + 7) >> 3) as u64;

            let bitmap_size = glyph.bpr.wrapping_mul(glyph.bbx.height as u64);
            if glyph.bpr > 0xFFFF || bitmap_size > 0xFFFF {
                return Err(FT_ERR_BBX_TOO_BIG.into());
            } else {
                glyph.bytes = bitmap_size as u16;
            }

            glyph.bitmap = super::super::base::ftmemory::ft_alloc(glyph.bytes as FtLong)?;
        }

        p.row = 0;
        p.flags |= BDF_BITMAP_;

        return Ok(());
    }

    Err(FT_ERR_INVALID_FILE_FORMAT.into())
}

/// `bdf_parse_properties_`: Load the font properties.
fn bdf_parse_properties_(
    line: &mut [u8],
    linelen: u64,
    lineno: u64,
    next: &mut BdfLineFunc,
    p: &mut BdfParse,
) -> BdfLineResult {
    let Some(font) = p.font.as_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT.into());
    };

    /* Check for the end of the properties. */
    if !bdf_strncmp(line, b"ENDPROPERTIES", 13) {
        /* If the FONT_ASCENT or FONT_DESCENT properties have not been      */
        /* encountered yet, then make sure they are added as properties and */
        /* make sure they are set from the font bounding box info.          */
        /*                                                                  */
        /* This is *always* done regardless of the options, because X11     */
        /* requires these two fields to compile fonts.                      */
        if bdf_get_font_property(font, b"FONT_ASCENT").is_none() {
            font.font_ascent = font.bbx.ascent as i64;
            let nbuf = format!("{}", font.bbx.ascent);
            bdf_add_property_(font, b"FONT_ASCENT", Some(nbuf.as_bytes()), lineno)?;
        }

        if bdf_get_font_property(font, b"FONT_DESCENT").is_none() {
            font.font_descent = font.bbx.descent as i64;
            let nbuf = format!("{}", font.bbx.descent);
            bdf_add_property_(font, b"FONT_DESCENT", Some(nbuf.as_bytes()), lineno)?;
        }

        p.flags &= !BDF_PROPS_;
        *next = BdfLineFunc::Glyphs;

        return Ok(());
    }

    /* Ignore the _XFREE86_GLYPH_RANGES properties. */
    if !bdf_strncmp(line, b"_XFREE86_GLYPH_RANGES", 21) {
        return Ok(());
    }

    /* Handle COMMENT fields and properties in a special way to preserve */
    /* the spacing.                                                      */
    if !bdf_strncmp(line, b"COMMENT", 7) {
        let name = 0;
        let mut value = 7;
        if at(line, value) != 0 {
            line[value] = 0;
            value += 1;
        }
        let name = cstr(&line[name..]).to_vec();
        let value = cstr(&line[value..]).to_vec();
        bdf_add_property_(font, &name, Some(&value), lineno)?;
    } else if let Some((name, value)) = bdf_is_atom_(line, linelen, font) {
        let name = cstr(&line[name..]).to_vec();
        let value = cstr(line.get(value..).unwrap_or(&[])).to_vec();
        bdf_add_property_(font, &name, Some(&value), lineno)?;
    } else {
        bdf_list_split_(&mut p.list, b" +", line, linelen)?;
        let name = field_str(line, p.list.get(0)).map(|s| s.to_vec());

        bdf_list_shift_(&mut p.list, 1);
        let mut vlen = 0;
        let value = bdf_list_join_(&p.list, line, b' ', &mut vlen);
        let value = field_str(line, value).map(|s| s.to_vec());

        bdf_add_property_(
            font,
            name.as_deref().unwrap_or(b""),
            value.as_deref(),
            lineno,
        )?;
    }

    Ok(())
}

/// `bdf_parse_start_`: Load the font header.
fn bdf_parse_start_(
    line: &mut [u8],
    mut linelen: u64,
    lineno: u64,
    next: &mut BdfLineFunc,
    p: &mut BdfParse,
) -> BdfLineResult {
    /* Check for a comment.  This is done to handle those fonts that have */
    /* comments before the STARTFONT line for some reason.                */
    if !bdf_strncmp(line, b"COMMENT", 7) {
        if p.opts.keep_comments != 0 {
            if let Some(font) = p.font.as_mut() {
                linelen -= 7;

                let mut s = 7;
                if at(line, s) != 0 {
                    s += 1;
                    linelen -= 1;
                }
                bdf_add_comment_(font, &line[s..], linelen)?;
            }
        }
        return Ok(());
    }

    if p.flags & BDF_START_ == 0 {
        if bdf_strncmp(line, b"STARTFONT", 9) {
            /* we don't emit an error message since this code gets */
            /* explicitly caught one level higher                  */
            return Err(FT_ERR_MISSING_STARTFONT_FIELD.into());
        }

        p.flags = BDF_START_;
        p.font = None;

        let mut font = Box::<BdfFont>::default();

        {
            /* setup */
            if font.proptbl.try_reserve(NUM_BDF_PROPERTIES_).is_err() {
                return Err(FT_ERR_OUT_OF_MEMORY.into());
            }
            for (i, (name, _)) in BDF_PROPERTIES_.iter().enumerate() {
                font.proptbl.insert(name.as_bytes().to_vec(), i);
            }
        }

        font.internal = Some(HashMap::new());
        font.spacing = p.opts.font_spacing;
        font.default_char = !0;

        p.font = Some(font);

        return Ok(());
    }

    let Some(font) = p.font.as_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT.into());
    };

    /* Check for the start of the properties. */
    if !bdf_strncmp(line, b"STARTPROPERTIES", 15) {
        if p.flags & BDF_FONT_BBX_ == 0 {
            /* Missing the FONTBOUNDINGBOX field. */
            return Err(FT_ERR_MISSING_FONTBOUNDINGBOX_FIELD.into());
        }

        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        /* at this point, `p->font' can't be NULL */
        p.cnt = bdf_atoul_(field_str(line, p.list.get(1)));
        font.props_size = p.cnt;
        /* We need at least 4 bytes per property. */
        if p.cnt > p.size / 4 {
            font.props_size = 0;

            return Err(FT_ERR_INVALID_ARGUMENT.into());
        }

        match ft_new_array(p.cnt as FtLong) {
            Ok(v) => font.props = v,
            Err(e) => {
                font.props_size = 0;
                return Err(e.into());
            }
        }

        p.flags |= BDF_PROPS_;
        *next = BdfLineFunc::Properties;

        return Ok(());
    }

    /* Check for the FONTBOUNDINGBOX field. */
    if !bdf_strncmp(line, b"FONTBOUNDINGBOX", 15) {
        if p.flags & BDF_SIZE_ == 0 {
            /* Missing the SIZE field. */
            return Err(FT_ERR_MISSING_SIZE_FIELD.into());
        }

        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        font.bbx.width = bdf_atous_(field_str(line, p.list.get(1)));
        font.bbx.height = bdf_atous_(field_str(line, p.list.get(2)));

        font.bbx.x_offset = bdf_atos_(field_str(line, p.list.get(3)));
        font.bbx.y_offset = bdf_atos_(field_str(line, p.list.get(4)));

        font.bbx.ascent = (font.bbx.height as i32 + font.bbx.y_offset as i32) as i16;

        font.bbx.descent = (-(font.bbx.y_offset as i32)) as i16;

        p.flags |= BDF_FONT_BBX_;

        return Ok(());
    }

    /* The next thing to check for is the FONT field. */
    if !bdf_strncmp(line, b"FONT", 4) {
        bdf_list_split_(&mut p.list, b" +", line, linelen)?;
        bdf_list_shift_(&mut p.list, 1);

        let mut slen = 0;
        let s = bdf_list_join_(&p.list, line, b' ', &mut slen);

        let Some(s) = field_str(line, s) else {
            return Err(FT_ERR_INVALID_FILE_FORMAT.into());
        };

        /* Allowing multiple `FONT' lines (which is invalid) doesn't hurt... */
        font.name = None;

        let mut name = super::super::base::ftmemory::ft_qalloc(slen as FtLong + 1)?;
        let n = (slen as usize).min(s.len());
        name[..n].copy_from_slice(&s[..n]);
        font.name = Some(name);

        /* If the font name is an XLFD name, set the spacing to the one in  */
        /* the font name.  If there is no spacing fall back on the default. */
        bdf_set_default_spacing_(font, &p.opts, lineno)?;

        p.flags |= BDF_FONT_NAME_;

        return Ok(());
    }

    /* Check for the SIZE field. */
    if !bdf_strncmp(line, b"SIZE", 4) {
        if p.flags & BDF_FONT_NAME_ == 0 {
            /* Missing the FONT field. */
            return Err(FT_ERR_MISSING_FONT_FIELD.into());
        }

        bdf_list_split_(&mut p.list, b" +", line, linelen)?;

        font.point_size = bdf_atoul_(field_str(line, p.list.get(1)));
        font.resolution_x = bdf_atoul_(field_str(line, p.list.get(2)));
        font.resolution_y = bdf_atoul_(field_str(line, p.list.get(3)));

        /* Check for the bits per pixel field. */
        if p.list.used == 5 {
            let bpp = bdf_atous_(field_str(line, p.list.get(4)));

            /* Only values 1, 2, 4, 8 are allowed for greymap fonts. */
            if bpp > 4 {
                font.bpp = 8;
            } else if bpp > 2 {
                font.bpp = 4;
            } else if bpp > 1 {
                font.bpp = 2;
            } else {
                font.bpp = 1;
            }
        } else {
            font.bpp = 1;
        }

        p.flags |= BDF_SIZE_;

        return Ok(());
    }

    /* Check for the CHARS field -- font properties are optional */
    if !bdf_strncmp(line, b"CHARS", 5) {
        if p.flags & BDF_FONT_BBX_ == 0 {
            /* Missing the FONTBOUNDINGBOX field. */
            return Err(FT_ERR_MISSING_FONTBOUNDINGBOX_FIELD.into());
        }

        /* Add the two standard X11 properties which are required */
        /* for compiling fonts.                                   */
        font.font_ascent = font.bbx.ascent as i64;
        let nbuf = format!("{}", font.bbx.ascent);
        debug_assert!(nbuf.len() < BUFSIZE);
        bdf_add_property_(font, b"FONT_ASCENT", Some(nbuf.as_bytes()), lineno)?;

        font.font_descent = font.bbx.descent as i64;
        let nbuf = format!("{}", font.bbx.descent);
        bdf_add_property_(font, b"FONT_DESCENT", Some(nbuf.as_bytes()), lineno)?;

        *next = BdfLineFunc::Glyphs;

        /* A special return value. */
        return Err(BdfLineError::Redo);
    }

    Err(FT_ERR_INVALID_FILE_FORMAT.into())
}

/*************************************************************************
 *
 * API.
 *
 */

/// `bdf_load_font`
pub fn bdf_load_font(
    stream: &mut FtStreamRec,
    opts: Option<&BdfOptions>,
) -> FtResult<Box<BdfFont>> {
    let mut lineno: u64 = 0; /* make compiler happy */

    let mut p = Box::new(BdfParse {
        flags: 0,
        cnt: 0,
        row: 0,
        minlb: 32767,
        maxlb: 0,
        maxrb: 0,
        maxas: 0,
        maxds: 0,
        rbearing: 0,
        glyph_name: None,
        glyph_enc: 0,
        font: None,
        opts: *opts.unwrap_or(&BDF_OPTS_),
        list: BdfListRec::default(),
        size: stream.size,
    });

    bdf_list_init_(&mut p.list);

    let mut error = bdf_readstream_(stream, BdfLineFunc::Start, &mut p, &mut lineno);

    if error.is_ok() {
        if let Some(font) = p.font.as_mut() {
            /* If the font is not proportional, set the font's monowidth */
            /* field to the width of the font bounding box.              */

            if font.spacing != BDF_PROPORTIONAL {
                font.monowidth = font.bbx.width;
            }

            /* If the number of glyphs loaded is not that of the original count, */
            /* indicate the difference.                                          */

            /* Once the font has been loaded, adjust the overall font metrics if */
            /* necessary.                                                        */
            if p.opts.correct_metrics != 0 && (font.glyphs_used > 0 || font.unencoded_used > 0) {
                if p.maxrb as i32 - p.minlb as i32 != font.bbx.width as i32 {
                    font.bbx.width = (p.maxrb as i32 - p.minlb as i32) as u16;
                }

                if font.bbx.x_offset != p.minlb {
                    font.bbx.x_offset = p.minlb;
                }

                if font.bbx.ascent != p.maxas {
                    font.bbx.ascent = p.maxas;
                }

                if font.bbx.descent != p.maxds {
                    font.bbx.descent = p.maxds;
                    font.bbx.y_offset = (-(p.maxds as i32)) as i16;
                }

                if p.maxas as i32 + p.maxds as i32 != font.bbx.height as i32 {
                    font.bbx.height = (p.maxas as i32 + p.maxds as i32) as u16;
                }
            }
        }

        if p.flags & BDF_START_ != 0 {
            /* The ENDFONT field was never reached or did not exist. */
            if p.flags & BDF_GLYPHS_ == 0 {
                /* Error happened while parsing header. */
                error = Err(FT_ERR_CORRUPTED_FONT_HEADER);
            } else {
                /* Error happened when parsing glyphs. */
                error = Err(FT_ERR_CORRUPTED_FONT_GLYPHS);
            }
        }
    }

    if let Err(e) = error {
        /* Fail: */
        bdf_free_font(p.font.take());

        /* Exit: */
        bdf_list_done_(&mut p.list);
        p.glyph_name = None;
        return Err(e);
    }

    let font = p.font.take();

    /* Exit: */
    bdf_list_done_(&mut p.list);
    p.glyph_name = None;

    match font {
        Some(font) => Ok(font),
        None => Err(FT_ERR_INVALID_FILE_FORMAT),
    }
}

/// `bdf_free_font` (the font's data goes with it)
pub fn bdf_free_font(_font: Option<Box<BdfFont>>) {}

/// `bdf_get_font_property`
pub fn bdf_get_font_property<'a>(font: &'a BdfFont, name: &[u8]) -> Option<&'a BdfProperty> {
    if font.props_size == 0 || name.is_empty() {
        return None;
    }

    let propid = *font.internal.as_ref()?.get(name)?;

    font.props.get(propid)
}
