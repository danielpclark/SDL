// Rust translation of src/loaders/umx_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Epic Games UMX container loader.

use super::super::common::strlen;
use super::super::hio::*;
use super::common::*;
use super::it_load::LIBXMP_LOADER_IT;
use super::mod_load::LIBXMP_LOADER_MOD;
use super::s3m_load::LIBXMP_LOADER_S3M;
use super::xm_load::LIBXMP_LOADER_XM;
use super::*;

pub(crate) const LIBXMP_LOADER_UMX: FormatLoader = FormatLoader {
    name: "Epic Games UMX",
    test: umx_test,
    loader: umx_load,
};

/* UPKG parsing partially based on Unreal Media Ripper (UMR) v0.3
 * by Andy Ward <wardwh@swbell.net>, with additional fixes & updates
 * by O. Sezer - see git repo at https://github.com/sezero/umr.git
 */
type FciT = i32; /* FCompactIndex */

const UPKG_HDR_TAG: u32 = 0x9e2a83c1;

/// Translation of `struct upkg_hdr` (the generation history isn't used).
#[derive(Default)]
struct UpkgHdr {
    tag: u32, /* UPKG_HDR_TAG */
    file_version: i32,
    pkg_flags: u32,
    name_count: i32,    /* number of names in name table (>= 0) */
    name_offset: i32,   /* offset to name table  (>= 0) */
    export_count: i32,  /* num. exports in export table  (>= 0) */
    export_offset: i32, /* offset to export table (>= 0) */
    import_count: i32,  /* num. imports in export table  (>= 0) */
    import_offset: i32, /* offset to import table (>= 0) */
}

const UMUSIC_IT: i32 = 0;
const UMUSIC_S3M: i32 = 1;
const UMUSIC_XM: i32 = 2;
const UMUSIC_MOD: i32 = 3;
const UMUSIC_WAV: i32 = 4;
const UMUSIC_MP2: i32 = 5;

static MUSTYPE: [&[u8]; 6] = [b"IT", b"S3M", b"XM", b"MOD", b"WAV", b"MP2"];

/// Translation of `get_fci()`: decode an FCompactIndex.
///
/// original documentation by Tim Sweeney was at
/// <http://unreal.epicgames.com/Packages.htm>
/// also see Unreal Wiki:
/// <http://wiki.beyondunreal.com/Legacy:Package_File_Format/Data_Details>
fn get_fci(in_: &[u8], pos: &mut usize) -> FciT {
    let mut size = 1;
    let mut a = (in_[0] & 0x3f) as i32;

    if in_[0] & 0x40 != 0 {
        size += 1;
        a |= ((in_[1] & 0x7f) as i32) << 6;

        if in_[1] & 0x80 != 0 {
            size += 1;
            a |= ((in_[2] & 0x7f) as i32) << 13;

            if in_[2] & 0x80 != 0 {
                size += 1;
                a |= ((in_[3] & 0x7f) as i32) << 20;

                if in_[3] & 0x80 != 0 {
                    size += 1;
                    a |= (((in_[4] & 0x3f) as u32) << 27) as i32;
                }
            }
        }
    }

    if in_[0] & 0x80 != 0 {
        a = a.wrapping_neg();
    }

    *pos += size;

    a
}

/// Translation of `get_objtype()`.
fn get_objtype(f: &mut HioHandle<'_>, ofs: i32, mut type_: i32) -> i32 {
    let mut sig = [0u8; 16];
    loop {
        /* _retry: */
        sig.fill(0);
        hio_seek(f, ofs as i64, SEEK_SET);
        hio_read(&mut sig, 16, 1, f);
        if type_ == UMUSIC_IT {
            if &sig[..4] == b"IMPM" {
                return UMUSIC_IT;
            }
            return -1;
        }
        if type_ == UMUSIC_XM {
            if &sig != b"Extended Module:" {
                return -1;
            }
            hio_read(&mut sig, 16, 1, f);
            if sig[0] != b' ' {
                return -1;
            }
            hio_read(&mut sig, 16, 1, f);
            if sig[5] != 0x1a {
                return -1;
            }
            return UMUSIC_XM;
        }
        if type_ == UMUSIC_MP2 {
            let u = (((sig[0] as u16) << 8) | sig[1] as u16) & 0xFFFE;
            if u == 0xFFFC || u == 0xFFF4 {
                return UMUSIC_MP2;
            }
            return -1;
        }
        if type_ == UMUSIC_WAV {
            if &sig[..4] == b"RIFF" && &sig[8..12] == b"WAVE" {
                return UMUSIC_WAV;
            }
            return -1;
        }

        hio_seek(f, ofs as i64 + 44, SEEK_SET);
        hio_read(&mut sig, 4, 1, f);
        if type_ == UMUSIC_S3M {
            if &sig[..4] == b"SCRM" {
                return UMUSIC_S3M;
            }
            /*return -1;*/
            /* SpaceMarines.umx and Starseek.umx from Return to NaPali
             * report as "s3m" whereas the actual music format is "it" */
            type_ = UMUSIC_IT;
            continue;
        }

        hio_seek(f, ofs as i64 + 1080, SEEK_SET);
        hio_read(&mut sig, 4, 1, f);
        if type_ == UMUSIC_MOD {
            if &sig[..4] == b"M.K." || &sig[..4] == b"M!K!" {
                return UMUSIC_MOD;
            }
            return -1;
        }

        return -1;
    }
}

