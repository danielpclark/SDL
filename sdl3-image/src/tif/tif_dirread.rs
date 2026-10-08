// Rust translation of libtiff/tif_dirread.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Directory Read Support Routines.
//!
//! The image directory reader. The custom directory readers
//! (`TIFFReadCustomDirectory()`, `TIFFReadEXIFDirectory()`,
//! `TIFFReadGPSDirectory()` with `TIFFFetchSubjectDistance()`) and the
//! deferred and lazy strile loading (the 'D' and 'O' open flags, which
//! SDL_image doesn't pass: `_TIFFPartialReadStripArray()` and
//! `_TIFFFetchStrileValue()`) are left out, as is the memory-mapped path
//! (SDL_image opens with 'm'). The entry readers keep the C's checks
//! (their counts, accepted types, ranges and error codes); the
//! repetitive per-type functions of the C are generic here over the
//! destination type.

/* Suggested pending improvements:
 * - add a field 'field_info' to the TIFFDirEntry structure, and set that with
 *   the pointer to the appropriate TIFFField structure early on in
 *   TIFFReadDirectory, so as to eliminate current possibly repetitive lookup.
 */

use std::collections::HashMap;

use super::tif_aux::_tiff_check_malloc_vec;
use super::tif_dir::*;
use super::tif_dirinfo::{
    _tiff_check_field_is_valid_for_codec, _tiff_create_anon_field, _tiff_merge_fields,
    tiff_data_width, tiff_field_with_tag,
};
use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tif_strip::{
    tiff_number_of_strips, tiff_scanline_size, tiff_scanline_size64, tiff_strip_size,
    tiff_strip_size64, tiff_vstrip_size64,
};
use super::tif_tile::{tiff_number_of_tiles, tiff_tile_size, tiff_tile_size64, tiff_vtile_size64};
use super::tiff::*;
use super::tiffio::TIFFField;
use super::tiffiop::*;

const FAILED_FII: u32 = u32::MAX;

/// Translation of `enum TIFFReadDirEntryErr`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TIFFReadDirEntryErr {
    Ok = 0,
    Count = 1,
    Type = 2,
    Io = 3,
    Range = 4,
    Psdif = 5,
    Sizesan = 6,
    Alloc = 7,
}

use TIFFReadDirEntryErr as Err;

type EResult<T> = Result<T, TIFFReadDirEntryErr>;

/// Translation of `TIFFReadUInt64()`: Unaligned safe copy of a uint64_t
/// value from an octet array.
fn tiff_read_uint64(value: &[u8]) -> u64 {
    let mut c = [0u8; 8];
    for (d, s) in c.iter_mut().zip(value) {
        *d = *s;
    }
    u64::from_ne_bytes(c)
}

/*
 * The scalar readers: TIFFReadDirEntryByte(), TIFFReadDirEntrySbyte(),
 * TIFFReadDirEntryShort(), TIFFReadDirEntrySshort(), TIFFReadDirEntryLong(),
 * TIFFReadDirEntrySlong(), TIFFReadDirEntryLong8() and
 * TIFFReadDirEntrySlong8() read a value of any integer type and check that
 * it fits the destination (the TIFFReadDirEntryCheckRange*() functions);
 * Byte and Sbyte also take TIFF_UNDEFINED, Long and Long8 TIFF_IFD and
 * TIFF_IFD8.
 */

/// Translation of `TIFFReadDirEntryByte()` and its integer siblings: one
/// value of type `T`, `undefined` and `ifd` saying whether TIFF_UNDEFINED
/// and TIFF_IFD/TIFF_IFD8 are accepted.
fn tiff_read_dir_entry_int<T: TryFrom<i128>>(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
    undefined: bool,
    ifd: bool,
) -> EResult<T> {
    if direntry.tdir_count != 1 {
        return Err(Err::Count);
    }
    let m: i128 = match direntry.tdir_type {
        TIFF_BYTE => tiff_read_dir_entry_checked_byte(direntry) as i128,
        TIFF_UNDEFINED if undefined => {
            /* Support to read TIFF_UNDEFINED with field_readcount==1 */
            tiff_read_dir_entry_checked_byte(direntry) as i128
        }
        TIFF_SBYTE => tiff_read_dir_entry_checked_sbyte(direntry) as i128,
        TIFF_SHORT => tiff_read_dir_entry_checked_short(tif, direntry) as i128,
        TIFF_SSHORT => tiff_read_dir_entry_checked_sshort(tif, direntry) as i128,
        TIFF_LONG => tiff_read_dir_entry_checked_long(tif, direntry) as i128,
        TIFF_IFD if ifd => tiff_read_dir_entry_checked_long(tif, direntry) as i128,
        TIFF_SLONG => tiff_read_dir_entry_checked_slong(tif, direntry) as i128,
        TIFF_LONG8 => tiff_read_dir_entry_checked_long8(tif, direntry)? as i128,
        TIFF_IFD8 if ifd => tiff_read_dir_entry_checked_long8(tif, direntry)? as i128,
        TIFF_SLONG8 => tiff_read_dir_entry_checked_slong8(tif, direntry)? as i128,
        _ => return Err(Err::Type),
    };
    T::try_from(m).map_err(|_| Err::Range)
}

