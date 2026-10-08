// Rust translation of libtiff/tif_dirinfo.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Core Directory Tag Support.
//!
//! The tag table of the image directories (the EXIF and GPS tables, and
//! the obsolete `TIFFFieldInfo` interfaces, are left out). A handle's
//! table of registered tags holds copies of the descriptors, kept sorted
//! by tag; the found-field cache (`tif_foundfield`) is left out, as it
//! only saves searches.

use std::borrow::Cow;

use super::tif_codec::tiff_is_codec_configured;
use super::tif_dir::*;
use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tiff::*;
use super::tiffio::{field, TIFFField, FIELD_CUSTOM, TIFF_ANY, TIFF_VARIABLE2};
use super::tiffiop::Tiff;

/// `tiffFieldArray`: the tag table, sorted by tag.
pub(crate) fn _tiff_get_fields() -> &'static [TIFFField] {
    &TIFF_FIELDS
}

/// Translation of `_TIFFSetupFields()`.
pub(crate) fn _tiff_setup_fields(tif: &mut Tiff<'_>, fieldarray: &[TIFFField]) {
    // (the anonymous fields are dropped with the table)
    tif.tif_fields = Vec::new();
    if _tiff_merge_fields(tif, fieldarray) == 0 {
        tiff_error_ext_r!("_TIFFSetupFields", "Setting up field info failed");
    }
}

/// Translation of `tagCompare()`.
fn tag_compare(ta: &TIFFField, tb: &TIFFField) -> std::cmp::Ordering {
    /* NB: be careful of return values for 16-bit platforms */
    if ta.field_tag != tb.field_tag {
        ta.field_tag.cmp(&tb.field_tag)
    } else if ta.field_type == TIFF_ANY {
        std::cmp::Ordering::Equal
    } else {
        tb.field_type.cmp(&ta.field_type)
    }
}

/// Translation of `_TIFFMergeFields()`.
pub(crate) fn _tiff_merge_fields(tif: &mut Tiff<'_>, info: &[TIFFField]) -> i32 {
    const MODULE: &str = "_TIFFMergeFields";
    const REASON: &str = "for fields array";

    if tif.tif_fields.try_reserve(info.len()).is_err() {
        tiff_error_ext_r!(
            tif.tif_name,
            "Failed to allocate memory for {} ({} elements of {} bytes each)",
            REASON,
            tif.tif_fields.len() + info.len(),
            std::mem::size_of::<usize>()
        );
        tif.tif_fields = Vec::new();
        tiff_error_ext_r!(MODULE, "Failed to allocate fields array");
        return 0;
    }

    for f in info {
        let fip = tiff_find_field(tif, f.field_tag, TIFF_ANY);

        /* only add definitions that aren't already present */
        if fip.is_none() {
            tif.tif_fields.push(f.clone());
        }
    }

    /* Sort the field info by tag number */
    tif.tif_fields.sort_by(tag_compare);

    info.len() as i32
}

/// Translation of `TIFFDataWidth()`: Return size of TIFFDataType within
/// TIFF-file in bytes
pub(crate) fn tiff_data_width(type_: TIFFDataType) -> i32 {
    match type_ {
        0 /* nothing */ | TIFF_BYTE | TIFF_ASCII | TIFF_SBYTE | TIFF_UNDEFINED => 1,
        TIFF_SHORT | TIFF_SSHORT => 2,
        TIFF_LONG | TIFF_SLONG | TIFF_FLOAT | TIFF_IFD => 4,
        TIFF_RATIONAL | TIFF_SRATIONAL | TIFF_DOUBLE | TIFF_LONG8 | TIFF_SLONG8 | TIFF_IFD8 => 8,
        _ => 0, /* will return 0 for unknown types */
    }
}

