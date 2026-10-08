// Rust translation of src/truetype/ttdriver.c (and ttdriver.h) from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! TrueType font driver implementation (body).
//!
//! Translation notes: the multiple masters, metrics variations, and
//! `glyf' services (`tt_service_gx_multi_masters`,
//! `tt_service_metrics_variations`, `tt_service_truetype_glyf`) are not in
//! the service list, as their users call the functions of `ttgxvar` and
//! `ttpload` directly; and `tt_get_interface` asks the `sfnt' module's
//! `sfnt_get_interface` directly for the default interfaces (the module
//! is always part of the library).

use super::super::base::ftcalc::ft_mul_div;
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::super::sfnt::sfdriver::sfnt_get_interface;
use super::super::sfnt::ttkern::tt_face_get_kerning;
use super::super::sfnt::ttload::with_stream;
use super::super::sfnt::ttsbit::{tt_face_load_strike_metrics, tt_face_set_sbit_strike};
use super::super::tttypes::*;
use super::ttgload::{tt_get_hmetrics, tt_get_vmetrics, tt_load_glyph};
use super::ttobjs::*;

/*
 * PROPERTY SERVICE
 *
 */

/// `tt_property_set`
fn tt_property_set(
    module: &FtModuleRec, /* TT_Driver */
    property_name: &str,
    value: &FtPropertyValue,
    value_is_string: bool,
) -> FtResult<()> {
    if property_name == "interpreter-version" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        let interpreter_version: FtUInt = if value_is_string {
            match value {
                FtPropertyValue::Str(s) => ft_strtol(s) as FtUInt,
                _ => 0,
            }
        } else {
            match *value {
                FtPropertyValue::UInt(iv) => iv,
                FtPropertyValue::Int(iv) => iv as FtUInt,
                _ => 0,
            }
        };

        let new_version = match interpreter_version {
            TT_INTERPRETER_VERSION_35 => TT_INTERPRETER_VERSION_35,

            /* TT_SUPPORT_SUBPIXEL_HINTING_MINIMAL */
            TT_INTERPRETER_VERSION_38 | TT_INTERPRETER_VERSION_40 => TT_INTERPRETER_VERSION_40,

            _ => return Err(FT_ERR_UNIMPLEMENTED_FEATURE),
        };

        module.with_props(|driver: &mut TtDriverRec| driver.interpreter_version = new_version);

        return Ok(());
    }

    Err(FT_ERR_MISSING_PROPERTY)
}

/// `ft_strtol( s, NULL, 10 )`
fn ft_strtol(s: &str) -> FtLong {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };

    let mut v: FtLong = 0;
    for b in digits.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        v = v.saturating_mul(10).saturating_add((b - b'0') as FtLong);
    }

    if neg {
        -v
    } else {
        v
    }
}

/// `tt_property_get`
fn tt_property_get(
    module: &FtModuleRec, /* TT_Driver */
    property_name: &str,
) -> FtResult<FtPropertyValue> {
    let interpreter_version = module
        .with_props(|driver: &mut TtDriverRec| driver.interpreter_version)
        .unwrap_or(TT_INTERPRETER_VERSION_40);

    if property_name == "interpreter-version" {
        return Ok(FtPropertyValue::UInt(interpreter_version));
    }

    Err(FT_ERR_MISSING_PROPERTY)
}

/// `tt_service_properties`
static TT_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: tt_property_set, /* FT_Properties_SetFunc set_property */
    get_property: tt_property_get, /* FT_Properties_GetFunc get_property */
};

/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****                                                                 ****/
/****                          F A C E S                              ****/
/****                                                                 ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/

/// `tt_get_kerning`: A driver method used to return the kerning vector
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
fn tt_get_kerning(
    face: &mut FtFace, /* TT_Face */
    left_glyph: FtUInt,
    right_glyph: FtUInt,
) -> FtResult<FtVector> {
    let FtFace::Tt(ttface) = face;

    let kerning = FtVector {
        x: tt_face_get_kerning(ttface, left_glyph, right_glyph) as FtPos,
        y: 0,
    };

    Ok(kerning)
}