/// Translation of `TIFFReadDirEntryByte()`.
fn tiff_read_dir_entry_byte(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<u8> {
    tiff_read_dir_entry_int(tif, d, true, false)
}

/// Translation of `TIFFReadDirEntrySbyte()`.
fn tiff_read_dir_entry_sbyte(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<i8> {
    tiff_read_dir_entry_int(tif, d, true, false)
}

/// Translation of `TIFFReadDirEntryShort()`.
fn tiff_read_dir_entry_short(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<u16> {
    tiff_read_dir_entry_int(tif, d, false, false)
}

/// Translation of `TIFFReadDirEntrySshort()`.
fn tiff_read_dir_entry_sshort(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<i16> {
    tiff_read_dir_entry_int(tif, d, false, false)
}

/// Translation of `TIFFReadDirEntryLong()`.
fn tiff_read_dir_entry_long(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<u32> {
    tiff_read_dir_entry_int(tif, d, false, true)
}

/// Translation of `TIFFReadDirEntrySlong()`.
fn tiff_read_dir_entry_slong(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<i32> {
    tiff_read_dir_entry_int(tif, d, false, false)
}

/// Translation of `TIFFReadDirEntryLong8()`.
fn tiff_read_dir_entry_long8(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<u64> {
    tiff_read_dir_entry_int(tif, d, false, true)
}

/// Translation of `TIFFReadDirEntrySlong8()`.
fn tiff_read_dir_entry_slong8(tif: &mut Tiff<'_>, d: &TIFFDirEntry) -> EResult<i64> {
    tiff_read_dir_entry_int(tif, d, false, false)
}

/// Translation of `TIFFReadDirEntryFloat()`.
fn tiff_read_dir_entry_float(tif: &mut Tiff<'_>, direntry: &TIFFDirEntry) -> EResult<f32> {
    if direntry.tdir_count != 1 {
        return Err(Err::Count);
    }
    Ok(match direntry.tdir_type {
        TIFF_BYTE => tiff_read_dir_entry_checked_byte(direntry) as f32,
        TIFF_SBYTE => tiff_read_dir_entry_checked_sbyte(direntry) as f32,
        TIFF_SHORT => tiff_read_dir_entry_checked_short(tif, direntry) as f32,
        TIFF_SSHORT => tiff_read_dir_entry_checked_sshort(tif, direntry) as f32,
        TIFF_LONG => tiff_read_dir_entry_checked_long(tif, direntry) as f32,
        TIFF_SLONG => tiff_read_dir_entry_checked_slong(tif, direntry) as f32,
        TIFF_LONG8 => tiff_read_dir_entry_checked_long8(tif, direntry)? as f32,
        TIFF_SLONG8 => tiff_read_dir_entry_checked_slong8(tif, direntry)? as f32,
        TIFF_RATIONAL => tiff_read_dir_entry_checked_rational(tif, direntry)? as f32,
        TIFF_SRATIONAL => tiff_read_dir_entry_checked_srational(tif, direntry)? as f32,
        TIFF_FLOAT => tiff_read_dir_entry_checked_float(tif, direntry),
        TIFF_DOUBLE => {
            let m = tiff_read_dir_entry_checked_double(tif, direntry)?;
            if (m > f32::MAX as f64) || (m < -(f32::MAX as f64)) {
                return Err(Err::Range);
            }
            m as f32
        }
        _ => return Err(Err::Type),
    })
}

/// Translation of `TIFFReadDirEntryDouble()`.
fn tiff_read_dir_entry_double(tif: &mut Tiff<'_>, direntry: &TIFFDirEntry) -> EResult<f64> {
    if direntry.tdir_count != 1 {
        return Err(Err::Count);
    }
    Ok(match direntry.tdir_type {
        TIFF_BYTE => tiff_read_dir_entry_checked_byte(direntry) as f64,
        TIFF_SBYTE => tiff_read_dir_entry_checked_sbyte(direntry) as f64,
        TIFF_SHORT => tiff_read_dir_entry_checked_short(tif, direntry) as f64,
        TIFF_SSHORT => tiff_read_dir_entry_checked_sshort(tif, direntry) as f64,
        TIFF_LONG => tiff_read_dir_entry_checked_long(tif, direntry) as f64,
        TIFF_SLONG => tiff_read_dir_entry_checked_slong(tif, direntry) as f64,
        TIFF_LONG8 => tiff_read_dir_entry_checked_long8(tif, direntry)? as f64,
        TIFF_SLONG8 => tiff_read_dir_entry_checked_slong8(tif, direntry)? as f64,
        TIFF_RATIONAL => tiff_read_dir_entry_checked_rational(tif, direntry)?,
        TIFF_SRATIONAL => tiff_read_dir_entry_checked_srational(tif, direntry)?,
        TIFF_FLOAT => tiff_read_dir_entry_checked_float(tif, direntry) as f64,
        TIFF_DOUBLE => tiff_read_dir_entry_checked_double(tif, direntry)?,
        _ => return Err(Err::Type),
    })
}

/// Translation of `TIFFReadDirEntryIfd8()`.
fn tiff_read_dir_entry_ifd8(tif: &mut Tiff<'_>, direntry: &TIFFDirEntry) -> EResult<u64> {
    if direntry.tdir_count != 1 {
        return Err(Err::Count);
    }
    match direntry.tdir_type {
        TIFF_LONG | TIFF_IFD => Ok(tiff_read_dir_entry_checked_long(tif, direntry) as u64),
        TIFF_LONG8 | TIFF_IFD8 => tiff_read_dir_entry_checked_long8(tif, direntry),
        _ => Err(Err::Type),
    }
}

const INITIAL_THRESHOLD: TmSize = 1024 * 1024;
const THRESHOLD_MULTIPLIER: TmSize = 10;
const MAX_THRESHOLD: TmSize =
    THRESHOLD_MULTIPLIER * THRESHOLD_MULTIPLIER * THRESHOLD_MULTIPLIER * INITIAL_THRESHOLD;

/// Translation of `TIFFReadDirEntryDataAndRealloc()`.
fn tiff_read_dir_entry_data_and_realloc(
    tif: &mut Tiff<'_>,
    offset: u64,
    size: TmSize,
    pdest: &mut Vec<u8>,
) -> EResult<()> {
    let mut threshold: TmSize = INITIAL_THRESHOLD;
    let mut already_read: TmSize = 0;

    if !tif.seek_ok(offset) {
        return Err(Err::Io);
    }

    /* On 64 bit processes, read first a maximum of 1 MB, then 10 MB, etc */
    /* so as to avoid allocating too much memory in case the file is too */
    /* short. We could ask for the file size, but this might be */
    /* expensive with some I/O layers (think of reading a gzipped file) */
    /* Restrict to 64 bit processes, so as to avoid reallocs() */
    /* on 32 bit processes where virtual memory is scarce.  */
    while already_read < size {
        let mut to_read = size - already_read;
        if cfg!(target_pointer_width = "64") && to_read >= threshold && threshold < MAX_THRESHOLD {
            to_read = threshold;
            threshold *= THRESHOLD_MULTIPLIER;
        }

        let new_len = (already_read + to_read) as usize;
        if pdest.try_reserve_exact(new_len - pdest.len()).is_err() {
            tiff_error_ext_r!(
                tif.tif_name,
                "Failed to allocate memory for {} ({} elements of {} bytes each)",
                "TIFFReadDirEntryArray",
                1,
                already_read + to_read
            );
            return Err(Err::Alloc);
        }
        pdest.resize(new_len, 0);

        let bytes_read = tif.read_file(&mut pdest[already_read as usize..new_len]);
        if bytes_read < 0 {
            return Err(Err::Io);
        }
        already_read += bytes_read;
        if bytes_read != to_read {
            return Err(Err::Io);
        }
    }
    Ok(())
}

/* Caution: if raising that value, make sure int32 / uint32 overflows can't
 * occur elsewhere */
const MAX_SIZE_TAG_DATA: u32 = 2147483647;

/// Translation of `TIFFReadDirEntryArrayWithLimit()`: the raw values of
/// an entry (as the file has them, unswabbed) and their count; `None`
/// where the C returns a `NULL` array (no values).
fn tiff_read_dir_entry_array_with_limit(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
    desttypesize: u32,
    maxcount: u64,
) -> EResult<Option<(Vec<u8>, u32)>> {
    let typesize = tiff_data_width(direntry.tdir_type);

    let target_count64 = if direntry.tdir_count > maxcount {
        maxcount
    } else {
        direntry.tdir_count
    };

    if (target_count64 == 0) || (typesize == 0) {
        return Ok(None);
    }

    /* We just want to know if the original tag size is more than 4 bytes
     * (classic TIFF) or 8 bytes (BigTIFF)
     */
    let original_datasize_clamped = (if direntry.tdir_count > 10 {
        10
    } else {
        direntry.tdir_count as i32
    }) * typesize;

    /*
     * As a sanity check, make sure we have no more than a 2GB tag array
     * in either the current data type or the dest data type.  This also
     * avoids problems with overflow of tmsize_t on 32bit systems.
     */
    if ((MAX_SIZE_TAG_DATA / typesize as u32) as u64) < target_count64 {
        return Err(Err::Sizesan);
    }
    if ((MAX_SIZE_TAG_DATA / desttypesize) as u64) < target_count64 {
        return Err(Err::Sizesan);
    }

    let count = target_count64 as u32;
    let datasize = count * typesize as u32;

    if datasize > 100 * 1024 * 1024 {
        /* Before allocating a huge amount of memory for corrupted files, check
         * if size of requested memory is not greater than file size.
         */
        let filesize = tif.get_file_size();
        if datasize as u64 > filesize {
            tiff_warning_ext_r!(
                "ReadDirEntryArray",
                "Requested memory size for tag {} (0x{:x}) {} is greater than filesize {}. Memory not allocated, tag not read",
                direntry.tdir_tag,
                direntry.tdir_tag,
                datasize,
                filesize
            );
            return Err(Err::Alloc);
        }
    }

    // (not mapped)
    let mut data: Vec<u8>;
    if ((tif.tif_flags & TIFF_BIGTIFF) != 0 && datasize > 8)
        || ((tif.tif_flags & TIFF_BIGTIFF) == 0 && datasize > 4)
    {
        data = Vec::new();
    } else {
        match _tiff_check_malloc_vec::<u8>(
            tif,
            count as TmSize,
            typesize as TmSize,
            "ReadDirEntryArray",
        ) {
            Some(d) => data = d,
            None => return Err(Err::Alloc),
        }
    }
    if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
        /* Only the condition on original_datasize_clamped. The second
         * one is implied, but Coverity Scan cannot see it. */
        if original_datasize_clamped <= 4 && datasize <= 4 {
            data[..datasize as usize].copy_from_slice(&direntry.tdir_offset[..datasize as usize]);
        } else {
            let mut offset = direntry.toff_long();
            if (tif.tif_flags & TIFF_SWAB) != 0 {
                offset = offset.swap_bytes();
            }
            tiff_read_dir_entry_data_and_realloc(
                tif,
                offset as u64,
                datasize as TmSize,
                &mut data,
            )?;
        }
    } else {
        /* See above comment for the Classic TIFF case */
        if original_datasize_clamped <= 8 && datasize <= 8 {
            data[..datasize as usize].copy_from_slice(&direntry.tdir_offset[..datasize as usize]);
        } else {
            let mut offset = direntry.toff_long8();
            if (tif.tif_flags & TIFF_SWAB) != 0 {
                offset = offset.swap_bytes();
            }
            tiff_read_dir_entry_data_and_realloc(tif, offset, datasize as TmSize, &mut data)?;
        }
    }
    data.truncate(datasize as usize);
    Ok(Some((data, count)))
}

/// Translation of `TIFFReadDirEntryArray()`.
fn tiff_read_dir_entry_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
    desttypesize: u32,
) -> EResult<Option<(Vec<u8>, u32)>> {
    tiff_read_dir_entry_array_with_limit(tif, direntry, desttypesize, !0u64)
}

/// Element `n` of a raw array of `type_` values, swabbed, as an integer.
fn raw_int(tif: &Tiff<'_>, type_: TIFFDataType, raw: &[u8], n: usize) -> i128 {
    let swab = (tif.tif_flags & TIFF_SWAB) != 0;
    let get = |size: usize| -> [u8; 8] {
        let mut b = [0u8; 8];
        let s = &raw[n * size..n * size + size];
        b[..size].copy_from_slice(s);
        if swab {
            b[..size].reverse();
        }
        b
    };
    match type_ {
        TIFF_BYTE | TIFF_ASCII | TIFF_UNDEFINED => raw[n] as i128,
        TIFF_SBYTE => raw[n] as i8 as i128,
        TIFF_SHORT => {
            let b = get(2);
            u16::from_ne_bytes([b[0], b[1]]) as i128
        }
        TIFF_SSHORT => {
            let b = get(2);
            i16::from_ne_bytes([b[0], b[1]]) as i128
        }
        TIFF_LONG | TIFF_IFD => {
            let b = get(4);
            u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as i128
        }
        TIFF_SLONG => {
            let b = get(4);
            i32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as i128
        }
        TIFF_LONG8 | TIFF_IFD8 => u64::from_ne_bytes(get(8)) as i128,
        TIFF_SLONG8 => i64::from_ne_bytes(get(8)) as i128,
        _ => 0,
    }
}

/*
 * The array readers: TIFFReadDirEntryByteArray(), ...SbyteArray(),
 * ...ShortArray(), ...SshortArray(), ...LongArray(), ...SlongArray(),
 * ...Long8Array(WithLimit)(), ...Slong8Array() and ...Ifd8Array() read
 * arrays of integers (of the types each accepts), checking that every
 * value fits the destination type.
 */

/// Translation of the integer array readers: `accepted` are the types the
/// C function takes (anything else is `TIFFReadDirEntryErrType`).
fn tiff_read_dir_entry_int_array<T: TryFrom<i128>>(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
    accepted: &[TIFFDataType],
    maxcount: u64,
) -> EResult<Option<Vec<T>>> {
    if !accepted.contains(&direntry.tdir_type) {
        return Err(Err::Type);
    }
    let desttypesize = std::mem::size_of::<T>() as u32;
    let Some((origdata, count)) =
        tiff_read_dir_entry_array_with_limit(tif, direntry, desttypesize, maxcount)?
    else {
        return Ok(None);
    };
    let mut data = Vec::new();
    if data.try_reserve_exact(count as usize).is_err() {
        return Err(Err::Alloc);
    }
    for n in 0..count as usize {
        let m = raw_int(tif, direntry.tdir_type, &origdata, n);
        data.push(T::try_from(m).map_err(|_| Err::Range)?);
    }
    Ok(Some(data))
}

const INT_TYPES: [TIFFDataType; 8] = [
    TIFF_BYTE,
    TIFF_SBYTE,
    TIFF_SHORT,
    TIFF_SSHORT,
    TIFF_LONG,
    TIFF_SLONG,
    TIFF_LONG8,
    TIFF_SLONG8,
];

/// Translation of `TIFFReadDirEntryByteArray()`.
fn tiff_read_dir_entry_byte_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<u8>>> {
    const T: [TIFFDataType; 10] = [
        TIFF_ASCII,
        TIFF_UNDEFINED,
        TIFF_BYTE,
        TIFF_SBYTE,
        TIFF_SHORT,
        TIFF_SSHORT,
        TIFF_LONG,
        TIFF_SLONG,
        TIFF_LONG8,
        TIFF_SLONG8,
    ];
    tiff_read_dir_entry_int_array(tif, direntry, &T, !0)
}

/// Translation of `TIFFReadDirEntrySbyteArray()`.
fn tiff_read_dir_entry_sbyte_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<i8>>> {
    const T: [TIFFDataType; 9] = [
        TIFF_UNDEFINED,
        TIFF_BYTE,
        TIFF_SBYTE,
        TIFF_SHORT,
        TIFF_SSHORT,
        TIFF_LONG,
        TIFF_SLONG,
        TIFF_LONG8,
        TIFF_SLONG8,
    ];
    tiff_read_dir_entry_int_array(tif, direntry, &T, !0)
}

/// Translation of `TIFFReadDirEntryShortArray()`.
fn tiff_read_dir_entry_short_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<u16>>> {
    tiff_read_dir_entry_int_array(tif, direntry, &INT_TYPES, !0)
}

/// Translation of `TIFFReadDirEntrySshortArray()`.
fn tiff_read_dir_entry_sshort_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<i16>>> {
    tiff_read_dir_entry_int_array(tif, direntry, &INT_TYPES, !0)
}

const INT_IFD_TYPES: [TIFFDataType; 10] = [
    TIFF_BYTE,
    TIFF_SBYTE,
    TIFF_SHORT,
    TIFF_SSHORT,
    TIFF_LONG,
    TIFF_SLONG,
    TIFF_LONG8,
    TIFF_SLONG8,
    TIFF_IFD,
    TIFF_IFD8,
];

/// Translation of `TIFFReadDirEntryLongArray()`.
fn tiff_read_dir_entry_long_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<u32>>> {
    tiff_read_dir_entry_int_array(tif, direntry, &INT_IFD_TYPES, !0)
}

/// Translation of `TIFFReadDirEntrySlongArray()`.
fn tiff_read_dir_entry_slong_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<i32>>> {
    tiff_read_dir_entry_int_array(tif, direntry, &INT_TYPES, !0)
}

/// Translation of `TIFFReadDirEntryLong8ArrayWithLimit()`.
fn tiff_read_dir_entry_long8_array_with_limit(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
    maxcount: u64,
) -> EResult<Option<Vec<u64>>> {
    tiff_read_dir_entry_int_array(tif, direntry, &INT_IFD_TYPES, maxcount)
}

/// Translation of `TIFFReadDirEntryLong8Array()`.
fn tiff_read_dir_entry_long8_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<u64>>> {
    tiff_read_dir_entry_long8_array_with_limit(tif, direntry, !0u64)
}

/// Translation of `TIFFReadDirEntrySlong8Array()`.
fn tiff_read_dir_entry_slong8_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<i64>>> {
    tiff_read_dir_entry_int_array(tif, direntry, &INT_TYPES, !0)
}

/// Translation of `TIFFReadDirEntryIfd8Array()`.
fn tiff_read_dir_entry_ifd8_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<u64>>> {
    const T: [TIFFDataType; 4] = [TIFF_LONG, TIFF_LONG8, TIFF_IFD, TIFF_IFD8];
    tiff_read_dir_entry_int_array(tif, direntry, &T, !0)
}

const REAL_TYPES: [TIFFDataType; 12] = [
    TIFF_BYTE,
    TIFF_SBYTE,
    TIFF_SHORT,
    TIFF_SSHORT,
    TIFF_LONG,
    TIFF_SLONG,
    TIFF_LONG8,
    TIFF_SLONG8,
    TIFF_RATIONAL,
    TIFF_SRATIONAL,
    TIFF_FLOAT,
    TIFF_DOUBLE,
];

/// Element `n` of a raw array of 32-bit values, swabbed.
fn raw_u32(tif: &Tiff<'_>, raw: &[u8], n: usize) -> u32 {
    let mut b = [raw[4 * n], raw[4 * n + 1], raw[4 * n + 2], raw[4 * n + 3]];
    if (tif.tif_flags & TIFF_SWAB) != 0 {
        b.reverse();
    }
    u32::from_ne_bytes(b)
}

/// Element `n` of a raw array of 64-bit values, swabbed.
fn raw_u64(tif: &Tiff<'_>, raw: &[u8], n: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&raw[8 * n..8 * n + 8]);
    if (tif.tif_flags & TIFF_SWAB) != 0 {
        b.reverse();
    }
    u64::from_ne_bytes(b)
}

/// Translation of `TIFFReadDirEntryFloatArray()`.
fn tiff_read_dir_entry_float_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<f32>>> {
    if !REAL_TYPES.contains(&direntry.tdir_type) {
        return Err(Err::Type);
    }
    let Some((origdata, count)) = tiff_read_dir_entry_array(tif, direntry, 4)? else {
        return Ok(None);
    };
    let mut data = Vec::new();
    if data.try_reserve_exact(count as usize).is_err() {
        return Err(Err::Alloc);
    }
    for n in 0..count as usize {
        let v = match direntry.tdir_type {
            TIFF_FLOAT => f32::from_bits(raw_u32(tif, &origdata, n)),
            TIFF_RATIONAL => {
                let maa = raw_u32(tif, &origdata, 2 * n);
                let mab = raw_u32(tif, &origdata, 2 * n + 1);
                if mab == 0 {
                    0.0
                } else {
                    maa as f32 / mab as f32
                }
            }
            TIFF_SRATIONAL => {
                let maa = raw_u32(tif, &origdata, 2 * n) as i32;
                let mab = raw_u32(tif, &origdata, 2 * n + 1);
                if mab == 0 {
                    0.0
                } else {
                    maa as f32 / mab as f32
                }
            }
            TIFF_DOUBLE => {
                let mut val = f64::from_bits(raw_u64(tif, &origdata, n));
                if val > f32::MAX as f64 {
                    val = f32::MAX as f64;
                } else if val < -(f32::MAX as f64) {
                    val = -(f32::MAX as f64);
                }
                val as f32
            }
            t => {
                let m = raw_int(tif, t, &origdata, n);
                match t {
                    TIFF_LONG8 => m as u64 as f32,
                    TIFF_SLONG8 => m as i64 as f32,
                    _ => m as i64 as f32,
                }
            }
        };
        data.push(v);
    }
    Ok(Some(data))
}

/// Translation of `TIFFReadDirEntryDoubleArray()`.
fn tiff_read_dir_entry_double_array(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<Option<Vec<f64>>> {
    if !REAL_TYPES.contains(&direntry.tdir_type) {
        return Err(Err::Type);
    }
    let Some((origdata, count)) = tiff_read_dir_entry_array(tif, direntry, 8)? else {
        return Ok(None);
    };
    let mut data = Vec::new();
    if data.try_reserve_exact(count as usize).is_err() {
        return Err(Err::Alloc);
    }
    for n in 0..count as usize {
        let v = match direntry.tdir_type {
            TIFF_DOUBLE => f64::from_bits(raw_u64(tif, &origdata, n)),
            TIFF_RATIONAL => {
                let maa = raw_u32(tif, &origdata, 2 * n);
                let mab = raw_u32(tif, &origdata, 2 * n + 1);
                if mab == 0 {
                    0.0
                } else {
                    maa as f64 / mab as f64
                }
            }
            TIFF_SRATIONAL => {
                let maa = raw_u32(tif, &origdata, 2 * n) as i32;
                let mab = raw_u32(tif, &origdata, 2 * n + 1);
                if mab == 0 {
                    0.0
                } else {
                    maa as f64 / mab as f64
                }
            }
            TIFF_FLOAT => f32::from_bits(raw_u32(tif, &origdata, n)) as f64,
            t => {
                let m = raw_int(tif, t, &origdata, n);
                match t {
                    TIFF_LONG8 => m as u64 as f64,
                    _ => m as i64 as f64,
                }
            }
        };
        data.push(v);
    }
    Ok(Some(data))
}

/// Translation of `TIFFReadDirEntryPersampleShort()`.
fn tiff_read_dir_entry_persample_short(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<u16> {
    let spp = tif.tif_dir.td_samplesperpixel;
    if direntry.tdir_count != spp as u64 {
        let fip = tiff_field_with_tag(tif, direntry.tdir_tag as u32);
        let name = fip
            .as_ref()
            .map_or("unknown tagname", |f| f.field_name.as_ref());
        if direntry.tdir_count == 0 {
            return Err(Err::Count);
        } else if direntry.tdir_count < spp as u64 {
            tiff_warning_ext_r!(
                "TIFFReadDirEntryPersampleShort",
                "Tag {} entry count is {} , whereas it should be SamplesPerPixel={}. Assuming that missing entries are all at the value of the first one",
                name,
                direntry.tdir_count,
                spp
            );
        } else {
            tiff_warning_ext_r!(
                "TIFFReadDirEntryPersampleShort",
                "Tag {} entry count is {} , whereas it should be SamplesPerPixel={}. Ignoring extra entries",
                name,
                direntry.tdir_count,
                spp
            );
        }
    }
    let Some(m) = tiff_read_dir_entry_short_array(tif, direntry)? else {
        // (the C returns TIFFReadDirEntryErrOk without setting the value)
        return Err(Err::Ok);
    };
    let mut nb = tif.tif_dir.td_samplesperpixel as usize;
    if (direntry.tdir_count as usize) < nb {
        nb = direntry.tdir_count as usize;
    }
    let value = m[0];
    for &x in m.iter().take(nb).skip(1) {
        if x != value {
            return Err(Err::Psdif);
        }
    }
    Ok(value)
}

/// Translation of `TIFFReadDirEntryCheckedByte()`.
fn tiff_read_dir_entry_checked_byte(direntry: &TIFFDirEntry) -> u8 {
    direntry.tdir_offset[0]
}

/// Translation of `TIFFReadDirEntryCheckedSbyte()`.
fn tiff_read_dir_entry_checked_sbyte(direntry: &TIFFDirEntry) -> i8 {
    direntry.tdir_offset[0] as i8
}

/// Translation of `TIFFReadDirEntryCheckedShort()`.
fn tiff_read_dir_entry_checked_short(tif: &Tiff<'_>, direntry: &TIFFDirEntry) -> u16 {
    let mut value = direntry.toff_short();
    /* *value=*(uint16_t*)(&direntry->tdir_offset); */
    if (tif.tif_flags & TIFF_SWAB) != 0 {
        value = value.swap_bytes();
    }
    value
}

/// Translation of `TIFFReadDirEntryCheckedSshort()`.
fn tiff_read_dir_entry_checked_sshort(tif: &Tiff<'_>, direntry: &TIFFDirEntry) -> i16 {
    tiff_read_dir_entry_checked_short(tif, direntry) as i16
}

/// Translation of `TIFFReadDirEntryCheckedLong()`.
fn tiff_read_dir_entry_checked_long(tif: &Tiff<'_>, direntry: &TIFFDirEntry) -> u32 {
    let mut value = direntry.toff_long();
    if (tif.tif_flags & TIFF_SWAB) != 0 {
        value = value.swap_bytes();
    }
    value
}

/// Translation of `TIFFReadDirEntryCheckedSlong()`.
fn tiff_read_dir_entry_checked_slong(tif: &Tiff<'_>, direntry: &TIFFDirEntry) -> i32 {
    tiff_read_dir_entry_checked_long(tif, direntry) as i32
}

/// The 8 bytes of a value stored at the offset (classic TIFF) or in the
/// entry (BigTIFF), unswabbed: the common part of
/// `TIFFReadDirEntryCheckedLong8()` and its 8-byte siblings.
fn tiff_read_dir_entry_checked_8(tif: &mut Tiff<'_>, direntry: &TIFFDirEntry) -> EResult<[u8; 8]> {
    if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
        let mut offset = direntry.toff_long();
        if (tif.tif_flags & TIFF_SWAB) != 0 {
            offset = offset.swap_bytes();
        }
        let mut value = [0u8; 8];
        tiff_read_dir_entry_data(tif, offset as u64, &mut value)?;
        Ok(value)
    } else {
        Ok(direntry.tdir_offset)
    }
}

/// Translation of `TIFFReadDirEntryCheckedLong8()`.
fn tiff_read_dir_entry_checked_long8(tif: &mut Tiff<'_>, direntry: &TIFFDirEntry) -> EResult<u64> {
    let mut value = u64::from_ne_bytes(tiff_read_dir_entry_checked_8(tif, direntry)?);
    if (tif.tif_flags & TIFF_SWAB) != 0 {
        value = value.swap_bytes();
    }
    Ok(value)
}

/// Translation of `TIFFReadDirEntryCheckedSlong8()`.
fn tiff_read_dir_entry_checked_slong8(tif: &mut Tiff<'_>, direntry: &TIFFDirEntry) -> EResult<i64> {
    Ok(tiff_read_dir_entry_checked_long8(tif, direntry)? as i64)
}

/// The two 32-bit halves of a rational, swabbed.
fn tiff_read_dir_entry_checked_rational_parts(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<[u32; 2]> {
    let m = tiff_read_dir_entry_checked_8(tif, direntry)?;
    let mut i = [
        u32::from_ne_bytes([m[0], m[1], m[2], m[3]]),
        u32::from_ne_bytes([m[4], m[5], m[6], m[7]]),
    ];
    if (tif.tif_flags & TIFF_SWAB) != 0 {
        i[0] = i[0].swap_bytes();
        i[1] = i[1].swap_bytes();
    }
    Ok(i)
}

/// Translation of `TIFFReadDirEntryCheckedRational()`.
fn tiff_read_dir_entry_checked_rational(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<f64> {
    let m = tiff_read_dir_entry_checked_rational_parts(tif, direntry)?;
    /* Not completely sure what we should do when m.i[1]==0, but some */
    /* sanitizers do not like division by 0.0: */
    /* http://bugzilla.maptools.org/show_bug.cgi?id=2644 */
    if m[0] == 0 || m[1] == 0 {
        Ok(0.0)
    } else {
        Ok(m[0] as f64 / m[1] as f64)
    }
}

/// Translation of `TIFFReadDirEntryCheckedSrational()`.
fn tiff_read_dir_entry_checked_srational(
    tif: &mut Tiff<'_>,
    direntry: &TIFFDirEntry,
) -> EResult<f64> {
    let m = tiff_read_dir_entry_checked_rational_parts(tif, direntry)?;
    /* Not completely sure what we should do when m.i[1]==0, but some */
    /* sanitizers do not like division by 0.0: */
    /* http://bugzilla.maptools.org/show_bug.cgi?id=2644 */
    if (m[0] as i32) == 0 || m[1] == 0 {
        Ok(0.0)
    } else {
        Ok((m[0] as i32) as f64 / m[1] as f64)
    }
}

/// Translation of `TIFFReadDirEntryCheckedFloat()`.
fn tiff_read_dir_entry_checked_float(tif: &Tiff<'_>, direntry: &TIFFDirEntry) -> f32 {
    f32::from_bits(tiff_read_dir_entry_checked_long(tif, direntry))
}

/// Translation of `TIFFReadDirEntryCheckedDouble()`.
fn tiff_read_dir_entry_checked_double(tif: &mut Tiff<'_>, direntry: &TIFFDirEntry) -> EResult<f64> {
    Ok(f64::from_bits(tiff_read_dir_entry_checked_long8(
        tif, direntry,
    )?))
}

/// Translation of `TIFFReadDirEntryData()` (not mapped).
fn tiff_read_dir_entry_data(tif: &mut Tiff<'_>, offset: u64, dest: &mut [u8]) -> EResult<()> {
    if !tif.seek_ok(offset) {
        return Err(Err::Io);
    }
    if !tif.read_ok(dest) {
        return Err(Err::Io);
    }
    Ok(())
}

/// Translation of `TIFFReadDirEntryOutputErr()`.
fn tiff_read_dir_entry_output_err(
    err: TIFFReadDirEntryErr,
    module: &str,
    tagname: &str,
    recover: bool,
) {
    if !recover {
        match err {
            Err::Count => tiff_error_ext_r!(module, "Incorrect count for \"{}\"", tagname),
            Err::Type => tiff_error_ext_r!(module, "Incompatible type for \"{}\"", tagname),
            Err::Io => tiff_error_ext_r!(module, "IO error during reading of \"{}\"", tagname),
            Err::Range => tiff_error_ext_r!(module, "Incorrect value for \"{}\"", tagname),
            Err::Psdif => tiff_error_ext_r!(
                module,
                "Cannot handle different values per sample for \"{}\"",
                tagname
            ),
            Err::Sizesan => tiff_error_ext_r!(
                module,
                "Sanity check on size of \"{}\" value failed",
                tagname
            ),
            Err::Alloc => tiff_error_ext_r!(module, "Out of memory reading of \"{}\"", tagname),
            Err::Ok => { /* we should never get here */ }
        }
    } else {
        match err {
            Err::Count => {
                tiff_warning_ext_r!(module, "Incorrect count for \"{}\"; tag ignored", tagname)
            }
            Err::Type => {
                tiff_warning_ext_r!(module, "Incompatible type for \"{}\"; tag ignored", tagname)
            }
            Err::Io => tiff_warning_ext_r!(
                module,
                "IO error during reading of \"{}\"; tag ignored",
                tagname
            ),
            Err::Range => {
                tiff_warning_ext_r!(module, "Incorrect value for \"{}\"; tag ignored", tagname)
            }
            Err::Psdif => tiff_warning_ext_r!(
                module,
                "Cannot handle different values per sample for \"{}\"; tag ignored",
                tagname
            ),
            Err::Sizesan => tiff_warning_ext_r!(
                module,
                "Sanity check on size of \"{}\" value failed; tag ignored",
                tagname
            ),
            Err::Alloc => tiff_warning_ext_r!(
                module,
                "Out of memory reading of \"{}\"; tag ignored",
                tagname
            ),
            Err::Ok => { /* we should never get here */ }
        }
    }
}

/// Translation of `_TIFFGetMaxColorChannels()`: Return the maximum number
/// of color channels specified for a given photometric type. 0 is
/// returned if photometric type isn't supported or no default value is
/// defined by the specification.
fn _tiff_get_max_color_channels(photometric: u16) -> i32 {
    match photometric {
        PHOTOMETRIC_PALETTE | PHOTOMETRIC_MINISWHITE | PHOTOMETRIC_MINISBLACK => 1,
        PHOTOMETRIC_YCBCR | PHOTOMETRIC_RGB | PHOTOMETRIC_CIELAB | PHOTOMETRIC_LOGLUV
        | PHOTOMETRIC_ITULAB | PHOTOMETRIC_ICCLAB => 3,
        PHOTOMETRIC_SEPARATED | PHOTOMETRIC_MASK => 4,
        _ => 0, /* PHOTOMETRIC_LOGL, PHOTOMETRIC_CFA */
    }
}

/// Translation of `ByteCountLooksBad()`.
fn byte_count_looks_bad(tif: &mut Tiff<'_>) -> i32 {
    /*
     * Assume we have wrong StripByteCount value (in case
     * of single strip) in following cases:
     *   - it is equal to zero along with StripOffset;
     *   - it is larger than file itself (in case of uncompressed
     *     image);
     *   - it is smaller than the size of the bytes per row
     *     multiplied on the number of rows.  The last case should
     *     not be checked in the case of writing new image,
     *     because we may do not know the exact strip size
     *     until the whole image will be written and directory
     *     dumped out.
     */
    let bytecount = tiff_get_strile_byte_count(tif, 0);
    let offset = tiff_get_strile_offset(tif, 0);

    if offset == 0 {
        return 0;
    }
    if bytecount == 0 {
        return 1;
    }
    if tif.tif_dir.td_compression != COMPRESSION_NONE {
        return 0;
    }
    let filesize = tif.get_file_size();
    if offset <= filesize && bytecount > filesize - offset {
        return 1;
    }
    if tif.tif_mode == O_RDONLY {
        let scanlinesize = tiff_scanline_size64(tif);
        let imagelength = tif.tif_dir.td_imagelength as u64;
        if imagelength > 0 && scanlinesize > u64::MAX / imagelength {
            return 1;
        }
        if bytecount < scanlinesize * imagelength {
            return 1;
        }
    }
    0
}

/// Translation of `EvaluateIFDdatasizeReading()`: To evaluate the IFD
/// data size when reading, save the offset and data size of all data that
/// does not fit into the IFD entries themselves.
fn evaluate_ifd_datasize_reading(tif: &mut Tiff<'_>, dp: &TIFFDirEntry) -> bool {
    let data_width = tiff_data_width(dp.tdir_type) as u64;
    if data_width != 0 && dp.tdir_count > u64::MAX / data_width {
        tiff_error_ext_r!("EvaluateIFDdatasizeReading", "Too large IFD data size");
        return false;
    }
    let datalength = dp.tdir_count * data_width;
    if datalength
        > (if (tif.tif_flags & TIFF_BIGTIFF) != 0 {
            0x8
        } else {
            0x4
        })
    {
        if tif.tif_dir.td_dirdatasize_read > u64::MAX - datalength {
            tiff_error_ext_r!("EvaluateIFDdatasizeReading", "Too large IFD data size");
            return false;
        }
        tif.tif_dir.td_dirdatasize_read += datalength;
        let offset = if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
            /* The offset of TIFFDirEntry are not swapped when read in. That has
             * to be done when used. */
            let mut offset = dp.toff_long();
            if (tif.tif_flags & TIFF_SWAB) != 0 {
                offset = offset.swap_bytes();
            }
            offset as u64
        } else {
            let mut offset = dp.toff_long8();
            if (tif.tif_flags & TIFF_SWAB) != 0 {
                offset = offset.swap_bytes();
            }
            offset
        };
        if let Some(offsets) = tif.tif_dir.td_dirdatasize_offsets.as_mut() {
            offsets.push(TIFFEntryOffsetAndLength {
                offset,
                length: datalength,
            });
        }
        tif.tif_dir.td_dirdatasize_Noffsets += 1;
    }
    true
}

/// Translation of `CalcFinalIFDdatasizeReading()`: Determine the IFD data
/// size after reading an IFD from the file that can be overwritten and
/// saving it in tif_dir.td_dirdatasize_read. (Only needed if
/// file-writing is enabled, which it never is here.)
fn calc_final_ifd_datasize_reading(tif: &mut Tiff<'_>, _dircount: u16) {
    /* IFD data size is only needed if file-writing is enabled.
     * This also avoids the seek() to EOF to determine the file size, which
     * causes the stdin-streaming-friendly mode of libtiff for GDAL to fail. */
    if tif.tif_mode == O_RDONLY {
        // (always)
    }
}

/// How `TIFFReadDirectory()` goes on after a step: on, or to `bad:`.
type Step = Result<(), ()>;

/// Translation of `TIFFReadDirectory()`: Read the next TIFF directory
/// from a file and convert it to the internal format. We read
/// directories sequentially.
pub(crate) fn tiff_read_directory(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "TIFFReadDirectory";

    if tif.tif_nextdiroff == 0 {
        /* In this special case, tif_diroff needs also to be set to 0.
         * This is behind the last IFD, thus no checking or reading necessary.
         */
        tif.tif_diroff = tif.tif_nextdiroff;
        return 0;
    }

    let nextdiroff = tif.tif_nextdiroff;
    /* tif_curdir++ and tif_nextdiroff should only be updated after SUCCESSFUL
     * reading of the directory. Otherwise, invalid IFD offsets could corrupt
     * the IFD list. */
    let dirn = if tif.tif_curdir == TIFF_NON_EXISTENT_DIR_NUMBER {
        0
    } else {
        tif.tif_curdir.wrapping_add(1)
    };
    if _tiff_check_dir_number_and_offset(tif, dirn, nextdiroff) == 0 {
        return 0; /* bad offset (IFD looping or more than TIFF_MAX_DIR_COUNT
                  IFDs) */
    }
    let mut new_nextdiroff = 0u64;
    let Some(mut dir) = tiff_fetch_directory(tif, nextdiroff, Some(&mut new_nextdiroff)) else {
        tif.tif_nextdiroff = new_nextdiroff;
        tiff_error_ext_r!(MODULE, "Failed to read directory at offset {}", nextdiroff);
        return 0;
    };
    tif.tif_nextdiroff = new_nextdiroff;
    let dircount = dir.len() as u16;
    /* Set global values after a valid directory has been fetched.
     * tif_diroff is already set to nextdiroff in TIFFFetchDirectory() in the
     * beginning. */
    if tif.tif_curdir == TIFF_NON_EXISTENT_DIR_NUMBER {
        tif.tif_curdir = 0;
    } else {
        tif.tif_curdir = tif.tif_curdir.wrapping_add(1);
    }

    /* bad: 0 */
    tiff_read_directory_body(tif, &mut dir, dircount).unwrap_or_default()
}

/// The part of `TIFFReadDirectory()` after the directory is fetched; `Err`
/// is `goto bad`.
fn tiff_read_directory_body(
    tif: &mut Tiff<'_>,
    dir: &mut [TIFFDirEntry],
    dircount: u16,
) -> Result<i32, ()> {
    const MODULE: &str = "TIFFReadDirectory";
    let mut bitspersample_read = false;
    let mut fii: u32 = FAILED_FII;
    let bad = |ok: bool| -> Step {
        if ok {
            Ok(())
        } else {
            Err(())
        }
    };

    tiff_read_directory_check_order(dir);

    /*
     * Mark duplicates of any tag to be ignored (bugzilla 1994)
     * to avoid certain pathological problems.
     */
    for mb in 0..dir.len() {
        for nb in mb + 1..dir.len() {
            if dir[mb].tdir_tag == dir[nb].tdir_tag {
                dir[nb].tdir_ignore = 1;
            }
        }
    }

    tif.tif_flags &= !TIFF_BEENWRITING; /* reset before new dir */
    tif.tif_flags &= !TIFF_BUF4WRITE; /* reset before new dir */
    tif.tif_flags &= !TIFF_CHOPPEDUPARRAYS;

    /* When changing directory, in deferred strile loading mode, we must also
     * unset the TIFF_LAZYSTRILELOAD_DONE bit if it was initially set,
     * to make sure the strile offset/bytecount are read again (when they fit
     * in the tag data area).
     */
    tif.tif_flags &= !TIFF_LAZYSTRILELOAD_DONE;

    /* Free any old stuff and reinit i/o and other parameters within
     * TIFFDefaultDirectory() since we are starting on a new directory. */
    tiff_free_directory(tif);
    tiff_default_directory(tif);

    /* After setup a fresh directory indicate that now active IFD is also
     * present on file, even if its entries could not be read successfully
     * below.  */
    tif.tif_dir.td_iswrittentofile = 1;

    /* Allocate arrays for offset values outside IFD entry for IFD data size
     * checking. Note: Counter are reset within TIFFFreeDirectory(). */
    let mut offsets = Vec::new();
    if offsets.try_reserve_exact(dircount as usize).is_err() {
        tiff_error_ext_r!(
            MODULE,
            "Failed to allocate memory for counting IFD data size at reading"
        );
        return Err(());
    }
    tif.tif_dir.td_dirdatasize_offsets = Some(offsets);
    /*
     * Electronic Arts writes gray-scale TIFF files
     * without a PlanarConfiguration directory entry.
     * Thus we setup a default value here, even though
     * the TIFF spec says there is no default value.
     * After PlanarConfiguration is preset in TIFFDefaultDirectory()
     * the following setting is not needed, but does not harm either.
     */
    tiff_set_field(
        tif,
        TIFFTAG_PLANARCONFIG,
        &[Va::Int(PLANARCONFIG_CONTIG as i64)],
    );
    /*
     * Setup default value and then make a pass over
     * the fields to check type and tag information,
     * and to extract info required to size data
     * structures.  A second pass is made afterwards
     * to read in everything not taken in the first pass.
     * But we must process the Compression tag first
     * in order to merge in codec-private tag definitions (otherwise
     * we may get complaints about unknown tags).  However, the
     * Compression tag may be dependent on the SamplesPerPixel
     * tag value because older TIFF specs permitted Compression
     * to be written as a SamplesPerPixel-count tag entry.
     * Thus if we don't first figure out the correct SamplesPerPixel
     * tag value then we may end up ignoring the Compression tag
     * value because it has an incorrect count value (if the
     * true value of SamplesPerPixel is not 1).
     */
    if let Some(i) = tiff_read_directory_find_entry(dir, TIFFTAG_SAMPLESPERPIXEL as u16) {
        bad(tiff_fetch_normal_tag(tif, &mut dir[i], false) != 0)?;
        dir[i].tdir_ignore = 1;
    }
    if let Some(i) = tiff_read_directory_find_entry(dir, TIFFTAG_COMPRESSION as u16) {
        /*
         * The 5.0 spec says the Compression tag has one value, while
         * earlier specs say it has one value per sample.  Because of
         * this, we accept the tag if one value is supplied with either
         * count.
         */
        let dp = dir[i];
        let mut r = tiff_read_dir_entry_short(tif, &dp);
        if r == Err(Err::Count) {
            r = tiff_read_dir_entry_persample_short(tif, &dp);
        }
        let value = match r {
            Ok(v) => v,
            Err(err) => {
                // (Err::Ok: the C goes on with the value uninitialized)
                if err != Err::Ok {
                    tiff_read_dir_entry_output_err(err, MODULE, "Compression", false);
                    return Err(());
                }
                0
            }
        };
        bad(tiff_set_field(tif, TIFFTAG_COMPRESSION, &[Va::Int(value as i64)]) != 0)?;
        dir[i].tdir_ignore = 1;
    } else {
        bad(tiff_set_field(
            tif,
            TIFFTAG_COMPRESSION,
            &[Va::Int(COMPRESSION_NONE as i64)],
        ) != 0)?;
    }
    /*
     * First real pass over the directory.
     */
    for di in 0..dir.len() {
        if dir[di].tdir_ignore == 0 {
            tiff_read_directory_find_field_info(tif, dir[di].tdir_tag, &mut fii);
            if fii == FAILED_FII {
                let tag = dir[di].tdir_tag;
                if tif.tif_warn_about_unknown_tags != 0 {
                    tiff_warning_ext_r!(
                        MODULE,
                        "Unknown field with tag {} (0x{:x}) encountered",
                        tag,
                        tag
                    );
                }
                /* the following knowingly leaks the
                anonymous field structure */
                let fld = _tiff_create_anon_field(tag as u32, dir[di].tdir_type);
                if _tiff_merge_fields(tif, std::slice::from_ref(&fld)) == 0 {
                    tiff_warning_ext_r!(
                        MODULE,
                        "Registering anonymous field with tag {} (0x{:x}) failed",
                        tag,
                        tag
                    );
                    dir[di].tdir_ignore = 1;
                } else {
                    tiff_read_directory_find_field_info(tif, tag, &mut fii);
                }
            }
        }
        if dir[di].tdir_ignore == 0 && fii != FAILED_FII {
            let fip = tif.tif_fields[fii as usize].clone();
            if fip.field_bit == FIELD_IGNORE {
                dir[di].tdir_ignore = 1;
            } else {
                match dir[di].tdir_tag as u32 {
                    TIFFTAG_STRIPOFFSETS
                    | TIFFTAG_STRIPBYTECOUNTS
                    | TIFFTAG_TILEOFFSETS
                    | TIFFTAG_TILEBYTECOUNTS => {
                        tiff_set_field_bit(tif, fip.field_bit);
                    }
                    TIFFTAG_IMAGEWIDTH | TIFFTAG_IMAGELENGTH | TIFFTAG_IMAGEDEPTH
                    | TIFFTAG_TILELENGTH | TIFFTAG_TILEWIDTH | TIFFTAG_TILEDEPTH
                    | TIFFTAG_PLANARCONFIG | TIFFTAG_ROWSPERSTRIP | TIFFTAG_EXTRASAMPLES => {
                        bad(tiff_fetch_normal_tag(tif, &mut dir[di], false) != 0)?;
                        dir[di].tdir_ignore = 1;
                    }
                    _ => {
                        if _tiff_check_field_is_valid_for_codec(tif, dir[di].tdir_tag as u32) == 0 {
                            dir[di].tdir_ignore = 1;
                        }
                    }
                }
            }
        }
    }
    /*
     * XXX: OJPEG hack.
     * If a) compression is OJPEG, b) planarconfig tag says it's separate,
     * c) strip offsets/bytecounts tag are both present and
     * d) both contain exactly one value, then we consistently find
     * that the buggy implementation of the buggy compression scheme
     * matches contig planarconfig best. So we 'fix-up' the tag here
     */
    if (tif.tif_dir.td_compression == COMPRESSION_OJPEG)
        && (tif.tif_dir.td_planarconfig == PLANARCONFIG_SEPARATE)
    {
        bad(_tiff_fill_striles(tif) != 0)?;
        if let Some(i) = tiff_read_directory_find_entry(dir, TIFFTAG_STRIPOFFSETS as u16) {
            if dir[i].tdir_count == 1 {
                if let Some(j) = tiff_read_directory_find_entry(dir, TIFFTAG_STRIPBYTECOUNTS as u16)
                {
                    if dir[j].tdir_count == 1 {
                        tif.tif_dir.td_planarconfig = PLANARCONFIG_CONTIG;
                        tiff_warning_ext_r!(
                            MODULE,
                            "Planarconfig tag value assumed incorrect, assuming data is contig instead of chunky"
                        );
                    }
                }
            }
        }
    }
    /*
     * Allocate directory structure and setup defaults.
     */
    if !tiff_field_set(tif, FIELD_IMAGEDIMENSIONS) {
        missing_required("ImageLength");
        return Err(());
    }

    /*
     * Second pass: extract other information.
     */
    for di in 0..dir.len() {
        if dir[di].tdir_ignore != 0 {
            continue;
        }
        let dp = dir[di];
        let tagname = |tif: &Tiff<'_>| {
            tiff_field_with_tag(tif, dp.tdir_tag as u32)
                .map_or(String::from("unknown tagname"), |f| {
                    f.field_name.into_owned()
                })
        };
        match dp.tdir_tag as u32 {
            TIFFTAG_MINSAMPLEVALUE
            | TIFFTAG_MAXSAMPLEVALUE
            | TIFFTAG_BITSPERSAMPLE
            | TIFFTAG_DATATYPE
            | TIFFTAG_SAMPLEFORMAT => {
                /*
                 * The MinSampleValue, MaxSampleValue, BitsPerSample
                 * DataType and SampleFormat tags are supposed to be
                 * written as one value/sample, but some vendors
                 * incorrectly write one value only -- so we accept
                 * that as well (yuck). Other vendors write correct
                 * value for NumberOfSamples, but incorrect one for
                 * BitsPerSample and friends, and we will read this
                 * too.
                 */
                let mut r = tiff_read_dir_entry_short(tif, &dp);
                bad(evaluate_ifd_datasize_reading(tif, &dp))?;
                if r == Err(Err::Count) {
                    r = tiff_read_dir_entry_persample_short(tif, &dp);
                }
                let value = match r {
                    Ok(v) => v,
                    Err(err) => {
                        if err != Err::Ok {
                            tiff_read_dir_entry_output_err(err, MODULE, &tagname(tif), false);
                            return Err(());
                        }
                        // (the C goes on with the value uninitialized)
                        0
                    }
                };
                bad(tiff_set_field(tif, dp.tdir_tag as u32, &[Va::Int(value as i64)]) != 0)?;
                if dp.tdir_tag as u32 == TIFFTAG_BITSPERSAMPLE {
                    bitspersample_read = true;
                }
            }
            TIFFTAG_SMINSAMPLEVALUE | TIFFTAG_SMAXSAMPLEVALUE => {
                let r = if dp.tdir_count != tif.tif_dir.td_samplesperpixel as u64 {
                    Err(Err::Count)
                } else {
                    tiff_read_dir_entry_double_array(tif, &dp)
                };
                bad(evaluate_ifd_datasize_reading(tif, &dp))?;
                let data = match r {
                    Ok(d) => d,
                    Err(err) => {
                        tiff_read_dir_entry_output_err(err, MODULE, &tagname(tif), false);
                        return Err(());
                    }
                };
                let saved_flags = tif.tif_flags;
                tif.tif_flags |= TIFF_PERSAMPLE;
                let arg = match &data {
                    Some(d) => Va::F64s(d),
                    None => Va::Null,
                };
                let m = tiff_set_field(tif, dp.tdir_tag as u32, &[arg]);
                tif.tif_flags = saved_flags;
                bad(m != 0)?;
            }
            TIFFTAG_STRIPOFFSETS | TIFFTAG_TILEOFFSETS => {
                match dp.tdir_type {
                    TIFF_SHORT | TIFF_LONG | TIFF_LONG8 => {}
                    _ => {
                        /* Warn except if directory typically created with
                         * TIFFDeferStrileArrayWriting() */
                        if !(tif.tif_mode == O_RDWR
                            && dp.tdir_count == 0
                            && dp.tdir_type == 0
                            && dp.toff_long8() == 0)
                        {
                            tiff_warning_ext_r!(
                                MODULE,
                                "Invalid data type for tag {}",
                                tagname(tif)
                            );
                        }
                    }
                }
                tif.tif_dir.td_stripoffset_entry = dp;
                bad(evaluate_ifd_datasize_reading(tif, &dp))?;
            }
            TIFFTAG_STRIPBYTECOUNTS | TIFFTAG_TILEBYTECOUNTS => {
                match dp.tdir_type {
                    TIFF_SHORT | TIFF_LONG | TIFF_LONG8 => {}
                    _ => {
                        /* Warn except if directory typically created with
                         * TIFFDeferStrileArrayWriting() */
                        if !(tif.tif_mode == O_RDWR
                            && dp.tdir_count == 0
                            && dp.tdir_type == 0
                            && dp.toff_long8() == 0)
                        {
                            tiff_warning_ext_r!(
                                MODULE,
                                "Invalid data type for tag {}",
                                tagname(tif)
                            );
                        }
                    }
                }
                tif.tif_dir.td_stripbytecount_entry = dp;
                bad(evaluate_ifd_datasize_reading(tif, &dp))?;
            }
            TIFFTAG_COLORMAP | TIFFTAG_TRANSFERFUNCTION => {
                /* It would be dangerous to instantiate those tag values */
                /* since if td_bitspersample has not yet been read (due to
                 */
                /* unordered tags), it could be read afterwards with a */
                /* values greater than the default one (1), which may cause
                 */
                /* crashes in user code */
                if !bitspersample_read {
                    tiff_warning_ext_r!(
                        MODULE,
                        "Ignoring {} since BitsPerSample tag not found",
                        tagname(tif)
                    );
                    continue;
                }
                /* ColorMap or TransferFunction for high bit */
                /* depths do not make much sense and could be */
                /* used as a denial of service vector */
                if tif.tif_dir.td_bitspersample > 24 {
                    tiff_warning_ext_r!(
                        MODULE,
                        "Ignoring {} because BitsPerSample={}>24",
                        tagname(tif),
                        tif.tif_dir.td_bitspersample
                    );
                    continue;
                }
                let countpersample: u32 = 1u32 << tif.tif_dir.td_bitspersample;
                let countrequired: u32;
                let incrementpersample: u32;
                if (dp.tdir_tag as u32 == TIFFTAG_TRANSFERFUNCTION)
                    && (dp.tdir_count == countpersample as u64)
                {
                    countrequired = countpersample;
                    incrementpersample = 0;
                } else {
                    countrequired = 3 * countpersample;
                    incrementpersample = countpersample;
                }
                let r = if dp.tdir_count != countrequired as u64 {
                    Err(Err::Count)
                } else {
                    tiff_read_dir_entry_short_array(tif, &dp)
                };
                bad(evaluate_ifd_datasize_reading(tif, &dp))?;
                match r {
                    Err(err) => {
                        tiff_read_dir_entry_output_err(err, MODULE, &tagname(tif), true);
                    }
                    Ok(value) => {
                        let value = value.unwrap_or_default();
                        let at = |k: u32| -> Va<'_> {
                            match value.get((k * incrementpersample) as usize..) {
                                Some(s) if !value.is_empty() => Va::U16s(s),
                                _ => Va::Null,
                            }
                        };
                        tiff_set_field(tif, dp.tdir_tag as u32, &[at(0), at(1), at(2)]);
                    }
                }
            }
            /* BEGIN REV 4.0 COMPATIBILITY */
            TIFFTAG_OSUBFILETYPE => {
                if let Ok(valueo) = tiff_read_dir_entry_short(tif, &dp) {
                    let value: u32 = match valueo {
                        OFILETYPE_REDUCEDIMAGE => FILETYPE_REDUCEDIMAGE,
                        OFILETYPE_PAGE => FILETYPE_PAGE,
                        _ => 0,
                    };
                    if value != 0 {
                        tiff_set_field(tif, TIFFTAG_SUBFILETYPE, &[Va::Int(value as i64)]);
                    }
                }
            }
            /* END REV 4.0 COMPATIBILITY */
            _ => {
                let _ = tiff_fetch_normal_tag(tif, &mut dir[di], true);
            }
        } /* -- switch (dp->tdir_tag) -- */
    } /* -- for-loop -- */

    /* Evaluate final IFD data size. */
    calc_final_ifd_datasize_reading(tif, dircount);

    /*
     * OJPEG hack:
     * - If a) compression is OJPEG, and b) photometric tag is missing,
     * then we consistently find that photometric should be YCbCr
     * - If a) compression is OJPEG, and b) photometric tag says it's RGB,
     * then we consistently find that the buggy implementation of the
     * buggy compression scheme matches photometric YCbCr instead.
     * - If a) compression is OJPEG, and b) bitspersample tag is missing,
     * then we consistently find bitspersample should be 8.
     * - If a) compression is OJPEG, b) samplesperpixel tag is missing,
     * and c) photometric is RGB or YCbCr, then we consistently find
     * samplesperpixel should be 3
     * - If a) compression is OJPEG, b) samplesperpixel tag is missing,
     * and c) photometric is MINISWHITE or MINISBLACK, then we consistently
     * find samplesperpixel should be 3
     */
    if tif.tif_dir.td_compression == COMPRESSION_OJPEG {
        if !tiff_field_set(tif, FIELD_PHOTOMETRIC) {
            tiff_warning_ext_r!(MODULE, "Photometric tag is missing, assuming data is YCbCr");
            bad(tiff_set_field(
                tif,
                TIFFTAG_PHOTOMETRIC,
                &[Va::Int(PHOTOMETRIC_YCBCR as i64)],
            ) != 0)?;
        } else if tif.tif_dir.td_photometric == PHOTOMETRIC_RGB {
            tif.tif_dir.td_photometric = PHOTOMETRIC_YCBCR;
            tiff_warning_ext_r!(
                MODULE,
                "Photometric tag value assumed incorrect, assuming data is YCbCr instead of RGB"
            );
        }
        if !tiff_field_set(tif, FIELD_BITSPERSAMPLE) {
            tiff_warning_ext_r!(
                MODULE,
                "BitsPerSample tag is missing, assuming 8 bits per sample"
            );
            bad(tiff_set_field(tif, TIFFTAG_BITSPERSAMPLE, &[Va::Int(8)]) != 0)?;
        }
        if !tiff_field_set(tif, FIELD_SAMPLESPERPIXEL) {
            if tif.tif_dir.td_photometric == PHOTOMETRIC_RGB {
                tiff_warning_ext_r!(
                    MODULE,
                    "SamplesPerPixel tag is missing, assuming correct SamplesPerPixel value is 3"
                );
                bad(tiff_set_field(tif, TIFFTAG_SAMPLESPERPIXEL, &[Va::Int(3)]) != 0)?;
            }
            if tif.tif_dir.td_photometric == PHOTOMETRIC_YCBCR {
                tiff_warning_ext_r!(
                    MODULE,
                    "SamplesPerPixel tag is missing, applying correct SamplesPerPixel value of 3"
                );
                bad(tiff_set_field(tif, TIFFTAG_SAMPLESPERPIXEL, &[Va::Int(3)]) != 0)?;
            } else if (tif.tif_dir.td_photometric == PHOTOMETRIC_MINISWHITE)
                || (tif.tif_dir.td_photometric == PHOTOMETRIC_MINISBLACK)
            {
                /*
                 * SamplesPerPixel tag is missing, but is not required
                 * by spec.  Assume correct SamplesPerPixel value of 1.
                 */
                bad(tiff_set_field(tif, TIFFTAG_SAMPLESPERPIXEL, &[Va::Int(1)]) != 0)?;
            }
        }
    }

    /*
     * Setup appropriate structures (by strip or by tile)
     * We do that only after the above OJPEG hack which alters SamplesPerPixel
     * and thus influences the number of strips in the separate planarconfig.
     */
    if !tiff_field_set(tif, FIELD_TILEDIMENSIONS) {
        tif.tif_dir.td_nstrips = tiff_number_of_strips(tif);
        tif.tif_dir.td_tilewidth = tif.tif_dir.td_imagewidth;
        tif.tif_dir.td_tilelength = tif.tif_dir.td_rowsperstrip;
        tif.tif_dir.td_tiledepth = tif.tif_dir.td_imagedepth;
        tif.tif_flags &= !TIFF_ISTILED;
    } else {
        tif.tif_dir.td_nstrips = tiff_number_of_tiles(tif);
        tif.tif_flags |= TIFF_ISTILED;
    }
    if tif.tif_dir.td_nstrips == 0 {
        tiff_error_ext_r!(
            MODULE,
            "Cannot handle zero number of {}",
            if tif.is_tiled() { "tiles" } else { "strips" }
        );
        return Err(());
    }
    tif.tif_dir.td_stripsperimage = tif.tif_dir.td_nstrips;
    if tif.tif_dir.td_planarconfig == PLANARCONFIG_SEPARATE {
        // (td_samplesperpixel is never 0: TIFFSetField() refuses it)
        tif.tif_dir.td_stripsperimage /= tif.tif_dir.td_samplesperpixel.max(1) as u32;
    }
    if !tiff_field_set(tif, FIELD_STRIPOFFSETS) {
        // (no OJPEG_SUPPORT)
        missing_required(if tif.is_tiled() {
            "TileOffsets"
        } else {
            "StripOffsets"
        });
        return Err(());
    }

    let soe = tif.tif_dir.td_stripoffset_entry;
    let sbe = tif.tif_dir.td_stripbytecount_entry;
    if tif.tif_mode == O_RDWR
        && soe.tdir_tag != 0
        && soe.tdir_count == 0
        && soe.tdir_type == 0
        && soe.toff_long8() == 0
        && sbe.tdir_tag != 0
        && sbe.tdir_count == 0
        && sbe.tdir_type == 0
        && sbe.toff_long8() == 0
    {
        /* Directory typically created with TIFFDeferStrileArrayWriting() */
        // (TIFFSetupStrips(): only when writing)
    } else if (tif.tif_flags & TIFF_DEFERSTRILELOAD) == 0 {
        if soe.tdir_tag != 0 {
            let nstrips = tif.tif_dir.td_nstrips;
            match tiff_fetch_strip_thing(tif, &soe, nstrips) {
                Some(d) => tif.tif_dir.td_stripoffset_p = d,
                None => return Err(()),
            }
        }
        if sbe.tdir_tag != 0 {
            let nstrips = tif.tif_dir.td_nstrips;
            match tiff_fetch_strip_thing(tif, &sbe, nstrips) {
                Some(d) => tif.tif_dir.td_stripbytecount_p = d,
                None => return Err(()),
            }
        }
    }

    /*
     * Make sure all non-color channels are extrasamples.
     * If it's not the case, define them as such.
     */
    let color_channels = _tiff_get_max_color_channels(tif.tif_dir.td_photometric);
    if color_channels != 0
        && tif.tif_dir.td_samplesperpixel as i32 - tif.tif_dir.td_extrasamples as i32
            > color_channels
    {
        tiff_warning_ext_r!(
            MODULE,
            "Sum of Photometric type-related color channels and ExtraSamples doesn't match SamplesPerPixel. Defining non-color channels as ExtraSamples."
        );

        let old_extrasamples = tif.tif_dir.td_extrasamples;
        tif.tif_dir.td_extrasamples =
            (tif.tif_dir.td_samplesperpixel as i32 - color_channels) as u16;

        // sampleinfo should contain information relative to these new extra
        // samples
        let Some(mut new_sampleinfo) = try_vec::<u16>(tif.tif_dir.td_extrasamples as usize) else {
            tiff_error_ext_r!(
                MODULE,
                "Failed to allocate memory for temporary new sampleinfo array ({} 16 bit elements)",
                tif.tif_dir.td_extrasamples
            );
            return Err(());
        };

        if old_extrasamples > 0 {
            if let Some(old) = tif.tif_dir.td_sampleinfo.as_ref() {
                let n = (old_extrasamples as usize)
                    .min(old.len())
                    .min(new_sampleinfo.len());
                new_sampleinfo[..n].copy_from_slice(&old[..n]);
            }
        }
        tif.tif_dir.td_sampleinfo = if new_sampleinfo.is_empty() {
            None
        } else {
            Some(new_sampleinfo)
        };
    }

    /*
     * Verify Palette image has a Colormap.
     */
    if tif.tif_dir.td_photometric == PHOTOMETRIC_PALETTE && !tiff_field_set(tif, FIELD_COLORMAP) {
        if tif.tif_dir.td_bitspersample >= 8 && tif.tif_dir.td_samplesperpixel == 3 {
            tif.tif_dir.td_photometric = PHOTOMETRIC_RGB;
        } else if tif.tif_dir.td_bitspersample >= 8 {
            tif.tif_dir.td_photometric = PHOTOMETRIC_MINISBLACK;
        } else {
            missing_required("Colormap");
            return Err(());
        }
    }
    /*
     * OJPEG hack:
     * We do no further messing with strip/tile offsets/bytecounts in OJPEG
     * TIFFs
     */
    if tif.tif_dir.td_compression != COMPRESSION_OJPEG {
        /*
         * Attempt to deal with a missing StripByteCounts tag.
         */
        if !tiff_field_set(tif, FIELD_STRIPBYTECOUNTS) {
            /*
             * Some manufacturers violate the spec by not giving
             * the size of the strips.  In this case, assume there
             * is one uncompressed strip of data.
             */
            let td = &tif.tif_dir;
            if (td.td_planarconfig == PLANARCONFIG_CONTIG && td.td_nstrips > 1)
                || (td.td_planarconfig == PLANARCONFIG_SEPARATE
                    && td.td_nstrips != td.td_samplesperpixel as u32)
            {
                missing_required("StripByteCounts");
                return Err(());
            }
            tiff_warning_ext_r!(
                MODULE,
                "TIFF directory is missing required \"StripByteCounts\" field, calculating from imagelength"
            );
            bad(estimate_strip_byte_counts(tif, dir) >= 0)?;
        } else if tif.tif_dir.td_nstrips == 1
            && (tif.tif_flags & TIFF_ISTILED) == 0
            && byte_count_looks_bad(tif) != 0
        {
            /*
             * XXX: Plexus (and others) sometimes give a value of
             * zero for a tag when they don't know what the
             * correct value is!  Try and handle the simple case
             * of estimating the size of a one strip image.
             */
            tiff_warning_ext_r!(
                MODULE,
                "Bogus \"StripByteCounts\" field, ignoring and calculating from imagelength"
            );
            bad(estimate_strip_byte_counts(tif, dir) >= 0)?;
        } else if (tif.tif_flags & TIFF_DEFERSTRILELOAD) == 0
            && tif.tif_dir.td_planarconfig == PLANARCONFIG_CONTIG
            && tif.tif_dir.td_nstrips > 2
            && tif.tif_dir.td_compression == COMPRESSION_NONE
            && tiff_get_strile_byte_count(tif, 0) != tiff_get_strile_byte_count(tif, 1)
            && tiff_get_strile_byte_count(tif, 0) != 0
            && tiff_get_strile_byte_count(tif, 1) != 0
        {
            /*
             * XXX: Some vendors fill StripByteCount array with
             * absolutely wrong values (it can be equal to
             * StripOffset array, for example). Catch this case
             * here.
             *
             * We avoid this check if deferring strile loading
             * as it would always force us to load the strip/tile
             * information.
             */
            tiff_warning_ext_r!(
                MODULE,
                "Wrong \"StripByteCounts\" field, ignoring and calculating from imagelength"
            );
            bad(estimate_strip_byte_counts(tif, dir) >= 0)?;
        }
    }
    if !tiff_field_set(tif, FIELD_MAXSAMPLEVALUE) {
        if tif.tif_dir.td_bitspersample >= 16 {
            tif.tif_dir.td_maxsamplevalue = 0xFFFF;
        } else {
            tif.tif_dir.td_maxsamplevalue = ((1 << tif.tif_dir.td_bitspersample) - 1) as u16;
        }
    }

    /*
     * An opportunity for compression mode dependent tag fixup
     */
    (tif.tif_fixuptags)(tif);

    /*
     * Some manufacturers make life difficult by writing
     * large amounts of uncompressed data as a single strip.
     * This is contrary to the recommendations of the spec.
     * The following makes an attempt at breaking such images
     * into strips closer to the recommended 8k bytes.  A
     * side effect, however, is that the RowsPerStrip tag
     * value may be changed.
     */
    if (tif.tif_dir.td_planarconfig == PLANARCONFIG_CONTIG)
        && (tif.tif_dir.td_nstrips == 1)
        && (tif.tif_dir.td_compression == COMPRESSION_NONE)
        && ((tif.tif_flags & (TIFF_STRIPCHOP | TIFF_ISTILED)) == TIFF_STRIPCHOP)
    {
        chop_up_single_uncompressed_strip(tif);
    }

    /* There are also uncompressed striped files with strips larger than */
    /* 2 GB, which make them unfriendly with a lot of code. If possible, */
    /* try to expose smaller "virtual" strips. */
    if tif.tif_dir.td_planarconfig == PLANARCONFIG_CONTIG
        && tif.tif_dir.td_compression == COMPRESSION_NONE
        && (tif.tif_flags & (TIFF_STRIPCHOP | TIFF_ISTILED)) == TIFF_STRIPCHOP
        && tiff_strip_size64(tif) > 0x7FFFFFFF
    {
        try_chop_up_uncompressed_big_tiff(tif);
    }

    /*
     * Clear the dirty directory flag.
     */
    tif.tif_flags &= !TIFF_DIRTYDIRECT;
    tif.tif_flags &= !TIFF_DIRTYSTRIP;

    /*
     * Reinitialize some further i/o since we are starting on a new directory.
     */
    tif.tif_dir.td_scanlinesize = tiff_scanline_size(tif);
    if tif.tif_dir.td_scanlinesize == 0 {
        tiff_error_ext_r!(MODULE, "Cannot handle zero scanline size");
        return Ok(0);
    }

    if tif.is_tiled() {
        tif.tif_dir.td_tilesize = tiff_tile_size(tif);
        if tif.tif_dir.td_tilesize == 0 {
            tiff_error_ext_r!(MODULE, "Cannot handle zero tile size");
            return Ok(0);
        }
    } else if tiff_strip_size(tif) == 0 {
        tiff_error_ext_r!(MODULE, "Cannot handle zero strip size");
        return Ok(0);
    }
    Ok(1)
} /*-- TIFFReadDirectory() --*/

/// Translation of `TIFFReadDirectoryCheckOrder()`.
fn tiff_read_directory_check_order(dir: &[TIFFDirEntry]) {
    const MODULE: &str = "TIFFReadDirectoryCheckOrder";
    let mut m: u32 = 0;
    for o in dir {
        if (o.tdir_tag as u32) < m {
            tiff_warning_ext_r!(
                MODULE,
                "Invalid TIFF directory; tags are not sorted in ascending order"
            );
            break;
        }
        m = o.tdir_tag as u32 + 1;
    }
}

/// Translation of `TIFFReadDirectoryFindEntry()`: the index of the entry.
fn tiff_read_directory_find_entry(dir: &[TIFFDirEntry], tagid: u16) -> Option<usize> {
    dir.iter().position(|m| m.tdir_tag == tagid)
}

/// Translation of `TIFFReadDirectoryFindFieldInfo()`.
fn tiff_read_directory_find_field_info(tif: &Tiff<'_>, tagid: u16, fii: &mut u32) {
    let mut ma: i32 = -1;
    let mut mc: i32 = tif.tif_fields.len() as i32;
    let mut mb;
    loop {
        if ma + 1 == mc {
            *fii = FAILED_FII;
            return;
        }
        mb = (ma + mc) / 2;
        if tif.tif_fields[mb as usize].field_tag == tagid as u32 {
            break;
        }
        if tif.tif_fields[mb as usize].field_tag < tagid as u32 {
            ma = mb;
        } else {
            mc = mb;
        }
    }
    loop {
        if mb == 0 {
            break;
        }
        if tif.tif_fields[(mb - 1) as usize].field_tag != tagid as u32 {
            break;
        }
        mb -= 1;
    }
    *fii = mb as u32;
}

/// Translation of `EstimateStripByteCounts()`.
fn estimate_strip_byte_counts(tif: &mut Tiff<'_>, dir: &[TIFFDirEntry]) -> i32 {
    const MODULE: &str = "EstimateStripByteCounts";

    /* Do not try to load stripbytecount as we will compute it */
    if _tiff_fill_striles_internal(tif, false) == 0 {
        return -1;
    }

    let nstrips = tif.tif_dir.td_nstrips;
    let allocsize = nstrips as u64 * 8;
    let mut filesize: u64 = 0;
    if allocsize > 100 * 1024 * 1024 {
        /* Before allocating a huge amount of memory for corrupted files, check
         * if size of requested memory is not greater than file size. */
        filesize = tif.get_file_size();
        if allocsize > filesize {
            tiff_warning_ext_r!(
                MODULE,
                "Requested memory size for StripByteCounts of {} is greater than filesize {}. Memory not allocated",
                allocsize,
                filesize
            );
            return -1;
        }
    }

    tif.tif_dir.td_stripbytecount_p = None;
    let Some(mut counts) =
        _tiff_check_malloc_vec::<u64>(tif, nstrips as TmSize, 8, "for \"StripByteCounts\" array")
    else {
        return -1;
    };

    if tif.tif_dir.td_compression != COMPRESSION_NONE {
        let mut space: u64 = if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
            SIZEOF_TIFF_HEADER_CLASSIC as u64 + 2 + dir.len() as u64 * 12 + 4
        } else {
            SIZEOF_TIFF_HEADER_BIG as u64 + 8 + dir.len() as u64 * 20 + 8
        };
        /* calculate amount of space used by indirect values */
        for dp in dir {
            let typewidth = tiff_data_width(dp.tdir_type) as u32;
            if typewidth == 0 {
                tiff_error_ext_r!(
                    MODULE,
                    "Cannot determine size of unknown tag type {}",
                    dp.tdir_type
                );
                return -1;
            }
            if dp.tdir_count > u64::MAX / typewidth as u64 {
                return -1;
            }
            let mut datasize = typewidth as u64 * dp.tdir_count;
            if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
                if datasize <= 4 {
                    datasize = 0;
                }
            } else if datasize <= 8 {
                datasize = 0;
            }
            if space > u64::MAX - datasize {
                return -1;
            }
            space += datasize;
        }
        if filesize == 0 {
            filesize = tif.get_file_size();
        }
        if filesize < space {
            /* we should perhaps return in error ? */
            space = filesize;
        } else {
            space = filesize - space;
        }
        if tif.tif_dir.td_planarconfig == PLANARCONFIG_SEPARATE {
            space /= tif.tif_dir.td_samplesperpixel.max(1) as u64;
        }
        for c in counts.iter_mut() {
            *c = space;
        }
        /*
         * This gross hack handles the case were the offset to
         * the last strip is past the place where we think the strip
         * should begin.  Since a strip of data must be contiguous,
         * it's safe to assume that we've overestimated the amount
         * of data in the strip and trim this number back accordingly.
         */
        let strip = nstrips as usize - 1;
        let last_offset = tif
            .tif_dir
            .td_stripoffset_p
            .as_ref()
            .and_then(|o| o.get(strip).copied())
            .unwrap_or(0);
        if last_offset > u64::MAX - counts[strip] {
            return -1;
        }
        if last_offset + counts[strip] > filesize {
            if last_offset >= filesize {
                /* Not sure what we should in that case... */
                counts[strip] = 0;
            } else {
                counts[strip] = filesize - last_offset;
            }
        }
    } else if tif.is_tiled() {
        let bytespertile = tiff_tile_size64(tif);

        for c in counts.iter_mut() {
            *c = bytespertile;
        }
    } else {
        let rowbytes = tiff_scanline_size64(tif);
        let rowsperstrip = tif.tif_dir.td_imagelength / tif.tif_dir.td_stripsperimage.max(1);
        for c in counts.iter_mut() {
            if rowbytes > 0 && rowsperstrip as u64 > u64::MAX / rowbytes {
                return -1;
            }
            *c = rowbytes * rowsperstrip as u64;
        }
    }
    tif.tif_dir.td_stripbytecount_p = Some(counts);
    tiff_set_field_bit(tif, FIELD_STRIPBYTECOUNTS);
    if !tiff_field_set(tif, FIELD_ROWSPERSTRIP) {
        tif.tif_dir.td_rowsperstrip = tif.tif_dir.td_imagelength;
    }
    1
}

/// Translation of `MissingRequired()`.
fn missing_required(tagname: &str) {
    const MODULE: &str = "MissingRequired";

    tiff_error_ext_r!(
        MODULE,
        "TIFF directory is missing required \"{}\" field",
        tagname
    );
}

/// Translation of `_TIFFCheckDirNumberAndOffset()`: Check the directory
/// number and offset against the list of already seen directory numbers
/// and offsets. This is a trick to prevent IFD looping. The one can
/// create TIFF file with looped directory pointers. We will maintain a
/// list of already seen directories and check every IFD offset and its
/// IFD number against that list. However, the offset of an IFD number can
/// change - e.g. when writing updates to file. Returns 1 if all is ok; 0
/// if last directory or IFD loop is encountered, or an error has occurred.
/// (The C's two hash sets of `TIFFOffsetAndDirNumber` entries are two
/// maps.)
pub(crate) fn _tiff_check_dir_number_and_offset(tif: &mut Tiff<'_>, dirn: u32, diroff: u64) -> i32 {
    if diroff == 0 {
        /* no more directories */
        return 0;
    }

    let offset_to_number = tif
        .tif_map_dir_offset_to_number
        .get_or_insert_with(HashMap::new);

    /* Check if offset is already in the list:
     * - yes: check, if offset is at the same IFD number - if not, it is an IFD
     * loop
     * -  no: add to list or update offset at that IFD number
     */
    if let Some(&found) = offset_to_number.get(&diroff) {
        if found == dirn {
            return 1;
        } else {
            tiff_warning_ext_r!(
                "_TIFFCheckDirNumberAndOffset",
                "TIFF directory {} has IFD looping to directory {} at offset 0x{:x} ({})",
                dirn as i32 - 1,
                found,
                diroff,
                diroff
            );
            return 0;
        }
    }

    /* Check if offset of an IFD has been changed and update offset of that IFD
     * number. */
    let number_to_offset = tif
        .tif_map_dir_number_to_offset
        .get_or_insert_with(HashMap::new);
    if let Some(&found_offset) = number_to_offset.get(&dirn) {
        if found_offset != diroff {
            number_to_offset.remove(&dirn);
            number_to_offset.insert(dirn, diroff);
            let offset_to_number = tif
                .tif_map_dir_offset_to_number
                .get_or_insert_with(HashMap::new);
            offset_to_number.remove(&found_offset);
            offset_to_number.insert(diroff, dirn);
        }
        return 1;
    }

    /* Arbitrary (hopefully big enough) limit */
    let offset_to_number = tif
        .tif_map_dir_offset_to_number
        .get_or_insert_with(HashMap::new);
    if offset_to_number.len() >= TIFF_MAX_DIR_COUNT as usize {
        tiff_error_ext_r!(
            "_TIFFCheckDirNumberAndOffset",
            "Cannot handle more than {} TIFF directories",
            TIFF_MAX_DIR_COUNT
        );
        return 0;
    }

    /* Add IFD offset and dirn to IFD directory list */
    offset_to_number.insert(diroff, dirn);
    tif.tif_map_dir_number_to_offset
        .get_or_insert_with(HashMap::new)
        .insert(dirn, diroff);

    1
} /* --- _TIFFCheckDirNumberAndOffset() ---*/

/// Translation of `CheckDirCount()`.
#[allow(dead_code)] // (used by the custom directory reader in the C)
fn check_dir_count(tif: &Tiff<'_>, dir: &mut TIFFDirEntry, count: u32) -> i32 {
    if (count as u64) > dir.tdir_count {
        let fip = tiff_field_with_tag(tif, dir.tdir_tag as u32);
        tiff_warning_ext_r!(
            tif.tif_name,
            "incorrect count for field \"{}\" ({}, expecting {}); tag ignored",
            fip.as_ref()
                .map_or("unknown tagname", |f| f.field_name.as_ref()),
            dir.tdir_count,
            count
        );
        return 0;
    } else if (count as u64) < dir.tdir_count {
        let fip = tiff_field_with_tag(tif, dir.tdir_tag as u32);
        tiff_warning_ext_r!(
            tif.tif_name,
            "incorrect count for field \"{}\" ({}, expecting {}); tag trimmed",
            fip.as_ref()
                .map_or("unknown tagname", |f| f.field_name.as_ref()),
            dir.tdir_count,
            count
        );
        dir.tdir_count = count as u64;
        return 1;
    }
    1
}

/// Translation of `TIFFFetchDirectory()`: Read IFD structure from the
/// specified offset. If the pointer to nextdiroff variable has been
/// specified, read it too. Function returns the fields in the directory,
/// or `None` if failed (the C's 0 count).
fn tiff_fetch_directory(
    tif: &mut Tiff<'_>,
    diroff: u64,
    mut nextdiroff: Option<&mut u64>,
) -> Option<Vec<TIFFDirEntry>> {
    const MODULE: &str = "TIFFFetchDirectory";

    tif.tif_diroff = diroff;
    if let Some(n) = nextdiroff.as_deref_mut() {
        *n = 0;
    }
    // (not mapped)
    if !tif.seek_ok(tif.tif_diroff) {
        tiff_error_ext_r!(
            MODULE,
            "{}: Seek error accessing TIFF directory",
            tif.tif_name
        );
        return None;
    }
    let dircount16: u16;
    let dirsize: u32;
    if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
        let mut b = [0u8; 2];
        if !tif.read_ok(&mut b) {
            tiff_error_ext_r!(
                MODULE,
                "{}: Can not read TIFF directory count",
                tif.tif_name
            );
            return None;
        }
        let mut d = u16::from_ne_bytes(b);
        if (tif.tif_flags & TIFF_SWAB) != 0 {
            d = d.swap_bytes();
        }
        if d > 4096 {
            tiff_error_ext_r!(
                MODULE,
                "Sanity check on directory count failed, this is probably not a valid IFD offset"
            );
            return None;
        }
        dircount16 = d;
        dirsize = 12;
    } else {
        let mut b = [0u8; 8];
        if !tif.read_ok(&mut b) {
            tiff_error_ext_r!(
                MODULE,
                "{}: Can not read TIFF directory count",
                tif.tif_name
            );
            return None;
        }
        let mut dircount64 = u64::from_ne_bytes(b);
        if (tif.tif_flags & TIFF_SWAB) != 0 {
            dircount64 = dircount64.swap_bytes();
        }
        if dircount64 > 4096 {
            tiff_error_ext_r!(
                MODULE,
                "Sanity check on directory count failed, this is probably not a valid IFD offset"
            );
            return None;
        }
        dircount16 = dircount64 as u16;
        dirsize = 20;
    }
    let mut origdir = _tiff_check_malloc_vec::<u8>(
        tif,
        dircount16 as TmSize,
        dirsize as TmSize,
        "to read TIFF directory",
    )?;
    if !tif.read_ok(&mut origdir) {
        tiff_error_ext_r!(MODULE, "{:.100}: Can not read TIFF directory", tif.tif_name);
        return None;
    }
    /*
     * Read offset to next directory for sequential scans if
     * needed.
     */
    if let Some(n) = nextdiroff {
        if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
            let mut b = [0u8; 4];
            let mut nextdiroff32 = if tif.read_ok(&mut b) {
                u32::from_ne_bytes(b)
            } else {
                0
            };
            if (tif.tif_flags & TIFF_SWAB) != 0 {
                nextdiroff32 = nextdiroff32.swap_bytes();
            }
            *n = nextdiroff32 as u64;
        } else {
            let mut b = [0u8; 8];
            *n = if tif.read_ok(&mut b) {
                u64::from_ne_bytes(b)
            } else {
                0
            };
            if (tif.tif_flags & TIFF_SWAB) != 0 {
                *n = n.swap_bytes();
            }
        }
    }
    /* No check against filesize needed here because "dir" should have same size
     * than "origdir" checked above. */
    let mut dir: Vec<TIFFDirEntry> = Vec::new();
    if dir.try_reserve_exact(dircount16 as usize).is_err() {
        tiff_error_ext_r!(
            tif.tif_name,
            "Failed to allocate memory for {} ({} elements of {} bytes each)",
            "to read TIFF directory",
            dircount16,
            std::mem::size_of::<TIFFDirEntry>()
        );
        return None;
    }
    let swab = (tif.tif_flags & TIFF_SWAB) != 0;
    let u16_at = |b: &[u8]| {
        let v = u16::from_ne_bytes([b[0], b[1]]);
        if swab {
            v.swap_bytes()
        } else {
            v
        }
    };
    for ma in origdir.chunks_exact(dirsize as usize) {
        let mut mb = TIFFDirEntry {
            tdir_ignore: 0,
            tdir_tag: u16_at(&ma[0..2]),
            tdir_type: u16_at(&ma[2..4]),
            ..TIFFDirEntry::default()
        };
        if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
            let mut c = u32::from_ne_bytes([ma[4], ma[5], ma[6], ma[7]]);
            if swab {
                c = c.swap_bytes();
            }
            mb.tdir_count = c as u64;
            mb.tdir_offset = [0; 8];
            mb.tdir_offset[..4].copy_from_slice(&ma[8..12]);
        } else {
            let mut c = tiff_read_uint64(&ma[4..12]);
            if swab {
                c = c.swap_bytes();
            }
            mb.tdir_count = c;
            mb.tdir_offset.copy_from_slice(&ma[12..20]);
        }
        dir.push(mb);
    }
    Some(dir)
}

