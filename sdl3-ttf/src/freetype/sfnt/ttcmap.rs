// Rust translation of src/sfnt/ttcmap.c, ttcmap.h, ttcmapc.h and
// include/freetype/internal/ftvalid.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType character mapping table (cmap) support (body).
//!
//! A charmap's `data` pointer into the face's extracted `cmap` table is
//! the table (shared) and an offset into it; the formats' extra fields
//! (`TT_CMap4Rec`, `TT_CMap12Rec`, ...) are in [`TtCMapData`]. The
//! validator's `setjmp`/`longjmp` is `Result` propagation. The face fields
//! the functions read through `FT_CMAP_FACE` (`num_glyphs`, the `cmap`
//! table's size) are kept in the charmap's data when it is built.

use std::sync::Arc;

use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftobjs::*;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::psnames::psmodule::*;
use super::super::tttypes::*;
use super::ttpost::tt_face_get_ps_name;

pub const TT_CMAP_FLAG_UNSORTED: FtInt = 1;
pub const TT_CMAP_FLAG_OVERLAPPING: FtInt = 2;

/// `FT_ValidationLevel`
pub type FtValidationLevel = u32;
pub const FT_VALIDATE_DEFAULT: FtValidationLevel = 0;
pub const FT_VALIDATE_TIGHT: FtValidationLevel = 1;
pub const FT_VALIDATE_PARANOID: FtValidationLevel = 2;

/// `TT_ValidatorRec` (with its `FT_ValidatorRec`): `base`/`limit` are the
/// table and its end offset.
#[derive(Debug, Clone, Copy)]
pub struct TtValidatorRec<'a> {
    pub base: &'a [u8],
    pub limit: usize,
    pub level: FtValidationLevel,
    pub num_glyphs: FtUInt,
}

/// The state of a format 4 charmap (`TT_CMap4Rec`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TtCMap4State {
    pub cur_charcode: FtUInt32, /* current charcode */
    pub cur_gindex: FtUInt,     /* current glyph index */

    pub num_ranges: FtUInt,
    pub cur_range: FtUInt,
    pub cur_start: FtUInt,
    pub cur_end: FtUInt,
    pub cur_delta: FtInt,
    /// `cur_values`, an offset into the table (`None` is NULL)
    pub cur_values: Option<usize>,
}

/// The state of a format 12 or 13 charmap (`TT_CMap12Rec`, `TT_CMap13Rec`).
#[derive(Debug, Clone, Copy, Default)]
pub struct TtCMap12State {
    pub valid: bool,
    pub cur_charcode: FtULong,
    pub cur_gindex: FtUInt,
    pub cur_group: FtULong,
    pub num_groups: FtULong,
}

/// `TT_CMapRec` (minus its `FT_CMapRec` root) with the format-specific
/// fields.
#[derive(Debug, Clone)]
pub struct TtCMapData {
    /// the face's `cmap' table
    pub table: Arc<[u8]>,
    /// `data`: the subtable's offset in `table`
    pub data: usize,
    /// `flags` (for format 4 only)
    pub flags: FtInt,
    /// the TrueType class (for `format` and `get_cmap_info`)
    pub tt_class: &'static TtCMapClassRec,
    /// `face->root.num_glyphs`
    pub num_glyphs: FtLong,

    pub cmap4: TtCMap4State,
    pub cmap12: TtCMap12State,
    /// `num_selectors` (format 14)
    pub num_selectors: FtULong,
}

/// `TT_CMap_ClassRec`
#[derive(Debug)]
pub struct TtCMapClassRec {
    pub clazz: FtCMapClassRec,
    pub format: FtUInt,
    pub validate: fn(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt>,
    pub get_cmap_info: fn(cmap: &TtCMapData) -> TtCMapInfo,
    /// `init`
    pub init: fn(cmap: &mut TtCMapData),
}

fn tt(cmap: &FtCMapRec) -> &TtCMapData {
    match &cmap.data {
        FtCMapData::Tt(d) => d,
        _ => unreachable!("not a TrueType charmap"),
    }
}

fn tt_mut(cmap: &mut FtCMapRec) -> &mut TtCMapData {
    match &mut cmap.data {
        FtCMapData::Tt(d) => d,
        _ => unreachable!("not a TrueType charmap"),
    }
}

/* Too large glyph index return values are caught in `FT_Get_Char_Index' */
/* and `FT_Get_Next_Char' (the latter calls the internal `next' function */
/* again in this case).  To mark character code return values as invalid */
/* it is sufficient to set the corresponding glyph index return value to */
/* zero.                                                                 */

/// `tt_cmap_init`
fn tt_cmap_init(_cmap: &mut TtCMapData) {}

/*************************************************************************/
/*****                                                               *****/
/*****                           FORMAT 0                            *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME        OFFSET         TYPE          DESCRIPTION
 *
 *   format      0              USHORT        must be 0
 *   length      2              USHORT        table length in bytes
 *   language    4              USHORT        Mac language code
 *   glyph_ids   6              BYTE[256]     array of glyph indices
 *               262
 */

fn tt_cmap0_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;

    if table + 2 + 2 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = table + 2; /* skip format */
    let length = ft_next_ushort(t, &mut p) as usize;

    if table + length > valid.limit || length < 262 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check glyph indices whenever necessary */
    if valid.level >= FT_VALIDATE_TIGHT {
        p = table + 6;
        for _ in 0..256 {
            let idx = ft_next_byte(t, &mut p) as FtUInt;
            if idx >= valid.num_glyphs {
                return Err(FT_ERR_INVALID_GLYPH_INDEX);
            }
        }
    }

    Ok(0)
}

fn tt_cmap0_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let c = tt(cmap);
    if char_code < 256 {
        c.table
            .get(c.data + 6 + char_code as usize)
            .copied()
            .unwrap_or(0) as FtUInt
    } else {
        0
    }
}

fn tt_cmap0_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let mut charcode = *pchar_code;
    let mut result: FtUInt32 = 0;
    let mut gindex: FtUInt = 0;

    let table = c.data + 6; /* go to glyph IDs */
    loop {
        charcode = charcode.wrapping_add(1);
        if charcode >= 256 {
            break;
        }
        gindex = c.table.get(table + charcode as usize).copied().unwrap_or(0) as FtUInt;
        if gindex != 0 {
            result = charcode;
            break;
        }
    }

    *pchar_code = result;
    gindex
}

fn tt_cmap0_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 0,
        language: ft_peek_ushort(&cmap.table, cmap.data + 4) as FtULong,
    }
}

pub static TT_CMAP0_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap0_char_index,
        char_next: tt_cmap0_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 0,
    validate: tt_cmap0_validate,
    get_cmap_info: tt_cmap0_get_info,
    init: tt_cmap_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                          FORMAT 2                             *****/
/*****                                                               *****/
/***** This is used for certain CJK encodings that encode text in a  *****/
/***** mixed 8/16 bits encoding along the following lines.           *****/
/*****                                                               *****/
/***** * Certain byte values correspond to an 8-bit character code   *****/
/*****   (typically in the range 0..127 for ASCII compatibility).    *****/
/*****                                                               *****/
/***** * Certain byte values signal the first byte of a 2-byte       *****/
/*****   character code (but these values are also valid as the      *****/
/*****   second byte of a 2-byte character).                         *****/
/*****                                                               *****/
/***** The following charmap lookup and iteration functions all      *****/
/***** assume that the value `charcode' fulfills the following.      *****/
/*****                                                               *****/
/*****   - For one-byte characters, `charcode' is simply the         *****/
/*****     character code.                                           *****/
/*****                                                               *****/
/*****   - For two-byte characters, `charcode' is the 2-byte         *****/
/*****     character code in big endian format.  More precisely:     *****/
/*****                                                               *****/
/*****       (charcode >> 8)    is the first byte value              *****/
/*****       (charcode & 0xFF)  is the second byte value             *****/
/*****                                                               *****/
/***** Note that not all values of `charcode' are valid according    *****/
/***** to these rules, and the function moderately checks the        *****/
/***** arguments.                                                    *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME        OFFSET         TYPE            DESCRIPTION
 *
 *   format      0              USHORT          must be 2
 *   length      2              USHORT          table length in bytes
 *   language    4              USHORT          Mac language code
 *   keys        6              USHORT[256]     sub-header keys
 *   subs        518            SUBHEAD[NSUBS]  sub-headers array
 *   glyph_ids   518+NSUB*8     USHORT[]        glyph ID array
 *
 * The `keys' table is used to map charcode high bytes to sub-headers.
 * The value of `NSUBS' is the number of sub-headers defined in the
 * table and is computed by finding the maximum of the `keys' table.
 *
 * Note that for any `n', `keys[n]' is a byte offset within the `subs'
 * table, i.e., it is the corresponding sub-header index multiplied
 * by 8.
 *
 * Each sub-header has the following format.
 *
 *   NAME        OFFSET      TYPE            DESCRIPTION
 *
 *   first       0           USHORT          first valid low-byte
 *   count       2           USHORT          number of valid low-bytes
 *   delta       4           SHORT           see below
 *   offset      6           USHORT          see below
 *
 * A sub-header defines, for each high byte, the range of valid
 * low bytes within the charmap.  Note that the range defined by `first'
 * and `count' must be completely included in the interval [0..255]
 * according to the specification.
 *
 * If a character code is contained within a given sub-header, then
 * mapping it to a glyph index is done as follows.
 *
 * - The value of `offset' is read.  This is a _byte_ distance from the
 *   location of the `offset' field itself into a slice of the
 *   `glyph_ids' table.  Let's call it `slice' (it is a USHORT[], too).
 *
 * - The value `slice[char.lo - first]' is read.  If it is 0, there is
 *   no glyph for the charcode.  Otherwise, the value of `delta' is
 *   added to it (modulo 65536) to form a new glyph index.
 *
 * It is up to the validation routine to check that all offsets fall
 * within the glyph IDs table (and not within the `subs' table itself or
 * outside of the CMap).
 */

