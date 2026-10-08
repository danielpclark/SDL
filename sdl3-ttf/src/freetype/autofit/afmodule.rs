// Rust translation of src/autofit/afmodule.c and afmodule.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter module implementation.
//!
//! Translation notes: the module's data (`AF_ModuleRec` past its `root`)
//! is the module record's properties; a glyph load works on a copy of it.
//! The face properties (`increase-x-height` and `glyph-to-script-map`)
//! take a face, which the property values of this translation can't
//! carry; [`af_property_set_increase_x_height`] and
//! [`af_property_get_face_globals`] are their face-taking forms.

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::afglobal::*;
use super::afhints::{af_glyph_hints_done, af_glyph_hints_init};
use super::afloader::*;
use super::afstyles::AF_STYLE_CLASSES;
use super::aftypes::*;

/// The stem darkening parameters of the CFF driver
/// (`CFF_CONFIG_OPTION_DARKENING_PARAMETER_*`, from `ftoption.h`)
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X1: FtInt = 500;
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y1: FtInt = 400;
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X2: FtInt = 1000;
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y2: FtInt = 275;
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X3: FtInt = 1667;
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y3: FtInt = 275;
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X4: FtInt = 2333;
const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y4: FtInt = 0;

/*
 * This is the `extended' FT_Module structure that holds the
 * autofitter's global data.
 */

/// `AF_ModuleRec` (without its `root`)
#[derive(Debug, Clone, Copy, Default)]
pub struct AfModuleRec {
    pub fallback_style: FtUInt,
    pub default_script: AfScript,
    pub no_stem_darkening: bool,
    pub darken_params: [FtInt; 8],
}

/// `af_property_get_face_globals`: runs `f` on the face's globals,
/// computing them if necessary
pub fn af_property_get_face_globals<R>(
    face: &mut FtFace,
    module: &AfModuleRec,
    f: impl FnOnce(&mut AfFaceGlobalsRec) -> R,
) -> FtResult<R> {
    let mut globals = match face.autohint.take() {
        Some(data) => match data.downcast::<AfFaceGlobalsRec>() {
            Ok(globals) => globals,
            Err(data) => {
                face.autohint = Some(data);
                return Err(FT_ERR_INVALID_ARGUMENT);
            }
        },
        None => {
            /* trigger computation of the global style data */
            /* in case it hasn't been done yet              */
            af_face_globals_new(face, module)?
        }
    };

    let r = f(&mut globals);
    face.autohint = Some(globals);
    Ok(r)
}

/// `af_property_set` of `increase-x-height` (an `FT_Prop_IncreaseXHeight`)
pub fn af_property_set_increase_x_height(
    library: &FtLibraryRec,
    face: &mut FtFace,
    limit: FtUInt,
) -> FtResult<()> {
    let module = af_module_props(library)?;
    af_property_get_face_globals(face, &module, |globals| globals.increase_x_height = limit)
}

/// The auto-fitter module's data of `library`.
fn af_module_props(library: &FtLibraryRec) -> FtResult<AfModuleRec> {
    let module = library
        .get_module("autofitter")
        .ok_or(FT_ERR_MISSING_MODULE)?;
    library.modules[module]
        .with_props(|m: &mut AfModuleRec| *m)
        .ok_or(FT_ERR_MISSING_MODULE)
}

