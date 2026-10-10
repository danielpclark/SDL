// Rust translation of pngpriv.h (and the settings of pnglibconf.h, as
// libpng's CMake build makes it) from libpng 1.6.59.
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngpriv.h: private declarations, used internally by libpng; and the
//! pnglibconf.h values of the default configuration.

/* pnglibconf.h */

pub(crate) const PNG_GAMMA_THRESHOLD_FIXED: i32 = 5000;
pub(crate) const PNG_INFLATE_BUF_SIZE: usize = 1024;
pub(crate) const PNG_USER_CHUNK_CACHE_MAX: u32 = 1000;
pub(crate) const PNG_USER_CHUNK_MALLOC_MAX: usize = 8000000;
pub(crate) const PNG_USER_HEIGHT_MAX: u32 = 1000000;
pub(crate) const PNG_USER_WIDTH_MAX: u32 = 1000000;
pub(crate) const PNG_ZBUF_SIZE: u32 = 8192;
pub(crate) const PNG_IDAT_READ_SIZE: u32 = PNG_ZBUF_SIZE;
pub(crate) const PNG_TEXT_Z_DEFAULT_COMPRESSION: i32 = -1;
pub(crate) const PNG_TEXT_Z_DEFAULT_STRATEGY: i32 = 0;
pub(crate) const PNG_Z_DEFAULT_COMPRESSION: i32 = -1;
pub(crate) const PNG_Z_DEFAULT_NOFILTER_STRATEGY: i32 = 0;
pub(crate) const PNG_Z_DEFAULT_STRATEGY: i32 = 1;

/* pngstruct.h */

/* zlib.h declares a magic type 'uInt' that limits the amount of data that zlib
 * can handle at once.  This type need be no larger than 16 bits (so maximum of
 * 65535), this define allows us to discover how big it is, but limited by the
 * maximum for size_t.  The value can be overridden in a library build
 * (pngusr.h, or set it in CPPFLAGS) and it works to set it to a considerably
 * lower value (e.g. 255 works).  A lower value may help memory usage (slightly)
 * and may even improve performance on some systems (and degrade it on others.)
 */
pub(crate) const ZLIB_IO_MAX: u32 = u32::MAX;

/* pngpriv.h */

/* Flags for the transformations the PNG library does on the image data */
pub(crate) const PNG_HAVE_IDAT: u32 = 0x04;
pub(crate) const PNG_HAVE_IEND: u32 = 0x10;
pub(crate) const PNG_HAVE_CHUNK_HEADER: u32 = 0x100;
pub(crate) const PNG_WROTE_tIME: u32 = 0x200;
pub(crate) const PNG_WROTE_INFO_BEFORE_PLTE: u32 = 0x400;
pub(crate) const PNG_BACKGROUND_IS_GRAY: u32 = 0x800;
pub(crate) const PNG_HAVE_PNG_SIGNATURE: u32 = 0x1000;
pub(crate) const PNG_HAVE_CHUNK_AFTER_IDAT: u32 = 0x2000; /* Have another chunk after IDAT */
pub(crate) const PNG_WROTE_eXIf: u32 = 0x4000;
pub(crate) const PNG_IS_READ_STRUCT: u32 = 0x8000; /* Else is a write struct */

/* Flags for the transformations the PNG library does on the image data */
pub(crate) const PNG_BGR: u32 = 0x0001;
pub(crate) const PNG_INTERLACE: u32 = 0x0002;
pub(crate) const PNG_PACK: u32 = 0x0004;
pub(crate) const PNG_SHIFT: u32 = 0x0008;
pub(crate) const PNG_SWAP_BYTES: u32 = 0x0010;
pub(crate) const PNG_INVERT_MONO: u32 = 0x0020;
pub(crate) const PNG_QUANTIZE: u32 = 0x0040;
pub(crate) const PNG_COMPOSE: u32 = 0x0080; /* Was PNG_BACKGROUND */
pub(crate) const PNG_BACKGROUND_EXPAND: u32 = 0x0100;
pub(crate) const PNG_EXPAND_16: u32 = 0x0200; /* Added to libpng 1.5.2 */
pub(crate) const PNG_16_TO_8: u32 = 0x0400; /* Becomes 'chop' in 1.5.4 */
pub(crate) const PNG_RGBA: u32 = 0x0800;
pub(crate) const PNG_EXPAND: u32 = 0x1000;
pub(crate) const PNG_GAMMA: u32 = 0x2000;
pub(crate) const PNG_GRAY_TO_RGB: u32 = 0x4000;
pub(crate) const PNG_FILLER: u32 = 0x8000;
pub(crate) const PNG_PACKSWAP: u32 = 0x10000;
pub(crate) const PNG_SWAP_ALPHA: u32 = 0x20000;
pub(crate) const PNG_STRIP_ALPHA: u32 = 0x40000;
pub(crate) const PNG_INVERT_ALPHA: u32 = 0x80000;
pub(crate) const PNG_USER_TRANSFORM: u32 = 0x100000;
pub(crate) const PNG_RGB_TO_GRAY_ERR: u32 = 0x200000;
pub(crate) const PNG_RGB_TO_GRAY_WARN: u32 = 0x400000;
pub(crate) const PNG_RGB_TO_GRAY: u32 = 0x600000; /* two bits, RGB_TO_GRAY_ERR|WARN */
pub(crate) const PNG_ENCODE_ALPHA: u32 = 0x800000; /* Added to libpng-1.5.4 */
pub(crate) const PNG_ADD_ALPHA: u32 = 0x1000000; /* Added to libpng-1.2.7 */
pub(crate) const PNG_EXPAND_tRNS: u32 = 0x2000000; /* Added to libpng-1.2.9 */
pub(crate) const PNG_SCALE_16_TO_8: u32 = 0x4000000; /* Added to libpng-1.5.4 */

