// Rust translation of src/cff/cffdrivr.c (and cffdrivr.h) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! OpenType font driver implementation (body).
//!
//! Translation notes: the `sfnt` module is always part of the library, so
//! its services are called directly where C asks the module for them.
//! The PostScript info, CID, multiple masters, metrics variations, and
//! CFF load services (`cff_service_ps_info`, `cff_service_cid_info`,
//! `cff_service_multi_masters`, `cff_service_metrics_variations`,
//! `cff_service_cff_load`) are not translated: nothing in SDL_ttf asks
//! for them (the loaders call `cffload` and `ttgxvar` directly).

use super::super::base::ftobjs::*;
use super::super::base::ftpsprop::{ps_property_get, ps_property_set};
use super::super::fttypes::*;
use super::super::psnames::psmodule::ps_get_standard_strings;
use super::super::sfnt::sfdriver::{
    sfnt_get_interface, SFNT_SERVICE_GLYPH_DICT, SFNT_SERVICE_PS_NAME, TT_SERVICE_GET_CMAP_INFO,
};
use super::super::sfnt::ttkern::tt_face_get_kerning;
use super::super::sfnt::ttload::with_stream;
use super::super::sfnt::ttmtx::tt_face_get_metrics;
use super::super::tttypes::*;
use super::cffcmap::{CFF_CMAP_ENCODING_CLASS_REC, CFF_CMAP_UNICODE_CLASS_REC};
use super::cffgload::cff_slot_load;
use super::cffload::cff_index_get_sid_string;
use super::cffload::cff_index_get_string;
use super::cffobjs::*;

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****                                                                 ****/
/****                          F A C E S                              ****/
/****                                                                 ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/// `cff_get_kerning`: A driver method used to return the kerning vector
/// between two glyphs of the same face.
///
/// Returns the kerning vector.  This is in font units for scalable
/// formats, and in pixels for fixed-sizes formats.
///
/// Only horizontal layouts (left-to-right & right-to-left) are supported
/// by this function.  Other layouts, or more sophisticated kernings, are
/// out of scope of this method (the basic driver interface is meant to be
/// simple).
///
/// They can be implemented by format-specific interfaces.
fn cff_get_kerning(
    face: &mut FtFace, /* CFF_Face */
    left_glyph: FtUInt,
    right_glyph: FtUInt,
) -> FtResult<FtVector> {
    let FtFace::Tt(cffface) = face;

    let mut kerning = FtVector { x: 0, y: 0 };

    /* (the `sfnt' module is always there) */
    kerning.x = tt_face_get_kerning(cffface, left_glyph, right_glyph) as FtPos;

    Ok(kerning)
}

/// `cff_glyph_load`: A driver method used to load a glyph within a given
/// glyph slot.
///
/// `glyph_index`: The index of the glyph in the font file.
///
/// `load_flags`: A flag indicating what to load for this glyph.  The
/// FT_LOAD_??? constants can be used to control the glyph loading process
/// (e.g., whether the outline should be scaled, whether to load bitmaps or
/// not, whether to hint the outline, etc).
fn cff_glyph_load(face: &mut FtFace, glyph_index: FtUInt, load_flags: FtInt32) -> FtResult<()> {
    let FtFace::Tt(cffface) = face;

    /* (the face's slot and size exist together) */
    if !cffface.root.has_slot_and_size {
        return Err(FT_ERR_INVALID_SLOT_HANDLE);
    }

    /* check whether we want a scaled outline or bitmap */
    /* (the face's size always exists here) */

    /* reset the size object if necessary */
    /* FIXME (upstream): C sets `size' to NULL for FT_LOAD_NO_SCALE but  */
    /* passes `cffsize', which it does not reset, to `cff_slot_load'; so */
    /* the size is always passed (its `y_ppem' sets the outline's        */
    /* FT_OUTLINE_HIGH_PRECISION flag even for unscaled loads)           */

    /* (`size->face != slot->face' cannot happen: both are the face's) */

    /* now load the glyph outline if necessary */
    cff_slot_load(cffface, true, glyph_index, load_flags)
}