/// Translation of `read_export()`.
fn read_export(f: &mut HioHandle<'_>, hdr: &UpkgHdr, ofs: &mut i32, objsize: &mut i32) -> i32 {
    let mut buf = [0u8; 40];
    let mut idx = 0;

    hio_seek(f, *ofs as i64, SEEK_SET);
    if hio_read(&mut buf, 4, 10, f) < 10 {
        return -1;
    }

    if hdr.file_version < 40 {
        idx += 8; /* 00 00 00 00 00 00 00 00 */
    }
    if hdr.file_version < 60 {
        idx += 16; /* 81 00 00 00 00 00 FF FF FF FF FF FF FF FF 00 00 */
    }
    let i = idx;
    get_fci(&buf[i..], &mut idx); /* skip junk */
    let i = idx;
    let t = get_fci(&buf[i..], &mut idx); /* type_name */
    if hdr.file_version > 61 {
        idx += 4; /* skip export size */
    }
    let i = idx;
    *objsize = get_fci(&buf[i..], &mut idx);
    *ofs = ofs.wrapping_add(idx as i32); /* offset for real data */

    t /* return type_name index */
}

/// Translation of `read_typname()`.
fn read_typname(f: &mut HioHandle<'_>, hdr: &UpkgHdr, idx: i32, out: &mut [u8; 64]) -> i32 {
    let mut buf = [0u8; 64];

    if idx >= hdr.name_count {
        return -1;
    }
    let mut l: i64 = 0;
    for _ in 0..=idx {
        if hio_seek(f, hdr.name_offset as i64 + l, SEEK_SET) < 0 {
            return -1;
        }
        if hio_read(&mut buf[..63], 1, 63, f) == 0 {
            return -1;
        }
        if hdr.file_version >= 64 {
            let s = buf[0] as i8 as i64; /* numchars *including* terminator */
            if s <= 0 {
                return -1;
            }
            l += s + 5; /* 1 for buf[0], 4 for int32 name_flags */
        } else {
            l += strlen(&buf) as i64;
            l += 5; /* 1 for terminator, 4 for int32 name_flags */
        }
    }

    let src = if hdr.file_version >= 64 {
        &buf[1..]
    } else {
        &buf[..]
    };
    let n = strlen(src);
    out[..n].copy_from_slice(&src[..n]);
    out[n] = b'\0';
    0
}

/// Translation of `umx_strupr()`.
fn umx_strupr(str_: &mut [u8]) {
    for c in str_.iter_mut() {
        if *c == 0 {
            break;
        }
        c.make_ascii_uppercase();
    }
}

