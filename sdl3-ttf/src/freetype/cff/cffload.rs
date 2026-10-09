// Rust translation of src/cff/cffload.c and src/cff/cffload.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! OpenType and CFF data/program tables loader (body).
//!
//! The functions take the face's stream (C keeps it in the font and its
//! indices). An index element is returned as owned bytes (C points into a
//! loaded index or extracts a frame); the subroutine and string pointer
//! tables hold offsets into the index bytes and the string pool.

use std::sync::Arc;

use super::super::base::ftcalc::*;
use super::super::base::ftobjs::FtModuleRec;
use super::super::base::ftstream::{ft_peek_ulong, ft_peek_uoff3, ft_peek_ushort, FtStreamRec};
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::super::psaux::psobjs::cff_random;
use super::super::psaux::PsDriverRec;
use super::super::psnames::psmodule::ps_get_standard_strings;
use super::super::tttypes::TtFaceRec;
use super::cffparse::*;

const FT_FIXED_ONE: FtFixed = 0x10000;

static CFF_ISOADOBE_CHARSET: [FtUShort; 229] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
    74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97,
    98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
    117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135,
    136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154,
    155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173,
    174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191, 192,
    193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211,
    212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228,
];

static CFF_EXPERT_CHARSET: [FtUShort; 166] = [
    0, 1, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 13, 14, 15, 99, 239, 240, 241, 242,
    243, 244, 245, 246, 247, 248, 27, 28, 249, 250, 251, 252, 253, 254, 255, 256, 257, 258, 259,
    260, 261, 262, 263, 264, 265, 266, 109, 110, 267, 268, 269, 270, 271, 272, 273, 274, 275, 276,
    277, 278, 279, 280, 281, 282, 283, 284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295,
    296, 297, 298, 299, 300, 301, 302, 303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313, 314,
    315, 316, 317, 318, 158, 155, 163, 319, 320, 321, 322, 323, 324, 325, 326, 150, 164, 169, 327,
    328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343, 344, 345, 346,
    347, 348, 349, 350, 351, 352, 353, 354, 355, 356, 357, 358, 359, 360, 361, 362, 363, 364, 365,
    366, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378,
];

static CFF_EXPERTSUBSET_CHARSET: [FtUShort; 87] = [
    0, 1, 231, 232, 235, 236, 237, 238, 13, 14, 15, 99, 239, 240, 241, 242, 243, 244, 245, 246,
    247, 248, 27, 28, 249, 250, 251, 253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264,
    265, 266, 109, 110, 267, 268, 269, 270, 272, 300, 301, 302, 305, 314, 315, 158, 155, 163, 320,
    321, 322, 323, 324, 325, 326, 150, 164, 169, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336,
    337, 338, 339, 340, 341, 342, 343, 344, 345, 346,
];

static CFF_STANDARD_ENCODING: [FtUShort; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74,
    75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 96,
    97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 0, 111, 112, 113, 114, 0,
    115, 116, 117, 118, 119, 120, 121, 122, 0, 123, 0, 124, 125, 126, 127, 128, 129, 130, 131, 0,
    132, 133, 0, 134, 135, 136, 137, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 138, 0, 139,
    0, 0, 0, 0, 140, 141, 142, 143, 0, 0, 0, 0, 0, 144, 0, 0, 0, 145, 0, 0, 146, 147, 148, 149, 0,
    0, 0, 0,
];

static CFF_EXPERT_ENCODING: [FtUShort; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 229, 230, 0, 231, 232, 233, 234, 235, 236, 237, 238, 13, 14, 15, 99, 239, 240, 241, 242,
    243, 244, 245, 246, 247, 248, 27, 28, 249, 250, 251, 252, 0, 253, 254, 255, 256, 257, 0, 0, 0,
    258, 0, 0, 259, 260, 261, 262, 0, 0, 263, 264, 265, 0, 266, 109, 110, 267, 268, 269, 0, 270,
    271, 272, 273, 274, 275, 276, 277, 278, 279, 280, 281, 282, 283, 284, 285, 286, 287, 288, 289,
    290, 291, 292, 293, 294, 295, 296, 297, 298, 299, 300, 301, 302, 303, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 304, 305, 306, 0,
    0, 307, 308, 309, 310, 311, 0, 312, 0, 0, 312, 0, 0, 314, 315, 0, 0, 316, 317, 318, 0, 0, 0,
    158, 155, 163, 319, 320, 321, 322, 323, 324, 325, 0, 0, 326, 150, 164, 169, 327, 328, 329, 330,
    331, 332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343, 344, 345, 346, 347, 348, 349,
    350, 351, 352, 353, 354, 355, 356, 357, 358, 359, 360, 361, 362, 363, 364, 365, 366, 367, 368,
    369, 370, 371, 372, 373, 374, 375, 376, 377, 378,
];

/// `cff_get_standard_encoding`
pub fn cff_get_standard_encoding(charcode: FtUInt) -> FtUShort {
    if charcode < 256 {
        CFF_STANDARD_ENCODING[charcode as usize]
    } else {
        0
    }
}

/// A fallible vector of `count` default items (`FT_QNEW_ARRAY`).
fn qnew_array<T: Clone + Default>(count: usize) -> FtResult<Vec<T>> {
    let mut v = Vec::new();
    if v.try_reserve_exact(count).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    v.resize(count, T::default());
    Ok(v)
}

/* read an offset from the index's stream current position */
fn cff_index_read_offset(idx: &CffIndexRec, stream: &mut FtStreamRec) -> FtResult<FtULong> {
    let mut tmp = [0u8; 4];
    let mut result: FtULong = 0;

    stream.read(&mut tmp[..idx.off_size as usize])?;

    for &b in &tmp[..idx.off_size as usize] {
        result = (result << 8) | b as FtULong;
    }

    Ok(result)
}

fn cff_index_init(
    idx: &mut CffIndexRec,
    stream: &mut FtStreamRec,
    load: bool,
    cff2: bool,
) -> FtResult<()> {
    *idx = CffIndexRec::default();

    idx.has_stream = true;
    idx.start = stream.pos();

    let count: FtUInt = if cff2 {
        let c = stream.read_ulong()?;
        idx.hdr_size = 5;
        c
    } else {
        let c = stream.read_ushort()? as FtUInt;
        idx.hdr_size = 3;
        c
    };

    if count > 0 {
        /* there is at least one element; read the offset size,           */
        /* then access the offset table to compute the index's total size */
        let offsize = stream.read_byte()?;

        if !(1..=4).contains(&offsize) {
            return Err(FT_ERR_INVALID_TABLE);
        }

        idx.count = count;
        idx.off_size = offsize;
        let mut size: FtULong = count.wrapping_add(1) as FtULong * offsize as FtULong;

        idx.data_offset = idx
            .start
            .wrapping_add(idx.hdr_size as FtULong)
            .wrapping_add(size);

        stream.skip(size.wrapping_sub(offsize as FtULong) as FtLong)?;

        size = cff_index_read_offset(idx, stream)?;

        if size == 0 {
            return Err(FT_ERR_INVALID_TABLE);
        }

        size -= 1;
        idx.data_size = size;

        if load {
            /* load the data */
            idx.bytes = Some(Arc::from(stream.extract_frame(size)?));
        } else {
            /* skip the data */
            stream.skip(size as FtLong)?;
        }
    }

    Ok(())
}

fn cff_index_done(idx: &mut CffIndexRec) {
    if idx.has_stream {
        *idx = CffIndexRec::default();
    }
}