fn tt_cmap2_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;

    if table + 2 + 2 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = table + 2; /* skip format */
    let length = ft_next_ushort(t, &mut p) as usize;

    if table + length > valid.limit || length < 6 + 512 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let keys = table + 6;

    /* parse keys to compute sub-headers count */
    p = keys;
    let mut max_subs: FtUInt = 0;
    for _ in 0..256 {
        let mut idx = ft_next_ushort(t, &mut p) as FtUInt;

        /* value must be multiple of 8 */
        if valid.level >= FT_VALIDATE_PARANOID && (idx & 7) != 0 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        idx >>= 3;

        if idx > max_subs {
            max_subs = idx;
        }
    }

    let subs = p;
    let glyph_ids = subs + (max_subs as usize + 1) * 8;
    if glyph_ids > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* parse sub-headers */
    for _ in 0..=max_subs {
        let first_code = ft_next_ushort(t, &mut p) as FtUInt;
        let code_count = ft_next_ushort(t, &mut p) as FtUInt;
        let delta = ft_next_short(t, &mut p) as FtInt;
        let offset = ft_next_ushort(t, &mut p) as usize;

        /* many Dynalab fonts have empty sub-headers */
        if code_count == 0 {
            continue;
        }

        /* check range within 0..255 */
        if valid.level >= FT_VALIDATE_PARANOID
            && (first_code >= 256 || code_count > 256 - first_code)
        {
            return Err(FT_ERR_INVALID_TABLE);
        }

        /* check offset */
        if offset != 0 {
            let ids = p - 2 + offset;
            if ids < glyph_ids || ids + code_count as usize * 2 > table + length {
                return Err(FT_ERR_INVALID_OFFSET);
            }

            /* check glyph IDs */
            if valid.level >= FT_VALIDATE_TIGHT {
                let limit = p + code_count as usize * 2;

                while p < limit {
                    let mut idx = ft_next_ushort(t, &mut p) as FtUInt;
                    if idx != 0 {
                        idx = ((idx as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;
                        if idx >= valid.num_glyphs {
                            return Err(FT_ERR_INVALID_GLYPH_INDEX);
                        }
                    }
                }
            }
        }
    }

    Ok(0)
}

/// `tt_cmap2_get_subheader`: return sub header corresponding to a given
/// character code; NULL on invalid charcode
fn tt_cmap2_get_subheader(t: &[u8], table: usize, char_code: FtUInt32) -> Option<usize> {
    let mut result = None;

    if char_code < 0x10000 {
        let char_lo = char_code & 0xFF;
        let char_hi = char_code >> 8;
        let mut p = table + 6; /* keys table       */
        let subs = table + 518; /* subheaders table */
        let sub;

        if char_hi == 0 {
            /* an 8-bit character code -- we use subHeader 0 in this case */
            /* to test whether the character code is in the charmap       */
            /*                                                            */
            sub = subs; /* jump to first sub-header */

            /* check that the sub-header for this byte is 0, which */
            /* indicates that it is really a valid one-byte value; */
            /* otherwise, return 0                                 */
            /*                                                     */
            p += char_lo as usize * 2;
            if ft_peek_ushort(t, p) != 0 {
                return None;
            }
        } else {
            /* a 16-bit character code */

            /* jump to key entry  */
            p += char_hi as usize * 2;
            /* jump to sub-header */
            sub = subs + ft_pad_floor(ft_peek_ushort(t, p) as i64, 8) as usize;

            /* check that the high byte isn't a valid one-byte value */
            if sub == subs {
                return None;
            }
        }

        result = Some(sub);
    }

    result
}

fn tt_cmap2_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut result = 0;

    if let Some(subheader) = tt_cmap2_get_subheader(t, c.data, char_code) {
        let mut p = subheader;
        let mut idx = char_code & 0xFF;

        let start = ft_next_ushort(t, &mut p) as FtUInt;
        let count = ft_next_ushort(t, &mut p) as FtUInt;
        let delta = ft_next_short(t, &mut p) as FtInt;
        let offset = ft_peek_ushort(t, p) as usize;

        idx = idx.wrapping_sub(start);
        if idx < count && offset != 0 {
            p += offset + 2 * idx as usize;
            idx = ft_peek_ushort(t, p) as FtUInt;

            if idx != 0 {
                result = ((idx as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;
            }
        }
    }

    result
}

fn tt_cmap2_char_next(cmap: &mut FtCMapRec, pcharcode: &mut FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut gindex: FtUInt = 0;
    let mut result: FtUInt32 = 0;
    let mut charcode = pcharcode.wrapping_add(1);

    'exit: while charcode < 0x10000 {
        'next_subheader: {
            if let Some(subheader) = tt_cmap2_get_subheader(t, c.data, charcode) {
                let mut p = subheader;
                let start = ft_next_ushort(t, &mut p) as FtUInt;
                let count = ft_next_ushort(t, &mut p) as FtUInt;
                let delta = ft_next_short(t, &mut p) as FtInt;
                let offset = ft_peek_ushort(t, p) as usize;
                let mut char_lo = charcode & 0xFF;
                let mut pos: FtUInt;

                if char_lo >= start + count && charcode <= 0xFF {
                    /* this happens only for a malformed cmap */
                    charcode = 0x100;
                    continue 'exit;
                }

                if offset == 0 {
                    if charcode == 0x100 {
                        break 'exit; /* this happens only for a malformed cmap */
                    }
                    break 'next_subheader;
                }

                if char_lo < start {
                    char_lo = start;
                    pos = 0;
                } else {
                    pos = char_lo - start;
                }

                p += offset + pos as usize * 2;
                charcode = ft_pad_floor(charcode as i64, 256) as FtUInt32 + char_lo;

                while pos < count {
                    let idx = ft_next_ushort(t, &mut p) as FtUInt;

                    if idx != 0 {
                        gindex = ((idx as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;
                        if gindex != 0 {
                            result = charcode;
                            break 'exit;
                        }
                    }
                    pos += 1;
                    charcode += 1;
                }

                /* if unsuccessful, avoid `charcode' leaving */
                /* the current 256-character block           */
                if count != 0 {
                    charcode -= 1;
                }
            }
        }

        /* If `charcode' is <= 0xFF, retry with `charcode + 1'.      */
        /* Otherwise jump to the next 256-character block and retry. */
        /* Next_SubHeader: */
        if charcode <= 0xFF {
            charcode += 1;
        } else {
            charcode = ft_pad_floor(charcode as i64, 0x100) as FtUInt32 + 0x100;
        }
    }

    /* Exit: */
    *pcharcode = result;

    gindex
}

fn tt_cmap2_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 2,
        language: ft_peek_ushort(&cmap.table, cmap.data + 4) as FtULong,
    }
}

pub static TT_CMAP2_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap2_char_index,
        char_next: tt_cmap2_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 2,
    validate: tt_cmap2_validate,
    get_cmap_info: tt_cmap2_get_info,
    init: tt_cmap_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                           FORMAT 4                            *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME          OFFSET         TYPE              DESCRIPTION
 *
 *   format        0              USHORT            must be 4
 *   length        2              USHORT            table length
 *                                                  in bytes
 *   language      4              USHORT            Mac language code
 *
 *   segCountX2    6              USHORT            2*NUM_SEGS
 *   searchRange   8              USHORT            2*(1 << LOG_SEGS)
 *   entrySelector 10             USHORT            LOG_SEGS
 *   rangeShift    12             USHORT            segCountX2 -
 *                                                    searchRange
 *
 *   endCount      14             USHORT[NUM_SEGS]  end charcode for
 *                                                  each segment; last
 *                                                  is 0xFFFF
 *
 *   pad           14+NUM_SEGS*2  USHORT            padding
 *
 *   startCount    16+NUM_SEGS*2  USHORT[NUM_SEGS]  first charcode for
 *                                                  each segment
 *
 *   idDelta       16+NUM_SEGS*4  SHORT[NUM_SEGS]   delta for each
 *                                                  segment
 *   idOffset      16+NUM_SEGS*6  SHORT[NUM_SEGS]   range offset for
 *                                                  each segment; can be
 *                                                  zero
 *
 *   glyphIds      16+NUM_SEGS*8  USHORT[]          array of glyph ID
 *                                                  ranges
 *
 * Character codes are modelled by a series of ordered (increasing)
 * intervals called segments.  Each segment has start and end codes,
 * provided by the `startCount' and `endCount' arrays.  Segments must
 * not overlap, and the last segment should always contain the value
 * 0xFFFF for `endCount'.
 *
 * The fields `searchRange', `entrySelector' and `rangeShift' are better
 * ignored (they are traces of over-engineering in the TrueType
 * specification).
 *
 * Each segment also has a signed `delta', as well as an optional offset
 * within the `glyphIds' table.
 *
 * If a segment's idOffset is 0, the glyph index corresponding to any
 * charcode within the segment is obtained by adding the value of
 * `idDelta' directly to the charcode, modulo 65536.
 *
 * Otherwise, a glyph index is taken from the glyph IDs sub-array for
 * the segment, and the value of `idDelta' is added to it.
 *
 *
 * Finally, note that a lot of fonts contain an invalid last segment,
 * where `start' and `end' are correctly set to 0xFFFF but both `delta'
 * and `offset' are incorrect (e.g., `opens___.ttf' which comes with
 * OpenOffice.org).  We need special code to deal with them correctly.
 */

fn tt_cmap4_init(cmap: &mut TtCMapData) {
    let p = cmap.data + 6;
    cmap.cmap4.num_ranges = (ft_peek_ushort(&cmap.table, p) >> 1) as FtUInt;
    cmap.cmap4.cur_charcode = 0xFFFFFFFF;
    cmap.cmap4.cur_gindex = 0;
}

fn tt_cmap4_set_range(cmap: &mut TtCMapData, mut range_index: FtUInt) -> FtInt {
    let t = &cmap.table[..];
    let table = cmap.data;
    let num_ranges = cmap.cmap4.num_ranges;
    let c4 = &mut cmap.cmap4;

    while range_index < num_ranges {
        let mut p = table + 14 + range_index as usize * 2;
        c4.cur_end = ft_peek_ushort(t, p) as FtUInt;

        p += 2 + num_ranges as usize * 2;
        c4.cur_start = ft_peek_ushort(t, p) as FtUInt;

        p += num_ranges as usize * 2;
        c4.cur_delta = ft_peek_short(t, p) as FtInt;

        p += num_ranges as usize * 2;
        let mut offset = ft_peek_ushort(t, p) as FtUInt;

        /* some fonts have an incorrect last segment; */
        /* we have to catch it                        */
        if range_index >= num_ranges.wrapping_sub(1)
            && c4.cur_start == 0xFFFF
            && c4.cur_end == 0xFFFF
        {
            let limit = t.len();

            if offset != 0 && p + offset as usize + 2 > limit {
                c4.cur_delta = 1;
                offset = 0;
            }
        }

        if offset != 0xFFFF {
            c4.cur_values = if offset != 0 {
                Some(p + offset as usize)
            } else {
                None
            };
            c4.cur_range = range_index;
            return 0;
        }

        /* we skip empty segments */
        range_index += 1;
    }

    -1
}