/// The value of `TIFFSetField()` of an array read: the array, or `NULL`.
fn arg<'a, T>(data: &'a Option<Vec<T>>, f: fn(&'a [T]) -> Va<'a>) -> Va<'a> {
    match data {
        Some(d) => f(d),
        None => Va::Null,
    }
}

/// Translation of `TIFFFetchNormalTag()`: Fetch a tag that is not handled
/// by special case code.
fn tiff_fetch_normal_tag(tif: &mut Tiff<'_>, dp: &mut TIFFDirEntry, recover: bool) -> i32 {
    const MODULE: &str = "TIFFFetchNormalTag";
    let mut fii = FAILED_FII;
    tiff_read_directory_find_field_info(tif, dp.tdir_tag, &mut fii);
    if fii == FAILED_FII {
        tiff_error_ext_r!(
            "TIFFFetchNormalTag",
            "No definition found for tag {}",
            dp.tdir_tag
        );
        return 0;
    }
    let fip: TIFFField = tif.tif_fields[fii as usize].clone();
    let tag = dp.tdir_tag as u32;
    let mut err = Err::Ok;

    /// `err = <read>; if (err == TIFFReadDirEntryErrOk) { ... }`.
    macro_rules! read_ok {
        ($read:expr, |$v:ident| $body:block) => {
            match $read {
                Ok($v) => $body,
                Err(e) => err = e,
            }
        };
    }
    /// The count check of the TIFF_SETGET_C0_* cases.
    macro_rules! c0_count {
        () => {
            if dp.tdir_count != fip.field_readcount as u64 {
                tiff_warning_ext_r!(
                    MODULE,
                    "incorrect count for field \"{}\", expected {}, got {}",
                    fip.field_name,
                    fip.field_readcount as i32,
                    dp.tdir_count
                );
                return 0;
            }
        };
    }
    /// A TIFF_SETGET_C0_* case: `TIFFSetField(tif, tag, data)`.
    macro_rules! c0 {
        ($read:ident, $va:path) => {{
            c0_count!();
            read_ok!($read(tif, dp), |data| {
                if !evaluate_ifd_datasize_reading(tif, dp) {
                    return 0;
                }
                let m = tiff_set_field(tif, tag, &[arg(&data, $va)]);
                if m == 0 {
                    return 0;
                }
            });
        }};
    }
    /// A TIFF_SETGET_C16_* case: `TIFFSetField(tif, tag, (uint16_t)count,
    /// data)`.
    macro_rules! c16 {
        ($read:ident, $va:path) => {{
            if dp.tdir_count > 0xFFFF {
                err = Err::Count;
            } else {
                read_ok!($read(tif, dp), |data| {
                    if !evaluate_ifd_datasize_reading(tif, dp) {
                        return 0;
                    }
                    let m = tiff_set_field(
                        tif,
                        tag,
                        &[Va::Int(dp.tdir_count as u16 as i64), arg(&data, $va)],
                    );
                    if m == 0 {
                        return 0;
                    }
                });
            }
        }};
    }
    /// A TIFF_SETGET_C32_* case: `TIFFSetField(tif, tag, (uint32_t)count,
    /// data)`.
    macro_rules! c32 {
        ($read:ident, $va:path) => {{
            read_ok!($read(tif, dp), |data| {
                if !evaluate_ifd_datasize_reading(tif, dp) {
                    return 0;
                }
                let m = tiff_set_field(
                    tif,
                    tag,
                    &[Va::Int(dp.tdir_count as u32 as i64), arg(&data, $va)],
                );
                if m == 0 {
                    return 0;
                }
            });
        }};
    }
    /// A scalar case: `TIFFSetField(tif, tag, data)`, `$eval` saying
    /// whether the C evaluates the IFD data size first.
    macro_rules! scalar {
        ($read:ident, $eval:expr, |$v:ident| $va:expr) => {{
            read_ok!($read(tif, dp), |$v| {
                let evaluate: bool = $eval;
                if evaluate && !evaluate_ifd_datasize_reading(tif, dp) {
                    return 0;
                }
                if tiff_set_field(tif, tag, &[$va]) == 0 {
                    return 0;
                }
            });
        }};
    }

    match fip.set_get_field_type {
        TIFF_SETGET_UNDEFINED => {
            tiff_error_ext_r!(
                "TIFFFetchNormalTag",
                "Defined set_get_field_type of custom tag {} ({}) is TIFF_SETGET_UNDEFINED and thus tag is not read from file",
                fip.field_tag,
                fip.field_name
            );
        }
        TIFF_SETGET_ASCII => {
            read_ok!(tiff_read_dir_entry_byte_array(tif, dp), |data| {
                let mut mb: usize = 0;
                if let Some(d) = data.as_ref() {
                    if dp.tdir_count > 0 && d[dp.tdir_count as usize - 1] == 0 {
                        /* optimization: if data is known to be 0 terminated, we
                         * can use strlen() */
                        mb = d.iter().position(|&c| c == 0).unwrap_or(d.len());
                    } else {
                        /* general case. equivalent to non-portable */
                        /* mb = strnlen((const char*)data,
                         * (uint32_t)dp->tdir_count); */
                        while mb < dp.tdir_count as u32 as usize {
                            if d[mb] == 0 {
                                break;
                            }
                            mb += 1;
                        }
                    }
                }
                if !evaluate_ifd_datasize_reading(tif, dp) {
                    return 0;
                }
                let mut data = data;
                if (mb as u64 + 1) < dp.tdir_count as u32 as u64 {
                    tiff_warning_ext_r!(
                        MODULE,
                        "ASCII value for tag \"{}\" contains null byte in value; value incorrectly truncated during reading due to implementation limitations",
                        fip.field_name
                    );
                } else if (mb as u64 + 1) > dp.tdir_count as u32 as u64 {
                    tiff_warning_ext_r!(
                        MODULE,
                        "ASCII value for tag \"{}\" does not end in null byte. Forcing it to be null",
                        fip.field_name
                    );
                    /* TIFFReadDirEntryArrayWithLimit() ensures this can't be
                     * larger than MAX_SIZE_TAG_DATA */
                    let n = dp.tdir_count as u32 as usize;
                    let Some(mut o) = try_vec::<u8>(n + 1) else {
                        return 0;
                    };
                    if let Some(d) = data.as_ref() {
                        o[..n].copy_from_slice(&d[..n]);
                    }
                    o[n] = 0;
                    data = Some(o);
                }
                let n = tiff_set_field(tif, tag, &[arg(&data, Va::U8s)]);
                if n == 0 {
                    return 0;
                }
            });
        }
        TIFF_SETGET_UINT8 => {
            scalar!(tiff_read_dir_entry_byte, false, |data| Va::Int(data as i64))
        }
        TIFF_SETGET_SINT8 => {
            scalar!(tiff_read_dir_entry_sbyte, false, |data| Va::Int(
                data as i64
            ))
        }
        TIFF_SETGET_UINT16 => {
            scalar!(tiff_read_dir_entry_short, false, |data| Va::Int(
                data as i64
            ))
        }
        TIFF_SETGET_SINT16 => {
            scalar!(tiff_read_dir_entry_sshort, false, |data| Va::Int(
                data as i64
            ))
        }
        TIFF_SETGET_UINT32 => {
            scalar!(tiff_read_dir_entry_long, false, |data| Va::Int(data as i64))
        }
        TIFF_SETGET_SINT32 => {
            scalar!(tiff_read_dir_entry_slong, false, |data| Va::Int(
                data as i64
            ))
        }
        TIFF_SETGET_UINT64 => {
            scalar!(tiff_read_dir_entry_long8, true, |data| Va::Int(data as i64))
        }
        TIFF_SETGET_SINT64 => {
            scalar!(tiff_read_dir_entry_slong8, true, |data| Va::Int(data))
        }
        TIFF_SETGET_FLOAT => {
            scalar!(tiff_read_dir_entry_float, true, |data| Va::Double(
                data as f64
            ))
        }
        TIFF_SETGET_DOUBLE => {
            scalar!(tiff_read_dir_entry_double, true, |data| Va::Double(data))
        }
        TIFF_SETGET_IFD8 => {
            scalar!(tiff_read_dir_entry_ifd8, true, |data| Va::Int(data as i64))
        }
        TIFF_SETGET_UINT16_PAIR => {
            if dp.tdir_count != 2 {
                tiff_warning_ext_r!(
                    MODULE,
                    "incorrect count for field \"{}\", expected 2, got {}",
                    fip.field_name,
                    dp.tdir_count
                );
                return 0;
            }
            read_ok!(tiff_read_dir_entry_short_array(tif, dp), |data| {
                let data = data.unwrap_or_default();
                let d0 = data.first().copied().unwrap_or(0);
                let d1 = data.get(1).copied().unwrap_or(0);
                let m = tiff_set_field(tif, tag, &[Va::Int(d0 as i64), Va::Int(d1 as i64)]);
                if m == 0 {
                    return 0;
                }
            });
        }
        TIFF_SETGET_C0_UINT8 => c0!(tiff_read_dir_entry_byte_array, Va::U8s),
        TIFF_SETGET_C0_SINT8 => c0!(tiff_read_dir_entry_sbyte_array, Va::I8s),
        TIFF_SETGET_C0_UINT16 => c0!(tiff_read_dir_entry_short_array, Va::U16s),
        TIFF_SETGET_C0_SINT16 => c0!(tiff_read_dir_entry_sshort_array, Va::I16s),
        TIFF_SETGET_C0_UINT32 => c0!(tiff_read_dir_entry_long_array, Va::U32s),
        TIFF_SETGET_C0_SINT32 => c0!(tiff_read_dir_entry_slong_array, Va::I32s),
        TIFF_SETGET_C0_UINT64 => c0!(tiff_read_dir_entry_long8_array, Va::U64s),
        TIFF_SETGET_C0_SINT64 => c0!(tiff_read_dir_entry_slong8_array, Va::I64s),
        TIFF_SETGET_C0_FLOAT => c0!(tiff_read_dir_entry_float_array, Va::F32s),
        /*--: Rational2Double: Extend for Double Arrays and Rational-Arrays read
         * into Double-Arrays. */
        TIFF_SETGET_C0_DOUBLE => c0!(tiff_read_dir_entry_double_array, Va::F64s),
        TIFF_SETGET_C0_IFD8 => c0!(tiff_read_dir_entry_ifd8_array, Va::U64s),
        TIFF_SETGET_C16_ASCII => {
            if dp.tdir_count > 0xFFFF {
                err = Err::Count;
            } else {
                read_ok!(tiff_read_dir_entry_byte_array(tif, dp), |data| {
                    if !evaluate_ifd_datasize_reading(tif, dp) {
                        return 0;
                    }
                    let mut data = data;
                    if let Some(d) = data.as_ref() {
                        if dp.tdir_count > 0 && d[dp.tdir_count as usize - 1] != 0 {
                            tiff_warning_ext_r!(
                                MODULE,
                                "ASCII value for ASCII array tag \"{}\" does not end in null byte. Forcing it to be null",
                                fip.field_name
                            );
                            /* Enlarge buffer and add terminating null. */
                            let n = dp.tdir_count as u32 as usize;
                            let Some(mut o) = try_vec::<u8>(n + 1) else {
                                return 0;
                            };
                            o[..n].copy_from_slice(&d[..n]);
                            o[n] = 0;
                            dp.tdir_count += 1; /* Increment for added null. */
                            data = Some(o);
                        }
                    }
                    let m = tiff_set_field(
                        tif,
                        tag,
                        &[Va::Int(dp.tdir_count as u16 as i64), arg(&data, Va::U8s)],
                    );
                    if m == 0 {
                        return 0;
                    }
                });
            }
        }
        TIFF_SETGET_C16_UINT8 => c16!(tiff_read_dir_entry_byte_array, Va::U8s),
        TIFF_SETGET_C16_SINT8 => c16!(tiff_read_dir_entry_sbyte_array, Va::I8s),
        TIFF_SETGET_C16_UINT16 => c16!(tiff_read_dir_entry_short_array, Va::U16s),
        TIFF_SETGET_C16_SINT16 => c16!(tiff_read_dir_entry_sshort_array, Va::I16s),
        TIFF_SETGET_C16_UINT32 => c16!(tiff_read_dir_entry_long_array, Va::U32s),
        TIFF_SETGET_C16_SINT32 => c16!(tiff_read_dir_entry_slong_array, Va::I32s),
        TIFF_SETGET_C16_UINT64 => c16!(tiff_read_dir_entry_long8_array, Va::U64s),
        TIFF_SETGET_C16_SINT64 => c16!(tiff_read_dir_entry_slong8_array, Va::I64s),
        TIFF_SETGET_C16_FLOAT => c16!(tiff_read_dir_entry_float_array, Va::F32s),
        TIFF_SETGET_C16_DOUBLE => c16!(tiff_read_dir_entry_double_array, Va::F64s),
        TIFF_SETGET_C16_IFD8 => c16!(tiff_read_dir_entry_ifd8_array, Va::U64s),
        TIFF_SETGET_C32_ASCII => {
            read_ok!(tiff_read_dir_entry_byte_array(tif, dp), |data| {
                if !evaluate_ifd_datasize_reading(tif, dp) {
                    return 0;
                }
                let mut data = data;
                if let Some(d) = data.as_ref() {
                    if dp.tdir_count > 0 && d[dp.tdir_count as usize - 1] != 0 {
                        tiff_warning_ext_r!(
                            MODULE,
                            "ASCII value for ASCII array tag \"{}\" does not end in null byte. Forcing it to be null",
                            fip.field_name
                        );
                        /* Enlarge buffer and add terminating null. */
                        let n = dp.tdir_count as u32 as usize;
                        let Some(mut o) = try_vec::<u8>(n + 1) else {
                            return 0;
                        };
                        o[..n].copy_from_slice(&d[..n]);
                        o[n] = 0;
                        dp.tdir_count += 1; /* Increment for added null. */
                        data = Some(o);
                    }
                }
                let m = tiff_set_field(
                    tif,
                    tag,
                    &[Va::Int(dp.tdir_count as u32 as i64), arg(&data, Va::U8s)],
                );
                if m == 0 {
                    return 0;
                }
            });
        }
        TIFF_SETGET_C32_UINT8 => {
            let r: EResult<(Option<Vec<u8>>, u32)> =
                if fip.field_tag == TIFFTAG_RICHTIFFIPTC && dp.tdir_type == TIFF_LONG {
                    /* Adobe's software (wrongly) writes RichTIFFIPTC tag with
                     * data type LONG instead of UNDEFINED. Work around this
                     * frequently found issue */
                    match tiff_read_dir_entry_array(tif, dp, 4) {
                        Ok(Some((mut origdata, count))) => {
                            if (tif.tif_flags & TIFF_SWAB) != 0 {
                                super::tif_swab::tiff_swab_array_of_long(
                                    &mut origdata,
                                    count as TmSize,
                                );
                            }
                            Ok((Some(origdata), count.wrapping_mul(4)))
                        }
                        Ok(None) => Ok((None, 0)),
                        Err(e) => Err(e),
                    }
                } else {
                    tiff_read_dir_entry_byte_array(tif, dp).map(|d| (d, dp.tdir_count as u32))
                };
            read_ok!(r, |data| {
                if !evaluate_ifd_datasize_reading(tif, dp) {
                    return 0;
                }
                let (data, count) = data;
                let m = tiff_set_field(tif, tag, &[Va::Int(count as i64), arg(&data, Va::U8s)]);
                if m == 0 {
                    return 0;
                }
            });
        }
        TIFF_SETGET_C32_SINT8 => c32!(tiff_read_dir_entry_sbyte_array, Va::I8s),
        TIFF_SETGET_C32_UINT16 => c32!(tiff_read_dir_entry_short_array, Va::U16s),
        TIFF_SETGET_C32_SINT16 => c32!(tiff_read_dir_entry_sshort_array, Va::I16s),
        TIFF_SETGET_C32_UINT32 => c32!(tiff_read_dir_entry_long_array, Va::U32s),
        TIFF_SETGET_C32_SINT32 => c32!(tiff_read_dir_entry_slong_array, Va::I32s),
        TIFF_SETGET_C32_UINT64 => c32!(tiff_read_dir_entry_long8_array, Va::U64s),
        TIFF_SETGET_C32_SINT64 => c32!(tiff_read_dir_entry_slong8_array, Va::I64s),
        TIFF_SETGET_C32_FLOAT => c32!(tiff_read_dir_entry_float_array, Va::F32s),
        TIFF_SETGET_C32_DOUBLE => c32!(tiff_read_dir_entry_double_array, Va::F64s),
        TIFF_SETGET_C32_IFD8 => c32!(tiff_read_dir_entry_ifd8_array, Va::U64s),
        _ => {
            /* TIFF_SETGET_INT, TIFF_SETGET_C0_ASCII, TIFF_SETGET_OTHER:
             * these should not arrive here */
        }
    }
    if err != Err::Ok {
        tiff_read_dir_entry_output_err(err, MODULE, &fip.field_name, recover);
    }
    1
}

