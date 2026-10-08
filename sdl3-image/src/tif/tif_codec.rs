// Rust translation of libtiff/tif_codec.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Builtin Compression Scheme Configuration Support.
//!
//! As SDL_image configures its vendored libtiff: CCITT (RLE, RLE/W, Group
//! 3 and 4), PackBits, LZW, ThunderScan, NeXT and SGI Log (LogLuv) are
//! built in; JPEG, Old-style JPEG, JBIG, Deflate (SDL_image's build leaves
//! zlib out), PixarLog, LZMA, ZSTD, WebP and LERC are not configured (an
//! `init` of `None` is the C's `NotConfigured()`).

use super::tif_dumpmode::tiff_init_dump_mode;
use super::tif_error::tiff_error_ext_r;
use super::tif_fax3::{
    tiff_init_ccitt_fax3, tiff_init_ccitt_fax4, tiff_init_ccitt_rle, tiff_init_ccitt_rlew,
};
use super::tif_luv::tiff_init_sg_log;
use super::tif_lzw::tiff_init_lzw;
use super::tif_next::tiff_init_next;
use super::tif_packbits::tiff_init_pack_bits;
use super::tif_thunder::tiff_init_thunder_scan;
use super::tiff::*;
use super::tiffiop::Tiff;

/// `TIFFInitMethod`
pub(crate) type TIFFInitMethod = fn(&mut Tiff<'_>, i32) -> i32;

/// Translation of `TIFFCodec`.
pub(crate) struct TIFFCodec {
    pub(crate) name: &'static str,
    pub(crate) scheme: u16,
    /// `None` for `NotConfigured()`.
    pub(crate) init: Option<TIFFInitMethod>,
}

/*
 * Compression schemes statically built into the library.
 */
pub(crate) static _TIFF_BUILTIN_CODECS: &[TIFFCodec] = &[
    TIFFCodec { name: "None", scheme: COMPRESSION_NONE, init: Some(tiff_init_dump_mode) },
    TIFFCodec { name: "LZW", scheme: COMPRESSION_LZW, init: Some(tiff_init_lzw) },
    TIFFCodec { name: "PackBits", scheme: COMPRESSION_PACKBITS, init: Some(tiff_init_pack_bits) },
    TIFFCodec {
        name: "ThunderScan",
        scheme: COMPRESSION_THUNDERSCAN,
        init: Some(tiff_init_thunder_scan),
    },
    TIFFCodec { name: "NeXT", scheme: COMPRESSION_NEXT, init: Some(tiff_init_next) },
    TIFFCodec { name: "JPEG", scheme: COMPRESSION_JPEG, init: None },
    TIFFCodec { name: "Old-style JPEG", scheme: COMPRESSION_OJPEG, init: None },
    TIFFCodec { name: "CCITT RLE", scheme: COMPRESSION_CCITTRLE, init: Some(tiff_init_ccitt_rle) },
    TIFFCodec {
        name: "CCITT RLE/W",
        scheme: COMPRESSION_CCITTRLEW,
        init: Some(tiff_init_ccitt_rlew),
    },
    TIFFCodec {
        name: "CCITT Group 3",
        scheme: COMPRESSION_CCITTFAX3,
        init: Some(tiff_init_ccitt_fax3),
    },
    TIFFCodec {
        name: "CCITT Group 4",
        scheme: COMPRESSION_CCITTFAX4,
        init: Some(tiff_init_ccitt_fax4),
    },
    TIFFCodec { name: "ISO JBIG", scheme: COMPRESSION_JBIG, init: None },
    TIFFCodec { name: "Deflate", scheme: COMPRESSION_DEFLATE, init: None },
    TIFFCodec { name: "AdobeDeflate", scheme: COMPRESSION_ADOBE_DEFLATE, init: None },
    TIFFCodec { name: "PixarLog", scheme: COMPRESSION_PIXARLOG, init: None },
    TIFFCodec { name: "SGILog", scheme: COMPRESSION_SGILOG, init: Some(tiff_init_sg_log) },
    TIFFCodec { name: "SGILog24", scheme: COMPRESSION_SGILOG24, init: Some(tiff_init_sg_log) },
    TIFFCodec { name: "LZMA", scheme: COMPRESSION_LZMA, init: None },
    TIFFCodec { name: "ZSTD", scheme: COMPRESSION_ZSTD, init: None },
    TIFFCodec { name: "WEBP", scheme: COMPRESSION_WEBP, init: None },
    TIFFCodec { name: "LERC", scheme: COMPRESSION_LERC, init: None },
];

/// Translation of `_notConfigured()`.
fn _not_configured(tif: &mut Tiff<'_>) -> i32 {
    let c = tiff_find_codec(tif.tif_dir.td_compression);
    let compression_code = format!("{}", tif.tif_dir.td_compression);

    tiff_error_ext_r!(
        tif.tif_name,
        "{} compression support is not configured",
        c.map_or(compression_code.as_str(), |c| c.name)
    );
    0
}

/// Translation of `NotConfigured()`.
pub(crate) fn not_configured(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    tif.tif_fixuptags = _not_configured;
    tif.tif_decodestatus = 0;
    tif.tif_setupdecode = _not_configured;
    tif.tif_encodestatus = 0;
    1
}

/// Translation of `TIFFIsCODECConfigured()`: Check whether we have
/// working codec for the specific coding scheme.
///
/// @return returns 1 if the codec is configured and working. Otherwise
/// 0 will be returned.
pub(crate) fn tiff_is_codec_configured(scheme: u16) -> i32 {
    match tiff_find_codec(scheme) {
        None => 0,
        Some(codec) => codec.init.is_some() as i32,
    }
}

/// Translation of `TIFFFindCODEC()` (no codec is registered).
pub(crate) fn tiff_find_codec(scheme: u16) -> Option<&'static TIFFCodec> {
    _TIFF_BUILTIN_CODECS.iter().find(|c| c.scheme == scheme)
}