/// `tt_cmap4_next`: search the index of the charcode next to
/// cmap->cur_charcode; caller should call tt_cmap4_set_range with proper
/// range before calling this function
fn tt_cmap4_next(cmap: &mut TtCMapData) {
    let limit = cmap.table.len();
    let num_glyphs = cmap.num_glyphs;

    let mut charcode: FtUInt = cmap.cmap4.cur_charcode.wrapping_add(1);

    if charcode < cmap.cmap4.cur_start {
        charcode = cmap.cmap4.cur_start;
    }

    loop {
        let values = cmap.cmap4.cur_values;
        let end = cmap.cmap4.cur_end;
        let delta = cmap.cmap4.cur_delta;

        'next_segment: {
            if charcode <= end {
                if let Some(values) = values {
                    let t = &cmap.table[..];
                    let mut p = values + 2 * (charcode.wrapping_sub(cmap.cmap4.cur_start)) as usize;

                    /* if p > limit, the whole segment is invalid */
                    if p > limit {
                        break 'next_segment;
                    }

                    loop {
                        let mut gindex = ft_next_ushort(t, &mut p) as FtUInt;

                        if gindex != 0 {
                            gindex = ((gindex as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;
                            if gindex != 0 {
                                cmap.cmap4.cur_charcode = charcode;
                                cmap.cmap4.cur_gindex = gindex;
                                return;
                            }
                        }

                        charcode = charcode.wrapping_add(1);
                        if charcode > end {
                            break;
                        }
                    }
                } else {
                    loop {
                        let mut gindex =
                            ((charcode as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;

                        if gindex as FtLong >= num_glyphs {
                            /* we have an invalid glyph index; if there is an overflow, */
                            /* we can adjust `charcode', otherwise the whole segment is */
                            /* invalid                                                  */
                            gindex = 0;

                            if (charcode as FtInt).wrapping_add(delta) < 0
                                && (end as FtInt).wrapping_add(delta) >= 0
                            {
                                charcode = delta.wrapping_neg() as FtUInt;
                            } else if (charcode as FtInt).wrapping_add(delta) < 0x10000
                                && (end as FtInt).wrapping_add(delta) >= 0x10000
                            {
                                charcode = (0x10000 as FtInt).wrapping_sub(delta) as FtUInt;
                            } else {
                                break 'next_segment;
                            }
                        }

                        if gindex != 0 {
                            cmap.cmap4.cur_charcode = charcode;
                            cmap.cmap4.cur_gindex = gindex;
                            return;
                        }

                        charcode = charcode.wrapping_add(1);
                        if charcode > end {
                            break;
                        }
                    }
                }
            }
        }

        /* Next_Segment: */
        /* we need to find another range */
        if tt_cmap4_set_range(cmap, cmap.cmap4.cur_range.wrapping_add(1)) < 0 {
            break;
        }

        if charcode < cmap.cmap4.cur_start {
            charcode = cmap.cmap4.cur_start;
        }
    }

    cmap.cmap4.cur_charcode = 0xFFFFFFFF;
    cmap.cmap4.cur_gindex = 0;
}

fn tt_cmap4_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;
    let mut error: FtInt = 0;

    if table + 2 + 2 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = table + 2; /* skip format */
    let mut length = ft_next_ushort(t, &mut p) as usize;

    /* in certain fonts, the `length' field is invalid and goes */
    /* out of bound.  We try to correct this here...            */
    if table + length > valid.limit {
        if valid.level >= FT_VALIDATE_TIGHT {
            return Err(FT_ERR_INVALID_TABLE);
        }

        length = valid.limit - table;
    }

    /* it also happens that the `length' field is too small; */
    /* this is easy to correct                               */
    if length < valid.limit - table {
        if valid.level >= FT_VALIDATE_PARANOID {
            return Err(FT_ERR_INVALID_TABLE);
        }

        length = valid.limit - table;
    }

    if length < 16 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    p = table + 6;
    let mut num_segs = ft_next_ushort(t, &mut p) as FtUInt; /* read segCountX2 */

    if valid.level >= FT_VALIDATE_PARANOID {
        /* check that we have an even value here */
        if num_segs & 1 != 0 {
            return Err(FT_ERR_INVALID_TABLE);
        }
    }

    num_segs /= 2;

    if length < 16 + num_segs as usize * 2 * 4 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check the search parameters - even though we never use them */
    /*                                                             */
    if valid.level >= FT_VALIDATE_PARANOID {
        /* check the values of `searchRange', `entrySelector', `rangeShift' */
        let mut search_range = ft_next_ushort(t, &mut p) as FtUInt;
        let entry_selector = ft_next_ushort(t, &mut p) as FtUInt;
        let mut range_shift = ft_next_ushort(t, &mut p) as FtUInt;

        if ((search_range | range_shift) & 1) != 0 {
            /* must be even values */
            return Err(FT_ERR_INVALID_TABLE);
        }

        search_range /= 2;
        range_shift /= 2;

        /* `search range' is the greatest power of 2 that is <= num_segs */

        if search_range > num_segs
            || search_range * 2 < num_segs
            || search_range + range_shift != num_segs
            || search_range != 1u32.wrapping_shl(entry_selector)
        {
            return Err(FT_ERR_INVALID_TABLE);
        }
    }

    let ends = table + 14;
    let starts = table + 16 + num_segs as usize * 2;
    let deltas = starts + num_segs as usize * 2;
    let offsets = deltas + num_segs as usize * 2;
    let glyph_ids = offsets + num_segs as usize * 2;

    /* check last segment; its end count value must be 0xFFFF */
    if valid.level >= FT_VALIDATE_PARANOID {
        p = ends + (num_segs as usize).wrapping_sub(1).wrapping_mul(2);
        if ft_peek_ushort(t, p) != 0xFFFF {
            return Err(FT_ERR_INVALID_TABLE);
        }
    }

    {
        let mut last_start: FtUInt = 0;
        let mut last_end: FtUInt = 0;
        let mut p_start = starts;
        let mut p_end = ends;
        let mut p_delta = deltas;
        let mut p_offset = offsets;

        for n in 0..num_segs {
            p = p_offset;
            let start = ft_next_ushort(t, &mut p_start) as FtUInt;
            let end = ft_next_ushort(t, &mut p_end) as FtUInt;
            let delta = ft_next_short(t, &mut p_delta) as FtInt;
            let offset = ft_next_ushort(t, &mut p_offset) as FtUInt;

            if start > end {
                return Err(FT_ERR_INVALID_TABLE);
            }

            /* this test should be performed at default validation level; */
            /* unfortunately, some popular Asian fonts have overlapping   */
            /* ranges in their charmaps                                   */
            /*                                                            */
            if start <= last_end && n > 0 {
                if valid.level >= FT_VALIDATE_TIGHT {
                    return Err(FT_ERR_INVALID_TABLE);
                } else {
                    /* allow overlapping segments, provided their start points */
                    /* and end points, respectively, are in ascending order    */
                    /*                                                         */
                    if last_start > start || last_end > end {
                        error |= TT_CMAP_FLAG_UNSORTED;
                    } else {
                        error |= TT_CMAP_FLAG_OVERLAPPING;
                    }
                }
            }

            if offset != 0 && offset != 0xFFFF {
                p += offset as usize; /* start of glyph ID array */

                /* check that we point within the glyph IDs table only */
                if valid.level >= FT_VALIDATE_TIGHT {
                    if p < glyph_ids || p + (end - start + 1) as usize * 2 > table + length {
                        return Err(FT_ERR_INVALID_TABLE);
                    }
                }
                /* Some fonts handle the last segment incorrectly.  In */
                /* theory, 0xFFFF might point to an ordinary glyph --  */
                /* a cmap 4 is versatile and could be used for any     */
                /* encoding, not only Unicode.  However, reality shows */
                /* that far too many fonts are sloppy and incorrectly  */
                /* set all fields but `start' and `end' for the last   */
                /* segment if it contains only a single character.     */
                /*                                                     */
                /* We thus omit the test here, delaying it to the      */
                /* routines that actually access the cmap.             */
                else if (n != num_segs - 1 || !(start == 0xFFFF && end == 0xFFFF))
                    && (p < glyph_ids || p + (end - start + 1) as usize * 2 > valid.limit)
                {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                /* check glyph indices within the segment range */
                if valid.level >= FT_VALIDATE_TIGHT {
                    for _ in start..end {
                        let mut idx = ft_next_ushort(t, &mut p) as FtUInt;
                        if idx != 0 {
                            idx = ((idx as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;

                            if idx >= valid.num_glyphs {
                                return Err(FT_ERR_INVALID_GLYPH_INDEX);
                            }
                        }
                    }
                }
            } else if offset == 0xFFFF {
                /* some fonts (erroneously?) use a range offset of 0xFFFF */
                /* to mean missing glyph in cmap table                    */
                /*                                                        */
                if valid.level >= FT_VALIDATE_PARANOID
                    || n != num_segs - 1
                    || !(start == 0xFFFF && end == 0xFFFF)
                {
                    return Err(FT_ERR_INVALID_TABLE);
                }
            }

            last_start = start;
            last_end = end;
        }
    }

    Ok(error)
}

fn tt_cmap4_char_map_linear(cmap: &TtCMapData, pcharcode: &mut FtUInt32, next: bool) -> FtUInt {
    let t = &cmap.table[..];
    let limit = t.len();
    let num_glyphs = cmap.num_glyphs;

    let mut charcode: FtUInt32 = pcharcode.wrapping_add(next as u32);
    let mut gindex: FtUInt = 0;

    let mut p = cmap.data + 6;
    let num_segs = (ft_peek_ushort(t, p) >> 1) as FtUInt;

    if num_segs == 0 {
        return 0;
    }

    let num_segs2 = (num_segs << 1) as usize;

    /* linear search */
    p = cmap.data + 14; /* ends table   */
    let mut q = cmap.data + 16 + num_segs2; /* starts table */

    'outer: for i in 0..num_segs {
        let end = ft_next_ushort(t, &mut p) as FtUInt;
        let start = ft_next_ushort(t, &mut q) as FtUInt;

        if charcode < start {
            if next {
                charcode = start;
            } else {
                break;
            }
        }

        /* Again: */
        loop {
            if charcode <= end {
                let mut r = q - 2 + num_segs2;
                let mut delta = ft_peek_short(t, r) as FtInt;
                r += num_segs2;
                let mut offset = ft_peek_ushort(t, r) as FtUInt;

                /* some fonts have an incorrect last segment; */
                /* we have to catch it                        */
                if i >= num_segs - 1
                    && start == 0xFFFF
                    && end == 0xFFFF
                    && offset != 0
                    && r + offset as usize + 2 > limit
                {
                    delta = 1;
                    offset = 0;
                }

                if offset == 0xFFFF {
                    continue 'outer;
                }

                if offset != 0 {
                    r += offset as usize + (charcode.wrapping_sub(start)) as usize * 2;

                    /* if r > limit, the whole segment is invalid */
                    if next && r > limit {
                        continue 'outer;
                    }

                    gindex = ft_peek_ushort(t, r) as FtUInt;
                    if gindex != 0 {
                        gindex = ((gindex as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;
                        if gindex as FtLong >= num_glyphs {
                            gindex = 0;
                        }
                    }
                } else {
                    gindex = ((charcode as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;

                    if next && gindex as FtLong >= num_glyphs {
                        /* we have an invalid glyph index; if there is an overflow, */
                        /* we can adjust `charcode', otherwise the whole segment is */
                        /* invalid                                                  */
                        gindex = 0;

                        if (charcode as FtInt).wrapping_add(delta) < 0
                            && (end as FtInt).wrapping_add(delta) >= 0
                        {
                            charcode = delta.wrapping_neg() as FtUInt;
                        } else if (charcode as FtInt).wrapping_add(delta) < 0x10000
                            && (end as FtInt).wrapping_add(delta) >= 0x10000
                        {
                            charcode = (0x10000 as FtInt).wrapping_sub(delta) as FtUInt;
                        } else {
                            continue 'outer;
                        }
                    }
                }

                if next && gindex == 0 {
                    if charcode >= 0xFFFF {
                        break 'outer;
                    }

                    charcode += 1;
                    continue;
                }

                break 'outer;
            }
            break;
        }
    }

    if next {
        *pcharcode = charcode;
    }

    gindex
}

fn tt_cmap4_char_map_binary(cmap: &mut TtCMapData, pcharcode: &mut FtUInt32, next: bool) -> FtUInt {
    let limit = cmap.table.len();
    let num_glyphs = cmap.num_glyphs;
    let mut start: FtUInt;
    let mut end: FtUInt = 0;
    let mut offset: FtUInt;
    let mut delta: FtInt;
    let mut max: FtUInt;
    let mut min: FtUInt;
    let mut mid: FtUInt = 0;
    let mut charcode: FtUInt = pcharcode.wrapping_add(next as u32);
    let mut gindex: FtUInt = 0;

    {
        let t = &cmap.table[..];
        let data = cmap.data;

        let mut p = data + 6;
        let num_segs = (ft_peek_ushort(t, p) >> 1) as FtUInt;

        if num_segs == 0 {
            return 0;
        }

        let num_segs2 = (num_segs << 1) as usize;

        min = 0;
        max = num_segs;

        /* binary search */
        loop {
            mid = (min + max) >> 1;
            p = data + 14 + mid as usize * 2;
            end = ft_peek_ushort(t, p) as FtUInt;
            p += 2 + num_segs2;
            start = ft_peek_ushort(t, p) as FtUInt;

            if charcode < start {
                max = mid;
            } else if charcode > end {
                min = mid + 1;
            } else {
                p += num_segs2;
                delta = ft_peek_short(t, p) as FtInt;
                p += num_segs2;
                offset = ft_peek_ushort(t, p) as FtUInt;

                /* some fonts have an incorrect last segment; */
                /* we have to catch it                        */
                if mid >= num_segs - 1
                    && start == 0xFFFF
                    && end == 0xFFFF
                    && offset != 0
                    && p + offset as usize + 2 > limit
                {
                    delta = 1;
                    offset = 0;
                }

                /* search the first segment containing `charcode' */
                if cmap.flags & TT_CMAP_FLAG_OVERLAPPING != 0 {
                    let mut i: FtUInt;

                    /* call the current segment `max' */
                    max = mid;

                    if offset == 0xFFFF {
                        mid = max + 1;
                    }

                    /* search in segments before the current segment */
                    i = max;
                    while i > 0 {
                        let old_p = p;
                        p = data + 14 + (i - 1) as usize * 2;
                        let prev_end = ft_peek_ushort(t, p) as FtUInt;

                        if charcode > prev_end {
                            p = old_p;
                            break;
                        }

                        end = prev_end;
                        p += 2 + num_segs2;
                        start = ft_peek_ushort(t, p) as FtUInt;
                        p += num_segs2;
                        delta = ft_peek_short(t, p) as FtInt;
                        p += num_segs2;
                        offset = ft_peek_ushort(t, p) as FtUInt;

                        if offset != 0xFFFF {
                            mid = i - 1;
                        }
                        i -= 1;
                    }

                    /* no luck */
                    if mid == max + 1 {
                        if i != max {
                            p = data + 14 + max as usize * 2;
                            end = ft_peek_ushort(t, p) as FtUInt;
                            p += 2 + num_segs2;
                            start = ft_peek_ushort(t, p) as FtUInt;
                            p += num_segs2;
                            delta = ft_peek_short(t, p) as FtInt;
                            p += num_segs2;
                            offset = ft_peek_ushort(t, p) as FtUInt;
                        }

                        mid = max;

                        /* search in segments after the current segment */
                        i = max + 1;
                        while i < num_segs {
                            p = data + 14 + i as usize * 2;
                            let next_end = ft_peek_ushort(t, p) as FtUInt;
                            p += 2 + num_segs2;
                            let next_start = ft_peek_ushort(t, p) as FtUInt;

                            if charcode < next_start {
                                break;
                            }

                            end = next_end;
                            start = next_start;
                            p += num_segs2;
                            delta = ft_peek_short(t, p) as FtInt;
                            p += num_segs2;
                            offset = ft_peek_ushort(t, p) as FtUInt;

                            if offset != 0xFFFF {
                                mid = i;
                            }
                            i += 1;
                        }
                        i = i.wrapping_sub(1);

                        /* still no luck */
                        if mid == max {
                            mid = i;

                            break;
                        }
                    }

                    /* end, start, delta, and offset are for the i'th segment */
                    if mid != i {
                        p = data + 14 + mid as usize * 2;
                        end = ft_peek_ushort(t, p) as FtUInt;
                        p += 2 + num_segs2;
                        start = ft_peek_ushort(t, p) as FtUInt;
                        p += num_segs2;
                        delta = ft_peek_short(t, p) as FtInt;
                        p += num_segs2;
                        offset = ft_peek_ushort(t, p) as FtUInt;
                    }
                } else if offset == 0xFFFF {
                    break;
                }

                if offset != 0 {
                    p += offset as usize + (charcode.wrapping_sub(start)) as usize * 2;

                    /* if p > limit, the whole segment is invalid */
                    if next && p > limit {
                        break;
                    }

                    gindex = ft_peek_ushort(t, p) as FtUInt;
                    if gindex != 0 {
                        gindex = ((gindex as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;
                        if gindex as FtLong >= num_glyphs {
                            gindex = 0;
                        }
                    }
                } else {
                    gindex = ((charcode as FtInt).wrapping_add(delta) as FtUInt) & 0xFFFF;

                    if next && gindex as FtLong >= num_glyphs {
                        /* we have an invalid glyph index; if there is an overflow, */
                        /* we can adjust `charcode', otherwise the whole segment is */
                        /* invalid                                                  */
                        gindex = 0;

                        if (charcode as FtInt).wrapping_add(delta) < 0
                            && (end as FtInt).wrapping_add(delta) >= 0
                        {
                            charcode = delta.wrapping_neg() as FtUInt;
                        } else if (charcode as FtInt).wrapping_add(delta) < 0x10000
                            && (end as FtInt).wrapping_add(delta) >= 0x10000
                        {
                            charcode = (0x10000 as FtInt).wrapping_sub(delta) as FtUInt;
                        }
                    }
                }

                break;
            }

            if min >= max {
                break;
            }
        }

        if next {
            /* if `charcode' is not in any segment, then `mid' is */
            /* the segment nearest to `charcode'                  */

            if charcode > end {
                mid += 1;
                if mid == num_segs {
                    return 0;
                }
            }
        }
    }

    if next {
        if tt_cmap4_set_range(cmap, mid) != 0 {
            if gindex != 0 {
                *pcharcode = charcode;
            }
        } else {
            cmap.cmap4.cur_charcode = charcode;

            if gindex != 0 {
                cmap.cmap4.cur_gindex = gindex;
            } else {
                tt_cmap4_next(cmap);
                gindex = cmap.cmap4.cur_gindex;
            }

            if gindex != 0 {
                *pcharcode = cmap.cmap4.cur_charcode;
            }
        }
    }

    gindex
}

fn tt_cmap4_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let mut char_code = char_code;

    if char_code >= 0x10000 {
        return 0;
    }

    if c.flags & TT_CMAP_FLAG_UNSORTED != 0 {
        tt_cmap4_char_map_linear(c, &mut char_code, false)
    } else {
        /* (the lookup without `next' doesn't touch the cmap's state) */
        let mut c = c.clone();
        tt_cmap4_char_map_binary(&mut c, &mut char_code, false)
    }
}

fn tt_cmap4_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let c = tt_mut(cmap);
    let gindex: FtUInt;

    if *pchar_code >= 0xFFFF {
        return 0;
    }

    if c.flags & TT_CMAP_FLAG_UNSORTED != 0 {
        gindex = tt_cmap4_char_map_linear(c, pchar_code, true);
    } else {
        /* no need to search */
        if *pchar_code == c.cmap4.cur_charcode {
            tt_cmap4_next(c);
            gindex = c.cmap4.cur_gindex;
            if gindex != 0 {
                *pchar_code = c.cmap4.cur_charcode;
            }
        } else {
            gindex = tt_cmap4_char_map_binary(c, pchar_code, true);
        }
    }

    gindex
}

fn tt_cmap4_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 4,
        language: ft_peek_ushort(&cmap.table, cmap.data + 4) as FtULong,
    }
}

pub static TT_CMAP4_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap4_char_index,
        char_next: tt_cmap4_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 4,
    validate: tt_cmap4_validate,
    get_cmap_info: tt_cmap4_get_info,
    init: tt_cmap4_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                          FORMAT 6                             *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME        OFFSET          TYPE             DESCRIPTION
 *
 *   format       0              USHORT           must be 6
 *   length       2              USHORT           table length in bytes
 *   language     4              USHORT           Mac language code
 *
 *   first        6              USHORT           first segment code
 *   count        8              USHORT           segment size in chars
 *   glyphIds     10             USHORT[count]    glyph IDs
 *
 * A very simplified segment mapping.
 */

fn tt_cmap6_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;

    if table + 10 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = table + 2;
    let length = ft_next_ushort(t, &mut p) as usize;

    p = table + 8; /* skip language and start index */
    let mut count = ft_next_ushort(t, &mut p) as FtUInt;

    if table + length > valid.limit || length < 10 + count as usize * 2 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check glyph indices */
    if valid.level >= FT_VALIDATE_TIGHT {
        while count > 0 {
            let gindex = ft_next_ushort(t, &mut p) as FtUInt;
            if gindex >= valid.num_glyphs {
                return Err(FT_ERR_INVALID_GLYPH_INDEX);
            }
            count -= 1;
        }
    }

    Ok(0)
}

fn tt_cmap6_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut result = 0;
    let mut p = c.data + 6;
    let start = ft_next_ushort(t, &mut p) as FtUInt;
    let count = ft_next_ushort(t, &mut p) as FtUInt;
    let idx = char_code.wrapping_sub(start);

    if idx < count {
        p += 2 * idx as usize;
        result = ft_peek_ushort(t, p) as FtUInt;
    }

    result
}

fn tt_cmap6_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut result: FtUInt32 = 0;
    let mut char_code = pchar_code.wrapping_add(1);
    let mut gindex: FtUInt = 0;

    let mut p = c.data + 6;
    let start = ft_next_ushort(t, &mut p) as FtUInt;
    let count = ft_next_ushort(t, &mut p) as FtUInt;

    if char_code >= 0x10000 {
        return 0;
    }

    if char_code < start {
        char_code = start;
    }

    let mut idx = char_code - start;
    p += 2 * idx as usize;

    while idx < count {
        gindex = ft_next_ushort(t, &mut p) as FtUInt;
        if gindex != 0 {
            result = char_code;
            break;
        }

        if char_code >= 0xFFFF {
            return 0;
        }

        char_code += 1;
        idx += 1;
    }

    *pchar_code = result;
    gindex
}

fn tt_cmap6_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 6,
        language: ft_peek_ushort(&cmap.table, cmap.data + 4) as FtULong,
    }
}

pub static TT_CMAP6_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap6_char_index,
        char_next: tt_cmap6_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 6,
    validate: tt_cmap6_validate,
    get_cmap_info: tt_cmap6_get_info,
    init: tt_cmap_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                          FORMAT 8                             *****/
/*****                                                               *****/
/***** It is hard to completely understand what the OpenType spec    *****/
/***** says about this format, but here is my conclusion.            *****/
/*****                                                               *****/
/***** The purpose of this format is to easily map UTF-16 text to    *****/
/***** glyph indices.  Basically, the `char_code' must be in one of  *****/
/***** the following formats.                                        *****/
/*****                                                               *****/
/*****   - A 16-bit value that isn't part of the Unicode Surrogates  *****/
/*****     Area (i.e. U+D800-U+DFFF).                                *****/
/*****                                                               *****/
/*****   - A 32-bit value, made of two surrogate values, i.e.. if    *****/
/*****     `char_code = (char_hi << 16) | char_lo', then both        *****/
/*****     `char_hi' and `char_lo' must be in the Surrogates Area.   *****/
/*****      Area.                                                    *****/
/*****                                                               *****/
/***** The `is32' table embedded in the charmap indicates whether a  *****/
/***** given 16-bit value is in the surrogates area or not.          *****/
/*****                                                               *****/
/***** So, for any given `char_code', we can assert the following.   *****/
/*****                                                               *****/
/*****   If `char_hi == 0' then we must have `is32[char_lo] == 0'.   *****/
/*****                                                               *****/
/*****   If `char_hi != 0' then we must have both                    *****/
/*****   `is32[char_hi] != 0' and `is32[char_lo] != 0'.              *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME        OFFSET         TYPE        DESCRIPTION
 *
 *   format      0              USHORT      must be 8
 *   reserved    2              USHORT      reserved
 *   length      4              ULONG       length in bytes
 *   language    8              ULONG       Mac language code
 *   is32        12             BYTE[8192]  32-bitness bitmap
 *   count       8204           ULONG       number of groups
 *
 * This header is followed by `count' groups of the following format:
 *
 *   start       0              ULONG       first charcode
 *   end         4              ULONG       last charcode
 *   startId     8              ULONG       start glyph ID for the group
 */

fn tt_cmap8_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;
    let mut p = table + 4;

    if table + 16 + 8192 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let length = ft_next_ulong(t, &mut p);
    if length as usize > valid.limit - table || length < 8192 + 16 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let is32 = table + 12;
    p = is32 + 8192; /* skip `is32' array */
    let num_groups = ft_next_ulong(t, &mut p);

    /* p + num_groups * 12 > valid->limit ? */
    if num_groups as usize > (valid.limit - p) / 12 {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check groups, they must be in increasing order */
    {
        let mut last: FtUInt32 = 0;

        for n in 0..num_groups {
            let mut start = ft_next_ulong(t, &mut p);
            let end = ft_next_ulong(t, &mut p);
            let start_id = ft_next_ulong(t, &mut p);

            if start > end {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if n > 0 && start <= last {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if valid.level >= FT_VALIDATE_TIGHT {
                let d = end - start;

                /* start_id + end - start >= TT_VALID_GLYPH_COUNT( valid ) ? */
                if d > valid.num_glyphs || start_id >= valid.num_glyphs - d {
                    return Err(FT_ERR_INVALID_GLYPH_INDEX);
                }

                let mut count = end.wrapping_sub(start).wrapping_add(1);

                let bit = |i: u32| -> bool {
                    (byte_at(t, is32 + (i >> 3) as usize) & (0x80 >> (i & 7))) != 0
                };

                if start & !0xFFFF != 0 {
                    /* start_hi != 0; check that is32[i] is 1 for each i in */
                    /* the `hi' and `lo' of the range [start..end]          */
                    while count > 0 {
                        let hi = start >> 16;
                        let lo = start & 0xFFFF;

                        if !bit(hi) {
                            return Err(FT_ERR_INVALID_TABLE);
                        }

                        if !bit(lo) {
                            return Err(FT_ERR_INVALID_TABLE);
                        }
                        count -= 1;
                        start = start.wrapping_add(1);
                    }
                } else {
                    /* start_hi == 0; check that is32[i] is 0 for each i in */
                    /* the range [start..end]                               */

                    /* end_hi cannot be != 0! */
                    if end & !0xFFFF != 0 {
                        return Err(FT_ERR_INVALID_TABLE);
                    }

                    while count > 0 {
                        let lo = start & 0xFFFF;

                        if bit(lo) {
                            return Err(FT_ERR_INVALID_TABLE);
                        }
                        count -= 1;
                        start = start.wrapping_add(1);
                    }
                }
            }

            last = end;
        }
    }

    Ok(0)
}

#[inline]
fn byte_at(t: &[u8], i: usize) -> u8 {
    t.get(i).copied().unwrap_or(0)
}

fn tt_cmap8_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut result = 0;
    let mut p = c.data + 8204;
    let mut num_groups = ft_next_ulong(t, &mut p);

    while num_groups > 0 {
        let start = ft_next_ulong(t, &mut p);
        let end = ft_next_ulong(t, &mut p);
        let start_id = ft_next_ulong(t, &mut p);

        if char_code < start {
            break;
        }

        if char_code <= end {
            if start_id > 0xFFFFFFFF - (char_code - start) {
                return 0;
            }

            result = start_id + (char_code - start);
            break;
        }
        num_groups -= 1;
    }
    result
}

fn tt_cmap8_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut result: FtUInt32 = 0;
    let mut gindex: FtUInt = 0;
    let mut p = c.data + 8204;
    let mut num_groups = ft_next_ulong(t, &mut p);

    if *pchar_code == 0xFFFFFFFF {
        return 0;
    }

    let mut char_code = *pchar_code + 1;

    p = c.data + 8208;

    'groups: while num_groups > 0 {
        let start = ft_next_ulong(t, &mut p);
        let end = ft_next_ulong(t, &mut p);
        let start_id = ft_next_ulong(t, &mut p);

        num_groups -= 1;

        if char_code < start {
            char_code = start;
        }

        /* Again: */
        loop {
            if char_code <= end {
                /* ignore invalid group */
                if start_id > 0xFFFFFFFF - (char_code - start) {
                    continue 'groups;
                }

                gindex = start_id + (char_code - start);

                /* does first element of group point to `.notdef' glyph? */
                if gindex == 0 {
                    if char_code == 0xFFFFFFFF {
                        break 'groups;
                    }

                    char_code += 1;
                    continue;
                }

                /* if `gindex' is invalid, the remaining values */
                /* in this group are invalid, too               */
                if gindex as FtLong >= c.num_glyphs {
                    gindex = 0;
                    continue 'groups;
                }

                result = char_code;
                break 'groups;
            }
            break;
        }
    }

    *pchar_code = result;
    gindex
}

