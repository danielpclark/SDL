// Rust translation of src/psaux/afmparse.c and src/psaux/afmparse.h, and
// of the AFM parser records of include/freetype/internal/psaux.h, from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2006-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! AFM parser (body).
//!
//! The stream's `cursor`, `base` and `limit` are offsets into the parsed
//! bytes; the keys and values it returns are offsets into them too (C's
//! `char*`), with their lengths. `T1_CONFIG_OPTION_NO_AFM` is undefined.

use super::super::base::ftmemory::ft_new_array;
use super::super::fttypes::*;
use super::super::t1types::*;
use super::psconv::*;

/***************************************************************************
 *
 * AFM_Stream
 *
 * The use of AFM_Stream is largely inspired by parseAFM.[ch] from t1lib.
 *
 */

const AFM_STREAM_STATUS_NORMAL: FtInt = 0;
const AFM_STREAM_STATUS_EOC: FtInt = 1;
const AFM_STREAM_STATUS_EOL: FtInt = 2;
const AFM_STREAM_STATUS_EOF: FtInt = 3;

/// `AFM_StreamRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct AfmStreamRec {
    pub cursor: usize,
    pub base: usize,
    pub limit: usize,

    pub status: FtInt,
}

const EOF: FtInt = -1;

/* this works because empty lines are ignored */
#[inline]
fn afm_is_newline(ch: FtInt) -> bool {
    ch == b'\r' as FtInt || ch == b'\n' as FtInt
}

#[inline]
fn afm_is_eof(ch: FtInt) -> bool {
    ch == EOF || ch == 0x1A
}

#[inline]
fn afm_is_space(ch: FtInt) -> bool {
    ch == b' ' as FtInt || ch == b'\t' as FtInt
}

/* column separator; there is no `column' in the spec actually */
#[inline]
fn afm_is_sep(ch: FtInt) -> bool {
    ch == b';' as FtInt
}

/// `AFM_GETC`
#[inline]
fn afm_getc(stream: &mut AfmStreamRec, b: &[u8]) -> FtInt {
    if stream.cursor < stream.limit {
        let c = at(b, stream.cursor) as FtInt;
        stream.cursor += 1;
        c
    } else {
        EOF
    }
}

/// `AFM_STREAM_KEY_BEGIN`
#[inline]
fn afm_stream_key_begin(stream: &AfmStreamRec) -> usize {
    stream.cursor.wrapping_sub(1)
}

/// `AFM_STREAM_KEY_LEN`
#[inline]
fn afm_stream_key_len(stream: &AfmStreamRec, key: usize) -> usize {
    stream.cursor.wrapping_sub(key).wrapping_sub(1)
}

#[inline]
fn afm_status_eoc(stream: &AfmStreamRec) -> bool {
    stream.status >= AFM_STREAM_STATUS_EOC
}

#[inline]
fn afm_status_eol(stream: &AfmStreamRec) -> bool {
    stream.status >= AFM_STREAM_STATUS_EOL
}

#[inline]
fn afm_status_eof(stream: &AfmStreamRec) -> bool {
    stream.status >= AFM_STREAM_STATUS_EOF
}

/// `afm_stream_skip_spaces`
fn afm_stream_skip_spaces(stream: &mut AfmStreamRec, b: &[u8]) -> FtInt {
    let mut ch: FtInt; /* make stupid compiler happy */

    if afm_status_eoc(stream) {
        return b';' as FtInt;
    }

    loop {
        ch = afm_getc(stream, b);
        if !afm_is_space(ch) {
            break;
        }
    }

    if afm_is_newline(ch) {
        stream.status = AFM_STREAM_STATUS_EOL;
    } else if afm_is_sep(ch) {
        stream.status = AFM_STREAM_STATUS_EOC;
    } else if afm_is_eof(ch) {
        stream.status = AFM_STREAM_STATUS_EOF;
    }

    ch
}

