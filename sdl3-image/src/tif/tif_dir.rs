// Rust translation of libtiff/tif_dir.c and libtiff/tif_dir.h from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Directory Tag Get & Set Routines.
//! (and also some miscellaneous stuff)
//!
//! `TIFFSetField()` and `TIFFGetField()` are variadic in the C: here the
//! arguments of a set are a slice of [`Va`] (read in order, as `va_arg()`
//! reads them), and a get pushes the values it would store through its
//! pointer arguments, in order, onto a vector of [`Gv`] (arrays are copied
//! out, where the C hands out pointers into the directory). The writing
//! side (`TIFFCreateDirectory()`, `TIFFSetDirectory()` and the directory
//! number bookkeeping of multi-image files) is left out: SDL_image reads
//! the first directory only.

use super::tif_aux::{_tiff_clamp_double_to_float, _tiff_multiply_ssize};
use super::tif_compress::tiff_set_compression_scheme;
use super::tif_dirinfo::{
    _tiff_get_fields, _tiff_setup_fields, tiff_field_set_get_size, tiff_field_with_tag,
    tiff_find_field,
};
use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tiff::*;
use super::tiffio::{TIFFField, FIELD_CUSTOM, TIFF_ANY, TIFF_SPP, TIFF_VARIABLE, TIFF_VARIABLE2};
use super::tiffiop::{
    is_pseudo_tag, try_vec, TIFFPostMethod, Tiff, TmSize, O_RDONLY, TIFF_BIGTIFF,
    TIFF_CODERSETUP, TIFF_DIRTYDIRECT, TIFF_INSUBIFD, TIFF_ISTILED, TIFF_PERSAMPLE,
    TIFF_SWAB,
};

/*
 * ``Library-private'' Directory-related Definitions.
 */

/// Translation of `TIFFTagValue`: a custom tag's value, as the bytes the
/// C stores (`count` elements of `TIFFFieldSetGetSize()` bytes each, in
/// host order; `None` for the C's `NULL`).
#[derive(Clone, Debug)]
pub(crate) struct TIFFTagValue {
    pub(crate) info: TIFFField,
    pub(crate) count: i32,
    pub(crate) value: Option<Vec<u8>>,
}

/// Translation of `TIFFDirEntry`. `tdir_offset` is the union of
/// `toff_short`, `toff_long` and `toff_long8`, as its bytes: either the
/// offset or the data itself if fits, as the file has it (unswabbed).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TIFFDirEntry {
    pub(crate) tdir_tag: u16,   /* see below */
    pub(crate) tdir_type: u16,  /* data type; see below */
    pub(crate) tdir_count: u64, /* number of items; length in spec */
    pub(crate) tdir_offset: [u8; 8], /* either offset or the data itself if fits */
    pub(crate) tdir_ignore: u8, /* flag status to ignore tag when parsing tags in
                                tif_dirread.c */
}

impl TIFFDirEntry {
    /// `tdir_offset.toff_short`
    pub(crate) fn toff_short(&self) -> u16 {
        u16::from_ne_bytes([self.tdir_offset[0], self.tdir_offset[1]])
    }

    /// `tdir_offset.toff_long`
    pub(crate) fn toff_long(&self) -> u32 {
        u32::from_ne_bytes([
            self.tdir_offset[0],
            self.tdir_offset[1],
            self.tdir_offset[2],
            self.tdir_offset[3],
        ])
    }

    /// `tdir_offset.toff_long8`
    pub(crate) fn toff_long8(&self) -> u64 {
        u64::from_ne_bytes(self.tdir_offset)
    }
}