/// Translation of `TIFFFieldSetGetSize()`: Return internal storage size
/// of TIFFSetGetFieldType in bytes. TIFFSetField() and TIFFGetField() have
/// to provide the parameter accordingly.
pub(crate) fn tiff_field_set_get_size(fip: Option<&TIFFField>) -> i32 {
    /*
     * TIFFSetField() and TIFFGetField() must provide the parameter accordingly
     * to the definition of "set_get_field_type" of the tag definition in
     * dir_info.c. This function returns the data size for that purpose.
     *
     * Furthermore, this data size is also used for the internal storage,
     * even for TIFF_RATIONAL values for FIELD_CUSTOM, which are stored
     * internally as 4-byte float, but some of them should be stored internally
     * as 8-byte double, depending on the "set_get_field_type" _FLOAT_ or
     * _DOUBLE_.
     */
    let Some(fip) = fip else {
        return 0;
    };

    match fip.set_get_field_type {
        TIFF_SETGET_UNDEFINED | TIFF_SETGET_ASCII | TIFF_SETGET_C0_ASCII
        | TIFF_SETGET_C16_ASCII | TIFF_SETGET_C32_ASCII | TIFF_SETGET_OTHER => 1,
        TIFF_SETGET_UINT8 | TIFF_SETGET_SINT8 | TIFF_SETGET_C0_UINT8 | TIFF_SETGET_C0_SINT8
        | TIFF_SETGET_C16_UINT8 | TIFF_SETGET_C16_SINT8 | TIFF_SETGET_C32_UINT8
        | TIFF_SETGET_C32_SINT8 => 1,
        TIFF_SETGET_UINT16 | TIFF_SETGET_SINT16 | TIFF_SETGET_C0_UINT16
        | TIFF_SETGET_C0_SINT16 | TIFF_SETGET_C16_UINT16 | TIFF_SETGET_C16_SINT16
        | TIFF_SETGET_C32_UINT16 | TIFF_SETGET_C32_SINT16 => 2,
        TIFF_SETGET_INT | TIFF_SETGET_UINT32 | TIFF_SETGET_SINT32 | TIFF_SETGET_FLOAT
        | TIFF_SETGET_UINT16_PAIR | TIFF_SETGET_C0_UINT32 | TIFF_SETGET_C0_SINT32
        | TIFF_SETGET_C0_FLOAT | TIFF_SETGET_C16_UINT32 | TIFF_SETGET_C16_SINT32
        | TIFF_SETGET_C16_FLOAT | TIFF_SETGET_C32_UINT32 | TIFF_SETGET_C32_SINT32
        | TIFF_SETGET_C32_FLOAT => 4,
        TIFF_SETGET_UINT64 | TIFF_SETGET_SINT64 | TIFF_SETGET_DOUBLE | TIFF_SETGET_IFD8
        | TIFF_SETGET_C0_UINT64 | TIFF_SETGET_C0_SINT64 | TIFF_SETGET_C0_DOUBLE
        | TIFF_SETGET_C0_IFD8 | TIFF_SETGET_C16_UINT64 | TIFF_SETGET_C16_SINT64
        | TIFF_SETGET_C16_DOUBLE | TIFF_SETGET_C16_IFD8 | TIFF_SETGET_C32_UINT64
        | TIFF_SETGET_C32_SINT64 | TIFF_SETGET_C32_DOUBLE | TIFF_SETGET_C32_IFD8 => 8,
        _ => 0,
    }
} /*-- TIFFFieldSetGetSize() --- */

/// Translation of `TIFFFindField()`: the registered field of a tag (of
/// type `dt`, unless `TIFF_ANY`), as a copy. A binary search, as the C's
/// `bsearch()`.
pub(crate) fn tiff_find_field(tif: &Tiff<'_>, tag: u32, dt: TIFFDataType) -> Option<TIFFField> {
    /* NB: use sorted search (e.g. binary search) */
    let key = TIFFField {
        field_tag: tag,
        field_readcount: 0,
        field_writecount: 0,
        field_type: dt,
        field_anonymous: 0,
        set_get_field_type: TIFF_SETGET_UNDEFINED,
        field_bit: 0,
        field_oktochange: 0,
        field_passcount: 0,
        field_name: Cow::Borrowed(""),
    };
    tif.tif_fields
        .binary_search_by(|e| tag_compare(&key, e).reverse())
        .ok()
        .map(|i| tif.tif_fields[i].clone())
}

/// Translation of `TIFFFieldWithTag()`.
pub(crate) fn tiff_field_with_tag(tif: &Tiff<'_>, tag: u32) -> Option<TIFFField> {
    let fip = tiff_find_field(tif, tag, TIFF_ANY);
    if fip.is_none() {
        tiff_warning_ext_r!("TIFFFieldWithTag", "Warning, unknown tag 0x{:x}", tag);
    }
    fip
}

/// Translation of `_TIFFCreateAnonField()`.
pub(crate) fn _tiff_create_anon_field(tag: u32, field_type: TIFFDataType) -> TIFFField {
    let set_get_field_type = match field_type {
        TIFF_BYTE | TIFF_UNDEFINED => TIFF_SETGET_C32_UINT8,
        TIFF_ASCII => TIFF_SETGET_C32_ASCII,
        TIFF_SHORT => TIFF_SETGET_C32_UINT16,
        TIFF_LONG => TIFF_SETGET_C32_UINT32,
        TIFF_RATIONAL | TIFF_SRATIONAL | TIFF_FLOAT => TIFF_SETGET_C32_FLOAT,
        TIFF_SBYTE => TIFF_SETGET_C32_SINT8,
        TIFF_SSHORT => TIFF_SETGET_C32_SINT16,
        TIFF_SLONG => TIFF_SETGET_C32_SINT32,
        TIFF_DOUBLE => TIFF_SETGET_C32_DOUBLE,
        TIFF_IFD | TIFF_IFD8 => TIFF_SETGET_C32_IFD8,
        TIFF_LONG8 => TIFF_SETGET_C32_UINT64,
        TIFF_SLONG8 => TIFF_SETGET_C32_SINT64,
        _ => TIFF_SETGET_UNDEFINED, /* TIFF_NOTYPE */
    };
    TIFFField {
        field_tag: tag,
        field_readcount: TIFF_VARIABLE2,
        field_writecount: TIFF_VARIABLE2,
        field_type,
        field_anonymous: 1, /* indicate that this is an anonymous / unknown tag */
        set_get_field_type,
        field_bit: FIELD_CUSTOM,
        field_oktochange: 1,
        field_passcount: 1,
        /*
         * note that this name is a special sign to TIFFClose() and
         * _TIFFSetupFields() to free the field
         * Update:
         *   This special sign is replaced by fld->field_anonymous  flag.
         */
        field_name: Cow::Owned(format!("Tag {}", tag as i32)),
    }
}