/// `afm_stream_read_one`: read a key or value in current column
fn afm_stream_read_one(stream: &mut AfmStreamRec, b: &[u8]) -> Option<usize> {
    afm_stream_skip_spaces(stream, b);
    if afm_status_eoc(stream) {
        return None;
    }

    let str = afm_stream_key_begin(stream);

    loop {
        let ch = afm_getc(stream, b);
        if afm_is_space(ch) {
            break;
        } else if afm_is_newline(ch) {
            stream.status = AFM_STREAM_STATUS_EOL;
            break;
        } else if afm_is_sep(ch) {
            stream.status = AFM_STREAM_STATUS_EOC;
            break;
        } else if afm_is_eof(ch) {
            stream.status = AFM_STREAM_STATUS_EOF;
            break;
        }
    }

    Some(str)
}

/// `afm_stream_read_string`: read a string (i.e., read to EOL)
fn afm_stream_read_string(stream: &mut AfmStreamRec, b: &[u8]) -> Option<usize> {
    afm_stream_skip_spaces(stream, b);
    if afm_status_eol(stream) {
        return None;
    }

    let str = afm_stream_key_begin(stream);

    /* scan to eol */
    loop {
        let ch = afm_getc(stream, b);
        if afm_is_newline(ch) {
            stream.status = AFM_STREAM_STATUS_EOL;
            break;
        } else if afm_is_eof(ch) {
            stream.status = AFM_STREAM_STATUS_EOF;
            break;
        }
    }

    Some(str)
}

/***************************************************************************
 *
 * AFM_Parser
 *
 */

/* all keys defined in Ch. 7-10 of 5004.AFM_Spec.pdf */
/// `AFM_Token`
type AfmToken = usize;

const AFM_TOKEN_ASCENDER: AfmToken = 0;
const AFM_TOKEN_AXISLABEL: AfmToken = 1;
const AFM_TOKEN_AXISTYPE: AfmToken = 2;
const AFM_TOKEN_B: AfmToken = 3;
const AFM_TOKEN_BLENDAXISTYPES: AfmToken = 4;
const AFM_TOKEN_BLENDDESIGNMAP: AfmToken = 5;
const AFM_TOKEN_BLENDDESIGNPOSITIONS: AfmToken = 6;
const AFM_TOKEN_C: AfmToken = 7;
const AFM_TOKEN_CC: AfmToken = 8;
const AFM_TOKEN_CH: AfmToken = 9;
const AFM_TOKEN_CAPHEIGHT: AfmToken = 10;
const AFM_TOKEN_CHARWIDTH: AfmToken = 11;
const AFM_TOKEN_CHARACTERSET: AfmToken = 12;
const AFM_TOKEN_CHARACTERS: AfmToken = 13;
const AFM_TOKEN_DESCENDER: AfmToken = 14;
const AFM_TOKEN_ENCODINGSCHEME: AfmToken = 15;
const AFM_TOKEN_ENDAXIS: AfmToken = 16;
const AFM_TOKEN_ENDCHARMETRICS: AfmToken = 17;
const AFM_TOKEN_ENDCOMPOSITES: AfmToken = 18;
const AFM_TOKEN_ENDDIRECTION: AfmToken = 19;
const AFM_TOKEN_ENDFONTMETRICS: AfmToken = 20;
const AFM_TOKEN_ENDKERNDATA: AfmToken = 21;
const AFM_TOKEN_ENDKERNPAIRS: AfmToken = 22;
const AFM_TOKEN_ENDTRACKKERN: AfmToken = 23;
const AFM_TOKEN_ESCCHAR: AfmToken = 24;
const AFM_TOKEN_FAMILYNAME: AfmToken = 25;
const AFM_TOKEN_FONTBBOX: AfmToken = 26;
const AFM_TOKEN_FONTNAME: AfmToken = 27;
const AFM_TOKEN_FULLNAME: AfmToken = 28;
const AFM_TOKEN_ISBASEFONT: AfmToken = 29;
const AFM_TOKEN_ISCIDFONT: AfmToken = 30;
const AFM_TOKEN_ISFIXEDPITCH: AfmToken = 31;
const AFM_TOKEN_ISFIXEDV: AfmToken = 32;
const AFM_TOKEN_ITALICANGLE: AfmToken = 33;
const AFM_TOKEN_KP: AfmToken = 34;
const AFM_TOKEN_KPH: AfmToken = 35;
const AFM_TOKEN_KPX: AfmToken = 36;
const AFM_TOKEN_KPY: AfmToken = 37;
const AFM_TOKEN_L: AfmToken = 38;
const AFM_TOKEN_MAPPINGSCHEME: AfmToken = 39;
const AFM_TOKEN_METRICSSETS: AfmToken = 40;
const AFM_TOKEN_N: AfmToken = 41;
const AFM_TOKEN_NOTICE: AfmToken = 42;
const AFM_TOKEN_PCC: AfmToken = 43;
const AFM_TOKEN_STARTAXIS: AfmToken = 44;
const AFM_TOKEN_STARTCHARMETRICS: AfmToken = 45;
const AFM_TOKEN_STARTCOMPOSITES: AfmToken = 46;
const AFM_TOKEN_STARTDIRECTION: AfmToken = 47;
const AFM_TOKEN_STARTFONTMETRICS: AfmToken = 48;
const AFM_TOKEN_STARTKERNDATA: AfmToken = 49;
const AFM_TOKEN_STARTKERNPAIRS: AfmToken = 50;
const AFM_TOKEN_STARTKERNPAIRS0: AfmToken = 51;
const AFM_TOKEN_STARTKERNPAIRS1: AfmToken = 52;
const AFM_TOKEN_STARTTRACKKERN: AfmToken = 53;
const AFM_TOKEN_STDHW: AfmToken = 54;
const AFM_TOKEN_STDVW: AfmToken = 55;
const AFM_TOKEN_TRACKKERN: AfmToken = 56;
const AFM_TOKEN_UNDERLINEPOSITION: AfmToken = 57;
const AFM_TOKEN_UNDERLINETHICKNESS: AfmToken = 58;
const AFM_TOKEN_VV: AfmToken = 59;
const AFM_TOKEN_VVECTOR: AfmToken = 60;
const AFM_TOKEN_VERSION: AfmToken = 61;
const AFM_TOKEN_W: AfmToken = 62;
const AFM_TOKEN_W0: AfmToken = 63;
const AFM_TOKEN_W0X: AfmToken = 64;
const AFM_TOKEN_W0Y: AfmToken = 65;
const AFM_TOKEN_W1: AfmToken = 66;
const AFM_TOKEN_W1X: AfmToken = 67;
const AFM_TOKEN_W1Y: AfmToken = 68;
const AFM_TOKEN_WX: AfmToken = 69;
const AFM_TOKEN_WY: AfmToken = 70;
const AFM_TOKEN_WEIGHT: AfmToken = 71;
const AFM_TOKEN_WEIGHTVECTOR: AfmToken = 72;
const AFM_TOKEN_XHEIGHT: AfmToken = 73;
const N_AFM_TOKENS: AfmToken = 74;
const AFM_TOKEN_UNKNOWN: AfmToken = 75;