/// Translation of `TIFFFetchStripThing()`: Fetch a set of offsets or
/// lengths. While this routine says "strips", in fact it's also used for
/// tiles. `None` is the C's failure; `Some(array)` what it stores.
fn tiff_fetch_strip_thing(
    tif: &mut Tiff<'_>,
    dir: &TIFFDirEntry,
    nstrips: u32,
) -> Option<Option<Vec<u64>>> {
    const MODULE: &str = "TIFFFetchStripThing";
    let data = match tiff_read_dir_entry_long8_array_with_limit(tif, dir, nstrips as u64) {
        Ok(d) => d,
        Err(err) => {
            let fip = tiff_field_with_tag(tif, dir.tdir_tag as u32);
            tiff_read_dir_entry_output_err(
                err,
                MODULE,
                fip.as_ref()
                    .map_or("unknown tagname", |f| f.field_name.as_ref()),
                false,
            );
            return None;
        }
    };
    let mut data = data;
    if dir.tdir_count < nstrips as u64 {
        let fip = tiff_field_with_tag(tif, dir.tdir_tag as u32);
        let mut max_nstrips: u32 = 1000000;
        if let Ok(psz_max) = std::env::var("LIBTIFF_STRILE_ARRAY_MAX_RESIZE_COUNT") {
            max_nstrips = atoi(&psz_max) as u32;
        }
        tiff_read_dir_entry_output_err(
            Err::Count,
            MODULE,
            fip.as_ref()
                .map_or("unknown tagname", |f| f.field_name.as_ref()),
            nstrips <= max_nstrips,
        );

        if nstrips > max_nstrips {
            return None;
        }

        let allocsize = nstrips as u64 * 8;
        if allocsize > 100 * 1024 * 1024 {
            /* Before allocating a huge amount of memory for corrupted files,
             * check if size of requested memory is not greater than file size.
             */
            let filesize = tif.get_file_size();
            if allocsize > filesize {
                tiff_warning_ext_r!(
                    MODULE,
                    "Requested memory size for StripArray of {} is greater than filesize {}. Memory not allocated",
                    allocsize,
                    filesize
                );
                return None;
            }
        }
        let mut resizeddata =
            _tiff_check_malloc_vec::<u64>(tif, nstrips as TmSize, 8, "for strip array")?;
        if dir.tdir_count != 0 {
            if let Some(d) = data.as_ref() {
                let n = (dir.tdir_count as usize).min(d.len());
                resizeddata[..n].copy_from_slice(&d[..n]);
            }
        }
        data = Some(resizeddata);
    }
    Some(data)
}

