// Rust translation of libtiff/tif_read.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Scanline-oriented Read Support: the strip and tile readers the RGBA
//! image reader uses. The scanline interface (`TIFFReadScanline()` with
//! `TIFFSeek()` and `TIFFFillStripPartial()`), the raw readers
//! (`TIFFReadRawStrip()`, `TIFFReadRawTile()`) and
//! `TIFFReadFromUserBuffer()` are left out, as is the memory-mapped path
//! (SDL_image opens with 'm'). `_TIFFmallocExt()`'s buffers are zeroed
//! here where the C leaves them uninitialized.

use super::tif_compress::tiff_get_max_compression_ratio;
use super::tif_dir::{NOSTRIP, NOTILE};
use super::tif_dirread::{tiff_get_strile_byte_count, tiff_get_strile_offset};
use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tif_strip::{tiff_strip_size, tiff_vstrip_size};
use super::tif_swab::{
    tiff_reverse_bits, tiff_swab_array_of_double, tiff_swab_array_of_long,
    tiff_swab_array_of_short, tiff_swab_array_of_triples,
};
use super::tif_tile::{tiff_check_tile, tiff_compute_tile, tiff_tile_size};
use super::tiff::*;
use super::tiffiop::*;

const INITIAL_THRESHOLD: TmSize = 1024 * 1024;
const THRESHOLD_MULTIPLIER: TmSize = 10;
const MAX_THRESHOLD: TmSize =
    THRESHOLD_MULTIPLIER * THRESHOLD_MULTIPLIER * THRESHOLD_MULTIPLIER * INITIAL_THRESHOLD;

const TIFF_INT64_MAX: i64 = i64::MAX;

/// Translation of `TIFFReadAndRealloc()`: Read 'size' bytes in
/// tif_rawdata buffer starting at offset 'rawdata_offset' Returns 1 in
/// case of success, 0 otherwise.
fn tiff_read_and_realloc(
    tif: &mut Tiff<'_>,
    size: TmSize,
    rawdata_offset: TmSize,
    is_strip: bool,
    strip_or_tile: u32,
    module: &str,
) -> i32 {
    let mut threshold: TmSize = INITIAL_THRESHOLD;
    let mut already_read: TmSize = 0;

    if !cfg!(target_pointer_width = "64") {
        /* On 32 bit processes, if the request is large enough, check against */
        /* file size */
        if size > 1000 * 1000 * 1000 {
            let filesize = tif.get_file_size();
            if size as u64 >= filesize {
                tiff_error_ext_r!(module, "Chunk size requested is larger than file size.");
                return 0;
            }
        }
    }

    /* On 64 bit processes, read first a maximum of 1 MB, then 10 MB, etc */
    /* so as to avoid allocating too much memory in case the file is too */
    /* short. We could ask for the file size, but this might be */
    /* expensive with some I/O layers (think of reading a gzipped file) */
    /* Restrict to 64 bit processes, so as to avoid reallocs() */
    /* on 32 bit processes where virtual memory is scarce.  */
    while already_read < size {
        let mut to_read = size - already_read;
        if cfg!(target_pointer_width = "64")
            && to_read >= threshold
            && threshold < MAX_THRESHOLD
            && already_read + to_read + rawdata_offset > tif.tif_rawdatasize
        {
            to_read = threshold;
            threshold *= THRESHOLD_MULTIPLIER;
        }
        if already_read + to_read + rawdata_offset > tif.tif_rawdatasize {
            tif.tif_rawdatasize = tiff_roundup_64(
                (already_read as u64)
                    .wrapping_add(to_read as u64)
                    .wrapping_add(rawdata_offset as u64),
                1024,
            ) as TmSize;
            if tif.tif_rawdatasize == 0 {
                tiff_error_ext_r!(module, "Invalid buffer size");
                return 0;
            }
            let new_size = tif.tif_rawdatasize as usize;
            if tif
                .tif_rawdata
                .try_reserve_exact(new_size.saturating_sub(tif.tif_rawdata.len()))
                .is_err()
            {
                tiff_error_ext_r!(
                    module,
                    "No space for data buffer at scanline {}",
                    tif.tif_dir.td_row
                );
                tif.tif_rawdata = Vec::new();
                tif.tif_rawdatasize = 0;
                return 0;
            }
            tif.tif_rawdata.resize(new_size, 0);
        }
        if tif.tif_rawdata.is_empty() {
            /* should not happen in practice but helps CoverityScan */
            return 0;
        }

        let start = (rawdata_offset + already_read) as usize;
        let end = start + to_read as usize;
        let mut buf = std::mem::take(&mut tif.tif_rawdata);
        let mut bytes_read = tif.read_file(&mut buf[start..end]);
        tif.tif_rawdata = buf;
        if bytes_read < 0 {
            /* Treat read errors as short reads before updating offsets. */
            bytes_read = 0;
        }
        already_read += bytes_read;
        if bytes_read != to_read {
            let from = (rawdata_offset + already_read) as usize;
            let to = tif.tif_rawdatasize as usize;
            tif.tif_rawdata[from..to].fill(0);
            if is_strip {
                tiff_error_ext_r!(
                    module,
                    "Read error at scanline {}; got {} bytes, expected {}",
                    tif.tif_dir.td_row,
                    already_read,
                    size
                );
            } else {
                tiff_error_ext_r!(
                    module,
                    "Read error at row {}, col {}, tile {}; got {} bytes, expected {}",
                    tif.tif_dir.td_row,
                    tif.tif_dir.td_col,
                    strip_or_tile,
                    already_read,
                    size
                );
            }
            return 0;
        }
    }
    1
}