/// `afm_key_table`
static AFM_KEY_TABLE: [&[u8]; N_AFM_TOKENS] = [
    b"Ascender",
    b"AxisLabel",
    b"AxisType",
    b"B",
    b"BlendAxisTypes",
    b"BlendDesignMap",
    b"BlendDesignPositions",
    b"C",
    b"CC",
    b"CH",
    b"CapHeight",
    b"CharWidth",
    b"CharacterSet",
    b"Characters",
    b"Descender",
    b"EncodingScheme",
    b"EndAxis",
    b"EndCharMetrics",
    b"EndComposites",
    b"EndDirection",
    b"EndFontMetrics",
    b"EndKernData",
    b"EndKernPairs",
    b"EndTrackKern",
    b"EscChar",
    b"FamilyName",
    b"FontBBox",
    b"FontName",
    b"FullName",
    b"IsBaseFont",
    b"IsCIDFont",
    b"IsFixedPitch",
    b"IsFixedV",
    b"ItalicAngle",
    b"KP",
    b"KPH",
    b"KPX",
    b"KPY",
    b"L",
    b"MappingScheme",
    b"MetricsSets",
    b"N",
    b"Notice",
    b"PCC",
    b"StartAxis",
    b"StartCharMetrics",
    b"StartComposites",
    b"StartDirection",
    b"StartFontMetrics",
    b"StartKernData",
    b"StartKernPairs",
    b"StartKernPairs0",
    b"StartKernPairs1",
    b"StartTrackKern",
    b"StdHW",
    b"StdVW",
    b"TrackKern",
    b"UnderlinePosition",
    b"UnderlineThickness",
    b"VV",
    b"VVector",
    b"Version",
    b"W",
    b"W0",
    b"W0X",
    b"W0Y",
    b"W1",
    b"W1X",
    b"W1Y",
    b"WX",
    b"WY",
    b"Weight",
    b"WeightVector",
    b"XHeight",
];