/// Translation of `TIFFEntryOffsetAndLength`: auxiliary for evaluating
/// size of IFD data
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TIFFEntryOffsetAndLength {
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

/*
 * Internal format of a TIFF directory entry.
 */
pub(crate) const FIELDSET_ITEMS: usize = 4;

/// Translation of `TIFFDirectory`. The arrays the C allocates are vectors
/// (`None` for the C's `NULL`).
#[derive(Clone, Debug, Default)]
pub(crate) struct TIFFDirectory {
    /* bit vector of fields that are set */
    pub(crate) td_fieldsset: [u32; FIELDSET_ITEMS],

    pub(crate) td_imagewidth: u32,
    pub(crate) td_imagelength: u32,
    pub(crate) td_imagedepth: u32,
    pub(crate) td_tilewidth: u32,
    pub(crate) td_tilelength: u32,
    pub(crate) td_tiledepth: u32,
    pub(crate) td_subfiletype: u32,
    pub(crate) td_bitspersample: u16,
    pub(crate) td_sampleformat: u16,
    pub(crate) td_compression: u16,
    pub(crate) td_photometric: u16,
    pub(crate) td_threshholding: u16,
    pub(crate) td_fillorder: u16,
    pub(crate) td_orientation: u16,
    pub(crate) td_samplesperpixel: u16,
    pub(crate) td_rowsperstrip: u32,
    pub(crate) td_minsamplevalue: u16,
    pub(crate) td_maxsamplevalue: u16,
    pub(crate) td_sminsamplevalue: Option<Vec<f64>>,
    pub(crate) td_smaxsamplevalue: Option<Vec<f64>>,
    pub(crate) td_xresolution: f32,
    pub(crate) td_yresolution: f32,
    pub(crate) td_resolutionunit: u16,
    pub(crate) td_planarconfig: u16,
    pub(crate) td_xposition: f32,
    pub(crate) td_yposition: f32,
    pub(crate) td_pagenumber: [u16; 2],
    pub(crate) td_colormap: [Option<Vec<u16>>; 3],
    pub(crate) td_halftonehints: [u16; 2],
    pub(crate) td_extrasamples: u16,
    pub(crate) td_sampleinfo: Option<Vec<u16>>,
    /* strip support */
    pub(crate) td_row: u32,             /* current scanline */
    pub(crate) td_curstrip: u32,        /* current strip for read/write */
    pub(crate) td_scanlinesize: TmSize, /* # of bytes in a scanline */
    /* tiling support */
    pub(crate) td_col: u32,         /* current column (offset by row too) */
    pub(crate) td_curtile: u32,     /* current tile for read/write */
    pub(crate) td_tilesize: TmSize, /* # of bytes in a tile */
    /* even though the name is misleading, td_stripsperimage is the number
     * of striles (=strips or tiles) per plane, and td_nstrips the total
     * number of striles.
     */
    pub(crate) td_stripsperimage: u32,
    pub(crate) td_nstrips: u32, /* size of offset & bytecount arrays */
    pub(crate) td_stripoffset_p: Option<Vec<u64>>, /* should be accessed with
                                                   TIFFGetStrileOffset */
    pub(crate) td_stripbytecount_p: Option<Vec<u64>>, /* should be accessed with
                                                      TIFFGetStrileByteCount */
    pub(crate) td_stripoffsetbyteallocsize: u32, /* number of elements currently allocated
                                                 for td_stripoffset/td_stripbytecount.
                                                 Only used if TIFF_LAZYSTRILELOAD is set
                                               */
    /* Be aware that the parameters of td_stripoffset_entry and
     * td_stripbytecount_entry are swapped but tdir_offset is not
     * and has to be swapped when used. */
    pub(crate) td_stripoffset_entry: TIFFDirEntry, /* for deferred loading */
    pub(crate) td_stripbytecount_entry: TIFFDirEntry, /* for deferred loading */
    pub(crate) td_nsubifd: u16,
    pub(crate) td_subifd: Option<Vec<u64>>,
    /* YCbCr parameters */
    pub(crate) td_ycbcrsubsampling: [u16; 2],
    pub(crate) td_ycbcrpositioning: u16,
    /* Colorimetry parameters */
    pub(crate) td_transferfunction: [Option<Vec<u16>>; 3],
    pub(crate) td_refblackwhite: Option<Vec<f32>>,
    /* CMYK parameters */
    pub(crate) td_inknameslen: i32,
    pub(crate) td_inknames: Option<Vec<u8>>,
    pub(crate) td_numberofinks: u16, /* number of inks in InkNames string */

    /// `td_customValueCount` is the length of `td_customValues`.
    pub(crate) td_customValues: Vec<TIFFTagValue>,

    pub(crate) td_deferstrilearraywriting: u8, /* see TIFFDeferStrileArrayWriting() */

    pub(crate) td_iswrittentofile: u8, /* indicates if current IFD is present on file */

    /* LibTIFF writes all data that does not fit into the IFD entries directly
     * after the IFD tag entry part. When reading, only the IFD data directly
     * and continuously behind the IFD tags is taken into account for the IFD
     * data size.*/
    pub(crate) td_dirdatasize_write: u64, /* auxiliary for evaluating size of IFD data
                                          to be written */
    pub(crate) td_dirdatasize_read: u64, /* auxiliary for evaluating size of IFD data
                                         read from file */
    pub(crate) td_dirdatasize_Noffsets: u32, /* auxiliary counter for
                                             tif_dir.td_dirdatasize_offsets array */
    pub(crate) td_dirdatasize_offsets: Option<Vec<TIFFEntryOffsetAndLength>>, /* auxiliary array for all offsets of IFD tag
                                                                              entries with data outside the IFD tag
                                                                              entries. */
}

pub(crate) const NOSTRIP: u32 = u32::MAX; /* undefined state */
pub(crate) const NOTILE: u32 = u32::MAX; /* undefined state */

/*
 * Field flags used to indicate fields that have been set in a directory, and
 * to reference fields when manipulating a directory.
 */

/*
 * FIELD_IGNORE is used to signify tags that are to be processed but otherwise
 * ignored.  This permits antiquated tags to be quietly read and discarded.
 * Note that a bit *is* allocated for ignored tags; this is understood by the
 * directory reading logic which uses this fact to avoid special-case handling
 */
pub(crate) const FIELD_IGNORE: u16 = 0;

/* multi-item fields */
pub(crate) const FIELD_IMAGEDIMENSIONS: u16 = 1;
pub(crate) const FIELD_TILEDIMENSIONS: u16 = 2;
pub(crate) const FIELD_RESOLUTION: u16 = 3;
pub(crate) const FIELD_POSITION: u16 = 4;

/* single-item fields */
pub(crate) const FIELD_SUBFILETYPE: u16 = 5;
pub(crate) const FIELD_BITSPERSAMPLE: u16 = 6;
pub(crate) const FIELD_COMPRESSION: u16 = 7;
pub(crate) const FIELD_PHOTOMETRIC: u16 = 8;
pub(crate) const FIELD_THRESHHOLDING: u16 = 9;
pub(crate) const FIELD_FILLORDER: u16 = 10;
pub(crate) const FIELD_ORIENTATION: u16 = 15;
pub(crate) const FIELD_SAMPLESPERPIXEL: u16 = 16;
pub(crate) const FIELD_ROWSPERSTRIP: u16 = 17;
pub(crate) const FIELD_MINSAMPLEVALUE: u16 = 18;
pub(crate) const FIELD_MAXSAMPLEVALUE: u16 = 19;
pub(crate) const FIELD_PLANARCONFIG: u16 = 20;
pub(crate) const FIELD_RESOLUTIONUNIT: u16 = 22;
pub(crate) const FIELD_PAGENUMBER: u16 = 23;
pub(crate) const FIELD_STRIPBYTECOUNTS: u16 = 24;
pub(crate) const FIELD_STRIPOFFSETS: u16 = 25;
pub(crate) const FIELD_COLORMAP: u16 = 26;
pub(crate) const FIELD_EXTRASAMPLES: u16 = 31;
pub(crate) const FIELD_SAMPLEFORMAT: u16 = 32;
pub(crate) const FIELD_SMINSAMPLEVALUE: u16 = 33;
pub(crate) const FIELD_SMAXSAMPLEVALUE: u16 = 34;
pub(crate) const FIELD_IMAGEDEPTH: u16 = 35;
pub(crate) const FIELD_TILEDEPTH: u16 = 36;
pub(crate) const FIELD_HALFTONEHINTS: u16 = 37;
pub(crate) const FIELD_YCBCRSUBSAMPLING: u16 = 39;
pub(crate) const FIELD_YCBCRPOSITIONING: u16 = 40;
pub(crate) const FIELD_REFBLACKWHITE: u16 = 41;
pub(crate) const FIELD_TRANSFERFUNCTION: u16 = 44;
pub(crate) const FIELD_INKNAMES: u16 = 46;
pub(crate) const FIELD_SUBIFD: u16 = 49;
pub(crate) const FIELD_NUMBEROFINKS: u16 = 50;
/*      FIELD_CUSTOM (see tiffio.h)     65 */
/* end of support for well-known tags; codec-private tags follow */
pub(crate) const FIELD_CODEC: u16 = 66; /* base of codec-private tags */

/*
 * Pseudo-tags don't normally need field bits since they are not written to an
 * output file (by definition). The library also has express logic to always
 * query a codec for a pseudo-tag so allocating a field bit for one is a
 * waste.   If codec wants to promote the notion of a pseudo-tag being ``set''
 * or ``unset'' then it can do using internal state flags without polluting
 * the field bit space defined for real tags.
 */
pub(crate) const FIELD_PSEUDO: u16 = 0;

pub(crate) const FIELD_LAST: u16 = (32 * FIELDSET_ITEMS - 1) as u16;

/// Translation of `BITn()`.
fn bitn(n: u16) -> u32 {
    1u32 << (n & 0x1f)
}

/// Translation of `TIFFFieldSet()`.
pub(crate) fn tiff_field_set(tif: &Tiff<'_>, field: u16) -> bool {
    (tif.tif_dir.td_fieldsset[(field / 32) as usize % FIELDSET_ITEMS] & bitn(field)) != 0
}

/// Translation of `TIFFSetFieldBit()`.
pub(crate) fn tiff_set_field_bit(tif: &mut Tiff<'_>, field: u16) {
    tif.tif_dir.td_fieldsset[(field / 32) as usize % FIELDSET_ITEMS] |= bitn(field);
}

/// Translation of `TIFFClrFieldBit()`.
pub(crate) fn tiff_clr_field_bit(tif: &mut Tiff<'_>, field: u16) {
    tif.tif_dir.td_fieldsset[(field / 32) as usize % FIELDSET_ITEMS] &= !bitn(field);
}

/// Translation of `TIFFSetGetFieldType`.
pub(crate) type TIFFSetGetFieldType = u8;
pub(crate) const TIFF_SETGET_UNDEFINED: TIFFSetGetFieldType = 0;
pub(crate) const TIFF_SETGET_ASCII: TIFFSetGetFieldType = 1;
pub(crate) const TIFF_SETGET_UINT8: TIFFSetGetFieldType = 2;
pub(crate) const TIFF_SETGET_SINT8: TIFFSetGetFieldType = 3;
pub(crate) const TIFF_SETGET_UINT16: TIFFSetGetFieldType = 4;
pub(crate) const TIFF_SETGET_SINT16: TIFFSetGetFieldType = 5;
pub(crate) const TIFF_SETGET_UINT32: TIFFSetGetFieldType = 6;
pub(crate) const TIFF_SETGET_SINT32: TIFFSetGetFieldType = 7;
pub(crate) const TIFF_SETGET_UINT64: TIFFSetGetFieldType = 8;
pub(crate) const TIFF_SETGET_SINT64: TIFFSetGetFieldType = 9;
pub(crate) const TIFF_SETGET_FLOAT: TIFFSetGetFieldType = 10;
pub(crate) const TIFF_SETGET_DOUBLE: TIFFSetGetFieldType = 11;
pub(crate) const TIFF_SETGET_IFD8: TIFFSetGetFieldType = 12;
pub(crate) const TIFF_SETGET_INT: TIFFSetGetFieldType = 13;
pub(crate) const TIFF_SETGET_UINT16_PAIR: TIFFSetGetFieldType = 14;
pub(crate) const TIFF_SETGET_C0_ASCII: TIFFSetGetFieldType = 15;
pub(crate) const TIFF_SETGET_C0_UINT8: TIFFSetGetFieldType = 16;
pub(crate) const TIFF_SETGET_C0_SINT8: TIFFSetGetFieldType = 17;
pub(crate) const TIFF_SETGET_C0_UINT16: TIFFSetGetFieldType = 18;
pub(crate) const TIFF_SETGET_C0_SINT16: TIFFSetGetFieldType = 19;
pub(crate) const TIFF_SETGET_C0_UINT32: TIFFSetGetFieldType = 20;
pub(crate) const TIFF_SETGET_C0_SINT32: TIFFSetGetFieldType = 21;
pub(crate) const TIFF_SETGET_C0_UINT64: TIFFSetGetFieldType = 22;
pub(crate) const TIFF_SETGET_C0_SINT64: TIFFSetGetFieldType = 23;
pub(crate) const TIFF_SETGET_C0_FLOAT: TIFFSetGetFieldType = 24;
pub(crate) const TIFF_SETGET_C0_DOUBLE: TIFFSetGetFieldType = 25;
pub(crate) const TIFF_SETGET_C0_IFD8: TIFFSetGetFieldType = 26;
pub(crate) const TIFF_SETGET_C16_ASCII: TIFFSetGetFieldType = 27;
pub(crate) const TIFF_SETGET_C16_UINT8: TIFFSetGetFieldType = 28;
pub(crate) const TIFF_SETGET_C16_SINT8: TIFFSetGetFieldType = 29;
pub(crate) const TIFF_SETGET_C16_UINT16: TIFFSetGetFieldType = 30;
pub(crate) const TIFF_SETGET_C16_SINT16: TIFFSetGetFieldType = 31;
pub(crate) const TIFF_SETGET_C16_UINT32: TIFFSetGetFieldType = 32;
pub(crate) const TIFF_SETGET_C16_SINT32: TIFFSetGetFieldType = 33;
pub(crate) const TIFF_SETGET_C16_UINT64: TIFFSetGetFieldType = 34;
pub(crate) const TIFF_SETGET_C16_SINT64: TIFFSetGetFieldType = 35;
pub(crate) const TIFF_SETGET_C16_FLOAT: TIFFSetGetFieldType = 36;
pub(crate) const TIFF_SETGET_C16_DOUBLE: TIFFSetGetFieldType = 37;
pub(crate) const TIFF_SETGET_C16_IFD8: TIFFSetGetFieldType = 38;
pub(crate) const TIFF_SETGET_C32_ASCII: TIFFSetGetFieldType = 39;
pub(crate) const TIFF_SETGET_C32_UINT8: TIFFSetGetFieldType = 40;
pub(crate) const TIFF_SETGET_C32_SINT8: TIFFSetGetFieldType = 41;
pub(crate) const TIFF_SETGET_C32_UINT16: TIFFSetGetFieldType = 42;
pub(crate) const TIFF_SETGET_C32_SINT16: TIFFSetGetFieldType = 43;
pub(crate) const TIFF_SETGET_C32_UINT32: TIFFSetGetFieldType = 44;
pub(crate) const TIFF_SETGET_C32_SINT32: TIFFSetGetFieldType = 45;
pub(crate) const TIFF_SETGET_C32_UINT64: TIFFSetGetFieldType = 46;
pub(crate) const TIFF_SETGET_C32_SINT64: TIFFSetGetFieldType = 47;
pub(crate) const TIFF_SETGET_C32_FLOAT: TIFFSetGetFieldType = 48;
pub(crate) const TIFF_SETGET_C32_DOUBLE: TIFFSetGetFieldType = 49;
pub(crate) const TIFF_SETGET_C32_IFD8: TIFFSetGetFieldType = 50;
pub(crate) const TIFF_SETGET_OTHER: TIFFSetGetFieldType = 51;

/// One argument of `TIFFSetField()`'s variable argument list.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Va<'a> {
    /// An integer argument (`int`, `uint16_vap`, `uint32_t`, `uint64_t`,
    /// stored as its bits).
    Int(i64),
    /// A `double` argument.
    Double(f64),
    /// A null pointer.
    Null,
    /// Pointers to arrays.
    U8s(&'a [u8]),
    I8s(&'a [i8]),
    U16s(&'a [u16]),
    I16s(&'a [i16]),
    U32s(&'a [u32]),
    I32s(&'a [i32]),
    U64s(&'a [u64]),
    I64s(&'a [i64]),
    F32s(&'a [f32]),
    F64s(&'a [f64]),
}

/// `va_list` over `TIFFSetField()`'s arguments: `va_arg()` reads the next.
/// A missing or mistyped argument reads as 0 (or `NULL`), where the C's
/// behavior is undefined.
#[derive(Debug)]
pub(crate) struct VaList<'a> {
    args: &'a [Va<'a>],
    pos: usize,
}

impl<'a> VaList<'a> {
    pub(crate) fn new(args: &'a [Va<'a>]) -> Self {
        VaList { args, pos: 0 }
    }

    fn next(&mut self) -> Va<'a> {
        let a = self.args.get(self.pos).copied().unwrap_or(Va::Null);
        self.pos += 1;
        a
    }

    /// `va_arg(ap, <integer type>)`, as its 64 bits.
    pub(crate) fn int(&mut self) -> i64 {
        match self.next() {
            Va::Int(v) => v,
            _ => 0,
        }
    }

    /// `va_arg(ap, double)`.
    pub(crate) fn double(&mut self) -> f64 {
        match self.next() {
            Va::Double(v) => v,
            Va::Int(v) => v as f64,
            _ => 0.0,
        }
    }

    /// `va_arg(ap, uint16_t *)`.
    pub(crate) fn u16s(&mut self) -> Option<&'a [u16]> {
        match self.next() {
            Va::U16s(v) => Some(v),
            _ => None,
        }
    }

    /// `va_arg(ap, float *)`.
    pub(crate) fn f32s(&mut self) -> Option<&'a [f32]> {
        match self.next() {
            Va::F32s(v) => Some(v),
            _ => None,
        }
    }

    /// `va_arg(ap, double *)`.
    pub(crate) fn f64s(&mut self) -> Option<&'a [f64]> {
        match self.next() {
            Va::F64s(v) => Some(v),
            _ => None,
        }
    }

    /// `va_arg(ap, uint64_t *)`.
    pub(crate) fn u64s(&mut self) -> Option<&'a [u64]> {
        match self.next() {
            Va::U64s(v) => Some(v),
            _ => None,
        }
    }

    /// `va_arg(ap, char *)` (or `uint8_t *`).
    pub(crate) fn bytes(&mut self) -> Option<&'a [u8]> {
        match self.next() {
            Va::U8s(v) => Some(v),
            _ => None,
        }
    }

    /// `va_arg(ap, void *)`: the bytes (host order) of the array pointed
    /// to, `None` for a null pointer.
    pub(crate) fn void_ptr(&mut self) -> Option<Vec<u8>> {
        fn ne<T: Copy, const N: usize>(v: &[T], f: impl Fn(T) -> [u8; N]) -> Vec<u8> {
            v.iter().flat_map(|&x| f(x)).collect()
        }
        Some(match self.next() {
            Va::U8s(v) => v.to_vec(),
            Va::I8s(v) => ne(v, i8::to_ne_bytes),
            Va::U16s(v) => ne(v, u16::to_ne_bytes),
            Va::I16s(v) => ne(v, i16::to_ne_bytes),
            Va::U32s(v) => ne(v, u32::to_ne_bytes),
            Va::I32s(v) => ne(v, i32::to_ne_bytes),
            Va::U64s(v) => ne(v, u64::to_ne_bytes),
            Va::I64s(v) => ne(v, i64::to_ne_bytes),
            Va::F32s(v) => ne(v, f32::to_ne_bytes),
            Va::F64s(v) => ne(v, f64::to_ne_bytes),
            Va::Int(_) | Va::Double(_) | Va::Null => return None,
        })
    }
}

/// One value `TIFFGetField()` stores through a pointer argument.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Gv {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    /// A `uint16_t *` (`None` for `NULL`).
    U16s(Option<Vec<u16>>),
    /// A `uint64_t *`.
    U64s(Option<Vec<u64>>),
    /// A `float *`.
    F32s(Option<Vec<f32>>),
    /// A `double *`.
    F64s(Option<Vec<f64>>),
    /// A `char *` or `void *`: the bytes pointed to.
    Bytes(Option<Vec<u8>>),
}