/* Flags for the png_ptr->flags rather than declaring a byte for each one */
pub(crate) const PNG_FLAG_ZLIB_CUSTOM_STRATEGY: u32 = 0x0001;
pub(crate) const PNG_FLAG_ZSTREAM_INITIALIZED: u32 = 0x0002; /* Added to libpng-1.6.0 */
pub(crate) const PNG_FLAG_ZSTREAM_ENDED: u32 = 0x0008; /* Added to libpng-1.6.0 */
pub(crate) const PNG_FLAG_ROW_INIT: u32 = 0x0040;
pub(crate) const PNG_FLAG_FILLER_AFTER: u32 = 0x0080;
pub(crate) const PNG_FLAG_CRC_ANCILLARY_USE: u32 = 0x0100;
pub(crate) const PNG_FLAG_CRC_ANCILLARY_NOWARN: u32 = 0x0200;
pub(crate) const PNG_FLAG_CRC_CRITICAL_USE: u32 = 0x0400;
pub(crate) const PNG_FLAG_CRC_CRITICAL_IGNORE: u32 = 0x0800;
pub(crate) const PNG_FLAG_OPTIMIZE_ALPHA: u32 = 0x2000; /* Added to libpng-1.5.4 */
pub(crate) const PNG_FLAG_DETECT_UNINITIALIZED: u32 = 0x4000; /* Added to libpng-1.5.4 */
pub(crate) const PNG_FLAG_LIBRARY_MISMATCH: u32 = 0x20000;
pub(crate) const PNG_FLAG_STRIP_ERROR_TEXT: u32 = 0x80000;
pub(crate) const PNG_FLAG_BENIGN_ERRORS_WARN: u32 = 0x100000; /* Added to libpng-1.4.0 */
pub(crate) const PNG_FLAG_APP_WARNINGS_WARN: u32 = 0x200000; /* Added to libpng-1.6.0 */
pub(crate) const PNG_FLAG_APP_ERRORS_WARN: u32 = 0x400000; /* Added to libpng-1.6.0 */

pub(crate) const PNG_FLAG_CRC_ANCILLARY_MASK: u32 =
    PNG_FLAG_CRC_ANCILLARY_USE | PNG_FLAG_CRC_ANCILLARY_NOWARN;

pub(crate) const PNG_FLAG_CRC_CRITICAL_MASK: u32 =
    PNG_FLAG_CRC_CRITICAL_USE | PNG_FLAG_CRC_CRITICAL_IGNORE;

/* png.h: the mng_features_permitted flags */
pub(crate) const PNG_FLAG_MNG_EMPTY_PLTE: u32 = 0x01;
pub(crate) const PNG_FLAG_MNG_FILTER_64: u32 = 0x04;

/// `PNG_ROWBYTES`: added to libpng-1.2.6 JB
pub(crate) const fn png_rowbytes(pixel_bits: u32, width: usize) -> usize {
    if pixel_bits >= 8 {
        width.wrapping_mul((pixel_bits as usize) >> 3)
    } else {
        (width.wrapping_mul(pixel_bits as usize).wrapping_add(7)) >> 3
    }
}

/// `PNG_TRAILBITS`: this returns the number of trailing bits in the last
/// byte of a row, 0 if the last byte is completely full of pixels.  It is,
/// in principle, (pixel_bits x width) % 8, but that would overflow for
/// large 'width'.  The second macro is the same except that it returns
/// the number of unused bits in the last byte; (8-TRAILBITS) % 8.
pub(crate) const fn png_trailbits(pixel_bits: u32, width: u32) -> u32 {
    (pixel_bits * (width % 8)) % 8
}

/// `PNG_PADBITS`
pub(crate) const fn png_padbits(pixel_bits: u32, width: u32) -> u32 {
    (8 - png_trailbits(pixel_bits, width)) % 8
}

/// `PNG_U32`: constants for known chunk types.
pub(crate) const fn png_u32(b1: u8, b2: u8, b3: u8, b4: u8) -> u32 {
    ((b1 as u32) << 24) | ((b2 as u32) << 16) | ((b3 as u32) << 8) | (b4 as u32)
}