fn cff_index_load_offsets(idx: &mut CffIndexRec, stream: &mut FtStreamRec) -> FtResult<()> {
    if idx.count > 0 && idx.offsets.is_none() {
        let offsize = idx.off_size;
        let data_size: FtULong = (idx.count as FtULong + 1) * offsize as FtULong;

        let mut offsets: Vec<FtULong> = qnew_array(idx.count as usize + 1)?;
        stream.seek(idx.start + idx.hdr_size as FtULong)?;
        stream.enter_frame(data_size)?;

        {
            let p = &stream.frame_data()[stream.cursor()..];
            let p_end = data_size as usize;
            let mut i = 0;
            let mut poff = 0;

            match offsize {
                1 => {
                    while i < p_end {
                        offsets[poff] = p[i] as FtULong;
                        i += 1;
                        poff += 1;
                    }
                }

                2 => {
                    while i < p_end {
                        offsets[poff] = ft_peek_ushort(p, i) as FtULong;
                        i += 2;
                        poff += 1;
                    }
                }

                3 => {
                    while i < p_end {
                        offsets[poff] = ft_peek_uoff3(p, i) as FtULong;
                        i += 3;
                        poff += 1;
                    }
                }

                _ => {
                    while i < p_end {
                        offsets[poff] = ft_peek_ulong(p, i) as FtULong;
                        i += 4;
                        poff += 1;
                    }
                }
            }
        }

        stream.exit_frame();
        idx.offsets = Some(offsets);
    }

    Ok(())
}

/// `cff_index_get_pointers` without a pool: the offsets of the index's
/// elements in its bytes (`count + 1` entries), or `None` for an empty
/// index.
fn cff_index_get_pointers(
    idx: &mut CffIndexRec,
    stream: &mut FtStreamRec,
) -> FtResult<Option<Arc<[usize]>>> {
    Ok(cff_index_get_pointers_pool(idx, stream, false)?.map(|(tbl, _, _)| Arc::from(tbl)))
}

/* Allocate a table containing pointers to an index's elements. */
/* The `pool' argument makes this function convert the index    */
/* entries to C-style strings (that is, null-terminated).       */
#[allow(clippy::type_complexity)]
fn cff_index_get_pointers_pool(
    idx: &mut CffIndexRec,
    stream: &mut FtStreamRec,
    pool: bool,
) -> FtResult<Option<(Vec<usize>, Vec<u8>, FtULong)>> {
    if idx.offsets.is_none() {
        cff_index_load_offsets(idx, stream)?;
    }

    let new_size: FtULong = idx.data_size + idx.count as FtULong;

    if idx.count > 0 {
        let mut tbl: Vec<usize> = qnew_array(idx.count as usize + 1)?;
        let mut new_bytes: Vec<u8> = Vec::new();
        if pool {
            new_bytes = super::super::base::ftmemory::ft_alloc(new_size as FtLong)?;
        }

        let offsets = idx.offsets.as_ref().unwrap();
        let empty: Arc<[u8]> = Arc::from(&[][..]);
        let org_bytes = idx.bytes.as_ref().unwrap_or(&empty);
        let mut extra: FtULong = 0;

        /* at this point, `idx->offsets' can't be NULL */
        let mut cur_offset: FtULong = offsets[0].wrapping_sub(1);

        /* sanity check */
        if cur_offset != 0 {
            cur_offset = 0;
        }

        tbl[0] = cur_offset as usize;

        for n in 1..=idx.count as usize {
            let mut next_offset: FtULong = offsets[n].wrapping_sub(1);

            /* two sanity checks for invalid offset tables */
            if next_offset < cur_offset {
                next_offset = cur_offset;
            } else if next_offset > idx.data_size {
                next_offset = idx.data_size;
            }

            if !pool {
                tbl[n] = next_offset as usize;
            } else {
                tbl[n] = (next_offset + extra) as usize;

                if next_offset != cur_offset {
                    let len = tbl[n] - tbl[n - 1];
                    let src = cur_offset as usize;
                    new_bytes[tbl[n - 1]..tbl[n - 1] + len]
                        .copy_from_slice(&org_bytes[src..src + len]);
                    new_bytes[tbl[n]] = 0;
                    tbl[n] += 1;
                    extra += 1;
                }
            }

            cur_offset = next_offset;
        }

        return Ok(Some((tbl, new_bytes, new_size)));
    }

    Ok(None)
}