/// `tt_get_advances`
fn tt_get_advances(
    face: &mut FtFace, /* TT_Face */
    start: FtUInt,
    count: FtUInt,
    flags: FtInt32,
    advances: &mut [FtFixed],
) -> FtResult<()> {
    let FtFace::Tt(ttface) = face;

    /* XXX: TODO: check for sbits */

    if flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        /* no fast retrieval for blended MM fonts without VVAR table */
        if (ft_is_named_instance(&ttface.root) || ft_is_variation(&ttface.root))
            && ttface.variation_support & TT_FACE_FLAG_VAR_VADVANCE == 0
        {
            return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
        }

        with_stream(ttface, |ttface, stream| {
            for nn in 0..count {
                /* since we don't need `tsb', we use zero for `yMax' parameter */
                let (_tsb, ah) = tt_get_vmetrics(ttface, stream, start + nn, 0);
                advances[nn as usize] = ah as FtFixed;
            }
        });
    } else {
        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        /* no fast retrieval for blended MM fonts without HVAR table */
        if (ft_is_named_instance(&ttface.root) || ft_is_variation(&ttface.root))
            && ttface.variation_support & TT_FACE_FLAG_VAR_HADVANCE == 0
        {
            return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
        }

        with_stream(ttface, |ttface, stream| {
            for nn in 0..count {
                let (_lsb, aw) = tt_get_hmetrics(ttface, stream, start + nn);
                advances[nn as usize] = aw as FtFixed;
            }
        });
    }

    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****                                                                 ****/
/****                           S I Z E S                             ****/
/****                                                                 ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/

/* TT_CONFIG_OPTION_EMBEDDED_BITMAPS */

/// `tt_size_select`
fn tt_size_select_tt(ttface: &mut TtFaceRec, strike_index: FtULong) -> FtResult<()> {
    let mut error = Ok(());

    ttface.size.strike_index = strike_index;

    if ft_is_scalable(&ttface.root) {
        /* use the scaled metrics, even when tt_size_reset fails */
        ft_select_metrics(&mut ttface.root, strike_index);

        let _ = tt_size_reset(ttface); /* ignore return value */
    } else {
        let mut size_metrics = ttface.root.size.metrics;

        error = with_stream(ttface, |ttface, stream| {
            tt_face_load_strike_metrics(ttface, stream, strike_index, &mut size_metrics)
        });
        ttface.root.size.metrics = size_metrics;

        if error.is_err() {
            ttface.size.strike_index = 0xFFFFFFFF;
        }
    }

    error
}

/// `tt_size_select` (the driver method)
fn tt_size_select(face: &mut FtFace, strike_index: FtULong) -> FtResult<()> {
    let FtFace::Tt(ttface) = face;
    tt_size_select_tt(ttface, strike_index)
}

/// `tt_size_request`
fn tt_size_request(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    let FtFace::Tt(ttface) = face;

    /* TT_CONFIG_OPTION_EMBEDDED_BITMAPS */
    if ft_has_fixed_sizes(&ttface.root) {
        match tt_face_set_sbit_strike(ttface, req) {
            Err(_) => ttface.size.strike_index = 0xFFFFFFFF,
            Ok(strike_index) => return tt_size_select_tt(ttface, strike_index),
        }
    }

    ft_request_metrics(&mut ttface.root, req)?;

    let mut error = Ok(());

    if ft_is_scalable(&ttface.root) {
        error = tt_size_reset(ttface);

        /* TT_USE_BYTECODE_INTERPRETER */
        /* for the `MPS' bytecode instruction we need the point size */
        if error.is_ok() {
            let metrics = ttface.size_metrics();
            let mut resolution = if metrics.x_ppem > metrics.y_ppem {
                req.horiResolution
            } else {
                req.vertResolution
            };

            /* if we don't have a resolution value, assume 72dpi */
            if req.type_ == FT_SIZE_REQUEST_TYPE_SCALES || resolution == 0 {
                resolution = 72;
            }

            ttface.size.point_size = ft_mul_div(
                ttface.size.ttmetrics.ppem as FtLong,
                64 * 72,
                resolution as FtLong,
            );
        }
    }

    error
}