/* afmparse.h */

/// `AFM_ValueType`
pub type AfmValueType = FtUInt;

pub const AFM_VALUE_TYPE_STRING: AfmValueType = 0;
pub const AFM_VALUE_TYPE_NAME: AfmValueType = 1;
pub const AFM_VALUE_TYPE_FIXED: AfmValueType = 2; /* real number */
pub const AFM_VALUE_TYPE_INTEGER: AfmValueType = 3;
pub const AFM_VALUE_TYPE_BOOL: AfmValueType = 4;
pub const AFM_VALUE_TYPE_INDEX: AfmValueType = 5; /* glyph index */

/// `AFM_ValueRec` (C's union is the fields of the value's type; `i` is
/// also the `u` of an index)
#[derive(Debug, Clone, Default)]
pub struct AfmValueRec {
    pub type_: AfmValueType,
    pub s: Option<Vec<u8>>,
    pub f: FtFixed,
    pub i: FtInt,
    pub b: FtBool,
}

pub const AFM_MAX_ARGUMENTS: FtInt = 5;

/// `AFM_ParserRec`: the parser of the AFM data `b` (with `stream`'s
/// offsets into it), the font info it fills, and the glyph index
/// callback (`get_index`, with its `user_data`).
pub struct AfmParserRec<'a> {
    pub b: &'a [u8],
    pub stream: AfmStreamRec,

    pub font_info: Option<&'a mut AfmFontInfoRec>,

    pub get_index: Option<&'a dyn Fn(&[u8]) -> FtInt>,
}

impl std::fmt::Debug for AfmParserRec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AfmParserRec")
            .field("stream", &self.stream)
            .field("font_info", &self.font_info)
            .field("get_index", &self.get_index.is_some())
            .finish_non_exhaustive()
    }
}

/// `afm_parser_read_vals`: read `n` integer or string values from the
/// current line
///
/// All values are numbers, booleans, or (glyph) names.
pub fn afm_parser_read_vals(
    parser: &mut AfmParserRec<'_>,
    vals: &mut [AfmValueRec],
    n: FtInt,
) -> FtInt {
    let b = parser.b;
    let stream = &mut parser.stream;

    if n > AFM_MAX_ARGUMENTS {
        return 0;
    }

    let mut i: FtInt = 0;
    while i < n {
        let val = &mut vals[i as usize];

        let str = if val.type_ == AFM_VALUE_TYPE_STRING {
            afm_stream_read_string(stream, b)
        } else {
            afm_stream_read_one(stream, b)
        };

        let Some(str) = str else {
            break;
        };

        let len = afm_stream_key_len(stream, str);

        match val.type_ {
            AFM_VALUE_TYPE_STRING | AFM_VALUE_TYPE_NAME => {
                if let Ok(mut s) = ft_new_array::<u8>(len as FtLong + 1) {
                    let src = b.get(str..str.wrapping_add(len)).unwrap_or(&[]);
                    s[..src.len()].copy_from_slice(src);
                    s.truncate(len);
                    val.s = Some(s);
                }
            }

            AFM_VALUE_TYPE_FIXED => {
                let mut cur = str;
                val.f = ps_conv_to_fixed(b, &mut cur, str.wrapping_add(len), 0);
            }

            AFM_VALUE_TYPE_INTEGER => {
                let mut cur = str;
                val.i = ps_conv_to_int(b, &mut cur, str.wrapping_add(len)) as FtInt;
            }

            AFM_VALUE_TYPE_BOOL => {
                val.b = len == 4 && b.get(str..str + 4) == Some(&b"true"[..]);
            }

            AFM_VALUE_TYPE_INDEX => {
                if let Some(get_index) = parser.get_index {
                    let name = b.get(str..str.wrapping_add(len)).unwrap_or(&[]);
                    val.i = get_index(name);
                } else {
                    val.i = 0;
                }
            }

            _ => {}
        }

        i += 1;
    }

    i
}

