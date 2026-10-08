// Rust translation of libtiff/tif_dumpmode.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! "Null" Compression Algorithm Support. (The encoder is left out.)

use super::tif_error::tiff_error_ext_r;
use super::tiffiop::{Tiff, TmSize, TIFF_TMSIZE_T_MAX};

/// Translation of `DumpFixupTags()`.
fn dump_fixup_tags(_tif: &mut Tiff<'_>) -> i32 {
    1
}

/// Translation of `DumpModeDecode()`: Decode a hunk of pixels.
fn dump_mode_decode(tif: &mut Tiff<'_>, buf: &mut [u8], cc: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "DumpModeDecode";
    if tif.tif_rawcc < cc {
        tiff_error_ext_r!(
            MODULE,
            "Not enough data for scanline {}, expected a request for at most {} bytes, got a request for {} bytes",
            tif.tif_dir.td_row,
            tif.tif_rawcc,
            cc
        );
        return 0;
    }
    /*
     * Avoid copy if client has setup raw
     * data buffer to avoid extra copy.
     */
    let n = cc.max(0) as usize;
    let src = tif.rawcp();
    let n = n.min(src.len()).min(buf.len());
    buf[..n].copy_from_slice(&src[..n]);
    tif.tif_rawcp += cc as usize;
    tif.tif_rawcc -= cc;
    1
}

/// Translation of `DumpModeSeek()`: Seek forwards nrows in the current
/// strip.
fn dump_mode_seek(tif: &mut Tiff<'_>, nrows: u32) -> i32 {
    if nrows > 0 && tif.tif_dir.td_scanlinesize > TIFF_TMSIZE_T_MAX / nrows as TmSize {
        tiff_error_ext_r!("DumpModeSeek", "Integer overflow computing seek size");
        return 0;
    }
    let seek_size = nrows as TmSize * tif.tif_dir.td_scanlinesize;
    if seek_size > tif.tif_rawcc {
        tiff_error_ext_r!("DumpModeSeek", "Seek beyond end of raw data buffer");
        return 0;
    }
    tif.tif_rawcp += seek_size as usize;
    tif.tif_rawcc -= seek_size;
    1
}

/// Translation of `TIFFInitDumpMode()`: Initialize dump mode.
pub(crate) fn tiff_init_dump_mode(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    tif.tif_fixuptags = dump_fixup_tags;
    tif.tif_decoderow = dump_mode_decode;
    tif.tif_decodestrip = dump_mode_decode;
    tif.tif_decodetile = dump_mode_decode;
    tif.tif_seek = dump_mode_seek;
    1
}
