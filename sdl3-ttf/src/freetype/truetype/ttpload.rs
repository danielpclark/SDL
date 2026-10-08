// Rust translation of src/truetype/ttpload.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType-specific tables loader (body).

use std::sync::Arc;

use super::super::base::ftmemory::ft_new_array;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::super::sfnt::ttload::tt_face_goto_table;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttgxvar::tt_face_vary_cvt;

/// `tt_face_load_loca`: Load the locations table.
pub fn tt_face_load_loca(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* we need the size of the `glyf' table for malformed `loca' tables */
    match tt_face_goto_table(face, TTAG_glyf as FtULong, stream) {
        /* it is possible that a font doesn't have a glyf table at all */
        /* or its size is zero                                         */
        Err(e) if ft_err_eq(e, FT_ERR_TABLE_MISSING) => {
            face.glyf_len = 0;
            face.glyf_offset = 0;
        }
        Err(e) => return Err(e),
        Ok(len) => {
            face.glyf_len = len;
            face.glyf_offset = stream.pos();
        }
    }

    let mut table_len = match tt_face_goto_table(face, TTAG_loca as FtULong, stream) {
        Ok(len) => len,
        Err(_) => return Err(FT_ERR_LOCATIONS_MISSING),
    };

    let shift = if face.header.Index_To_Loc_Format != 0 {
        2
    } else {
        1
    };

    if table_len > 0x10000 << shift {
        table_len = 0x10000 << shift;
    }

    face.num_locations = table_len >> shift;

    if face.num_locations != face.root.num_glyphs as FtULong + 1 {
        /* we only handle the case where `maxp' gives a larger value */
        if face.num_locations < face.root.num_glyphs as FtULong + 1 {
            let new_loca_len = (face.root.num_glyphs as FtULong + 1) << shift;

            let pos = stream.pos() as FtLong;
            let mut dist: FtLong = 0x7FFFFFFF;
            let mut found = false;

            /* compute the distance to next table in font file */
            for entry in &face.dir_tables[..face.num_tables as usize] {
                let diff = entry.Offset as FtLong - pos;

                if diff > 0 && diff < dist {
                    dist = diff;
                    found = true;
                }
            }

            if !found {
                /* `loca' is the last table */
                dist = stream.size as FtLong - pos;
            }

            if new_loca_len <= dist as FtULong {
                face.num_locations = face.root.num_glyphs as FtULong + 1;
                table_len = new_loca_len;
            } else {
                face.root.num_glyphs = if face.num_locations != 0 {
                    face.num_locations as FtLong - 1
                } else {
                    0
                };
            }
        }
    }

    /*
     * Extract the frame.  We don't need to decompress it since
     * we are able to parse it directly.
     */
    face.glyph_locations = stream.extract_frame(table_len)?;

    Ok(())
}

/// `tt_face_get_location`; returns the glyph's offset in the `glyf'
/// table and its size.
pub fn tt_face_get_location(ttface: &TtFaceRec, gindex: FtUInt) -> (FtULong, FtULong) {
    let mut pos1: FtULong = 0;
    let mut pos2: FtULong = 0;
    let asize: FtULong;
    let loca = &ttface.glyph_locations[..];

    if (gindex as FtULong) < ttface.num_locations {
        if ttface.header.Index_To_Loc_Format != 0 {
            let mut p = gindex as usize * 4;
            let p_limit = ttface.num_locations as usize * 4;

            pos1 = ft_next_ulong(loca, &mut p) as FtULong;
            pos2 = pos1;

            if p + 4 <= p_limit {
                pos2 = ft_next_ulong(loca, &mut p) as FtULong;
            }
        } else {
            let mut p = gindex as usize * 2;
            let p_limit = ttface.num_locations as usize * 2;

            pos1 = ft_next_ushort(loca, &mut p) as FtULong;
            pos2 = pos1;

            if p + 2 <= p_limit {
                pos2 = ft_next_ushort(loca, &mut p) as FtULong;
            }

            pos1 <<= 1;
            pos2 <<= 1;
        }
    }

    /* Check broken location data. */
    if pos1 > ttface.glyf_len {
        return (0, 0);
    }

    if pos2 > ttface.glyf_len {
        /* We try to sanitize the last `loca' entry. */
        if gindex as FtULong == ttface.num_locations.wrapping_sub(2) {
            pos2 = ttface.glyf_len;
        } else {
            return (0, 0);
        }
    }

    /* The `loca' table must be ordered; it refers to the length of */
    /* an entry as the difference between the current and the next  */
    /* position.  However, there do exist (malformed) fonts which   */
    /* don't obey this rule, so we are only able to provide an      */
    /* upper bound for the size.                                    */
    /*                                                              */
    /* We get (intentionally) a wrong, non-zero result in case the  */
    /* `glyf' table is missing.                                     */
    if pos2 >= pos1 {
        asize = pos2 - pos1;
    } else {
        asize = ttface.glyf_len - pos1;
    }

    (pos1, asize)
}

/// `tt_face_done_loca`
pub fn tt_face_done_loca(face: &mut TtFaceRec) {
    face.glyph_locations = Vec::new();
    face.num_locations = 0;
}