/// `afm_parser_next_key`: read the next key from the next line or column
/// (its offset and length)
pub fn afm_parser_next_key(
    parser: &mut AfmParserRec<'_>,
    line: bool,
    len: Option<&mut usize>,
) -> Option<usize> {
    let b = parser.b;
    let stream = &mut parser.stream;
    let mut key: Option<usize>; /* make stupid compiler happy */

    if line {
        loop {
            /* skip current line */
            if !afm_status_eol(stream) {
                afm_stream_read_string(stream, b);
            }

            stream.status = AFM_STREAM_STATUS_NORMAL;
            key = afm_stream_read_one(stream, b);

            /* skip empty line */
            if key.is_none() && !afm_status_eof(stream) && afm_status_eol(stream) {
                continue;
            }

            break;
        }
    } else {
        loop {
            /* skip current column */
            while !afm_status_eoc(stream) {
                afm_stream_read_one(stream, b);
            }

            stream.status = AFM_STREAM_STATUS_NORMAL;
            key = afm_stream_read_one(stream, b);

            /* skip empty column */
            if key.is_none() && !afm_status_eof(stream) && afm_status_eoc(stream) {
                continue;
            }

            break;
        }
    }

    if let Some(len) = len {
        *len = match key {
            Some(key) => afm_stream_key_len(stream, key),
            None => 0,
        };
    }

    key
}

/// `afm_tokenize`
fn afm_tokenize(b: &[u8], key: usize, len: usize) -> AfmToken {
    let k0 = at(b, key);

    let mut n: usize = 0;
    while n < N_AFM_TOKENS {
        if AFM_KEY_TABLE[n][0] == k0 {
            while n < N_AFM_TOKENS {
                if AFM_KEY_TABLE[n][0] != k0 {
                    return AFM_TOKEN_UNKNOWN;
                }

                /* (`ft_strncmp( afm_key_table[n], key, len )') */
                let entry = AFM_KEY_TABLE[n];
                let mut equal = true;
                for i in 0..len {
                    let a = entry.get(i).copied().unwrap_or(0);
                    let c = at(b, key + i);
                    if a != c {
                        equal = false;
                        break;
                    }
                    if a == 0 {
                        break;
                    }
                }
                if equal {
                    return n;
                }
                n += 1;
            }
        }
        n += 1;
    }

    AFM_TOKEN_UNKNOWN
}

/// `afm_parser_init`
pub fn afm_parser_init<'a>(b: &'a [u8], base: usize, limit: usize) -> FtResult<AfmParserRec<'a>> {
    let stream = AfmStreamRec {
        cursor: base,
        base,
        limit,

        /* don't skip the first line during the first call */
        status: AFM_STREAM_STATUS_EOL,
    };

    Ok(AfmParserRec {
        b,
        stream,
        font_info: None,
        get_index: None,
    })
}

