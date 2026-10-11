// Rust translation of src/type1/t1driver.c and src/type1/t1driver.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 1 driver interface (body).
//!
//! The multiple masters service's functions (`t1load`) are called
//! directly by `FT_Get_MM_Var` and `FT_Get_Var_Blend_Coordinates`, the
//! only ones hb-ft uses; nothing in SDL_ttf asks for the PostScript info
//! service (`t1_ps_get_font_info`, `t1_ps_get_font_extra`,
//! `t1_ps_has_glyph_names`, `t1_ps_get_font_private`,
//! `t1_ps_get_font_value`) or the track kerning service
//! (`T1_Get_Track_Kerning`, translated in `t1afm`), which are not
//! registered.

use super::super::base::ftobjs::*;
use super::super::base::ftpsprop::{ps_property_get, ps_property_set};
use super::super::fttypes::*;
use super::super::t1types::T1FaceRec;
use super::t1afm::*;
use super::t1gload::*;
use super::t1objs::*;

/*
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

/// `t1_get_glyph_name`
fn t1_get_glyph_name(face: &mut FtFace, glyph_index: FtUInt, buffer: &mut [u8]) -> FtResult<()> {
    let FtFace::T1(t1face) = &*face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    ft_strcpyn(
        buffer,
        t1face
            .type1
            .glyph_names
            .name(glyph_index as usize)
            .unwrap_or(b""),
    );

    Ok(())
}

/// `t1_get_name_index`
fn t1_get_name_index(face: &mut FtFace, glyph_name: &[u8]) -> FtUInt {
    let FtFace::T1(t1face) = &*face else {
        return 0;
    };

    for i in 0..t1face.type1.num_glyphs.max(0) as usize {
        let gname = t1face.type1.glyph_names.name(i).unwrap_or(b"");

        if glyph_name == gname {
            return i as FtUInt;
        }
    }

    0
}

/// `t1_service_glyph_dict`
static T1_SERVICE_GLYPH_DICT: FtServiceGlyphDictRec = FtServiceGlyphDictRec {
    get_name: t1_get_glyph_name,   /* FT_GlyphDict_GetNameFunc   get_name   */
    name_index: t1_get_name_index, /* FT_GlyphDict_NameIndexFunc name_index */
};

/*
 * POSTSCRIPT NAME SERVICE
 *
 */

/// `t1_get_ps_name`
fn t1_get_ps_name(face: &mut FtFace) -> Option<String> {
    let FtFace::T1(t1face) = &*face else {
        return None;
    };

    t1face.type1.font_name.as_ref().map(|name| {
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        String::from_utf8_lossy(&name[..len]).into_owned()
    })
}

/// `t1_service_ps_name`
static T1_SERVICE_PS_NAME: FtServicePsFontNameRec = FtServicePsFontNameRec {
    get_ps_font_name: t1_get_ps_name, /* get_ps_font_name */
};

/*
 * MULTIPLE MASTERS SERVICE
 *
 * (the functions of `t1load' are called directly; see the module
 * documentation)
 */

/*
 * POSTSCRIPT INFO SERVICE
 *
 * (not translated; see the module documentation)
 */

/*
 * PROPERTY SERVICE
 *
 */

/// `t1_service_properties`
static T1_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: ps_property_set, /* FT_Properties_SetFunc set_property */
    get_property: ps_property_get, /* FT_Properties_GetFunc get_property */
};

/*
 * SERVICE LIST
 *
 */

/// `FT_FONT_FORMAT_TYPE_1`
pub const FT_FONT_FORMAT_TYPE_1: &str = "Type 1";

/// `t1_services` (see the module documentation for the services left
/// out)
static T1_SERVICES: [(&str, FtService); 4] = [
    (
        FT_SERVICE_ID_POSTSCRIPT_FONT_NAME,
        FtService::PsFontName(&T1_SERVICE_PS_NAME),
    ),
    (
        FT_SERVICE_ID_GLYPH_DICT,
        FtService::GlyphDict(&T1_SERVICE_GLYPH_DICT),
    ),
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_TYPE_1),
    ),
    (
        FT_SERVICE_ID_PROPERTIES,
        FtService::Properties(&T1_SERVICE_PROPERTIES),
    ),
];