impl Gv {
    /// The value as an integer (of the integer variants).
    pub(crate) fn as_u64(&self) -> Option<u64> {
        Some(match *self {
            Gv::U8(v) => v as u64,
            Gv::I8(v) => v as u64,
            Gv::U16(v) => v as u64,
            Gv::I16(v) => v as u64,
            Gv::U32(v) => v as u64,
            Gv::I32(v) => v as u64,
            Gv::U64(v) => v,
            Gv::I64(v) => v as u64,
            _ => return None,
        })
    }

    /// The value as a `float *` (a float array, or the bytes of one).
    pub(crate) fn as_f32s(&self) -> Option<Vec<f32>> {
        match self {
            Gv::F32s(v) => v.clone(),
            Gv::Bytes(Some(b)) => Some(
                b.chunks_exact(4)
                    .map(|c| f32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
            ),
            _ => None,
        }
    }

    /// The value as a `uint16_t *`.
    pub(crate) fn as_u16s(&self) -> Option<Vec<u16>> {
        match self {
            Gv::U16s(v) => v.clone(),
            Gv::Bytes(Some(b)) => Some(
                b.chunks_exact(2)
                    .map(|c| u16::from_ne_bytes([c[0], c[1]]))
                    .collect(),
            ),
            _ => None,
        }
    }
}

/// `TIFFVSetMethod`
pub(crate) type TIFFVSetMethod = fn(&mut Tiff<'_>, u32, &mut VaList<'_>) -> i32;
/// `TIFFVGetMethod`
pub(crate) type TIFFVGetMethod = fn(&mut Tiff<'_>, u32, &mut Vec<Gv>) -> i32;

/// Translation of `TIFFTagMethods` (without the directory printer).
#[derive(Clone, Copy)]
pub(crate) struct TIFFTagMethods {
    pub(crate) vsetfield: TIFFVSetMethod, /* tag set routine */
    pub(crate) vgetfield: TIFFVGetMethod, /* tag get routine */
}

/*
 * These are used in the backwards compatibility code...
 */
const DATATYPE_VOID: u32 = 0; /* !untyped data */
const DATATYPE_INT: u32 = 1; /* !signed integer data */
const DATATYPE_UINT: u32 = 2; /* !unsigned integer data */
const DATATYPE_IEEEFP: u32 = 3; /* !IEEE floating point data */

/// Translation of `setByteArray()` (and of `_TIFFsetShortArrayExt()` and
/// its siblings): `*vpp` becomes a copy of the `nmemb` elements at `vp`,
/// or `NULL` when there are none (or they can't be allocated). Elements
/// past the end of `vp` (which the C would read out of bounds) are zero.
fn set_byte_array<T: Clone + Default>(vpp: &mut Option<Vec<T>>, vp: Option<&[T]>, nmemb: usize) {
    *vpp = None;
    if let Some(vp) = vp {
        let bytes = _tiff_multiply_ssize(
            None,
            nmemb as TmSize,
            std::mem::size_of::<T>() as TmSize,
            None,
        );
        if bytes != 0 {
            if let Some(mut v) = try_vec::<T>(nmemb) {
                let n = vp.len().min(nmemb);
                v[..n].clone_from_slice(&vp[..n]);
                *vpp = Some(v);
            }
        }
    }
}

/// Translation of `setDoubleArrayOneValue()`.
fn set_double_array_one_value(vpp: &mut Option<Vec<f64>>, value: f64, nmemb: usize) {
    *vpp = None;
    if let Some(mut v) = try_vec::<f64>(nmemb) {
        for e in v.iter_mut() {
            *e = value;
        }
        *vpp = Some(v);
    }
}

/// Translation of `setExtraSamples()`: Install extra samples information.
fn set_extra_samples(tif: &mut Tiff<'_>, ap: &mut VaList<'_>, v: &mut u32) -> i32 {
    /* XXX: Unassociated alpha data == 999 is a known Corel Draw bug, see below */
    const EXTRASAMPLE_COREL_UNASSALPHA: u16 = 999;

    const MODULE: &str = "setExtraSamples";

    *v = ap.int() as u16 as u32;
    if *v as u16 > tif.tif_dir.td_samplesperpixel {
        return 0;
    }
    let va = ap.u16s();
    if *v > 0 && va.is_none() {
        /* typically missing param */
        return 0;
    }
    let mut va: Vec<u16> = va.unwrap_or(&[]).to_vec();
    va.resize(va.len().max(*v as usize), 0);
    for i in 0..*v as usize {
        if va[i] > EXTRASAMPLE_UNASSALPHA {
            /*
             * XXX: Corel Draw is known to produce incorrect
             * ExtraSamples tags which must be patched here if we
             * want to be able to open some of the damaged TIFF
             * files:
             */
            if va[i] == EXTRASAMPLE_COREL_UNASSALPHA {
                va[i] = EXTRASAMPLE_UNASSALPHA;
            } else {
                return 0;
            }
        }
    }

    let td = &mut tif.tif_dir;
    if td.td_transferfunction[0].is_some()
        && (td.td_samplesperpixel as i64 - *v as i64 > 1)
        && !(td.td_samplesperpixel as i32 - td.td_extrasamples as i32 > 1)
    {
        tiff_warning_ext_r!(
            MODULE,
            "ExtraSamples tag value is changing, but TransferFunction was read with a different value. Canceling it"
        );
        tiff_clr_field_bit(tif, FIELD_TRANSFERFUNCTION);
        tif.tif_dir.td_transferfunction[0] = None;
    }

    let td = &mut tif.tif_dir;
    td.td_extrasamples = *v as u16;
    set_byte_array(
        &mut td.td_sampleinfo,
        Some(&va),
        td.td_extrasamples as usize,
    );
    1
}

/// Translation of `countInkNamesString()`: Count ink names separated by
/// \0.  Returns zero if the ink names are not as expected.
fn count_ink_names_string(tif: &Tiff<'_>, slen: u32, s: &[u8]) -> u16 {
    let mut i: u16 = 0;

    if slen > 0 {
        let ep = slen as usize;
        let mut cp = 0usize;
        let bad = loop {
            while cp < ep && s.get(cp).copied().unwrap_or(0) != 0 {
                cp += 1;
            }
            if cp >= ep {
                break true;
            }
            cp += 1; /* skip \0 */
            i = i.wrapping_add(1);
            if cp >= ep {
                break false;
            }
        };
        if !bad {
            return i;
        }
    }
    tiff_error_ext_r!(
        "TIFFSetField",
        "{}: Invalid InkNames value; no null at given buffer end location {}, after {} ink",
        tif.tif_name,
        slen,
        i
    );
    0
}

/// How `_TIFFVSetField()` ends: its `end:` and `bad*:` labels.
enum SetEnd {
    Done(i32),
    End(i32),
    BadValue(u32),
    BadValue32(u32),
    BadValueDouble(f64),
    BadValueIfd8Long8,
}

/// Translation of `_TIFFVSetField()`.
fn _tiff_vset_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut VaList<'_>) -> i32 {
    const MODULE: &str = "_TIFFVSetField";

    let Some(fip) = tiff_find_field(tif, tag, TIFF_ANY) else {
        /* cannot happen since OkToChangeTag() already checks it */
        return 0;
    };
    let mut standard_tag = tag;
    /*
     * We want to force the custom code to be used for custom
     * fields even if the tag happens to match a well known
     * one - important for reinterpreted handling of standard
     * tag values in custom directories (i.e. EXIF)
     */
    if fip.field_bit == FIELD_CUSTOM {
        standard_tag = 0;
    }

    let end = vset_standard(tif, standard_tag, tag, &fip, ap);
    let status = match end {
        SetEnd::Done(status) => status,
        SetEnd::End(status) => return status,
        SetEnd::BadValue(v) | SetEnd::BadValue32(v) => {
            let fip2 = tiff_field_with_tag(tif, tag);
            tiff_error_ext_r!(
                MODULE,
                "{}: Bad value {} for \"{}\" tag",
                tif.tif_name,
                v,
                fip2.as_ref().map_or("Unknown", |f| &f.field_name)
            );
            return 0;
        }
        SetEnd::BadValueDouble(dblval) => {
            let fip2 = tiff_field_with_tag(tif, tag);
            tiff_error_ext_r!(
                MODULE,
                "{}: Bad value {:.6} for \"{}\" tag",
                tif.tif_name,
                dblval,
                fip2.as_ref().map_or("Unknown", |f| &f.field_name)
            );
            return 0;
        }
        SetEnd::BadValueIfd8Long8 => {
            /* Error message issued already above. */
            let td = &mut tif.tif_dir;
            /* Find the existing entry for this custom value. */
            if let Some(i) = td
                .td_customValues
                .iter()
                .position(|tv| tv.info.field_tag == tag)
            {
                /* Remove custom field from custom list */
                /* Shorten list and close gap in customValues list.
                 * Re-allocation of td_customValues not necessary here. */
                td.td_customValues.remove(i);
            }
            return 0;
        }
    };
    if status != 0 {
        if let Some(fip2) = tiff_field_with_tag(tif, tag) {
            tiff_set_field_bit(tif, fip2.field_bit);
        }
        tif.tif_flags |= TIFF_DIRTYDIRECT;
    }
    status
}

/// The `switch (standard_tag)` of `_TIFFVSetField()`.
fn vset_standard(
    tif: &mut Tiff<'_>,
    standard_tag: u32,
    tag: u32,
    fip: &TIFFField,
    ap: &mut VaList<'_>,
) -> SetEnd {
    const MODULE: &str = "_TIFFVSetField";
    let mut status = 1;
    let v: u32;
    let v32: u32;

    match standard_tag {
        TIFFTAG_SUBFILETYPE => {
            tif.tif_dir.td_subfiletype = ap.int() as u32;
        }
        TIFFTAG_IMAGEWIDTH => {
            tif.tif_dir.td_imagewidth = ap.int() as u32;
        }
        TIFFTAG_IMAGELENGTH => {
            tif.tif_dir.td_imagelength = ap.int() as u32;
        }
        TIFFTAG_BITSPERSAMPLE => {
            tif.tif_dir.td_bitspersample = ap.int() as u16;
            /*
             * If the data require post-decoding processing to byte-swap
             * samples, set it up here.  Note that since tags are required
             * to be ordered, compression code can override this behavior
             * in the setup method if it wants to roll the post decoding
             * work in with its normal work.
             */
            if (tif.tif_flags & TIFF_SWAB) != 0 {
                match tif.tif_dir.td_bitspersample {
                    8 => tif.tif_postdecode = TIFFPostMethod::NoPostDecode,
                    16 => tif.tif_postdecode = TIFFPostMethod::Swab16BitData,
                    24 => tif.tif_postdecode = TIFFPostMethod::Swab24BitData,
                    32 => tif.tif_postdecode = TIFFPostMethod::Swab32BitData,
                    64 => tif.tif_postdecode = TIFFPostMethod::Swab64BitData,
                    128 => {
                        /* two 64's */
                        tif.tif_postdecode = TIFFPostMethod::Swab64BitData
                    }
                    _ => {}
                }
            }
        }
        TIFFTAG_COMPRESSION => {
            v = ap.int() as u16 as u32;
            /*
             * If we're changing the compression scheme, notify the
             * previous module so that it can cleanup any state it's
             * setup.
             */
            if tiff_field_set(tif, FIELD_COMPRESSION) {
                if tif.tif_dir.td_compression as u32 == v {
                    return SetEnd::Done(status);
                }
                (tif.tif_cleanup)(tif);
                tif.tif_flags &= !TIFF_CODERSETUP;
            }
            /*
             * Setup new compression routine state.
             */
            status = tiff_set_compression_scheme(tif, v as i32);
            if status != 0 {
                tif.tif_dir.td_compression = v as u16;
            } else {
                status = 0;
            }
        }
        TIFFTAG_PHOTOMETRIC => {
            tif.tif_dir.td_photometric = ap.int() as u16;
        }
        TIFFTAG_THRESHHOLDING => {
            tif.tif_dir.td_threshholding = ap.int() as u16;
        }
        TIFFTAG_FILLORDER => {
            v = ap.int() as u16 as u32;
            if v != FILLORDER_LSB2MSB as u32 && v != FILLORDER_MSB2LSB as u32 {
                return SetEnd::BadValue(v);
            }
            tif.tif_dir.td_fillorder = v as u16;
        }
        TIFFTAG_ORIENTATION => {
            v = ap.int() as u16 as u32;
            if v < ORIENTATION_TOPLEFT as u32 || (ORIENTATION_LEFTBOT as u32) < v {
                return SetEnd::BadValue(v);
            } else {
                tif.tif_dir.td_orientation = v as u16;
            }
        }
        TIFFTAG_SAMPLESPERPIXEL => {
            v = ap.int() as u16 as u32;
            if v == 0 {
                return SetEnd::BadValue(v);
            }
            if v != tif.tif_dir.td_samplesperpixel as u32 {
                /* See http://bugzilla.maptools.org/show_bug.cgi?id=2500 */
                if tif.tif_dir.td_sminsamplevalue.is_some() {
                    tiff_warning_ext_r!(
                        MODULE,
                        "SamplesPerPixel tag value is changing, but SMinSampleValue tag was read with a different value. Canceling it"
                    );
                    tiff_clr_field_bit(tif, FIELD_SMINSAMPLEVALUE);
                    tif.tif_dir.td_sminsamplevalue = None;
                }
                if tif.tif_dir.td_smaxsamplevalue.is_some() {
                    tiff_warning_ext_r!(
                        MODULE,
                        "SamplesPerPixel tag value is changing, but SMaxSampleValue tag was read with a different value. Canceling it"
                    );
                    tiff_clr_field_bit(tif, FIELD_SMAXSAMPLEVALUE);
                    tif.tif_dir.td_smaxsamplevalue = None;
                }
                /* Test if 3 transfer functions instead of just one are now
                   needed See http://bugzilla.maptools.org/show_bug.cgi?id=2820
                 */
                let td = &tif.tif_dir;
                if td.td_transferfunction[0].is_some()
                    && (v as i32 - td.td_extrasamples as i32 > 1)
                    && !(td.td_samplesperpixel as i32 - td.td_extrasamples as i32 > 1)
                {
                    tiff_warning_ext_r!(
                        MODULE,
                        "SamplesPerPixel tag value is changing, but TransferFunction was read with a different value. Canceling it"
                    );
                    tiff_clr_field_bit(tif, FIELD_TRANSFERFUNCTION);
                    tif.tif_dir.td_transferfunction[0] = None;
                }
            }
            tif.tif_dir.td_samplesperpixel = v as u16;
        }
        TIFFTAG_ROWSPERSTRIP => {
            v32 = ap.int() as u32;
            if v32 == 0 {
                return SetEnd::BadValue32(v32);
            }
            tif.tif_dir.td_rowsperstrip = v32;
            if !tiff_field_set(tif, FIELD_TILEDIMENSIONS) {
                tif.tif_dir.td_tilelength = v32;
                tif.tif_dir.td_tilewidth = tif.tif_dir.td_imagewidth;
            }
        }
        TIFFTAG_MINSAMPLEVALUE => {
            tif.tif_dir.td_minsamplevalue = ap.int() as u16;
        }
        TIFFTAG_MAXSAMPLEVALUE => {
            tif.tif_dir.td_maxsamplevalue = ap.int() as u16;
        }
        TIFFTAG_SMINSAMPLEVALUE => {
            let n = tif.tif_dir.td_samplesperpixel as usize;
            if (tif.tif_flags & TIFF_PERSAMPLE) != 0 {
                set_byte_array(&mut tif.tif_dir.td_sminsamplevalue, ap.f64s(), n);
            } else {
                set_double_array_one_value(&mut tif.tif_dir.td_sminsamplevalue, ap.double(), n);
            }
        }
        TIFFTAG_SMAXSAMPLEVALUE => {
            let n = tif.tif_dir.td_samplesperpixel as usize;
            if (tif.tif_flags & TIFF_PERSAMPLE) != 0 {
                set_byte_array(&mut tif.tif_dir.td_smaxsamplevalue, ap.f64s(), n);
            } else {
                set_double_array_one_value(&mut tif.tif_dir.td_smaxsamplevalue, ap.double(), n);
            }
        }
        TIFFTAG_XRESOLUTION => {
            let dblval = ap.double();
            if dblval.is_nan() || dblval < 0.0 {
                return SetEnd::BadValueDouble(dblval);
            }
            tif.tif_dir.td_xresolution = _tiff_clamp_double_to_float(dblval);
        }
        TIFFTAG_YRESOLUTION => {
            let dblval = ap.double();
            if dblval.is_nan() || dblval < 0.0 {
                return SetEnd::BadValueDouble(dblval);
            }
            tif.tif_dir.td_yresolution = _tiff_clamp_double_to_float(dblval);
        }
        TIFFTAG_PLANARCONFIG => {
            v = ap.int() as u16 as u32;
            if v != PLANARCONFIG_CONTIG as u32 && v != PLANARCONFIG_SEPARATE as u32 {
                return SetEnd::BadValue(v);
            }
            tif.tif_dir.td_planarconfig = v as u16;
        }
        TIFFTAG_XPOSITION => {
            tif.tif_dir.td_xposition = _tiff_clamp_double_to_float(ap.double());
        }
        TIFFTAG_YPOSITION => {
            tif.tif_dir.td_yposition = _tiff_clamp_double_to_float(ap.double());
        }
        TIFFTAG_RESOLUTIONUNIT => {
            v = ap.int() as u16 as u32;
            if v < RESUNIT_NONE as u32 || (RESUNIT_CENTIMETER as u32) < v {
                return SetEnd::BadValue(v);
            }
            tif.tif_dir.td_resolutionunit = v as u16;
        }
        TIFFTAG_PAGENUMBER => {
            tif.tif_dir.td_pagenumber[0] = ap.int() as u16;
            tif.tif_dir.td_pagenumber[1] = ap.int() as u16;
        }
        TIFFTAG_HALFTONEHINTS => {
            tif.tif_dir.td_halftonehints[0] = ap.int() as u16;
            tif.tif_dir.td_halftonehints[1] = ap.int() as u16;
        }
        TIFFTAG_COLORMAP => {
            if tif.tif_dir.td_bitspersample >= 32 {
                v = tif.tif_dir.td_bitspersample as u32;
                return SetEnd::BadValue(v);
            }
            v32 = 1u32 << tif.tif_dir.td_bitspersample;
            let td = &mut tif.tif_dir;
            set_byte_array(&mut td.td_colormap[0], ap.u16s(), v32 as usize);
            set_byte_array(&mut td.td_colormap[1], ap.u16s(), v32 as usize);
            set_byte_array(&mut td.td_colormap[2], ap.u16s(), v32 as usize);
        }
        TIFFTAG_EXTRASAMPLES => {
            let mut v = 0;
            if set_extra_samples(tif, ap, &mut v) == 0 {
                return SetEnd::BadValue(v);
            }
        }
        TIFFTAG_MATTEING => {
            tif.tif_dir.td_extrasamples = ((ap.int() as u16) != 0) as u16;
            if tif.tif_dir.td_extrasamples != 0 {
                let sv = [EXTRASAMPLE_ASSOCALPHA];
                set_byte_array(&mut tif.tif_dir.td_sampleinfo, Some(&sv), 1);
            }
        }
        TIFFTAG_TILEWIDTH => {
            v32 = ap.int() as u32;
            if v32 % 16 != 0 {
                if tif.tif_mode != O_RDONLY {
                    return SetEnd::BadValue32(v32);
                }
                tiff_warning_ext_r!(
                    tif.tif_name,
                    "Nonstandard tile width {}, convert file",
                    v32
                );
            }
            tif.tif_dir.td_tilewidth = v32;
            tif.tif_flags |= TIFF_ISTILED;
        }
        TIFFTAG_TILELENGTH => {
            v32 = ap.int() as u32;
            if v32 % 16 != 0 {
                if tif.tif_mode != O_RDONLY {
                    return SetEnd::BadValue32(v32);
                }
                tiff_warning_ext_r!(
                    tif.tif_name,
                    "Nonstandard tile length {}, convert file",
                    v32
                );
            }
            tif.tif_dir.td_tilelength = v32;
            tif.tif_flags |= TIFF_ISTILED;
        }
        TIFFTAG_TILEDEPTH => {
            v32 = ap.int() as u32;
            if v32 == 0 {
                return SetEnd::BadValue32(v32);
            }
            tif.tif_dir.td_tiledepth = v32;
        }
        TIFFTAG_DATATYPE => {
            let mut v = ap.int() as u16 as u32;
            match v {
                DATATYPE_VOID => v = SAMPLEFORMAT_VOID as u32,
                DATATYPE_INT => v = SAMPLEFORMAT_INT as u32,
                DATATYPE_UINT => v = SAMPLEFORMAT_UINT as u32,
                DATATYPE_IEEEFP => v = SAMPLEFORMAT_IEEEFP as u32,
                _ => return SetEnd::BadValue(v),
            }
            tif.tif_dir.td_sampleformat = v as u16;
        }
        TIFFTAG_SAMPLEFORMAT => {
            v = ap.int() as u16 as u32;
            if v < SAMPLEFORMAT_UINT as u32 || (SAMPLEFORMAT_COMPLEXIEEEFP as u32) < v {
                return SetEnd::BadValue(v);
            }
            tif.tif_dir.td_sampleformat = v as u16;

            /*  Try to fix up the SWAB function for complex data. */
            let td = &tif.tif_dir;
            if td.td_sampleformat == SAMPLEFORMAT_COMPLEXINT
                && td.td_bitspersample == 32
                && tif.tif_postdecode == TIFFPostMethod::Swab32BitData
            {
                tif.tif_postdecode = TIFFPostMethod::Swab16BitData;
            } else if (td.td_sampleformat == SAMPLEFORMAT_COMPLEXINT
                || td.td_sampleformat == SAMPLEFORMAT_COMPLEXIEEEFP)
                && td.td_bitspersample == 64
                && tif.tif_postdecode == TIFFPostMethod::Swab64BitData
            {
                tif.tif_postdecode = TIFFPostMethod::Swab32BitData;
            }
        }
        TIFFTAG_IMAGEDEPTH => {
            tif.tif_dir.td_imagedepth = ap.int() as u32;
        }
        TIFFTAG_SUBIFD => {
            if (tif.tif_flags & TIFF_INSUBIFD) == 0 {
                tif.tif_dir.td_nsubifd = ap.int() as u16;
                let n = tif.tif_dir.td_nsubifd as usize;
                set_byte_array(&mut tif.tif_dir.td_subifd, ap.u64s(), n);
            } else {
                tiff_error_ext_r!(MODULE, "{}: Sorry, cannot nest SubIFDs", tif.tif_name);
                status = 0;
            }
        }
        TIFFTAG_YCBCRPOSITIONING => {
            tif.tif_dir.td_ycbcrpositioning = ap.int() as u16;
        }
        TIFFTAG_YCBCRSUBSAMPLING => {
            tif.tif_dir.td_ycbcrsubsampling[0] = ap.int() as u16;
            tif.tif_dir.td_ycbcrsubsampling[1] = ap.int() as u16;
        }
        TIFFTAG_TRANSFERFUNCTION => {
            if tif.tif_dir.td_bitspersample >= 32 {
                v = tif.tif_dir.td_bitspersample as u32;
                return SetEnd::BadValue(v);
            }
            let count = 1u32 << tif.tif_dir.td_bitspersample;
            let td = &mut tif.tif_dir;
            let v = if (td.td_samplesperpixel as i32 - td.td_extrasamples as i32) > 1 {
                3
            } else {
                1
            };
            for i in 0..v {
                set_byte_array(&mut td.td_transferfunction[i], ap.u16s(), count as usize);
            }
        }
        TIFFTAG_REFERENCEBLACKWHITE => {
            /* XXX should check for null range */
            set_byte_array(&mut tif.tif_dir.td_refblackwhite, ap.f32s(), 6);
        }
        TIFFTAG_INKNAMES => {
            let v = ap.int() as u16 as u32;
            let s = ap.bytes().unwrap_or(&[]);
            let ninksinstring = count_ink_names_string(tif, v, s);
            status = (ninksinstring > 0) as i32;
            if ninksinstring > 0 {
                set_byte_array(&mut tif.tif_dir.td_inknames, Some(s), v as usize);
                tif.tif_dir.td_inknameslen = v as i32;
                /* Set NumberOfInks to the value ninksinstring */
                if tiff_field_set(tif, FIELD_NUMBEROFINKS) {
                    if tif.tif_dir.td_numberofinks != ninksinstring {
                        tiff_error_ext_r!(
                            MODULE,
                            "Warning {}; Tag {}:\n  Value {} of NumberOfInks is different from the number of inks {}.\n  -> NumberOfInks value adapted to {}",
                            tif.tif_name,
                            fip.field_name,
                            tif.tif_dir.td_numberofinks,
                            ninksinstring,
                            ninksinstring
                        );
                        tif.tif_dir.td_numberofinks = ninksinstring;
                    }
                } else {
                    tif.tif_dir.td_numberofinks = ninksinstring;
                    tiff_set_field_bit(tif, FIELD_NUMBEROFINKS);
                }
                if tiff_field_set(tif, FIELD_SAMPLESPERPIXEL)
                    && tif.tif_dir.td_numberofinks != tif.tif_dir.td_samplesperpixel
                {
                    tiff_error_ext_r!(
                        MODULE,
                        "Warning {}; Tag {}:\n  Value {} of NumberOfInks is different from the SamplesPerPixel value {}",
                        tif.tif_name,
                        fip.field_name,
                        tif.tif_dir.td_numberofinks,
                        tif.tif_dir.td_samplesperpixel
                    );
                }
            }
        }
        TIFFTAG_NUMBEROFINKS => {
            let v = ap.int() as u16 as u32;
            /* If InkNames already set also NumberOfInks is set accordingly and
             * should be equal */
            if tiff_field_set(tif, FIELD_INKNAMES) {
                if v != tif.tif_dir.td_numberofinks as u32 {
                    tiff_error_ext_r!(
                        MODULE,
                        "Error {}; Tag {}:\n  It is not possible to set the value {} for NumberOfInks\n  which is different from the number of inks in the InkNames tag ({})",
                        tif.tif_name,
                        fip.field_name,
                        v,
                        tif.tif_dir.td_numberofinks
                    );
                    /* Do not set / overwrite number of inks already set by
                     * InkNames case accordingly. */
                    status = 0;
                }
            } else {
                tif.tif_dir.td_numberofinks = v as u16;
                if tiff_field_set(tif, FIELD_SAMPLESPERPIXEL)
                    && tif.tif_dir.td_numberofinks != tif.tif_dir.td_samplesperpixel
                {
                    tiff_error_ext_r!(
                        MODULE,
                        "Warning {}; Tag {}:\n  Value {} of NumberOfInks is different from the SamplesPerPixel value {}",
                        tif.tif_name,
                        fip.field_name,
                        v,
                        tif.tif_dir.td_samplesperpixel
                    );
                }
            }
        }
        TIFFTAG_PERSAMPLE => {
            let v = ap.int() as u16;
            if v == PERSAMPLE_MULTI {
                tif.tif_flags |= TIFF_PERSAMPLE;
            } else {
                tif.tif_flags &= !TIFF_PERSAMPLE;
            }
        }
        _ => return vset_custom(tif, tag, fip, ap),
    }
    SetEnd::Done(status)
}

/// The `default:` case of `_TIFFVSetField()`: a custom tag.
fn vset_custom(tif: &mut Tiff<'_>, tag: u32, fip: &TIFFField, ap: &mut VaList<'_>) -> SetEnd {
    const MODULE: &str = "_TIFFVSetField";
    let mut status = 1;

    /*
     * This can happen if multiple images are open with different
     * codecs which have private tags.  The global tag information
     * table may then have tags that are valid for one file but not
     * the other. If the client tries to set a tag that is not valid
     * for the image's codec then we'll arrive here.  This
     * happens, for example, when tiffcp is used to convert between
     * compression schemes and codec-specific tags are blindly copied.
     *
     * This also happens when a FIELD_IGNORE tag is written.
     */
    if fip.field_bit == FIELD_IGNORE {
        tiff_error_ext_r!(
            MODULE,
            "{}: Ignored {}tag \"{}\" (not supported by libtiff)",
            tif.tif_name,
            if is_pseudo_tag(tag) { "pseudo-" } else { "" },
            fip.field_name
        );
        return SetEnd::Done(0);
    }
    if fip.field_bit != FIELD_CUSTOM {
        tiff_error_ext_r!(
            MODULE,
            "{}: Invalid {}tag \"{}\" (not supported by codec)",
            tif.tif_name,
            if is_pseudo_tag(tag) { "pseudo-" } else { "" },
            fip.field_name
        );
        return SetEnd::Done(0);
    }

    /*
     * Find the existing entry for this custom value.
     */
    let td = &mut tif.tif_dir;
    let itv = match td
        .td_customValues
        .iter()
        .position(|tv| tv.info.field_tag == tag)
    {
        Some(i) => {
            td.td_customValues[i].value = None;
            i
        }
        None => {
            /*
             * Grow the custom list if the entry was not found.
             */
            if td.td_customValues.try_reserve(1).is_err() {
                tiff_error_ext_r!(
                    MODULE,
                    "{}: Failed to allocate space for list of custom values",
                    tif.tif_name
                );
                return SetEnd::End(0);
            }
            td.td_customValues.push(TIFFTagValue {
                info: fip.clone(),
                value: None,
                count: 0,
            });
            td.td_customValues.len() - 1
        }
    };

    /*
     * Set custom value ... save a copy of the custom tag value.
     */
    /*--: Rational2Double: For Rationals evaluate "set_get_field_type"
     * to determine internal storage size. */
    let tv_size = tiff_field_set_get_size(Some(fip));
    if tv_size == 0 {
        tiff_error_ext_r!(
            MODULE,
            "{}: Bad field type {} for \"{}\"",
            tif.tif_name,
            fip.field_type,
            fip.field_name
        );
        return SetEnd::End(0);
    }

    if fip.field_type == TIFF_ASCII {
        let ma: u32;
        let mb: &[u8];
        if fip.field_passcount != 0 {
            ma = ap.int() as u32;
            mb = ap.bytes().unwrap_or(&[]);
        } else {
            mb = ap.bytes().unwrap_or(&[]);
            let len = mb.iter().position(|&c| c == 0).unwrap_or(mb.len()) + 1;
            if len >= 0x80000000 {
                tiff_error_ext_r!(
                    MODULE,
                    "{}: Too long string value for \"{}\". Maximum supported is 2147483647 bytes",
                    tif.tif_name,
                    fip.field_name
                );
                return SetEnd::End(0);
            }
            ma = len as u32;
        }
        let tv = &mut tif.tif_dir.td_customValues[itv];
        tv.count = ma as i32;
        set_byte_array(&mut tv.value, Some(mb), ma as usize);
    } else {
        let count: i32 = if fip.field_passcount != 0 {
            if fip.field_writecount == TIFF_VARIABLE2 {
                ap.int() as u32 as i32
            } else {
                ap.int() as i32
            }
        } else if fip.field_writecount == TIFF_VARIABLE || fip.field_writecount == TIFF_VARIABLE2 {
            1
        } else if fip.field_writecount == TIFF_SPP {
            tif.tif_dir.td_samplesperpixel as i32
        } else {
            fip.field_writecount as i32
        };
        tif.tif_dir.td_customValues[itv].count = count;

        if count == 0 {
            tiff_warning_ext_r!(
                MODULE,
                "{}: Null count for \"{}\" (type {}, writecount {}, passcount {})",
                tif.tif_name,
                fip.field_name,
                fip.field_type,
                fip.field_writecount,
                fip.field_passcount
            );
            return SetEnd::Done(status);
        }

        let Some(mut value) =
            super::tif_aux::_tiff_check_malloc_vec::<u8>(tif, count as TmSize, tv_size as TmSize, "custom tag binary object")
        else {
            return SetEnd::End(0);
        };

        if fip.field_tag == TIFFTAG_DOTRANGE && fip.field_name == "DotRange" {
            /* TODO: This is an evil exception and should not have been
               handled this way ... likely best if we move it into
               the directory structure with an explicit field in
               libtiff 4.1 and assign it a FIELD_ value */
            let v2 = [ap.int() as u16, ap.int() as u16];
            value[..2].copy_from_slice(&v2[0].to_ne_bytes());
            value[2..4].copy_from_slice(&v2[1].to_ne_bytes());
        } else if fip.field_passcount != 0
            || fip.field_writecount == TIFF_VARIABLE
            || fip.field_writecount == TIFF_VARIABLE2
            || fip.field_writecount == TIFF_SPP
            || count > 1
        {
            /*--: Rational2Double: For Rationals tv_size is set above to
             * 4 or 8 according to fip->set_get_field_type! */
            let src = ap.void_ptr().unwrap_or_default();
            let n = value.len().min(src.len());
            value[..n].copy_from_slice(&src[..n]);
            /* Test here for too big values for LONG8, IFD8, SLONG8 in
             * ClassicTIFF and delete custom field from custom list */
            if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
                if fip.field_type == TIFF_LONG8 || fip.field_type == TIFF_IFD8 {
                    for (i, c) in value.chunks_exact(8).enumerate() {
                        let v = u64::from_ne_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                        if v > 0xffffffff {
                            tiff_error_ext_r!(
                                MODULE,
                                "{}: Bad {} value {} at {}. array position for \"{}\" tag {} in ClassicTIFF. Tag won't be written to file",
                                tif.tif_name,
                                if fip.field_type == TIFF_LONG8 { "LONG8" } else { "IFD8" },
                                v,
                                i,
                                fip.field_name,
                                tag
                            );
                            return SetEnd::BadValueIfd8Long8;
                        }
                    }
                } else if fip.field_type == TIFF_SLONG8 {
                    for (i, c) in value.chunks_exact(8).enumerate() {
                        let v = i64::from_ne_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
                        if v > 2147483647 || v < (-2147483647 - 1) {
                            tiff_error_ext_r!(
                                MODULE,
                                "{}: Bad SLONG8 value {} at {}. array position for \"{}\" tag {} in ClassicTIFF. Tag won't be written to file",
                                tif.tif_name,
                                v,
                                i,
                                fip.field_name,
                                tag
                            );
                            return SetEnd::BadValueIfd8Long8;
                        }
                    }
                }
            }
        } else {
            let val = &mut value;
            match fip.field_type {
                TIFF_BYTE | TIFF_UNDEFINED => {
                    let v2 = ap.int() as u8;
                    put_ne(val, &[v2], tv_size);
                }
                TIFF_SBYTE => {
                    let v2 = ap.int() as i8;
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                }
                TIFF_SHORT => {
                    let v2 = ap.int() as u16;
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                }
                TIFF_SSHORT => {
                    let v2 = ap.int() as i16;
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                }
                TIFF_LONG | TIFF_IFD => {
                    let v2 = ap.int() as u32;
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                }
                TIFF_SLONG => {
                    let v2 = ap.int() as i32;
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                }
                TIFF_LONG8 | TIFF_IFD8 => {
                    let v2 = ap.int() as u64;
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                    /* Test here for too big values for ClassicTIFF and
                     * delete custom field from custom list */
                    if (tif.tif_flags & TIFF_BIGTIFF) == 0 && v2 > 0xffffffff {
                        tiff_error_ext_r!(
                            MODULE,
                            "{}: Bad LONG8 or IFD8 value {} for \"{}\" tag {} in ClassicTIFF. Tag won't be written to file",
                            tif.tif_name,
                            v2,
                            fip.field_name,
                            tag
                        );
                        tif.tif_dir.td_customValues[itv].value = Some(value);
                        return SetEnd::BadValueIfd8Long8;
                    }
                }
                TIFF_SLONG8 => {
                    let v2 = ap.int();
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                    /* Test here for too big values for ClassicTIFF and
                     * delete custom field from custom list */
                    if (tif.tif_flags & TIFF_BIGTIFF) == 0
                        && (v2 > 2147483647 || v2 < (-2147483647 - 1))
                    {
                        tiff_error_ext_r!(
                            MODULE,
                            "{}: Bad SLONG8 value {} for \"{}\" tag {} in ClassicTIFF. Tag won't be written to file",
                            tif.tif_name,
                            v2,
                            fip.field_name,
                            tag
                        );
                        tif.tif_dir.td_customValues[itv].value = Some(value);
                        return SetEnd::BadValueIfd8Long8;
                    }
                }
                TIFF_RATIONAL | TIFF_SRATIONAL => {
                    /*-- Rational2Double: For Rationals tv_size is set
                     * above to 4 or 8 according to fip->set_get_field_type!
                     */
                    if tv_size == 8 {
                        let v2 = ap.double();
                        put_ne(val, &v2.to_ne_bytes(), tv_size);
                    } else {
                        /*-- default should be tv_size == 4 */
                        let v3 = ap.double() as f32;
                        put_ne(val, &v3.to_ne_bytes(), tv_size);
                        /*-- ToDo: After Testing, this should be
                         * removed and tv_size==4 should be set as
                         * default. */
                        if tv_size != 4 {
                            tiff_error_ext_r!(
                                MODULE,
                                "Rational2Double: .set_get_field_type in not 4 but {}",
                                tv_size
                            );
                        }
                    }
                }
                TIFF_FLOAT => {
                    let v2 = _tiff_clamp_double_to_float(ap.double());
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                }
                TIFF_DOUBLE => {
                    let v2 = ap.double();
                    put_ne(val, &v2.to_ne_bytes(), tv_size);
                }
                _ => {
                    /* TIFF_NOTYPE, TIFF_ASCII */
                    for b in val.iter_mut().take(tv_size as usize) {
                        *b = 0;
                    }
                    status = 0;
                }
            }
        }
        tif.tif_dir.td_customValues[itv].value = Some(value);
    }
    SetEnd::Done(status)
}