/// `afm_parser_done`
pub fn afm_parser_done(_parser: &mut AfmParserRec<'_>) {}

/// `afm_parser_read_int`
fn afm_parser_read_int(parser: &mut AfmParserRec<'_>, aint: &mut FtInt) -> FtResult<()> {
    let mut val = [AfmValueRec {
        type_: AFM_VALUE_TYPE_INTEGER,
        ..Default::default()
    }];

    if afm_parser_read_vals(parser, &mut val, 1) == 1 {
        *aint = val[0].i;

        Ok(())
    } else {
        Err(FT_ERR_SYNTAX_ERROR)
    }
}

/// `afm_parse_track_kern`
fn afm_parse_track_kern(parser: &mut AfmParserRec<'_>) -> FtResult<()> {
    let mut len: usize = 0;
    let mut n: FtInt = -1;
    let mut tmp: FtInt = 0;

    'fail: {
        if afm_parser_read_int(parser, &mut tmp).is_err() {
            break 'fail;
        }

        if tmp < 0 {
            break 'fail;
        }

        let Some(fi) = parser.font_info.as_deref_mut() else {
            break 'fail;
        };
        fi.NumTrackKern = tmp as FtUInt;

        /* Rough sanity check: The minimum line length of the `TrackKern` */
        /* command is 20 characters (including the EOL character).        */
        if (parser.stream.limit.wrapping_sub(parser.stream.cursor) as FtULong) / 20
            < fi.NumTrackKern as FtULong
        {
            break 'fail;
        }

        if fi.NumTrackKern != 0 {
            fi.TrackKerns = ft_new_array(fi.NumTrackKern as FtLong)?;
        }

        while let Some(key) = afm_parser_next_key(parser, true, Some(&mut len)) {
            let mut shared_vals: [AfmValueRec; 5] = Default::default();

            match afm_tokenize(parser.b, key, len) {
                AFM_TOKEN_TRACKKERN => {
                    n += 1;

                    let num_track_kern = parser.font_info.as_deref().map_or(0, |f| f.NumTrackKern);
                    if n >= num_track_kern as FtInt {
                        break 'fail;
                    }

                    shared_vals[0].type_ = AFM_VALUE_TYPE_INTEGER;
                    shared_vals[1].type_ = AFM_VALUE_TYPE_FIXED;
                    shared_vals[2].type_ = AFM_VALUE_TYPE_FIXED;
                    shared_vals[3].type_ = AFM_VALUE_TYPE_FIXED;
                    shared_vals[4].type_ = AFM_VALUE_TYPE_FIXED;
                    if afm_parser_read_vals(parser, &mut shared_vals, 5) != 5 {
                        break 'fail;
                    }

                    let Some(fi) = parser.font_info.as_deref_mut() else {
                        break 'fail;
                    };
                    let tk = &mut fi.TrackKerns[n as usize];

                    tk.degree = shared_vals[0].i;
                    tk.min_ptsize = shared_vals[1].f;
                    tk.min_kern = shared_vals[2].f;
                    tk.max_ptsize = shared_vals[3].f;
                    tk.max_kern = shared_vals[4].f;
                }

                AFM_TOKEN_ENDTRACKKERN | AFM_TOKEN_ENDKERNDATA | AFM_TOKEN_ENDFONTMETRICS => {
                    tmp = n + 1;
                    let Some(fi) = parser.font_info.as_deref_mut() else {
                        break 'fail;
                    };
                    if tmp as FtUInt != fi.NumTrackKern {
                        fi.NumTrackKern = tmp as FtUInt;
                    }
                    return Ok(());
                }

                AFM_TOKEN_UNKNOWN => {}

                _ => {
                    break 'fail;
                }
            }
        }
    }

    /* Fail: */
    Err(FT_ERR_SYNTAX_ERROR)
}

/// `KERN_INDEX`
#[inline]
fn kern_index(g1: FtUInt, g2: FtUInt) -> FtULong {
    ((g1 as FtULong) << 16) | g2 as FtULong
}

/// `afm_compare_kern_pairs`: compare two kerning pairs
pub fn afm_compare_kern_pairs(kp1: &AfmKernPairRec, kp2: &AfmKernPairRec) -> std::cmp::Ordering {
    let index1 = kern_index(kp1.index1, kp1.index2);
    let index2 = kern_index(kp2.index1, kp2.index2);

    index1.cmp(&index2)
}

