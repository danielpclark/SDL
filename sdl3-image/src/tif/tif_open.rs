// Rust translation of libtiff/tif_open.c and libtiff/tif_close.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! TIFF Library.
//!
//! Opening a file through client procedures for reading
//! (`TIFFClientOpen()` without open options), and closing it. Writing
//! (the 'w' and 'a' modes) is left out, as is memory mapping: SDL_image's
//! map procedure maps nothing (and it opens with 'm' anyway). The
//! memory limits of the open options don't apply (there are none).

use std::collections::HashMap;

use super::tif_compress::_tiff_set_default_compression_state;
use super::tif_dir::{
    _tiff_reset_tif_dir_and_init_strile_counters, default_tag_methods, tiff_default_directory,
    tiff_free_directory, TIFFDirectory,
};
use super::tif_dirread::tiff_read_directory;
use super::tif_error::tiff_error_ext_r;
use super::tiff::*;
use super::tiffiop::{
    TIFFPostMethod, TifData, Tiff, TiffClient, O_CREAT, O_RDONLY, O_RDWR, O_TRUNC,
    TIFF_BIGTIFF, TIFF_DEFERSTRILELOAD, TIFF_FILLORDER, TIFF_HEADERONLY,
    TIFF_LAZYSTRILELOAD_ASKED, TIFF_MAPPED, TIFF_MYBUFFER, TIFF_NON_EXISTENT_DIR_NUMBER,
    TIFF_STRIPCHOP, TIFF_SWAB,
};

/// `STRIPCHOP_DEFAULT` (SDL_image's build enables strip chopping).
const STRIPCHOP_DEFAULT: u32 = TIFF_STRIPCHOP;

/// Translation of `_TIFFgetMode()`.
fn _tiff_get_mode(mode: &str, module: &str) -> i32 {
    let mut m = -1;

    let b = mode.as_bytes();
    match b.first() {
        Some(b'r') => {
            m = O_RDONLY;
            if b.get(1) == Some(&b'+') {
                m = O_RDWR;
            }
        }
        Some(b'w') | Some(b'a') => {
            m = O_RDWR | O_CREAT;
            if b[0] == b'w' {
                m |= O_TRUNC;
            }
        }
        _ => {
            tiff_error_ext_r!(module, "\"{}\": Bad mode", mode);
        }
    }
    m
}