/// `af_property_set`
fn af_property_set(
    ft_module: &FtModuleRec,
    property_name: &str,
    value: &FtPropertyValue,
    value_is_string: bool,
) -> FtResult<()> {
    let mut error: FtResult<()> = Ok(());

    if property_name == "fallback-script" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        if value_is_string {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        let fallback_script = match *value {
            FtPropertyValue::UInt(v) => v,
            FtPropertyValue::Int(v) => v as FtUInt,
            _ => return Err(FT_ERR_INVALID_ARGUMENT),
        };

        /* We translate the fallback script to a fallback style that uses */
        /* `fallback-script' as its script and `AF_COVERAGE_NONE' as its  */
        /* coverage value.                                                */
        let found = AF_STYLE_CLASSES.iter().position(|style_class| {
            style_class.script == fallback_script && style_class.coverage == AF_COVERAGE_DEFAULT
        });

        match found {
            Some(ss) => {
                ft_module
                    .with_props(|module: &mut AfModuleRec| module.fallback_style = ss as FtUInt);
            }
            None => return Err(FT_ERR_INVALID_ARGUMENT),
        }

        return error;
    } else if property_name == "default-script" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        if value_is_string {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        let default_script = match *value {
            FtPropertyValue::UInt(v) => v,
            FtPropertyValue::Int(v) => v as FtUInt,
            _ => return Err(FT_ERR_INVALID_ARGUMENT),
        };

        ft_module.with_props(|module: &mut AfModuleRec| module.default_script = default_script);

        return error;
    } else if property_name == "increase-x-height" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        if value_is_string {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        /* (an `FT_Prop_IncreaseXHeight' names a face, which this */
        /* property value can't; see af_property_set_increase_x_height) */
        error = Err(FT_ERR_INVALID_ARGUMENT);
        return error;
    } else if property_name == "darkening-parameters" {
        let darken_params: [FtInt; 8];

        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        if value_is_string {
            let FtPropertyValue::Str(s) = value else {
                return Err(FT_ERR_INVALID_ARGUMENT);
            };
            let s = s.as_bytes();
            let mut dp: [FtInt; 8] = [0; 8];
            let mut pos = 0usize;

            /* eight comma-separated numbers */
            for (i, d) in dp.iter_mut().enumerate().take(7) {
                let _ = i;
                let (v, ep) = ft_strtol_prefix(s, pos);
                *d = v as FtInt;
                if s.get(ep) != Some(&b',') || pos == ep {
                    return Err(FT_ERR_INVALID_ARGUMENT);
                }

                pos = ep + 1;
            }

            let (v, ep) = ft_strtol_prefix(s, pos);
            dp[7] = v as FtInt;
            if !(ep == s.len() || s[ep] == b' ') || pos == ep {
                return Err(FT_ERR_INVALID_ARGUMENT);
            }

            darken_params = dp;
        } else {
            match value {
                FtPropertyValue::IntArray(v) if v.len() >= 8 => {
                    let mut dp = [0; 8];
                    dp.copy_from_slice(&v[..8]);
                    darken_params = dp;
                }
                _ => return Err(FT_ERR_INVALID_ARGUMENT),
            }
        }

        let x1 = darken_params[0];
        let y1 = darken_params[1];
        let x2 = darken_params[2];
        let y2 = darken_params[3];
        let x3 = darken_params[4];
        let y3 = darken_params[5];
        let x4 = darken_params[6];
        let y4 = darken_params[7];

        if x1 < 0
            || x2 < 0
            || x3 < 0
            || x4 < 0
            || y1 < 0
            || y2 < 0
            || y3 < 0
            || y4 < 0
            || x1 > x2
            || x2 > x3
            || x3 > x4
            || y1 > 500
            || y2 > 500
            || y3 > 500
            || y4 > 500
        {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        ft_module.with_props(|module: &mut AfModuleRec| {
            module.darken_params = [x1, y1, x2, y2, x3, y3, x4, y4];
        });

        return error;
    } else if property_name == "no-stem-darkening" {
        /* FT_CONFIG_OPTION_ENVIRONMENT_PROPERTIES */
        let no_stem_darkening = if value_is_string {
            let FtPropertyValue::Str(s) = value else {
                return Err(FT_ERR_INVALID_ARGUMENT);
            };
            let nsd = ft_strtol_prefix(s.as_bytes(), 0).0;

            nsd != 0
        } else {
            match *value {
                FtPropertyValue::Bool(b) => b,
                FtPropertyValue::Int(v) => v != 0,
                FtPropertyValue::UInt(v) => v != 0,
                _ => return Err(FT_ERR_INVALID_ARGUMENT),
            }
        };

        ft_module
            .with_props(|module: &mut AfModuleRec| module.no_stem_darkening = no_stem_darkening);

        return error;
    }

    Err(FT_ERR_MISSING_PROPERTY)
}

/// `ft_strtol( s + pos, &ep, 10 )`: the value and the end position
fn ft_strtol_prefix(s: &[u8], pos: usize) -> (FtLong, usize) {
    let mut i = pos;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        neg = s[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut v: FtLong = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add((s[i] - b'0') as FtLong);
        i += 1;
    }
    if i == start {
        /* no conversion: the end is the start of the string */
        return (0, pos);
    }
    (if neg { -v } else { v }, i)
}

/// `af_property_get`
fn af_property_get(ft_module: &FtModuleRec, property_name: &str) -> FtResult<FtPropertyValue> {
    let module = ft_module
        .with_props(|m: &mut AfModuleRec| *m)
        .ok_or(FT_ERR_MISSING_MODULE)?;

    if property_name == "glyph-to-script-map" {
        /* (an `FT_Prop_GlyphToScriptMap' names a face; see */
        /* af_property_get_face_globals)                   */
        return Err(FT_ERR_INVALID_ARGUMENT);
    } else if property_name == "fallback-script" {
        let style_class = &AF_STYLE_CLASSES[module.fallback_style as usize];

        return Ok(FtPropertyValue::UInt(style_class.script));
    } else if property_name == "default-script" {
        return Ok(FtPropertyValue::UInt(module.default_script));
    } else if property_name == "increase-x-height" {
        /* (an `FT_Prop_IncreaseXHeight' names a face; see */
        /* af_property_get_face_globals)                  */
        return Err(FT_ERR_INVALID_ARGUMENT);
    } else if property_name == "darkening-parameters" {
        let darken_params = module.darken_params;

        return Ok(FtPropertyValue::IntArray(darken_params.to_vec()));
    } else if property_name == "no-stem-darkening" {
        let no_stem_darkening = module.no_stem_darkening;

        return Ok(FtPropertyValue::Bool(no_stem_darkening));
    }

    Err(FT_ERR_MISSING_PROPERTY)
}

/// `af_service_properties`
static AF_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: af_property_set, /* FT_Properties_SetFunc set_property */
    get_property: af_property_get, /* FT_Properties_GetFunc get_property */
};

