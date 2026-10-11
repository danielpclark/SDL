// Rust translation of src/sfnt/sfdriver.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! High-level SFNT driver interface (body).
//!
//! The `SFNT_Interface` table is not needed: the `truetype` driver calls
//! the `sfnt` module's functions directly.  What remains here are the
//! services and the module class.
//!
//! Not translated yet: `sfnt_get_var_ps_name` (and its helpers
//! `fixed2float`, the MurmurHash3 `murmur_hash_3_128`, and
//! `sfnt_is_alphanumeric`), which build the PostScript names of variation
//! font instances for `FT_Get_Postscript_Name`; SDL_ttf never asks for
//! PostScript names, so `sfnt_get_ps_name` always reads the `name` table.
//! The BDF service (`sfnt_get_charset_id`, `tt_face_find_bdf_prop`) has no
//! service record either, since nothing in SDL_ttf queries BDF properties;
//! the property lookup itself is in [`super::ttbdf`].

use super::super::base::ftmemory::ft_mem_strcpyn;
use super::super::base::ftobjs::*;
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::*;
use super::ttcmap::tt_get_cmap_info;
use super::ttload::{tt_face_load_any, with_stream};
use super::ttpost::tt_face_get_ps_name;

/*
 *
 * SFNT TABLE SERVICE
 *
 */

/// `sfnt_load_table`
fn sfnt_load_table(
    face: &mut FtFace,
    tag: FtULong,
    offset: FtLong,
    buffer: Option<&mut [u8]>,
    length: &mut FtULong,
) -> FtResult<()> {
    let FtFace::Tt(ttface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    with_stream(ttface, |ttface, stream| {
        tt_face_load_any(ttface, stream, tag, offset, buffer, Some(length))
    })
}

/// `get_sfnt_table`
fn get_sfnt_table(face: &FtFace, tag: FtSfntTag) -> Option<FtSfntTable<'_>> {
    let FtFace::Tt(ttface) = face else {
        return None;
    };

    match tag {
        FT_SFNT_HEAD => Some(FtSfntTable::Head(&ttface.header)),

        FT_SFNT_HHEA => Some(FtSfntTable::Hhea(&ttface.horizontal)),

        FT_SFNT_VHEA => {
            if ttface.vertical_info {
                Some(FtSfntTable::Vhea(&ttface.vertical))
            } else {
                None
            }
        }

        FT_SFNT_OS2 => {
            if ttface.os2.version == 0xFFFF {
                None
            } else {
                Some(FtSfntTable::Os2(&ttface.os2))
            }
        }

        FT_SFNT_POST => Some(FtSfntTable::Post(&ttface.postscript)),

        FT_SFNT_MAXP => Some(FtSfntTable::Maxp(&ttface.max_profile)),

        FT_SFNT_PCLT => {
            if ttface.pclt.Version != 0 {
                Some(FtSfntTable::Pclt(&ttface.pclt))
            } else {
                None
            }
        }

        _ => None,
    }
}