/// `tt_glyph_load`: A driver method used to load a glyph within a given
/// glyph slot.
///
/// `glyph_index`: The index of the glyph in the font file.
///
/// `load_flags`: A flag indicating what to load for this glyph.  The
/// FT_LOAD_XXX constants can be used to control the glyph loading process
/// (e.g., whether the outline should be scaled, whether to load bitmaps or
/// not, whether to hint the outline, etc).
fn tt_glyph_load(face: &mut FtFace, glyph_index: FtUInt, load_flags: FtInt32) -> FtResult<()> {
    let FtFace::Tt(ttface) = face;
    let mut load_flags = load_flags;

    if !ttface.root.has_slot_and_size {
        return Err(FT_ERR_INVALID_SIZE_HANDLE);
    }

    if glyph_index >= ttface.root.num_glyphs as FtUInt {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if load_flags & FT_LOAD_NO_HINTING != 0 {
        /* both FT_LOAD_NO_HINTING and FT_LOAD_NO_AUTOHINT   */
        /* are necessary to disable hinting for tricky fonts */

        if ft_is_tricky(&ttface.root) {
            load_flags &= !FT_LOAD_NO_HINTING;
        }

        if load_flags & FT_LOAD_NO_AUTOHINT != 0 {
            load_flags |= FT_LOAD_NO_HINTING;
        }
    }

    if load_flags & (FT_LOAD_NO_RECURSE | FT_LOAD_NO_SCALE) != 0 {
        load_flags |= FT_LOAD_NO_BITMAP | FT_LOAD_NO_SCALE;

        if !ft_is_tricky(&ttface.root) {
            load_flags |= FT_LOAD_NO_HINTING;
        }
    }

    /* use hinted metrics only if we load a glyph with hinting */
    ttface.size.metrics_root = load_flags & FT_LOAD_NO_HINTING != 0;

    /* now fill in the glyph slot with outline/bitmap/layered */

    /* force drop-out mode to 2 - irrelevant now */
    /* slot->outline.dropout_mode = 2; */

    tt_load_glyph(ttface, glyph_index, load_flags)
}

/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****                                                                 ****/
/****                D R I V E R  I N T E R F A C E                   ****/
/****                                                                 ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/

/// `tt_service_truetype_engine` (TT_USE_BYTECODE_INTERPRETER)
const TT_SERVICE_TRUETYPE_ENGINE: FtTrueTypeEngineType = FT_TRUETYPE_ENGINE_TYPE_PATENTED;

/// `tt_services` (see the module documentation for the services left
/// out)
static TT_SERVICES: [(&str, FtService); 3] = [
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_TRUETYPE),
    ),
    (
        FT_SERVICE_ID_TRUETYPE_ENGINE,
        FtService::TrueTypeEngine(TT_SERVICE_TRUETYPE_ENGINE),
    ),
    (
        FT_SERVICE_ID_PROPERTIES,
        FtService::Properties(&TT_SERVICE_PROPERTIES),
    ),
];

/// `tt_get_interface`
fn tt_get_interface(
    driver: &FtModuleRec, /* TT_Driver */
    tt_interface: &str,
) -> Option<FtService> {
    let result = TT_SERVICES
        .iter()
        .find(|(id, _)| *id == tt_interface)
        .map(|(_, s)| *s);
    if result.is_some() {
        return result;
    }

    /* only return the default interface from the SFNT module */
    sfnt_get_interface(driver, tt_interface)
}

/* The FT_DriverInterface structure is defined in ftdriver.h. */

/* TT_USE_BYTECODE_INTERPRETER */
const TT_HINTER_FLAG: FtULong = FT_MODULE_DRIVER_HAS_HINTER;

fn tt_new_face(root: FtFaceRec) -> FtFace {
    FtFace::Tt(Box::new(TtFaceRec {
        root,
        ..Default::default()
    }))
}

fn tt_face_init_drv(
    face: &mut FtFace,
    typeface_index: FtInt,
    params: &[FtParameter],
) -> FtResult<()> {
    let FtFace::Tt(ttface) = face;
    tt_face_init(ttface, typeface_index, params)
}

fn tt_face_done_drv(face: &mut FtFace) {
    let FtFace::Tt(ttface) = face;
    tt_face_done(ttface)
}

fn tt_size_init_drv(face: &mut FtFace) -> FtResult<()> {
    let FtFace::Tt(ttface) = face;
    tt_size_init(ttface)
}

fn tt_size_done_drv(face: &mut FtFace) {
    let FtFace::Tt(ttface) = face;
    tt_size_done(ttface)
}

/// `tt_driver_class`
pub static TT_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER | FT_MODULE_DRIVER_SCALABLE | TT_HINTER_FLAG,

        module_name: "truetype",  /* driver name                           */
        module_version: 0x10000,  /* driver version == 1.0                 */
        module_requires: 0x20000, /* driver requires FreeType 2.0 or above */

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: Some(tt_driver_init), /* FT_Module_Constructor  module_init   */
        module_done: Some(tt_driver_done), /* FT_Module_Destructor   module_done   */
        get_interface: Some(tt_get_interface), /* FT_Module_Requester    get_interface */
    },

    new_face: tt_new_face,

    init_face: Some(tt_face_init_drv), /* FT_Face_InitFunc  init_face */
    done_face: Some(tt_face_done_drv), /* FT_Face_DoneFunc  done_face */
    init_size: Some(tt_size_init_drv), /* FT_Size_InitFunc  init_size */
    done_size: Some(tt_size_done_drv), /* FT_Size_DoneFunc  done_size */
    init_slot: Some(tt_slot_init),     /* FT_Slot_InitFunc  init_slot */
    done_slot: None,                   /* FT_Slot_DoneFunc  done_slot */
    load_glyph: Some(tt_glyph_load),   /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: Some(tt_get_kerning), /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,                 /* FT_Face_AttachFunc       attach_file  */
    get_advances: Some(tt_get_advances), /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: Some(tt_size_request), /* FT_Size_RequestFunc  request_size */
    select_size: Some(tt_size_select),   /* FT_Size_SelectFunc   select_size  */
};