/// `cff_get_advances`
fn cff_get_advances(
    face: &mut FtFace,
    start: FtUInt,
    count: FtUInt,
    flags: FtInt32,
    advances: &mut [FtFixed],
) -> FtResult<()> {
    let mut flags = flags;
    let mut error = Ok(());

    'missing_table: {
        let FtFace::Tt(cffface) = face;

        if !ft_is_sfnt(&cffface.root) {
            break 'missing_table;
        }

        if flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
            /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
            /* no fast retrieval for blended MM fonts without VVAR table */
            if (ft_is_named_instance(&cffface.root) || ft_is_variation(&cffface.root))
                && cffface.variation_support & TT_FACE_FLAG_VAR_VADVANCE == 0
            {
                return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
            }

            /* check whether we have data from the `vmtx' table at all */
            if !cffface.vertical_info {
                break 'missing_table;
            }

            with_stream(cffface, |cffface, stream| {
                for nn in 0..count {
                    let (_dummy, ah) = tt_face_get_metrics(cffface, stream, true, start + nn);

                    advances[nn as usize] = ah as FtFixed;
                }
            });
        } else {
            /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
            /* no fast retrieval for blended MM fonts without HVAR table */
            if (ft_is_named_instance(&cffface.root) || ft_is_variation(&cffface.root))
                && cffface.variation_support & TT_FACE_FLAG_VAR_HADVANCE == 0
            {
                return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
            }

            /* check whether we have data from the `hmtx' table at all */
            if cffface.horizontal.number_Of_HMetrics == 0 {
                break 'missing_table;
            }

            with_stream(cffface, |cffface, stream| {
                for nn in 0..count {
                    let (_dummy, aw) = tt_face_get_metrics(cffface, stream, false, start + nn);

                    advances[nn as usize] = aw as FtFixed;
                }
            });
        }

        return error;
    }

    /* Missing_Table: */
    flags |= FT_LOAD_ADVANCE_ONLY;

    for nn in 0..count {
        error = cff_glyph_load(face, start + nn, flags);
        if error.is_err() {
            break;
        }

        let FtFace::Tt(cffface) = face;
        let slot = &cffface.root.glyph;
        advances[nn as usize] = if flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
            slot.linearVertAdvance
        } else {
            slot.linearHoriAdvance
        };
    }

    error
}

/*
 * GLYPH DICT SERVICE
 *
 */

/// `cff_get_glyph_name`
fn cff_get_glyph_name(
    face: &mut FtFace, /* CFF_Face */
    glyph_index: FtUInt,
    buffer: &mut [u8],
) -> FtResult<()> {
    let FtFace::Tt(cffface) = &*face;
    let Some(font) = cffface.cff.as_deref() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    if font.version_major == 2 {
        /* (the `sfnt' module's service) */
        return (SFNT_SERVICE_GLYPH_DICT.get_name)(face, glyph_index, buffer);
    }

    if !font.psnames {
        return Err(FT_ERR_MISSING_MODULE);
    }

    /* first, locate the sid in the charset table */
    let sid = font
        .charset
        .sids
        .as_ref()
        .and_then(|sids| sids.get(glyph_index as usize))
        .copied()
        .unwrap_or(0);

    /* now, look up the name itself */
    let gname = cff_index_get_sid_string(font, sid as FtUInt);

    if let Some(gname) = gname {
        ft_strcpyn(buffer, gname);
    }

    Ok(())
}

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

/// `cff_get_name_index`
fn cff_get_name_index(face: &mut FtFace /* CFF_Face */, glyph_name: &[u8]) -> FtUInt {
    let FtFace::Tt(cffface) = &*face;
    let Some(cff) = cffface.cff.as_deref() else {
        return 0;
    };
    let charset = &cff.charset;

    if cff.version_major == 2 {
        /* (the `sfnt' module's service) */
        return (SFNT_SERVICE_GLYPH_DICT.name_index)(face, glyph_name);
    }

    /* (FT_FACE_FIND_GLOBAL_SERVICE: the `psnames' module is always part
    of the library) */

    let Some(sids) = charset.sids.as_ref() else {
        return 0;
    };

    for i in 0..cff.num_glyphs {
        let sid = sids.get(i as usize).copied().unwrap_or(0) as FtUInt;

        let name = if sid > 390 {
            cff_index_get_string(cff, sid - 391)
        } else {
            ps_get_standard_strings(sid)
        };

        let Some(name) = name else {
            continue;
        };

        if glyph_name == name {
            return i;
        }
    }

    0
}

