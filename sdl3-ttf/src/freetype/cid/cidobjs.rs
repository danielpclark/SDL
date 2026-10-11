// Rust translation of src/cid/cidobjs.c and src/cid/cidobjs.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! CID objects manager (body).
//!
//! The size and glyph slot are the face's, as for Type 1 faces; the root
//! face's family and style names are copies of the font info's strings.

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::super::psaux::*;
use super::super::pshinter::pshglob::PshGlobalsFuncsRec;
use super::super::pshinter::pshmod::T1HintsFuncs;
use super::super::t1types::*;
use super::cidload::cid_face_open;

/*************************************************************************
 *
 *                           SLOT  FUNCTIONS
 *
 */

/// `cid_slot_done`
pub fn cid_slot_done(slot: &mut FtGlyphSlotRec) {
    slot.internal.glyph_hints = None;
}

/// `cid_slot_init` (the face's `pshinter` is the library's PS hinter)
pub fn cid_slot_init(slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    let Some(library) = slot.library.as_ref() else {
        return Ok(());
    };

    if let Some(module) = library.get_module("pshinter") {
        if let FtModuleInterface::PsHinter(pshinter) =
            library.modules[module].clazz.root().module_interface
        {
            let funcs: T1HintsFuncs = (pshinter.get_t1_funcs)(&library.modules[module]);
            slot.internal.glyph_hints = Some(Box::new(funcs));
        }
    }

    Ok(())
}

/*************************************************************************
 *
 *                          SIZE  FUNCTIONS
 *
 */

/// `cid_size_get_globals_funcs`
fn cid_size_get_globals_funcs(face: &CidFaceRec) -> Option<&'static PshGlobalsFuncsRec> {
    let library = face.root.library.as_ref()?;

    let module = library.get_module("pshinter");

    match (module, face.pshinter) {
        (Some(module), true) => match library.modules[module].clazz.root().module_interface {
            FtModuleInterface::PsHinter(pshinter) => {
                Some((pshinter.get_globals_funcs)(&library.modules[module]))
            }
            _ => None,
        },
        _ => None,
    }
}

/// `cid_size_done`
pub fn cid_size_done(face: &mut CidFaceRec) {
    if let Some(globals) = face.size_module_data.take() {
        if let Some(funcs) = cid_size_get_globals_funcs(face) {
            (funcs.destroy)(Some(globals));
        }
    }
}

/// `cid_size_init`
pub fn cid_size_init(face: &mut CidFaceRec) -> FtResult<()> {
    if let Some(funcs) = cid_size_get_globals_funcs(face) {
        let Some(dict) = face.cid.font_dicts.get(face.root.face_index as usize) else {
            return Err(FT_ERR_INVALID_ARGUMENT);
        };
        let priv_ = &dict.private_dict;

        let globals = (funcs.create)(priv_)?;
        face.size_module_data = Some(globals);
    }

    Ok(())
}

/// `cid_size_request`
pub fn cid_size_request(face: &mut CidFaceRec, req: &FtSizeRequestRec) -> FtResult<()> {
    ft_request_metrics(&mut face.root, req)?;

    if let Some(funcs) = cid_size_get_globals_funcs(face) {
        let metrics = face.root.size.metrics;
        if let Some(globals) = face.size_module_data.as_mut() {
            (funcs.set_scale)(globals, metrics.x_scale, metrics.y_scale, 0, 0);
        }
    }

    /* Exit: */
    Ok(())
}

/*************************************************************************
 *
 *                          FACE  FUNCTIONS
 *
 */

/// `cid_face_done`: finalizes a given face object.
pub fn cid_face_done(face: &mut CidFaceRec) {
    let cid = &mut face.cid;

    /* release subrs */
    face.subrs = Vec::new();

    /* release FontInfo strings */
    let info = &mut cid.font_info;
    info.version = None;
    info.notice = None;
    info.full_name = None;
    info.family_name = None;
    info.weight = None;

    /* release font dictionaries */
    cid.font_dicts = Vec::new();
    cid.num_dicts = 0;

    /* release other strings */
    cid.cid_font_name = None;
    cid.registry = None;
    cid.ordering = None;

    face.root.family_name = None;
    face.root.style_name = None;

    face.cid_stream = None;
}

/// A C string's bytes (up to its first null byte).
fn c_str(s: &[u8]) -> &[u8] {
    let len = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..len]
}

/// A C string's bytes as a string.
fn name_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(c_str(bytes)).into_owned()
}

/// Whether the library has the module `name` with the interface `f`
/// checks (`FT_Get_Module_Interface`).
fn has_module_interface(face: &FtFaceRec, name: &str, f: fn(&FtModuleInterface) -> bool) -> bool {
    let Some(library) = face.library.as_ref() else {
        return false;
    };
    match library.get_module(name) {
        Some(m) => f(&library.modules[m].clazz.root().module_interface),
        None => false,
    }
}