/// Translation of `TIFFClientOpen()` (`TIFFClientOpenExt()` without open
/// options) for reading: the handle, or `None` (reported) when the file
/// can't be opened.
pub(crate) fn tiff_client_open<'a>(
    name: &str,
    mode: &str,
    clientdata: &'a mut dyn TiffClient,
) -> Option<Tiff<'a>> {
    const MODULE: &str = "TIFFClientOpenExt";

    let m = _tiff_get_mode(mode, MODULE);
    if m == -1 {
        return None;
    }
    if m != O_RDONLY {
        // (writing isn't translated)
        tiff_error_ext_r!(name, "Cannot read TIFF header");
        return None;
    }
    let mut tif = Tiff {
        tif_name: name.to_string(),
        tif_mode: m & !(O_CREAT | O_TRUNC),
        tif_flags: 0,
        tif_diroff: 0,
        tif_nextdiroff: 0,
        tif_lastdiroff: 0,
        tif_map_dir_offset_to_number: None::<HashMap<u64, u32>>,
        tif_map_dir_number_to_offset: None::<HashMap<u32, u64>>,
        tif_setdirectory_force_absolute: 0,
        tif_dir: TIFFDirectory::default(),
        tif_header: [0; 16],
        tif_header_size: 0,
        tif_curdir: TIFF_NON_EXISTENT_DIR_NUMBER, /* non-existent directory */
        tif_curdircount: TIFF_NON_EXISTENT_DIR_NUMBER,
        tif_curoff: 0,
        tif_decodestatus: 0,
        tif_fixuptags: |_| 1,
        tif_setupdecode: |_| 1,
        tif_predecode: |_, _| 1,
        tif_encodestatus: 0,
        tif_decoderow: |_, _, _, _| 0,
        tif_decodestrip: |_, _, _, _| 0,
        tif_decodetile: |_, _, _, _| 0,
        tif_close: |_| {},
        tif_seek: |_, _| 0,
        tif_cleanup: |_| {},
        tif_getmaxcompressionratio: |_| 0,
        tif_data: TifData::None,
        tif_rawdata: Vec::new(),
        tif_rawdatasize: 0,
        tif_rawdataoff: 0,
        tif_rawdataloaded: 0,
        tif_rawcp: 0,
        tif_rawcc: 0,
        tif_clientdata: clientdata,
        tif_postdecode: TIFFPostMethod::NoPostDecode,
        tif_fields: Vec::new(),
        tif_tagmethods: default_tag_methods(),
        tif_warn_about_unknown_tags: 0,
    };

    /* Reset tif->tif_dir structure to zero and
     * initialize some IFD strile counter and index parameters. */
    _tiff_reset_tif_dir_and_init_strile_counters(&mut tif.tif_dir);

    _tiff_set_default_compression_state(&mut tif); /* setup default state */
    /*
     * Default is to return data MSB2LSB and enable the
     * use of memory-mapped files and strip chopping when
     * a file is opened read-only.
     */
    tif.tif_flags = FILLORDER_MSB2LSB as u32;
    if m == O_RDONLY {
        tif.tif_flags |= TIFF_MAPPED;
    }

    if m == O_RDONLY || m == O_RDWR {
        tif.tif_flags |= STRIPCHOP_DEFAULT;
    }

    /*
     * Process library-specific flags in the open mode string.
     * The following flags may be used to control intrinsic library
     * behavior that may or may not be desirable (usually for
     * compatibility with some application that claims to support
     * TIFF but only supports some brain dead idea of what the
     * vendor thinks TIFF is):
     *
     * 'l' use little-endian byte order for creating a file
     * 'b' use big-endian byte order for creating a file
     * 'L' read/write information using LSB2MSB bit order
     * 'B' read/write information using MSB2LSB bit order
     * 'H' read/write information using host bit order
     * 'M' enable use of memory-mapped files when supported
     * 'm' disable use of memory-mapped files
     * 'C' enable strip chopping support when reading
     * 'c' disable strip chopping support
     * 'h' read TIFF header only, do not load the first IFD
     * '4' ClassicTIFF for creating a file (default)
     * '8' BigTIFF for creating a file
     * 'D' enable use of deferred strip/tile offset/bytecount array loading.
     * 'O' on-demand loading of values instead of whole array loading (implies
     * D)
     *
     * (The rest of the C's comment is on the flags for writing.)
     */
    for &c in mode.as_bytes() {
        match c {
            b'b' => {
                if cfg!(target_endian = "little") && (m & O_CREAT) != 0 {
                    tif.tif_flags |= TIFF_SWAB;
                }
            }
            b'l' => {
                if cfg!(target_endian = "big") && (m & O_CREAT) != 0 {
                    tif.tif_flags |= TIFF_SWAB;
                }
            }
            b'B' => {
                tif.tif_flags = (tif.tif_flags & !TIFF_FILLORDER) | FILLORDER_MSB2LSB as u32;
            }
            b'L' => {
                tif.tif_flags = (tif.tif_flags & !TIFF_FILLORDER) | FILLORDER_LSB2MSB as u32;
            }
            b'H' => {
                super::tif_error::tiff_warning_ext_r!(
                    name,
                    "H(ost) mode is deprecated. Since libtiff 4.5.1, it is an alias of 'B' / FILLORDER_MSB2LSB."
                );
                tif.tif_flags = (tif.tif_flags & !TIFF_FILLORDER) | FILLORDER_MSB2LSB as u32;
            }
            b'M' => {
                if m == O_RDONLY {
                    tif.tif_flags |= TIFF_MAPPED;
                }
            }
            b'm' => {
                if m == O_RDONLY {
                    tif.tif_flags &= !TIFF_MAPPED;
                }
            }
            b'C' => {
                if m == O_RDONLY {
                    tif.tif_flags |= TIFF_STRIPCHOP;
                }
            }
            b'c' => {
                if m == O_RDONLY {
                    tif.tif_flags &= !TIFF_STRIPCHOP;
                }
            }
            b'h' => tif.tif_flags |= TIFF_HEADERONLY,
            b'8' => {
                if (m & O_CREAT) != 0 {
                    tif.tif_flags |= TIFF_BIGTIFF;
                }
            }
            b'D' => tif.tif_flags |= TIFF_DEFERSTRILELOAD,
            b'O' => {
                if m == O_RDONLY {
                    tif.tif_flags |= TIFF_LAZYSTRILELOAD_ASKED | TIFF_DEFERSTRILELOAD;
                }
            }
            _ => {}
        }
    }

    /*
     * Read in TIFF header.
     */
    let mut header = [0u8; SIZEOF_TIFF_HEADER_CLASSIC];
    if !tif.read_ok(&mut header) {
        // (tif_mode is O_RDONLY: the header isn't written)
        tiff_error_ext_r!(name, "Cannot read TIFF header");
        tiff_cleanup(tif);
        return None;
    }
    tif.tif_header[..SIZEOF_TIFF_HEADER_CLASSIC].copy_from_slice(&header);

    /*
     * Setup the byte order handling according to the opened file for reading.
     */
    let magic = tif.header_magic();
    let mdi_magic = if cfg!(target_endian = "big") {
        MDI_BIGENDIAN
    } else {
        MDI_LITTLEENDIAN
    };
    if magic != TIFF_BIGENDIAN && magic != TIFF_LITTLEENDIAN && magic != mdi_magic {
        tiff_error_ext_r!(
            name,
            "Not a TIFF or MDI file, bad magic number {} (0x{:x})",
            magic,
            magic
        );
        tiff_cleanup(tif);
        return None;
    }
    if magic == TIFF_BIGENDIAN {
        if cfg!(target_endian = "little") {
            tif.tif_flags |= TIFF_SWAB;
        }
    } else if cfg!(target_endian = "big") {
        tif.tif_flags |= TIFF_SWAB;
    }
    let swab = (tif.tif_flags & TIFF_SWAB) != 0;
    if swab {
        tif.tif_header.swap(2, 3);
    }
    let version = u16::from_ne_bytes([tif.tif_header[2], tif.tif_header[3]]);
    if version != TIFF_VERSION_CLASSIC && version != TIFF_VERSION_BIG {
        tiff_error_ext_r!(
            name,
            "Not a TIFF file, bad version number {} (0x{:x})",
            version,
            version
        );
        tiff_cleanup(tif);
        return None;
    }
    if version == TIFF_VERSION_CLASSIC {
        if swab {
            tif.tif_header[4..8].reverse();
        }
        tif.tif_header_size = SIZEOF_TIFF_HEADER_CLASSIC as u16;
    } else {
        let mut rest = [0u8; SIZEOF_TIFF_HEADER_BIG - SIZEOF_TIFF_HEADER_CLASSIC];
        if !tif.read_ok(&mut rest) {
            tiff_error_ext_r!(name, "Cannot read TIFF header");
            tiff_cleanup(tif);
            return None;
        }
        tif.tif_header[SIZEOF_TIFF_HEADER_CLASSIC..].copy_from_slice(&rest);
        if swab {
            tif.tif_header.swap(4, 5);
            tif.tif_header[8..16].reverse();
        }
        let offsetsize = u16::from_ne_bytes([tif.tif_header[4], tif.tif_header[5]]);
        if offsetsize != 8 {
            tiff_error_ext_r!(
                name,
                "Not a TIFF file, bad BigTIFF offsetsize {} (0x{:x})",
                offsetsize,
                offsetsize
            );
            tiff_cleanup(tif);
            return None;
        }
        let unused = u16::from_ne_bytes([tif.tif_header[6], tif.tif_header[7]]);
        if unused != 0 {
            tiff_error_ext_r!(
                name,
                "Not a TIFF file, bad BigTIFF unused {} (0x{:x})",
                unused,
                unused
            );
            tiff_cleanup(tif);
            return None;
        }
        tif.tif_header_size = SIZEOF_TIFF_HEADER_BIG as u16;
        tif.tif_flags |= TIFF_BIGTIFF;
    }
    tif.tif_flags |= TIFF_MYBUFFER;
    tif.tif_rawcp = 0;
    tif.tif_rawdata = Vec::new();
    tif.tif_rawdatasize = 0;
    tif.tif_rawdataoff = 0;
    tif.tif_rawdataloaded = 0;

    // case 'r':
    let h = &tif.tif_header;
    if (tif.tif_flags & TIFF_BIGTIFF) == 0 {
        tif.tif_nextdiroff = u32::from_ne_bytes([h[4], h[5], h[6], h[7]]) as u64;
    } else {
        let mut d = [0u8; 8];
        d.copy_from_slice(&h[8..16]);
        tif.tif_nextdiroff = u64::from_ne_bytes(d);
    }
    /*
     * Try to use a memory-mapped file if the client
     * has not explicitly suppressed usage with the
     * 'm' flag in the open mode (see above).
     */
    if (tif.tif_flags & TIFF_MAPPED) != 0 {
        // (the map procedure maps nothing)
        tif.tif_flags &= !TIFF_MAPPED;
    }
    /*
     * Sometimes we do not want to read the first directory (for
     * example, it may be broken) and want to proceed to other
     * directories. I this case we use the TIFF_HEADERONLY flag to open
     * file and return immediately after reading TIFF header.
     * However, the pointer to TIFFSetField() and TIFFGetField()
     * (i.e. tif->tif_tagmethods.vsetfield and
     * tif->tif_tagmethods.vgetfield) need to be initialized, which is
     * done in TIFFDefaultDirectory().
     */
    if (tif.tif_flags & TIFF_HEADERONLY) != 0 {
        if tiff_default_directory(&mut tif) == 0 {
            tiff_cleanup(tif);
            return None;
        }
        return Some(tif);
    }

    /*
     * Setup initial directory.
     */
    if tiff_read_directory(&mut tif) != 0 {
        return Some(tif);
    }
    // bad:
    tif.tif_mode = O_RDONLY; /* XXX avoid flush */
    tiff_cleanup(tif);
    None
}

/// Auxiliary function to free the TIFF structure.
/// Translation of `TIFFCleanup()`.
pub(crate) fn tiff_cleanup(mut tif: Tiff<'_>) {
    /*
     * Flush buffered data and directory (if dirty).
     */
    // (the handle is read-only: there's nothing to flush)
    tiff_free_directory(&mut tif);

    // (_TIFFCleanupIFDOffsetAndNumberMaps(), the buffers and the fields
    // are freed when the handle is dropped)
    drop(tif);
}

/// Close a previously opened TIFF file.
///
/// TIFFClose closes a file that was previously opened with TIFFOpen().
/// Any buffered data are flushed to the file, including the contents of
/// the current directory (if modified); and all resources are reclaimed.
/// Translation of `TIFFClose()` (SDL_image's close procedure does
/// nothing).
pub(crate) fn tiff_close(tif: Tiff<'_>) {
    tiff_cleanup(tif);
}