/// C's `atoi()`: the leading decimal integer (after white space), 0 if
/// none.
fn atoi(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i64 = 0;
    for c in digits.bytes() {
        if !c.is_ascii_digit() {
            break;
        }
        v = (v * 10 + (c - b'0') as i64).min(i64::from(u32::MAX) + 1);
    }
    (if neg { -v } else { v }) as i32
}

/// Translation of `allocChoppedUpStripArrays()`.
fn alloc_chopped_up_strip_arrays(
    tif: &mut Tiff<'_>,
    nstrips: u32,
    mut stripbytes: u64,
    rowsperstrip: u32,
) {
    let mut offset = tiff_get_strile_offset(tif, 0);
    let last = tif.tif_dir.td_nstrips.wrapping_sub(1);
    let last_offset = tiff_get_strile_offset(tif, last);
    let last_bytecount = tiff_get_strile_byte_count(tif, last);
    if last_offset > u64::MAX - last_bytecount || last_offset + last_bytecount < offset {
        return;
    }
    let mut bytecount = last_offset + last_bytecount - offset;

    /* Before allocating a huge amount of memory for corrupted files, check if
     * size of StripByteCount and StripOffset tags is not greater than
     * file size.
     */
    let allocsize = nstrips as u64 * 8 * 2;
    if allocsize > 100 * 1024 * 1024 {
        let filesize = tif.get_file_size();
        if allocsize > filesize {
            tiff_warning_ext_r!(
                "allocChoppedUpStripArrays",
                "Requested memory size for StripByteCount and StripOffsets {} is greater than filesize {}. Memory not allocated",
                allocsize,
                filesize
            );
            return;
        }
    }

    let newcounts = _tiff_check_malloc_vec::<u64>(
        tif,
        nstrips as TmSize,
        8,
        "for chopped \"StripByteCounts\" array",
    );
    let newoffsets = _tiff_check_malloc_vec::<u64>(
        tif,
        nstrips as TmSize,
        8,
        "for chopped \"StripOffsets\" array",
    );
    let (Some(mut newcounts), Some(mut newoffsets)) = (newcounts, newoffsets) else {
        /*
         * Unable to allocate new strip information, give up and use
         * the original one strip information.
         */
        return;
    };

    /*
     * Fill the strip information arrays with new bytecounts and offsets
     * that reflect the broken-up format.
     */
    for i in 0..nstrips as usize {
        if stripbytes > bytecount {
            stripbytes = bytecount;
        }
        newcounts[i] = stripbytes;
        newoffsets[i] = if stripbytes != 0 { offset } else { 0 };
        offset = offset.wrapping_add(stripbytes);
        bytecount -= stripbytes;
    }

    /*
     * Replace old single strip info with multi-strip info.
     */
    tif.tif_dir.td_nstrips = nstrips;
    tif.tif_dir.td_stripsperimage = nstrips;
    tiff_set_field(tif, TIFFTAG_ROWSPERSTRIP, &[Va::Int(rowsperstrip as i64)]);

    tif.tif_dir.td_stripbytecount_p = Some(newcounts);
    tif.tif_dir.td_stripoffset_p = Some(newoffsets);
    tif.tif_flags |= TIFF_CHOPPEDUPARRAYS;
}