/// `_TIFFmemcpy(val, &v2, tv_size)`: the first `tv_size` bytes of a
/// value (of a value narrower than `tv_size`, the value then zeros, where
/// the C copies what follows it in memory).
fn put_ne(val: &mut [u8], bytes: &[u8], tv_size: i32) {
    let n = (tv_size.max(0) as usize).min(val.len());
    for (i, b) in val[..n].iter_mut().enumerate() {
        *b = bytes.get(i).copied().unwrap_or(0);
    }
}

/// Translation of `OkToChangeTag()`: Return 1/0 according to whether or
/// not it is permissible to set the tag's value. Note that we allow
/// ImageLength to be changed so that we can append and extend to images.
/// Any other tag may not be altered once writing has commenced, unless its
/// value has no effect on the format of the data that is written.
fn ok_to_change_tag(tif: &mut Tiff<'_>, tag: u32) -> i32 {
    let Some(_fip) = tiff_find_field(tif, tag, TIFF_ANY) else {
        /* unknown tag */
        tiff_error_ext_r!(
            "TIFFSetField",
            "{}: Unknown {}tag {}",
            tif.tif_name,
            if is_pseudo_tag(tag) { "pseudo-" } else { "" },
            tag
        );
        return 0;
    };
    // (the check against TIFF_BEENWRITING: the file is only read)
    1
}