/// Translation of `TIFFReadEncodedStripGetStripSize()`: Calculate the
/// strip size according to the number of rows in the strip (check for
/// truncated last strip on any of the separations).
fn tiff_read_encoded_strip_get_strip_size(
    tif: &mut Tiff<'_>,
    strip: u32,
    pplane: Option<&mut u16>,
) -> TmSize {
    const MODULE: &str = "TIFFReadEncodedStrip";
    if tiff_check_read(tif, false) == 0 {
        return -1;
    }
    let td = &tif.tif_dir;
    if strip >= td.td_nstrips {
        tiff_error_ext_r!(
            MODULE,
            "{}: Strip out of range, max {}",
            strip,
            td.td_nstrips
        );
        return -1;
    }

    let mut rowsperstrip = td.td_rowsperstrip;
    if rowsperstrip > td.td_imagelength {
        rowsperstrip = td.td_imagelength;
    }
    if rowsperstrip == 0 {
        tiff_error_ext_r!(MODULE, "rowsperstrip is zero");
        return -1;
    }
    let stripsperplane = tiff_howmany_32_maxuint_compat(td.td_imagelength, rowsperstrip);
    let stripinplane = strip % stripsperplane;
    if let Some(p) = pplane {
        *p = (strip / stripsperplane) as u16;
    }
    let mut rows = td
        .td_imagelength
        .wrapping_sub(stripinplane.wrapping_mul(rowsperstrip));
    if rows > rowsperstrip {
        rows = rowsperstrip;
    }
    let stripsize = tiff_vstrip_size(tif, rows);
    if stripsize == 0 {
        return -1;
    }
    stripsize
}

/// Translation of `TIFFReadEncodedStrip()`: Read a strip of data and
/// decompress the specified amount into the user-supplied buffer.
pub(crate) fn tiff_read_encoded_strip(
    tif: &mut Tiff<'_>,
    strip: u32,
    buf: &mut [u8],
    size: TmSize,
) -> TmSize {
    const MODULE: &str = "TIFFReadEncodedStrip";
    let mut plane: u16 = 0;

    let mut stripsize = tiff_read_encoded_strip_get_strip_size(tif, strip, Some(&mut plane));
    if stripsize == -1 {
        return -1;
    }

    /* shortcut to avoid an extra memcpy() */
    if tif.tif_dir.td_compression == COMPRESSION_NONE
        && size != -1
        && size >= stripsize
        && !tif.is_mapped()
        && (tif.tif_flags & TIFF_NOREADRAW) == 0
    {
        if tiff_read_raw_strip1(tif, strip, buf, stripsize, MODULE) != stripsize {
            return -1;
        }

        if !tif.is_fill_order(tif.tif_dir.td_fillorder) && (tif.tif_flags & TIFF_NOBITREV) == 0 {
            tiff_reverse_bits(buf, stripsize);
        }

        tiff_post_decode(tif, buf, stripsize);
        return stripsize;
    }

    if (size != -1) && (size < stripsize) {
        stripsize = size;
    }
    if tiff_fill_strip(tif, strip) == 0 {
        /* The output buf may be NULL, in particular if TIFFTAG_FAXFILLFUNC
        is being used. Thus, memset must be conditional on buf not NULL. */
        let n = (stripsize.max(0) as usize).min(buf.len());
        buf[..n].fill(0);
        return -1;
    }
    let n = (stripsize.max(0) as usize).min(buf.len());
    if (tif.tif_decodestrip)(tif, &mut buf[..n], stripsize, plane) <= 0 {
        return -1;
    }
    tiff_post_decode(tif, buf, stripsize);
    stripsize
}

