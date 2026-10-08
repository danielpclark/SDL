// Rust translation of src/sfnt/sfwoff.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! WOFF format management (base).

use std::sync::Arc;

use super::super::base::ftmemory::ft_qalloc;
use super::super::base::ftstream::{FtStream, FtStreamRec};
use super::super::fttypes::*;
use super::super::gzip::ftgzip::ft_gzip_uncompress;
use super::super::tttables::*;
use super::super::tttypes::*;

/// `WRITE_USHORT`
fn write_ushort(buf: &mut [u8], p: &mut usize, v: FtUInt) {
    buf[*p] = (v >> 8) as FtByte;
    buf[*p + 1] = v as FtByte;
    *p += 2;
}

/// `WRITE_ULONG`
fn write_ulong(buf: &mut [u8], p: &mut usize, v: FtULong) {
    buf[*p] = (v >> 24) as FtByte;
    buf[*p + 1] = (v >> 16) as FtByte;
    buf[*p + 2] = (v >> 8) as FtByte;
    buf[*p + 3] = v as FtByte;
    *p += 4;
}

/// `WOFF_HeaderRec`
#[derive(Debug, Clone, Copy, Default)]
struct WoffHeaderRec {
    signature: FtULong,
    flavor: FtULong,
    length: FtULong,
    num_tables: FtUShort,
    reserved: FtUShort,
    total_sfnt_size: FtULong,
    major_version: FtUShort,
    minor_version: FtUShort,
    meta_offset: FtULong,
    meta_length: FtULong,
    meta_orig_length: FtULong,
    priv_offset: FtULong,
    priv_length: FtULong,
}

/// `WOFF_TableRec`
#[derive(Debug, Clone, Copy, Default)]
struct WoffTableRec {
    tag: FtULong,         /* table ID                  */
    offset: FtULong,      /* table file offset         */
    comp_length: FtULong, /* compressed table length   */
    orig_length: FtULong, /* uncompressed table length */
    check_sum: FtULong,   /* uncompressed checksum     */

    orig_offset: FtULong, /* uncompressed table file offset */
                          /* (not in the WOFF file)         */
}