/// `tt_face_load_cvt`: Load the control value table into a face object.
pub fn tt_face_load_cvt(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let table_len = match tt_face_goto_table(face, TTAG_cvt as FtULong, stream) {
        Ok(len) => len,
        Err(_) => {
            face.cvt_size = 0;
            face.cvt = Vec::new();
            return Ok(());
        }
    };

    face.cvt_size = table_len / 2;
    face.cvt = ft_new_array(face.cvt_size as FtLong)?;

    stream.enter_frame(face.cvt_size * 2)?;

    for cur in face.cvt.iter_mut() {
        *cur = stream.get_short() as FtInt32 * 64;
    }

    stream.exit_frame();

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    if face.doblend {
        return tt_face_vary_cvt(face, stream);
    }

    Ok(())
}

/// `tt_face_load_fpgm`: Load the font program.
pub fn tt_face_load_fpgm(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* The font program is optional */
    match tt_face_goto_table(face, TTAG_fpgm as FtULong, stream) {
        Err(_) => {
            face.font_program = Arc::from(Vec::new());
            face.font_program_size = 0;
        }
        Ok(table_len) => {
            face.font_program_size = table_len;
            face.font_program = Arc::from(stream.extract_frame(table_len)?);
        }
    }

    Ok(())
}

/// `tt_face_load_prep`: Load the cvt program.
pub fn tt_face_load_prep(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    match tt_face_goto_table(face, TTAG_prep as FtULong, stream) {
        Err(_) => {
            face.cvt_program = Arc::from(Vec::new());
            face.cvt_program_size = 0;
        }
        Ok(table_len) => {
            face.cvt_program_size = table_len;
            face.cvt_program = Arc::from(stream.extract_frame(table_len)?);
        }
    }

    Ok(())
}

/// `tt_face_load_hdmx`: Load the `hdmx' table into the face object.
pub fn tt_face_load_hdmx(face: &mut TtFaceRec, stream: &mut FtStreamRec) -> FtResult<()> {
    /* this table is optional */
    let table_size = match tt_face_goto_table(face, TTAG_hdmx as FtULong, stream) {
        Ok(size) if size >= 8 => size,
        _ => return Ok(()),
    };

    face.hdmx_table = stream.extract_frame(table_size)?;

    let t = &face.hdmx_table[..];
    let mut p = 0usize;
    let limit = table_size as usize;

    /* Given that `hdmx' tables are losing its importance (for example, */
    /* variation fonts introduced in OpenType 1.8 must not have this    */
    /* table) we no longer test for a correct `version' field.          */
    p += 2;
    let num_records = ft_next_ushort(t, &mut p) as FtUInt;
    let mut record_size = ft_next_ulong(t, &mut p) as FtULong;

    /* There are at least two fonts, HANNOM-A and HANNOM-B version */
    /* 2.0 (2005), which get this wrong: The upper two bytes of    */
    /* the size value are set to 0xFF instead of 0x00.  We catch   */
    /* and fix this.                                               */
    if record_size >= 0xFFFF0000 {
        record_size &= 0xFFFF;
    }

    let fail = |face: &mut TtFaceRec| {
        face.hdmx_table = Vec::new();
        face.hdmx_table_size = 0;
        Ok(())
    };

    /* The limit for `num_records' is a heuristic value. */
    if num_records > 255 || num_records == 0 {
        return fail(face);
    }

    /* Out-of-spec tables are rejected.  The record size must be */
    /* equal to the number of glyphs + 2 + 32-bit padding.       */
    if record_size as FtLong != ((face.root.num_glyphs + 2 + 3) & !3) {
        return fail(face);
    }

    let mut records: Vec<usize> = match ft_new_array(num_records as FtLong) {
        Ok(r) => r,
        Err(_) => return fail(face),
    };

    let mut nn = 0usize;
    while nn < num_records as usize {
        if p + record_size as usize > limit {
            break;
        }

        records[nn] = p;
        p += record_size as usize;
        nn += 1;
    }

    /* The records must be already sorted by ppem but it does not */
    /* hurt to make sure so that the binary search works later.   */
    /* (glibc's qsort is a stable merge sort for arrays like this one) */
    records[..nn].sort_by_key(|&r| t[r]);

    face.hdmx_records = records;
    face.hdmx_record_count = nn as FtUInt;
    face.hdmx_table_size = table_size;
    face.hdmx_record_size = record_size;

    Ok(())
}

/// `tt_face_free_hdmx`
pub fn tt_face_free_hdmx(face: &mut TtFaceRec) {
    face.hdmx_records = Vec::new();
    face.hdmx_table = Vec::new();
}

/// `tt_face_get_device_metrics`: Return the advance width table for a
/// given pixel size if it is found in the font's `hdmx' table (if any), as
/// an offset into `hdmx_table`.  The records must be sorted for the binary
/// search to work properly.
pub fn tt_face_get_device_metrics(face: &TtFaceRec, ppem: FtUInt, gindex: FtUInt) -> Option<usize> {
    let mut min: FtUInt = 0;
    let mut max = face.hdmx_record_count;
    let mut result = None;

    while min < max {
        let mid = (min + max) >> 1;
        let rec = face.hdmx_records[mid as usize];
        let rppem = face.hdmx_table[rec] as FtUInt;

        if rppem > ppem {
            max = mid;
        } else if rppem < ppem {
            min = mid + 1;
        } else {
            result = Some(rec + 2 + gindex as usize);
            break;
        }
    }

    result
}