/// `Get_Interface`
fn get_interface(_module: &FtModuleRec, t1_interface: &str) -> Option<FtService> {
    T1_SERVICES
        .iter()
        .find(|(id, _)| *id == t1_interface)
        .map(|(_, s)| *s)
}

/* (T1_CONFIG_OPTION_NO_AFM is undefined) */

/// `Get_Kerning`: a driver method used to return the kerning vector
/// between two glyphs of the same face.
///
/// Returns the kerning vector.  This is in font units for scalable
/// formats, and in pixels for fixed-sizes formats.
///
/// Only horizontal layouts (left-to-right & right-to-left) are supported
/// by this function.  Other layouts, or more sophisticated kernings are
/// out of scope of this method (the basic driver interface is meant to be
/// simple).
///
/// They can be implemented by format-specific interfaces.
fn get_kerning(t1face: &mut FtFace, left_glyph: FtUInt, right_glyph: FtUInt) -> FtResult<FtVector> {
    let FtFace::T1(face) = t1face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    let mut kerning = FtVector { x: 0, y: 0 };

    if let Some(afm_data) = face.afm_data.as_deref() {
        t1_get_kerning(afm_data, left_glyph, right_glyph, &mut kerning);
    }

    Ok(kerning)
}

fn t1_new_face(root: FtFaceRec) -> FtFace {
    FtFace::T1(Box::new(T1FaceRec {
        root,
        ..Default::default()
    }))
}

fn t1_face_init_drv(face: &mut FtFace, face_index: FtInt, params: &[FtParameter]) -> FtResult<()> {
    let FtFace::T1(t1face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    t1_face_init(t1face, face_index, params)
}

fn t1_face_done_drv(face: &mut FtFace) {
    let FtFace::T1(t1face) = face else {
        return;
    };
    t1_face_done(t1face)
}

fn t1_size_init_drv(face: &mut FtFace) -> FtResult<()> {
    let FtFace::T1(t1face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    t1_size_init(t1face)
}

fn t1_size_done_drv(face: &mut FtFace) {
    let FtFace::T1(t1face) = face else {
        return;
    };
    t1_size_done(t1face)
}

fn t1_size_request_drv(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::T1(t1face) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    t1_size_request(t1face, req)
}

/// `t1_driver_class`
pub static T1_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER
            | FT_MODULE_DRIVER_SCALABLE
            | FT_MODULE_DRIVER_HAS_HINTER,

        module_name: "type1",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: Some(t1_driver_init), /* FT_Module_Constructor  module_init   */
        module_done: Some(t1_driver_done), /* FT_Module_Destructor   module_done   */
        get_interface: Some(get_interface), /* FT_Module_Requester    get_interface */
    },

    new_face: t1_new_face,

    init_face: Some(t1_face_init_drv), /* FT_Face_InitFunc  init_face */
    done_face: Some(t1_face_done_drv), /* FT_Face_DoneFunc  done_face */
    init_size: Some(t1_size_init_drv), /* FT_Size_InitFunc  init_size */
    done_size: Some(t1_size_done_drv), /* FT_Size_DoneFunc  done_size */
    init_slot: Some(t1_glyph_slot_init), /* FT_Slot_InitFunc  init_slot */
    done_slot: Some(t1_glyph_slot_done), /* FT_Slot_DoneFunc  done_slot */

    load_glyph: Some(t1_load_glyph), /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: Some(get_kerning), /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: Some(t1_read_metrics), /* FT_Face_AttachFunc       attach_file  */
    get_advances: Some(t1_get_advances), /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(t1_size_request_drv), /* FT_Size_RequestFunc  request_size */
    select_size: None,                       /* FT_Size_SelectFunc   select_size  */
};