fn tt_cmap8_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 8,
        language: ft_peek_ulong(&cmap.table, cmap.data + 8) as FtULong,
    }
}

pub static TT_CMAP8_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap8_char_index,
        char_next: tt_cmap8_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 8,
    validate: tt_cmap8_validate,
    get_cmap_info: tt_cmap8_get_info,
    init: tt_cmap_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                          FORMAT 10                            *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME      OFFSET  TYPE               DESCRIPTION
 *
 *   format     0      USHORT             must be 10
 *   reserved   2      USHORT             reserved
 *   length     4      ULONG              length in bytes
 *   language   8      ULONG              Mac language code
 *
 *   start     12      ULONG              first char in range
 *   count     16      ULONG              number of chars in range
 *   glyphIds  20      USHORT[count]      glyph indices covered
 */

fn tt_cmap10_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;
    let mut p = table + 4;

    if table + 20 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let length = ft_next_ulong(t, &mut p) as FtULong;
    p = table + 16;
    let mut count = ft_next_ulong(t, &mut p) as FtULong;

    if length > (valid.limit - table) as FtULong
        /* length < 20 + count * 2 ? */
        || length < 20
        || (length - 20) / 2 < count
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check glyph indices */
    if valid.level >= FT_VALIDATE_TIGHT {
        while count > 0 {
            let gindex = ft_next_ushort(t, &mut p) as FtUInt;
            if gindex >= valid.num_glyphs {
                return Err(FT_ERR_INVALID_GLYPH_INDEX);
            }
            count -= 1;
        }
    }

    Ok(0)
}

