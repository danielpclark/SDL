// Rust translation of src/type1/t1objs.c and src/type1/t1objs.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 1 objects manager (body).
//!
//! The size and glyph slot are the face's: the size's PS hinter globals
//! (`T1_SizeRec`'s `internal->module_data`) and the slot's additions
//! (`T1_GlyphSlotRec`) are in the face record. The `psnames`, `psaux`
//! and `pshinter` modules' functions are called directly; the face
//! records whether the library has them. The root face's family and
//! style names are copies of the font info's strings (C points to them).

use super::super::base::ftcalc::fixed_to_int;
use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::super::psaux::t1cmap::*;
use super::super::psaux::*;
use super::super::pshinter::pshglob::PshGlobalsFuncsRec;
use super::super::pshinter::pshmod::T1HintsFuncs;
use super::super::t1tables::*;
use super::super::t1types::*;
use super::super::tttables::*;
use super::t1gload::t1_compute_max_advance;
use super::t1load::*;

/*************************************************************************
 *
 *                           SIZE FUNCTIONS
 *
 */

/// `T1_Size_Get_Globals_Funcs`
fn t1_size_get_globals_funcs(face: &T1FaceRec) -> Option<&'static PshGlobalsFuncsRec> {
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

/// `T1_Size_Done`
pub fn t1_size_done(face: &mut T1FaceRec) {
    if let Some(globals) = face.size_module_data.take() {
        if let Some(funcs) = t1_size_get_globals_funcs(face) {
            (funcs.destroy)(Some(globals));
        }
    }
}

/// `T1_Size_Init`
pub fn t1_size_init(face: &mut T1FaceRec) -> FtResult<()> {
    if let Some(funcs) = t1_size_get_globals_funcs(face) {
        let globals = (funcs.create)(&face.type1.private_dict)?;
        face.size_module_data = Some(globals);
    }

    Ok(())
}