/// Translation of `TIFFSetField()`: Record the value of a field in the
/// internal directory structure.  The field will be written to the file
/// when/if the directory structure is updated.
pub(crate) fn tiff_set_field(tif: &mut Tiff<'_>, tag: u32, args: &[Va<'_>]) -> i32 {
    let mut ap = VaList::new(args);
    tiff_vset_field(tif, tag, &mut ap)
}

/// Translation of `TIFFVSetField()`: Like TIFFSetField, but taking a
/// varargs parameter list.  This routine is useful for building
/// higher-level interfaces on top of the library.
pub(crate) fn tiff_vset_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut VaList<'_>) -> i32 {
    if ok_to_change_tag(tif, tag) != 0 {
        (tif.tif_tagmethods.vsetfield)(tif, tag, ap)
    } else {
        0
    }
}

/// Translation of `_TIFFVGetField()`.
fn _tiff_vget_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut Vec<Gv>) -> i32 {
    let mut ret_val = 1;
    let mut standard_tag = tag;
    let Some(fip) = tiff_find_field(tif, tag, TIFF_ANY) else {
        /* cannot happen since TIFFGetField() already checks it */
        return 0;
    };

    /*
     * We want to force the custom code to be used for custom
     * fields even if the tag happens to match a well known
     * one - important for reinterpreted handling of standard
     * tag values in custom directories (i.e. EXIF)
     */
    if fip.field_bit == FIELD_CUSTOM {
        standard_tag = 0;
    }

    let td = &tif.tif_dir;
    match standard_tag {
        TIFFTAG_SUBFILETYPE => ap.push(Gv::U32(td.td_subfiletype)),
        TIFFTAG_IMAGEWIDTH => ap.push(Gv::U32(td.td_imagewidth)),
        TIFFTAG_IMAGELENGTH => ap.push(Gv::U32(td.td_imagelength)),
        TIFFTAG_BITSPERSAMPLE => ap.push(Gv::U16(td.td_bitspersample)),
        TIFFTAG_COMPRESSION => ap.push(Gv::U16(td.td_compression)),
        TIFFTAG_PHOTOMETRIC => ap.push(Gv::U16(td.td_photometric)),
        TIFFTAG_THRESHHOLDING => ap.push(Gv::U16(td.td_threshholding)),
        TIFFTAG_FILLORDER => ap.push(Gv::U16(td.td_fillorder)),
        TIFFTAG_ORIENTATION => ap.push(Gv::U16(td.td_orientation)),
        TIFFTAG_SAMPLESPERPIXEL => ap.push(Gv::U16(td.td_samplesperpixel)),
        TIFFTAG_ROWSPERSTRIP => ap.push(Gv::U32(td.td_rowsperstrip)),
        TIFFTAG_MINSAMPLEVALUE => ap.push(Gv::U16(td.td_minsamplevalue)),
        TIFFTAG_MAXSAMPLEVALUE => ap.push(Gv::U16(td.td_maxsamplevalue)),
        TIFFTAG_SMINSAMPLEVALUE => {
            if (tif.tif_flags & TIFF_PERSAMPLE) != 0 {
                ap.push(Gv::F64s(td.td_sminsamplevalue.clone()));
            } else {
                /* libtiff historically treats this as a single value. */
                let s = td.td_sminsamplevalue.as_deref().unwrap_or(&[]);
                let mut v = s.first().copied().unwrap_or(0.0);
                for i in 1..td.td_samplesperpixel as usize {
                    if let Some(&x) = s.get(i) {
                        if x < v {
                            v = x;
                        }
                    }
                }
                ap.push(Gv::F64(v));
            }
        }
        TIFFTAG_SMAXSAMPLEVALUE => {
            if (tif.tif_flags & TIFF_PERSAMPLE) != 0 {
                ap.push(Gv::F64s(td.td_smaxsamplevalue.clone()));
            } else {
                /* libtiff historically treats this as a single value. */
                let s = td.td_smaxsamplevalue.as_deref().unwrap_or(&[]);
                let mut v = s.first().copied().unwrap_or(0.0);
                for i in 1..td.td_samplesperpixel as usize {
                    if let Some(&x) = s.get(i) {
                        if x > v {
                            v = x;
                        }
                    }
                }
                ap.push(Gv::F64(v));
            }
        }
        TIFFTAG_XRESOLUTION => ap.push(Gv::F32(td.td_xresolution)),
        TIFFTAG_YRESOLUTION => ap.push(Gv::F32(td.td_yresolution)),
        TIFFTAG_PLANARCONFIG => ap.push(Gv::U16(td.td_planarconfig)),
        TIFFTAG_XPOSITION => ap.push(Gv::F32(td.td_xposition)),
        TIFFTAG_YPOSITION => ap.push(Gv::F32(td.td_yposition)),
        TIFFTAG_RESOLUTIONUNIT => ap.push(Gv::U16(td.td_resolutionunit)),
        TIFFTAG_PAGENUMBER => {
            ap.push(Gv::U16(td.td_pagenumber[0]));
            ap.push(Gv::U16(td.td_pagenumber[1]));
        }
        TIFFTAG_HALFTONEHINTS => {
            ap.push(Gv::U16(td.td_halftonehints[0]));
            ap.push(Gv::U16(td.td_halftonehints[1]));
        }
        TIFFTAG_COLORMAP => {
            ap.push(Gv::U16s(td.td_colormap[0].clone()));
            ap.push(Gv::U16s(td.td_colormap[1].clone()));
            ap.push(Gv::U16s(td.td_colormap[2].clone()));
        }
        TIFFTAG_STRIPOFFSETS | TIFFTAG_TILEOFFSETS => {
            super::tif_dirread::_tiff_fill_striles(tif);
            let td = &tif.tif_dir;
            ap.push(Gv::U64s(td.td_stripoffset_p.clone()));
            if td.td_stripoffset_p.is_none() {
                ret_val = 0;
            }
        }
        TIFFTAG_STRIPBYTECOUNTS | TIFFTAG_TILEBYTECOUNTS => {
            super::tif_dirread::_tiff_fill_striles(tif);
            let td = &tif.tif_dir;
            ap.push(Gv::U64s(td.td_stripbytecount_p.clone()));
            if td.td_stripbytecount_p.is_none() {
                ret_val = 0;
            }
        }
        TIFFTAG_MATTEING => ap.push(Gv::U16(
            (td.td_extrasamples == 1
                && td
                    .td_sampleinfo
                    .as_ref()
                    .is_some_and(|s| s.first() == Some(&EXTRASAMPLE_ASSOCALPHA))) as u16,
        )),
        TIFFTAG_EXTRASAMPLES => {
            ap.push(Gv::U16(td.td_extrasamples));
            ap.push(Gv::U16s(td.td_sampleinfo.clone()));
        }
        TIFFTAG_TILEWIDTH => ap.push(Gv::U32(td.td_tilewidth)),
        TIFFTAG_TILELENGTH => ap.push(Gv::U32(td.td_tilelength)),
        TIFFTAG_TILEDEPTH => ap.push(Gv::U32(td.td_tiledepth)),
        TIFFTAG_DATATYPE => match td.td_sampleformat {
            SAMPLEFORMAT_UINT => ap.push(Gv::U16(DATATYPE_UINT as u16)),
            SAMPLEFORMAT_INT => ap.push(Gv::U16(DATATYPE_INT as u16)),
            SAMPLEFORMAT_IEEEFP => ap.push(Gv::U16(DATATYPE_IEEEFP as u16)),
            SAMPLEFORMAT_VOID => ap.push(Gv::U16(DATATYPE_VOID as u16)),
            _ => {}
        },
        TIFFTAG_SAMPLEFORMAT => ap.push(Gv::U16(td.td_sampleformat)),
        TIFFTAG_IMAGEDEPTH => ap.push(Gv::U32(td.td_imagedepth)),
        TIFFTAG_SUBIFD => {
            ap.push(Gv::U16(td.td_nsubifd));
            ap.push(Gv::U64s(td.td_subifd.clone()));
        }
        TIFFTAG_YCBCRPOSITIONING => ap.push(Gv::U16(td.td_ycbcrpositioning)),
        TIFFTAG_YCBCRSUBSAMPLING => {
            ap.push(Gv::U16(td.td_ycbcrsubsampling[0]));
            ap.push(Gv::U16(td.td_ycbcrsubsampling[1]));
        }
        TIFFTAG_TRANSFERFUNCTION => {
            ap.push(Gv::U16s(td.td_transferfunction[0].clone()));
            if td.td_samplesperpixel as i32 - td.td_extrasamples as i32 > 1 {
                ap.push(Gv::U16s(td.td_transferfunction[1].clone()));
                ap.push(Gv::U16s(td.td_transferfunction[2].clone()));
            } else {
                ap.push(Gv::U16s(None));
                ap.push(Gv::U16s(None));
            }
        }
        TIFFTAG_REFERENCEBLACKWHITE => ap.push(Gv::F32s(td.td_refblackwhite.clone())),
        TIFFTAG_INKNAMES => ap.push(Gv::Bytes(td.td_inknames.clone())),
        TIFFTAG_NUMBEROFINKS => ap.push(Gv::U16(td.td_numberofinks)),
        _ => {
            /*
             * This can happen if multiple images are open
             * with different codecs which have private
             * tags.  The global tag information table may
             * then have tags that are valid for one file
             * but not the other. If the client tries to
             * get a tag that is not valid for the image's
             * codec then we'll arrive here.
             */
            if fip.field_bit != FIELD_CUSTOM {
                tiff_error_ext_r!(
                    "_TIFFVGetField",
                    "{}: Invalid {}tag \"{}\" (not supported by codec)",
                    tif.tif_name,
                    if is_pseudo_tag(tag) { "pseudo-" } else { "" },
                    fip.field_name
                );
                return 0;
            }

            /*
             * Do we have a custom value?
             */
            ret_val = 0;
            if let Some(tv) = td.td_customValues.iter().find(|tv| tv.info.field_tag == tag) {
                let val = tv.value.as_deref().unwrap_or(&[]);
                let b = |n: usize| -> [u8; 8] {
                    let mut a = [0u8; 8];
                    for (i, x) in a.iter_mut().take(n).enumerate() {
                        *x = val.get(i).copied().unwrap_or(0);
                    }
                    a
                };
                if fip.field_passcount != 0 {
                    if fip.field_readcount == TIFF_VARIABLE2 {
                        ap.push(Gv::U32(tv.count as u32));
                    } else {
                        /* Assume TIFF_VARIABLE */
                        ap.push(Gv::U16(tv.count as u16));
                    }
                    ap.push(Gv::Bytes(tv.value.clone()));
                    ret_val = 1;
                } else if fip.field_tag == TIFFTAG_DOTRANGE && fip.field_name == "DotRange" {
                    /* TODO: This is an evil exception and should not have been
                       handled this way ... likely best if we move it into
                       the directory structure with an explicit field in
                       libtiff 4.1 and assign it a FIELD_ value */
                    let a = b(4);
                    ap.push(Gv::U16(u16::from_ne_bytes([a[0], a[1]])));
                    ap.push(Gv::U16(u16::from_ne_bytes([a[2], a[3]])));
                    ret_val = 1;
                } else if fip.field_type == TIFF_ASCII
                    || fip.field_readcount == TIFF_VARIABLE
                    || fip.field_readcount == TIFF_VARIABLE2
                    || fip.field_readcount == TIFF_SPP
                    || tv.count > 1
                {
                    ap.push(Gv::Bytes(tv.value.clone()));
                    ret_val = 1;
                } else {
                    let a = b(8);
                    match fip.field_type {
                        TIFF_BYTE | TIFF_UNDEFINED => {
                            ap.push(Gv::U8(a[0]));
                            ret_val = 1;
                        }
                        TIFF_SBYTE => {
                            ap.push(Gv::I8(a[0] as i8));
                            ret_val = 1;
                        }
                        TIFF_SHORT => {
                            ap.push(Gv::U16(u16::from_ne_bytes([a[0], a[1]])));
                            ret_val = 1;
                        }
                        TIFF_SSHORT => {
                            ap.push(Gv::I16(i16::from_ne_bytes([a[0], a[1]])));
                            ret_val = 1;
                        }
                        TIFF_LONG | TIFF_IFD => {
                            ap.push(Gv::U32(u32::from_ne_bytes([a[0], a[1], a[2], a[3]])));
                            ret_val = 1;
                        }
                        TIFF_SLONG => {
                            ap.push(Gv::I32(i32::from_ne_bytes([a[0], a[1], a[2], a[3]])));
                            ret_val = 1;
                        }
                        TIFF_LONG8 | TIFF_IFD8 => {
                            ap.push(Gv::U64(u64::from_ne_bytes(a)));
                            ret_val = 1;
                        }
                        TIFF_SLONG8 => {
                            ap.push(Gv::I64(i64::from_ne_bytes(a)));
                            ret_val = 1;
                        }
                        TIFF_RATIONAL | TIFF_SRATIONAL => {
                            /*-- Rational2Double: For Rationals evaluate
                             * "set_get_field_type" to determine internal
                             * storage size and return value size. */
                            let tv_size = tiff_field_set_get_size(Some(&fip));
                            if tv_size == 8 {
                                ap.push(Gv::F64(f64::from_ne_bytes(a)));
                                ret_val = 1;
                            } else {
                                /*-- default should be tv_size == 4  */
                                ap.push(Gv::F32(f32::from_ne_bytes([a[0], a[1], a[2], a[3]])));
                                ret_val = 1;
                                /*-- ToDo: After Testing, this should be
                                 * removed and tv_size==4 should be set as
                                 * default. */
                                if tv_size != 4 {
                                    tiff_error_ext_r!(
                                        "_TIFFVGetField",
                                        "Rational2Double: .set_get_field_type in not 4 but {}",
                                        tv_size
                                    );
                                }
                            }
                        }
                        TIFF_FLOAT => {
                            ap.push(Gv::F32(f32::from_ne_bytes([a[0], a[1], a[2], a[3]])));
                            ret_val = 1;
                        }
                        TIFF_DOUBLE => {
                            ap.push(Gv::F64(f64::from_ne_bytes(a)));
                            ret_val = 1;
                        }
                        _ => {
                            /* TIFF_NOTYPE, TIFF_ASCII */
                            ret_val = 0;
                        }
                    }
                }
            }
        }
    }
    ret_val
}