/// `cff_index_access_element`: the bytes of element `element` (empty for
/// an empty element).
pub fn cff_index_access_element(
    idx: &CffIndexRec,
    stream: &mut FtStreamRec,
    mut element: FtUInt,
) -> FtResult<Vec<u8>> {
    if idx.count > element {
        /* compute start and end offsets */
        let off1: FtULong;
        let mut off2: FtULong = 0;

        /* load offsets from file or the offset table */
        match &idx.offsets {
            None => {
                let pos: FtULong = element as FtULong * idx.off_size as FtULong;

                stream.seek(idx.start + idx.hdr_size as FtULong + pos)?;

                off1 = cff_index_read_offset(idx, stream)?;

                if off1 != 0 {
                    loop {
                        element += 1;
                        off2 = cff_index_read_offset(idx, stream).unwrap_or(0);

                        if !(off2 == 0 && element < idx.count) {
                            break;
                        }
                    }
                }
            }
            Some(offsets) => {
                /* use offsets table */
                off1 = offsets[element as usize];
                if off1 != 0 {
                    loop {
                        element += 1;
                        off2 = offsets[element as usize];

                        if !(off2 == 0 && element < idx.count) {
                            break;
                        }
                    }
                }
            }
        }

        /* XXX: should check off2 does not exceed the end of this entry; */
        /*      at present, only truncate off2 at the end of this stream */
        if off2 > stream.size.wrapping_add(1)
            || idx.data_offset > stream.size.wrapping_sub(off2).wrapping_add(1)
        {
            off2 = stream.size.wrapping_sub(idx.data_offset).wrapping_add(1);
        }

        /* access element */
        if off1 != 0 && off2 > off1 {
            let byte_len = off2 - off1;

            if let Some(bytes) = idx
                .bytes
                .as_ref()
                .filter(|b| (off1 - 1 + byte_len) as usize <= b.len())
            {
                /* this index was completely loaded in memory, that's easy */
                let s = (off1 - 1) as usize;
                return Ok(bytes[s..s + byte_len as usize].to_vec());
            } else {
                /* this index is still on disk/file, access it through a frame */
                /* (a loaded index whose element would run past its bytes is   */
                /* read from the file too, as C's pointer into a memory-based  */
                /* stream reads on into the file)                              */
                stream.seek(idx.data_offset + off1 - 1)?;
                return stream.extract_frame(off2 - off1);
            }
        } else {
            /* empty index element */
            return Ok(Vec::new());
        }
    }

    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `cff_index_forget_element`: the element's bytes go when they are
/// dropped.
pub fn cff_index_forget_element(_idx: &CffIndexRec, _bytes: Vec<u8>) {}

/// The C string at the start of `bytes` (up to its first NUL).
fn c_str(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// `cff_index_get_name`: get an entry from Name INDEX (as a C string, up
/// to its first NUL)
pub fn cff_index_get_name(
    font: &CffFontRec,
    stream: &mut FtStreamRec,
    element: FtUInt,
) -> Option<Vec<u8>> {
    let idx = &font.name_index;

    if !idx.has_stream {
        /* CFF2 does not include a name index */
        return None;
    }

    let bytes = cff_index_access_element(idx, stream, element).ok()?;

    let mut name = Vec::new();
    if name.try_reserve_exact(bytes.len() + 1).is_err() {
        return None;
    }
    name.extend_from_slice(c_str(&bytes));
    cff_index_forget_element(idx, bytes);

    Some(name)
}

/// `cff_index_get_string`: get an entry from String INDEX
pub fn cff_index_get_string(font: &CffFontRec, element: FtUInt) -> Option<&[u8]> {
    if element < font.num_strings {
        font.strings
            .as_ref()
            .and_then(|s| s.get(element as usize))
            .map(|&off| c_str(&font.string_pool[off.min(font.string_pool.len())..]))
    } else {
        None
    }
}

/// `cff_index_get_sid_string`
pub fn cff_index_get_sid_string(font: &CffFontRec, sid: FtUInt) -> Option<&[u8]> {
    /* value 0xFFFFU indicates a missing dictionary entry */
    if sid == 0xFFFF {
        return None;
    }

    /* if it is not a standard string, return it */
    if sid > 390 {
        return cff_index_get_string(font, sid - 391);
    }

    /* CID-keyed CFF fonts don't have glyph names */
    if !font.psnames {
        return None;
    }

    /* this is a standard string */
    ps_get_standard_strings(sid)
}

/*************************************************************************/
/*************************************************************************/
/***                                                                   ***/
/***   FD Select table support                                         ***/
/***                                                                   ***/
/*************************************************************************/
/*************************************************************************/

fn cff_done_fd_select(fdselect: &mut CffFdSelectRec) {
    fdselect.data = None;

    fdselect.data_size = 0;
    fdselect.format = 0;
    fdselect.range_count = 0;
}

fn cff_load_fd_select(
    fdselect: &mut CffFdSelectRec,
    num_glyphs: FtUInt,
    stream: &mut FtStreamRec,
    offset: FtULong,
) -> FtResult<()> {
    /* read format */
    stream.seek(offset)?;
    let format = stream.read_byte()?;

    fdselect.format = format;
    fdselect.cache_count = 0; /* clear cache */

    match format {
        0 => {
            /* format 0, that's simple */
            fdselect.data_size = num_glyphs;
        }

        3 => {
            /* format 3, a tad more complex */
            let num_ranges = stream.read_ushort()? as FtUInt;

            if num_ranges == 0 {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }

            fdselect.data_size = num_ranges * 3 + 2;
        }

        _ => {
            /* hmm... that's wrong */
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
    }

    /* Load_Data: */
    fdselect.data = Some(stream.extract_frame(fdselect.data_size as FtULong)?);

    Ok(())
}

/// `cff_fd_select_get`
pub fn cff_fd_select_get(fdselect: &mut CffFdSelectRec, glyph_index: FtUInt) -> FtByte {
    let mut fd: FtByte = 0;

    /* if there is no FDSelect, return zero               */
    /* Note: CFF2 with just one Font Dict has no FDSelect */
    let Some(data) = fdselect.data.as_ref() else {
        return fd;
    };
    let at = |i: usize| data.get(i).copied().unwrap_or(0);

    match fdselect.format {
        0 => {
            fd = at(glyph_index as usize);
        }

        3 => {
            /* first, compare to the cache */
            if glyph_index.wrapping_sub(fdselect.cache_first) < fdselect.cache_count {
                return fdselect.cache_fd;
            }

            /* then, look up the ranges array */
            {
                let mut p = 0usize;
                let p_limit = fdselect.data_size as usize;
                let mut fd2: FtByte;
                let mut first: FtUInt;
                let mut limit: FtUInt;

                first = ft_peek_ushort(data, p) as FtUInt;
                p += 2;
                loop {
                    if glyph_index < first {
                        break;
                    }

                    fd2 = at(p);
                    p += 1;
                    limit = ft_peek_ushort(data, p) as FtUInt;
                    p += 2;

                    if glyph_index < limit {
                        fd = fd2;

                        /* update cache */
                        fdselect.cache_first = first;
                        fdselect.cache_count = limit.wrapping_sub(first);
                        fdselect.cache_fd = fd2;
                        break;
                    }
                    first = limit;

                    if p >= p_limit {
                        break;
                    }
                }
            }
        }

        _ => {}
    }

    fd
}

/*************************************************************************/
/*************************************************************************/
/***                                                                   ***/
/***   CFF font support                                                ***/
/***                                                                   ***/
/*************************************************************************/
/*************************************************************************/

fn cff_charset_compute_cids(charset: &mut CffCharsetRec, num_glyphs: FtUInt) -> FtResult<()> {
    let mut max_cid: FtUShort = 0;

    if charset.max_cid > 0 {
        return Ok(());
    }

    let sids = charset.sids.as_ref().map(|s| &s[..]).unwrap_or(&[]);
    for i in 0..num_glyphs as usize {
        if sids[i] > max_cid {
            max_cid = sids[i];
        }
    }

    let mut cids: Vec<FtUShort> =
        super::super::base::ftmemory::ft_new_array(max_cid as FtLong + 1)?;

    /* When multiple GIDs map to the same CID, we choose the lowest */
    /* GID.  This is not described in any spec, but it matches the  */
    /* behaviour of recent Acroread versions.  The loop stops when  */
    /* the unsigned index wraps around after reaching zero.         */
    for i in (0..num_glyphs as usize).rev() {
        cids[sids[i] as usize] = i as FtUShort;
    }

    charset.cids = Some(cids);
    charset.max_cid = max_cid as FtUInt;
    charset.num_glyphs = num_glyphs;

    Ok(())
}

/// `cff_charset_cid_to_gindex`
pub fn cff_charset_cid_to_gindex(charset: &CffCharsetRec, cid: FtUInt) -> FtUInt {
    let mut result: FtUInt = 0;

    if cid <= charset.max_cid {
        if let Some(cids) = &charset.cids {
            result = cids[cid as usize] as FtUInt;
        }
    }

    result
}

fn cff_charset_free_cids(charset: &mut CffCharsetRec) {
    charset.cids = None;
    charset.max_cid = 0;
}

fn cff_charset_done(charset: &mut CffCharsetRec) {
    cff_charset_free_cids(charset);

    charset.sids = None;
    charset.format = 0;
    charset.offset = 0;
}

fn cff_charset_load(
    charset: &mut CffCharsetRec,
    num_glyphs: FtUInt,
    stream: &mut FtStreamRec,
    base_offset: FtULong,
    offset: FtULong,
    invert: bool,
) -> FtResult<()> {
    let r = cff_charset_load_inner(charset, num_glyphs, stream, base_offset, offset, invert);

    /* Clean up if there was an error. */
    if r.is_err() {
        charset.sids = None;
        charset.cids = None;
        charset.format = 0;
        charset.offset = 0;
    }

    r
}

fn cff_charset_load_inner(
    charset: &mut CffCharsetRec,
    num_glyphs: FtUInt,
    stream: &mut FtStreamRec,
    base_offset: FtULong,
    offset: FtULong,
    invert: bool,
) -> FtResult<()> {
    /* If the offset is greater than 2, we have to parse the charset */
    /* table.                                                        */
    if offset > 2 {
        charset.offset = base_offset.wrapping_add(offset);

        /* Get the format of the table. */
        stream.seek(charset.offset)?;
        charset.format = stream.read_byte()? as FtUInt;

        /* Allocate memory for sids. */
        charset.sids = Some(qnew_array(num_glyphs as usize)?);
        let sids = charset.sids.as_mut().unwrap();

        /* assign the .notdef glyph */
        sids[0] = 0;

        match charset.format {
            0 => {
                if num_glyphs > 0 {
                    stream.enter_frame((num_glyphs as FtULong - 1) * 2)?;

                    for j in 1..num_glyphs as usize {
                        sids[j] = stream.get_ushort();
                    }

                    stream.exit_frame();
                }
            }

            1 | 2 => {
                let mut nleft: FtUInt;
                let mut j: FtUInt = 1;

                while j < num_glyphs {
                    /* Read the first glyph sid of the range. */
                    let mut glyph_sid: FtUShort = stream.read_ushort()?;

                    /* Read the number of glyphs in the range.  */
                    if charset.format == 2 {
                        nleft = stream.read_ushort()? as FtUInt;
                    } else {
                        nleft = stream.read_byte()? as FtUInt;
                    }

                    /* try to rescue some of the SIDs if `nleft' is too large */
                    if glyph_sid as FtLong > 0xFFFF - nleft as FtLong {
                        nleft = (0xFFFF - glyph_sid as FtLong) as FtUInt;
                    }

                    /* Fill in the range of sids -- `nleft + 1' glyphs. */
                    let mut i: FtUInt = 0;
                    while j < num_glyphs && i <= nleft {
                        sids[j as usize] = glyph_sid;
                        i += 1;
                        j += 1;
                        glyph_sid = glyph_sid.wrapping_add(1);
                    }
                }
            }

            _ => {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }
        }
    } else {
        /* Parse default tables corresponding to offset == 0, 1, or 2.  */
        /* CFF specification intimates the following:                   */
        /*                                                              */
        /* In order to use a predefined charset, the following must be  */
        /* true: The charset constructed for the glyphs in the font's   */
        /* charstrings dictionary must match the predefined charset in  */
        /* the first num_glyphs.                                        */

        charset.offset = offset; /* record charset type */

        let table: &[FtUShort] = match offset as FtUInt {
            0 => {
                if num_glyphs > 229 {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }
                &CFF_ISOADOBE_CHARSET
            }

            1 => {
                if num_glyphs > 166 {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }
                &CFF_EXPERT_CHARSET
            }

            2 => {
                if num_glyphs > 87 {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }
                &CFF_EXPERTSUBSET_CHARSET
            }

            _ => {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }
        };

        /* Allocate memory for sids. */
        let mut sids: Vec<FtUShort> = qnew_array(num_glyphs as usize)?;

        /* Copy the predefined charset into the allocated memory. */
        sids.copy_from_slice(&table[..num_glyphs as usize]);
        charset.sids = Some(sids);
    }

    /* we have to invert the `sids' array for subsetted CID-keyed fonts */
    if invert {
        cff_charset_compute_cids(charset, num_glyphs)?;
    }

    Ok(())
}

fn cff_vstore_done(vstore: &mut CffVStoreRec) {
    /* free regionList and axisLists */
    vstore.varRegionList = Vec::new();

    /* free varData and indices */
    vstore.varData = Vec::new();
}

/* convert 2.14 to Fixed */
fn ft_fdot14_to_fixed(x: FtInt16) -> FtFixed {
    ((x as FtLong as FtULong) << 2) as FtFixed
}

type FtInt16 = i16;

fn cff_vstore_load(
    vstore: &mut CffVStoreRec,
    stream: &mut FtStreamRec,
    base_offset: FtULong,
    offset: FtULong,
) -> FtResult<()> {
    let r = cff_vstore_load_inner(vstore, stream, base_offset, offset);
    if r.is_err() {
        cff_vstore_done(vstore);
    }
    r
}

fn cff_vstore_load_inner(
    vstore: &mut CffVStoreRec,
    stream: &mut FtStreamRec,
    base_offset: FtULong,
    offset: FtULong,
) -> FtResult<()> {
    /* no offset means no vstore to parse */
    if offset != 0 {
        /* we need to parse the table to determine its size; */
        /* skip table length                                 */
        stream.seek(base_offset.wrapping_add(offset))?;
        stream.skip(2)?;

        /* actual variation store begins after the length */
        let vs_offset: FtUInt = stream.pos() as FtUInt;

        /* check the header */
        let format: FtUInt = stream.read_ushort()? as FtUInt;
        if format != 1 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        /* read top level fields */
        let region_list_offset: FtULong = stream.read_ulong()? as FtULong;
        let data_count: FtUInt = stream.read_ushort()? as FtUInt;

        /* make temporary copy of item variation data offsets; */
        /* we'll parse region list first, then come back       */
        let mut data_offset_array: Vec<FtULong> = qnew_array(data_count as usize)?;

        for d in data_offset_array.iter_mut() {
            *d = stream.read_ulong()? as FtULong;
        }

        /* parse regionList and axisLists */
        stream.seek((vs_offset as FtULong).wrapping_add(region_list_offset))?;
        vstore.axisCount = stream.read_ushort()?;
        let region_count: FtUInt = stream.read_ushort()? as FtUInt;

        vstore.regionCount = 0;
        vstore.varRegionList = Vec::new();
        if vstore
            .varRegionList
            .try_reserve_exact(region_count as usize)
            .is_err()
        {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }

        for _ in 0..region_count {
            let mut axis_list: Vec<CffAxisCoords> = qnew_array(vstore.axisCount as usize)?;

            for axis in axis_list.iter_mut() {
                let start14 = stream.read_short();
                let start14 = match start14 {
                    Ok(v) => v,
                    Err(e) => {
                        /* keep track of how many axisList to deallocate on error */
                        vstore.varRegionList.push(CffVarRegion {
                            axisList: axis_list,
                        });
                        vstore.regionCount += 1;
                        return Err(e);
                    }
                };
                let peak14 = stream.read_short()?;
                let end14 = stream.read_short()?;

                axis.startCoord = ft_fdot14_to_fixed(start14);
                axis.peakCoord = ft_fdot14_to_fixed(peak14);
                axis.endCoord = ft_fdot14_to_fixed(end14);
            }

            /* keep track of how many axisList to deallocate on error */
            vstore.varRegionList.push(CffVarRegion {
                axisList: axis_list,
            });
            vstore.regionCount += 1;
        }

        /* use dataOffsetArray now to parse varData items */
        vstore.dataCount = 0;
        vstore.varData = Vec::new();
        if vstore
            .varData
            .try_reserve_exact(data_count as usize)
            .is_err()
        {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }

        for &data_offset in &data_offset_array {
            stream.seek((vs_offset as FtULong).wrapping_add(data_offset))?;

            /* ignore `itemCount' and `shortDeltaCount' */
            /* because CFF2 has no delta sets           */
            stream.skip(4)?;

            /* Note: just record values; consistency is checked later    */
            /*       by cff_blend_build_vector when it consumes `vstore' */

            let region_idx_count: FtUInt = stream.read_ushort()? as FtUInt;

            let mut region_indices: Vec<FtUInt> = qnew_array(region_idx_count as usize)?;

            /* keep track of how many regionIndices to deallocate on error */
            vstore.dataCount += 1;

            let mut res = Ok(());
            for r in region_indices.iter_mut() {
                match stream.read_ushort() {
                    Ok(v) => *r = v as FtUInt,
                    Err(e) => {
                        res = Err(e);
                        break;
                    }
                }
            }

            vstore.varData.push(CffVarData {
                regionIdxCount: region_idx_count,
                regionIndices: region_indices,
            });
            res?;
        }
    }

    Ok(())
}

/// `cff_blend_clear`: clear blend stack (after blend values are
/// consumed).
///
/// TODO: Should do this in cff_run_parse, but subFont
///       ref is not available there.
///
/// Allocation is not changed when stack is cleared.
pub fn cff_blend_clear(sub_font: &mut CffSubFontRec) {
    sub_font.blend_top = 0;
    sub_font.blend_used = 0;
}

/// `cff_blend_doBlend`: blend numOperands on the stack, store results into
/// the first numBlends values, then pop remaining arguments.
///
/// This is comparable to `cf2_doBlend' but the cffparse stack is different
/// and can't be written.  Blended values are written to a different
/// buffer, using reserved operator 255.
///
/// Blend calculation is done in 16.16 fixed-point.
pub fn cff_blend_do_blend(parser: &mut CffParserRec<'_, '_>, num_blends: FtUInt) -> FtResult<()> {
    let len_bv = match &parser.object {
        CffParserObject::Private { subfont, .. } => subfont.blend.lenBV,
        CffParserObject::FontDict(_) => return Err(FT_ERR_INVALID_FILE_FORMAT),
    };

    /* compute expected number of operands for this blend */
    let num_operands: FtUInt = num_blends.wrapping_mul(len_bv);
    let count: FtUInt = (parser.top as FtUInt).wrapping_sub(1);

    if num_operands > count {
        return Err(FT_ERR_STACK_UNDERFLOW);
    }

    /* check whether we have room for `numBlends' values at `blend_top' */
    let size: FtUInt = 5 * num_blends; /* add 5 bytes per entry    */
    {
        let CffParserObject::Private { subfont, .. } = &mut parser.object else {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        };

        if subfont.blend_used + size > subfont.blend_alloc {
            /* increase or allocate `blend_stack' and reset `blend_top'; */
            /* prepare to append `numBlends' values to the buffer        */
            let new_alloc = (subfont.blend_alloc + size) as usize;
            if new_alloc > subfont.blend_stack.len()
                && subfont
                    .blend_stack
                    .try_reserve_exact(new_alloc - subfont.blend_stack.len())
                    .is_err()
            {
                return Err(FT_ERR_OUT_OF_MEMORY);
            }
            subfont.blend_stack.resize(new_alloc, 0);

            subfont.blend_top = subfont.blend_used as usize;
            subfont.blend_alloc += size;

            /* (the parser stack holds positions, which stay valid) */
        }
        subfont.blend_used += size;
    }

    let base: FtUInt = count - num_operands; /* index of first blend arg */
    let mut delta: FtUInt = base + num_blends; /* index of first delta arg */

    for i in 0..num_blends {
        /* convert inputs to 16.16 fixed point */
        let mut sum: FtFixed = cff_parse_fixed(parser, parser.stack[(i + base) as usize]);

        let bv: Vec<FtInt32> = match &parser.object {
            CffParserObject::Private { subfont, .. } => subfont.blend.BV.clone(),
            CffParserObject::FontDict(_) => Vec::new(),
        };
        for j in 1..len_bv as usize {
            let weight = bv[j];
            sum = sum.wrapping_add(ft_mul_fix(
                cff_parse_fixed(parser, parser.stack[delta as usize]),
                weight as FtFixed,
            ));
            delta += 1;
        }

        let CffParserObject::Private { subfont, .. } = &mut parser.object else {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        };

        /* point parser stack to new value on blend_stack */
        let top = subfont.blend_top;
        parser.stack[(i + base) as usize] = CffOperand::Blend(top);

        /* Push blended result as Type 2 5-byte fixed-point number.  This */
        /* will not conflict with actual DICTs because 255 is a reserved  */
        /* opcode in both CFF and CFF2 DICTs.  See `cff_parse_num' for    */
        /* decode of this, which rounds to an integer.                    */
        subfont.blend_stack[top] = 255;
        subfont.blend_stack[top + 1] = (sum >> 24) as FtByte;
        subfont.blend_stack[top + 2] = (sum >> 16) as FtByte;
        subfont.blend_stack[top + 3] = (sum >> 8) as FtByte;
        subfont.blend_stack[top + 4] = sum as FtByte;
        subfont.blend_top = top + 5;
    }

    /* leave only numBlends results on parser stack */
    parser.top = (base + num_blends) as usize;

    Ok(())
}

/// `cff_blend_build_vector`: compute a blend vector from variation store
/// index and normalized vector based on pseudo-code in OpenType Font
/// Variations Overview.
///
/// Note: lenNDV == 0 produces a default blend vector, (1,0,0,...).
pub fn cff_blend_build_vector(
    blend: &mut CffBlendRec,
    vs: &CffVStoreRec,
    vsindex: FtUInt,
    len_ndv: FtUInt,
    ndv: Option<&[FtFixed]>,
) -> FtResult<()> {
    /* protect against malformed fonts */
    if !(len_ndv == 0 || ndv.is_some()) {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }
    let ndv = ndv.unwrap_or(&[]);

    blend.builtBV = false;

    /* VStore and fvar must be consistent */
    if len_ndv != 0 && len_ndv != vs.axisCount as FtUInt {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    if vsindex >= vs.dataCount {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* select the item variation data structure */
    let var_data = &vs.varData[vsindex as usize];

    /* prepare buffer for the blend vector */
    let len: FtUInt = var_data.regionIdxCount + 1; /* add 1 for default component */
    super::super::base::ftmemory::ft_renew_array(&mut blend.BV, len as FtLong)?;

    blend.lenBV = len;

    /* outer loop steps through master designs to be blended */
    for master in 0..len as usize {
        /* default factor is always one */
        if master == 0 {
            blend.BV[master] = FT_FIXED_ONE as FtInt32;
            continue;
        }

        /* VStore array does not include default master, so subtract one */
        let idx: FtUInt = var_data.regionIndices[master - 1];

        if idx >= vs.regionCount {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
        let var_region = &vs.varRegionList[idx as usize];

        /* Note: `lenNDV' could be zero.                              */
        /*       In that case, build default blend vector (1,0,0...). */
        if len_ndv == 0 {
            blend.BV[master] = 0;
            continue;
        }

        /* In the normal case, initialize each component to 1 */
        /* before inner loop.                                 */
        blend.BV[master] = FT_FIXED_ONE as FtInt32; /* default */

        /* inner loop steps through axes in this region */
        for j in 0..len_ndv as usize {
            let axis = &var_region.axisList[j];
            let axis_scalar: FtFixed;

            /* compute the scalar contribution of this axis; */
            /* ignore invalid ranges                         */
            if axis.startCoord > axis.peakCoord || axis.peakCoord > axis.endCoord {
                axis_scalar = FT_FIXED_ONE;
            } else if axis.startCoord < 0 && axis.endCoord > 0 && axis.peakCoord != 0 {
                axis_scalar = FT_FIXED_ONE;
            }
            /* peak of 0 means ignore this axis */
            else if axis.peakCoord == 0 {
                axis_scalar = FT_FIXED_ONE;
            }
            /* ignore this region if coords are out of range */
            else if ndv[j] < axis.startCoord || ndv[j] > axis.endCoord {
                axis_scalar = 0;
            }
            /* calculate a proportional factor */
            else if ndv[j] == axis.peakCoord {
                axis_scalar = FT_FIXED_ONE;
            } else if ndv[j] < axis.peakCoord {
                axis_scalar =
                    ft_div_fix(ndv[j] - axis.startCoord, axis.peakCoord - axis.startCoord);
            } else {
                axis_scalar = ft_div_fix(axis.endCoord - ndv[j], axis.endCoord - axis.peakCoord);
            }

            /* take product of all the axis scalars */
            blend.BV[master] = ft_mul_fix(blend.BV[master] as FtLong, axis_scalar) as FtInt32;
        }
    }

    /* record the parameters used to build the blend vector */
    blend.lastVsindex = vsindex;

    if len_ndv != 0 {
        /* user has set a normalized vector */
        super::super::base::ftmemory::ft_renew_array(&mut blend.lastNDV, len_ndv as FtLong)?;

        blend.lastNDV[..len_ndv as usize].copy_from_slice(&ndv[..len_ndv as usize]);
    }

    blend.lenNDV = len_ndv;
    blend.builtBV = true;

    Ok(())
}

/// `cff_blend_check_vector`: `lenNDV' is zero for default vector; return
/// TRUE if blend vector needs to be built.
pub fn cff_blend_check_vector(
    blend: &CffBlendRec,
    vsindex: FtUInt,
    len_ndv: FtUInt,
    ndv: Option<&[FtFixed]>,
) -> bool {
    if !blend.builtBV
        || blend.lastVsindex != vsindex
        || blend.lenNDV != len_ndv
        || (len_ndv != 0
            && ndv.unwrap_or(&[])[..len_ndv as usize] != blend.lastNDV[..len_ndv as usize])
    {
        /* need to build blend vector */
        return true;
    }

    false
}

/* TT_CONFIG_OPTION_GX_VAR_SUPPORT */

/// `cff_get_var_blend` (the `mm` service is the TrueType driver's): the
/// number of axes and the normalized coordinates.
pub fn cff_get_var_blend(face: &TtFaceRec) -> (FtUInt, Option<&[FtFixed]>) {
    let (num_coords, _coords, normalizedcoords, _mm_var) =
        super::super::truetype::ttgxvar::tt_get_var_blend(face);
    (num_coords, normalizedcoords)
}

/// `cff_done_blend`
pub fn cff_done_blend(face: &mut TtFaceRec) {
    super::super::truetype::ttgxvar::tt_done_blend(face);
}

fn cff_encoding_done(encoding: &mut CffEncodingRec) {
    encoding.format = 0;
    encoding.offset = 0;
    encoding.count = 0;
}

fn cff_encoding_load(
    encoding: &mut CffEncodingRec,
    charset: &mut CffCharsetRec,
    num_glyphs: FtUInt,
    stream: &mut FtStreamRec,
    base_offset: FtULong,
    offset: FtULong,
) -> FtResult<()> {
    let mut count: FtUInt;
    let mut glyph_code: FtUInt;

    /* Check for charset->sids.  If we do not have this, we fail. */
    if charset.sids.is_none() {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* Note: The encoding table in a CFF font is indexed by glyph index;  */
    /* the first encoded glyph index is 1.  Hence, we read the character  */
    /* code (`glyph_code') at index j and make the assignment:            */
    /*                                                                    */
    /*    encoding->codes[glyph_code] = j + 1                             */
    /*                                                                    */
    /* We also make the assignment:                                       */
    /*                                                                    */
    /*    encoding->sids[glyph_code] = charset->sids[j + 1]               */
    /*                                                                    */
    /* This gives us both a code to GID and a code to SID mapping.        */

    if offset > 1 {
        let sids = charset.sids.as_ref().unwrap();

        /* Zero out the code to gid/sid mappings. */
        encoding.sids = [0; 256];
        encoding.codes = [0; 256];

        encoding.offset = base_offset.wrapping_add(offset);

        /* we need to parse the table to determine its size */
        stream.seek(encoding.offset)?;
        encoding.format = stream.read_byte()? as FtUInt;
        count = stream.read_byte()? as FtUInt;

        match encoding.format & 0x7F {
            0 => {
                /* By convention, GID 0 is always ".notdef" and is never */
                /* coded in the font.  Hence, the number of codes found  */
                /* in the table is `count+1'.                            */
                /*                                                       */
                encoding.count = count + 1;

                stream.enter_frame(count as FtULong)?;

                {
                    let c = stream.cursor();
                    let p = &stream.frame_data()[c..];

                    for j in 1..=count {
                        glyph_code = p[j as usize - 1] as FtUInt;

                        /* Make sure j is not too big. */
                        if j < num_glyphs {
                            /* Assign code to GID mapping. */
                            encoding.codes[glyph_code as usize] = j as FtUShort;

                            /* Assign code to SID mapping. */
                            encoding.sids[glyph_code as usize] = sids[j as usize];
                        }
                    }
                }

                stream.exit_frame();
            }

            1 => {
                let mut nleft: FtUInt;
                let mut i: FtUInt = 1;

                encoding.count = 0;

                /* Parse the Format1 ranges. */
                let mut j: FtUInt = 0;
                while j < count {
                    /* Read the first glyph code of the range. */
                    glyph_code = stream.read_byte()? as FtUInt;

                    /* Read the number of codes in the range. */
                    nleft = stream.read_byte()? as FtUInt;

                    /* Increment nleft, so we read `nleft + 1' codes/sids. */
                    nleft += 1;

                    /* compute max number of character codes */
                    if nleft > encoding.count {
                        encoding.count = nleft;
                    }

                    /* Fill in the range of codes/sids. */
                    let mut k = i;
                    while k < nleft + i {
                        /* Make sure k is not too big. */
                        if k < num_glyphs && glyph_code < 256 {
                            /* Assign code to GID mapping. */
                            encoding.codes[glyph_code as usize] = k as FtUShort;

                            /* Assign code to SID mapping. */
                            encoding.sids[glyph_code as usize] = sids[k as usize];
                        }
                        k += 1;
                        glyph_code += 1;
                    }

                    j += 1;
                    i += nleft;
                }

                /* simple check; one never knows what can be found in a font */
                if encoding.count > 256 {
                    encoding.count = 256;
                }
            }

            _ => {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }
        }

        /* Parse supplemental encodings, if any. */
        if encoding.format & 0x80 != 0 {
            /* count supplements */
            count = stream.read_byte()? as FtUInt;

            for _ in 0..count {
                /* Read supplemental glyph code. */
                glyph_code = stream.read_byte()? as FtUInt;

                /* Read the SID associated with this glyph code. */
                let glyph_sid: FtUShort = stream.read_ushort()?;

                /* Assign code to SID mapping. */
                encoding.sids[glyph_code as usize] = glyph_sid;

                /* First, look up GID which has been assigned to */
                /* SID glyph_sid.                                */
                for gindex in 0..num_glyphs {
                    if sids[gindex as usize] == glyph_sid {
                        encoding.codes[glyph_code as usize] = gindex as FtUShort;
                        break;
                    }
                }
            }
        }
    } else {
        /* We take into account the fact a CFF font can use a predefined */
        /* encoding without containing all of the glyphs encoded by this */
        /* encoding (see the note at the end of section 12 in the CFF    */
        /* specification).                                               */

        match offset as FtUInt {
            0 => {
                /* First, copy the code to SID mapping. */
                encoding.sids = CFF_STANDARD_ENCODING;
            }

            1 => {
                /* First, copy the code to SID mapping. */
                encoding.sids = CFF_EXPERT_ENCODING;
            }

            _ => {
                return Err(FT_ERR_INVALID_FILE_FORMAT);
            }
        }

        /* Populate: */
        /* Construct code to GID mapping from code to SID mapping */
        /* and charset.                                           */

        encoding.offset = offset; /* used in cff_face_init */
        encoding.count = 0;

        cff_charset_compute_cids(charset, num_glyphs)?;

        for j in 0..256 {
            let sid: FtUInt = encoding.sids[j] as FtUInt;
            let mut gid: FtUInt = 0;

            if sid != 0 {
                gid = cff_charset_cid_to_gindex(charset, sid);
            }

            if gid != 0 {
                encoding.codes[j] = gid as FtUShort;
                encoding.count = j as FtUInt + 1;
            } else {
                encoding.codes[j] = 0;
                encoding.sids[j] = 0;
            }
        }
    }

    /* Exit: */

    /* Clean up if there was an error. */
    Ok(())
}

/// `cff_load_private_dict`: parse private dictionary; first call is always
/// from `cff_face_init', so NDV has not been set for CFF2 variation.
///
/// `cff_slot_load' must call this function each time NDV changes.
pub fn cff_load_private_dict(
    font: &mut CffFontRec,
    sub: CffSubFontId,
    stream: &mut FtStreamRec,
    len_ndv: FtUInt,
    ndv: Option<&[FtFixed]>,
) -> FtResult<()> {
    let cff2 = font.cff2;
    let top_maxstack = font.top_font.font_dict.maxstack;
    let base_offset = font.base_offset;
    let (subfont, vstore): (&mut CffSubFontRec, &CffVStoreRec) = match sub {
        CffSubFontId::Top => (&mut font.top_font, &font.vstore),
        CffSubFontId::Sub(i) => (&mut font.subfonts[i], &font.vstore),
    };

    /* store handle needed to access memory, vstore for blend;    */
    /* we need this for clean-up even if there is no private DICT */
    subfont.blend.font = true;
    subfont.blend.usedBV = false; /* clear state */

    let top = subfont.font_dict;
    if top.private_offset == 0 || top.private_size == 0 {
        /* Exit2: */
        return Ok(()); /* no private DICT, do nothing */
    }

    /* set defaults */
    subfont.private_dict = CffPrivateRec::default();
    {
        let priv_ = &mut subfont.private_dict;

        priv_.blue_shift = 7;
        priv_.blue_fuzz = 1;
        priv_.lenIV = -1;
        priv_.expansion_factor = (0.06 * 0x10000 as f64) as FtFixed;
        priv_.blue_scale = (0.039625 * 0x10000 as f64 * 1000.0) as FtFixed;

        /* provide inputs for blend calculations */
        priv_.subfont = true;
    }
    subfont.lenNDV = len_ndv;
    subfont.NDV = ndv.map(|v| v.to_vec());

    /* add 1 for the operator */
    let stack_size = if cff2 {
        top_maxstack + 1
    } else {
        CFF_MAX_STACK_DEPTH + 1
    };

    let error = (|| -> FtResult<()> {
        let Ok(mut parser) = cff_parser_init(
            if cff2 {
                CFF2_CODE_PRIVATE
            } else {
                CFF_CODE_PRIVATE
            },
            CffParserObject::Private {
                subfont: &mut *subfont,
                vstore,
            },
            stack_size,
            top.num_designs,
            top.num_axes,
        ) else {
            /* FIXME (upstream): a failed parser allocation jumps to `Exit' */
            /* without setting `error', so it is not reported              */
            return Ok(());
        };

        stream.seek(base_offset.wrapping_add(top.private_offset))?;
        stream.enter_frame(top.private_size)?;

        let r = {
            let c = stream.cursor();
            let data = &stream.frame_data()[c..];
            cff_parser_run(&mut parser, data)
        };
        drop(parser);
        stream.exit_frame();

        r
    })();

    if error.is_ok() {
        let priv_ = &mut subfont.private_dict;

        /* ensure that `num_blue_values' is even */
        priv_.num_blue_values &= !1;

        /* sanitize `initialRandomSeed' to be a positive value, if necessary;  */
        /* this is not mandated by the specification but by our implementation */
        if priv_.initial_random_seed < 0 {
            priv_.initial_random_seed = priv_.initial_random_seed.wrapping_neg();
        } else if priv_.initial_random_seed == 0 {
            priv_.initial_random_seed = 987654321;
        }

        /* some sanitizing to avoid overflows later on; */
        /* the upper limits are ad-hoc values           */
        if priv_.blue_shift > 1000 || priv_.blue_shift < 0 {
            priv_.blue_shift = 7;
        }

        if priv_.blue_fuzz > 1000 || priv_.blue_fuzz < 0 {
            priv_.blue_fuzz = 1;
        }
    }

    /* Exit: */
    /* clean up */
    cff_blend_clear(subfont); /* clear blend stack */
    /* (the parser's stack is freed with it) */

    error
}

/* There are 3 ways to call this function, distinguished by code.  */
/*                                                                 */
/* . CFF_CODE_TOPDICT for either a CFF Top DICT or a CFF Font DICT */
/* . CFF2_CODE_TOPDICT for CFF2 Top DICT                           */
/* . CFF2_CODE_FONTDICT for CFF2 Font DICT                         */

#[allow(clippy::too_many_arguments)]
fn cff_subfont_load(
    font: &mut CffFontRec,
    sub: CffSubFontId,
    idx: &CffIndexRec,
    font_index: FtUInt,
    stream: &mut FtStreamRec,
    base_offset: FtULong,
    code: FtUInt,
    face_random_seed: &mut FtInt32,
    driver: &FtModuleRec,
) -> FtResult<()> {
    let cff2 = code == CFF2_CODE_TOPDICT || code == CFF2_CODE_FONTDICT;
    let stack_size = if cff2 {
        CFF2_DEFAULT_STACK
    } else {
        CFF_MAX_STACK_DEPTH
    };

    {
        let subfont = font.subfont_mut(sub);

        /* Note: We use default stack size for CFF2 Font DICT because        */
        /*       Top and Font DICTs are not allowed to have blend operators. */
        let mut parser = cff_parser_init(
            code,
            CffParserObject::FontDict(&mut subfont.font_dict),
            stack_size,
            0,
            0,
        )?;

        /* set defaults */
        if let CffParserObject::FontDict(top) = &mut parser.object {
            **top = CffFontRecDictRec::default();

            top.underline_position = -(100 << 16);
            top.underline_thickness = 50 << 16;
            top.charstring_type = 2;
            top.font_matrix.xx = 0x10000;
            top.font_matrix.yy = 0x10000;
            top.cid_count = 8720;

            /* we use the implementation specific SID value 0xFFFF to indicate */
            /* missing entries                                                 */
            top.version = 0xFFFF;
            top.notice = 0xFFFF;
            top.copyright = 0xFFFF;
            top.full_name = 0xFFFF;
            top.family_name = 0xFFFF;
            top.weight = 0xFFFF;
            top.embedded_postscript = 0xFFFF;

            top.cid_registry = 0xFFFF;
            top.cid_ordering = 0xFFFF;
            top.cid_font_name = 0xFFFF;

            /* set default stack size */
            top.maxstack = if cff2 { CFF2_DEFAULT_STACK } else { 48 };
        }

        let dict: FtResult<Vec<u8>> = if idx.count != 0 {
            /* count is nonzero for a real index */
            cff_index_access_element(idx, stream, font_index)
        } else {
            /* CFF2 has a fake top dict index;     */
            /* simulate `cff_index_access_element' */

            /* Note: macros implicitly use `stream' and set `error' */
            stream.seek(idx.data_offset)?;
            Ok(stream.extract_frame(idx.data_size)?)
        };

        let error = match &dict {
            Ok(dict) => cff_parser_run(&mut parser, dict),
            Err(e) => Err(*e),
        };

        /* clean up regardless of error */
        drop(parser);
        drop(dict);

        error?;
    }

    let top = font.subfont(sub).font_dict;

    /* if it is a CID font, we stop there */
    if top.cid_registry != 0xFFFF {
        return Ok(());
    }

    /* Parse the private dictionary, if any.                   */
    /*                                                         */
    /* CFF2 does not have a private dictionary in the Top DICT */
    /* but may have one in a Font DICT.  We need to parse      */
    /* the latter here in order to load any local subrs.       */
    cff_load_private_dict(font, sub, stream, 0, None)?;

    let subfont = font.subfont_mut(sub);

    if !cff2 {
        /*
         * Initialize the random number generator.
         *
         * - If we have a face-specific seed, use it.
         *   If non-zero, update it to a positive value.
         *
         * - Otherwise, use the seed from the CFF driver.
         *   If non-zero, update it to a positive value.
         *
         * - If the random value is zero, use the seed given by the subfont's
         *   `initialRandomSeed' value.
         *
         */
        if *face_random_seed == -1 {
            driver.with_props(|driver: &mut PsDriverRec| {
                subfont.random = driver.random_seed as FtUInt32;
                if driver.random_seed != 0 {
                    loop {
                        driver.random_seed = cff_random(driver.random_seed as FtUInt32) as FtInt32;

                        if driver.random_seed >= 0 {
                            break;
                        }
                    }
                }
            });
        } else {
            subfont.random = *face_random_seed as FtUInt32;
            if *face_random_seed != 0 {
                loop {
                    *face_random_seed = cff_random(*face_random_seed as FtUInt32) as FtInt32;

                    if *face_random_seed >= 0 {
                        break;
                    }
                }
            }
        }

        if subfont.random == 0 {
            subfont.random = subfont.private_dict.initial_random_seed as FtUInt32;
        }
    }

    /* read the local subrs, if any */
    if subfont.private_dict.local_subrs_offset != 0 {
        stream.seek(
            base_offset
                .wrapping_add(top.private_offset)
                .wrapping_add(subfont.private_dict.local_subrs_offset),
        )?;

        cff_index_init(&mut subfont.local_subrs_index, stream, true, cff2)?;

        subfont.local_subrs = cff_index_get_pointers(&mut subfont.local_subrs_index, stream)?;
    }

    /* Exit: */
    /* (the parser's stack is freed with it) */
    Ok(())
}

fn cff_subfont_done(subfont: &mut CffSubFontRec) {
    cff_index_done(&mut subfont.local_subrs_index);
    subfont.local_subrs = None;

    subfont.blend.lastNDV = Vec::new();
    subfont.blend.BV = Vec::new();
    subfont.blend_stack = Vec::new();
}

/// `cff_font_load`: `face_random_seed` is the face's
/// `internal->random_seed`, `driver` the face's driver module.
#[allow(clippy::too_many_arguments)]
pub fn cff_font_load(
    stream: &mut FtStreamRec,
    face_index: FtInt,
    font: &mut CffFontRec,
    face_random_seed: &mut FtInt32,
    driver: &FtModuleRec,
    pure_cff: bool,
    cff2: bool,
) -> FtResult<()> {
    let mut string_index = CffIndexRec::default();

    let r = cff_font_load_inner(
        stream,
        face_index,
        font,
        face_random_seed,
        driver,
        pure_cff,
        cff2,
        &mut string_index,
    );

    /* Exit: */
    cff_index_done(&mut string_index);

    r
}

#[allow(clippy::too_many_arguments)]
fn cff_font_load_inner(
    stream: &mut FtStreamRec,
    face_index: FtInt,
    font: &mut CffFontRec,
    face_random_seed: &mut FtInt32,
    driver: &FtModuleRec,
    pure_cff: bool,
    cff2: bool,
    string_index: &mut CffIndexRec,
) -> FtResult<()> {
    *font = CffFontRec::default();

    let base_offset: FtULong = stream.pos();

    font.cff2 = cff2;
    font.base_offset = base_offset;

    /* read CFF font header */
    stream.enter_frame(3)?;
    font.version_major = stream.get_byte();
    font.version_minor = stream.get_byte();
    font.header_size = stream.get_byte();
    stream.exit_frame();

    if cff2 {
        if font.version_major != 2 || font.header_size < 5 {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }

        font.top_dict_length = stream.read_ushort()? as FtUInt;
    } else {
        let absolute_offset = stream.read_byte()?;

        if font.version_major != 1 || font.header_size < 4 || absolute_offset > 4 {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }
    }

    /* skip the rest of the header */
    if let Err(e) = stream.seek(base_offset + font.header_size as FtULong) {
        /* For pure CFFs we have read only four bytes so far.  Contrary to */
        /* other formats like SFNT those bytes doesn't define a signature; */
        /* it is thus possible that the font isn't a CFF at all.           */
        if pure_cff {
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }
        return Err(e);
    }

    if cff2 {
        /* For CFF2, the top dict data immediately follow the header    */
        /* and the length is stored in the header `offSize' field;      */
        /* there is no index for it.                                    */
        /*                                                              */
        /* Use the `font_dict_index' to save the current position       */
        /* and length of data, but leave count at zero as an indicator. */
        font.font_dict_index = CffIndexRec::default();

        font.font_dict_index.data_offset = stream.pos();
        font.font_dict_index.data_size = font.top_dict_length as FtULong;

        /* skip the top dict data for now, we will parse it later */
        stream.skip(font.top_dict_length as FtLong)?;

        /* next, read the global subrs index */
        cff_index_init(&mut font.global_subrs_index, stream, true, cff2)?;
    } else {
        /* for CFF, read the name, top dict, string and global subrs index */
        if let Err(e) = cff_index_init(&mut font.name_index, stream, false, cff2) {
            if pure_cff {
                return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
            }
            return Err(e);
        }

        /* if we have an empty font name,      */
        /* it must be the only font in the CFF */
        if font.name_index.count > 1 && font.name_index.data_size < font.name_index.count as FtULong
        {
            /* for pure CFFs, we still haven't checked enough bytes */
            /* to be sure that it is a CFF at all                   */
            return Err(if pure_cff {
                FT_ERR_UNKNOWN_FILE_FORMAT
            } else {
                FT_ERR_INVALID_FILE_FORMAT
            });
        }

        cff_index_init(&mut font.font_dict_index, stream, false, cff2)?;
        cff_index_init(string_index, stream, true, cff2)?;
        cff_index_init(&mut font.global_subrs_index, stream, true, cff2)?;
        if let Some((strings, pool, pool_size)) =
            cff_index_get_pointers_pool(string_index, stream, true)?
        {
            font.strings = Some(strings);
            font.string_pool = pool;
            font.string_pool_size = pool_size;
        }

        /* there must be a Top DICT index entry for each name index entry */
        if font.name_index.count > font.font_dict_index.count {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
    }

    font.num_strings = string_index.count;

    let subfont_index: FtUInt;
    if pure_cff {
        /* well, we don't really forget the `disabled' fonts... */
        subfont_index = (face_index & 0xFFFF) as FtUInt;

        if face_index > 0 && subfont_index >= font.name_index.count {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        font.num_faces = font.name_index.count;
    } else {
        subfont_index = 0;

        if font.name_index.count > 1 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }
    }

    /* in case of a font format check, simply exit now */
    if face_index < 0 {
        return Ok(());
    }

    /* now, parse the top-level font dictionary */
    let font_dict_index = font.font_dict_index.clone();
    cff_subfont_load(
        font,
        CffSubFontId::Top,
        &font_dict_index,
        subfont_index,
        stream,
        base_offset,
        if cff2 {
            CFF2_CODE_TOPDICT
        } else {
            CFF_CODE_TOPDICT
        },
        face_random_seed,
        driver,
    )?;

    let dict = font.top_font.font_dict;

    stream.seek(base_offset.wrapping_add(dict.charstrings_offset))?;

    cff_index_init(&mut font.charstrings_index, stream, false, cff2)?;

    /* now, check for a CID or CFF2 font */
    if dict.cid_registry != 0xFFFF || cff2 {
        let mut fd_index = CffIndexRec::default();

        /* for CFF2, read the Variation Store if available;                 */
        /* this must follow the Top DICT parse and precede any Private DICT */
        cff_vstore_load(&mut font.vstore, stream, base_offset, dict.vstore_offset)?;

        /* this is a CID-keyed font, we must now allocate a table of */
        /* sub-fonts, then load each of them separately              */
        stream.seek(base_offset.wrapping_add(dict.cid_fd_array_offset))?;

        cff_index_init(&mut fd_index, stream, false, cff2)?;

        let error = (|| -> FtResult<()> {
            /* Font Dicts are not limited to 256 for CFF2. */
            /* TODO: support this for CFF2                 */
            if fd_index.count as usize > CFF_MAX_CID_FONTS {
                /* FD array too large in CID font */
                return Ok(());
            }

            /* allocate & read each font dict independently */
            font.num_subfonts = fd_index.count;
            font.subfonts = super::super::base::ftmemory::ft_new_array(fd_index.count as FtLong)?;

            /* now load each subfont independently */
            for idx in 0..fd_index.count {
                cff_subfont_load(
                    font,
                    CffSubFontId::Sub(idx as usize),
                    &fd_index,
                    idx,
                    stream,
                    base_offset,
                    if cff2 {
                        CFF2_CODE_FONTDICT
                    } else {
                        CFF_CODE_TOPDICT
                    },
                    face_random_seed,
                    driver,
                )?;
            }

            /* now load the FD Select array;               */
            /* CFF2 omits FDSelect if there is only one FD */
            if !cff2 || fd_index.count > 1 {
                cff_load_fd_select(
                    &mut font.fd_select,
                    font.charstrings_index.count,
                    stream,
                    base_offset.wrapping_add(dict.cid_fd_select_offset),
                )?;
            }

            Ok(())
        })();

        /* Fail_CID: */
        cff_index_done(&mut fd_index);

        error?;
    } else {
        font.num_subfonts = 0;
    }

    /* read the charstrings index now */
    if dict.charstrings_offset == 0 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    font.num_glyphs = font.charstrings_index.count;

    font.global_subrs = cff_index_get_pointers(&mut font.global_subrs_index, stream)?;

    /* read the Charset and Encoding tables if available */
    if !cff2 && font.num_glyphs > 0 {
        let invert = dict.cid_registry != 0xFFFF && pure_cff;

        cff_charset_load(
            &mut font.charset,
            font.num_glyphs,
            stream,
            base_offset,
            dict.charset_offset,
            invert,
        )?;

        /* CID-keyed CFFs don't have an encoding */
        if dict.cid_registry == 0xFFFF {
            cff_encoding_load(
                &mut font.encoding,
                &mut font.charset,
                font.num_glyphs,
                stream,
                base_offset,
                dict.encoding_offset,
            )?;
        }
    }

    /* get the font name (/CIDFontName for CID-keyed fonts, */
    /* /FontName otherwise)                                 */
    font.font_name = cff_index_get_name(font, stream, subfont_index);

    Ok(())
}

/// `cff_font_done`
pub fn cff_font_done(font: &mut CffFontRec) {
    cff_index_done(&mut font.global_subrs_index);
    cff_index_done(&mut font.font_dict_index);
    cff_index_done(&mut font.name_index);
    cff_index_done(&mut font.charstrings_index);

    /* release font dictionaries, but only if working with */
    /* a CID keyed CFF font or a CFF2 font                 */
    if font.num_subfonts > 0 {
        for sub in font.subfonts.iter_mut() {
            cff_subfont_done(sub);
        }

        /* the subfonts array has been allocated as a single block */
        font.subfonts = Vec::new();
    }

    cff_encoding_done(&mut font.encoding);
    cff_charset_done(&mut font.charset);
    cff_vstore_done(&mut font.vstore);

    cff_subfont_done(&mut font.top_font);

    cff_done_fd_select(&mut font.fd_select);

    font.font_info = None;

    font.font_name = None;
    font.global_subrs = None;
    font.strings = None;
    font.string_pool = Vec::new();

    /* (the CF2 instance's finalizer frees its blend vectors) */
    font.cf2_instance = None;

    font.font_extra = None;
}