/// Translation of `_TIFFCheckFieldIsValidForCodec()`.
pub(crate) fn _tiff_check_field_is_valid_for_codec(tif: &Tiff<'_>, tag: u32) -> i32 {
    /* Filter out non-codec specific tags */
    match tag {
        /* Shared tags */
        TIFFTAG_PREDICTOR
        /* JPEG tags */
        | TIFFTAG_JPEGTABLES
        /* OJPEG tags */
        | TIFFTAG_JPEGIFOFFSET
        | TIFFTAG_JPEGIFBYTECOUNT
        | TIFFTAG_JPEGQTABLES
        | TIFFTAG_JPEGDCTABLES
        | TIFFTAG_JPEGACTABLES
        | TIFFTAG_JPEGPROC
        | TIFFTAG_JPEGRESTARTINTERVAL
        /* CCITT* */
        | TIFFTAG_BADFAXLINES
        | TIFFTAG_CLEANFAXDATA
        | TIFFTAG_CONSECUTIVEBADFAXLINES
        | TIFFTAG_GROUP3OPTIONS
        | TIFFTAG_GROUP4OPTIONS
        /* LERC */
        | TIFFTAG_LERC_PARAMETERS => {}
        _ => return 1,
    }
    if tiff_is_codec_configured(tif.tif_dir.td_compression) == 0 {
        return 0;
    }
    /* Check if codec specific tags are allowed for the current
     * compression scheme (codec) */
    match tif.tif_dir.td_compression {
        COMPRESSION_LZW => {
            if tag == TIFFTAG_PREDICTOR {
                return 1;
            }
        }
        COMPRESSION_PACKBITS => {
            /* No codec-specific tags */
        }
        COMPRESSION_THUNDERSCAN => {
            /* No codec-specific tags */
        }
        COMPRESSION_NEXT => {
            /* No codec-specific tags */
        }
        COMPRESSION_JPEG => {
            if tag == TIFFTAG_JPEGTABLES {
                return 1;
            }
        }
        COMPRESSION_OJPEG => match tag {
            TIFFTAG_JPEGIFOFFSET
            | TIFFTAG_JPEGIFBYTECOUNT
            | TIFFTAG_JPEGQTABLES
            | TIFFTAG_JPEGDCTABLES
            | TIFFTAG_JPEGACTABLES
            | TIFFTAG_JPEGPROC
            | TIFFTAG_JPEGRESTARTINTERVAL => return 1,
            _ => {}
        },
        COMPRESSION_CCITTRLE | COMPRESSION_CCITTRLEW | COMPRESSION_CCITTFAX3
        | COMPRESSION_CCITTFAX4 => match tag {
            TIFFTAG_BADFAXLINES | TIFFTAG_CLEANFAXDATA | TIFFTAG_CONSECUTIVEBADFAXLINES => {
                return 1
            }
            TIFFTAG_GROUP3OPTIONS => {
                if tif.tif_dir.td_compression == COMPRESSION_CCITTFAX3 {
                    return 1;
                }
            }
            TIFFTAG_GROUP4OPTIONS => {
                if tif.tif_dir.td_compression == COMPRESSION_CCITTFAX4 {
                    return 1;
                }
            }
            _ => {}
        },
        COMPRESSION_JBIG => {
            /* No codec-specific tags */
        }
        COMPRESSION_DEFLATE | COMPRESSION_ADOBE_DEFLATE => {
            if tag == TIFFTAG_PREDICTOR {
                return 1;
            }
        }
        COMPRESSION_PIXARLOG => {
            if tag == TIFFTAG_PREDICTOR {
                return 1;
            }
        }
        COMPRESSION_SGILOG | COMPRESSION_SGILOG24 => {
            /* No codec-specific tags */
        }
        COMPRESSION_LZMA => {
            if tag == TIFFTAG_PREDICTOR {
                return 1;
            }
        }
        COMPRESSION_ZSTD => {
            if tag == TIFFTAG_PREDICTOR {
                return 1;
            }
        }
        COMPRESSION_LERC => {
            if tag == TIFFTAG_LERC_PARAMETERS {
                return 1;
            }
        }
        _ => {}
    }
    0
}