/// Translation of `_TIFFReadEncodedStripAndAllocBuffer()`: Variant of
/// TIFFReadEncodedStrip() that does
/// * if *buf == NULL, *buf = _TIFFmallocExt(tif, bufsizetoalloc) only after
///   TIFFFillStrip() has succeeded. This avoid excessive memory allocation in
///   case of truncated file.
/// * calls regular TIFFReadEncodedStrip() if *buf != NULL
pub(crate) fn _tiff_read_encoded_strip_and_alloc_buffer(
    tif: &mut Tiff<'_>,
    strip: u32,
    buf: &mut Option<Vec<u8>>,
    bufsizetoalloc: TmSize,
    size_to_read: TmSize,
) -> TmSize {
    let mut plane: u16 = 0;

    if let Some(b) = buf.as_mut() {
        return tiff_read_encoded_strip(tif, strip, b, size_to_read);
    }

    let mut this_stripsize = tiff_read_encoded_strip_get_strip_size(tif, strip, Some(&mut plane));
    if this_stripsize == -1 {
        return -1;
    }

    if (size_to_read != -1) && (size_to_read < this_stripsize) {
        this_stripsize = size_to_read;
    }
    if tiff_fill_strip(tif, strip) == 0 {
        return -1;
    }

    /* Sanity checks to avoid excessive memory allocation */
    /* Max compression ratio experimentally determined. Might be fragile...
     * Only apply this heuristics to situations where the memory allocation
     * would be big, to avoid breaking nominal use cases.
     */
    if bufsizetoalloc > 100 * 1024 * 1024 {
        let max_compression_ratio = tiff_get_max_compression_ratio(tif);
        if max_compression_ratio > 0
            && (tif.tif_rawdatasize as u64) < (this_stripsize as u64) / max_compression_ratio
        {
            tiff_error_ext_r!(
                tif.tif_name,
                "Likely invalid strip byte count for strip {}. Uncompressed strip size is {}, compressed one is {}",
                strip,
                this_stripsize as u64,
                tif.tif_rawdatasize as u64
            );
            return -1;
        }
    }

    let Some(b) = (if bufsizetoalloc > 0 {
        try_vec::<u8>(bufsizetoalloc as usize)
    } else {
        None
    }) else {
        tiff_error_ext_r!(tif.tif_name, "No space for strip buffer");
        return -1;
    };
    let b = buf.insert(b);

    let n = (this_stripsize.max(0) as usize).min(b.len());
    if (tif.tif_decodestrip)(tif, &mut b[..n], this_stripsize, plane) <= 0 {
        return -1;
    }
    tiff_post_decode(tif, b, this_stripsize);
    this_stripsize
}

/// Translation of `TIFFReadRawStrip1()` (not mapped).
fn tiff_read_raw_strip1(
    tif: &mut Tiff<'_>,
    strip: u32,
    buf: &mut [u8],
    size: TmSize,
    module: &str,
) -> TmSize {
    let off = tiff_get_strile_offset(tif, strip);
    if !tif.seek_ok(off) {
        tiff_error_ext_r!(
            module,
            "Seek error at scanline {}, strip {}",
            tif.tif_dir.td_row,
            strip
        );
        return -1;
    }
    let n = (size.max(0) as usize).min(buf.len());
    let cc = tif.read_file(&mut buf[..n]);
    if cc != size {
        tiff_error_ext_r!(
            module,
            "Read error at scanline {}; got {} bytes, expected {}",
            tif.tif_dir.td_row,
            cc,
            size
        );
        return -1;
    }
    size
}