/// Translation of `ChopUpSingleUncompressedStrip()`: Replace a single
/// strip (tile) of uncompressed data by multiple strips (tiles), each
/// approximately STRIP_SIZE_DEFAULT bytes. This is useful for dealing with
/// large images or for dealing with machines with a limited amount memory.
fn chop_up_single_uncompressed_strip(tif: &mut Tiff<'_>) {
    let bytecount = tiff_get_strile_byte_count(tif, 0);
    /* On a newly created file, just re-opened to be filled, we */
    /* don't want strip chop to trigger as it is going to cause issues */
    /* later ( StripOffsets and StripByteCounts improperly filled) . */
    if bytecount == 0 && tif.tif_mode != O_RDONLY {
        return;
    }
    let offset = tiff_get_strile_offset(tif, 0);
    let rowblock: u32 =
        if (tif.tif_dir.td_photometric == PHOTOMETRIC_YCBCR) && (!tif.is_up_sampled()) {
            tif.tif_dir.td_ycbcrsubsampling[1] as u32
        } else {
            1
        };
    let rowblockbytes = tiff_vtile_size64(tif, rowblock);
    /*
     * Make the rows hold at least one scanline, but fill specified amount
     * of data if possible.
     */
    let stripbytes: u64;
    let rowsperstrip: u32;
    if rowblockbytes > STRIP_SIZE_DEFAULT {
        stripbytes = rowblockbytes;
        rowsperstrip = rowblock;
    } else if let Some(q) = STRIP_SIZE_DEFAULT.checked_div(rowblockbytes) {
        let rowblocksperstrip = q as u32;
        rowsperstrip = rowblocksperstrip.wrapping_mul(rowblock);
        stripbytes = rowblocksperstrip as u64 * rowblockbytes;
    } else {
        return;
    }

    /*
     * never increase the number of rows per strip
     */
    if rowsperstrip >= tif.tif_dir.td_rowsperstrip || rowsperstrip == 0 {
        return;
    }
    let nstrips = tiff_howmany_32(tif.tif_dir.td_imagelength, rowsperstrip);
    if nstrips == 0 {
        return;
    }

    /* If we are going to allocate a lot of memory, make sure that the */
    /* file is as big as needed */
    if tif.tif_mode == O_RDONLY && nstrips > 1000000 {
        let filesize = tif.get_file_size();
        if offset >= filesize || stripbytes > (filesize - offset) / (nstrips as u64 - 1) {
            return;
        }
    }

    alloc_chopped_up_strip_arrays(tif, nstrips, stripbytes, rowsperstrip);
}