/// `af_get_interface`
fn af_get_interface(_module: &FtModuleRec, module_interface: &str) -> Option<FtService> {
    if module_interface == FT_SERVICE_ID_PROPERTIES {
        return Some(FtService::Properties(&AF_SERVICE_PROPERTIES));
    }
    None
}

/// `af_autofitter_init`
fn af_autofitter_init(library: &mut FtLibraryRec, ft_module: usize) -> FtResult<()> {
    let module = AfModuleRec {
        fallback_style: AF_STYLE_FALLBACK,
        default_script: AF_SCRIPT_DEFAULT,
        no_stem_darkening: true,

        darken_params: [
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_X1,
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y1,
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_X2,
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y2,
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_X3,
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y3,
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_X4,
            CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y4,
        ],
    };

    let m = &library.modules[ft_module];
    *m.props.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(module));

    Ok(())
}

/// `af_autofitter_done`
fn af_autofitter_done(_ft_module: &FtModuleRec) {}

/// `af_autofitter_load_glyph`
fn af_autofitter_load_glyph(
    library: &FtLibraryRec,
    module_: usize,
    face: &mut FtFace,
    glyph_index: FtUInt,
    load_flags: FtInt32,
) -> FtResult<()> {
    let module = library.modules[module_]
        .with_props(|m: &mut AfModuleRec| *m)
        .ok_or(FT_ERR_MISSING_MODULE)?;

    /* !FT_DEBUG_AUTOFIT */
    let hints = af_glyph_hints_init();
    let mut loader = af_loader_init(hints);

    let error = af_loader_load_glyph(&mut loader, &module, face, glyph_index, load_flags);

    af_loader_done(&mut loader);
    af_glyph_hints_done(&mut loader.hints);

    error
}

/// `af_autofitter_interface`
static AF_AUTOFITTER_INTERFACE: FtAutoHinterInterfaceRec = FtAutoHinterInterfaceRec {
    /* NULL, FT_AutoHinter_GlobalResetFunc reset_face        */
    /* NULL, FT_AutoHinter_GlobalGetFunc   get_global_hints  */
    /* NULL, FT_AutoHinter_GlobalDoneFunc  done_global_hints */
    load_glyph: af_autofitter_load_glyph, /* FT_AutoHinter_GlyphLoadFunc   load_glyph        */
};

/// `autofit_module_class`
pub static AUTOFIT_MODULE_CLASS: FtModuleClass = FtModuleClass {
    module_flags: FT_MODULE_HINTER,
    /* sizeof ( AF_ModuleRec ) */
    module_name: "autofitter",
    module_version: 0x10000,  /* version 1.0 of the autofitter  */
    module_requires: 0x20000, /* requires FreeType 2.0 or above */

    module_interface: FtModuleInterface::AutoHinter(&AF_AUTOFITTER_INTERFACE),

    module_init: Some(af_autofitter_init), /* FT_Module_Constructor module_init   */
    module_done: Some(af_autofitter_done), /* FT_Module_Destructor  module_done   */
    get_interface: Some(af_get_interface), /* FT_Module_Requester   get_interface */
};
