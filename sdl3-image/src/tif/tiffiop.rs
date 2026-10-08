// Rust translation of libtiff/tiffiop.h from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! ``Library-private'' definitions.
//!
//! The `TIFF` handle of a file opened for reading through client
//! procedures (`TIFFClientOpen()`, the only way SDL_image opens one): the
//! procedures are the [`TiffClient`] the handle borrows, the writing state
//! is left out, and the raw data buffer is a vector with the current
//! position (`tif_rawcp`) an index into it. The compression scheme's
//! private data (`tif_data`) is a [`TifData`] of the codecs translated.

#![allow(dead_code)] // (definitions the reading path doesn't all use)

use std::collections::HashMap;

use super::tif_dir::{TIFFDirectory, TIFFTagMethods};
use super::tif_fax3::Fax3CodecState;
use super::tif_luv::LogLuvState;
use super::tif_lzw::LZWCodecState;
use super::tiffio::TIFFField;

/// `tmsize_t`: a signed size, the width of a pointer.
pub(crate) type TmSize = isize;
/// Translation of `TIFF_TMSIZE_T_MAX`.
pub(crate) const TIFF_TMSIZE_T_MAX: TmSize = TmSize::MAX;

pub(crate) const STRIP_SIZE_DEFAULT: u64 = 8192;

pub(crate) const TIFF_MAX_DIR_COUNT: u32 = 1048576;

pub(crate) const TIFF_NON_EXISTENT_DIR_NUMBER: u32 = u32::MAX;

/// The open modes of `<fcntl.h>` the library compares with.
pub(crate) const O_RDONLY: i32 = 0;
pub(crate) const O_WRONLY: i32 = 1;
pub(crate) const O_RDWR: i32 = 2;
pub(crate) const O_CREAT: i32 = 0o100;
pub(crate) const O_TRUNC: i32 = 0o1000;

/// `SEEK_SET`, `SEEK_CUR` and `SEEK_END` (the values of `SDL_IOWhence`).
pub(crate) const SEEK_SET: i32 = 0;
pub(crate) const SEEK_END: i32 = 2;

/// The client procedures of `TIFFClientOpen()` this handle uses: read,
/// seek and size (there is no writing, and the close, map and unmap
/// procedures SDL_image passes do nothing).
pub(crate) trait TiffClient {
    /// `TIFFReadWriteProc` for reading: the bytes read, or -1.
    fn read(&mut self, buf: &mut [u8]) -> TmSize;
    /// `TIFFSeekProc`: the new offset, or `(toff_t)-1`.
    fn seek(&mut self, off: u64, whence: i32) -> u64;
    /// `TIFFSizeProc`: the size of the file.
    fn size(&mut self) -> u64;
}