fn tt_cmap10_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut result = 0;
    let mut p = c.data + 12;
    let start = ft_next_ulong(t, &mut p);
    let count = ft_next_ulong(t, &mut p);

    if char_code < start {
        return 0;
    }

    let idx = char_code - start;

    if idx < count {
        p += 2 * idx as usize;
        result = ft_peek_ushort(t, p) as FtUInt;
    }

    result
}

fn tt_cmap10_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut gindex: FtUInt = 0;
    let mut p = c.data + 12;
    let start = ft_next_ulong(t, &mut p);
    let count = ft_next_ulong(t, &mut p);

    if *pchar_code == 0xFFFFFFFF {
        return 0;
    }

    let mut char_code = *pchar_code + 1;

    if char_code < start {
        char_code = start;
    }

    let mut idx = char_code - start;
    p += 2 * idx as usize;

    while idx < count {
        gindex = ft_next_ushort(t, &mut p) as FtUInt;
        if gindex != 0 {
            break;
        }

        if char_code == 0xFFFFFFFF {
            return 0;
        }

        char_code += 1;
        idx += 1;
    }

    *pchar_code = char_code;
    gindex
}

fn tt_cmap10_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 10,
        language: ft_peek_ulong(&cmap.table, cmap.data + 8) as FtULong,
    }
}

pub static TT_CMAP10_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap10_char_index,
        char_next: tt_cmap10_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 10,
    validate: tt_cmap10_validate,
    get_cmap_info: tt_cmap10_get_info,
    init: tt_cmap_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                          FORMAT 12                            *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME        OFFSET     TYPE       DESCRIPTION
 *
 *   format      0          USHORT     must be 12
 *   reserved    2          USHORT     reserved
 *   length      4          ULONG      length in bytes
 *   language    8          ULONG      Mac language code
 *   count       12         ULONG      number of groups
 *               16
 *
 * This header is followed by `count' groups of the following format:
 *
 *   start       0          ULONG      first charcode
 *   end         4          ULONG      last charcode
 *   startId     8          ULONG      start glyph ID for the group
 */

fn tt_cmap12_init(cmap: &mut TtCMapData) {
    cmap.cmap12.num_groups = ft_peek_ulong(&cmap.table, cmap.data + 12) as FtULong;
    cmap.cmap12.valid = false;
}

fn tt_cmap12_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;

    if table + 16 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = table + 4;
    let length = ft_next_ulong(t, &mut p) as FtULong;

    p = table + 12;
    let num_groups = ft_next_ulong(t, &mut p) as FtULong;

    if length > (valid.limit - table) as FtULong
        /* length < 16 + 12 * num_groups ? */
        || length < 16
        || (length - 16) / 12 < num_groups
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check groups, they must be in increasing order */
    {
        let mut last: FtULong = 0;

        for n in 0..num_groups {
            let start = ft_next_ulong(t, &mut p) as FtULong;
            let end = ft_next_ulong(t, &mut p) as FtULong;
            let start_id = ft_next_ulong(t, &mut p) as FtULong;

            if start > end {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if n > 0 && start <= last {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if valid.level >= FT_VALIDATE_TIGHT {
                let d = (end - start) as FtUInt32;

                /* start_id + end - start >= TT_VALID_GLYPH_COUNT( valid ) ? */
                if d > valid.num_glyphs || start_id >= (valid.num_glyphs - d) as FtULong {
                    return Err(FT_ERR_INVALID_GLYPH_INDEX);
                }
            }

            last = end;
        }
    }

    Ok(0)
}

/// `tt_cmap12_next`: search the index of the charcode next to
/// cmap->cur_charcode; cmap->cur_group should be set up properly by caller
fn tt_cmap12_next(cmap: &mut TtCMapData) {
    let t = &cmap.table[..];
    let c12 = &mut cmap.cmap12;

    let mut char_code = c12.cur_charcode.wrapping_add(1);

    let mut n = c12.cur_group;
    'groups: while n < c12.num_groups {
        let mut p = cmap.data + 16 + 12 * n as usize;
        let start = ft_next_ulong(t, &mut p) as FtULong;
        let end = ft_next_ulong(t, &mut p) as FtULong;
        let start_id = ft_peek_ulong(t, p) as FtULong;

        if char_code < start {
            char_code = start;
        }

        /* Again: */
        loop {
            if char_code <= end {
                /* ignore invalid group */
                if start_id > 0xFFFFFFFF - (char_code - start) {
                    n += 1;
                    continue 'groups;
                }

                let gindex = (start_id + (char_code - start)) as FtUInt;

                /* does first element of group point to `.notdef' glyph? */
                if gindex == 0 {
                    if char_code == 0xFFFFFFFF {
                        break 'groups;
                    }

                    char_code += 1;
                    continue;
                }

                /* if `gindex' is invalid, the remaining values */
                /* in this group are invalid, too               */
                if gindex as FtLong >= cmap.num_glyphs {
                    n += 1;
                    continue 'groups;
                }

                c12.cur_charcode = char_code;
                c12.cur_gindex = gindex;
                c12.cur_group = n;

                return;
            }
            break;
        }
        n += 1;
    }

    /* Fail: */
    c12.valid = false;
}