/// `sfnt_table_info`; a zero `*tag` on entry stands for C's NULL `tag`
/// (asking for the number of tables).
fn sfnt_table_info(
    face: &FtFace,
    idx: FtUInt,
    tag: &mut FtULong,
    offset: &mut FtULong,
    length: &mut FtULong,
) -> FtResult<()> {
    let FtFace::Tt(ttface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    if *tag == 0 {
        *length = ttface.num_tables as FtULong;
    } else {
        if idx >= ttface.num_tables as FtUInt {
            return Err(FT_ERR_TABLE_MISSING);
        }

        *tag = ttface.dir_tables[idx as usize].Tag;
        *offset = ttface.dir_tables[idx as usize].Offset;
        *length = ttface.dir_tables[idx as usize].Length;
    }

    Ok(())
}

/// `sfnt_service_sfnt_table`
pub static SFNT_SERVICE_SFNT_TABLE: FtServiceSfntTableRec = FtServiceSfntTableRec {
    load_table: sfnt_load_table, /* FT_SFNT_TableLoadFunc load_table */
    get_table: get_sfnt_table,   /* FT_SFNT_TableGetFunc  get_table  */
    table_info: sfnt_table_info, /* FT_SFNT_TableInfoFunc table_info */
};

/* TT_CONFIG_OPTION_POSTSCRIPT_NAMES */

/*
 *
 * GLYPH DICT SERVICE
 *
 */

/// `sfnt_get_glyph_name`
fn sfnt_get_glyph_name(face: &mut FtFace, glyph_index: FtUInt, buffer: &mut [u8]) -> FtResult<()> {
    let FtFace::Tt(ttface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    let gname = with_stream(ttface, |ttface, stream| {
        tt_face_get_ps_name(ttface, stream, glyph_index)
    })?;
    let max = buffer.len();
    ft_mem_strcpyn(buffer, &gname, max);

    Ok(())
}

/// `sfnt_get_name_index`
fn sfnt_get_name_index(face: &mut FtFace, glyph_name: &[u8]) -> FtUInt {
    let FtFace::Tt(ttface) = face else {
        return 0;
    };

    let mut max_gid = FtUInt::MAX;

    if ttface.root.num_glyphs < 0 {
        return 0;
    } else if (ttface.root.num_glyphs as FtULong) < FtUInt::MAX as FtULong {
        max_gid = ttface.root.num_glyphs as FtUInt;
    }

    with_stream(ttface, |ttface, stream| {
        for i in 0..max_gid {
            let gname = match tt_face_get_ps_name(ttface, stream, i) {
                Ok(g) => g,
                Err(_) => continue,
            };

            if glyph_name == &gname[..] {
                return i;
            }
        }

        0
    })
}

/// `sfnt_service_glyph_dict`
pub static SFNT_SERVICE_GLYPH_DICT: FtServiceGlyphDictRec = FtServiceGlyphDictRec {
    get_name: sfnt_get_glyph_name, /* FT_GlyphDict_GetNameFunc   get_name   */
    name_index: sfnt_get_name_index, /* FT_GlyphDict_NameIndexFunc name_index */
};

/*
 *
 * POSTSCRIPT NAME SERVICE
 *
 */

/* an array representing allowed ASCII characters in a PS string */
static SFNT_PS_MAP: [u8; 16] = [
    /*             4        0        C        8 */
    0x00, 0x00, /* 0x00: 0 0 0 0  0 0 0 0  0 0 0 0  0 0 0 0 */
    0x00, 0x00, /* 0x10: 0 0 0 0  0 0 0 0  0 0 0 0  0 0 0 0 */
    0xDE, 0x7C, /* 0x20: 1 1 0 1  1 1 1 0  0 1 1 1  1 1 0 0 */
    0xFF, 0xAF, /* 0x30: 1 1 1 1  1 1 1 1  1 0 1 0  1 1 1 1 */
    0xFF, 0xFF, /* 0x40: 1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1 */
    0xFF, 0xD7, /* 0x50: 1 1 1 1  1 1 1 1  1 1 0 1  0 1 1 1 */
    0xFF, 0xFF, /* 0x60: 1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1 */
    0xFF, 0x57, /* 0x70: 1 1 1 1  1 1 1 1  0 1 0 1  0 1 1 1 */
];

/// `sfnt_is_postscript`
fn sfnt_is_postscript(c: FtInt) -> bool {
    if !(0..0x80).contains(&c) {
        return false;
    }

    let cc = c as FtUInt;

    SFNT_PS_MAP[(cc >> 3) as usize] & (1 << (cc & 0x07)) != 0
}

/// `char_type_func`
type CharTypeFunc = fn(FtInt) -> bool;

/* Handling of PID/EID 3/0 and 3/1 is the same. */
fn is_win(n: &TtNameRec) -> bool {
    n.platformID == 3 && (n.encodingID == 1 || n.encodingID == 0)
}

fn is_apple(n: &TtNameRec) -> bool {
    n.platformID == 1 && n.encodingID == 0
}

/// `get_win_string`
fn get_win_string(
    stream: &mut FtStreamRec,
    entry: &mut TtNameRec,
    char_type: CharTypeFunc,
    _report_invalid_characters: bool,
) -> Option<String> {
    let r = (|| -> FtResult<String> {
        let mut result = String::new();
        if result
            .try_reserve_exact(entry.stringLength as usize / 2 + 1)
            .is_err()
        {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }

        stream.seek(entry.stringOffset)?;
        stream.enter_frame(entry.stringLength as FtULong)?;

        {
            let p = stream.frame_data();
            for pair in p.chunks_exact(2).take(entry.stringLength as usize / 2) {
                /* (FT_Char is signed) */
                if pair[0] == 0 && char_type(pair[1] as i8 as FtInt) {
                    result.push(pair[1] as char);
                }
            }
        }

        stream.exit_frame();

        Ok(result)
    })();

    match r {
        Ok(s) if !s.is_empty() => Some(s),
        Err(FT_ERR_OUT_OF_MEMORY) => None,
        _ => {
            /* get_win_string_error: */
            entry.stringLength = 0;
            entry.stringOffset = 0;
            entry.string = None;

            None
        }
    }
}

/// `get_apple_string`
fn get_apple_string(
    stream: &mut FtStreamRec,
    entry: &mut TtNameRec,
    char_type: CharTypeFunc,
    _report_invalid_characters: bool,
) -> Option<String> {
    let r = (|| -> FtResult<String> {
        let mut result = String::new();
        if result
            .try_reserve_exact(entry.stringLength as usize + 1)
            .is_err()
        {
            return Err(FT_ERR_OUT_OF_MEMORY);
        }

        stream.seek(entry.stringOffset)?;
        stream.enter_frame(entry.stringLength as FtULong)?;

        {
            let p = stream.frame_data();
            for &c in p.iter().take(entry.stringLength as usize) {
                /* (FT_Char is signed) */
                if char_type(c as i8 as FtInt) {
                    result.push(c as char);
                }
            }
        }

        stream.exit_frame();

        Ok(result)
    })();

    match r {
        Ok(s) if !s.is_empty() => Some(s),
        Err(FT_ERR_OUT_OF_MEMORY) => None,
        _ => {
            /* get_apple_string_error: */
            entry.stringOffset = 0;
            entry.stringLength = 0;
            entry.string = None;

            None
        }
    }
}

/// `sfnt_get_name_id`
pub fn sfnt_get_name_id(
    face: &TtFaceRec,
    id: FtUShort,
    win: &mut FtInt,
    apple: &mut FtInt,
) -> bool {
    *win = -1;
    *apple = -1;

    for n in 0..face.num_names as usize {
        let name = &face.name_table.names[n];

        if name.nameID == id && name.stringLength > 0 {
            if is_win(name) && (name.languageID == 0x409 || *win == -1) {
                *win = n as FtInt;
            }

            if is_apple(name) && (name.languageID == 0 || *apple == -1) {
                *apple = n as FtInt;
            }
        }
    }

    (*win >= 0) || (*apple >= 0)
}

/// `sfnt_get_ps_name`
fn sfnt_get_ps_name(face: &mut FtFace) -> Option<String> {
    let FtFace::Tt(ttface) = face else {
        return None;
    };

    let mut win = -1;
    let mut apple = -1;
    let mut result = None;

    if ttface.postscript_name.is_some() {
        return ttface.postscript_name.clone();
    }

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT: `sfnt_get_var_ps_name' is not */
    /* translated yet (see the module documentation).                 */

    /* scan the name table to see whether we have a Postscript name here, */
    /* either in Macintosh or Windows platform encodings                  */
    let found = sfnt_get_name_id(ttface, TT_NAME_ID_PS_NAME, &mut win, &mut apple);
    if !found {
        return None;
    }

    with_stream(ttface, |ttface, stream| {
        /* prefer Windows entries over Apple */
        if win != -1 {
            result = get_win_string(
                stream,
                &mut ttface.name_table.names[win as usize],
                sfnt_is_postscript,
                true,
            );
        }

        if result.is_none() && apple != -1 {
            result = get_apple_string(
                stream,
                &mut ttface.name_table.names[apple as usize],
                sfnt_is_postscript,
                true,
            );
        }
    });

    ttface.postscript_name = result.clone();

    result
}

/// `sfnt_service_ps_name`
pub static SFNT_SERVICE_PS_NAME: FtServicePsFontNameRec = FtServicePsFontNameRec {
    get_ps_font_name: sfnt_get_ps_name, /* FT_PsName_GetFunc get_ps_font_name */
};

/*
 * TT CMAP INFO
 */

fn tt_service_get_cmap_info_func(face: &FtFace, charmap: usize) -> FtResult<TtCMapInfo> {
    tt_get_cmap_info(face, charmap)
}

/// `tt_service_get_cmap_info`
pub static TT_SERVICE_GET_CMAP_INFO: FtServiceTtCMapsRec = FtServiceTtCMapsRec {
    get_cmap_info: tt_service_get_cmap_info_func, /* TT_CMap_Info_GetFunc get_cmap_info */
};

/*
 * SERVICE LIST
 */

/// `sfnt_services` (the BDF service is left out; see the module
/// documentation)
static SFNT_SERVICES: [(&str, FtService); 4] = [
    (
        FT_SERVICE_ID_SFNT_TABLE,
        FtService::SfntTable(&SFNT_SERVICE_SFNT_TABLE),
    ),
    (
        FT_SERVICE_ID_POSTSCRIPT_FONT_NAME,
        FtService::PsFontName(&SFNT_SERVICE_PS_NAME),
    ),
    (
        FT_SERVICE_ID_GLYPH_DICT,
        FtService::GlyphDict(&SFNT_SERVICE_GLYPH_DICT),
    ),
    (
        FT_SERVICE_ID_TT_CMAP,
        FtService::TtCMaps(&TT_SERVICE_GET_CMAP_INFO),
    ),
];

/// `sfnt_get_interface`
pub fn sfnt_get_interface(_module: &FtModuleRec, module_interface: &str) -> Option<FtService> {
    SFNT_SERVICES
        .iter()
        .find(|(id, _)| *id == module_interface)
        .map(|(_, s)| *s)
}

/// `sfnt_module_class`
pub static SFNT_MODULE_CLASS: FtModuleClass = FtModuleClass {
    module_flags: 0,          /* not a font driver or renderer */
    module_name: "sfnt",      /* driver name                            */
    module_version: 0x10000,  /* driver version 1.0                     */
    module_requires: 0x20000, /* driver requires FreeType 2.0 or higher */

    module_interface: FtModuleInterface::None, /* (called directly) */

    module_init: None, /* FT_Module_Constructor module_init   */
    module_done: None, /* FT_Module_Destructor  module_done   */
    get_interface: Some(sfnt_get_interface), /* FT_Module_Requester   get_interface */
};