/// `cff_service_glyph_dict`
static CFF_SERVICE_GLYPH_DICT: FtServiceGlyphDictRec = FtServiceGlyphDictRec {
    get_name: cff_get_glyph_name, /* FT_GlyphDict_GetNameFunc   get_name   */
    name_index: cff_get_name_index, /* FT_GlyphDict_NameIndexFunc name_index */
};

/*
 * POSTSCRIPT INFO SERVICE
 *
 * (`cff_ps_has_glyph_names', `cff_ps_get_font_info',
 * `cff_ps_get_font_extra': not translated; see the module documentation)
 */

/*
 * POSTSCRIPT NAME SERVICE
 *
 */

/// `cff_get_ps_name`
fn cff_get_ps_name(face: &mut FtFace) -> Option<String> {
    let FtFace::Tt(cffface) = &*face;

    /* following the OpenType specification 1.7, we return the name stored */
    /* in the `name' table for a CFF wrapped into an SFNT container        */

    if ft_is_sfnt(&cffface.root) {
        /* (the `sfnt' module's service) */
        return (SFNT_SERVICE_PS_NAME.get_ps_font_name)(face);
    }

    let FtFace::Tt(cffface) = &*face;
    cffface
        .cff
        .as_ref()
        .and_then(|cff| cff.font_name.as_ref())
        .map(|name| String::from_utf8_lossy(name).into_owned())
}

/// `cff_service_ps_name`
static CFF_SERVICE_PS_NAME: FtServicePsFontNameRec = FtServicePsFontNameRec {
    get_ps_font_name: cff_get_ps_name, /* FT_PsName_GetFunc get_ps_font_name */
};

/*
 * TT CMAP INFO
 *
 * If the charmap is a synthetic Unicode encoding cmap or
 * a Type 1 standard (or expert) encoding cmap, hide TT CMAP INFO
 * service defined in SFNT module.
 *
 * Otherwise call the service function in the sfnt module.
 *
 */

/// `cff_get_cmap_info`
fn cff_get_cmap_info(face: &FtFace, charmap: usize) -> FtResult<TtCMapInfo> {
    let FtFace::Tt(cffface) = face;

    let clazz = cffface.root.charmaps.get(charmap).map(|cmap| cmap.clazz);

    let is_cff_class = clazz.is_some_and(|clazz| {
        core::ptr::eq(clazz, &CFF_CMAP_ENCODING_CLASS_REC)
            || core::ptr::eq(clazz, &CFF_CMAP_UNICODE_CLASS_REC)
    });

    if !is_cff_class {
        /* (the `sfnt' module's service) */
        (TT_SERVICE_GET_CMAP_INFO.get_cmap_info)(face, charmap)
    } else {
        Err(FT_ERR_INVALID_CHARMAP_FORMAT)
    }
}

/// `cff_service_get_cmap_info`
static CFF_SERVICE_GET_CMAP_INFO: FtServiceTtCMapsRec = FtServiceTtCMapsRec {
    get_cmap_info: cff_get_cmap_info, /* TT_CMap_Info_GetFunc get_cmap_info */
};

/*
 * CID INFO SERVICE
 *
 * (`cff_get_ros', `cff_get_is_cid', `cff_get_cid_from_glyph_index': not
 * translated; see the module documentation)
 */

/*
 * PROPERTY SERVICE
 *
 */

/// `cff_service_properties`
static CFF_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: ps_property_set, /* FT_Properties_SetFunc set_property */
    get_property: ps_property_get, /* FT_Properties_GetFunc get_property */
};