fn tt_cmap12_char_map_binary(
    cmap: &mut TtCMapData,
    pchar_code: &mut FtUInt32,
    next: bool,
) -> FtUInt {
    let t = &cmap.table[..];
    let mut gindex: FtUInt = 0;
    let mut p = cmap.data + 12;
    let num_groups = ft_peek_ulong(t, p);
    let char_code = pchar_code.wrapping_add(next as u32);
    let mut end: FtUInt32 = 0;
    let mut max: FtUInt32;
    let mut min: FtUInt32;
    let mut mid: FtUInt32;

    if num_groups == 0 {
        return 0;
    }

    min = 0;
    max = num_groups;

    /* binary search */
    loop {
        mid = (min + max) >> 1;
        p = cmap.data + 16 + 12 * mid as usize;

        let start = ft_next_ulong(t, &mut p);
        end = ft_next_ulong(t, &mut p);

        if char_code < start {
            max = mid;
        } else if char_code > end {
            min = mid + 1;
        } else {
            let start_id = ft_peek_ulong(t, p);

            /* reject invalid glyph index */
            if start_id > 0xFFFFFFFF - (char_code - start) {
                gindex = 0;
            } else {
                gindex = start_id + (char_code - start);
            }
            break;
        }

        if min >= max {
            break;
        }
    }

    if next {
        /* if `char_code' is not in any group, then `mid' is */
        /* the group nearest to `char_code'                  */

        if char_code > end {
            mid += 1;
            if mid == num_groups {
                return 0;
            }
        }

        cmap.cmap12.valid = true;
        cmap.cmap12.cur_charcode = char_code as FtULong;
        cmap.cmap12.cur_group = mid as FtULong;

        if gindex as FtLong >= cmap.num_glyphs {
            gindex = 0;
        }

        if gindex == 0 {
            tt_cmap12_next(cmap);

            if cmap.cmap12.valid {
                gindex = cmap.cmap12.cur_gindex;
            }
        } else {
            cmap.cmap12.cur_gindex = gindex;
        }

        *pchar_code = cmap.cmap12.cur_charcode as FtUInt32;
    }

    gindex
}

fn tt_cmap12_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let mut c = tt(cmap).clone();
    let mut char_code = char_code;
    tt_cmap12_char_map_binary(&mut c, &mut char_code, false)
}

fn tt_cmap12_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let c = tt_mut(cmap);
    let gindex;

    if *pchar_code == 0xFFFFFFFF {
        return 0;
    }

    /* no need to search */
    if c.cmap12.valid && c.cmap12.cur_charcode == *pchar_code as FtULong {
        tt_cmap12_next(c);
        if c.cmap12.valid {
            gindex = c.cmap12.cur_gindex;
            *pchar_code = c.cmap12.cur_charcode as FtUInt32;
        } else {
            gindex = 0;
        }
    } else {
        gindex = tt_cmap12_char_map_binary(c, pchar_code, true);
    }

    gindex
}

fn tt_cmap12_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 12,
        language: ft_peek_ulong(&cmap.table, cmap.data + 8) as FtULong,
    }
}

pub static TT_CMAP12_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap12_char_index,
        char_next: tt_cmap12_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 12,
    validate: tt_cmap12_validate,
    get_cmap_info: tt_cmap12_get_info,
    init: tt_cmap12_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                          FORMAT 13                            *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME        OFFSET     TYPE       DESCRIPTION
 *
 *   format      0          USHORT     must be 13
 *   reserved    2          USHORT     reserved
 *   length      4          ULONG      length in bytes
 *   language    8          ULONG      Mac language code
 *   count       12         ULONG      number of groups
 *               16
 *
 * This header is followed by `count' groups of the following format:
 *
 *   start       0          ULONG      first charcode
 *   end         4          ULONG      last charcode
 *   glyphId     8          ULONG      glyph ID for the whole group
 */

fn tt_cmap13_init(cmap: &mut TtCMapData) {
    cmap.cmap12.num_groups = ft_peek_ulong(&cmap.table, cmap.data + 12) as FtULong;
    cmap.cmap12.valid = false;
}

fn tt_cmap13_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;

    if table + 16 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = table + 4;
    let length = ft_next_ulong(t, &mut p) as FtULong;

    p = table + 12;
    let num_groups = ft_next_ulong(t, &mut p) as FtULong;

    if length > (valid.limit - table) as FtULong
        /* length < 16 + 12 * num_groups ? */
        || length < 16
        || (length - 16) / 12 < num_groups
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check groups, they must be in increasing order */
    {
        let mut last: FtULong = 0;

        for n in 0..num_groups {
            let start = ft_next_ulong(t, &mut p) as FtULong;
            let end = ft_next_ulong(t, &mut p) as FtULong;
            let glyph_id = ft_next_ulong(t, &mut p) as FtULong;

            if start > end {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if n > 0 && start <= last {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if valid.level >= FT_VALIDATE_TIGHT && glyph_id >= valid.num_glyphs as FtULong {
                return Err(FT_ERR_INVALID_GLYPH_INDEX);
            }

            last = end;
        }
    }

    Ok(0)
}

/// `tt_cmap13_next`: search the index of the charcode next to
/// cmap->cur_charcode; cmap->cur_group should be set up properly by caller
fn tt_cmap13_next(cmap: &mut TtCMapData) {
    let t = &cmap.table[..];
    let c13 = &mut cmap.cmap12;

    let mut char_code = c13.cur_charcode.wrapping_add(1);

    for n in c13.cur_group..c13.num_groups {
        let mut p = cmap.data + 16 + 12 * n as usize;
        let start = ft_next_ulong(t, &mut p) as FtULong;
        let end = ft_next_ulong(t, &mut p) as FtULong;
        let glyph_id = ft_peek_ulong(t, p) as FtULong;

        if char_code < start {
            char_code = start;
        }

        if char_code <= end {
            let gindex = glyph_id as FtUInt;

            if gindex != 0 && (gindex as FtLong) < cmap.num_glyphs {
                c13.cur_charcode = char_code;
                c13.cur_gindex = gindex;
                c13.cur_group = n;

                return;
            }
        }
    }

    c13.valid = false;
}

fn tt_cmap13_char_map_binary(
    cmap: &mut TtCMapData,
    pchar_code: &mut FtUInt32,
    next: bool,
) -> FtUInt {
    let t = &cmap.table[..];
    let mut gindex: FtUInt = 0;
    let mut p = cmap.data + 12;
    let num_groups = ft_peek_ulong(t, p);
    let char_code = pchar_code.wrapping_add(next as u32);
    let mut end: FtUInt32 = 0;
    let mut max: FtUInt32;
    let mut min: FtUInt32;
    let mut mid: FtUInt32;

    if num_groups == 0 {
        return 0;
    }

    min = 0;
    max = num_groups;

    /* binary search */
    loop {
        mid = (min + max) >> 1;
        p = cmap.data + 16 + 12 * mid as usize;

        let start = ft_next_ulong(t, &mut p);
        end = ft_next_ulong(t, &mut p);

        if char_code < start {
            max = mid;
        } else if char_code > end {
            min = mid + 1;
        } else {
            gindex = ft_peek_ulong(t, p);

            break;
        }

        if min >= max {
            break;
        }
    }

    if next {
        /* if `char_code' is not in any group, then `mid' is */
        /* the group nearest to `char_code'                  */

        if char_code > end {
            mid += 1;
            if mid == num_groups {
                return 0;
            }
        }

        cmap.cmap12.valid = true;
        cmap.cmap12.cur_charcode = char_code as FtULong;
        cmap.cmap12.cur_group = mid as FtULong;

        if gindex as FtLong >= cmap.num_glyphs {
            gindex = 0;
        }

        if gindex == 0 {
            tt_cmap13_next(cmap);

            if cmap.cmap12.valid {
                gindex = cmap.cmap12.cur_gindex;
            }
        } else {
            cmap.cmap12.cur_gindex = gindex;
        }

        *pchar_code = cmap.cmap12.cur_charcode as FtUInt32;
    }

    gindex
}

fn tt_cmap13_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    let mut c = tt(cmap).clone();
    let mut char_code = char_code;
    tt_cmap13_char_map_binary(&mut c, &mut char_code, false)
}

fn tt_cmap13_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    let c = tt_mut(cmap);
    let gindex;

    if *pchar_code == 0xFFFFFFFF {
        return 0;
    }

    /* no need to search */
    if c.cmap12.valid && c.cmap12.cur_charcode == *pchar_code as FtULong {
        tt_cmap13_next(c);
        if c.cmap12.valid {
            gindex = c.cmap12.cur_gindex;
            *pchar_code = c.cmap12.cur_charcode as FtUInt32;
        } else {
            gindex = 0;
        }
    } else {
        gindex = tt_cmap13_char_map_binary(c, pchar_code, true);
    }

    gindex
}

fn tt_cmap13_get_info(cmap: &TtCMapData) -> TtCMapInfo {
    TtCMapInfo {
        format: 13,
        language: ft_peek_ulong(&cmap.table, cmap.data + 8) as FtULong,
    }
}

pub static TT_CMAP13_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap13_char_index,
        char_next: tt_cmap13_char_next,
        char_var_index: None,
        char_var_default: None,
        variant_list: None,
        charvariant_list: None,
        variantchar_list: None,
    },
    format: 13,
    validate: tt_cmap13_validate,
    get_cmap_info: tt_cmap13_get_info,
    init: tt_cmap13_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                           FORMAT 14                           *****/
/*****                                                               *****/
/*************************************************************************/