/// Translation of `TIFFGetField()`: Return the value of a field in the
/// internal directory structure.
pub(crate) fn tiff_get_field(tif: &mut Tiff<'_>, tag: u32, out: &mut Vec<Gv>) -> i32 {
    tiff_vget_field(tif, tag, out)
}

/// Translation of `TIFFVGetField()`: Like TIFFGetField, but taking a
/// varargs parameter list.  This routine is useful for building
/// higher-level interfaces on top of the library.
pub(crate) fn tiff_vget_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut Vec<Gv>) -> i32 {
    match tiff_find_field(tif, tag, TIFF_ANY) {
        Some(fip) if is_pseudo_tag(tag) || tiff_field_set(tif, fip.field_bit) => {
            (tif.tif_tagmethods.vgetfield)(tif, tag, ap)
        }
        _ => 0,
    }
}

/// `TIFFGetField(tif, tag, &v)` for a field of one integer value.
pub(crate) fn tiff_get_field_int(tif: &mut Tiff<'_>, tag: u32) -> Option<u64> {
    let mut out = Vec::new();
    if tiff_get_field(tif, tag, &mut out) == 0 {
        return None;
    }
    out.first().and_then(Gv::as_u64)
}

/// Translation of `_TIFFResetTifDirAndInitStrileCounters()`: Reset
/// tif->tif_dir structure to zero and initialize some IFD strile counter
/// and index parameters.
pub(crate) fn _tiff_reset_tif_dir_and_init_strile_counters(td: &mut TIFFDirectory) {
    *td = TIFFDirectory::default();
    td.td_curstrip = NOSTRIP; /* invalid strip = NOSTRIP */
    td.td_row = u32::MAX; /* read/write pre-increment */
    td.td_col = u32::MAX; /* read/write pre-increment */
    td.td_scanlinesize = 0; /* initialize to zero */
    td.td_curtile = NOTILE; /* invalid tile = NOTILE */
    td.td_tilesize = -1; /* invalidate tilezize */
}