/// Translation of `TryChopUpUncompressedBigTiff()`: Replace a file with
/// contiguous strips > 2 GB of uncompressed data by multiple smaller
/// strips. This is useful for dealing with large images or for dealing
/// with machines with a limited amount memory.
fn try_chop_up_uncompressed_big_tiff(tif: &mut Tiff<'_>) {
    let stripsize = tiff_strip_size64(tif);

    /* On a newly created file, just re-opened to be filled, we */
    /* don't want strip chop to trigger as it is going to cause issues */
    /* later ( StripOffsets and StripByteCounts improperly filled) . */
    if tiff_get_strile_byte_count(tif, 0) == 0 && tif.tif_mode != O_RDONLY {
        return;
    }

    let rowblock: u32 =
        if (tif.tif_dir.td_photometric == PHOTOMETRIC_YCBCR) && (!tif.is_up_sampled()) {
            tif.tif_dir.td_ycbcrsubsampling[1] as u32
        } else {
            1
        };
    let rowblockbytes = tiff_vstrip_size64(tif, rowblock);
    if rowblockbytes == 0 || rowblockbytes > 0x7FFFFFFF {
        /* In case of file with gigantic width */
        return;
    }

    /* Check that the strips are contiguous and of the expected size */
    let nstrips0 = tif.tif_dir.td_nstrips;
    for i in 0..nstrips0 {
        if i == nstrips0 - 1 {
            let rows = tif
                .tif_dir
                .td_imagelength
                .wrapping_sub(i.wrapping_mul(tif.tif_dir.td_rowsperstrip));
            if tiff_get_strile_byte_count(tif, i) < tiff_vstrip_size64(tif, rows) {
                return;
            }
        } else {
            if tiff_get_strile_byte_count(tif, i) != stripsize {
                return;
            }
            if i > 0
                && tiff_get_strile_offset(tif, i)
                    != tiff_get_strile_offset(tif, i - 1)
                        .wrapping_add(tiff_get_strile_byte_count(tif, i - 1))
            {
                return;
            }
        }
    }

    /* Aim for 512 MB strips (that will still be manageable by 32 bit builds */
    let mut rowblocksperstrip = (512 * 1024 * 1024 / rowblockbytes) as u32;
    if rowblocksperstrip == 0 {
        rowblocksperstrip = 1;
    }
    let rowsperstrip = rowblocksperstrip.wrapping_mul(rowblock);
    let stripbytes = rowblocksperstrip as u64 * rowblockbytes;

    if rowsperstrip == 0 {
        return;
    }
    let nstrips = tiff_howmany_32(tif.tif_dir.td_imagelength, rowsperstrip);
    if nstrips == 0 {
        return;
    }

    /* If we are going to allocate a lot of memory, make sure that the */
    /* file is as big as needed */
    if tif.tif_mode == O_RDONLY && nstrips > 1000000 {
        let last = tif.tif_dir.td_nstrips - 1;
        let last_offset = tiff_get_strile_offset(tif, last);
        let filesize = tif.get_file_size();
        let last_bytecount = tiff_get_strile_byte_count(tif, last);
        if last_offset > filesize || last_bytecount > filesize - last_offset {
            return;
        }
    }

    alloc_chopped_up_strip_arrays(tif, nstrips, stripbytes, rowsperstrip);
}