/*
 * TABLE OVERVIEW
 * --------------
 *
 *   NAME         OFFSET  TYPE    DESCRIPTION
 *
 *   format         0     USHORT  must be 14
 *   length         2     ULONG   table length in bytes
 *   numSelector    6     ULONG   number of variation sel. records
 *
 * Followed by numSelector records, each of which looks like
 *
 *   varSelector    0     UINT24  Unicode codepoint of sel.
 *   defaultOff     3     ULONG   offset to a default UVS table
 *                                describing any variants to be found in
 *                                the normal Unicode subtable.
 *   nonDefOff      7     ULONG   offset to a non-default UVS table
 *                                describing any variants not in the
 *                                standard cmap, with GIDs here
 * (either offset may be 0 NULL)
 *
 * Selectors are sorted by code point.
 *
 * A default Unicode Variation Selector (UVS) subtable is just a list of
 * ranges of code points which are to be found in the standard cmap.  No
 * glyph IDs (GIDs) here.
 *
 *   numRanges      0     ULONG   number of ranges following
 *
 * A range looks like
 *
 *   uniStart       0     UINT24  code point of the first character in
 *                                this range
 *   additionalCnt  3     UBYTE   count of additional characters in this
 *                                range (zero means a range of a single
 *                                character)
 *
 * Ranges are sorted by `uniStart'.
 *
 * A non-default Unicode Variation Selector (UVS) subtable is a list of
 * mappings from codepoint to GID.
 *
 *   numMappings    0     ULONG   number of mappings
 *
 * A range looks like
 *
 *   uniStart       0     UINT24  code point of the first character in
 *                                this range
 *   GID            3     USHORT  and its GID
 *
 * Ranges are sorted by `uniStart'.
 */

/// `tt_cmap14_ensure`: the results array (C keeps it in the charmap and
/// overwrites it on each call; here each call returns its own)
fn tt_cmap14_ensure(num_results: FtUInt32) -> Option<Vec<FtUInt32>> {
    let mut v: Vec<FtUInt32> = ft_new_array(num_results as FtLong).ok()?;
    v.clear();
    Some(v)
}

fn tt_cmap14_init(cmap: &mut TtCMapData) {
    cmap.num_selectors = ft_peek_ulong(&cmap.table, cmap.data + 6) as FtULong;
}