/// Translation of `TIFFFreeDirectory()`: Release storage associated with
/// a directory.
pub(crate) fn tiff_free_directory(tif: &mut Tiff<'_>) {
    (tif.tif_cleanup)(tif);
    let td = &mut tif.tif_dir;
    td.td_fieldsset = [0; FIELDSET_ITEMS];
    td.td_sminsamplevalue = None;
    td.td_smaxsamplevalue = None;
    td.td_colormap = [None, None, None];
    td.td_sampleinfo = None;
    td.td_subifd = None;
    td.td_inknames = None;
    td.td_refblackwhite = None;
    td.td_transferfunction = [None, None, None];
    td.td_stripoffset_p = None;
    td.td_stripbytecount_p = None;
    td.td_stripoffsetbyteallocsize = 0;
    tiff_clr_field_bit(tif, FIELD_YCBCRSUBSAMPLING);
    tiff_clr_field_bit(tif, FIELD_YCBCRPOSITIONING);

    /* Cleanup custom tag values */
    let td = &mut tif.tif_dir;
    td.td_customValues = Vec::new();

    td.td_stripoffset_entry = TIFFDirEntry::default();
    td.td_stripbytecount_entry = TIFFDirEntry::default();

    /* Reset some internal parameters for IFD data size checking. */
    td.td_dirdatasize_read = 0;
    td.td_dirdatasize_write = 0;
    if td.td_dirdatasize_offsets.is_some() {
        td.td_dirdatasize_offsets = None;
        td.td_dirdatasize_Noffsets = 0;
    }
    td.td_iswrittentofile = 0;
    /* Note: tif->tif_dir structure is set to zero in TIFFDefaultDirectory() */
}