/// Translation of `TIFFReadRawStripOrTile2()`.
fn tiff_read_raw_strip_or_tile2(
    tif: &mut Tiff<'_>,
    strip_or_tile: u32,
    is_strip: bool,
    size: TmSize,
    module: &str,
) -> TmSize {
    let off = tiff_get_strile_offset(tif, strip_or_tile);
    if !tif.seek_ok(off) {
        if is_strip {
            tiff_error_ext_r!(
                module,
                "Seek error at scanline {}, strip {}",
                tif.tif_dir.td_row,
                strip_or_tile
            );
        } else {
            tiff_error_ext_r!(
                module,
                "Seek error at row {}, col {}, tile {}",
                tif.tif_dir.td_row,
                tif.tif_dir.td_col,
                strip_or_tile
            );
        }
        return -1;
    }

    if tiff_read_and_realloc(tif, size, 0, is_strip, strip_or_tile, module) == 0 {
        return -1;
    }

    size
}

/// The checks `TIFFFillStrip()` and `TIFFFillTile()` share before reading:
/// the byte count, limited, or `None` when it is refused.
fn check_strile_byte_count(
    tif: &mut Tiff<'_>,
    strile: u32,
    strilesize: TmSize,
    module: &str,
    is_strip: bool,
) -> Option<u64> {
    let mut bytecount = tiff_get_strile_byte_count(tif, strile);
    if bytecount == 0 || bytecount > TIFF_INT64_MAX as u64 {
        if is_strip {
            tiff_error_ext_r!(
                module,
                "Invalid strip byte count {}, strip {}",
                bytecount,
                strile
            );
        } else {
            tiff_error_ext_r!(
                module,
                "{}: Invalid tile byte count, tile {}",
                bytecount,
                strile
            );
        }
        return None;
    }

    /* To avoid excessive memory allocations: */
    if strilesize > 0 {
        if bytecount > 1024 * 1024 && (bytecount - 4096) / 10 > strilesize as u64 {
            /* Byte count should normally not be larger than a number of */
            /* times the uncompressed size plus some margin */
            /* 10 and 4096 are just values that could be adjusted. */
            /* Hopefully they are safe enough for all codecs */
            /* What happens next will depend on whether only the bytecount
             */
            /* was corrupted to a large value but the strip/tile data is */
            /* fine. In that situation most codecs should work fine and */
            /* only used part of the tile/strip data. If the strip/tile */
            /* data is corrupted too, then codecs will later error out. */
            let newbytecount = strilesize as u64 * 10 + 4096;
            if is_strip {
                tiff_warning_ext_r!(
                    module,
                    "Too large strip byte count {}, strip {}. Limiting to {}",
                    bytecount,
                    strile,
                    newbytecount
                );
            } else {
                tiff_warning_ext_r!(
                    module,
                    "Too large tile byte count {}, tile {}. Limiting to {}",
                    bytecount,
                    strile,
                    newbytecount
                );
            }
            bytecount = newbytecount;
        } else if strilesize > 100 * 1024 * 1024 {
            /* Max compression ratio experimentally determined. Might be
             * fragile... Only apply this heuristics to situations where the
             * memory allocation would be big, to avoid breaking nominal use
             * cases.
             */
            let max_compression_ratio = tiff_get_max_compression_ratio(tif);
            if max_compression_ratio > 0 && bytecount < strilesize as u64 / max_compression_ratio
            {
                if is_strip {
                    tiff_error_ext_r!(
                        module,
                        "Likely invalid strip byte count for strip {}. Uncompressed strip size is {}, compressed one is {}",
                        strile,
                        strilesize as u64,
                        bytecount
                    );
                } else {
                    tiff_error_ext_r!(
                        module,
                        "Likely invalid tile byte count for tile {}. Uncompressed tile size is {}, compressed one is {}",
                        strile,
                        strilesize as u64,
                        bytecount
                    );
                }
                return None;
            }
        }
    }
    Some(bytecount)
}

