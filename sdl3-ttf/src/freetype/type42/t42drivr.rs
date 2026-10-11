// Rust translation of src/type42/t42drivr.c and src/type42/t42drivr.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2002-2023 by Roberto Alameda.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! High-level Type 42 driver interface (body).
//!
//! This driver implements Type42 fonts as described in the
//! Technical Note #5012 from Adobe, with these limitations:
//!
//! 1) CID Fonts are not currently supported.
//! 2) Incremental fonts making use of the GlyphDirectory keyword
//!    will be loaded, but the rendering will be using the TrueType
//!    tables.
//! 3) As for Type1 fonts, CDevProc is not supported.
//! 4) The Metrics dictionary is not supported.
//! 5) AFM metrics are not supported.
//!
//! In other words, this driver supports Type42 fonts derived from
//! TrueType fonts in a non-CID manner, as done by usual conversion
//! programs.
//!
//! Nothing in SDL_ttf asks for the PostScript info service
//! (`t42_ps_get_font_info`, `t42_ps_get_font_extra`,
//! `t42_ps_has_glyph_names`), which is not translated.

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::t42objs::*;

/*
 *
 * GLYPH DICT SERVICE
 *
 */

/// `FT_STRCPYN`: copies `src` into `dst` as a NUL-terminated string,
/// truncating it to fit
fn ft_strcpyn(dst: &mut [u8], src: &[u8]) {
    if dst.is_empty() {
        return;
    }
    let src = src.split(|&c| c == 0).next().unwrap_or(&[]);
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
    dst[n] = 0;
}

/// `t42_get_glyph_name`
fn t42_get_glyph_name(face: &mut FtFace, glyph_index: FtUInt, buffer: &mut [u8]) -> FtResult<()> {
    let FtFace::T42(t42face) = &*face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    ft_strcpyn(
        buffer,
        t42face
            .type1
            .glyph_names
            .name(glyph_index as usize)
            .unwrap_or(b""),
    );

    Ok(())
}

/// `t42_get_name_index` (the glyph's index in the TrueType font)
fn t42_get_name_index(face: &mut FtFace, glyph_name: &[u8]) -> FtUInt {
    let FtFace::T42(t42face) = &*face else {
        return 0;
    };

    for i in 0..t42face.type1.num_glyphs.max(0) as usize {
        let gname = t42face.type1.glyph_names.name(i).unwrap_or(b"");

        if glyph_name.first().copied().unwrap_or(0) == gname.first().copied().unwrap_or(0)
            && glyph_name == gname
        {
            return t42_ttf_glyph_index(&t42face.type1, i);
        }
    }

    0
}

/// `t42_service_glyph_dict`
static T42_SERVICE_GLYPH_DICT: FtServiceGlyphDictRec = FtServiceGlyphDictRec {
    get_name: t42_get_glyph_name,   /* get_name   */
    name_index: t42_get_name_index, /* name_index */
};

/*
 *
 * POSTSCRIPT NAME SERVICE
 *
 */

/// `t42_get_ps_font_name`
fn t42_get_ps_font_name(face: &mut FtFace) -> Option<String> {
    let FtFace::T42(t42face) = &*face else {
        return None;
    };

    t42face.type1.font_name.as_ref().map(|name| {
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        String::from_utf8_lossy(&name[..len]).into_owned()
    })
}

/// `t42_service_ps_font_name`
static T42_SERVICE_PS_FONT_NAME: FtServicePsFontNameRec = FtServicePsFontNameRec {
    get_ps_font_name: t42_get_ps_font_name, /* get_ps_font_name */
};

/*
 *
 * POSTSCRIPT INFO SERVICE
 *
 * (not translated; see the module documentation)
 */

/*
 *
 * SERVICE LIST
 *
 */

/// `FT_FONT_FORMAT_TYPE_42`
pub const FT_FONT_FORMAT_TYPE_42: &str = "Type 42";

/// `t42_services` (see the module documentation for the service left
/// out)
static T42_SERVICES: [(&str, FtService); 3] = [
    (
        FT_SERVICE_ID_GLYPH_DICT,
        FtService::GlyphDict(&T42_SERVICE_GLYPH_DICT),
    ),
    (
        FT_SERVICE_ID_POSTSCRIPT_FONT_NAME,
        FtService::PsFontName(&T42_SERVICE_PS_FONT_NAME),
    ),
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_TYPE_42),
    ),
];

/// `T42_Get_Interface`
fn t42_get_interface(_module: &FtModuleRec, t42_interface: &str) -> Option<FtService> {
    T42_SERVICES
        .iter()
        .find(|(id, _)| *id == t42_interface)
        .map(|(_, s)| *s)
}

fn t42_new_face(root: FtFaceRec) -> FtFace {
    FtFace::T42(Box::new(T42FaceRec {
        root,
        ..Default::default()
    }))
}

fn t42_face_init_drv(face: &mut FtFace, face_index: FtInt, params: &[FtParameter]) -> FtResult<()> {
    let FtFace::T42(t42face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    t42_face_init(t42face, face_index, params)
}

fn t42_face_done_drv(face: &mut FtFace) {
    let FtFace::T42(t42face) = face else {
        return;
    };
    t42_face_done(t42face)
}

fn t42_size_init_drv(face: &mut FtFace) -> FtResult<()> {
    let FtFace::T42(t42face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    t42_size_init(t42face)
}

fn t42_size_done_drv(face: &mut FtFace) {
    let FtFace::T42(t42face) = face else {
        return;
    };
    t42_size_done(t42face)
}

fn t42_size_request_drv(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::T42(t42face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    t42_size_request(t42face, req)
}

fn t42_size_select_drv(face: &mut FtFace, strike_index: FtULong) -> FtResult<()> {
    let FtFace::T42(t42face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    t42_size_select(t42face, strike_index)
}

/// `t42_driver_class`
pub static T42_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        /* (TT_USE_BYTECODE_INTERPRETER is defined) */
        module_flags: FT_MODULE_FONT_DRIVER
            | FT_MODULE_DRIVER_SCALABLE
            | FT_MODULE_DRIVER_HAS_HINTER,

        module_name: "type42",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: Some(t42_driver_init), /* FT_Module_Constructor  module_init   */
        module_done: Some(t42_driver_done), /* FT_Module_Destructor   module_done   */
        get_interface: Some(t42_get_interface), /* FT_Module_Requester    get_interface */
    },

    new_face: t42_new_face,

    init_face: Some(t42_face_init_drv), /* FT_Face_InitFunc  init_face */
    done_face: Some(t42_face_done_drv), /* FT_Face_DoneFunc  done_face */
    init_size: Some(t42_size_init_drv), /* FT_Size_InitFunc  init_size */
    done_size: Some(t42_size_done_drv), /* FT_Size_DoneFunc  done_size */
    init_slot: Some(t42_glyph_slot_init), /* FT_Slot_InitFunc  init_slot */
    done_slot: Some(t42_glyph_slot_done), /* FT_Slot_DoneFunc  done_slot */

    load_glyph: Some(t42_glyph_slot_load), /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: None,  /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,  /* FT_Face_AttachFunc       attach_file  */
    get_advances: None, /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(t42_size_request_drv), /* FT_Size_RequestFunc  request_size */
    select_size: Some(t42_size_select_drv),   /* FT_Size_SelectFunc   select_size  */
};