pub(crate) const png_IDAT: u32 = png_u32(73, 68, 65, 84);
pub(crate) const png_IEND: u32 = png_u32(73, 69, 78, 68);
pub(crate) const png_IHDR: u32 = png_u32(73, 72, 68, 82);
pub(crate) const png_PLTE: u32 = png_u32(80, 76, 84, 69);
pub(crate) const png_acTL: u32 = png_u32(97, 99, 84, 76); /* PNGv3: APNG */
pub(crate) const png_bKGD: u32 = png_u32(98, 75, 71, 68);
pub(crate) const png_cHRM: u32 = png_u32(99, 72, 82, 77);
pub(crate) const png_cICP: u32 = png_u32(99, 73, 67, 80); /* PNGv3 */
pub(crate) const png_cLLI: u32 = png_u32(99, 76, 76, 73); /* PNGv3 */
pub(crate) const png_eXIf: u32 = png_u32(101, 88, 73, 102); /* registered July 2017 */
pub(crate) const png_fcTL: u32 = png_u32(102, 99, 84, 76); /* PNGv3: APNG */
pub(crate) const png_fdAT: u32 = png_u32(102, 100, 65, 84); /* PNGv3: APNG */
pub(crate) const png_gAMA: u32 = png_u32(103, 65, 77, 65);
pub(crate) const png_hIST: u32 = png_u32(104, 73, 83, 84);
pub(crate) const png_iCCP: u32 = png_u32(105, 67, 67, 80);
pub(crate) const png_iTXt: u32 = png_u32(105, 84, 88, 116);
pub(crate) const png_mDCV: u32 = png_u32(109, 68, 67, 86); /* PNGv3 */
pub(crate) const png_oFFs: u32 = png_u32(111, 70, 70, 115);
pub(crate) const png_pCAL: u32 = png_u32(112, 67, 65, 76);
pub(crate) const png_pHYs: u32 = png_u32(112, 72, 89, 115);
pub(crate) const png_sBIT: u32 = png_u32(115, 66, 73, 84);
pub(crate) const png_sCAL: u32 = png_u32(115, 67, 65, 76);
pub(crate) const png_sPLT: u32 = png_u32(115, 80, 76, 84);
pub(crate) const png_sRGB: u32 = png_u32(115, 82, 71, 66);
pub(crate) const png_tEXt: u32 = png_u32(116, 69, 88, 116);
pub(crate) const png_tIME: u32 = png_u32(116, 73, 77, 69);
pub(crate) const png_tRNS: u32 = png_u32(116, 82, 78, 83);
pub(crate) const png_zTXt: u32 = png_u32(122, 84, 88, 116);

/// `PNG_CHUNK_FROM_STRING`: the 32-bit chunk name of a 4-byte string.
pub(crate) fn png_chunk_from_string(s: &[u8]) -> u32 {
    png_u32(s[0], s[1], s[2], s[3])
}

/// `PNG_STRING_FROM_CHUNK`
pub(crate) fn png_string_from_chunk(c: u32) -> [u8; 4] {
    c.to_be_bytes()
}

/* Find out if a chunk is ancillary, critical, private or reserved */
/// `PNG_CHUNK_ANCILLARY`
pub(crate) const fn png_chunk_ancillary(c: u32) -> u32 {
    1 & (c >> 29)
}
/// `PNG_CHUNK_CRITICAL`
pub(crate) const fn png_chunk_critical(c: u32) -> bool {
    png_chunk_ancillary(c) == 0
}

/* An internal value for png_zstream_error: the zlib return codes it knows */
pub(crate) const PNG_UNEXPECTED_ZLIB_RETURN: i32 = -7;

/// `png_index` (pngstruct.h): chunk index values (`PNG_KNOWN_CHUNKS`
/// order); `PNG_INDEX_unknown` is also a count of the known chunks.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PngIndex {
    IHDR = 0,
    PLTE = 1,
    IDAT = 2,
    IEND = 3,
    acTL = 4,
    bKGD = 5,
    cHRM = 6,
    cICP = 7,
    cLLI = 8,
    eXIf = 9,
    fcTL = 10,
    fdAT = 11,
    gAMA = 12,
    hIST = 13,
    iCCP = 14,
    iTXt = 15,
    mDCV = 16,
    oFFs = 17,
    pCAL = 18,
    pHYs = 19,
    sBIT = 20,
    sCAL = 21,
    sPLT = 22,
    sRGB = 23,
    tEXt = 24,
    tIME = 25,
    tRNS = 26,
    zTXt = 27,
    unknown = 28,
}

/// `png_chunk_flag_from_index`: the flag corresponding to the given
/// png_index enum value.
pub(crate) const fn png_chunk_flag_from_index(i: PngIndex) -> u32 {
    0x80000000u32 >> (31 - i as u32)
}

/// `png_chunk_max`: the size of the largest chunk libpng will allocate.
pub(crate) fn png_chunk_max(png_ptr: &super::pngstruct::PngStruct<'_, '_>) -> usize {
    png_ptr.user_chunk_malloc_max
}