/// Translation of `TIFFFillStrip()`: Read the specified strip and setup
/// for decoding. The data buffer is expanded, as necessary, to hold the
/// strip's data.
pub(crate) fn tiff_fill_strip(tif: &mut Tiff<'_>, strip: u32) -> i32 {
    const MODULE: &str = "TIFFFillStrip";

    if (tif.tif_flags & TIFF_NOREADRAW) == 0 {
        let stripsize = tiff_strip_size(tif);
        let Some(bytecount) = check_strile_byte_count(tif, strip, stripsize, MODULE, true) else {
            return 0;
        };

        // (not mapped)
        /*
         * Expand raw data buffer, if needed, to hold data
         * strip coming from file (perhaps should set upper
         * bound on the size of a buffer we'll use?).
         */
        let bytecountm = bytecount as TmSize;
        if bytecountm as u64 != bytecount {
            tiff_error_ext_r!(MODULE, "Integer overflow");
            return 0;
        }
        if bytecountm > tif.tif_rawdatasize {
            tif.tif_dir.td_curstrip = NOSTRIP;
            if (tif.tif_flags & TIFF_MYBUFFER) == 0 {
                tiff_error_ext_r!(MODULE, "Data buffer too small to hold strip {}", strip);
                return 0;
            }
        }
        if (tif.tif_flags & TIFF_BUFFERMMAP) != 0 {
            tif.tif_dir.td_curstrip = NOSTRIP;
            tif.tif_rawdata = Vec::new();
            tif.tif_rawdatasize = 0;
            tif.tif_flags &= !TIFF_BUFFERMMAP;
        }

        if tiff_read_raw_strip_or_tile2(tif, strip, true, bytecountm, MODULE) != bytecountm {
            return 0;
        }

        tif.tif_rawdataoff = 0;
        tif.tif_rawdataloaded = bytecountm;

        if !tif.is_fill_order(tif.tif_dir.td_fillorder) && (tif.tif_flags & TIFF_NOBITREV) == 0 {
            tiff_reverse_bits(&mut tif.tif_rawdata, bytecountm);
        }
    }
    tiff_start_strip(tif, strip)
}

/*
 * Tile-oriented Read Support
 * Contributed by Nancy Cam (Silicon Graphics).
 */

/// Translation of `TIFFReadTile()`: Read and decompress a tile of data.
/// The tile is selected by the (x,y,z,s) coordinates.
pub(crate) fn tiff_read_tile(
    tif: &mut Tiff<'_>,
    buf: &mut [u8],
    x: u32,
    y: u32,
    z: u32,
    s: u16,
) -> TmSize {
    if tiff_check_read(tif, true) == 0 || tiff_check_tile(tif, x, y, z, s) == 0 {
        return -1;
    }
    let tile = tiff_compute_tile(tif, x, y, z, s);
    tiff_read_encoded_tile(tif, tile, buf, -1)
}

/// Translation of `TIFFReadEncodedTile()`: Read a tile of data and
/// decompress the specified amount into the user-supplied buffer.
pub(crate) fn tiff_read_encoded_tile(
    tif: &mut Tiff<'_>,
    tile: u32,
    buf: &mut [u8],
    mut size: TmSize,
) -> TmSize {
    const MODULE: &str = "TIFFReadEncodedTile";
    let tilesize = tif.tif_dir.td_tilesize;

    if tiff_check_read(tif, true) == 0 {
        return -1;
    }
    if tile >= tif.tif_dir.td_nstrips {
        tiff_error_ext_r!(
            MODULE,
            "{}: Tile out of range, max {}",
            tile,
            tif.tif_dir.td_nstrips
        );
        return -1;
    }

    /* shortcut to avoid an extra memcpy() */
    if tif.tif_dir.td_compression == COMPRESSION_NONE
        && size != -1
        && size >= tilesize
        && !tif.is_mapped()
        && (tif.tif_flags & TIFF_NOREADRAW) == 0
    {
        if tiff_read_raw_tile1(tif, tile, buf, tilesize, MODULE) != tilesize {
            return -1;
        }

        if !tif.is_fill_order(tif.tif_dir.td_fillorder) && (tif.tif_flags & TIFF_NOBITREV) == 0 {
            tiff_reverse_bits(buf, tilesize);
        }

        tiff_post_decode(tif, buf, tilesize);
        return tilesize;
    }

    if size == -1 || size > tilesize {
        size = tilesize;
    }
    if tiff_fill_tile(tif, tile) == 0 {
        /* See TIFFReadEncodedStrip comment regarding TIFFTAG_FAXFILLFUNC. */
        let n = (size.max(0) as usize).min(buf.len());
        buf[..n].fill(0);
        return -1;
    }
    let plane = (tile / tif.tif_dir.td_stripsperimage.max(1)) as u16;
    let n = (size.max(0) as usize).min(buf.len());
    if (tif.tif_decodetile)(tif, &mut buf[..n], size, plane) != 0 {
        tiff_post_decode(tif, buf, size);
        size
    } else {
        -1
    }
}

