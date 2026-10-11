// Rust translation of src/cid/cidriver.c and src/cid/cidriver.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! CID driver interface (body).
//!
//! Nothing in SDL_ttf asks for the PostScript info service
//! (`cid_ps_get_font_info`, `cid_ps_get_font_extra`) or the CID info
//! service (`cid_get_ros`, `cid_get_is_cid`,
//! `cid_get_cid_from_glyph_index`), which are not translated.

use super::super::base::ftobjs::*;
use super::super::base::ftpsprop::{ps_property_get, ps_property_set};
use super::super::fttypes::*;
use super::super::t1types::CidFaceRec;
use super::cidgload::cid_slot_load_glyph;
use super::cidobjs::*;

/*
 * POSTSCRIPT NAME SERVICE
 *
 */

/// `cid_get_postscript_name`
fn cid_get_postscript_name(face: &mut FtFace) -> Option<String> {
    let FtFace::Cid(cidface) = &*face else {
        return None;
    };
    let result = cidface.cid.cid_font_name.as_deref()?;

    let result = if result.first() == Some(&b'/') {
        &result[1..]
    } else {
        result
    };

    let len = result.iter().position(|&c| c == 0).unwrap_or(result.len());
    Some(String::from_utf8_lossy(&result[..len]).into_owned())
}

/// `cid_service_ps_name`
static CID_SERVICE_PS_NAME: FtServicePsFontNameRec = FtServicePsFontNameRec {
    get_ps_font_name: cid_get_postscript_name, /* get_ps_font_name */
};

/*
 * POSTSCRIPT INFO SERVICE, CID INFO SERVICE
 *
 * (not translated; see the module documentation)
 */

/*
 * PROPERTY SERVICE
 *
 */

/// `cid_service_properties`
static CID_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: ps_property_set, /* FT_Properties_SetFunc set_property */
    get_property: ps_property_get, /* FT_Properties_GetFunc get_property */
};

/*
 * SERVICE LIST
 *
 */

/// `FT_FONT_FORMAT_CID`
pub const FT_FONT_FORMAT_CID: &str = "CID Type 1";

/// `cid_services` (see the module documentation for the services left
/// out)
static CID_SERVICES: [(&str, FtService); 3] = [
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_CID),
    ),
    (
        FT_SERVICE_ID_POSTSCRIPT_FONT_NAME,
        FtService::PsFontName(&CID_SERVICE_PS_NAME),
    ),
    (
        FT_SERVICE_ID_PROPERTIES,
        FtService::Properties(&CID_SERVICE_PROPERTIES),
    ),
];

/// `cid_get_interface`
fn cid_get_interface(_module: &FtModuleRec, cid_interface: &str) -> Option<FtService> {
    CID_SERVICES
        .iter()
        .find(|(id, _)| *id == cid_interface)
        .map(|(_, s)| *s)
}

fn cid_new_face(root: FtFaceRec) -> FtFace {
    FtFace::Cid(Box::new(CidFaceRec {
        root,
        ..Default::default()
    }))
}

fn cid_face_init_drv(face: &mut FtFace, face_index: FtInt, params: &[FtParameter]) -> FtResult<()> {
    let FtFace::Cid(cidface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    cid_face_init(cidface, face_index, params)
}

fn cid_face_done_drv(face: &mut FtFace) {
    let FtFace::Cid(cidface) = face else {
        return;
    };
    cid_face_done(cidface)
}

fn cid_size_init_drv(face: &mut FtFace) -> FtResult<()> {
    let FtFace::Cid(cidface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    cid_size_init(cidface)
}

fn cid_size_done_drv(face: &mut FtFace) {
    let FtFace::Cid(cidface) = face else {
        return;
    };
    cid_size_done(cidface)
}

fn cid_size_request_drv(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::Cid(cidface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    cid_size_request(cidface, req)
}

/// `t1cid_driver_class`
pub static T1CID_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER
            | FT_MODULE_DRIVER_SCALABLE
            | FT_MODULE_DRIVER_HAS_HINTER,

        module_name: "t1cid",     /* module name           */
        module_version: 0x10000,  /* version 1.0 of driver */
        module_requires: 0x20000, /* requires FreeType 2.0 */

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: Some(cid_driver_init), /* FT_Module_Constructor  module_init   */
        module_done: Some(cid_driver_done), /* FT_Module_Destructor   module_done   */
        get_interface: Some(cid_get_interface), /* FT_Module_Requester    get_interface */
    },

    new_face: cid_new_face,

    init_face: Some(cid_face_init_drv), /* FT_Face_InitFunc  init_face */
    done_face: Some(cid_face_done_drv), /* FT_Face_DoneFunc  done_face */
    init_size: Some(cid_size_init_drv), /* FT_Size_InitFunc  init_size */
    done_size: Some(cid_size_done_drv), /* FT_Size_DoneFunc  done_size */
    init_slot: Some(cid_slot_init),     /* FT_Slot_InitFunc  init_slot */
    done_slot: Some(cid_slot_done),     /* FT_Slot_DoneFunc  done_slot */

    load_glyph: Some(cid_slot_load_glyph), /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: None,  /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,  /* FT_Face_AttachFunc       attach_file  */
    get_advances: None, /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(cid_size_request_drv), /* FT_Size_RequestFunc  request_size */
    select_size: None,                        /* FT_Size_SelectFunc   select_size  */
};