fn tt_cmap14_validate(table: usize, valid: &TtValidatorRec) -> FtResult<FtInt> {
    let t = valid.base;

    if table + 2 + 4 + 4 > valid.limit {
        return Err(FT_ERR_INVALID_TABLE);
    }

    let mut p = table + 2;
    let length = ft_next_ulong(t, &mut p) as FtULong;
    let num_selectors = ft_next_ulong(t, &mut p) as FtULong;

    if length > (valid.limit - table) as FtULong
        /* length < 10 + 11 * num_selectors ? */
        || length < 10
        || (length - 10) / 11 < num_selectors
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* check selectors, they must be in increasing order */
    {
        /* we start lastVarSel at 1 because a variant selector value of 0
         * isn't valid.
         */
        let mut last_var_sel: FtULong = 1;

        for _ in 0..num_selectors {
            let var_sel = ft_next_uoff3(t, &mut p) as FtULong;
            let def_off = ft_next_ulong(t, &mut p) as FtULong;
            let nondef_off = ft_next_ulong(t, &mut p) as FtULong;

            if def_off >= length || nondef_off >= length {
                return Err(FT_ERR_INVALID_TABLE);
            }

            if var_sel < last_var_sel {
                return Err(FT_ERR_INVALID_TABLE);
            }

            last_var_sel = var_sel + 1;

            /* check the default table (these glyphs should be reached     */
            /* through the normal Unicode cmap, no GIDs, just check order) */
            if def_off != 0 {
                let mut defp = table + def_off as usize;
                let mut last_base: FtULong = 0;

                if defp + 4 > valid.limit {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                let num_ranges = ft_next_ulong(t, &mut defp) as FtULong;

                /* defp + numRanges * 4 > valid->limit ? */
                if num_ranges > ((valid.limit - defp) / 4) as FtULong {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                for _ in 0..num_ranges {
                    let base = ft_next_uoff3(t, &mut defp) as FtULong;
                    let cnt = ft_next_byte(t, &mut defp) as FtULong;

                    if base + cnt >= 0x110000 {
                        /* end of Unicode */
                        return Err(FT_ERR_INVALID_TABLE);
                    }

                    if base < last_base {
                        return Err(FT_ERR_INVALID_TABLE);
                    }

                    last_base = base + cnt + 1;
                }
            }

            /* and the non-default table (these glyphs are specified here) */
            if nondef_off != 0 {
                let mut ndp = table + nondef_off as usize;
                let mut last_uni: FtULong = 0;

                if ndp + 4 > valid.limit {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                let num_mappings = ft_next_ulong(t, &mut ndp) as FtULong;

                /* numMappings * 5 > (FT_ULong)( valid->limit - ndp ) ? */
                if num_mappings > ((valid.limit - ndp) as FtULong) / 5 {
                    return Err(FT_ERR_INVALID_TABLE);
                }

                for _ in 0..num_mappings {
                    let uni = ft_next_uoff3(t, &mut ndp) as FtULong;
                    let gid = ft_next_ushort(t, &mut ndp) as FtULong;

                    if uni >= 0x110000 {
                        /* end of Unicode */
                        return Err(FT_ERR_INVALID_TABLE);
                    }

                    if uni < last_uni {
                        return Err(FT_ERR_INVALID_TABLE);
                    }

                    last_uni = uni + 1;

                    if valid.level >= FT_VALIDATE_TIGHT && gid >= valid.num_glyphs as FtULong {
                        return Err(FT_ERR_INVALID_GLYPH_INDEX);
                    }
                }
            }
        }
    }

    Ok(0)
}

fn tt_cmap14_char_index(_cmap: &FtCMapRec, _char_code: FtUInt32) -> FtUInt {
    /* This can't happen */
    0
}

fn tt_cmap14_char_next(_cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    /* This can't happen */
    *pchar_code = 0;
    0
}

fn tt_cmap14_get_info(_cmap: &TtCMapData) -> TtCMapInfo {
    /* subtable 14 does not define a language field */
    TtCMapInfo {
        format: 14,
        language: 0xFFFFFFFF,
    }
}

fn tt_cmap14_char_map_def_binary(t: &[u8], base: usize, char_code: FtUInt32) -> bool {
    let num_ranges = ft_peek_ulong(t, base);
    let mut min: FtUInt32 = 0;
    let mut max: FtUInt32 = num_ranges;

    let base = base + 4;

    /* binary search */
    while min < max {
        let mid = (min + max) >> 1;
        let mut p = base + 4 * mid as usize;
        let start = ft_next_uoff3(t, &mut p) as FtULong;
        let cnt = ft_next_byte(t, &mut p) as FtULong;

        if (char_code as FtULong) < start {
            max = mid;
        } else if char_code as FtULong > start + cnt {
            min = mid + 1;
        } else {
            return true;
        }
    }

    false
}

fn tt_cmap14_char_map_nondef_binary(t: &[u8], base: usize, char_code: FtUInt32) -> FtUInt {
    let num_mappings = ft_peek_ulong(t, base);
    let mut min: FtUInt32 = 0;
    let mut max: FtUInt32 = num_mappings;

    let base = base + 4;

    /* binary search */
    while min < max {
        let mid = (min + max) >> 1;
        let mut p = base + 5 * mid as usize;
        let uni = ft_next_uoff3(t, &mut p);

        if char_code < uni {
            max = mid;
        } else if char_code > uni {
            min = mid + 1;
        } else {
            return ft_peek_ushort(t, p) as FtUInt;
        }
    }

    0
}

fn tt_cmap14_find_variant(t: &[u8], base: usize, variant_code: FtUInt32) -> Option<usize> {
    let num_var = ft_peek_ulong(t, base);
    let mut min: FtUInt32 = 0;
    let mut max: FtUInt32 = num_var;

    let base = base + 4;

    /* binary search */
    while min < max {
        let mid = (min + max) >> 1;
        let mut p = base + 11 * mid as usize;
        let var_sel = ft_next_uoff3(t, &mut p) as FtULong;

        if (variant_code as FtULong) < var_sel {
            max = mid;
        } else if variant_code as FtULong > var_sel {
            min = mid + 1;
        } else {
            return Some(p);
        }
    }

    None
}

fn tt_cmap14_char_var_index(
    cmap: &FtCMapRec,
    ucmap: &FtCMapRec,
    charcode: FtUInt32,
    variant_selector: FtUInt32,
) -> FtUInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut p = match tt_cmap14_find_variant(t, c.data + 6, variant_selector) {
        Some(p) => p,
        None => return 0,
    };

    let def_off = ft_next_ulong(t, &mut p) as usize;
    let nondef_off = ft_peek_ulong(t, p) as usize;

    if def_off != 0 && tt_cmap14_char_map_def_binary(t, c.data + def_off, charcode) {
        /* This is the default variant of this charcode.  GID not stored */
        /* here; stored in the normal Unicode charmap instead.           */
        return (ucmap.clazz.char_index)(ucmap, charcode);
    }

    if nondef_off != 0 {
        return tt_cmap14_char_map_nondef_binary(t, c.data + nondef_off, charcode);
    }

    0
}

fn tt_cmap14_char_var_isdefault(
    cmap: &FtCMapRec,
    charcode: FtUInt32,
    variant_selector: FtUInt32,
) -> FtInt {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut p = match tt_cmap14_find_variant(t, c.data + 6, variant_selector) {
        Some(p) => p,
        None => return -1,
    };

    let def_off = ft_next_ulong(t, &mut p) as usize;
    let nondef_off = ft_next_ulong(t, &mut p) as usize;

    if def_off != 0 && tt_cmap14_char_map_def_binary(t, c.data + def_off, charcode) {
        return 1;
    }

    if nondef_off != 0 && tt_cmap14_char_map_nondef_binary(t, c.data + nondef_off, charcode) != 0 {
        return 0;
    }

    -1
}

fn tt_cmap14_variants(cmap: &FtCMapRec) -> Option<Vec<FtUInt32>> {
    let c = tt(cmap);
    let t = &c.table[..];
    let count = c.num_selectors as FtUInt32;
    let mut p = c.data + 10;

    let mut result = tt_cmap14_ensure(count.wrapping_add(1))?;

    for _ in 0..count {
        result.push(ft_next_uoff3(t, &mut p));
        p += 8;
    }
    result.push(0);

    Some(result)
}

fn tt_cmap14_char_variants(cmap: &FtCMapRec, char_code: FtUInt32) -> Option<Vec<FtUInt32>> {
    let c = tt(cmap);
    let t = &c.table[..];
    let count = c.num_selectors as FtUInt32;
    let mut p = c.data + 10;

    let mut q = tt_cmap14_ensure(count.wrapping_add(1))?;

    for _ in 0..count {
        let var_sel = ft_next_uoff3(t, &mut p);
        let def_off = ft_next_ulong(t, &mut p) as usize;
        let nondef_off = ft_next_ulong(t, &mut p) as usize;

        if (def_off != 0 && tt_cmap14_char_map_def_binary(t, c.data + def_off, char_code))
            || (nondef_off != 0
                && tt_cmap14_char_map_nondef_binary(t, c.data + nondef_off, char_code) != 0)
        {
            q.push(var_sel);
        }
    }
    q.push(0);

    Some(q)
}

fn tt_cmap14_def_char_count(t: &[u8], mut p: usize) -> FtUInt {
    let mut num_ranges = ft_next_ulong(t, &mut p);
    let mut tot: FtUInt = 0;

    p += 3; /* point to the first `cnt' field */
    while num_ranges > 0 {
        tot = tot.wrapping_add(1 + byte_at(t, p) as FtUInt);
        p += 4;
        num_ranges -= 1;
    }

    tot
}

fn tt_cmap14_get_def_chars(t: &[u8], mut p: usize) -> Option<Vec<FtUInt32>> {
    let cnt = tt_cmap14_def_char_count(t, p);
    let mut num_ranges = ft_next_ulong(t, &mut p);

    let mut q = tt_cmap14_ensure(cnt.wrapping_add(1))?;

    while num_ranges > 0 {
        let mut uni = ft_next_uoff3(t, &mut p);

        let mut cnt = ft_next_byte(t, &mut p) as FtUInt + 1;
        loop {
            q.push(uni);
            uni = uni.wrapping_add(1);
            cnt -= 1;
            if cnt == 0 {
                break;
            }
        }
        num_ranges -= 1;
    }
    q.push(0);

    Some(q)
}

fn tt_cmap14_get_nondef_chars(t: &[u8], mut p: usize) -> Option<Vec<FtUInt32>> {
    let num_mappings = ft_next_ulong(t, &mut p);

    let mut ret = tt_cmap14_ensure(num_mappings.wrapping_add(1))?;

    for _ in 0..num_mappings {
        ret.push(ft_next_uoff3(t, &mut p));
        p += 2;
    }
    ret.push(0);

    Some(ret)
}

fn tt_cmap14_variant_chars(cmap: &FtCMapRec, variant_selector: FtUInt32) -> Option<Vec<FtUInt32>> {
    let c = tt(cmap);
    let t = &c.table[..];
    let mut p = tt_cmap14_find_variant(t, c.data + 6, variant_selector)?;

    let def_off = ft_next_ulong(t, &mut p) as usize;
    let nondef_off = ft_next_ulong(t, &mut p) as usize;

    if def_off == 0 && nondef_off == 0 {
        return None;
    }

    if def_off == 0 {
        tt_cmap14_get_nondef_chars(t, c.data + nondef_off)
    } else if nondef_off == 0 {
        tt_cmap14_get_def_chars(t, c.data + def_off)
    } else {
        /* Both a default and a non-default glyph set?  That's probably not */
        /* good font design, but the spec allows for it...                  */
        let mut p = c.data + nondef_off;
        let mut dp = c.data + def_off;

        let num_mappings = ft_next_ulong(t, &mut p);
        let mut dcnt = tt_cmap14_def_char_count(t, dp);
        let num_ranges = ft_next_ulong(t, &mut dp);

        if num_mappings == 0 {
            return tt_cmap14_get_def_chars(t, c.data + def_off);
        }
        if dcnt == 0 {
            return tt_cmap14_get_nondef_chars(t, c.data + nondef_off);
        }

        let mut ret = tt_cmap14_ensure(dcnt.wrapping_add(num_mappings).wrapping_add(1))?;

        let mut duni = ft_next_uoff3(t, &mut dp);
        dcnt = ft_next_byte(t, &mut dp) as FtUInt;
        let mut di: FtUInt = 1;
        let mut nuni = ft_next_uoff3(t, &mut p);
        p += 2;
        let mut ni: FtUInt = 1;

        loop {
            if nuni > duni.wrapping_add(dcnt) {
                for k in 0..=dcnt {
                    ret.push(duni.wrapping_add(k));
                }

                di += 1;

                if di > num_ranges {
                    break;
                }

                duni = ft_next_uoff3(t, &mut dp);
                dcnt = ft_next_byte(t, &mut dp) as FtUInt;
            } else {
                if nuni < duni {
                    ret.push(nuni);
                }

                /* If it is within the default range then ignore it -- */
                /* that should not have happened                       */
                ni += 1;
                if ni > num_mappings {
                    break;
                }

                nuni = ft_next_uoff3(t, &mut p);
                p += 2;
            }
        }

        if ni <= num_mappings {
            /* If we get here then we have run out of all default ranges.   */
            /* We have read one non-default mapping which we haven't stored */
            /* and there may be others that need to be read.                */
            ret.push(nuni);
            while ni < num_mappings {
                ret.push(ft_next_uoff3(t, &mut p));
                p += 2;
                ni += 1;
            }
        } else if di <= num_ranges {
            /* If we get here then we have run out of all non-default     */
            /* mappings.  We have read one default range which we haven't */
            /* stored and there may be others that need to be read.       */
            for k in 0..=dcnt {
                ret.push(duni.wrapping_add(k));
            }

            while di < num_ranges {
                duni = ft_next_uoff3(t, &mut dp);
                dcnt = ft_next_byte(t, &mut dp) as FtUInt;

                for k in 0..=dcnt {
                    ret.push(duni.wrapping_add(k));
                }
                di += 1;
            }
        }

        ret.push(0);

        Some(ret)
    }
}

pub static TT_CMAP14_CLASS_REC: TtCMapClassRec = TtCMapClassRec {
    clazz: FtCMapClassRec {
        char_index: tt_cmap14_char_index,
        char_next: tt_cmap14_char_next,
        /* Format 14 extension functions */
        char_var_index: Some(tt_cmap14_char_var_index),
        char_var_default: Some(tt_cmap14_char_var_isdefault),
        variant_list: Some(tt_cmap14_variants),
        charvariant_list: Some(tt_cmap14_char_variants),
        variantchar_list: Some(tt_cmap14_variant_chars),
    },
    format: 14,
    validate: tt_cmap14_validate,
    get_cmap_info: tt_cmap14_get_info,
    init: tt_cmap14_init,
};

/*************************************************************************/
/*****                                                               *****/
/*****                       SYNTHETIC UNICODE                       *****/
/*****                                                               *****/
/*************************************************************************/

/*        This charmap is generated using postscript glyph names.        */

/// `tt_cmap_unicode_init`: builds the synthetic Unicode charmap's table
/// from the face's glyph names (`tt_get_glyph_name`).
pub fn tt_cmap_unicode_init(
    face: &mut TtFaceRec,
    stream: &mut FtStreamRec,
) -> FtResult<PsUnicodesRec> {
    let mut unicodes = PsUnicodesRec::default();
    let num_glyphs = face.root.num_glyphs as FtUInt;

    /* tt_get_glyph_name */
    let mut get_glyph_name =
        |idx: FtUInt| -> Option<Vec<u8>> { tt_face_get_ps_name(face, stream, idx).ok() };

    ps_unicodes_init(&mut unicodes, num_glyphs, &mut get_glyph_name)?;
    Ok(unicodes)
}

fn tt_cmap_unicode_char_index(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt {
    match &cmap.data {
        FtCMapData::PsUnicodes(u) => ps_unicodes_char_index(u, char_code),
        _ => 0,
    }
}

fn tt_cmap_unicode_char_next(cmap: &mut FtCMapRec, pchar_code: &mut FtUInt32) -> FtUInt {
    match &cmap.data {
        FtCMapData::PsUnicodes(u) => ps_unicodes_char_next(u, pchar_code),
        _ => 0,
    }
}

/// `tt_cmap_unicode_class_rec`
pub static TT_CMAP_UNICODE_CLASS_REC: FtCMapClassRec = FtCMapClassRec {
    char_index: tt_cmap_unicode_char_index,
    char_next: tt_cmap_unicode_char_next,
    char_var_index: None,
    char_var_default: None,
    variant_list: None,
    charvariant_list: None,
    variantchar_list: None,
};

/// `tt_cmap_classes` (ttcmapc.h)
static TT_CMAP_CLASSES: [&TtCMapClassRec; 9] = [
    &TT_CMAP0_CLASS_REC,
    &TT_CMAP2_CLASS_REC,
    &TT_CMAP4_CLASS_REC,
    &TT_CMAP6_CLASS_REC,
    &TT_CMAP8_CLASS_REC,
    &TT_CMAP10_CLASS_REC,
    &TT_CMAP12_CLASS_REC,
    &TT_CMAP13_CLASS_REC,
    &TT_CMAP14_CLASS_REC,
];

/// `tt_face_build_cmaps`: parse the `cmap' table and build the
/// corresponding TT_CMap objects in the current face
pub fn tt_face_build_cmaps(face: &mut TtFaceRec) -> FtResult<()> {
    let table = match face.cmap_table.clone() {
        Some(t) if face.cmap_size >= 4 => t,
        _ => return Err(FT_ERR_INVALID_TABLE),
    };
    let t = &table[..];

    /* Version 1.8.3 of the OpenType specification contains the following */
    /* (https://docs.microsoft.com/en-us/typography/opentype/spec/cmap):  */
    /*                                                                    */
    /*   The 'cmap' table version number remains at 0x0000 for fonts that */
    /*   make use of the newer subtable formats.                          */
    /*                                                                    */
    /* This essentially means that a version format test is useless.      */

    /* ignore format */
    let mut p: usize = 2;

    let mut num_cmaps = ft_next_ushort(t, &mut p) as FtUInt;

    let limit = face.cmap_size as usize;
    while num_cmaps > 0 && p + 8 <= limit {
        let mut charmap = FtCharMapRec {
            platform_id: ft_next_ushort(t, &mut p),
            encoding_id: ft_next_ushort(t, &mut p),
            encoding: FT_ENCODING_NONE, /* will be filled later */
        };
        let offset = ft_next_ulong(t, &mut p) as FtULong;

        if offset != 0 && offset <= face.cmap_size - 2 {
            let cmap = offset as usize;
            let format = ft_peek_ushort(t, cmap) as FtUInt;

            if let Some(clazz) = TT_CMAP_CLASSES.iter().copied().find(|c| c.format == format) {
                let valid = TtValidatorRec {
                    base: t,
                    limit,
                    level: FT_VALIDATE_DEFAULT,
                    num_glyphs: face.max_profile.numGlyphs as FtUInt,
                };

                /* validate this cmap sub-table */
                if let Ok(flags) = (clazz.validate)(cmap, &valid) {
                    /* It might make sense to store the single variation         */
                    /* selector cmap somewhere special.  But it would have to be */
                    /* in the public FT_FaceRec, and we can't change that.       */

                    let mut data = TtCMapData {
                        table: table.clone(),
                        data: cmap,
                        flags: 0,
                        tt_class: clazz,
                        num_glyphs: face.root.num_glyphs,
                        cmap4: TtCMap4State::default(),
                        cmap12: TtCMap12State::default(),
                        num_selectors: 0,
                    };
                    (clazz.init)(&mut data);

                    /* it is simpler to directly set `flags' than adding */
                    /* a parameter to FT_CMap_New                        */
                    data.flags = flags;

                    charmap.encoding = FT_ENCODING_NONE;
                    let _ =
                        ft_cmap_new(&mut face.root, &clazz.clazz, FtCMapData::Tt(data), charmap);
                }
            }
        }

        num_cmaps -= 1;
    }

    Ok(())
}

/// `tt_get_cmap_info`
pub fn tt_get_cmap_info(face: &FtFaceRec, charmap: usize) -> FtResult<TtCMapInfo> {
    match face.charmaps.get(charmap).map(|c| &c.data) {
        Some(FtCMapData::Tt(d)) => Ok((d.tt_class.get_cmap_info)(d)),
        _ => Err(FT_ERR_INVALID_CHARMAP_FORMAT),
    }
}