/// `cid_face_init`: initializes a given CID face object (the stream is
/// the face's; the parameters are ignored).
pub fn cid_face_init(
    face: &mut CidFaceRec,
    face_index: FtInt,
    _params: &[FtParameter],
) -> FtResult<()> {
    face.root.num_faces = 1;

    if !face.psaux {
        face.psaux = has_module_interface(&face.root, "psaux", |i| {
            matches!(i, FtModuleInterface::PsAux)
        });
        if !face.psaux {
            return Err(FT_ERR_MISSING_MODULE);
        }
    }

    if !face.pshinter {
        face.pshinter = has_module_interface(&face.root, "pshinter", |i| {
            matches!(i, FtModuleInterface::PsHinter(_))
        });
    }

    /* (the decoder's FT_FACE_FIND_GLOBAL_SERVICE( face, psnames, ... )) */
    face.psnames = has_module_interface(&face.root, "psnames", |_| true);

    /* open the tokenizer; this will also check the font format */
    face.root
        .stream
        .as_mut()
        .ok_or(FT_ERR_INVALID_STREAM_HANDLE)?
        .seek(0)?;

    cid_face_open(face, face_index)?;

    /* if we just wanted to check the format, leave successfully now */
    if face_index < 0 {
        return Ok(());
    }

    /* check the face index */
    /* XXX: handle CID fonts with more than a single face */
    if (face_index & 0xFFFF) != 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* now load the font program into the face object */

    /* initialize the face object fields */

    /* set up root face fields */
    {
        let cid = &face.cid;
        let info = &cid.font_info;
        let cidface = &mut face.root;

        cidface.num_glyphs = cid.cid_count as FtLong;
        cidface.num_charmaps = 0;

        cidface.face_index = (face_index & 0xFFFF) as FtLong;

        cidface.face_flags |= FT_FACE_FLAG_SCALABLE /* scalable outlines */
            | FT_FACE_FLAG_HORIZONTAL /* horizontal data   */
            | FT_FACE_FLAG_HINTER; /* has native hinter */

        if info.is_fixed_pitch != 0 {
            cidface.face_flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }

        /*
         * For the sfnt-wrapped CID fonts for MacOS, currently,
         * its `cmap' tables are ignored, and the content in
         * its `CID ' table is treated the same as naked CID-keyed
         * font.  See ft_lookup_PS_in_sfnt_stream().
         */
        cidface.face_flags |= FT_FACE_FLAG_CID_KEYED;

        /* XXX: TODO: add kerning with .afm support */

        /* get style name -- be careful, some broken fonts only */
        /* have a /FontName dictionary entry!                   */
        let mut family_name: Option<&[u8]> = info.family_name.as_deref();

        /* assume "Regular" style if we don't know better */
        let mut style_name: &[u8] = b"Regular";
        if let Some(family) = family_name {
            if let Some(full) = info.full_name.as_deref() {
                let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
                let (mut f, mut g) = (0usize, 0usize);

                while at(full, f) != 0 {
                    if at(full, f) == at(family, g) {
                        g += 1;
                        f += 1;
                    } else if at(full, f) == b' ' || at(full, f) == b'-' {
                        f += 1;
                    } else if at(family, g) == b' ' || at(family, g) == b'-' {
                        g += 1;
                    } else {
                        if at(family, g) == 0 {
                            style_name = &full[f..];
                        }
                        break;
                    }
                }
            }
        } else {
            /* do we have a `/FontName'? */
            if let Some(name) = cid.cid_font_name.as_deref() {
                family_name = Some(name);
            }
        }

        cidface.family_name = family_name.map(name_string);
        cidface.style_name = Some(name_string(style_name));

        /* compute style flags */
        cidface.style_flags = 0;
        if info.italic_angle != 0 {
            cidface.style_flags |= FT_STYLE_FLAG_ITALIC;
        }
        if let Some(weight) = info.weight.as_deref() {
            if c_str(weight) == b"Bold" || c_str(weight) == b"Black" {
                cidface.style_flags |= FT_STYLE_FLAG_BOLD;
            }
        }

        /* no embedded bitmap support */
        cidface.num_fixed_sizes = 0;
        cidface.available_sizes = Vec::new();

        cidface.bbox.xMin = cid.font_bbox.xMin >> 16;
        cidface.bbox.yMin = cid.font_bbox.yMin >> 16;
        /* no `U' suffix here to 0xFFFF! */
        cidface.bbox.xMax = (cid.font_bbox.xMax + 0xFFFF) >> 16;
        cidface.bbox.yMax = (cid.font_bbox.yMax + 0xFFFF) >> 16;

        if cidface.units_per_EM == 0 {
            cidface.units_per_EM = 1000;
        }

        cidface.ascender = cidface.bbox.yMax as FtShort;
        cidface.descender = cidface.bbox.yMin as FtShort;

        cidface.height = ((cidface.units_per_EM as FtInt * 12) / 10) as FtShort;
        if (cidface.height as FtInt) < cidface.ascender as FtInt - cidface.descender as FtInt {
            cidface.height = (cidface.ascender as FtInt - cidface.descender as FtInt) as FtShort;
        }

        cidface.underline_position = info.underline_position;
        cidface.underline_thickness = info.underline_thickness as FtShort;
    }

    /* Exit: */
    Ok(())
}

/// `cid_driver_init`: initializes a given CID driver object.
pub fn cid_driver_init(library: &mut FtLibraryRec, module: usize) -> FtResult<()> {
    let mut driver = PsDriverRec {
        /* set default property values, cf. `ftt1drv.h' */
        hinting_engine: FT_HINTING_ADOBE,

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
        random_seed: 0,
    };

    /* compute random seed from some memory addresses */
    let seed_local: FtUInt32 = 0;
    let mut seed: FtUInt32 = ((&seed_local as *const FtUInt32 as usize)
        ^ (&module as *const usize as usize)
        ^ (library as *const FtLibraryRec as usize)) as FtUInt32;
    seed = seed ^ (seed >> 10) ^ (seed >> 20);

    driver.random_seed = seed as FtInt32;
    if driver.random_seed < 0 {
        driver.random_seed = driver.random_seed.wrapping_neg();
    } else if driver.random_seed == 0 {
        driver.random_seed = 123456789;
    }

    let m = &library.modules[module];
    *m.props.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(driver));

    Ok(())
}

/// `cid_driver_done`: finalizes a given CID driver.
pub fn cid_driver_done(_driver: &FtModuleRec) {}