/// Translation of `probe_umx()`.
fn probe_umx(f: &mut HioHandle<'_>, hdr: &UpkgHdr, ofs: &mut i32, objsize: &mut i32) -> i32 {
    let mut buf = [0u8; 64];

    let mut idx = 0;
    let fsiz = hio_size(f);

    if hdr.name_offset as i64 >= fsiz
        || hdr.export_offset as i64 >= fsiz
        || hdr.import_offset as i64 >= fsiz
    {
        return -1;
    }

    /* Find the offset and size of the first IT, S3M or XM
     * by parsing the exports table. The umx files should
     * have only one export. Kran32.umx from Unreal has two,
     * but both pointing to the same music. */
    if hdr.export_offset as i64 >= fsiz {
        return -1;
    }
    hio_seek(f, hdr.export_offset as i64, SEEK_SET);
    hio_read(&mut buf, 1, 64, f);

    let i = idx;
    get_fci(&buf[i..], &mut idx); /* skip class_index */
    let i = idx;
    get_fci(&buf[i..], &mut idx); /* skip super_index */
    if hdr.file_version >= 60 {
        idx += 4; /* skip int32 package_index */
    }
    let i = idx;
    get_fci(&buf[i..], &mut idx); /* skip object_name */
    idx += 4; /* skip int32 object_flags */

    let i = idx;
    let mut s = get_fci(&buf[i..], &mut idx); /* get serial_size */
    if s <= 0 {
        return -1;
    }
    let i = idx;
    let mut pos = get_fci(&buf[i..], &mut idx); /* get serial_offset */
    if pos < 0 || pos as i64 > fsiz - 40 {
        return -1;
    }

    let mut t = read_export(f, hdr, &mut pos, &mut s);
    if t < 0 {
        return -1;
    }
    if s <= 0 || s as i64 > fsiz - pos as i64 {
        return -1;
    }

    if read_typname(f, hdr, t, &mut buf) < 0 {
        return -1;
    }
    umx_strupr(&mut buf);
    let name = &buf[..strlen(&buf)];
    let Some(i) = MUSTYPE.iter().position(|&ty| ty == name) else {
        return -1;
    };
    t = i as i32;
    t = get_objtype(f, pos, t);
    if t < 0 {
        return -1;
    }

    *ofs = pos;
    *objsize = s;
    t
}

/// Translation of `probe_header()`.
fn probe_header(f: &mut HioHandle<'_>, hdr: &mut UpkgHdr) -> i32 {
    hdr.tag = hio_read32l(f);
    hdr.file_version = hio_read32l(f) as i32;
    hdr.pkg_flags = hio_read32l(f);
    hdr.name_count = hio_read32l(f) as i32;
    hdr.name_offset = hio_read32l(f) as i32;
    hdr.export_count = hio_read32l(f) as i32;
    hdr.export_offset = hio_read32l(f) as i32;
    hdr.import_count = hio_read32l(f) as i32;
    hdr.import_offset = hio_read32l(f) as i32;
    let _ = hdr.pkg_flags;

    if hdr.tag != UPKG_HDR_TAG {
        return -1;
    }
    if hdr.name_count < 0
        || hdr.export_count < 0
        || hdr.import_count < 0
        || hdr.name_offset < 36
        || hdr.export_offset < 36
        || hdr.import_offset < 36
    {
        return -1;
    }

    /* (upstream keeps a disabled list of known upkg versions here:
     * "no need being overzealous") */
    0
}

/// Translation of `process_upkg()`.
fn process_upkg(f: &mut HioHandle<'_>, ofs: &mut i32, objsize: &mut i32) -> i32 {
    let mut header = UpkgHdr::default();

    if probe_header(f, &mut header) < 0 {
        return -1;
    }
    probe_umx(f, &header, ofs, objsize)
}

/// Translation of `umx_test()`.
fn umx_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut ofs = 0;
    let mut size = 0;

    let type_ = process_upkg(f, &mut ofs, &mut size);
    hio_error(f);
    if type_ < 0 {
        return -1;
    }

    ofs = ofs.wrapping_add(start); /* FIXME? */
    match type_ {
        UMUSIC_IT => {
            hio_seek(f, ofs as i64 + 4, SEEK_SET);
            libxmp_read_title(f, t, 26);
            0
        }
        UMUSIC_S3M => {
            hio_seek(f, ofs as i64, SEEK_SET);
            libxmp_read_title(f, t, 28);
            0
        }
        UMUSIC_XM => {
            hio_seek(f, ofs as i64 + 17, SEEK_SET);
            libxmp_read_title(f, t, 20);
            0
        }
        UMUSIC_MOD => {
            hio_seek(f, ofs as i64, SEEK_SET);
            libxmp_read_title(f, t, 20);
            0
        }
        _ => -1,
    }
}

/// Translation of `umx_load()`.
fn umx_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut ofs = 0;
    let mut size = 0;

    let type_ = process_upkg(f, &mut ofs, &mut size);
    hio_error(f);
    if type_ < 0 {
        return -1;
    }

    ofs = ofs.wrapping_add(start); /* FIXME? */
    hio_seek(f, ofs as i64, SEEK_SET);
    match type_ {
        UMUSIC_IT => (LIBXMP_LOADER_IT.loader)(m, f, ofs),
        UMUSIC_S3M => (LIBXMP_LOADER_S3M.loader)(m, f, ofs),
        UMUSIC_XM => (LIBXMP_LOADER_XM.loader)(m, f, ofs),
        UMUSIC_MOD => (LIBXMP_LOADER_MOD.loader)(m, f, ofs),
        _ => -1,
    }
}
