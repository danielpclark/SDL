// Rust translation of libtiff/tif_compress.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Compression Scheme Configuration Support. (The encoding methods are
//! left out: the file is only read.)

use super::tif_codec::{not_configured, tiff_find_codec};
use super::tif_error::tiff_error_ext_r;
use super::tiff::COMPRESSION_NONE;
use super::tiffiop::{Tiff, TmSize, TIFF_NOBITREV, TIFF_NOREADRAW};

/// Translation of `TIFFNoDecode()`.
fn tiff_no_decode(tif: &mut Tiff<'_>, method: &str) -> i32 {
    let c = tiff_find_codec(tif.tif_dir.td_compression);
    if let Some(c) = c {
        tiff_error_ext_r!(
            tif.tif_name,
            "{} {} decoding is not implemented",
            c.name,
            method
        );
    } else {
        tiff_error_ext_r!(
            tif.tif_name,
            "Compression scheme {} {} decoding is not implemented",
            tif.tif_dir.td_compression,
            method
        );
    }
    0
}

/// Translation of `_TIFFNoFixupTags()`.
fn _tiff_no_fixup_tags(_tif: &mut Tiff<'_>) -> i32 {
    1
}

/// Translation of `_TIFFNoRowDecode()`.
pub(crate) fn _tiff_no_row_decode(tif: &mut Tiff<'_>, _pp: &mut [u8], _cc: TmSize, _s: u16) -> i32 {
    tiff_no_decode(tif, "scanline")
}

/// Translation of `_TIFFNoStripDecode()`.
pub(crate) fn _tiff_no_strip_decode(
    tif: &mut Tiff<'_>,
    _pp: &mut [u8],
    _cc: TmSize,
    _s: u16,
) -> i32 {
    tiff_no_decode(tif, "strip")
}

/// Translation of `_TIFFNoTileDecode()`.
pub(crate) fn _tiff_no_tile_decode(
    tif: &mut Tiff<'_>,
    _pp: &mut [u8],
    _cc: TmSize,
    _s: u16,
) -> i32 {
    tiff_no_decode(tif, "tile")
}

/// Translation of `_TIFFNoSeek()`.
pub(crate) fn _tiff_no_seek(tif: &mut Tiff<'_>, _off: u32) -> i32 {
    tiff_error_ext_r!(
        tif.tif_name,
        "Compression algorithm does not support random access"
    );
    0
}

/// Translation of `_TIFFNoPreCode()`.
pub(crate) fn _tiff_no_pre_code(_tif: &mut Tiff<'_>, _s: u16) -> i32 {
    1
}

/// Translation of `_TIFFtrue()`.
fn _tiff_true(_tif: &mut Tiff<'_>) -> i32 {
    1
}

/// Translation of `_TIFFvoid()`.
pub(crate) fn _tiff_void(_tif: &mut Tiff<'_>) {}

/// Translation of `_TIFFDefaultGetMaxCompressionRatio()`.
fn _tiff_default_get_max_compression_ratio(_tif: &Tiff<'_>) -> u64 {
    0 /* unknown */
}

/// Translation of `_TIFFGetMaxCompressionRatioOne()`.
fn _tiff_get_max_compression_ratio_one(_tif: &Tiff<'_>) -> u64 {
    1 /* no compression */
}

/// Translation of `_TIFFSetDefaultCompressionState()`.
pub(crate) fn _tiff_set_default_compression_state(tif: &mut Tiff<'_>) {
    tif.tif_fixuptags = _tiff_no_fixup_tags;
    tif.tif_decodestatus = 1;
    tif.tif_setupdecode = _tiff_true;
    tif.tif_predecode = _tiff_no_pre_code;
    tif.tif_decoderow = _tiff_no_row_decode;
    tif.tif_decodestrip = _tiff_no_strip_decode;
    tif.tif_decodetile = _tiff_no_tile_decode;
    tif.tif_encodestatus = 1;
    tif.tif_close = _tiff_void;
    tif.tif_seek = _tiff_no_seek;
    tif.tif_cleanup = _tiff_void;
    tif.tif_getmaxcompressionratio = _tiff_default_get_max_compression_ratio;
    tif.tif_flags &= !(TIFF_NOBITREV | TIFF_NOREADRAW);
}

/// Translation of `TIFFSetCompressionScheme()`.
pub(crate) fn tiff_set_compression_scheme(tif: &mut Tiff<'_>, scheme: i32) -> i32 {
    let c = tiff_find_codec(scheme as u16);

    _tiff_set_default_compression_state(tif);
    if scheme == COMPRESSION_NONE as i32 {
        tif.tif_getmaxcompressionratio = _tiff_get_max_compression_ratio_one;
    }
    /*
     * Don't treat an unknown compression scheme as an error.
     * This permits applications to open files with data that
     * the library does not have builtin support for, but which
     * may still be meaningful.
     */
    match c {
        Some(c) => match c.init {
            Some(init) => init(tif, scheme),
            None => not_configured(tif, scheme),
        },
        None => 1,
    }
}

/// Translation of `TIFFGetMaxCompressionRatio()`.
pub(crate) fn tiff_get_max_compression_ratio(tif: &Tiff<'_>) -> u64 {
    (tif.tif_getmaxcompressionratio)(tif)
}