/// Translation of `TIFFDefaultDirectory()`: Setup a default directory
/// structure. (There is no client tag extender.)
pub(crate) fn tiff_default_directory(tif: &mut Tiff<'_>) -> i32 {
    _tiff_setup_fields(tif, _tiff_get_fields());
    /* Reset tif->tif_dir structure to zero and
     * initialize some IFD strile counter and index parameters. */
    let td = &mut tif.tif_dir;
    _tiff_reset_tif_dir_and_init_strile_counters(td);
    td.td_fillorder = FILLORDER_MSB2LSB;
    td.td_bitspersample = 1;
    td.td_threshholding = THRESHHOLD_BILEVEL;
    td.td_orientation = ORIENTATION_TOPLEFT;
    td.td_samplesperpixel = 1;
    td.td_rowsperstrip = u32::MAX;
    td.td_tilewidth = 0;
    td.td_tilelength = 0;
    td.td_tiledepth = 1;
    td.td_resolutionunit = RESUNIT_INCH;
    td.td_sampleformat = SAMPLEFORMAT_UINT;
    td.td_imagedepth = 1;
    td.td_ycbcrsubsampling[0] = 2;
    td.td_ycbcrsubsampling[1] = 2;
    td.td_ycbcrpositioning = YCBCRPOSITION_CENTERED;
    tif.tif_postdecode = TIFFPostMethod::NoPostDecode;
    tif.tif_tagmethods.vsetfield = _tiff_vset_field;
    tif.tif_tagmethods.vgetfield = _tiff_vget_field;
    /* additional default values */
    let td = &mut tif.tif_dir;
    td.td_planarconfig = PLANARCONFIG_CONTIG;
    td.td_compression = COMPRESSION_NONE;
    td.td_subfiletype = 0;
    td.td_minsamplevalue = 0;
    /* td_bitspersample=1 is always set in TIFFDefaultDirectory().
     * Therefore, td_maxsamplevalue has to be re-calculated in
     * TIFFGetFieldDefaulted(). */
    td.td_maxsamplevalue = 1; /* Default for td_bitspersample=1 */
    td.td_extrasamples = 0;
    td.td_sampleinfo = None;

    /*
     *  Give client code a chance to install their own
     *  tag extensions & methods, prior to compression overloads,
     *  but do some prior cleanup first.
     * (http://trac.osgeo.org/gdal/ticket/5054)
     */
    // (no compatibility fields, no extender)
    let _ = tiff_set_field(tif, TIFFTAG_COMPRESSION, &[Va::Int(COMPRESSION_NONE as i64)]);
    /*
     * NB: The directory is marked dirty as a result of setting
     * up the default compression scheme.  However, this really
     * isn't correct -- we want TIFF_DIRTYDIRECT to be set only
     * if the user does something.  We could just do the setup
     * by hand, but it seems better to use the normal mechanism
     * (i.e. TIFFSetField).
     */
    tif.tif_flags &= !TIFF_DIRTYDIRECT;

    /*
     * As per http://bugzilla.remotesensing.org/show_bug.cgi?id=19
     * we clear the ISTILED flag when setting up a new directory.
     * Should we also be clearing stuff like INSUBIFD?
     */
    tif.tif_flags &= !TIFF_ISTILED;

    1
}

/// The default tag methods (`_TIFFVSetField()`, `_TIFFVGetField()`).
pub(crate) fn default_tag_methods() -> TIFFTagMethods {
    TIFFTagMethods {
        vsetfield: _tiff_vset_field,
        vgetfield: _tiff_vget_field,
    }
}