/// Translation of `_TIFFGetStrileOffsetOrByteCountValue()` (`bytecount`
/// choosing the array; the deferred loading is not translated).
fn _tiff_get_strile_offset_or_byte_count_value(
    tif: &mut Tiff<'_>,
    strile: u32,
    bytecount: bool,
    pb_err: Option<&mut i32>,
) -> u64 {
    let mut err = 0;
    let td = &tif.tif_dir;

    /* Avoid the "dirent->tdir_count <= 4" code path for one of
     * StripOffsets/StripByteCounts, and the other code path for the other one,
     * which will lead to inconsistencies and potential out-of-bounds reads.
     */
    let value = if (td.td_stripoffset_entry.tdir_count <= 4)
        != (td.td_stripbytecount_entry.tdir_count <= 4)
    {
        tiff_error_ext_r!(
            "_TIFFGetStrileOffsetOrByteCountValue",
            "Inconsistent directory count between StripOffsets and StripByteCounts"
        );
        err = 1;
        0
    } else {
        // (TIFF_DEFERSTRILELOAD is never set)
        let parray = if bytecount {
            &td.td_stripbytecount_p
        } else {
            &td.td_stripoffset_p
        };
        match parray {
            Some(a) if strile < td.td_nstrips => a.get(strile as usize).copied().unwrap_or(0),
            _ => {
                err = 1;
                0
            }
        }
    };
    if let Some(e) = pb_err {
        *e = err;
    }
    value
}

/// Translation of `TIFFGetStrileOffset()`: Return the value of the
/// TileOffsets/StripOffsets array for the specified tile/strile
pub(crate) fn tiff_get_strile_offset(tif: &mut Tiff<'_>, strile: u32) -> u64 {
    tiff_get_strile_offset_with_err(tif, strile, None)
}

/// Translation of `TIFFGetStrileOffsetWithErr()`.
pub(crate) fn tiff_get_strile_offset_with_err(
    tif: &mut Tiff<'_>,
    strile: u32,
    pb_err: Option<&mut i32>,
) -> u64 {
    _tiff_get_strile_offset_or_byte_count_value(tif, strile, false, pb_err)
}

/// Translation of `TIFFGetStrileByteCount()`: Return the value of the
/// TileByteCounts/StripByteCounts array for the specified tile/strile
pub(crate) fn tiff_get_strile_byte_count(tif: &mut Tiff<'_>, strile: u32) -> u64 {
    tiff_get_strile_byte_count_with_err(tif, strile, None)
}

/// Translation of `TIFFGetStrileByteCountWithErr()`.
pub(crate) fn tiff_get_strile_byte_count_with_err(
    tif: &mut Tiff<'_>,
    strile: u32,
    pb_err: Option<&mut i32>,
) -> u64 {
    _tiff_get_strile_offset_or_byte_count_value(tif, strile, true, pb_err)
}

/// Translation of `_TIFFFillStriles()`.
pub(crate) fn _tiff_fill_striles(tif: &mut Tiff<'_>) -> i32 {
    _tiff_fill_striles_internal(tif, true)
}

/// Translation of `_TIFFFillStrilesInternal()`.
fn _tiff_fill_striles_internal(tif: &mut Tiff<'_>, _load_strip_byte_count: bool) -> i32 {
    /* Do not do anything if TIFF_DEFERSTRILELOAD is not set */
    if (tif.tif_flags & TIFF_DEFERSTRILELOAD) == 0 || (tif.tif_flags & TIFF_CHOPPEDUPARRAYS) != 0 {
        return 1;
    }
    // (TIFF_DEFERSTRILELOAD is never set: the rest is not translated)
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_is_c_like() {
        assert_eq!(atoi("  42abc"), 42);
        assert_eq!(atoi("-7"), -7);
        assert_eq!(atoi("x"), 0);
        assert_eq!(atoi(""), 0);
    }

    #[test]
    fn read_uint64_is_unaligned_safe() {
        let b = [1u8, 2, 3, 4, 5, 6, 7, 8, 9];
        assert_eq!(
            tiff_read_uint64(&b[1..]),
            u64::from_ne_bytes([2, 3, 4, 5, 6, 7, 8, 9])
        );
    }

    #[test]
    fn max_color_channels() {
        assert_eq!(_tiff_get_max_color_channels(PHOTOMETRIC_RGB), 3);
        assert_eq!(_tiff_get_max_color_channels(PHOTOMETRIC_SEPARATED), 4);
        assert_eq!(_tiff_get_max_color_channels(PHOTOMETRIC_LOGL), 0);
    }
}