/// `afm_parse_kern_pairs`
fn afm_parse_kern_pairs(parser: &mut AfmParserRec<'_>) -> FtResult<()> {
    let mut len: usize = 0;
    let mut n: FtInt = -1;
    let mut tmp: FtInt = 0;

    'fail: {
        if afm_parser_read_int(parser, &mut tmp).is_err() {
            break 'fail;
        }

        if tmp < 0 {
            break 'fail;
        }

        let Some(fi) = parser.font_info.as_deref_mut() else {
            break 'fail;
        };
        fi.NumKernPair = tmp as FtUInt;

        /* Rough sanity check: The minimum line length of the `KP`,    */
        /* `KPH`,`KPX`, and `KPY` commands is 10 characters (including */
        /* the EOL character).                                         */
        if (parser.stream.limit.wrapping_sub(parser.stream.cursor) as FtULong) / 10
            < fi.NumKernPair as FtULong
        {
            break 'fail;
        }

        if fi.NumKernPair != 0 {
            fi.KernPairs = ft_new_array(fi.NumKernPair as FtLong)?;
        }

        while let Some(key) = afm_parser_next_key(parser, true, Some(&mut len)) {
            let token = afm_tokenize(parser.b, key, len);

            match token {
                AFM_TOKEN_KP | AFM_TOKEN_KPX | AFM_TOKEN_KPY => {
                    let mut shared_vals: [AfmValueRec; 4] = Default::default();

                    n += 1;

                    let num_kern_pair = parser.font_info.as_deref().map_or(0, |f| f.NumKernPair);
                    if n >= num_kern_pair as FtInt {
                        break 'fail;
                    }

                    shared_vals[0].type_ = AFM_VALUE_TYPE_INDEX;
                    shared_vals[1].type_ = AFM_VALUE_TYPE_INDEX;
                    shared_vals[2].type_ = AFM_VALUE_TYPE_INTEGER;
                    shared_vals[3].type_ = AFM_VALUE_TYPE_INTEGER;
                    let r = afm_parser_read_vals(parser, &mut shared_vals, 4);
                    if r < 3 {
                        break 'fail;
                    }

                    let Some(fi) = parser.font_info.as_deref_mut() else {
                        break 'fail;
                    };
                    let kp = &mut fi.KernPairs[n as usize];

                    /* index values can't be negative */
                    kp.index1 = shared_vals[0].i as FtUInt;
                    kp.index2 = shared_vals[1].i as FtUInt;
                    if token == AFM_TOKEN_KPY {
                        kp.x = 0;
                        kp.y = shared_vals[2].i;
                    } else {
                        kp.x = shared_vals[2].i;
                        kp.y = if token == AFM_TOKEN_KP && r == 4 {
                            shared_vals[3].i
                        } else {
                            0
                        };
                    }
                }

                AFM_TOKEN_ENDKERNPAIRS | AFM_TOKEN_ENDKERNDATA | AFM_TOKEN_ENDFONTMETRICS => {
                    tmp = n + 1;
                    let Some(fi) = parser.font_info.as_deref_mut() else {
                        break 'fail;
                    };
                    if tmp as FtUInt != fi.NumKernPair {
                        fi.NumKernPair = tmp as FtUInt;
                    }

                    /* (glibc's qsort is a stable merge sort) */
                    let count = (fi.NumKernPair as usize).min(fi.KernPairs.len());
                    fi.KernPairs[..count].sort_by(afm_compare_kern_pairs);

                    return Ok(());
                }

                AFM_TOKEN_UNKNOWN => {}

                _ => {
                    break 'fail;
                }
            }
        }
    }

    /* Fail: */
    Err(FT_ERR_SYNTAX_ERROR)
}

/// `afm_parse_kern_data`
fn afm_parse_kern_data(parser: &mut AfmParserRec<'_>) -> FtResult<()> {
    let mut len: usize = 0;
    let mut have_trackkern = false;
    let mut have_kernpairs = false;

    'fail: {
        while let Some(key) = afm_parser_next_key(parser, true, Some(&mut len)) {
            match afm_tokenize(parser.b, key, len) {
                AFM_TOKEN_STARTTRACKKERN => {
                    if have_trackkern {
                        break 'fail;
                    }

                    afm_parse_track_kern(parser)?;

                    have_trackkern = true;
                }

                AFM_TOKEN_STARTKERNPAIRS | AFM_TOKEN_STARTKERNPAIRS0 => {
                    if have_kernpairs {
                        break 'fail;
                    }

                    afm_parse_kern_pairs(parser)?;

                    have_kernpairs = true;
                }

                AFM_TOKEN_ENDKERNDATA | AFM_TOKEN_ENDFONTMETRICS => {
                    return Ok(());
                }

                AFM_TOKEN_UNKNOWN => {}

                _ => {
                    break 'fail;
                }
            }
        }
    }

    /* Fail: */
    Err(FT_ERR_SYNTAX_ERROR)
}

/// `afm_parser_skip_section`
fn afm_parser_skip_section(
    parser: &mut AfmParserRec<'_>,
    n: FtInt,
    end_section: AfmToken,
) -> FtResult<()> {
    let mut len: usize = 0;
    let mut n = n;

    'fail: {
        while n > 0 {
            n -= 1;
            if afm_parser_next_key(parser, true, None).is_none() {
                break 'fail;
            }
        }

        while let Some(key) = afm_parser_next_key(parser, true, Some(&mut len)) {
            let token = afm_tokenize(parser.b, key, len);

            if token == end_section || token == AFM_TOKEN_ENDFONTMETRICS {
                return Ok(());
            }
        }
    }

    /* Fail: */
    Err(FT_ERR_SYNTAX_ERROR)
}