/// `woff_open_font`: Replace `face->root.stream' with a stream containing
/// the extracted SFNT of a WOFF font.
///
/// Returns the new stream; the caller swaps it in (C does that here).
pub fn woff_open_font(stream: &mut FtStreamRec, _face: &mut TtFaceRec) -> FtResult<FtStream> {
    let mut old_tag: FtULong = 0;

    stream.enter_frame(44)?;
    let woff = WoffHeaderRec {
        signature: stream.get_ulong() as FtULong,
        flavor: stream.get_ulong() as FtULong,
        length: stream.get_ulong() as FtULong,
        num_tables: stream.get_ushort(),
        reserved: stream.get_ushort(),
        total_sfnt_size: stream.get_ulong() as FtULong,
        major_version: stream.get_ushort(),
        minor_version: stream.get_ushort(),
        meta_offset: stream.get_ulong() as FtULong,
        meta_length: stream.get_ulong() as FtULong,
        meta_orig_length: stream.get_ulong() as FtULong,
        priv_offset: stream.get_ulong() as FtULong,
        priv_length: stream.get_ulong() as FtULong,
    };
    stream.exit_frame();
    let _ = (
        woff.signature,
        woff.reserved,
        woff.major_version,
        woff.minor_version,
    );

    /* Make sure we don't recurse back here or hit TTC code. */
    if woff.flavor == TTAG_wOFF as FtULong || woff.flavor == TTAG_ttcf as FtULong {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* Miscellaneous checks. */
    if woff.length != stream.size
        || woff.num_tables == 0
        || 44 + woff.num_tables as FtULong * 20 >= woff.length
        || 12 + woff.num_tables as FtULong * 16 >= woff.total_sfnt_size
        || (woff.total_sfnt_size & 3) != 0
        || (woff.meta_offset == 0 && (woff.meta_length != 0 || woff.meta_orig_length != 0))
        || (woff.meta_length != 0 && woff.meta_orig_length == 0)
        || (woff.priv_offset == 0 && woff.priv_length != 0)
    {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* Don't trust `totalSfntSize' before thorough checks. */
    let mut sfnt = ft_qalloc(12)?;
    let mut sfnt_header = 0usize;

    /* Write sfnt header. */
    {
        let mut x = woff.num_tables as FtUInt;
        let mut entry_selector: FtUInt = 0;
        while x != 0 {
            x >>= 1;
            entry_selector += 1;
        }
        entry_selector -= 1;

        let search_range = (1 << entry_selector) * 16;
        let range_shift = (woff.num_tables as FtUInt * 16).wrapping_sub(search_range);

        write_ulong(&mut sfnt, &mut sfnt_header, woff.flavor);
        write_ushort(&mut sfnt, &mut sfnt_header, woff.num_tables as FtUInt);
        write_ushort(&mut sfnt, &mut sfnt_header, search_range);
        write_ushort(&mut sfnt, &mut sfnt_header, entry_selector);
        write_ushort(&mut sfnt, &mut sfnt_header, range_shift);
    }

    /* While the entries in the sfnt header must be sorted by the */
    /* tag value, the tables themselves are not.  We thus have to */
    /* sort them by offset and check that they don't overlap.     */
    let mut tables: Vec<WoffTableRec> =
        super::super::base::ftmemory::ft_new_array(woff.num_tables as FtLong)?;
    let mut indices: Vec<usize> =
        super::super::base::ftmemory::ft_new_array(woff.num_tables as FtLong)?;

    stream.enter_frame(20 * woff.num_tables as FtULong)?;

    for nn in 0..woff.num_tables as usize {
        let table = &mut tables[nn];

        table.tag = stream.get_ulong() as FtULong;
        table.offset = stream.get_ulong() as FtULong;
        table.comp_length = stream.get_ulong() as FtULong;
        table.orig_length = stream.get_ulong() as FtULong;
        table.check_sum = stream.get_ulong() as FtULong;

        if table.tag <= old_tag {
            stream.exit_frame();

            return Err(FT_ERR_INVALID_TABLE);
        }

        old_tag = table.tag;
        indices[nn] = nn;
    }

    stream.exit_frame();

    /* Sort by offset. */
    /* (glibc's qsort is a stable merge sort for arrays like this one) */
    indices.sort_by(|&a, &b| tables[a].offset.cmp(&tables[b].offset));

    /* Check offsets and lengths. */
    let mut woff_offset: FtULong = 44 + woff.num_tables as FtULong * 20;
    let mut sfnt_offset: FtULong = 12 + woff.num_tables as FtULong * 16;

    for &i in &indices {
        let table = &mut tables[i];

        if table.offset != woff_offset
            || table.comp_length > woff.length
            || table.offset > woff.length - table.comp_length
            || table.orig_length > woff.total_sfnt_size
            || sfnt_offset > woff.total_sfnt_size - table.orig_length
            || table.comp_length > table.orig_length
        {
            return Err(FT_ERR_INVALID_TABLE);
        }

        table.orig_offset = sfnt_offset;

        /* The offsets must be multiples of 4. */
        woff_offset += (table.comp_length + 3) & !3;
        sfnt_offset += (table.orig_length + 3) & !3;
    }

    /*
     * Final checks!
     *
     * We don't decode and check the metadata block.
     * We don't check table checksums either.
     * But other than those, I think we implement all
     * `MUST' checks from the spec.
     */

    if woff.meta_offset != 0 {
        if woff.meta_offset != woff_offset || woff.meta_offset + woff.meta_length > woff.length {
            return Err(FT_ERR_INVALID_TABLE);
        }

        /* We have padding only ... */
        woff_offset += woff.meta_length;
    }

    if woff.priv_offset != 0 {
        /* ... if it isn't the last block. */
        woff_offset = (woff_offset + 3) & !3;

        if woff.priv_offset != woff_offset || woff.priv_offset + woff.priv_length > woff.length {
            return Err(FT_ERR_INVALID_TABLE);
        }

        /* No padding for the last block. */
        woff_offset += woff.priv_length;
    }

    if sfnt_offset != woff.total_sfnt_size || woff_offset != woff.length {
        return Err(FT_ERR_INVALID_TABLE);
    }

    /* Now use `totalSfntSize'. */
    {
        let mut bigger = ft_qalloc(woff.total_sfnt_size as FtLong)?;
        bigger[..12].copy_from_slice(&sfnt[..12]);
        sfnt = bigger;
    }
    let mut sfnt_header = 12usize;

    /* Write the tables. */
    for table in &tables {
        /* Write SFNT table entry. */
        write_ulong(&mut sfnt, &mut sfnt_header, table.tag);
        write_ulong(&mut sfnt, &mut sfnt_header, table.check_sum);
        write_ulong(&mut sfnt, &mut sfnt_header, table.orig_offset);
        write_ulong(&mut sfnt, &mut sfnt_header, table.orig_length);

        /* Write table data. */
        stream.seek(table.offset)?;
        stream.enter_frame(table.comp_length)?;

        let orig = table.orig_offset as usize;
        let olen = table.orig_length as usize;
        if table.comp_length == table.orig_length {
            /* Uncompressed data; just copy. */
            sfnt[orig..orig + olen].copy_from_slice(&stream.frame_data()[..olen]);
        } else {
            /* Uncompress with zlib. */
            let mut output_len = table.orig_length;

            let r = ft_gzip_uncompress(
                &mut sfnt[orig..orig + olen],
                &mut output_len,
                stream.frame_data(),
            );
            if let Err(e) = r {
                /* Exit1: */
                stream.exit_frame();
                return Err(e);
            }

            if output_len != table.orig_length {
                /* Exit1: */
                stream.exit_frame();
                return Err(FT_ERR_INVALID_TABLE);
            }
        }

        stream.exit_frame();

        /* We don't check whether the padding bytes in the WOFF file are     */
        /* actually '\0'.  For the output, however, we do set them properly. */
        let mut sfnt_offset = table.orig_offset + table.orig_length;
        while sfnt_offset & 3 != 0 {
            sfnt[sfnt_offset as usize] = 0;
            sfnt_offset += 1;
        }
    }

    /* Ok!  Finally ready.  Swap out stream and return. */
    Ok(FtStreamRec::open_memory(Arc::from(sfnt)))
}