/// `T1_Size_Request`
pub fn t1_size_request(face: &mut T1FaceRec, req: &FtSizeRequestRec) -> FtResult<()> {
    let funcs = t1_size_get_globals_funcs(face);

    ft_request_metrics(&mut face.root, req)?;

    if let Some(funcs) = funcs {
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
 *                           SLOT  FUNCTIONS
 *
 */

/// `T1_GlyphSlot_Done`
pub fn t1_glyph_slot_done(slot: &mut FtGlyphSlotRec) {
    slot.internal.glyph_hints = None;
}

/// `T1_GlyphSlot_Init` (the face's `pshinter` is the library's PS hinter)
pub fn t1_glyph_slot_init(slot: &mut FtGlyphSlotRec) -> FtResult<()> {
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
 *                           FACE  FUNCTIONS
 *
 */

/// `T1_Face_Done`: the face object destructor.
pub fn t1_face_done(face: &mut T1FaceRec) {
    /* (T1_CONFIG_OPTION_NO_MM_SUPPORT is undefined) */
    /* release multiple masters information */
    if !face.buildchar.is_empty() {
        face.buildchar = Vec::new();

        face.len_buildchar = 0;
    }

    t1_done_blend(face);
    face.blend = None;

    let type1 = &mut face.type1;

    /* release font info strings */
    {
        let info = &mut type1.font_info;

        info.version = None;
        info.notice = None;
        info.full_name = None;
        info.family_name = None;
        info.weight = None;
    }

    /* release top dictionary */
    type1.charstrings = Default::default();
    type1.glyph_names = Default::default();

    type1.subrs = None;

    type1.subrs_hash = None;

    type1.encoding.char_index = Vec::new();
    type1.encoding.char_name = Vec::new();
    type1.font_name = None;

    /* (T1_CONFIG_OPTION_NO_AFM is undefined) */
    /* release afm data if present */
    if let Some(afm_data) = face.afm_data.take() {
        super::t1afm::t1_done_metrics(afm_data);
    }

    /* release unicode map, if any */
    /* (`#if 0') */

    face.root.family_name = None;
    face.root.style_name = None;
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

/// `T1_Face_Init`: the face object constructor (the stream is the
/// face's; the parameters are ignored).
pub fn t1_face_init(
    face: &mut T1FaceRec,
    face_index: FtInt,
    _params: &[FtParameter],
) -> FtResult<()> {
    face.root.num_faces = 1;

    /* FT_FACE_FIND_GLOBAL_SERVICE( face, psnames, POSTSCRIPT_CMAPS ) */
    let psnames = has_module_interface(&face.root, "psnames", |_| true);
    face.psnames = psnames;

    face.psaux = has_module_interface(&face.root, "psaux", |i| {
        matches!(i, FtModuleInterface::PsAux)
    });
    if !face.psaux {
        return Err(FT_ERR_MISSING_MODULE);
    }

    face.pshinter = has_module_interface(&face.root, "pshinter", |i| {
        matches!(i, FtModuleInterface::PsHinter(_))
    });

    /* open the tokenizer; this will also check the font format */
    {
        let Some(mut stream) = face.root.stream.take() else {
            return Err(FT_ERR_INVALID_STREAM_HANDLE);
        };
        let error = t1_open_face(face, &mut stream);
        face.root.stream = Some(stream);
        error?;
    }

    /* if we just wanted to check the format, leave successfully now */
    if face_index < 0 {
        return Ok(());
    }

    /* check the face index */
    if (face_index & 0xFFFF) > 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* now load the font program into the face object */

    /* initialize the face object fields */

    /* set up root face fields */
    {
        let type1 = &face.type1;
        let info = &type1.font_info;
        let root = &mut face.root;

        root.num_glyphs = type1.num_glyphs as FtLong;
        root.face_index = 0;

        root.face_flags |= FT_FACE_FLAG_SCALABLE
            | FT_FACE_FLAG_HORIZONTAL
            | FT_FACE_FLAG_GLYPH_NAMES
            | FT_FACE_FLAG_HINTER;

        if info.is_fixed_pitch != 0 {
            root.face_flags |= FT_FACE_FLAG_FIXED_WIDTH;
        }

        if face.blend.is_some() {
            root.face_flags |= FT_FACE_FLAG_MULTIPLE_MASTERS;
        }

        /* The following code to extract the family and the style is very   */
        /* simplistic and might get some things wrong.  For a full-featured */
        /* algorithm you might have a look at the whitepaper given at       */
        /*                                                                  */
        /*   https://blogs.msdn.com/text/archive/2007/04/23/wpf-font-selection-model.aspx */

        /* get style name -- be careful, some broken fonts only */
        /* have a `/FontName' dictionary entry!                 */
        let mut family_name: Option<&[u8]> = info.family_name.as_deref();
        let mut style_name: Option<&[u8]> = None;

        if let Some(family) = family_name {
            if let Some(full) = info.full_name.as_deref() {
                let mut the_same = true;
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
                        the_same = false;

                        if at(family, g) == 0 {
                            style_name = Some(&full[f..]);
                        }
                        break;
                    }
                }

                if the_same {
                    style_name = Some(b"Regular");
                }
            }
        } else {
            /* do we have a `/FontName'? */
            if let Some(font_name) = type1.font_name.as_deref() {
                family_name = Some(font_name);
            }
        }

        if style_name.is_none() {
            if let Some(weight) = info.weight.as_deref() {
                style_name = Some(weight);
            } else {
                /* assume `Regular' style because we don't know better */
                style_name = Some(b"Regular");
            }
        }

        root.family_name = family_name.map(name_string);
        root.style_name = style_name.map(name_string);

        /* compute style flags */
        root.style_flags = 0;
        if info.italic_angle != 0 {
            root.style_flags |= FT_STYLE_FLAG_ITALIC;
        }
        if let Some(weight) = info.weight.as_deref() {
            if c_str(weight) == b"Bold" || c_str(weight) == b"Black" {
                root.style_flags |= FT_STYLE_FLAG_BOLD;
            }
        }

        /* no embedded bitmap support */
        root.num_fixed_sizes = 0;
        root.available_sizes = Vec::new();

        root.bbox.xMin = type1.font_bbox.xMin >> 16;
        root.bbox.yMin = type1.font_bbox.yMin >> 16;
        /* no `U' suffix here to 0xFFFF! */
        root.bbox.xMax = (type1.font_bbox.xMax + 0xFFFF) >> 16;
        root.bbox.yMax = (type1.font_bbox.yMax + 0xFFFF) >> 16;

        /* Set units_per_EM if we didn't set it in t1_parse_font_matrix. */
        if root.units_per_EM == 0 {
            root.units_per_EM = 1000;
        }

        root.ascender = root.bbox.yMax as FtShort;
        root.descender = root.bbox.yMin as FtShort;

        root.height = ((root.units_per_EM as FtInt * 12) / 10) as FtShort;
        if (root.height as FtInt) < root.ascender as FtInt - root.descender as FtInt {
            root.height = (root.ascender as FtInt - root.descender as FtInt) as FtShort;
        }

        /* now compute the maximum advance width */
        root.max_advance_width = root.bbox.xMax as FtShort;
    }
    {
        let mut max_advance: FtPos = 0;

        let error = t1_compute_max_advance(face, &mut max_advance);

        /* in case of error, keep the standard width */
        if error.is_ok() {
            face.root.max_advance_width = fixed_to_int(max_advance) as FtShort;
        }
        /* (else clear error) */
    }
    {
        let info = &face.type1.font_info;
        let root = &mut face.root;

        root.max_advance_height = root.height;

        root.underline_position = info.underline_position;
        root.underline_thickness = info.underline_thickness as FtShort;
    }

    if psnames {
        /* (the `psaux' module's charmap classes) */
        let mut charmap = FtCharMapRec {
            /* first of all, try to synthesize a Unicode charmap */
            platform_id: TT_PLATFORM_MICROSOFT,
            encoding_id: TT_MS_ID_UNICODE_CS,
            encoding: FT_ENCODING_UNICODE,
        };

        match t1_cmap_unicode_init(&face.type1) {
            Ok(data) => {
                ft_cmap_new(&mut face.root, &T1_CMAP_UNICODE_CLASS_REC, data, charmap)?;
            }
            Err(error)
                if error != FT_ERR_NO_UNICODE_GLYPH_NAME
                    && error != FT_ERR_UNIMPLEMENTED_FEATURE =>
            {
                return Err(error);
            }
            Err(_) => {}
        }

        /* now, generate an Adobe Standard encoding when appropriate */
        charmap.platform_id = TT_PLATFORM_ADOBE;
        let mut clazz: Option<&'static FtCMapClassRec> = None;
        let mut init: Option<fn(&T1FontRec) -> FtResult<FtCMapData>> = None;

        match face.type1.encoding_type {
            T1_ENCODING_TYPE_STANDARD => {
                charmap.encoding = FT_ENCODING_ADOBE_STANDARD;
                charmap.encoding_id = TT_ADOBE_ID_STANDARD;
                clazz = Some(&T1_CMAP_STANDARD_CLASS_REC);
                init = Some(t1_cmap_standard_init);
            }

            T1_ENCODING_TYPE_EXPERT => {
                charmap.encoding = FT_ENCODING_ADOBE_EXPERT;
                charmap.encoding_id = TT_ADOBE_ID_EXPERT;
                clazz = Some(&T1_CMAP_EXPERT_CLASS_REC);
                init = Some(t1_cmap_expert_init);
            }

            T1_ENCODING_TYPE_ARRAY => {
                charmap.encoding = FT_ENCODING_ADOBE_CUSTOM;
                charmap.encoding_id = TT_ADOBE_ID_CUSTOM;
                clazz = Some(&T1_CMAP_CUSTOM_CLASS_REC);
                init = Some(t1_cmap_custom_init);
            }

            T1_ENCODING_TYPE_ISOLATIN1 => {
                charmap.encoding = FT_ENCODING_ADOBE_LATIN_1;
                charmap.encoding_id = TT_ADOBE_ID_LATIN_1;
                clazz = Some(&T1_CMAP_UNICODE_CLASS_REC);
                init = Some(t1_cmap_unicode_init);
            }

            _ => {}
        }

        if let (Some(clazz), Some(init)) = (clazz, init) {
            let data = init(&face.type1)?;
            ft_cmap_new(&mut face.root, clazz, data, charmap)?;
        }
    }

    /* Exit: */
    Ok(())
}

/// `T1_Driver_Init`: initializes a given Type 1 driver object.
pub fn t1_driver_init(library: &mut FtLibraryRec, module: usize) -> FtResult<()> {
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

/// `T1_Driver_Done`: finalizes a given Type 1 driver.
pub fn t1_driver_done(_driver: &FtModuleRec) {}