/// `afm_parser_parse`
pub fn afm_parser_parse(parser: &mut AfmParserRec<'_>) -> FtResult<()> {
    let mut error: FtError = FT_ERR_SYNTAX_ERROR;
    let mut len: usize = 0;
    let mut metrics_sets: FtInt = 0;

    if parser.font_info.is_none() {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let key = afm_parser_next_key(parser, true, Some(&mut len));
    match key {
        Some(key) if len == 16 && parser.b.get(key..key + 16) == Some(&b"StartFontMetrics"[..]) => {
        }
        _ => return Err(FT_ERR_UNKNOWN_FILE_FORMAT),
    }

    'fail: {
        while let Some(key) = afm_parser_next_key(parser, true, Some(&mut len)) {
            let mut shared_vals: [AfmValueRec; 4] = Default::default();

            match afm_tokenize(parser.b, key, len) {
                AFM_TOKEN_METRICSSETS => {
                    if afm_parser_read_int(parser, &mut metrics_sets).is_err() {
                        break 'fail;
                    }

                    if metrics_sets != 0 && metrics_sets != 2 {
                        error = FT_ERR_UNIMPLEMENTED_FEATURE;

                        break 'fail;
                    }
                }

                AFM_TOKEN_ISCIDFONT => {
                    shared_vals[0].type_ = AFM_VALUE_TYPE_BOOL;
                    if afm_parser_read_vals(parser, &mut shared_vals, 1) != 1 {
                        break 'fail;
                    }

                    if let Some(fi) = parser.font_info.as_deref_mut() {
                        fi.IsCIDFont = shared_vals[0].b;
                    }
                }

                AFM_TOKEN_FONTBBOX => {
                    shared_vals[0].type_ = AFM_VALUE_TYPE_FIXED;
                    shared_vals[1].type_ = AFM_VALUE_TYPE_FIXED;
                    shared_vals[2].type_ = AFM_VALUE_TYPE_FIXED;
                    shared_vals[3].type_ = AFM_VALUE_TYPE_FIXED;
                    if afm_parser_read_vals(parser, &mut shared_vals, 4) != 4 {
                        break 'fail;
                    }

                    if let Some(fi) = parser.font_info.as_deref_mut() {
                        fi.FontBBox.xMin = shared_vals[0].f;
                        fi.FontBBox.yMin = shared_vals[1].f;
                        fi.FontBBox.xMax = shared_vals[2].f;
                        fi.FontBBox.yMax = shared_vals[3].f;
                    }
                }

                AFM_TOKEN_ASCENDER => {
                    shared_vals[0].type_ = AFM_VALUE_TYPE_FIXED;
                    if afm_parser_read_vals(parser, &mut shared_vals, 1) != 1 {
                        break 'fail;
                    }

                    if let Some(fi) = parser.font_info.as_deref_mut() {
                        fi.Ascender = shared_vals[0].f;
                    }
                }

                AFM_TOKEN_DESCENDER => {
                    shared_vals[0].type_ = AFM_VALUE_TYPE_FIXED;
                    if afm_parser_read_vals(parser, &mut shared_vals, 1) != 1 {
                        break 'fail;
                    }

                    if let Some(fi) = parser.font_info.as_deref_mut() {
                        fi.Descender = shared_vals[0].f;
                    }
                }

                AFM_TOKEN_STARTCHARMETRICS => {
                    let mut n: FtInt = 0;

                    if afm_parser_read_int(parser, &mut n).is_err() {
                        break 'fail;
                    }

                    afm_parser_skip_section(parser, n, AFM_TOKEN_ENDCHARMETRICS)?;
                }

                AFM_TOKEN_STARTKERNDATA => {
                    if let Err(e) = afm_parse_kern_data(parser) {
                        error = e;
                        break 'fail;
                    }

                    /* we only support kern data, so ... */
                    return Ok(());
                }

                AFM_TOKEN_ENDFONTMETRICS => {
                    return Ok(());
                }

                _ => {}
            }
        }
    }

    /* Fail: */
    if let Some(fi) = parser.font_info.as_deref_mut() {
        fi.TrackKerns = Vec::new();
        fi.NumTrackKern = 0;

        fi.KernPairs = Vec::new();
        fi.NumKernPair = 0;

        fi.IsCIDFont = false;
    }

    Err(error)
}