/// Translation of `_TIFFReadTileAndAllocBuffer()`: Variant of
/// TIFFReadTile() that does
/// * if *buf == NULL, *buf = _TIFFmallocExt(tif, bufsizetoalloc) only after
///   TIFFFillTile() has succeeded. This avoid excessive memory allocation in
///   case of truncated file.
/// * calls regular TIFFReadEncodedTile() if *buf != NULL
#[allow(clippy::too_many_arguments)]
pub(crate) fn _tiff_read_tile_and_alloc_buffer(
    tif: &mut Tiff<'_>,
    buf: &mut Option<Vec<u8>>,
    bufsizetoalloc: TmSize,
    x: u32,
    y: u32,
    z: u32,
    s: u16,
) -> TmSize {
    if tiff_check_read(tif, true) == 0 || tiff_check_tile(tif, x, y, z, s) == 0 {
        return -1;
    }
    let tile = tiff_compute_tile(tif, x, y, z, s);
    _tiff_read_encoded_tile_and_alloc_buffer(tif, tile, buf, bufsizetoalloc, -1)
}

/// Translation of `_TIFFReadEncodedTileAndAllocBuffer()`: Variant of
/// TIFFReadEncodedTile() that does
/// * if *buf == NULL, *buf = _TIFFmallocExt(tif, bufsizetoalloc) only after
///   TIFFFillTile() has succeeded. This avoid excessive memory allocation in
///   case of truncated file.
/// * calls regular TIFFReadEncodedTile() if *buf != NULL
pub(crate) fn _tiff_read_encoded_tile_and_alloc_buffer(
    tif: &mut Tiff<'_>,
    tile: u32,
    buf: &mut Option<Vec<u8>>,
    bufsizetoalloc: TmSize,
    mut size_to_read: TmSize,
) -> TmSize {
    const MODULE: &str = "_TIFFReadEncodedTileAndAllocBuffer";
    let tilesize = tif.tif_dir.td_tilesize;

    if let Some(b) = buf.as_mut() {
        return tiff_read_encoded_tile(tif, tile, b, size_to_read);
    }

    if tiff_check_read(tif, true) == 0 {
        return -1;
    }
    if tile >= tif.tif_dir.td_nstrips {
        tiff_error_ext_r!(
            MODULE,
            "{}: Tile out of range, max {}",
            tile,
            tif.tif_dir.td_nstrips
        );
        return -1;
    }

    if tiff_fill_tile(tif, tile) == 0 {
        return -1;
    }

    /* Sanity checks to avoid excessive memory allocation */
    /* Cf https://gitlab.com/libtiff/libtiff/-/issues/479 */
    if tif.tif_dir.td_compression == COMPRESSION_NONE {
        if tif.tif_rawdatasize != tilesize {
            tiff_error_ext_r!(
                tif.tif_name,
                "Invalid tile byte count for tile {}. Expected {}, got {}",
                tile,
                tilesize as u64,
                tif.tif_rawdatasize as u64
            );
            return -1;
        }
    } else {
        /* Max compression ratio experimentally determined. Might be fragile...
         * Only apply this heuristics to situations where the memory allocation
         * would be big, to avoid breaking nominal use cases.
         */
        if bufsizetoalloc > 100 * 1024 * 1024 {
            let max_compression_ratio = tiff_get_max_compression_ratio(tif);
            if max_compression_ratio > 0
                && (tif.tif_rawdatasize as u64) < (tilesize as u64) / max_compression_ratio
            {
                tiff_error_ext_r!(
                    tif.tif_name,
                    "Likely invalid tile byte count for tile {}. Uncompressed tile size is {}, compressed one is {}",
                    tile,
                    tilesize as u64,
                    tif.tif_rawdatasize as u64
                );
                return -1;
            }
        }
    }

    let Some(b) = (if bufsizetoalloc > 0 {
        try_vec::<u8>(bufsizetoalloc as usize)
    } else {
        None
    }) else {
        tiff_error_ext_r!(tif.tif_name, "No space for tile buffer");
        return -1;
    };
    let b = buf.insert(b);

    if size_to_read == -1 || size_to_read > tilesize {
        size_to_read = tilesize;
    }
    let plane = (tile / tif.tif_dir.td_stripsperimage.max(1)) as u16;
    let n = (size_to_read.max(0) as usize).min(b.len());
    if (tif.tif_decodetile)(tif, &mut b[..n], size_to_read, plane) != 0 {
        tiff_post_decode(tif, b, size_to_read);
        size_to_read
    } else {
        -1
    }
}