/*
 * MULTIPLE MASTER SERVICE, METRICS VARIATIONS SERVICE, CFFLOAD SERVICE
 *
 * (not translated; see the module documentation)
 */

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****                                                                 ****/
/****                D R I V E R  I N T E R F A C E                   ****/
/****                                                                 ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/// `FT_FONT_FORMAT_CFF`
pub const FT_FONT_FORMAT_CFF: &str = "CFF";

/// `cff_services` (see the module documentation for the services left
/// out)
static CFF_SERVICES: [(&str, FtService); 5] = [
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_CFF),
    ),
    (
        FT_SERVICE_ID_POSTSCRIPT_FONT_NAME,
        FtService::PsFontName(&CFF_SERVICE_PS_NAME),
    ),
    (
        FT_SERVICE_ID_GLYPH_DICT,
        FtService::GlyphDict(&CFF_SERVICE_GLYPH_DICT),
    ),
    (
        FT_SERVICE_ID_TT_CMAP,
        FtService::TtCMaps(&CFF_SERVICE_GET_CMAP_INFO),
    ),
    (
        FT_SERVICE_ID_PROPERTIES,
        FtService::Properties(&CFF_SERVICE_PROPERTIES),
    ),
];

/// `cff_get_interface`
fn cff_get_interface(
    driver: &FtModuleRec, /* CFF_Driver */
    module_interface: &str,
) -> Option<FtService> {
    let result = CFF_SERVICES
        .iter()
        .find(|(id, _)| *id == module_interface)
        .map(|(_, s)| *s);
    if result.is_some() {
        return result;
    }

    /* we pass our request to the `sfnt' module */
    sfnt_get_interface(driver, module_interface)
}

/* The FT_DriverInterface structure is defined in ftdriver.h. */

fn cff_new_face(root: FtFaceRec) -> FtFace {
    FtFace::Tt(Box::new(TtFaceRec {
        root,
        ..Default::default()
    }))
}

fn cff_face_init_drv(
    face: &mut FtFace,
    typeface_index: FtInt,
    params: &[FtParameter],
) -> FtResult<()> {
    let FtFace::Tt(cffface) = face;
    cff_face_init(cffface, typeface_index, params)
}

fn cff_face_done_drv(face: &mut FtFace) {
    let FtFace::Tt(cffface) = face;
    cff_face_done(cffface)
}

fn cff_size_init_drv(face: &mut FtFace) -> FtResult<()> {
    let FtFace::Tt(cffface) = face;
    cff_size_init(cffface)
}

fn cff_size_done_drv(face: &mut FtFace) {
    let FtFace::Tt(cffface) = face;
    cff_size_done(cffface)
}

fn cff_size_request_drv(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::Tt(cffface) = face;
    cff_size_request(cffface, req)
}

fn cff_size_select_drv(face: &mut FtFace, strike_index: FtULong) -> FtResult<()> {
    let FtFace::Tt(cffface) = face;
    cff_size_select(cffface, strike_index)
}

/// `cff_driver_class`
pub static CFF_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER
            | FT_MODULE_DRIVER_SCALABLE
            | FT_MODULE_DRIVER_HAS_HINTER
            | FT_MODULE_DRIVER_HINTS_LIGHTLY,

        module_name: "cff",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: Some(cff_driver_init), /* FT_Module_Constructor  module_init   */
        module_done: Some(cff_driver_done), /* FT_Module_Destructor   module_done   */
        get_interface: Some(cff_get_interface), /* FT_Module_Requester    get_interface */
    },

    new_face: cff_new_face,

    init_face: Some(cff_face_init_drv), /* FT_Face_InitFunc  init_face */
    done_face: Some(cff_face_done_drv), /* FT_Face_DoneFunc  done_face */
    init_size: Some(cff_size_init_drv), /* FT_Size_InitFunc  init_size */
    done_size: Some(cff_size_done_drv), /* FT_Size_DoneFunc  done_size */
    init_slot: Some(cff_slot_init),     /* FT_Slot_InitFunc  init_slot */
    done_slot: Some(cff_slot_done),     /* FT_Slot_DoneFunc  done_slot */
    load_glyph: Some(cff_glyph_load),   /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: Some(cff_get_kerning), /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,                  /* FT_Face_AttachFunc       attach_file  */
    get_advances: Some(cff_get_advances), /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(cff_size_request_drv), /* FT_Size_RequestFunc  request_size */
    select_size: Some(cff_size_select_drv),   /* FT_Size_SelectFunc   select_size  */
};
