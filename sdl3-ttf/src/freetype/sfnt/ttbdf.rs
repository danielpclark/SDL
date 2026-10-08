// Rust translation of src/sfnt/ttbdf.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2005-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType and OpenType embedded BDF properties (body).

use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttload::tt_face_goto_table;

/// `BDF_PropertyRec` (ftbdf.h)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BdfPropertyRec {
    /// `BDF_PROPERTY_TYPE_NONE`
    None,
    /// `BDF_PROPERTY_TYPE_ATOM` (the string, without its terminating NUL)
    Atom(Vec<u8>),
    /// `BDF_PROPERTY_TYPE_INTEGER`
    Integer(FtInt32),
    /// `BDF_PROPERTY_TYPE_CARDINAL`
    Cardinal(FtUInt32),
}

/// `tt_face_free_bdf_props`
pub fn tt_face_free_bdf_props(face: &mut TtFaceRec) {
    let bdf = &mut face.bdf;

    if bdf.loaded {
        bdf.table = Vec::new();
        bdf.strings = 0;
        bdf.strings_size = 0;
    }
}

/// `tt_face_load_bdf_props`
fn tt_face_load_bdf_props(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    face.bdf = TtBdfRec::default();

    let length = match tt_face_goto_table(face, TTAG_BDF as FtULong, stream) {
        Ok(length) if length >= 8 => length,
        _ => return Err(FT_ERR_INVALID_TABLE),
    };
    match stream.extract_frame(length) {
        Ok(t) => face.bdf.table = t,
        Err(_) => return Err(FT_ERR_INVALID_TABLE),
    }

    let bdf = &mut face.bdf;
    {
        let t = &bdf.table[..];
        let mut p = 0usize;
        let version = ft_next_ushort(t, &mut p) as FtUInt;
        let num_strikes = ft_next_ushort(t, &mut p) as FtUInt;
        let strings = ft_next_ulong(t, &mut p) as FtULong;

        if version != 0x0001
            || strings < 8
            || (strings - 8) / 4 < num_strikes as FtULong
            || strings + 1 > length
        {
            /* BadTable: */
            *bdf = TtBdfRec::default();
            return Err(FT_ERR_INVALID_TABLE);
        }

        let mut count = num_strikes;
        let mut p = 8usize;
        let mut strike = p + count as usize * 4;

        while count > 0 {
            let num_items = ft_peek_ushort(t, p + 2) as FtUInt;

            /*
             * We don't need to check the value sets themselves, since this
             * is done later.
             */
            strike += 10 * num_items as usize;

            p += 4;
            count -= 1;
        }

        if strike > strings as usize {
            /* BadTable: */
            *bdf = TtBdfRec::default();
            return Err(FT_ERR_INVALID_TABLE);
        }

        bdf.num_strikes = num_strikes;
        bdf.strings = strings as usize;
        bdf.strings_size = length - strings;
    }

    bdf.loaded = true;

    Ok(())
}

/// `tt_face_find_bdf_prop`
pub fn tt_face_find_bdf_prop(
    ttface: &mut TtFaceRec,
    stream: &mut FtStreamRec,
    property_name: &[u8],
) -> FtResult<BdfPropertyRec> {
    if !ttface.bdf.loaded {
        tt_face_load_bdf_props(ttface, stream)?;
    }

    let has_size = ttface.root.has_slot_and_size;
    let y_ppem = ttface.root.size.metrics.y_ppem as FtUInt;
    let bdf = &ttface.bdf;
    let t = &bdf.table[..];

    let mut count = bdf.num_strikes;
    let mut p = 8usize;
    let mut strike = p + 4 * count as usize;

    if !has_size {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let property_len = property_name.len();
    if property_len == 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let mut found = false;
    while count > 0 {
        let ppem = ft_next_ushort(t, &mut p) as FtUInt;
        let cnt = ft_next_ushort(t, &mut p) as FtUInt;

        if ppem == y_ppem {
            count = cnt;
            found = true;
            break;
        }

        strike += 10 * cnt as usize;
        count -= 1;
    }

    if !found {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* FoundStrike: */
    let strings = t.get(bdf.strings..).unwrap_or(&[]);
    let strings_size = bdf.strings_size as usize;
    let mut p = strike;
    while count > 0 {
        let type_ = ft_peek_ushort(t, p + 4) as FtUInt;

        if (type_ & 0x10) != 0 {
            let name_offset = ft_peek_ulong(t, p) as usize;
            let value = ft_peek_ulong(t, p + 6);

            /* be a bit paranoid for invalid entries here */
            if name_offset < strings_size
                && property_len < strings_size - name_offset
                && strncmp(property_name, &strings[name_offset..strings_size])
            {
                match type_ & 0x0F {
                    0x00 | 0x01 => {
                        /* string */
                        /* atoms */
                        /* check that the content is really 0-terminated */
                        /* FIXME (upstream): C searches `strings_size' bytes from */
                        /* `strings + value', reading past the table's end; the   */
                        /* search stops at the end of the table here.             */
                        if (value as usize) < strings_size {
                            let s = &strings[value as usize..];
                            if let Some(end) = s.iter().position(|&c| c == 0) {
                                return Ok(BdfPropertyRec::Atom(s[..end].to_vec()));
                            }
                        }
                    }

                    0x02 => return Ok(BdfPropertyRec::Integer(value as FtInt32)),

                    0x03 => return Ok(BdfPropertyRec::Cardinal(value)),

                    _ => {}
                }
            }
        }

        p += 10;
        count -= 1;
    }

    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `ft_strncmp( name, s, n ) == 0` with `s` holding the `n` bytes.
fn strncmp(name: &[u8], s: &[u8]) -> bool {
    for (i, &c) in s.iter().enumerate() {
        let a = name.get(i).copied().unwrap_or(0);
        if a != c {
            return false;
        }
        if a == 0 {
            return true;
        }
    }
    true
}