/// Translation of `TIFFReadRawTile1()` (not mapped).
fn tiff_read_raw_tile1(
    tif: &mut Tiff<'_>,
    tile: u32,
    buf: &mut [u8],
    size: TmSize,
    module: &str,
) -> TmSize {
    let off = tiff_get_strile_offset(tif, tile);
    if !tif.seek_ok(off) {
        tiff_error_ext_r!(
            module,
            "Seek error at row {}, col {}, tile {}",
            tif.tif_dir.td_row,
            tif.tif_dir.td_col,
            tile
        );
        return -1;
    }
    let n = (size.max(0) as usize).min(buf.len());
    let cc = tif.read_file(&mut buf[..n]);
    if cc != size {
        tiff_error_ext_r!(
            module,
            "Read error at row {}, col {}; got {} bytes, expected {}",
            tif.tif_dir.td_row,
            tif.tif_dir.td_col,
            cc,
            size
        );
        return -1;
    }
    size
}

/// Translation of `TIFFFillTile()`: Read the specified tile and setup for
/// decoding. The data buffer is expanded, as necessary, to hold the tile's
/// data.
pub(crate) fn tiff_fill_tile(tif: &mut Tiff<'_>, tile: u32) -> i32 {
    const MODULE: &str = "TIFFFillTile";

    if (tif.tif_flags & TIFF_NOREADRAW) == 0 {
        let tilesize = tiff_tile_size(tif);
        let Some(bytecount) = check_strile_byte_count(tif, tile, tilesize, MODULE, false) else {
            return 0;
        };

        // (not mapped)
        /*
         * Expand raw data buffer, if needed, to hold data
         * tile coming from file (perhaps should set upper
         * bound on the size of a buffer we'll use?).
         */
        let bytecountm = bytecount as TmSize;
        if bytecountm as u64 != bytecount {
            tiff_error_ext_r!(MODULE, "Integer overflow");
            return 0;
        }
        if bytecountm > tif.tif_rawdatasize {
            tif.tif_dir.td_curtile = NOTILE;
            if (tif.tif_flags & TIFF_MYBUFFER) == 0 {
                tiff_error_ext_r!(MODULE, "Data buffer too small to hold tile {}", tile);
                return 0;
            }
        }
        if (tif.tif_flags & TIFF_BUFFERMMAP) != 0 {
            tif.tif_dir.td_curtile = NOTILE;
            tif.tif_rawdata = Vec::new();
            tif.tif_rawdatasize = 0;
            tif.tif_flags &= !TIFF_BUFFERMMAP;
        }

        if tiff_read_raw_strip_or_tile2(tif, tile, false, bytecountm, MODULE) != bytecountm {
            return 0;
        }

        tif.tif_rawdataoff = 0;
        tif.tif_rawdataloaded = bytecountm;

        if !tif.tif_rawdata.is_empty()
            && !tif.is_fill_order(tif.tif_dir.td_fillorder)
            && (tif.tif_flags & TIFF_NOBITREV) == 0
        {
            let n = tif.tif_rawdataloaded;
            tiff_reverse_bits(&mut tif.tif_rawdata, n);
        }
    }
    tiff_start_tile(tif, tile)
}

/// Translation of `TIFFStartStrip()`: Set state to appear as if a strip
/// has just been read in.
fn tiff_start_strip(tif: &mut Tiff<'_>, strip: u32) -> i32 {
    if (tif.tif_flags & TIFF_CODERSETUP) == 0 {
        if (tif.tif_setupdecode)(tif) == 0 {
            return 0;
        }
        tif.tif_flags |= TIFF_CODERSETUP;
    }
    if tif.tif_dir.td_stripsperimage == 0 {
        tiff_error_ext_r!("TIFFStartStrip", "Zero strips per image");
        return 0;
    }
    let td = &mut tif.tif_dir;
    td.td_curstrip = strip;
    td.td_row = (strip % td.td_stripsperimage).wrapping_mul(td.td_rowsperstrip);
    tif.tif_flags &= !TIFF_BUF4WRITE;

    if (tif.tif_flags & TIFF_NOREADRAW) != 0 {
        tif.tif_rawcp = 0;
        tif.tif_rawcc = 0;
    } else {
        tif.tif_rawcp = 0;
        if tif.tif_rawdataloaded > 0 {
            tif.tif_rawcc = tif.tif_rawdataloaded;
        } else {
            tif.tif_rawcc = tiff_get_strile_byte_count(tif, strip) as TmSize;
        }
    }
    let plane = (strip / tif.tif_dir.td_stripsperimage) as u16;
    if (tif.tif_predecode)(tif, plane) == 0 {
        /* Needed for example for scanline access, if tif_predecode */
        /* fails, and we try to read the same strip again. Without invalidating
         */
        /* tif_curstrip, we'd call tif_decoderow() on a possibly invalid */
        /* codec state. */
        tif.tif_dir.td_curstrip = NOSTRIP;
        return 0;
    }
    1
}

