// Rust translation of src/pfr/pfrdrivr.c and src/pfr/pfrdrivr.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it), with the
// PFR metrics service of include/freetype/internal/services/svpfr.h.
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PFR driver interface (body).

use super::super::base::ftcalc::{ft_div_fix, ft_mul_div};
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::pfrobjs::*;

/* svpfr.h */

pub const FT_SERVICE_ID_PFR_METRICS: &str = "pfr-metrics";

/// `FT_PFR_GetMetricsFunc`: the outline and metrics resolutions and the
/// metrics' scales
pub type FtPfrGetMetricsFunc = fn(face: &FtFace) -> (FtUInt, FtUInt, FtFixed, FtFixed);

/// `FT_PFR_GetKerningFunc`
pub type FtPfrGetKerningFunc =
    fn(face: &mut FtFace, left: FtUInt, right: FtUInt) -> FtResult<FtVector>;

/// `FT_PFR_GetAdvanceFunc`
pub type FtPfrGetAdvanceFunc = fn(face: &FtFace, gindex: FtUInt) -> FtResult<FtPos>;

/// `FT_Service_PfrMetricsRec`
#[derive(Debug)]
pub struct FtServicePfrMetricsRec {
    pub get_metrics: FtPfrGetMetricsFunc,
    pub get_kerning: FtPfrGetKerningFunc,
    pub get_advance: FtPfrGetAdvanceFunc,
}

/* pfrdrivr.c */

/// `pfr_get_kerning`
fn pfr_get_kerning(face: &mut FtFace, left: FtUInt, right: FtUInt) -> FtResult<FtVector> {
    let FtFace::Pfr(pfrface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let mut avector = FtVector::default();

    let _ = pfr_face_get_kerning(pfrface, left, right, &mut avector);

    /* convert from metrics to outline units when necessary */
    let phys = &pfrface.phy_font;
    if phys.outline_resolution != phys.metrics_resolution {
        if avector.x != 0 {
            avector.x = ft_mul_div(
                avector.x,
                phys.outline_resolution as FtLong,
                phys.metrics_resolution as FtLong,
            );
        }

        if avector.y != 0 {
            avector.y = ft_mul_div(
                avector.y,
                phys.outline_resolution as FtLong,
                phys.metrics_resolution as FtLong,
            );
        }
    }

    Ok(avector)
}

/*
 * PFR METRICS SERVICE
 *
 */

/// `pfr_get_advance`
fn pfr_get_advance(face: &FtFace, gindex: FtUInt) -> FtResult<FtPos> {
    let error = FT_ERR_INVALID_ARGUMENT;

    if gindex == 0 {
        return Err(error);
    }

    let gindex = gindex - 1;

    if let FtFace::Pfr(pfrface) = face {
        let phys = &pfrface.phy_font;

        if gindex < phys.num_chars {
            return Ok(phys.chars[gindex as usize].advance as FtPos);
        }
    }

    /* Exit: */
    Err(error)
}

/// `pfr_get_metrics`
fn pfr_get_metrics(face: &FtFace) -> (FtUInt, FtUInt, FtFixed, FtFixed) {
    let FtFace::Pfr(pfrface) = face else {
        return (0, 0, 0x10000, 0x10000);
    };
    let phys = &pfrface.phy_font;

    let mut x_scale: FtFixed = 0x10000;
    let mut y_scale: FtFixed = 0x10000;

    if pfrface.root.has_slot_and_size {
        let size = &pfrface.root.size;

        x_scale = ft_div_fix(
            (size.metrics.x_ppem as FtLong) << 6,
            phys.metrics_resolution as FtLong,
        );

        y_scale = ft_div_fix(
            (size.metrics.y_ppem as FtLong) << 6,
            phys.metrics_resolution as FtLong,
        );
    }

    (
        phys.outline_resolution,
        phys.metrics_resolution,
        x_scale,
        y_scale,
    )
}

/// `pfr_face_get_kerning`, as the service's `get_kerning`
fn pfr_service_get_kerning(face: &mut FtFace, left: FtUInt, right: FtUInt) -> FtResult<FtVector> {
    let FtFace::Pfr(pfrface) = face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };
    let mut kerning = FtVector::default();

    pfr_face_get_kerning(pfrface, left, right, &mut kerning)?;
    Ok(kerning)
}

/// `pfr_metrics_service_rec`
static PFR_METRICS_SERVICE_REC: FtServicePfrMetricsRec = FtServicePfrMetricsRec {
    get_metrics: pfr_get_metrics,         /* get_metrics */
    get_kerning: pfr_service_get_kerning, /* get_kerning */
    get_advance: pfr_get_advance,         /* get_advance */
};

/// `FT_FONT_FORMAT_PFR`
pub const FT_FONT_FORMAT_PFR: &str = "PFR";

/*
 * SERVICE LIST
 *
 */

/// `pfr_services`
static PFR_SERVICES: [(&str, FtService); 2] = [
    (
        FT_SERVICE_ID_PFR_METRICS,
        FtService::PfrMetrics(&PFR_METRICS_SERVICE_REC),
    ),
    (
        FT_SERVICE_ID_FONT_FORMAT,
        FtService::FontFormat(FT_FONT_FORMAT_PFR),
    ),
];

/// `pfr_get_service`
fn pfr_get_service(_module: &FtModuleRec, service_id: &str) -> Option<FtService> {
    PFR_SERVICES
        .iter()
        .find(|(id, _)| *id == service_id)
        .map(|(_, s)| *s)
}

fn pfr_new_face(root: FtFaceRec) -> FtFace {
    FtFace::Pfr(Box::new(PfrFaceRec {
        root,
        ..Default::default()
    }))
}

/// `pfr_driver_class`
pub static PFR_DRIVER_CLASS: FtDriverClassRec = FtDriverClassRec {
    root: FtModuleClass {
        module_flags: FT_MODULE_FONT_DRIVER | FT_MODULE_DRIVER_SCALABLE,

        module_name: "pfr",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module-specific interface */

        module_init: None, /* FT_Module_Constructor  module_init   */
        module_done: None, /* FT_Module_Destructor   module_done   */
        get_interface: Some(pfr_get_service), /* FT_Module_Requester    get_interface */
    },

    new_face: pfr_new_face,

    init_face: Some(pfr_face_init), /* FT_Face_InitFunc  init_face */
    done_face: Some(pfr_face_done), /* FT_Face_DoneFunc  done_face */
    init_size: None,                /* FT_Size_InitFunc  init_size */
    done_size: None,                /* FT_Size_DoneFunc  done_size */
    init_slot: Some(pfr_slot_init), /* FT_Slot_InitFunc  init_slot */
    done_slot: Some(pfr_slot_done), /* FT_Slot_DoneFunc  done_slot */

    load_glyph: Some(pfr_slot_load), /* FT_Slot_LoadFunc  load_glyph */

    get_kerning: Some(pfr_get_kerning), /* FT_Face_GetKerningFunc   get_kerning  */
    attach_file: None,                  /* FT_Face_AttachFunc       attach_file  */
    get_advances: None,                 /* FT_Face_GetAdvancesFunc  get_advances */

    request_size: None, /* FT_Size_RequestFunc  request_size */
    select_size: None,  /* FT_Size_SelectFunc   select_size  */
};