/*
 * NOTE: THIS ARRAY IS ASSUMED TO BE SORTED BY TAG.
 *
 * NOTE: The second field (field_readcount) and third field (field_writecount)
 *       sometimes use the values TIFF_VARIABLE (-1), TIFF_VARIABLE2 (-3)
 *       and TIFF_SPP (-2). The macros should be used but would throw off
 *       the formatting of the code, so please interpret the -1, -2 and -3
 *       values accordingly.
 */

/*--: Rational2Double: --
 * The Rational2Double upgraded libtiff functionality allows the definition and
 * achievement of true double-precision accuracy for TIFF tags of RATIONAL type
 * and field_bit=FIELD_CUSTOM using the set_get_field_type = TIFF_SETGET_DOUBLE.
 * Unfortunately, that changes the old implemented interface for TIFFGetField().
 * In order to keep the old TIFFGetField() interface behavior those tags have to
 * be redefined with set_get_field_type = TIFF_SETGET_FLOAT!
 *
 *  Rational custom arrays are already defined as _Cxx_FLOAT, thus can stay.
 *
 */

/// `tiffFields` (the `#if 0` parts left out, as the preprocessor leaves
/// them out).
#[rustfmt::skip]
static TIFF_FIELDS: [TIFFField; 153] = [
    field(TIFFTAG_SUBFILETYPE, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_SUBFILETYPE, 1, 0, "SubfileType"),
    field(TIFFTAG_OSUBFILETYPE, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UNDEFINED, FIELD_IGNORE, 1, 0, "OldSubfileType"),
    field(TIFFTAG_IMAGEWIDTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_IMAGEDIMENSIONS, 0, 0, "ImageWidth"),
    field(TIFFTAG_IMAGELENGTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_IMAGEDIMENSIONS, 1, 0, "ImageLength"),
    field(TIFFTAG_BITSPERSAMPLE, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_BITSPERSAMPLE, 0, 0, "BitsPerSample"),
    field(TIFFTAG_COMPRESSION, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_COMPRESSION, 0, 0, "Compression"),
    field(TIFFTAG_PHOTOMETRIC, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_PHOTOMETRIC, 0, 0, "PhotometricInterpretation"),
    field(TIFFTAG_THRESHHOLDING, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_THRESHHOLDING, 1, 0, "Threshholding"),
    field(TIFFTAG_CELLWIDTH, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 1, 0, "CellWidth"),
    field(TIFFTAG_CELLLENGTH, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 1, 0, "CellLength"),
    field(TIFFTAG_FILLORDER, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_FILLORDER, 0, 0, "FillOrder"),
    field(TIFFTAG_DOCUMENTNAME, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "DocumentName"),
    field(TIFFTAG_IMAGEDESCRIPTION, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "ImageDescription"),
    field(TIFFTAG_MAKE, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "Make"),
    field(TIFFTAG_MODEL, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "Model"),
    field(TIFFTAG_STRIPOFFSETS, -1, -1, TIFF_LONG8, 0, TIFF_SETGET_UNDEFINED, FIELD_STRIPOFFSETS, 0, 0, "StripOffsets"),
    field(TIFFTAG_ORIENTATION, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_ORIENTATION, 0, 0, "Orientation"),
    field(TIFFTAG_SAMPLESPERPIXEL, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_SAMPLESPERPIXEL, 0, 0, "SamplesPerPixel"),
    field(TIFFTAG_ROWSPERSTRIP, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_ROWSPERSTRIP, 0, 0, "RowsPerStrip"),
    field(TIFFTAG_STRIPBYTECOUNTS, -1, -1, TIFF_LONG8, 0, TIFF_SETGET_UNDEFINED, FIELD_STRIPBYTECOUNTS, 0, 0, "StripByteCounts"),
    field(TIFFTAG_MINSAMPLEVALUE, -2, -1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_MINSAMPLEVALUE, 1, 0, "MinSampleValue"),
    field(TIFFTAG_MAXSAMPLEVALUE, -2, -1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_MAXSAMPLEVALUE, 1, 0, "MaxSampleValue"),
    field(TIFFTAG_XRESOLUTION, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_RESOLUTION, 1, 0, "XResolution"),
    field(TIFFTAG_YRESOLUTION, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_RESOLUTION, 1, 0, "YResolution"),
    field(TIFFTAG_PLANARCONFIG, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_PLANARCONFIG, 0, 0, "PlanarConfiguration"),
    field(TIFFTAG_PAGENAME, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "PageName"),
    field(TIFFTAG_XPOSITION, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_POSITION, 1, 0, "XPosition"),
    field(TIFFTAG_YPOSITION, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_POSITION, 1, 0, "YPosition"),
    field(TIFFTAG_FREEOFFSETS, -1, -1, TIFF_LONG8, 0, TIFF_SETGET_UNDEFINED, FIELD_IGNORE, 0, 0, "FreeOffsets"),
    field(TIFFTAG_FREEBYTECOUNTS, -1, -1, TIFF_LONG8, 0, TIFF_SETGET_UNDEFINED, FIELD_IGNORE, 0, 0, "FreeByteCounts"),
    field(TIFFTAG_GRAYRESPONSEUNIT, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UNDEFINED, FIELD_IGNORE, 1, 0, "GrayResponseUnit"),
    field(TIFFTAG_GRAYRESPONSECURVE, -1, -1, TIFF_SHORT, 0, TIFF_SETGET_UNDEFINED, FIELD_IGNORE, 1, 0, "GrayResponseCurve"),
    field(TIFFTAG_RESOLUTIONUNIT, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_RESOLUTIONUNIT, 1, 0, "ResolutionUnit"),
    field(TIFFTAG_PAGENUMBER, 2, 2, TIFF_SHORT, 0, TIFF_SETGET_UINT16_PAIR, FIELD_PAGENUMBER, 1, 0, "PageNumber"),
    field(TIFFTAG_COLORRESPONSEUNIT, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UNDEFINED, FIELD_IGNORE, 1, 0, "ColorResponseUnit"),
    field(TIFFTAG_TRANSFERFUNCTION, -1, -1, TIFF_SHORT, 0, TIFF_SETGET_OTHER, FIELD_TRANSFERFUNCTION, 1, 0, "TransferFunction"),
    field(TIFFTAG_SOFTWARE, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "Software"),
    field(TIFFTAG_DATETIME, 20, 20, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "DateTime"),
    field(TIFFTAG_ARTIST, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "Artist"),
    field(TIFFTAG_HOSTCOMPUTER, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "HostComputer"),
    field(TIFFTAG_WHITEPOINT, 2, 2, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "WhitePoint"),
    field(TIFFTAG_PRIMARYCHROMATICITIES, 6, 6, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "PrimaryChromaticities"),
    field(TIFFTAG_COLORMAP, -1, -1, TIFF_SHORT, 0, TIFF_SETGET_OTHER, FIELD_COLORMAP, 1, 0, "ColorMap"),
    field(TIFFTAG_HALFTONEHINTS, 2, 2, TIFF_SHORT, 0, TIFF_SETGET_UINT16_PAIR, FIELD_HALFTONEHINTS, 1, 0, "HalftoneHints"),
    field(TIFFTAG_TILEWIDTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_TILEDIMENSIONS, 0, 0, "TileWidth"),
    field(TIFFTAG_TILELENGTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_TILEDIMENSIONS, 0, 0, "TileLength"),
    field(TIFFTAG_TILEOFFSETS, -1, 1, TIFF_LONG8, 0, TIFF_SETGET_UNDEFINED, FIELD_STRIPOFFSETS, 0, 0, "TileOffsets"),
    field(TIFFTAG_TILEBYTECOUNTS, -1, 1, TIFF_LONG8, 0, TIFF_SETGET_UNDEFINED, FIELD_STRIPBYTECOUNTS, 0, 0, "TileByteCounts"),
    field(TIFFTAG_SUBIFD, -1, -1, TIFF_IFD8, 0, TIFF_SETGET_C16_IFD8, FIELD_SUBIFD, 1, 1, "SubIFD"),
    field(TIFFTAG_INKSET, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 0, 0, "InkSet"),
    field(TIFFTAG_INKNAMES, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_C16_ASCII, FIELD_INKNAMES, 1, 1, "InkNames"),
    field(TIFFTAG_NUMBEROFINKS, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_NUMBEROFINKS, 1, 0, "NumberOfInks"),
    field(TIFFTAG_DOTRANGE, 2, 2, TIFF_SHORT, 0, TIFF_SETGET_UINT16_PAIR, FIELD_CUSTOM, 0, 0, "DotRange"),
    field(TIFFTAG_TARGETPRINTER, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "TargetPrinter"),
    field(TIFFTAG_EXTRASAMPLES, -1, -1, TIFF_SHORT, 0, TIFF_SETGET_C16_UINT16, FIELD_EXTRASAMPLES, 0, 1, "ExtraSamples"),
    field(TIFFTAG_SAMPLEFORMAT, -1, -1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_SAMPLEFORMAT, 0, 0, "SampleFormat"),
    field(TIFFTAG_SMINSAMPLEVALUE, -2, -1, TIFF_ANY, 0, TIFF_SETGET_DOUBLE, FIELD_SMINSAMPLEVALUE, 1, 0, "SMinSampleValue"),
    field(TIFFTAG_SMAXSAMPLEVALUE, -2, -1, TIFF_ANY, 0, TIFF_SETGET_DOUBLE, FIELD_SMAXSAMPLEVALUE, 1, 0, "SMaxSampleValue"),
    field(TIFFTAG_CLIPPATH, -3, -3, TIFF_BYTE, 0, TIFF_SETGET_C32_UINT8, FIELD_CUSTOM, 0, 1, "ClipPath"),
    field(TIFFTAG_XCLIPPATHUNITS, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 0, 0, "XClipPathUnits"),
    field(TIFFTAG_YCLIPPATHUNITS, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 0, 0, "YClipPathUnits"),
    field(TIFFTAG_YCBCRCOEFFICIENTS, 3, 3, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 0, 0, "YCbCrCoefficients"),
    field(TIFFTAG_YCBCRSUBSAMPLING, 2, 2, TIFF_SHORT, 0, TIFF_SETGET_UINT16_PAIR, FIELD_YCBCRSUBSAMPLING, 0, 0, "YCbCrSubsampling"),
    field(TIFFTAG_YCBCRPOSITIONING, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_YCBCRPOSITIONING, 0, 0, "YCbCrPositioning"),
    field(TIFFTAG_REFERENCEBLACKWHITE, 6, 6, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_REFBLACKWHITE, 1, 0, "ReferenceBlackWhite"),
    field(TIFFTAG_XMLPACKET, -3, -3, TIFF_BYTE, 0, TIFF_SETGET_C32_UINT8, FIELD_CUSTOM, 1, 1, "XMLPacket"),
    // begin SGI tags
    field(TIFFTAG_MATTEING, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_EXTRASAMPLES, 0, 0, "Matteing"),
    field(TIFFTAG_DATATYPE, -2, -1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_SAMPLEFORMAT, 0, 0, "DataType"),
    field(TIFFTAG_IMAGEDEPTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_IMAGEDEPTH, 0, 0, "ImageDepth"),
    field(TIFFTAG_TILEDEPTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_TILEDEPTH, 0, 0, "TileDepth"),
    // end SGI tags
    // begin Pixar tags
    field(TIFFTAG_PIXAR_IMAGEFULLWIDTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "ImageFullWidth"),
    field(TIFFTAG_PIXAR_IMAGEFULLLENGTH, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "ImageFullLength"),
    field(TIFFTAG_PIXAR_TEXTUREFORMAT, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "TextureFormat"),
    field(TIFFTAG_PIXAR_WRAPMODES, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "TextureWrapModes"),
    field(TIFFTAG_PIXAR_FOVCOT, 1, 1, TIFF_FLOAT, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "FieldOfViewCotangent"),
    field(TIFFTAG_PIXAR_MATRIX_WORLDTOSCREEN, 16, 16, TIFF_FLOAT, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "MatrixWorldToScreen"),
    field(TIFFTAG_PIXAR_MATRIX_WORLDTOCAMERA, 16, 16, TIFF_FLOAT, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "MatrixWorldToCamera"),
    field(TIFFTAG_COPYRIGHT, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "Copyright"),
    // end Pixar tags
    field(TIFFTAG_RICHTIFFIPTC, -3, -3, TIFF_UNDEFINED, 0, TIFF_SETGET_C32_UINT8, FIELD_CUSTOM, 1, 1, "RichTIFFIPTC"),
    field(TIFFTAG_PHOTOSHOP, -3, -3, TIFF_BYTE, 0, TIFF_SETGET_C32_UINT8, FIELD_CUSTOM, 1, 1, "Photoshop"),
    // --: EXIFIFD and GPSIFD specified as TIFF_LONG by Aware-Systems and not TIFF_IFD8 as in original LibTiff. However, for IFD-like tags,
    // libtiff uses the data type TIFF_IFD8 in tiffFields[]-tag definition combined with a special handling procedure in order to write either
    // a 32-bit value and the TIFF_IFD type-id into ClassicTIFF files or a 64-bit value and the TIFF_IFD8 type-id into BigTIFF files.
    field(TIFFTAG_EXIFIFD, 1, 1, TIFF_LONG8, 0, TIFF_SETGET_UINT64, FIELD_CUSTOM, 1, 0, "EXIFIFDOffset"),
    field(TIFFTAG_ICCPROFILE, -3, -3, TIFF_UNDEFINED, 0, TIFF_SETGET_C32_UINT8, FIELD_CUSTOM, 1, 1, "ICC Profile"),
    field(TIFFTAG_GPSIFD, 1, 1, TIFF_LONG8, 0, TIFF_SETGET_UINT64, FIELD_CUSTOM, 1, 0, "GPSIFDOffset"),
    field(TIFFTAG_FAXRECVPARAMS, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "FaxRecvParams"),
    field(TIFFTAG_FAXSUBADDRESS, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "FaxSubAddress"),
    field(TIFFTAG_FAXRECVTIME, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "FaxRecvTime"),
    field(TIFFTAG_FAXDCS, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "FaxDcs"),
    field(TIFFTAG_STONITS, 1, 1, TIFF_DOUBLE, 0, TIFF_SETGET_DOUBLE, FIELD_CUSTOM, 0, 0, "StoNits"),
    field(TIFFTAG_IMAGESOURCEDATA, -3, -3, TIFF_UNDEFINED, 0, TIFF_SETGET_C32_UINT8, FIELD_CUSTOM, 1, 1, "Adobe Photoshop Document Data Block"),
    field(TIFFTAG_INTEROPERABILITYIFD, 1, 1, TIFF_IFD8, 0, TIFF_SETGET_IFD8, FIELD_CUSTOM, 0, 0, "InteroperabilityIFDOffset"),
    // begin DNG tags
    field(TIFFTAG_DNGVERSION, 4, 4, TIFF_BYTE, 0, TIFF_SETGET_C0_UINT8, FIELD_CUSTOM, 1, 0, "DNGVersion"),
    field(TIFFTAG_DNGBACKWARDVERSION, 4, 4, TIFF_BYTE, 0, TIFF_SETGET_C0_UINT8, FIELD_CUSTOM, 1, 0, "DNGBackwardVersion"),
    field(TIFFTAG_UNIQUECAMERAMODEL, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "UniqueCameraModel"),
    field(TIFFTAG_LOCALIZEDCAMERAMODEL, -1, -1, TIFF_BYTE, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "LocalizedCameraModel"),
    field(TIFFTAG_CFAPLANECOLOR, -1, -1, TIFF_BYTE, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "CFAPlaneColor"),
    field(TIFFTAG_CFALAYOUT, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 1, 0, "CFALayout"),
    field(TIFFTAG_LINEARIZATIONTABLE, -1, -1, TIFF_SHORT, 0, TIFF_SETGET_C16_UINT16, FIELD_CUSTOM, 1, 1, "LinearizationTable"),
    field(TIFFTAG_BLACKLEVELREPEATDIM, 2, 2, TIFF_SHORT, 0, TIFF_SETGET_C0_UINT16, FIELD_CUSTOM, 1, 0, "BlackLevelRepeatDim"),
    field(TIFFTAG_BLACKLEVEL, -1, -1, TIFF_RATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "BlackLevel"),
    field(TIFFTAG_BLACKLEVELDELTAH, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "BlackLevelDeltaH"),
    field(TIFFTAG_BLACKLEVELDELTAV, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "BlackLevelDeltaV"),
    field(TIFFTAG_WHITELEVEL, -1, -1, TIFF_LONG, 0, TIFF_SETGET_C16_UINT32, FIELD_CUSTOM, 1, 1, "WhiteLevel"),
    field(TIFFTAG_DEFAULTSCALE, 2, 2, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "DefaultScale"),
    field(TIFFTAG_BESTQUALITYSCALE, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "BestQualityScale"),
    field(TIFFTAG_DEFAULTCROPORIGIN, 2, 2, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "DefaultCropOrigin"),
    field(TIFFTAG_DEFAULTCROPSIZE, 2, 2, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "DefaultCropSize"),
    field(TIFFTAG_COLORMATRIX1, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "ColorMatrix1"),
    field(TIFFTAG_COLORMATRIX2, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "ColorMatrix2"),
    field(TIFFTAG_CAMERACALIBRATION1, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "CameraCalibration1"),
    field(TIFFTAG_CAMERACALIBRATION2, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "CameraCalibration2"),
    field(TIFFTAG_REDUCTIONMATRIX1, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "ReductionMatrix1"),
    field(TIFFTAG_REDUCTIONMATRIX2, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "ReductionMatrix2"),
    field(TIFFTAG_ANALOGBALANCE, -1, -1, TIFF_RATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "AnalogBalance"),
    field(TIFFTAG_ASSHOTNEUTRAL, -1, -1, TIFF_RATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "AsShotNeutral"),
    field(TIFFTAG_ASSHOTWHITEXY, 2, 2, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "AsShotWhiteXY"),
    field(TIFFTAG_BASELINEEXPOSURE, 1, 1, TIFF_SRATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "BaselineExposure"),
    field(TIFFTAG_BASELINENOISE, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "BaselineNoise"),
    field(TIFFTAG_BASELINESHARPNESS, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "BaselineSharpness"),
    field(TIFFTAG_BAYERGREENSPLIT, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "BayerGreenSplit"),
    field(TIFFTAG_LINEARRESPONSELIMIT, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "LinearResponseLimit"),
    field(TIFFTAG_CAMERASERIALNUMBER, -1, -1, TIFF_ASCII, 0, TIFF_SETGET_ASCII, FIELD_CUSTOM, 1, 0, "CameraSerialNumber"),
    field(TIFFTAG_LENSINFO, 4, 4, TIFF_RATIONAL, 0, TIFF_SETGET_C0_FLOAT, FIELD_CUSTOM, 1, 0, "LensInfo"),
    field(TIFFTAG_CHROMABLURRADIUS, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "ChromaBlurRadius"),
    field(TIFFTAG_ANTIALIASSTRENGTH, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "AntiAliasStrength"),
    field(TIFFTAG_SHADOWSCALE, 1, 1, TIFF_RATIONAL, 0, TIFF_SETGET_FLOAT, FIELD_CUSTOM, 1, 0, "ShadowScale"),
    field(TIFFTAG_DNGPRIVATEDATA, -1, -1, TIFF_BYTE, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "DNGPrivateData"),
    field(TIFFTAG_MAKERNOTESAFETY, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 1, 0, "MakerNoteSafety"),
    field(TIFFTAG_CALIBRATIONILLUMINANT1, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 1, 0, "CalibrationIlluminant1"),
    field(TIFFTAG_CALIBRATIONILLUMINANT2, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 1, 0, "CalibrationIlluminant2"),
    field(TIFFTAG_RAWDATAUNIQUEID, 16, 16, TIFF_BYTE, 0, TIFF_SETGET_C0_UINT8, FIELD_CUSTOM, 1, 0, "RawDataUniqueID"),
    field(TIFFTAG_ORIGINALRAWFILENAME, -1, -1, TIFF_BYTE, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "OriginalRawFileName"),
    field(TIFFTAG_ORIGINALRAWFILEDATA, -1, -1, TIFF_UNDEFINED, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "OriginalRawFileData"),
    field(TIFFTAG_ACTIVEAREA, 4, 4, TIFF_LONG, 0, TIFF_SETGET_C0_UINT32, FIELD_CUSTOM, 1, 0, "ActiveArea"),
    field(TIFFTAG_MASKEDAREAS, -1, -1, TIFF_LONG, 0, TIFF_SETGET_C16_UINT32, FIELD_CUSTOM, 1, 1, "MaskedAreas"),
    field(TIFFTAG_ASSHOTICCPROFILE, -1, -1, TIFF_UNDEFINED, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "AsShotICCProfile"),
    field(TIFFTAG_ASSHOTPREPROFILEMATRIX, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "AsShotPreProfileMatrix"),
    field(TIFFTAG_CURRENTICCPROFILE, -1, -1, TIFF_UNDEFINED, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "CurrentICCProfile"),
    field(TIFFTAG_CURRENTPREPROFILEMATRIX, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "CurrentPreProfileMatrix"),
    field(TIFFTAG_PERSAMPLE, 0, 0, TIFF_SHORT, 0, TIFF_SETGET_UNDEFINED, FIELD_PSEUDO, 1, 0, "PerSample"),
    // begin TIFF/EP tags
    field(TIFFTAG_EP_CFAREPEATPATTERNDIM, 2, 2, TIFF_SHORT, 0, TIFF_SETGET_C0_UINT16, FIELD_CUSTOM, 1, 0, "EP CFARepeatPatternDim"),
    field(TIFFTAG_EP_CFAPATTERN, -1, -1, TIFF_BYTE, 0, TIFF_SETGET_C16_UINT8, FIELD_CUSTOM, 1, 1, "EP CFAPattern"),
    // begin TIFF/FX tags
    field(TIFFTAG_INDEXED, 1, 1, TIFF_SHORT, 0, TIFF_SETGET_UINT16, FIELD_CUSTOM, 1, 0, "Indexed"),
    field(TIFFTAG_GLOBALPARAMETERSIFD, 1, 1, TIFF_IFD8, 0, TIFF_SETGET_IFD8, FIELD_CUSTOM, 1, 0, "GlobalParametersIFD"),
    field(TIFFTAG_PROFILETYPE, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "ProfileType"),
    field(TIFFTAG_FAXPROFILE, 1, 1, TIFF_BYTE, 0, TIFF_SETGET_UINT8, FIELD_CUSTOM, 1, 0, "FaxProfile"),
    field(TIFFTAG_CODINGMETHODS, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "CodingMethods"),
    field(TIFFTAG_VERSIONYEAR, 4, 4, TIFF_BYTE, 0, TIFF_SETGET_C0_UINT8, FIELD_CUSTOM, 1, 0, "VersionYear"),
    field(TIFFTAG_MODENUMBER, 1, 1, TIFF_BYTE, 0, TIFF_SETGET_UINT8, FIELD_CUSTOM, 1, 0, "ModeNumber"),
    field(TIFFTAG_DECODE, -1, -1, TIFF_SRATIONAL, 0, TIFF_SETGET_C16_FLOAT, FIELD_CUSTOM, 1, 1, "Decode"),
    field(TIFFTAG_IMAGEBASECOLOR, -1, -1, TIFF_SHORT, 0, TIFF_SETGET_C16_UINT16, FIELD_CUSTOM, 1, 1, "ImageBaseColor"),
    field(TIFFTAG_T82OPTIONS, 1, 1, TIFF_LONG, 0, TIFF_SETGET_UINT32, FIELD_CUSTOM, 1, 0, "T82Options"),
    field(TIFFTAG_STRIPROWCOUNTS, -1, -1, TIFF_LONG, 0, TIFF_SETGET_C16_UINT32, FIELD_CUSTOM, 1, 1, "StripRowCounts"),
    field(TIFFTAG_IMAGELAYER, 2, 2, TIFF_LONG, 0, TIFF_SETGET_C0_UINT32, FIELD_CUSTOM, 1, 0, "ImageLayer"),
    // end TIFF/FX tags
    // begin pseudo tags
];