/// Translation of `TIFFStartTile()`: Set state to appear as if a tile has
/// just been read in.
fn tiff_start_tile(tif: &mut Tiff<'_>, tile: u32) -> i32 {
    const MODULE: &str = "TIFFStartTile";

    if (tif.tif_flags & TIFF_CODERSETUP) == 0 {
        if (tif.tif_setupdecode)(tif) == 0 {
            return 0;
        }
        tif.tif_flags |= TIFF_CODERSETUP;
    }
    tif.tif_dir.td_curtile = tile;
    if tif.tif_dir.td_tilewidth == 0 {
        tiff_error_ext_r!(MODULE, "Zero tilewidth");
        return 0;
    }
    let td = &tif.tif_dir;
    let mut howmany32 = tiff_howmany_32(td.td_imagewidth, td.td_tilewidth);
    if howmany32 == 0 {
        tiff_error_ext_r!(MODULE, "Zero tiles");
        return 0;
    }
    tif.tif_dir.td_row = (tile % howmany32).wrapping_mul(tif.tif_dir.td_tilelength);
    let td = &tif.tif_dir;
    howmany32 = tiff_howmany_32(td.td_imagelength, td.td_tilelength);
    if howmany32 == 0 {
        tiff_error_ext_r!(MODULE, "Zero tiles");
        return 0;
    }
    tif.tif_dir.td_col = (tile % howmany32).wrapping_mul(tif.tif_dir.td_tilewidth);
    tif.tif_flags &= !TIFF_BUF4WRITE;
    if (tif.tif_flags & TIFF_NOREADRAW) != 0 {
        tif.tif_rawcp = 0;
        tif.tif_rawcc = 0;
    } else {
        tif.tif_rawcp = 0;
        if tif.tif_rawdataloaded > 0 {
            tif.tif_rawcc = tif.tif_rawdataloaded;
        } else {
            tif.tif_rawcc = tiff_get_strile_byte_count(tif, tile) as TmSize;
        }
    }
    let plane = (tile / tif.tif_dir.td_stripsperimage.max(1)) as u16;
    (tif.tif_predecode)(tif, plane)
}

/// Translation of `TIFFCheckRead()`.
fn tiff_check_read(tif: &Tiff<'_>, tiles: bool) -> i32 {
    if tif.tif_mode == O_WRONLY {
        tiff_error_ext_r!(tif.tif_name, "File not open for reading");
        return 0;
    }
    if tiles ^ tif.is_tiled() {
        tiff_error_ext_r!(
            tif.tif_name,
            "{}",
            if tiles {
                "Can not read tiles from a striped image"
            } else {
                "Can not read scanlines from a tiled image"
            }
        );
        return 0;
    }
    1
}

/// `(*tif->tif_postdecode)(tif, buf, cc)`: `_TIFFNoPostDecode()`,
/// `_TIFFSwab16BitData()`, `_TIFFSwab24BitData()`, `_TIFFSwab32BitData()`
/// or `_TIFFSwab64BitData()`.
pub(crate) fn tiff_post_decode(tif: &mut Tiff<'_>, buf: &mut [u8], cc: TmSize) {
    match tif.tif_postdecode {
        TIFFPostMethod::NoPostDecode => {}
        TIFFPostMethod::Swab16BitData => tiff_swab_array_of_short(buf, cc / 2),
        TIFFPostMethod::Swab24BitData => tiff_swab_array_of_triples(buf, cc / 3),
        TIFFPostMethod::Swab32BitData => tiff_swab_array_of_long(buf, cc / 4),
        TIFFPostMethod::Swab64BitData => tiff_swab_array_of_double(buf, cc / 8),
    }
}