// Typedefs for ``method pointers'' used internally.
pub(crate) type TIFFVoidMethod = fn(&mut Tiff<'_>);
pub(crate) type TIFFBoolMethod = fn(&mut Tiff<'_>) -> i32;
pub(crate) type TIFFPreMethod = fn(&mut Tiff<'_>, u16) -> i32;
pub(crate) type TIFFCodeMethod = fn(&mut Tiff<'_>, &mut [u8], TmSize, u16) -> i32;
pub(crate) type TIFFSeekMethod = fn(&mut Tiff<'_>, u32) -> i32;
pub(crate) type TIFFGetMaxCompressionRatioMethod = fn(&Tiff<'_>) -> u64;

/// `TIFFPostMethod`: the post-decoding routines (byte swapping), named
/// rather than pointed to, since the library compares them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TIFFPostMethod {
    /// `_TIFFNoPostDecode`
    NoPostDecode,
    /// `_TIFFSwab16BitData`
    Swab16BitData,
    /// `_TIFFSwab24BitData`
    Swab24BitData,
    /// `_TIFFSwab32BitData`
    Swab32BitData,
    /// `_TIFFSwab64BitData`
    Swab64BitData,
}

/// `tif_data`: the compression scheme's private data.
#[derive(Default)]
pub(crate) enum TifData {
    /// No private data (none, PackBits, NeXT, ThunderScan).
    #[default]
    None,
    /// CCITT Group 3 and 4 (and the RLE variants).
    Fax3(Box<Fax3CodecState>),
    /// LZW, with its predictor.
    Lzw(Box<LZWCodecState>),
    /// SGI LogL and LogLuv.
    LogLuv(Box<LogLuvState>),
}

/// A `TIFF` file handle (`struct tiff`).
pub(crate) struct Tiff<'a> {
    pub(crate) tif_name: String, /* name of open file */
    pub(crate) tif_mode: i32,    /* open mode (O_*) */
    pub(crate) tif_flags: u32,

    pub(crate) tif_diroff: u64,     /* file offset of current directory */
    pub(crate) tif_nextdiroff: u64, /* file offset of following directory */
    pub(crate) tif_lastdiroff: u64, /* file offset of last directory written so far */
    // (`TIFFHashSet`s of `TIFFOffsetAndDirNumber`s in the C)
    pub(crate) tif_map_dir_offset_to_number: Option<HashMap<u64, u32>>,
    pub(crate) tif_map_dir_number_to_offset: Option<HashMap<u32, u64>>,
    pub(crate) tif_setdirectory_force_absolute: i32, /* switch between relative and absolute
                                                     stepping in TIFFSetDirectory() */
    pub(crate) tif_dir: TIFFDirectory, /* internal rep of current directory */
    /// `TIFFHeaderUnion tif_header`: the file's header block, as the bytes
    /// of the union (Classic/BigTIFF), in host byte order once swabbed.
    pub(crate) tif_header: [u8; 16],
    pub(crate) tif_header_size: u16, /* file's header block and its length */

    /* There are IFDs in the file and an "active" IFD in memory,
     * from which fields are "set" and "get".
     * tif_curdir is set to:
     *   a) TIFF_NON_EXISTENT_DIR_NUMBER if there is no IFD in the file
     *      or the state is unknown,
     *      or the last read (i.e. TIFFFetchDirectory()) failed,
     *      or a custom directory was written.
     *   b) IFD index of last IFD written in the file. In this case the
     *      active IFD is a new (empty) one and tif_diroff is zero.
     *      If writing fails, tif_curdir is not changed.
     *   c) IFD index of IFD read from file into memory (=active IFD),
     *      even if IFD is corrupt and TIFFReadDirectory() returns 0.
     *      Then tif_diroff contains the offset of the IFD in the file.
     *   d) IFD index 0, whenever a custom directory or an unchained SubIFD
     *      was read. */
    pub(crate) tif_curdir: u32, /* current directory (index) */
    /* tif_curdircount: number of directories (main-IFDs) in file:
     * - TIFF_NON_EXISTENT_DIR_NUMBER means 'dont know number of IFDs'.
     * - 0 means 'empty file opened for writing, but no IFD written yet' */
    pub(crate) tif_curdircount: u32,
    pub(crate) tif_curoff: u64, /* current offset for read/write */
    /* compression scheme hooks */
    pub(crate) tif_decodestatus: i32,
    pub(crate) tif_fixuptags: TIFFBoolMethod, /* called in TIFFReadDirectory */
    pub(crate) tif_setupdecode: TIFFBoolMethod, /* called once before predecode */
    pub(crate) tif_predecode: TIFFPreMethod,  /* pre- row/strip/tile decoding */
    pub(crate) tif_encodestatus: i32,
    pub(crate) tif_decoderow: TIFFCodeMethod, /* scanline decoding routine */
    pub(crate) tif_decodestrip: TIFFCodeMethod, /* strip decoding routine */
    pub(crate) tif_decodetile: TIFFCodeMethod, /* tile decoding routine */
    pub(crate) tif_close: TIFFVoidMethod,     /* cleanup-on-close routine */
    pub(crate) tif_seek: TIFFSeekMethod,      /* position within a strip routine */
    pub(crate) tif_cleanup: TIFFVoidMethod,   /* cleanup state routine */
    /* returns maximum compression ratio for current compression method */
    pub(crate) tif_getmaxcompressionratio: TIFFGetMaxCompressionRatioMethod,
    pub(crate) tif_data: TifData, /* compression scheme private data */
    /* input/output buffering */
    pub(crate) tif_rawdata: Vec<u8>,      /* raw data buffer */
    pub(crate) tif_rawdatasize: TmSize,   /* # of bytes in raw data buffer */
    pub(crate) tif_rawdataoff: TmSize,    /* rawdata offset within strip */
    pub(crate) tif_rawdataloaded: TmSize, /* amount of data in rawdata */
    pub(crate) tif_rawcp: usize,          /* current spot in raw buffer */
    pub(crate) tif_rawcc: TmSize,         /* bytes unread from raw buffer */
    /* input/output callback methods */
    pub(crate) tif_clientdata: &'a mut dyn TiffClient, /* callback parameter */
    /* post-decoding support */
    pub(crate) tif_postdecode: TIFFPostMethod, /* post decoding routine */
    /* tag support */
    pub(crate) tif_fields: Vec<TIFFField>, /* sorted table of registered tags */
    pub(crate) tif_tagmethods: TIFFTagMethods, /* tag get/set/print routines */
    pub(crate) tif_warn_about_unknown_tags: i32,
}

pub(crate) const TIFF_FILLORDER: u32 = 0x00003; /* natural bit fill order for machine */
pub(crate) const TIFF_DIRTYHEADER: u32 = 0x00004; /* header must be written on close */
pub(crate) const TIFF_DIRTYDIRECT: u32 = 0x00008; /* current directory must be written */
pub(crate) const TIFF_BUFFERSETUP: u32 = 0x00010; /* data buffers setup */
pub(crate) const TIFF_CODERSETUP: u32 = 0x00020; /* encoder/decoder setup done */
pub(crate) const TIFF_BEENWRITING: u32 = 0x00040; /* written 1+ scanlines to file */
pub(crate) const TIFF_SWAB: u32 = 0x00080; /* byte swap file information */
pub(crate) const TIFF_NOBITREV: u32 = 0x00100; /* inhibit bit reversal logic */
pub(crate) const TIFF_MYBUFFER: u32 = 0x00200; /* my raw data buffer; free on close */
pub(crate) const TIFF_ISTILED: u32 = 0x00400; /* file is tile, not strip- based */
pub(crate) const TIFF_MAPPED: u32 = 0x00800; /* file is mapped into memory */
pub(crate) const TIFF_POSTENCODE: u32 = 0x01000; /* need call to postencode routine */
pub(crate) const TIFF_INSUBIFD: u32 = 0x02000; /* currently writing a subifd */
pub(crate) const TIFF_UPSAMPLED: u32 = 0x04000; /* library is doing data up-sampling */
pub(crate) const TIFF_STRIPCHOP: u32 = 0x08000; /* enable strip chopping support */
pub(crate) const TIFF_HEADERONLY: u32 = 0x10000; /* read header only, do not process the first directory */
pub(crate) const TIFF_NOREADRAW: u32 = 0x20000; /* skip reading of raw uncompressed image data */
pub(crate) const TIFF_INCUSTOMIFD: u32 = 0x40000; /* currently writing a custom IFD */
pub(crate) const TIFF_BIGTIFF: u32 = 0x80000; /* read/write bigtiff */
pub(crate) const TIFF_BUF4WRITE: u32 = 0x100000; /* rawcc bytes are for writing */
pub(crate) const TIFF_DIRTYSTRIP: u32 = 0x200000; /* stripoffsets/stripbytecount dirty*/
pub(crate) const TIFF_PERSAMPLE: u32 = 0x400000; /* get/set per sample tags as arrays */
pub(crate) const TIFF_BUFFERMMAP: u32 = 0x800000; /* read buffer (tif_rawdata) points into mmap() memory */
pub(crate) const TIFF_DEFERSTRILELOAD: u32 = 0x1000000; /* defer strip/tile offset/bytecount array loading. */
pub(crate) const TIFF_LAZYSTRILELOAD_DONE: u32 = 0x2000000; /* set when lazy/ondemand loading of strip/tile
                                                            offset/bytecount values has been done. Only used if
                                                            TIFF_DEFERSTRILELOAD is set and in read-only mode */
pub(crate) const TIFF_CHOPPEDUPARRAYS: u32 = 0x4000000; /* set when allocChoppedUpStripArrays() has modified strip
                                                        array */
pub(crate) const TIFF_LAZYSTRILELOAD_ASKED: u32 = 0x8000000; /* set when lazy/ondemand loading of strip/tile
                                                             offset/bytecount values has been requested on opening ('O'
                                                             flag) */

/// Translation of `isPseudoTag()`: is tag value normal or pseudo
pub(crate) fn is_pseudo_tag(t: u32) -> bool {
    t > 0xffff
}

impl Tiff<'_> {
    /// Translation of `isTiled()`.
    pub(crate) fn is_tiled(&self) -> bool {
        (self.tif_flags & TIFF_ISTILED) != 0
    }

    /// Translation of `isMapped()`.
    pub(crate) fn is_mapped(&self) -> bool {
        (self.tif_flags & TIFF_MAPPED) != 0
    }

    /// Translation of `isFillOrder()`.
    pub(crate) fn is_fill_order(&self, o: u16) -> bool {
        (self.tif_flags & o as u32) != 0
    }

    /// Translation of `isUpSampled()`.
    pub(crate) fn is_up_sampled(&self) -> bool {
        (self.tif_flags & TIFF_UPSAMPLED) != 0
    }

    /// Translation of `TIFFReadFile()`.
    pub(crate) fn read_file(&mut self, buf: &mut [u8]) -> TmSize {
        self.tif_clientdata.read(buf)
    }

    /// Translation of `TIFFSeekFile()`.
    pub(crate) fn seek_file(&mut self, off: u64, whence: i32) -> u64 {
        self.tif_clientdata.seek(off, whence)
    }

    /// Translation of `TIFFGetFileSize()`.
    pub(crate) fn get_file_size(&mut self) -> u64 {
        self.tif_clientdata.size()
    }

    /// Translation of `ReadOK()`.
    pub(crate) fn read_ok(&mut self, buf: &mut [u8]) -> bool {
        let size = buf.len() as TmSize;
        self.read_file(buf) == size
    }

    /// Translation of `SeekOK()`.
    pub(crate) fn seek_ok(&mut self, off: u64) -> bool {
        super::tif_aux::_tiff_seek_ok(self, off)
    }

    /// The raw data from the current spot (`tif_rawcp`) on.
    pub(crate) fn rawcp(&self) -> &[u8] {
        self.tif_rawdata.get(self.tif_rawcp..).unwrap_or(&[])
    }

    /// The header's byte order mark (`tif_header.common.tiff_magic`).
    pub(crate) fn header_magic(&self) -> u16 {
        u16::from_ne_bytes([self.tif_header[0], self.tif_header[1]])
    }
}

// NB: the uint32_t casts are to silence certain ANSI-C compilers

/// Translation of `TIFFhowmany_32()`.
pub(crate) fn tiff_howmany_32(x: u32, y: u32) -> u32 {
    if x < 0xffffffffu32.wrapping_sub(y.wrapping_sub(1)) {
        (x.wrapping_add(y.wrapping_sub(1))) / y
    } else {
        0
    }
}

/// Translation of `TIFFhowmany_32_maxuint_compat()`: variant of
/// `TIFFhowmany_32()` that doesn't return 0 if x close to MAXUINT.
/// Caution: `TIFFhowmany_32_maxuint_compat(x,y)*y` might overflow
pub(crate) fn tiff_howmany_32_maxuint_compat(x: u32, y: u32) -> u32 {
    (x / y) + if (x % y) != 0 { 1 } else { 0 }
}

/// Translation of `TIFFhowmany8_32()`.
pub(crate) fn tiff_howmany8_32(x: u32) -> u32 {
    if (x & 0x07) != 0 {
        (x >> 3) + 1
    } else {
        x >> 3
    }
}

/// Translation of `TIFFroundup_32()`.
pub(crate) fn tiff_roundup_32(x: u32, y: u32) -> u32 {
    tiff_howmany_32(x, y).wrapping_mul(y)
}

/// Translation of `TIFFhowmany_64()`.
pub(crate) fn tiff_howmany_64(x: u64, y: u64) -> u64 {
    (x.wrapping_add(y.wrapping_sub(1))) / y
}

/// Translation of `TIFFhowmany8_64()`.
pub(crate) fn tiff_howmany8_64(x: u64) -> u64 {
    if (x & 0x07) != 0 {
        (x >> 3) + 1
    } else {
        x >> 3
    }
}

/// Translation of `TIFFroundup_64()`.
pub(crate) fn tiff_roundup_64(x: u64, y: u64) -> u64 {
    tiff_howmany_64(x, y).wrapping_mul(y)
}

/// Translation of `TIFFSafeMultiply()` for `tmsize_t`: safe multiply
/// which returns zero if there is an *unsigned* integer overflow.
pub(crate) fn tiff_safe_multiply_tmsize(v: TmSize, m: TmSize) -> TmSize {
    if m != 0 && v.wrapping_mul(m) / m == v {
        v.wrapping_mul(m)
    } else {
        0
    }
}

/// A zeroed vector of `n` elements, or `None` when it can't be allocated:
/// `_TIFFmallocExt()`/`_TIFFcallocExt()` (the allocations malloc leaves
/// uninitialized are zeroed here).
pub(crate) fn try_vec<T: Clone + Default>(n: usize) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, T::default());
    Some(v)
}

/// A vector holding a copy of `src`, or `None` when it can't be allocated.
pub(crate) fn try_copy<T: Clone>(src: &[T]) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(src.len()).ok()?;
    v.extend_from_slice(src);
    Some(v)
}

/// `(int32_t)f` as x86-64 converts a float (`cvttss2si`): a value out of
/// range (and NaN), which is undefined behavior in C, gives `INT32_MIN`
/// where Rust's `as` would saturate.
pub(crate) fn c_f32_to_i32(f: f32) -> i32 {
    if f.is_nan() || f >= 2147483648.0 || f < -2147483648.0 {
        i32::MIN
    } else {
        f as i32
    }
}

/// `(int32_t)d` for a double, as [`c_f32_to_i32`] (`cvttsd2si`).
pub(crate) fn c_f64_to_i32(d: f64) -> i32 {
    if d.is_nan() || d >= 2147483648.0 || d <= -2147483649.0 {
        i32::MIN
    } else {
        d as i32
    }
}
